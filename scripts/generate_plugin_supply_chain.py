#!/usr/bin/env python3

import argparse
import datetime
import hashlib
import json
import subprocess
import tomllib
from pathlib import Path


MAX_COMMAND_OUTPUT_BYTES = 32 * 1024 * 1024
COMMAND_TIMEOUT_SECONDS = 180


class SupplyChainError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SupplyChainError(message)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(arguments: list[str], workspace_root: Path, label: str) -> subprocess.CompletedProcess[bytes]:
    try:
        result = subprocess.run(
            arguments,
            cwd=workspace_root,
            capture_output=True,
            check=False,
            timeout=COMMAND_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise SupplyChainError(f"{label} could not execute") from error
    require(
        len(result.stdout) <= MAX_COMMAND_OUTPUT_BYTES
        and len(result.stderr) <= MAX_COMMAND_OUTPUT_BYTES,
        f"{label} output exceeded its bound",
    )
    require(
        result.returncode == 0,
        f"{label} failed: {result.stderr.decode('utf-8', errors='replace').strip()}",
    )
    return result


def parse_json(payload: bytes, label: str) -> dict:
    try:
        value = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SupplyChainError(f"{label} returned invalid JSON") from error
    require(isinstance(value, dict), f"{label} JSON root must be an object")
    return value


def production_package_ids(metadata: dict) -> set[str]:
    packages = metadata.get("packages")
    resolve = metadata.get("resolve")
    require(isinstance(packages, list) and isinstance(resolve, dict), "cargo metadata is incomplete")
    nodes = resolve.get("nodes")
    require(isinstance(nodes, list), "cargo metadata omitted resolve nodes")
    package_by_id = {
        package["id"]: package
        for package in packages
        if isinstance(package, dict) and isinstance(package.get("id"), str)
    }
    roots = [
        package["id"]
        for package in packages
        if package.get("name") == "lili"
        and str(package.get("manifest_path", "")).replace("\\", "/").endswith("/lili/Cargo.toml")
    ]
    require(len(roots) == 1, "cargo metadata did not identify one lili package")
    node_by_id = {
        node["id"]: node
        for node in nodes
        if isinstance(node, dict) and isinstance(node.get("id"), str)
    }
    selected = set()
    pending = roots
    while pending:
        package_id = pending.pop()
        if package_id in selected:
            continue
        require(package_id in package_by_id and package_id in node_by_id, "dependency graph is incomplete")
        selected.add(package_id)
        for dependency in node_by_id[package_id].get("deps", []):
            kinds = dependency.get("dep_kinds", [])
            if any(kind.get("kind") != "dev" for kind in kinds):
                pending.append(dependency["pkg"])
    return selected


def lock_checksums(lockfile: Path) -> dict[tuple[str, str, str | None], str | None]:
    with lockfile.open("rb") as stream:
        document = tomllib.load(stream)
    return {
        (package["name"], package["version"], package.get("source")): package.get("checksum")
        for package in document.get("package", [])
    }


def dependency_inventory(metadata: dict, lockfile: Path) -> list[dict]:
    selected = production_package_ids(metadata)
    checksums = lock_checksums(lockfile)
    packages = []
    for package in metadata["packages"]:
        if package["id"] not in selected:
            continue
        license_expression = package.get("license")
        require(
            isinstance(license_expression, str) and license_expression,
            f"dependency has no SPDX license expression: {package.get('name')}",
        )
        source = package.get("source")
        checksum = checksums.get((package["name"], package["version"], source))
        if source is not None and source.startswith("registry+"):
            require(isinstance(checksum, str) and len(checksum) == 64, f"registry checksum is missing: {package['name']}")
        packages.append(
            {
                "name": package["name"],
                "version": package["version"],
                "source": source or "workspace",
                "checksum": checksum,
                "license": license_expression,
            }
        )
    packages.sort(key=lambda package: (package["name"], package["version"], package["source"]))
    require(packages, "dependency inventory is empty")
    return packages


def audit_summary(audit: dict) -> dict:
    vulnerabilities = audit.get("vulnerabilities")
    database = audit.get("database")
    warnings = audit.get("warnings")
    require(isinstance(vulnerabilities, dict), "cargo audit omitted vulnerabilities")
    require(vulnerabilities.get("found") is False and vulnerabilities.get("count") == 0, "vulnerability scan found an advisory")
    require(isinstance(database, dict), "cargo audit omitted database evidence")
    require(isinstance(warnings, dict), "cargo audit omitted warning evidence")
    warning_entries = []
    for category, entries in sorted(warnings.items()):
        require(isinstance(entries, list), "cargo audit warning category is invalid")
        for entry in entries:
            advisory = entry.get("advisory", {})
            package = entry.get("package", {})
            warning_entries.append(
                {
                    "category": category,
                    "advisoryId": advisory.get("id"),
                    "package": package.get("name"),
                    "version": package.get("version"),
                }
            )
    require(
        all(
            isinstance(entry["advisoryId"], str)
            and isinstance(entry["package"], str)
            and isinstance(entry["version"], str)
            for entry in warning_entries
        ),
        "cargo audit warning evidence is incomplete",
    )
    return {
        "result": "passed" if not warning_entries else "passed-with-informational-warnings",
        "vulnerabilityCount": 0,
        "databaseAdvisoryCount": database.get("advisory-count"),
        "databaseCommit": database.get("last-commit"),
        "databaseUpdatedAt": database.get("last-updated"),
        "informationalWarningCount": len(warning_entries),
        "informationalWarnings": warning_entries,
    }


def tool_version(command: list[str], workspace_root: Path, label: str) -> str:
    result = run(command, workspace_root, label)
    version = result.stdout.decode("utf-8", errors="strict").strip()
    require(version and "\n" not in version, f"{label} version output is invalid")
    return version


def build_evidence(
    workspace_root: Path,
    metadata: dict,
    audit: dict,
    cargo_audit_version: str,
    cargo_deny_version: str,
    generated_at: str,
) -> dict:
    lockfile = workspace_root / "Cargo.lock"
    cargo_manifest = workspace_root / "Cargo.toml"
    deny_policy = workspace_root / "deny.toml"
    for path in (lockfile, cargo_manifest, deny_policy):
        require(path.is_file(), f"supply-chain input is missing: {path.name}")
    with cargo_manifest.open("rb") as stream:
        version = tomllib.load(stream)["workspace"]["package"]["version"]
    packages = dependency_inventory(metadata, lockfile)
    return {
        "schemaVersion": 1,
        "product": "Lili",
        "component": "plugin",
        "version": version,
        "generatedAt": generated_at,
        "lockfile": {
            "path": "Cargo.lock",
            "sha256": sha256(lockfile),
        },
        "dependencyInventory": {
            "scope": "non-development transitive closure of the lili package across declared targets",
            "packageCount": len(packages),
            "packages": packages,
        },
        "licensePolicy": {
            "result": "passed",
            "tool": cargo_deny_version,
            "configuration": "deny.toml",
            "configurationSha256": sha256(deny_policy),
        },
        "vulnerabilityScan": {
            "tool": cargo_audit_version,
            **audit_summary(audit),
        },
    }


def generate(workspace_root: Path, output: Path) -> dict:
    workspace_root = workspace_root.resolve()
    metadata = parse_json(
        run(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            workspace_root,
            "cargo metadata",
        ).stdout,
        "cargo metadata",
    )
    audit = parse_json(
        run(["cargo", "audit", "--json"], workspace_root, "cargo audit").stdout,
        "cargo audit",
    )
    run(["cargo", "deny", "--locked", "check", "licenses"], workspace_root, "cargo deny")
    evidence = build_evidence(
        workspace_root,
        metadata,
        audit,
        tool_version(["cargo-audit", "--version"], workspace_root, "cargo audit"),
        tool_version(["cargo-deny", "--version"], workspace_root, "cargo deny"),
        datetime.datetime.now(datetime.timezone.utc).isoformat().replace("+00:00", "Z"),
    )
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return evidence


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate Lili plugin supply-chain evidence")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
    )
    arguments = parser.parse_args()
    try:
        evidence = generate(arguments.workspace_root, arguments.output)
    except (SupplyChainError, OSError, KeyError, TypeError, ValueError) as error:
        print(f"plugin supply-chain generation failed: {error}")
        return 1
    print(
        json.dumps(
            {
                "output": str(arguments.output),
                "packageCount": evidence["dependencyInventory"]["packageCount"],
                "vulnerabilityCount": evidence["vulnerabilityScan"]["vulnerabilityCount"],
            },
            separators=(",", ":"),
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
