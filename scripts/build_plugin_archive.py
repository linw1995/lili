#!/usr/bin/env python3

import argparse
import hashlib
import json
import os
import shutil
import stat
import tempfile
import zipfile
from pathlib import Path

import tomllib
from check_plugin_package import PolicyViolation, validate_workspace

TARGETS = {
    "arm64-apple-darwin": {
        "fileName": "lili-hook",
        "magics": (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf"),
    },
    "x86_64-unknown-linux-gnu": {
        "fileName": "lili-hook",
        "magics": (b"\x7fELF",),
    },
    "x86_64-pc-windows-msvc": {
        "fileName": "lili-hook.exe",
        "magics": (b"MZ",),
    },
}
SIGNATURE_KINDS = {"platform-standard", "signed"}
SIGNATURE_EVIDENCE = {
    "arm64-apple-darwin": {
        "verifier": "codesign --verify --strict",
        "unsignedStatus": "unsigned-allowed",
    },
    "x86_64-unknown-linux-gnu": {
        "verifier": "ELF format and SHA-256 integrity",
        "unsignedStatus": "not-applicable",
    },
    "x86_64-pc-windows-msvc": {
        "verifier": "Get-AuthenticodeSignature",
        "unsignedStatus": "unsigned-allowed",
    },
}
FIXED_ZIP_TIME = (1980, 1, 1, 0, 0, 0)


class ArchiveError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ArchiveError(message)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_json(path: Path, label: str = "forwarder manifest") -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ArchiveError(f"invalid {label}: {path}") from error
    require(isinstance(value, dict), f"{label} must be an object: {path}")
    return value


def workspace_version(workspace_root: Path) -> str:
    with (workspace_root / "Cargo.toml").open("rb") as cargo_file:
        return tomllib.load(cargo_file)["workspace"]["package"]["version"]


def validate_forwarder(
    forwarders_root: Path,
    target: str,
    version: str,
) -> tuple[Path, dict]:
    target_policy = TARGETS[target]
    target_root = forwarders_root / target
    require(
        not target_root.is_symlink() and target_root.is_dir(),
        f"invalid forwarder target directory: {target}",
    )
    binary = target_root / target_policy["fileName"]
    manifest_path = target_root / "manifest.json"
    require(
        not binary.is_symlink() and binary.is_file(), f"missing forwarder: {target}"
    )
    require(
        not manifest_path.is_symlink() and manifest_path.is_file(),
        f"missing forwarder manifest: {target}",
    )
    require(
        {path.name for path in target_root.iterdir()}
        == {target_policy["fileName"], "manifest.json"},
        f"unexpected forwarder artifact content: {target}",
    )

    manifest = load_json(manifest_path)
    require(
        set(manifest)
        == {
            "schemaVersion",
            "product",
            "component",
            "version",
            "reportedVersion",
            "platform",
            "fileName",
            "signatureKind",
            "signatureVerifier",
            "signatureStatus",
            "size",
            "sha256",
        },
        f"forwarder manifest fields drifted: {target}",
    )
    expected = {
        "schemaVersion": 2,
        "product": "Lili",
        "component": "lili-hook",
        "version": version,
        "reportedVersion": version,
        "platform": target,
        "fileName": target_policy["fileName"],
    }
    require(
        all(manifest.get(key) == value for key, value in expected.items()),
        f"forwarder identity drifted: {target}",
    )
    require(
        manifest["signatureKind"] in SIGNATURE_KINDS,
        f"invalid signature kind: {target}",
    )
    signature = SIGNATURE_EVIDENCE[target]
    require(
        manifest["signatureVerifier"] == signature["verifier"],
        f"signature verifier drifted: {target}",
    )
    expected_status = (
        "verified"
        if manifest["signatureKind"] == "signed"
        else signature["unsignedStatus"]
    )
    require(
        manifest["signatureStatus"] == expected_status,
        f"signature verification status drifted: {target}",
    )
    require(
        target != "x86_64-unknown-linux-gnu"
        or manifest["signatureKind"] == "platform-standard",
        "Linux forwarder declares an unsupported signing scheme",
    )

    contents = binary.read_bytes()
    require(manifest["size"] == len(contents), f"forwarder size drifted: {target}")
    require(
        manifest["sha256"] == sha256(contents), f"forwarder checksum drifted: {target}"
    )
    require(
        any(contents.startswith(magic) for magic in target_policy["magics"]),
        f"forwarder file format does not match target: {target}",
    )
    return binary, manifest


def normalized_mode(relative: str, declared_executables: set[str]) -> int:
    if relative in declared_executables and not relative.endswith(".ps1"):
        return 0o755
    return 0o644


def collect_entries(
    plugin_root: Path,
    workspace_root: Path,
    declared_executables: set[str],
) -> list[dict]:
    workspace_bytes = str(workspace_root.resolve()).encode()
    entries = []
    for path in sorted(
        plugin_root.rglob("*"),
        key=lambda value: value.relative_to(plugin_root).as_posix(),
    ):
        relative = path.relative_to(plugin_root).as_posix()
        require(
            not path.is_symlink(),
            f"plugin archive cannot contain symbolic links: {relative}",
        )
        if path.is_dir():
            continue
        require(path.is_file(), f"unsupported plugin archive entry: {relative}")
        contents = path.read_bytes()
        require(
            workspace_bytes not in contents,
            f"development path leaked into plugin archive: {relative}",
        )
        entries.append(
            {
                "path": relative,
                "contents": contents,
                "size": len(contents),
                "sha256": sha256(contents),
                "mode": normalized_mode(relative, declared_executables),
            }
        )
    return entries


def write_zip(output: Path, entries: list[dict]) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{output.name}.", dir=output.parent
    )
    os.close(descriptor)
    temporary = Path(temporary_name)
    try:
        with zipfile.ZipFile(
            temporary,
            "w",
            compression=zipfile.ZIP_DEFLATED,
            compresslevel=9,
        ) as archive:
            for entry in entries:
                info = zipfile.ZipInfo(entry["path"], FIXED_ZIP_TIME)
                info.create_system = 3
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = (stat.S_IFREG | entry["mode"]) << 16
                archive.writestr(
                    info,
                    entry["contents"],
                    compress_type=zipfile.ZIP_DEFLATED,
                    compresslevel=9,
                )
        os.replace(temporary, output)
    finally:
        temporary.unlink(missing_ok=True)


def build_archive(
    workspace_root: Path,
    forwarders_root: Path,
    output: Path,
    supply_chain_path: Path,
) -> dict:
    workspace_root = workspace_root.resolve()
    forwarders_root = forwarders_root.resolve()
    output = output.resolve()
    supply_chain_path = supply_chain_path.resolve()
    plugin_source = workspace_root / "plugins" / "lili"
    require(plugin_source.is_dir(), "plugin source is missing")
    require(
        not (plugin_source / "bin").exists(),
        "plugin source must not contain build outputs",
    )
    require(
        not output.is_relative_to(plugin_source.resolve()),
        "plugin archive output must be outside the source package",
    )
    require(supply_chain_path.is_file(), "plugin supply-chain evidence is missing")
    try:
        validate_workspace(workspace_root)
    except PolicyViolation as error:
        raise ArchiveError(f"plugin source violates package policy: {error}") from error

    version = workspace_version(workspace_root)
    supply_chain_contents = supply_chain_path.read_bytes()
    supply_chain = load_json(supply_chain_path, "plugin supply-chain evidence")
    require(
        supply_chain.get("schemaVersion") == 1
        and supply_chain.get("component") == "plugin"
        and supply_chain.get("version") == version,
        "plugin supply-chain identity drifted",
    )
    require(
        supply_chain.get("licensePolicy", {}).get("result") == "passed"
        and supply_chain.get("vulnerabilityScan", {}).get("vulnerabilityCount") == 0,
        "plugin supply-chain gates did not pass",
    )
    forwarders = {
        target: validate_forwarder(forwarders_root, target, version)
        for target in sorted(TARGETS)
    }
    require(
        {path.name for path in forwarders_root.iterdir()} == set(TARGETS),
        "forwarder target set differs from the published release matrix",
    )

    policy = json.loads(
        (workspace_root / "marketplace" / "lili" / "package-policy.json").read_text(
            encoding="utf-8"
        )
    )
    declared_executables = set(policy["declaredExecutables"])
    with tempfile.TemporaryDirectory(
        prefix="lili-plugin-archive-"
    ) as temporary_directory:
        staging_root = Path(temporary_directory)
        shutil.copy2(workspace_root / "Cargo.toml", staging_root / "Cargo.toml")
        (staging_root / "lili").mkdir()
        shutil.copy2(
            workspace_root / "lili" / "tauri.conf.json",
            staging_root / "lili" / "tauri.conf.json",
        )
        (staging_root / "marketplace" / "lili").mkdir(parents=True)
        for name in ("package-policy.json", "submission.json"):
            shutil.copy2(
                workspace_root / "marketplace" / "lili" / name,
                staging_root / "marketplace" / "lili" / name,
            )
        staged_plugin = staging_root / "plugins" / "lili"
        shutil.copytree(plugin_source, staged_plugin)
        for target, (binary, _) in forwarders.items():
            destination = staged_plugin / "bin" / target / TARGETS[target]["fileName"]
            destination.parent.mkdir(parents=True)
            shutil.copyfile(binary, destination)
            destination.chmod(0o755)

        try:
            validate_workspace(staging_root)
        except PolicyViolation as error:
            raise ArchiveError(
                f"assembled plugin violates package policy: {error}"
            ) from error
        entries = collect_entries(staged_plugin, workspace_root, declared_executables)

    require(
        {entry["path"] for entry in entries} == set(policy["allowedPackageFiles"]),
        "assembled plugin file set differs from package policy",
    )
    write_zip(output, entries)
    archive_contents = output.read_bytes()
    archive_hash = sha256(archive_contents)
    manifest = {
        "schemaVersion": 1,
        "product": "Lili",
        "component": "plugin",
        "version": version,
        "archive": output.name,
        "archiveSize": len(archive_contents),
        "archiveSha256": archive_hash,
        "compression": "deflate-9",
        "supplyChain": {
            "fileName": supply_chain_path.name,
            "size": len(supply_chain_contents),
            "sha256": sha256(supply_chain_contents),
        },
        "entries": [
            {
                "path": entry["path"],
                "mode": f"{entry['mode']:04o}",
                "size": entry["size"],
                "sha256": entry["sha256"],
            }
            for entry in entries
        ],
        "forwarders": [
            {
                "platform": target,
                "signatureKind": forwarders[target][1]["signatureKind"],
                "signatureVerifier": forwarders[target][1]["signatureVerifier"],
                "signatureStatus": forwarders[target][1]["signatureStatus"],
                "sha256": forwarders[target][1]["sha256"],
            }
            for target in sorted(forwarders)
        ],
    }
    manifest_path = output.with_suffix(".manifest.json")
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    output.with_suffix(output.suffix + ".sha256").write_text(
        f"{archive_hash}  {output.name}\n",
        encoding="utf-8",
    )
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build the deterministic universal Lili plugin"
    )
    parser.add_argument("--forwarders", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--supply-chain", required=True, type=Path)
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
    )
    arguments = parser.parse_args()
    try:
        manifest = build_archive(
            arguments.workspace_root,
            arguments.forwarders,
            arguments.output,
            arguments.supply_chain,
        )
    except (ArchiveError, OSError, KeyError, TypeError, ValueError) as error:
        print(f"plugin archive build failed: {error}")
        return 1
    print(
        json.dumps(
            {
                "archive": str(arguments.output),
                "sha256": manifest["archiveSha256"],
            },
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
