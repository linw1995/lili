#!/usr/bin/env python3

import argparse
import json
import re
import stat
import tomllib
from pathlib import Path, PurePosixPath
from typing import Iterator


EXPECTED_HOOK_EVENTS = {
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "Stop",
    "SessionEnd",
}
HANDLER_FIELDS = {"type", "command", "commandWindows", "timeout", "statusMessage", "async"}
GROUP_FIELDS = {"matcher", "hooks"}
TRUSTED_WINDOWS_POWERSHELL = '"C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"'
EXECUTABLE_SUFFIXES = {".bat", ".cmd", ".exe", ".ps1", ".sh"}
EXECUTABLE_MAGICS = (
    b"\x7fELF",
    b"MZ",
    b"\xcf\xfa\xed\xfe",
    b"\xce\xfa\xed\xfe",
    b"\xfe\xed\xfa\xcf",
    b"\xfe\xed\xfa\xce",
)


class PolicyViolation(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise PolicyViolation(message)


def load_json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PolicyViolation(f"invalid JSON file: {path}") from error
    require(isinstance(value, dict), f"JSON root must be an object: {path}")
    return value


def walk_values(value: object, path: tuple[str, ...] = ()) -> Iterator[tuple[tuple[str, ...], object]]:
    yield path, value
    if isinstance(value, dict):
        for key, child in value.items():
            yield from walk_values(child, (*path, key))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from walk_values(child, (*path, str(index)))


def resolve_manifest_path(plugin_root: Path, value: object, field: str) -> Path:
    require(isinstance(value, str), f"manifest path must be a string: {field}")
    require(value.startswith("./"), f"manifest path must start with ./: {field}")
    pure = PurePosixPath(value[2:])
    require(not pure.is_absolute(), f"manifest path must be relative: {field}")
    require(".." not in pure.parts and "\\" not in value, f"manifest path escapes plugin root: {field}")
    unresolved = plugin_root.joinpath(*pure.parts)
    require(not unresolved.is_symlink(), f"manifest path must not be a symbolic link: {field}")
    require(unresolved.exists(), f"manifest path does not exist: {value}")
    resolved = unresolved.resolve()
    require(resolved.is_relative_to(plugin_root.resolve()), f"manifest path escapes plugin root: {field}")
    return resolved


def validate_manifest_schema(manifest: dict, policy: dict, plugin_root: Path) -> None:
    allowed_fields = set(policy["allowedManifestFields"])
    require(set(manifest) == allowed_fields, "manifest fields differ from the confined Lili schema")
    require(manifest["name"] == "lili", "manifest name must be lili")
    require(
        isinstance(manifest["version"], str)
        and re.fullmatch(
            r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)",
            manifest["version"],
        ),
        "manifest version must be a release semantic version",
    )
    for field in ("description", "homepage", "repository", "license"):
        require(isinstance(manifest[field], str) and manifest[field], f"invalid manifest field: {field}")
    require(
        isinstance(manifest["author"], dict) and set(manifest["author"]) == {"name", "url"},
        "manifest author must contain only name and url",
    )
    require(
        isinstance(manifest["keywords"], list)
        and manifest["keywords"]
        and all(isinstance(value, str) and value for value in manifest["keywords"]),
        "manifest keywords must be non-empty strings",
    )

    interface = manifest["interface"]
    require(isinstance(interface, dict), "manifest interface must be an object")
    require(
        set(interface) == set(policy["allowedInterfaceFields"]),
        "interface fields differ from the UI-less Lili schema",
    )
    string_fields = set(policy["allowedInterfaceFields"]) - {"capabilities", "defaultPrompt"}
    for field in string_fields:
        require(isinstance(interface[field], str) and interface[field], f"invalid interface field: {field}")
    require(len(interface["displayName"]) <= 30, "displayName exceeds 30 characters")
    require(len(interface["shortDescription"]) <= 30, "shortDescription exceeds 30 characters")
    require(len(interface["longDescription"]) <= 4000, "longDescription exceeds 4000 characters")
    require(len(interface["developerName"]) <= 80, "developerName exceeds 80 characters")
    require(re.fullmatch(r"#[0-9A-Fa-f]{6}", interface["brandColor"]) is not None, "invalid brandColor")
    capabilities = interface["capabilities"]
    prompts = interface["defaultPrompt"]
    require(
        isinstance(capabilities, list)
        and 1 <= len(capabilities) <= 20
        and all(isinstance(value, str) and 1 <= len(value) <= 120 for value in capabilities),
        "invalid interface capabilities",
    )
    require(
        isinstance(prompts, list)
        and 1 <= len(prompts) <= 3
        and all(isinstance(value, str) and 1 <= len(value) <= 128 for value in prompts),
        "invalid interface default prompts",
    )

    skills = resolve_manifest_path(plugin_root, manifest["skills"], "skills")
    require(skills.is_dir(), "manifest skills path must be a directory")
    hooks = resolve_manifest_path(plugin_root, manifest["hooks"], "hooks")
    require(hooks.is_file(), "manifest hooks path must be a file")
    for field in ("composerIcon", "logo"):
        asset = resolve_manifest_path(plugin_root, interface[field], f"interface.{field}")
        require(asset.is_file(), f"manifest asset path must be a file: {field}")


def validate_hook_schema(path: Path) -> dict:
    document = load_json(path)
    require(set(document).issubset({"description", "hooks"}) and "hooks" in document, "invalid hooks root")
    if "description" in document:
        require(isinstance(document["description"], str) and document["description"], "invalid hook description")
    hooks = document["hooks"]
    require(isinstance(hooks, dict) and set(hooks) == EXPECTED_HOOK_EVENTS, "hook event set drifted")
    for event, groups in hooks.items():
        require(isinstance(groups, list) and groups, f"hook groups must be non-empty: {event}")
        for group in groups:
            require(isinstance(group, dict) and set(group).issubset(GROUP_FIELDS), f"invalid hook group: {event}")
            require("hooks" in group and isinstance(group["hooks"], list) and group["hooks"], f"empty hook group: {event}")
            if "matcher" in group:
                require(isinstance(group["matcher"], str), f"invalid hook matcher: {event}")
            for handler in group["hooks"]:
                require(
                    isinstance(handler, dict)
                    and set(handler).issubset(HANDLER_FIELDS)
                    and {"type", "command"}.issubset(handler),
                    f"invalid hook handler: {event}",
                )
                require(handler["type"] == "command", f"unsupported hook handler type: {event}")
                require(isinstance(handler["command"], str) and handler["command"], f"empty hook command: {event}")
                if "commandWindows" in handler:
                    require(isinstance(handler["commandWindows"], str) and handler["commandWindows"], f"invalid Windows command: {event}")
                    require(
                        handler["commandWindows"].startswith(TRUSTED_WINDOWS_POWERSHELL + " "),
                        f"Windows command must use the trusted absolute PowerShell path: {event}",
                    )
                if "timeout" in handler:
                    timeout = handler["timeout"]
                    require(isinstance(timeout, int) and not isinstance(timeout, bool) and timeout >= 1, f"invalid timeout: {event}")
                    if event == "SessionEnd":
                        require(timeout <= 3, "SessionEnd timeout exceeds 3 seconds")
                if "statusMessage" in handler:
                    require(isinstance(handler["statusMessage"], str) and handler["statusMessage"], f"invalid status message: {event}")
                if "async" in handler:
                    require(isinstance(handler["async"], bool), f"invalid async flag: {event}")
    return document


def validate_hook_references(document: dict, plugin_root: Path) -> None:
    for event, groups in document["hooks"].items():
        for group in groups:
            for handler in group["hooks"]:
                for field in ("command", "commandWindows"):
                    if field not in handler:
                        continue
                    references = re.findall(
                        r"\$\{PLUGIN_ROOT\}([/\\][^\"'\s]+)",
                        handler[field],
                    )
                    references.extend(
                        re.findall(
                            r"Join-Path \$env:PLUGIN_ROOT ['\"]([^'\"]+)['\"]",
                            handler[field],
                        )
                    )
                    require(len(references) == 1, f"{event} {field} must reference one packaged path")
                    relative = "./" + references[0].lstrip("/\\").replace("\\", "/")
                    target = resolve_manifest_path(plugin_root, relative, f"{event}.{field}")
                    require(target.is_file(), f"hook command path must be a file: {event}.{field}")


def validate_metadata(manifest: dict, submission: dict, workspace_root: Path) -> None:
    with (workspace_root / "Cargo.toml").open("rb") as cargo_file:
        cargo_version = tomllib.load(cargo_file)["workspace"]["package"]["version"]
    tauri_version = load_json(workspace_root / "lili" / "tauri.conf.json")["version"]
    expected_versions = {
        cargo_version,
        tauri_version,
        submission["identity"]["initialVersion"],
        manifest["version"],
    }
    require(len(expected_versions) == 1, "release version metadata drifted")

    expected_pairs = (
        (manifest["description"], submission["identity"]["description"]),
        (manifest["author"], submission["publisher"]["author"]),
        (manifest["homepage"], submission["publisher"]["homepage"]),
        (manifest["repository"], submission["publisher"]["repository"]),
        (manifest["license"], submission["publisher"]["license"]),
        (manifest["interface"]["developerName"], submission["publisher"]["developerName"]),
        (manifest["interface"]["shortDescription"], submission["listing"]["shortDescription"]),
        (manifest["interface"]["longDescription"], submission["listing"]["longDescription"]),
        (manifest["interface"]["capabilities"], submission["listing"]["capabilities"]),
        (manifest["interface"]["defaultPrompt"], submission["listing"]["starterPrompts"]),
        (manifest["interface"]["websiteURL"], submission["publisher"]["homepage"]),
        (manifest["interface"]["supportURL"], submission["publisher"]["supportURL"]),
        (manifest["interface"]["privacyPolicyURL"], submission["publisher"]["privacyPolicyURL"]),
        (manifest["interface"]["termsOfServiceURL"], submission["publisher"]["termsOfServiceURL"]),
    )
    require(all(actual == expected for actual, expected in expected_pairs), "submission metadata drifted")


def validate_package_files(plugin_root: Path, policy: dict, submission: dict) -> None:
    allowed_files = set(policy["allowedPackageFiles"])
    declared_executables = set(policy["declaredExecutables"])
    require(declared_executables.issubset(allowed_files), "executable policy is inconsistent")
    forbidden_names = set(policy["forbiddenPathNames"])
    forbidden_directories = set(policy["forbiddenDirectoryNames"])
    prohibited_claims = [claim.casefold() for claim in submission["prohibitedClaims"]]
    placeholder_tokens = [token.casefold() for token in policy["placeholderTokens"]]

    for path in plugin_root.rglob("*"):
        relative = path.relative_to(plugin_root).as_posix()
        require(not path.is_symlink(), f"symbolic links are not allowed: {relative}")
        require(path.name not in forbidden_names, f"forbidden plugin configuration: {relative}")
        if path.is_dir():
            require(path.name not in forbidden_directories, f"forbidden plugin directory: {relative}")
            continue
        require(path.is_file(), f"unsupported package entry: {relative}")

        header = path.read_bytes()[:4]
        mode = path.stat().st_mode
        executable = (
            bool(mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH))
            or path.suffix.casefold() in EXECUTABLE_SUFFIXES
            or any(header.startswith(magic) for magic in EXECUTABLE_MAGICS)
        )
        if executable:
            require(relative in declared_executables, f"undeclared executable: {relative}")
        require(relative in allowed_files, f"undeclared package file: {relative}")

        if path.suffix.casefold() in {".json", ".md", ".ps1", ".sh", ".yaml", ".yml"} or relative == "hooks/forward":
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError as error:
                raise PolicyViolation(f"declared text file is not UTF-8: {relative}") from error
            folded = text.casefold()
            for token in placeholder_tokens:
                require(token not in folded, f"placeholder token found in {relative}: {token}")
            for claim in prohibited_claims:
                require(claim not in folded, f"prohibited endorsement or capability claim in {relative}")
            if relative != ".codex-plugin/plugin.json":
                require(re.search(r"https?://", text, re.IGNORECASE) is None, f"undeclared network endpoint in {relative}")


def validate_structural_policy(manifest: dict, policy: dict) -> None:
    forbidden_keys = set(policy["forbiddenManifestKeys"])
    allowed_url_fields = set(policy["allowedNetworkMetadataFields"])
    for path, value in walk_values(manifest):
        if path:
            require(path[-1] not in forbidden_keys, f"forbidden manifest configuration: {'.'.join(path)}")
        if isinstance(value, str) and re.search(r"https?://", value, re.IGNORECASE):
            field = ".".join(part for part in path if not part.isdigit())
            require(field in allowed_url_fields, f"undeclared network metadata field: {field}")


def validate_skill(plugin_root: Path) -> None:
    skill_path = plugin_root / "skills" / "lili-setup" / "SKILL.md"
    require(skill_path.is_file(), "lili-setup skill is missing")
    text = skill_path.read_text(encoding="utf-8")
    lines = text.splitlines()
    require(lines and lines[0] == "---", "skill frontmatter is missing")
    try:
        closing = lines.index("---", 1)
    except ValueError as error:
        raise PolicyViolation("skill frontmatter is not closed") from error
    fields = {}
    for line in lines[1:closing]:
        key, separator, value = line.partition(":")
        require(bool(separator) and key and value.strip(), "invalid skill frontmatter")
        fields[key] = value.strip()
    require(set(fields) == {"name", "description"}, "skill frontmatter fields drifted")
    require(fields["name"] == "lili-setup", "skill name drifted")


def validate_workspace(workspace_root: Path) -> None:
    workspace_root = workspace_root.resolve()
    plugin_root = workspace_root / "plugins" / "lili"
    require(plugin_root.is_dir(), "plugin root is missing")
    manifest = load_json(plugin_root / ".codex-plugin" / "plugin.json")
    policy = load_json(workspace_root / "marketplace" / "lili" / "package-policy.json")
    submission = load_json(workspace_root / "marketplace" / "lili" / "submission.json")
    require(policy.get("schemaVersion") == 1, "unsupported package policy schema")
    require(submission.get("schemaVersion") == 1, "unsupported submission metadata schema")
    require(
        len(policy["allowedPackageFiles"]) == len(set(policy["allowedPackageFiles"])),
        "package policy contains duplicate file declarations",
    )

    validate_structural_policy(manifest, policy)
    validate_manifest_schema(manifest, policy, plugin_root)
    hooks = validate_hook_schema(plugin_root / manifest["hooks"][2:])
    validate_hook_references(hooks, plugin_root)
    validate_metadata(manifest, submission, workspace_root)
    validate_skill(plugin_root)
    validate_package_files(plugin_root, policy, submission)


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate the confined Lili plugin package")
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
    )
    arguments = parser.parse_args()
    try:
        validate_workspace(arguments.workspace_root)
    except PolicyViolation as error:
        print(f"plugin package policy failed: {error}")
        return 1
    print("plugin package policy passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
