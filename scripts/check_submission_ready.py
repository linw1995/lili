#!/usr/bin/env python3

import argparse
import datetime
import hashlib
import json
import re
import subprocess
import tomllib
from pathlib import Path

from check_marketplace_consistency import validate_marketplace
from inspect_plugin_release import inspect_release


SHA256_PATTERN = re.compile(r"[0-9a-f]{64}")
REVISION_PATTERN = re.compile(r"[0-9a-f]{40,64}")


class SubmissionReadinessError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SubmissionReadinessError(message)


def load_json(path: Path, label: str) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise SubmissionReadinessError(f"{label} is invalid") from error
    require(isinstance(value, dict), f"{label} root must be an object")
    return value


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def timestamp(value: object, label: str) -> datetime.datetime:
    require(isinstance(value, str) and value, f"{label} is missing")
    try:
        parsed = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise SubmissionReadinessError(f"{label} is invalid") from error
    require(parsed.tzinfo is not None, f"{label} must include a timezone")
    return parsed.astimezone(datetime.timezone.utc)


def require_current(
    value: object,
    label: str,
    now: datetime.datetime,
    maximum_age_hours: int,
) -> datetime.datetime:
    parsed = timestamp(value, label)
    require(parsed <= now + datetime.timedelta(minutes=5), f"{label} is in the future")
    require(
        now - parsed <= datetime.timedelta(hours=maximum_age_hours),
        f"{label} is stale",
    )
    return parsed


def require_bound_gate(gate: object, label: str, archive_hash: str) -> dict:
    require(isinstance(gate, dict), f"{label} evidence is missing")
    require(gate.get("result") == "passed", f"{label} did not pass")
    require(
        gate.get("archiveSha256") == archive_hash,
        f"{label} is not bound to the final archive",
    )
    return gate


def required_public_urls(submission: dict) -> set[str]:
    publisher = submission["publisher"]
    return {
        publisher["homepage"],
        publisher["repository"],
        publisher["supportURL"],
        publisher["privacyPolicyURL"],
        publisher["termsOfServiceURL"],
        submission["prerequisites"]["desktopApplication"]["distributionURL"],
    }


def validate_public_urls(
    gate: dict,
    expected_urls: set[str],
    now: datetime.datetime,
    maximum_age_hours: int,
) -> None:
    require_current(gate.get("checkedAt"), "public URL check time", now, maximum_age_hours)
    entries = gate.get("urls")
    require(isinstance(entries, list), "public URL evidence is missing")
    observed = {entry.get("url") for entry in entries if isinstance(entry, dict)}
    require(len(observed) == len(entries), "public URL evidence contains duplicates")
    require(observed == expected_urls, "public URL evidence set drifted")
    for entry in entries:
        require(entry.get("reachable") is True, f"public URL is unreachable: {entry.get('url')}")
        require(entry.get("authenticationRequired") is False, f"public URL requires authentication: {entry.get('url')}")
        status = entry.get("status")
        require(isinstance(status, int) and 200 <= status < 400, f"public URL status failed: {entry.get('url')}")
        require(
            isinstance(entry.get("finalUrl"), str) and entry["finalUrl"].startswith("https://"),
            f"public URL final location is invalid: {entry.get('url')}",
        )


def validate_materials(workspace_root: Path, gate: dict, paths: set[str]) -> None:
    entries = gate.get("files")
    require(isinstance(entries, list), "reviewer material evidence is missing")
    observed = {entry.get("path") for entry in entries if isinstance(entry, dict)}
    require(len(observed) == len(entries), "reviewer material evidence contains duplicates")
    require(observed == paths, "reviewer material set drifted")
    for entry in entries:
        relative = entry["path"]
        path = workspace_root / relative
        require(path.is_file() and not path.is_symlink(), f"reviewer material is missing: {relative}")
        require(entry.get("sha256") == sha256(path), f"reviewer material drifted: {relative}")


def validate_submission_readiness(
    workspace_root: Path,
    evidence_path: Path,
    archive_path: Path,
    manifest_path: Path,
    checksum_path: Path,
    supply_chain_path: Path,
    now: datetime.datetime,
    source_revision: str,
) -> dict:
    workspace_root = workspace_root.resolve()
    now = now.astimezone(datetime.timezone.utc)
    policy = load_json(
        workspace_root / "marketplace" / "lili" / "submission-readiness.json",
        "submission readiness policy",
    )
    evidence = load_json(evidence_path, "submission readiness evidence")
    submission = load_json(
        workspace_root / "marketplace" / "lili" / "submission.json",
        "submission metadata",
    )
    compatibility = load_json(
        workspace_root / "marketplace" / "lili" / "compatibility.json",
        "compatibility metadata",
    )
    require(policy.get("schemaVersion") == 1, "unsupported submission readiness policy schema")
    require(evidence.get("schemaVersion") == 1, "unsupported submission readiness evidence schema")
    maximum_age = policy["evidenceMaxAgeHours"]
    require_current(evidence.get("generatedAt"), "evidence generation time", now, maximum_age)

    with (workspace_root / "Cargo.toml").open("rb") as stream:
        version = tomllib.load(stream)["workspace"]["package"]["version"]
    require(submission["release"]["version"] == version, "submission release version drifted")
    require(evidence.get("release") == version, "evidence release version drifted")
    require(REVISION_PATTERN.fullmatch(source_revision) is not None, "source revision is invalid")
    require(evidence.get("sourceRevision") == source_revision, "evidence source revision drifted")

    inspection = inspect_release(
        archive_path.resolve(),
        manifest_path.resolve(),
        checksum_path.resolve(),
        supply_chain_path.resolve(),
    )
    archive_hash = inspection["sha256"]
    require(SHA256_PATTERN.fullmatch(archive_hash) is not None, "final archive hash is invalid")
    require(inspection.get("result") == "passed", "final archive inspection did not pass")
    require(inspection.get("version") == version, "final archive version drifted")
    require(evidence.get("archiveSha256") == archive_hash, "evidence final archive drifted")

    gates = evidence.get("gates")
    require(isinstance(gates, dict), "submission gate evidence is missing")
    openspec = require_bound_gate(gates.get("openSpec"), "strict OpenSpec validation", archive_hash)
    require_current(openspec.get("checkedAt"), "OpenSpec validation time", now, maximum_age)
    require(openspec.get("strict") is True, "OpenSpec validation was not strict")
    require(openspec.get("sourceRevision") == source_revision, "OpenSpec evidence source revision drifted")
    changes = openspec.get("changes")
    require(
        isinstance(changes, list)
        and len(changes) == len(set(changes))
        and set(changes) == set(policy["requiredOpenSpecChanges"]),
        "OpenSpec validated change set drifted",
    )

    automation = require_bound_gate(gates.get("automation"), "automated acceptance", archive_hash)
    require_current(automation.get("checkedAt"), "automated acceptance time", now, maximum_age)
    require(automation.get("sourceRevision") == source_revision, "automated acceptance source revision drifted")
    run_prefix = submission["publisher"]["repository"].rstrip("/") + "/actions/runs/"
    require(
        isinstance(automation.get("runUrl"), str)
        and automation["runUrl"].startswith(run_prefix),
        "automated acceptance run URL is missing",
    )
    checks = automation.get("checks")
    require(isinstance(checks, list), "automated acceptance checks are missing")
    observed_checks = {entry.get("id") for entry in checks if isinstance(entry, dict)}
    require(len(observed_checks) == len(checks), "automated acceptance checks contain duplicates")
    require(observed_checks == set(policy["requiredAutomation"]), "automated acceptance set drifted")
    require(all(entry.get("result") == "passed" for entry in checks), "an automated acceptance check failed")

    packaged = require_bound_gate(gates.get("packagedAcceptance"), "packaged acceptance", archive_hash)
    require_current(packaged.get("checkedAt"), "packaged acceptance time", now, maximum_age)
    require(packaged.get("sourceRevision") == source_revision, "packaged acceptance source revision drifted")
    targets = packaged.get("targets")
    require(isinstance(targets, list), "packaged acceptance targets are missing")
    observed_targets = {entry.get("target") for entry in targets if isinstance(entry, dict)}
    require(len(observed_targets) == len(targets), "packaged acceptance targets contain duplicates")
    require(observed_targets == set(policy["requiredPackagedTargets"]), "packaged acceptance target set drifted")
    tested_codex = compatibility["codex"]["testedVersions"]
    for entry in targets:
        require(entry.get("result") == "passed", f"packaged acceptance failed: {entry.get('target')}")
        require(entry.get("codexVersions") == tested_codex, f"packaged Codex matrix drifted: {entry.get('target')}")
        require(
            isinstance(entry.get("runUrl"), str) and entry["runUrl"].startswith(run_prefix),
            f"packaged acceptance run URL is missing: {entry.get('target')}",
        )

    urls = require_bound_gate(gates.get("publicUrls"), "public URL validation", archive_hash)
    validate_public_urls(urls, required_public_urls(submission), now, maximum_age)

    identity = require_bound_gate(gates.get("publisherIdentity"), "publisher identity", archive_hash)
    require_current(
        identity.get("verifiedAt"),
        "publisher identity verification time",
        now,
        policy["identityMaxAgeHours"],
    )
    developer_name = submission["publisher"]["developerName"]
    require(identity.get("verified") is True, "publisher identity is not verified")
    require(identity.get("developerName") == developer_name, "publisher identity does not match the listing")
    require(isinstance(identity.get("accountId"), str) and identity["accountId"], "publisher account identity is missing")

    validate_marketplace(workspace_root)
    materials = require_bound_gate(gates.get("reviewerMaterials"), "reviewer materials", archive_hash)
    require_current(materials.get("checkedAt"), "reviewer material review time", now, maximum_age)
    require(materials.get("sourceRevision") == source_revision, "reviewer material source revision drifted")
    validate_materials(workspace_root, materials, set(policy["requiredReviewerMaterials"]))

    rules = require_bound_gate(gates.get("currentRules"), "current-rule revalidation", archive_hash)
    require_current(rules.get("checkedAt"), "current-rule review time", now, maximum_age)
    require(rules.get("reviewedAt") == submission["reviewedAt"], "current-rule review date drifted")
    expected_sources = {(entry["id"], entry["url"]) for entry in policy["requiredRuleSources"]}
    source_entries = rules.get("sources")
    require(isinstance(source_entries, list), "current-rule source evidence is missing")
    actual_sources = {
        (entry.get("id"), entry.get("url"))
        for entry in source_entries
        if isinstance(entry, dict) and entry.get("result") == "passed"
    }
    require(len(actual_sources) == len(source_entries), "current-rule source evidence contains duplicates or failures")
    require(actual_sources == expected_sources, "current-rule source evidence drifted")

    portal = require_bound_gate(gates.get("portalPreflight"), "submission portal preflight", archive_hash)
    require_current(portal.get("checkedAt"), "submission portal preflight time", now, maximum_age)
    for field in (
        "packageAccepted",
        "scannerAccepted",
        "skillsOnlyWithCodexHooksAccepted",
        "nativeForwardersAccepted",
    ):
        require(portal.get(field) is True, f"submission portal preflight did not confirm {field}")
    require(isinstance(portal.get("draftId"), str) and portal["draftId"], "submission portal draft identity is missing")
    require(portal.get("unresolvedRestrictions") == [], "submission portal preflight has unresolved restrictions")

    return {
        "schemaVersion": 1,
        "release": version,
        "archiveSha256": archive_hash,
        "sourceRevision": source_revision,
        "result": "submission-ready",
    }


def git_source_revision(workspace_root: Path) -> str:
    revision = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=workspace_root,
        capture_output=True,
        check=False,
        text=True,
    )
    require(revision.returncode == 0, "source revision could not be read")
    dirty = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=no"],
        cwd=workspace_root,
        capture_output=True,
        check=False,
        text=True,
    )
    require(dirty.returncode == 0, "source status could not be read")
    require(not dirty.stdout.strip(), "tracked source changes are not committed")
    return revision.stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser(description="Require complete Lili submission readiness evidence")
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--checksum", required=True, type=Path)
    parser.add_argument("--supply-chain", required=True, type=Path)
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
    )
    arguments = parser.parse_args()
    try:
        result = validate_submission_readiness(
            arguments.workspace_root,
            arguments.evidence.resolve(),
            arguments.archive.resolve(),
            arguments.manifest.resolve(),
            arguments.checksum.resolve(),
            arguments.supply_chain.resolve(),
            datetime.datetime.now(datetime.timezone.utc),
            git_source_revision(arguments.workspace_root.resolve()),
        )
    except (SubmissionReadinessError, OSError, KeyError, TypeError, ValueError) as error:
        print(f"submission readiness failed: {error}")
        return 1
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
