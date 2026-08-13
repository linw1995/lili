#!/usr/bin/env python3

import hashlib
import json
import struct
import subprocess
from pathlib import Path


PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
PNG_RGBA_COLOR_TYPE = 6


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def resolve_confined(root: Path, relative: str) -> Path:
    if not relative.startswith("./assets/"):
        raise ValueError(f"asset path must start with ./assets/: {relative}")
    unresolved = root / relative[2:]
    if unresolved.is_symlink():
        raise ValueError(f"asset must not be a symbolic link: {relative}")
    candidate = unresolved.resolve(strict=True)
    if not candidate.is_relative_to((root / "assets").resolve()):
        raise ValueError(f"asset path escapes the plugin asset directory: {relative}")
    if not candidate.is_file():
        raise ValueError(f"asset must be a regular file: {relative}")
    return candidate


def read_png(path: Path) -> tuple[bytes, int, int, int]:
    data = path.read_bytes()
    if data[:8] != PNG_SIGNATURE or data[12:16] != b"IHDR":
        raise ValueError(f"asset is not a PNG with an IHDR header: {path}")
    width, height, _, color_type, _, _, _ = struct.unpack(">IIBBBBB", data[16:29])
    return data, width, height, color_type


def manifest_value(manifest: dict, dotted_field: str) -> object:
    value: object = manifest
    for part in dotted_field.split("."):
        if not isinstance(value, dict) or part not in value:
            raise ValueError(f"manifest field is missing: {dotted_field}")
        value = value[part]
    return value


def historical_source(workspace_root: Path, commit: str, source_path: str) -> bytes:
    return subprocess.check_output(
        ["git", "show", f"{commit}:{source_path}"],
        cwd=workspace_root,
    )


def main() -> int:
    workspace_root = Path(__file__).resolve().parent.parent
    plugin_root = workspace_root / "plugins" / "lili"
    manifest = json.loads(
        (plugin_root / ".codex-plugin" / "plugin.json").read_text(encoding="utf-8")
    )
    registry = json.loads(
        (workspace_root / "marketplace" / "lili" / "assets.json").read_text(
            encoding="utf-8"
        )
    )

    license_metadata = registry["license"]
    if manifest["license"] != license_metadata["spdx"]:
        raise ValueError("manifest and asset licenses differ")
    for key in ("licenseFile", "noticeFile"):
        legal_path = workspace_root / license_metadata[key]
        if not legal_path.is_file():
            raise ValueError(f"missing reviewed legal file: {legal_path}")
    if license_metadata["copyright"] not in (
        workspace_root / license_metadata["noticeFile"]
    ).read_text(encoding="utf-8"):
        raise ValueError("asset copyright is missing from NOTICE")

    declared_paths = set()
    for asset in registry["assets"]:
        relative = asset["path"]
        target = resolve_confined(plugin_root, relative)
        if manifest_value(manifest, asset["manifestField"]) != relative:
            raise ValueError(f"manifest asset reference drifted: {asset['manifestField']}")

        unresolved_source = workspace_root / asset["sourcePath"]
        if unresolved_source.is_symlink():
            raise ValueError(f"asset source must not be a symbolic link: {asset['sourcePath']}")
        source = unresolved_source.resolve(strict=True)
        if not source.is_relative_to(workspace_root.resolve()) or not source.is_file():
            raise ValueError(f"invalid asset source: {asset['sourcePath']}")

        data, width, height, color_type = read_png(target)
        if (width, height) != (asset["width"], asset["height"]):
            raise ValueError(f"asset dimensions drifted: {relative}")
        if asset["colorType"] != "rgba" or color_type != PNG_RGBA_COLOR_TYPE:
            raise ValueError(f"asset must use RGBA color: {relative}")
        if asset["mediaType"] != "image/png":
            raise ValueError(f"unexpected media type: {relative}")
        if sha256(data) != asset["sha256"]:
            raise ValueError(f"asset checksum drifted: {relative}")
        if source.read_bytes() != data:
            raise ValueError(f"packaged asset differs from reviewed source: {relative}")
        if sha256(
            historical_source(
                workspace_root,
                asset["introducedByCommit"],
                asset["sourcePath"],
            )
        ) != asset["sha256"]:
            raise ValueError(f"asset provenance drifted: {relative}")
        declared_paths.add(target)

    packaged_assets = {path.resolve() for path in (plugin_root / "assets").iterdir()}
    if packaged_assets != declared_paths:
        raise ValueError("plugin assets differ from the reviewed registry")
    if registry["screenshots"]["included"] is not False:
        raise ValueError("UI-less plugin must not include screenshots")
    if "screenshots" in manifest["interface"]:
        raise ValueError("UI-less plugin must omit interface.screenshots")
    if any("screenshot" in path.name.lower() for path in plugin_root.rglob("*")):
        raise ValueError("UI-less plugin must not package screenshot files")

    print(f"validated {len(declared_paths)} production plugin assets")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
