#!/usr/bin/env python3

import argparse
import hashlib
import json
import re
import stat
import zipfile
from pathlib import Path, PurePosixPath


MAX_ARCHIVE_BYTES = 512 * 1024 * 1024
MAX_ENTRY_BYTES = 100 * 1024 * 1024
MAX_ENTRIES = 5000
URL_PATTERN = re.compile(rb"https?://[A-Za-z0-9._~:/?#\[\]@!$&'*+=%()-]+")
SECRET_PATTERNS = {
    "private key": re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "OpenAI-style secret": re.compile(rb"\bsk-[A-Za-z0-9_-]{20,}\b"),
    "GitHub token": re.compile(rb"\bgh[opsu]_[A-Za-z0-9]{30,}\b"),
    "AWS access key": re.compile(rb"\bAKIA[0-9A-Z]{16}\b"),
}
DEVELOPMENT_PATH_PATTERNS = {
    "macOS development path": re.compile(rb"/Users/[^/\x00\s]+/(?:Documents|Projects|src|work|\.codex)/"),
    "GitHub Linux workspace": re.compile(rb"/home/runner/work/"),
    "GitHub Windows workspace": re.compile(rb"[A-Za-z]:\\(?:a|Users\\runneradmin)\\"),
    "temporary build path": re.compile(rb"/private/tmp/"),
}
PRIVATE_FIXTURE_MARKERS = (
    b"tests/fixtures",
    b"openai-plugin-contracts",
    b"preflight-marketplace",
)
ALLOWED_NON_RUNTIME_URLS = {
    "http://www.apple.com/DTDs/PropertyList-1.0.dtd",
    "http://schemas.microsoft.com/SMI/2005/WindowsSettings",
    "http://schemas.microsoft.com/SMI/2011/WindowsSettings",
    "http://schemas.microsoft.com/SMI/2016/WindowsSettings",
    "http://schemas.microsoft.com/SMI/2017/WindowsSettings",
    "http://schemas.microsoft.com/SMI/2019/WindowsSettings",
    "http://schemas.microsoft.com/SMI/2020/WindowsSettings",
    "http://www.w3.org/2001/XMLSchema-instance",
}
SIGNATURE_EVIDENCE = {
    "arm64-apple-darwin": ("codesign --verify --strict", {"verified", "unsigned-allowed"}),
    "x86_64-unknown-linux-gnu": ("ELF format and SHA-256 integrity", {"not-applicable"}),
    "x86_64-pc-windows-msvc": ("Get-AuthenticodeSignature", {"verified", "unsigned-allowed"}),
}


class InspectionError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise InspectionError(message)


def digest(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def load_json(path: Path, label: str) -> dict:
    try:
        payload = path.read_bytes()
        value = json.loads(payload)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise InspectionError(f"{label} is invalid") from error
    require(isinstance(value, dict), f"{label} root must be an object")
    return value


def collect_urls(value: object) -> set[str]:
    urls = set()
    if isinstance(value, dict):
        for child in value.values():
            urls.update(collect_urls(child))
    elif isinstance(value, list):
        for child in value:
            urls.update(collect_urls(child))
    elif isinstance(value, str) and re.fullmatch(r"https?://[^\s]+", value):
        urls.add(value)
    return urls


def scan_entry(path: str, contents: bytes, allowed_urls: set[str]) -> None:
    for label, pattern in SECRET_PATTERNS.items():
        require(pattern.search(contents) is None, f"{label} found in final package: {path}")
    for label, pattern in DEVELOPMENT_PATH_PATTERNS.items():
        require(pattern.search(contents) is None, f"{label} found in final package: {path}")
    folded = contents.lower()
    for marker in PRIVATE_FIXTURE_MARKERS:
        require(marker not in folded, f"private fixture marker found in final package: {path}")
    observed_urls = {
        match.group().decode("ascii", errors="strict") for match in URL_PATTERN.finditer(contents)
    }
    undeclared = observed_urls - allowed_urls - ALLOWED_NON_RUNTIME_URLS
    require(not undeclared, f"undeclared network URL found in final package: {path}: {sorted(undeclared)}")


def validate_signatures(manifest: dict) -> None:
    forwarders = manifest.get("forwarders")
    require(isinstance(forwarders, list), "archive manifest omitted forwarder evidence")
    require(
        {forwarder.get("platform") for forwarder in forwarders} == set(SIGNATURE_EVIDENCE),
        "archive manifest forwarder targets drifted",
    )
    for forwarder in forwarders:
        platform = forwarder["platform"]
        verifier, statuses = SIGNATURE_EVIDENCE[platform]
        require(forwarder.get("signatureVerifier") == verifier, f"signature verifier drifted: {platform}")
        require(forwarder.get("signatureStatus") in statuses, f"signature status drifted: {platform}")
        if forwarder.get("signatureKind") == "signed":
            require(forwarder["signatureStatus"] == "verified", f"signed binary lacks verification: {platform}")
        else:
            require(forwarder.get("signatureKind") == "platform-standard", f"signature kind drifted: {platform}")
            require(forwarder["signatureStatus"] != "verified", f"unsigned binary claims verification: {platform}")
        require(re.fullmatch(r"[0-9a-f]{64}", str(forwarder.get("sha256"))) is not None, f"forwarder hash is invalid: {platform}")


def validate_supply_chain(supply_chain: dict, version: str) -> None:
    require(supply_chain.get("schemaVersion") == 1, "unsupported supply-chain evidence schema")
    require(supply_chain.get("component") == "plugin", "supply-chain component drifted")
    require(supply_chain.get("version") == version, "supply-chain version drifted")
    lockfile = supply_chain.get("lockfile", {})
    require(re.fullmatch(r"[0-9a-f]{64}", str(lockfile.get("sha256"))) is not None, "lockfile hash is invalid")
    inventory = supply_chain.get("dependencyInventory", {})
    packages = inventory.get("packages")
    require(isinstance(packages, list) and packages, "dependency inventory is empty")
    require(inventory.get("packageCount") == len(packages), "dependency inventory count drifted")
    for package in packages:
        require(
            all(isinstance(package.get(field), str) and package[field] for field in ("name", "version", "source", "license")),
            "dependency inventory entry is incomplete",
        )
        checksum = package.get("checksum")
        require(
            checksum is None or re.fullmatch(r"[0-9a-f]{64}", str(checksum)) is not None,
            "dependency checksum is invalid",
        )
    license_policy = supply_chain.get("licensePolicy", {})
    require(license_policy.get("result") == "passed", "license policy did not pass")
    require(re.fullmatch(r"[0-9a-f]{64}", str(license_policy.get("configurationSha256"))) is not None, "license policy hash is invalid")
    scan = supply_chain.get("vulnerabilityScan", {})
    require(scan.get("vulnerabilityCount") == 0, "vulnerability scan found an advisory")
    require(scan.get("result") in {"passed", "passed-with-informational-warnings"}, "vulnerability scan did not pass")
    require(isinstance(scan.get("databaseCommit"), str) and scan["databaseCommit"], "vulnerability database evidence is missing")


def inspect_release(
    archive_path: Path,
    manifest_path: Path,
    checksum_path: Path,
    supply_chain_path: Path,
) -> dict:
    for path, label in (
        (archive_path, "plugin archive"),
        (manifest_path, "archive manifest"),
        (checksum_path, "archive checksum"),
        (supply_chain_path, "supply-chain evidence"),
    ):
        require(path.is_absolute() and path.is_file(), f"{label} must be an absolute file")
    require(archive_path.stat().st_size <= MAX_ARCHIVE_BYTES, "plugin archive exceeds its size bound")
    archive_contents = archive_path.read_bytes()
    archive_hash = digest(archive_contents)
    checksum = checksum_path.read_text(encoding="utf-8")
    require(checksum == f"{archive_hash}  {archive_path.name}\n", "archive checksum file drifted")
    manifest = load_json(manifest_path, "archive manifest")
    supply_chain = load_json(supply_chain_path, "supply-chain evidence")
    supply_chain_contents = supply_chain_path.read_bytes()
    require(manifest.get("component") == "plugin", "archive component drifted")
    require(manifest.get("archive") == archive_path.name, "archive name drifted")
    require(manifest.get("archiveSize") == len(archive_contents), "archive size drifted")
    require(manifest.get("archiveSha256") == archive_hash, "archive manifest checksum drifted")
    supply_chain_reference = manifest.get("supplyChain", {})
    require(
        supply_chain_reference.get("fileName") == supply_chain_path.name
        and supply_chain_reference.get("size") == len(supply_chain_contents)
        and supply_chain_reference.get("sha256") == digest(supply_chain_contents),
        "archive manifest supply-chain reference drifted",
    )
    version = manifest.get("version")
    require(isinstance(version, str) and version, "archive version is invalid")
    validate_signatures(manifest)
    validate_supply_chain(supply_chain, version)

    manifest_entries = manifest.get("entries")
    require(isinstance(manifest_entries, list) and manifest_entries, "archive manifest entries are empty")
    expected = {entry.get("path"): entry for entry in manifest_entries}
    require(len(expected) == len(manifest_entries), "archive manifest contains duplicate entries")
    with zipfile.ZipFile(archive_path) as archive:
        entries = archive.infolist()
        require(0 < len(entries) <= MAX_ENTRIES, "archive entry count is invalid")
        require(len(entries) == len(expected), "archive contains duplicate entries")
        require(
            sum(entry.file_size for entry in entries) <= MAX_ARCHIVE_BYTES,
            "archive expands beyond its size bound",
        )
        require(archive.testzip() is None, "archive integrity check failed")
        require({entry.filename for entry in entries} == set(expected), "archive entries differ from manifest")
        plugin_manifest = json.loads(archive.read(".codex-plugin/plugin.json"))
        allowed_urls = collect_urls(plugin_manifest)
        for entry in entries:
            relative = PurePosixPath(entry.filename)
            require(
                entry.filename == relative.as_posix()
                and relative.parts
                and not relative.is_absolute()
                and ".." not in relative.parts,
                f"unsafe archive path: {entry.filename}",
            )
            mode = entry.external_attr >> 16
            require(not entry.is_dir() and not stat.S_ISLNK(mode), f"unsafe archive entry: {entry.filename}")
            require(entry.file_size <= MAX_ENTRY_BYTES, f"archive entry is too large: {entry.filename}")
            contents = archive.read(entry)
            evidence = expected[entry.filename]
            require(evidence.get("size") == len(contents), f"entry size drifted: {entry.filename}")
            require(evidence.get("sha256") == digest(contents), f"entry checksum drifted: {entry.filename}")
            require(evidence.get("mode") == f"{stat.S_IMODE(mode):04o}", f"entry mode drifted: {entry.filename}")
            scan_entry(entry.filename, contents, allowed_urls)

    return {
        "schemaVersion": 1,
        "archive": archive_path.name,
        "version": version,
        "sha256": archive_hash,
        "entryCount": len(manifest_entries),
        "dependencyCount": supply_chain["dependencyInventory"]["packageCount"],
        "vulnerabilityCount": 0,
        "result": "passed",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Inspect the final Lili plugin release")
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--checksum", required=True, type=Path)
    parser.add_argument("--supply-chain", required=True, type=Path)
    arguments = parser.parse_args()
    try:
        result = inspect_release(
            arguments.archive.resolve(),
            arguments.manifest.resolve(),
            arguments.checksum.resolve(),
            arguments.supply_chain.resolve(),
        )
    except (InspectionError, OSError, KeyError, TypeError, ValueError, zipfile.BadZipFile) as error:
        print(f"plugin release inspection failed: {error}")
        return 1
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
