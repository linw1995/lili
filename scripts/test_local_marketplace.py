#!/usr/bin/env python3

import argparse
import json
import os
import re
import shutil
import stat
import subprocess
import tempfile
import zipfile
from pathlib import Path, PurePosixPath


MAX_COMMAND_OUTPUT_BYTES = 1024 * 1024
MAX_ARCHIVE_ENTRIES = 5000
MAX_ARCHIVE_ENTRY_BYTES = 100 * 1024 * 1024
MAX_ARCHIVE_BYTES = 512 * 1024 * 1024


class MarketplaceRoundTripError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise MarketplaceRoundTripError(message)


def load_json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise MarketplaceRoundTripError(f"invalid JSON file: {path}") from error
    require(isinstance(value, dict), f"JSON root must be an object: {path}")
    return value


def archive_entries(archive: zipfile.ZipFile) -> list[zipfile.ZipInfo]:
    entries = archive.infolist()
    require(0 < len(entries) <= MAX_ARCHIVE_ENTRIES, "plugin archive entry count is invalid")
    total_size = 0
    names = set()
    for entry in entries:
        relative = PurePosixPath(entry.filename)
        require(
            entry.filename == relative.as_posix()
            and relative.parts
            and not relative.is_absolute()
            and ".." not in relative.parts,
            f"plugin archive path is unsafe: {entry.filename}",
        )
        require(not entry.is_dir(), f"plugin archive contains a directory entry: {entry.filename}")
        require(entry.filename not in names, f"plugin archive entry is duplicated: {entry.filename}")
        require(entry.file_size <= MAX_ARCHIVE_ENTRY_BYTES, f"plugin archive entry is too large: {entry.filename}")
        mode = entry.external_attr >> 16
        require(not stat.S_ISLNK(mode), f"plugin archive contains a symbolic link: {entry.filename}")
        names.add(entry.filename)
        total_size += entry.file_size
    require(total_size <= MAX_ARCHIVE_BYTES, "plugin archive expands beyond the acceptance limit")
    require(".codex-plugin/plugin.json" in names, "plugin archive manifest is missing")
    return entries


def extract_archive(archive_path: Path, destination: Path) -> dict:
    require(archive_path.is_absolute() and archive_path.is_file(), "plugin archive path must be an absolute file")
    require(not destination.exists(), "plugin archive destination already exists")
    destination.mkdir(parents=True)
    with zipfile.ZipFile(archive_path) as archive:
        entries = archive_entries(archive)
        for entry in entries:
            target = destination.joinpath(*PurePosixPath(entry.filename).parts)
            require(target.resolve().is_relative_to(destination.resolve()), "plugin archive path escaped destination")
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(entry) as source, target.open("xb") as output:
                shutil.copyfileobj(source, output)
            mode = entry.external_attr >> 16 & 0o777
            target.chmod(mode or 0o644)
    return load_json(destination / ".codex-plugin" / "plugin.json")


def next_patch_version(version: str) -> str:
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", version)
    require(match is not None, "plugin version is not a three-part semantic version")
    major, minor, patch = (int(value) for value in match.groups())
    return f"{major}.{minor}.{patch + 1}"


class CodexRunner:
    def __init__(self, executable: Path, codex_home: Path, expected_version: str):
        require(executable.is_absolute() and executable.is_file(), "Codex executable must be an absolute file")
        self.executable = executable
        self.codex_home = codex_home
        self.expected_version = expected_version
        self.environment = {
            "CODEX_HOME": str(codex_home),
            "HOME": str(codex_home.parent / "home"),
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        }
        self.codex_home.mkdir(parents=True)
        Path(self.environment["HOME"]).mkdir(parents=True)

    def verify_version(self) -> None:
        output = self._run(["--version"], json_output=False)
        match = re.fullmatch(r"codex-cli (\S+)\n?", output)
        require(match is not None, "Codex version output is invalid")
        require(match.group(1) == self.expected_version, "Codex version differs from the reviewed contract")

    def json(self, arguments: list[str]) -> dict:
        output = self._run([*arguments, "--json"], json_output=True)
        try:
            value = json.loads(output)
        except json.JSONDecodeError as error:
            raise MarketplaceRoundTripError(f"Codex returned invalid JSON for {' '.join(arguments)}") from error
        require(isinstance(value, dict), f"Codex JSON root is not an object for {' '.join(arguments)}")
        return value

    def _run(self, arguments: list[str], json_output: bool) -> str:
        try:
            result = subprocess.run(
                [str(self.executable), *arguments],
                capture_output=True,
                check=False,
                env=self.environment,
                text=False,
                timeout=15,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise MarketplaceRoundTripError(f"Codex command failed to execute: {' '.join(arguments)}") from error
        require(
            len(result.stdout) <= MAX_COMMAND_OUTPUT_BYTES and len(result.stderr) <= MAX_COMMAND_OUTPUT_BYTES,
            f"Codex command output exceeded its bound: {' '.join(arguments)}",
        )
        stderr = result.stderr.decode("utf-8", errors="replace")
        require(result.returncode == 0, f"Codex command failed: {' '.join(arguments)}: {stderr.strip()}")
        stdout = result.stdout.decode("utf-8", errors="strict")
        if json_output:
            require(stdout.strip().startswith("{"), f"Codex command emitted non-JSON output: {' '.join(arguments)}")
        return stdout


def plugin_state(document: dict, selector: str, installed: bool, version: str) -> dict:
    collection = document.get("installed" if installed else "available")
    require(isinstance(collection, list), "Codex plugin list omitted the expected collection")
    matches = [item for item in collection if isinstance(item, dict) and item.get("pluginId") == selector]
    require(len(matches) == 1, f"Codex plugin list did not contain exactly one {selector} entry")
    item = matches[0]
    require(item.get("version") == version, f"Codex plugin version drifted from catalog: {version}")
    require(item.get("installed") is installed, "Codex installed state drifted")
    require(item.get("enabled") is installed, "Codex enabled state drifted")
    return item


def replace_catalog_plugin(catalog_plugin: Path, snapshot: Path) -> None:
    require(snapshot.is_dir(), "catalog snapshot is missing")
    if catalog_plugin.exists():
        shutil.rmtree(catalog_plugin)
    shutil.copytree(snapshot, catalog_plugin)


def run_round_trip(workspace_root: Path, archive_path: Path, codex_executable: Path) -> dict:
    workspace_root = workspace_root.resolve()
    archive_path = archive_path.resolve()
    lifecycle = load_json(workspace_root / "marketplace" / "local" / "lifecycle.json")
    template_root = workspace_root / "marketplace" / "local"
    expected_codex_version = lifecycle["codexVersion"]
    marketplace_name = lifecycle["marketplaceName"]
    selector = lifecycle["pluginSelector"]

    with tempfile.TemporaryDirectory(prefix="lili-marketplace-roundtrip-") as temporary_directory:
        temporary_root = Path(temporary_directory)
        catalog_root = temporary_root / "catalog"
        shutil.copytree(template_root, catalog_root)
        catalog_plugin = catalog_root / lifecycle["archiveDestination"]
        original_manifest = extract_archive(archive_path, catalog_plugin)
        original_version = original_manifest["version"]

        original_snapshot = temporary_root / "snapshots" / original_version
        original_snapshot.parent.mkdir()
        shutil.copytree(catalog_plugin, original_snapshot)
        updated_version = next_patch_version(original_version)
        updated_snapshot = temporary_root / "snapshots" / updated_version
        shutil.copytree(catalog_plugin, updated_snapshot)
        updated_manifest_path = updated_snapshot / ".codex-plugin" / "plugin.json"
        updated_manifest = load_json(updated_manifest_path)
        updated_manifest["version"] = updated_version
        updated_manifest_path.write_text(
            json.dumps(updated_manifest, indent=2, separators=(",", ": ")) + "\n",
            encoding="utf-8",
        )

        runner = CodexRunner(codex_executable.resolve(), temporary_root / "codex-home", expected_codex_version)
        runner.verify_version()
        added_marketplace = runner.json(["plugin", "marketplace", "add", str(catalog_root)])
        require(added_marketplace.get("marketplaceName") == marketplace_name, "marketplace name drifted")
        marketplaces = runner.json(["plugin", "marketplace", "list"])
        require(
            [item.get("name") for item in marketplaces.get("marketplaces", [])] == [marketplace_name],
            "isolated marketplace list drifted",
        )

        available = runner.json(["plugin", "list", "--marketplace", marketplace_name, "--available"])
        plugin_state(available, selector, False, original_version)

        installed = runner.json(["plugin", "add", selector])
        require(installed.get("version") == original_version, "initial install version drifted")
        plugin_state(
            runner.json(["plugin", "list", "--marketplace", marketplace_name, "--available"]),
            selector,
            True,
            original_version,
        )

        runner.json(["plugin", "remove", selector])
        plugin_state(
            runner.json(["plugin", "list", "--marketplace", marketplace_name, "--available"]),
            selector,
            False,
            original_version,
        )
        runner.json(["plugin", "add", selector])
        plugin_state(
            runner.json(["plugin", "list", "--marketplace", marketplace_name, "--available"]),
            selector,
            True,
            original_version,
        )

        replace_catalog_plugin(catalog_plugin, updated_snapshot)
        runner.json(["plugin", "add", selector])
        plugin_state(
            runner.json(["plugin", "list", "--marketplace", marketplace_name, "--available"]),
            selector,
            True,
            updated_version,
        )

        replace_catalog_plugin(catalog_plugin, original_snapshot)
        runner.json(["plugin", "add", selector])
        plugin_state(
            runner.json(["plugin", "list", "--marketplace", marketplace_name, "--available"]),
            selector,
            True,
            original_version,
        )

        runner.json(["plugin", "remove", selector])
        plugin_state(
            runner.json(["plugin", "list", "--marketplace", marketplace_name, "--available"]),
            selector,
            False,
            original_version,
        )
        removed_marketplace = runner.json(["plugin", "marketplace", "remove", marketplace_name])
        require(removed_marketplace.get("marketplaceName") == marketplace_name, "marketplace removal drifted")
        require(
            runner.json(["plugin", "marketplace", "list"]).get("marketplaces") == [],
            "marketplace remained configured after removal",
        )
        final_plugins = runner.json(["plugin", "list", "--available"])
        require(final_plugins.get("installed") == [] and final_plugins.get("available") == [], "plugin remained after cleanup")

    return {
        "schemaVersion": 1,
        "codexVersion": expected_codex_version,
        "marketplace": marketplace_name,
        "plugin": selector,
        "releaseVersion": original_version,
        "derivedUpdateVersion": updated_version,
        "operations": [
            "catalog-add",
            "list-available",
            "install-and-enable",
            "disable-by-remove",
            "enable-by-add",
            "update-by-add",
            "rollback-by-add",
            "plugin-remove",
            "catalog-remove",
        ],
        "result": "passed",
    }


def resolve_executable(value: str) -> Path:
    candidate = Path(value)
    if candidate.is_absolute():
        return candidate
    resolved = shutil.which(value)
    require(resolved is not None, f"executable was not found on PATH: {value}")
    return Path(resolved)


def main() -> int:
    parser = argparse.ArgumentParser(description="Run the Lili local Marketplace round trip")
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--codex", default="codex")
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
    )
    arguments = parser.parse_args()
    try:
        result = run_round_trip(
            arguments.workspace_root,
            arguments.archive,
            resolve_executable(arguments.codex),
        )
    except (MarketplaceRoundTripError, OSError, KeyError, TypeError, ValueError, zipfile.BadZipFile) as error:
        print(f"local Marketplace round trip failed: {error}")
        return 1
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
