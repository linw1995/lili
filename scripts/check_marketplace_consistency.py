#!/usr/bin/env python3

import argparse
import json
from pathlib import Path, PurePosixPath

from check_plugin_package import PolicyViolation, validate_workspace


class ConsistencyError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ConsistencyError(message)


def load_json(path: Path) -> dict:
    require(path.is_file(), f"required JSON file is missing: {path}")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ConsistencyError(f"invalid JSON file: {path}") from error
    require(isinstance(value, dict), f"JSON root must be an object: {path}")
    return value


def load_text(path: Path) -> str:
    require(path.is_file(), f"required text file is missing: {path}")
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise ConsistencyError(f"invalid UTF-8 text file: {path}") from error


def require_phrases(path: Path, phrases: tuple[str, ...]) -> str:
    text = load_text(path)
    for phrase in phrases:
        require(phrase in text, f"required disclosure drifted in {path}: {phrase}")
    return text


def resolve_marketplace_reference(marketplace_root: Path, value: object, field: str) -> Path:
    require(isinstance(value, str) and value.startswith("./"), f"invalid {field} reference")
    relative = PurePosixPath(value[2:])
    require(relative.parts and ".." not in relative.parts, f"{field} escapes Marketplace root")
    resolved = marketplace_root.joinpath(*relative.parts).resolve()
    require(resolved.is_relative_to(marketplace_root.resolve()), f"{field} escapes Marketplace root")
    require(resolved.is_file(), f"{field} target is missing")
    return resolved


def validate_reviewer_cases(path: Path, release: str, review_date: str, positive: bool) -> dict:
    document = load_json(path)
    require(document.get("schemaVersion") == 1, f"unsupported reviewer schema: {path}")
    require(document.get("release") == release, f"reviewer release drifted: {path}")
    require(document.get("reviewedAt") == review_date, f"review date drifted: {path}")
    require(document.get("commonPrerequisites"), f"reviewer prerequisites are missing: {path}")
    cases = document.get("cases")
    require(isinstance(cases, list), f"reviewer cases are missing: {path}")
    require(len(cases) >= (5 if positive else 3), f"reviewer case count is too small: {path}")
    identifiers = [case.get("id") for case in cases if isinstance(case, dict)]
    require(len(identifiers) == len(cases), f"reviewer case must be an object: {path}")
    require(
        len(identifiers) == len(set(identifiers)),
        f"reviewer case identifiers are duplicated: {path}",
    )
    for case in cases:
        for field in (
            "title",
            "coverage",
            "fixtureData",
            "workflow",
            "expectedResult",
            "automationEvidence",
        ):
            require(case.get(field), f"{case.get('id', 'unknown')} is missing {field}")
        expected = case["expectedResult"]
        require(
            isinstance(expected, dict) and expected.get("shape") and expected.get("assertions"),
            f"{case['id']} has no expected result shape or assertions",
        )
    return document


def validate_marketplace(workspace_root: Path) -> None:
    workspace_root = workspace_root.resolve()
    try:
        validate_workspace(workspace_root)
    except PolicyViolation as error:
        raise ConsistencyError(f"plugin package is inconsistent: {error}") from error

    marketplace_root = workspace_root / "marketplace" / "lili"
    plugin_root = workspace_root / "plugins" / "lili"
    manifest = load_json(plugin_root / ".codex-plugin" / "plugin.json")
    submission = load_json(marketplace_root / "submission.json")
    compatibility = load_json(marketplace_root / "compatibility.json")
    assets = load_json(marketplace_root / "assets.json")
    package_policy = load_json(marketplace_root / "package-policy.json")
    local_catalog = load_json(
        workspace_root
        / "marketplace"
        / "local"
        / ".agents"
        / "plugins"
        / "marketplace.json"
    )
    local_lifecycle = load_json(
        workspace_root / "marketplace" / "local" / "lifecycle.json"
    )

    version = manifest["version"]
    review_date = submission["reviewedAt"]
    require(submission["release"]["version"] == version, "release version drifted")
    for name, document in (
        ("compatibility", compatibility),
        ("assets", assets),
        ("package policy", package_policy),
    ):
        require(document.get("reviewedAt") == review_date, f"{name} review date drifted")
    require(local_lifecycle.get("reviewedAt") == review_date, "local catalog review date drifted")
    require(local_lifecycle.get("schemaVersion") == 1, "local catalog lifecycle schema drifted")
    require(local_lifecycle.get("codexVersion") in compatibility["codex"]["testedVersions"], "local catalog Codex version is unreviewed")
    require(local_catalog.get("name") == local_lifecycle.get("marketplaceName"), "local catalog name drifted")
    catalog_plugins = local_catalog.get("plugins")
    require(isinstance(catalog_plugins, list) and len(catalog_plugins) == 1, "local catalog plugin count drifted")
    catalog_plugin = catalog_plugins[0]
    require(catalog_plugin.get("name") == manifest["name"], "local catalog plugin name drifted")
    require(catalog_plugin.get("source") == {"source": "local", "path": "./plugins/lili"}, "local catalog source drifted")
    require(local_lifecycle.get("pluginSelector") == "lili@lili-local", "local plugin selector drifted")
    lifecycle_operations = local_lifecycle.get("operations")
    require(
        isinstance(lifecycle_operations, dict)
        and set(lifecycle_operations) == {"install", "enable", "disable", "update", "rollback", "remove"},
        "local catalog lifecycle coverage drifted",
    )
    require(
        {operation["command"][-1] for operation in lifecycle_operations.values()}
        == {"add", "remove"},
        "local catalog uses an unsupported plugin command",
    )

    release_notes = resolve_marketplace_reference(
        marketplace_root, submission["release"]["releaseNotes"], "release notes"
    )
    checklist = resolve_marketplace_reference(
        marketplace_root, submission["release"]["complianceChecklist"], "compliance checklist"
    )
    release_text = require_phrases(
        release_notes,
        (
            f"# Lili {version}",
            "ChatGPT lifecycle observation is not supported.",
            "does not approve permission requests",
            "does not install or update the Lili desktop application",
            "does not imply hook trust",
        ),
    )
    checklist_text = require_phrases(
        checklist,
        (
            f"Review date: {review_date}",
            f"Release: {version}",
            "Status: In preparation.",
            "The release must remain marked not submission-ready while any publication gate is unchecked.",
        ),
    )
    require(
        checklist_text.count("- [ ]") >= 5,
        "submission checklist no longer exposes pending gates",
    )

    repository = submission["publisher"]["repository"].rstrip("/")
    legal_documents = {
        "homepage": "marketplace.md",
        "supportURL": "support.md",
        "privacyPolicyURL": "privacy-policy.md",
        "termsOfServiceURL": "terms-of-service.md",
    }
    for field, name in legal_documents.items():
        expected_url = f"{repository}/blob/master/docs/{name}"
        require(submission["publisher"][field] == expected_url, f"{field} URL drifted")
        require((workspace_root / "docs" / name).is_file(), f"published document is missing: {name}")

    require(
        submission["availability"]["selection"]
        == "all-portal-supported-countries-and-regions",
        "availability drifted",
    )
    require(
        submission["availability"].get("instructions"),
        "availability instructions are missing",
    )
    prerequisites = submission["prerequisites"].get("instructions")
    require(
        isinstance(prerequisites, list) and len(prerequisites) >= 5,
        "prerequisite instructions are incomplete",
    )

    positive = validate_reviewer_cases(
        marketplace_root / "reviewer-cases" / "positive.json", version, review_date, True
    )
    negative = validate_reviewer_cases(
        marketplace_root / "reviewer-cases" / "negative.json", version, review_date, False
    )
    positive_coverage = {case["coverage"] for case in positive["cases"]}
    require(
        positive_coverage
        == {"setup", "trusted_delivery", "offline_recovery", "migration", "diagnostics"},
        "positive reviewer coverage drifted",
    )
    negative_coverage = {case["coverage"] for case in negative["cases"]}
    require(
        {
            "unsupported_chatgpt_lifecycle",
            "permission_authority",
            "private_data_access",
            "remote_transfer",
            "unsupported_host",
        }.issubset(negative_coverage),
        "negative reviewer coverage drifted",
    )
    represented_prompts = {case.get("starterPrompt") for case in positive["cases"]}
    require(
        set(submission["listing"]["starterPrompts"]).issubset(represented_prompts),
        "starter prompt coverage drifted",
    )

    asset_paths = {asset["path"] for asset in assets["assets"]}
    require(
        asset_paths == {manifest["interface"]["composerIcon"], manifest["interface"]["logo"]},
        "reviewed asset registry drifted from manifest",
    )
    allowed_files = set(package_policy["allowedPackageFiles"])
    required_release_files = {
        ".codex-plugin/plugin.json",
        manifest["skills"].removeprefix("./") + "lili-setup/SKILL.md",
        manifest["hooks"].removeprefix("./"),
        *(path.removeprefix("./") for path in asset_paths),
    }
    require(required_release_files.issubset(allowed_files), "release contents drifted from manifest")

    skill = require_phrases(
        plugin_root / "skills" / "lili-setup" / "SKILL.md",
        (
            "On ChatGPT, provide setup, compatibility, migration, and troubleshooting guidance only.",
            "Do not read `auth.json`",
            "Do not make network requests.",
            "Never approve or deny a `PermissionRequest`",
            "Do not request prompts, assistant messages, tokens, secrets, or raw session files.",
        ),
    )
    marketplace_text = require_phrases(
        workspace_root / "docs" / "marketplace.md",
        (
            "It does not observe ChatGPT conversation lifecycle events.",
            "The plugin does not install the desktop application, approve or deny permission requests",
            "Raw prompts, credentials, approval arguments",
            "Lili has no product telemetry or remote session-data recipient.",
        ),
    )
    privacy_text = require_phrases(
        workspace_root / "docs" / "privacy-policy.md",
        (
            "does not request or retain raw prompts, approval arguments",
            "do not send session data, diagnostics, or telemetry",
            "hard-capped at 256 records, 4 MiB total, and 24 hours",
            "Permission events are observation-only and never authorize a request.",
        ),
    )
    require_phrases(
        workspace_root / "docs" / "terms-of-service.md",
        (
            "does not provide automatic conversation lifecycle observation",
            "does not install the desktop application through the plugin",
            "approve or deny permission requests",
        ),
    )

    public_listing_text = json.dumps(
        {
            "manifest": manifest,
            "listing": submission["listing"],
            "surfaceCopy": submission["surfaceCopy"],
            "releaseNotes": release_text,
        },
        sort_keys=True,
    ).casefold()
    for claim in submission["prohibitedClaims"]:
        require(
            claim.casefold() not in public_listing_text,
            f"prohibited claim appears in public listing: {claim}",
        )

    runtime_evidence = {
        workspace_root / "lili-session" / "src" / "codex.rs": (
            "lifecycle_adapter_does_not_retain_prompt_or_permission_details",
            "plugin_diagnostics_require_observed_delivery_for_trust",
        ),
        workspace_root / "lili-session" / "src" / "forwarding.rs": (
            "credentials_rotate_and_debug_output_redacts_the_secret",
        ),
        workspace_root / "lili-session" / "src" / "spool.rs": (
            "pub const HARD_MAX_COUNT: usize = 256",
            "pub const HARD_MAX_BYTES: u64 = 4 * 1024 * 1024",
            "pub const HARD_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1_000",
        ),
        workspace_root / "lili-session" / "src" / "transport.rs": (
            ".reject_remote_clients(true)",
        ),
        workspace_root / "lili" / "tests" / "permission_hook.rs": (
            "permission hooks must not decide",
        ),
    }
    for path, markers in runtime_evidence.items():
        require_phrases(path, markers)

    require(
        "guidance only" in skill and "guidance only" in marketplace_text,
        "surface guidance drifted",
    )
    require("owner-only local spool" in privacy_text, "local spool disclosure drifted")


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate Lili Marketplace material consistency")
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
    )
    arguments = parser.parse_args()
    try:
        validate_marketplace(arguments.workspace_root)
    except ConsistencyError as error:
        print(f"marketplace consistency failed: {error}")
        return 1
    print("marketplace consistency passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
