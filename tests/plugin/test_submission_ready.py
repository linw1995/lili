import copy
import datetime
import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKSPACE_ROOT / "scripts"))

from check_submission_ready import SubmissionReadinessError, validate_submission_readiness


class SubmissionReadinessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        self.now = datetime.datetime(2026, 8, 14, 2, 0, tzinfo=datetime.timezone.utc)
        self.revision = "a" * 40
        self.archive = self.root / "lili-plugin-0.1.0.zip"
        self.manifest = self.root / "lili-plugin-0.1.0.manifest.json"
        self.checksum = self.root / "lili-plugin-0.1.0.zip.sha256"
        self.supply_chain = self.root / "lili-plugin-0.1.0.supply-chain.json"
        for path in (self.archive, self.manifest, self.checksum, self.supply_chain):
            path.write_text(path.name, encoding="utf-8")
        self.archive_hash = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        self.evidence_path = self.root / "submission-evidence.json"
        self.evidence = self.valid_evidence()
        self.write_evidence()

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def material_entries(self) -> list[dict]:
        policy = json.loads(
            (WORKSPACE_ROOT / "marketplace" / "lili" / "submission-readiness.json").read_text(
                encoding="utf-8"
            )
        )
        return [
            {
                "path": relative,
                "sha256": hashlib.sha256((WORKSPACE_ROOT / relative).read_bytes()).hexdigest(),
            }
            for relative in policy["requiredReviewerMaterials"]
        ]

    def bound(self, **values: object) -> dict:
        return {
            "result": "passed",
            "archiveSha256": self.archive_hash,
            **values,
        }

    def valid_evidence(self) -> dict:
        policy = json.loads(
            (WORKSPACE_ROOT / "marketplace" / "lili" / "submission-readiness.json").read_text(
                encoding="utf-8"
            )
        )
        submission = json.loads(
            (WORKSPACE_ROOT / "marketplace" / "lili" / "submission.json").read_text(
                encoding="utf-8"
            )
        )
        publisher = submission["publisher"]
        urls = {
            publisher["homepage"],
            publisher["repository"],
            publisher["supportURL"],
            publisher["privacyPolicyURL"],
            publisher["termsOfServiceURL"],
            submission["prerequisites"]["desktopApplication"]["distributionURL"],
        }
        checked_at = self.now.isoformat().replace("+00:00", "Z")
        return {
            "schemaVersion": 1,
            "release": "0.1.0",
            "sourceRevision": self.revision,
            "archiveSha256": self.archive_hash,
            "generatedAt": checked_at,
            "gates": {
                "openSpec": self.bound(
                    checkedAt=checked_at,
                    strict=True,
                    sourceRevision=self.revision,
                    changes=policy["requiredOpenSpecChanges"],
                ),
                "automation": self.bound(
                    checkedAt=checked_at,
                    sourceRevision=self.revision,
                    runUrl="https://github.com/linw1995/lili/actions/runs/123",
                    checks=[
                        {"id": identifier, "result": "passed"}
                        for identifier in policy["requiredAutomation"]
                    ]
                ),
                "packagedAcceptance": self.bound(
                    checkedAt=checked_at,
                    sourceRevision=self.revision,
                    targets=[
                        {
                            "target": target,
                            "codexVersions": ["0.147.0"],
                            "result": "passed",
                            "runUrl": f"https://github.com/linw1995/lili/actions/runs/{index}",
                        }
                        for index, target in enumerate(policy["requiredPackagedTargets"], start=1)
                    ]
                ),
                "publicUrls": self.bound(
                    checkedAt=checked_at,
                    urls=[
                        {
                            "url": url,
                            "finalUrl": url,
                            "status": 200,
                            "reachable": True,
                            "authenticationRequired": False,
                        }
                        for url in sorted(urls)
                    ],
                ),
                "publisherIdentity": self.bound(
                    verified=True,
                    verifiedAt=checked_at,
                    developerName="Jade Lin",
                    accountId="publisher-123",
                ),
                "reviewerMaterials": self.bound(
                    checkedAt=checked_at,
                    sourceRevision=self.revision,
                    files=self.material_entries(),
                ),
                "currentRules": self.bound(
                    checkedAt=checked_at,
                    reviewedAt="2026-08-14",
                    sources=[
                        {**source, "result": "passed"}
                        for source in policy["requiredRuleSources"]
                    ],
                ),
                "portalPreflight": self.bound(
                    checkedAt=checked_at,
                    packageAccepted=True,
                    scannerAccepted=True,
                    skillsOnlyWithCodexHooksAccepted=True,
                    nativeForwardersAccepted=True,
                    draftId="draft-123",
                    unresolvedRestrictions=[],
                ),
            },
        }

    def write_evidence(self) -> None:
        self.evidence_path.write_text(json.dumps(self.evidence), encoding="utf-8")

    def validate(self) -> dict:
        with (
            patch("check_submission_ready.validate_marketplace"),
            patch(
                "check_submission_ready.inspect_release",
                return_value={
                    "result": "passed",
                    "version": "0.1.0",
                    "sha256": self.archive_hash,
                },
            ),
        ):
            return validate_submission_readiness(
                WORKSPACE_ROOT,
                self.evidence_path,
                self.archive,
                self.manifest,
                self.checksum,
                self.supply_chain,
                self.now,
                self.revision,
            )

    def assert_rejected(self, expected: str) -> None:
        self.write_evidence()
        with self.assertRaisesRegex(SubmissionReadinessError, expected):
            self.validate()

    def test_complete_current_evidence_passes(self) -> None:
        self.assertEqual(self.validate()["result"], "submission-ready")

    def test_every_external_gate_fails_closed(self) -> None:
        mutations = {
            "OpenSpec": lambda value: value["gates"]["openSpec"].update(strict=False),
            "automated acceptance": lambda value: value["gates"]["automation"]["checks"].pop(),
            "packaged acceptance": lambda value: value["gates"]["packagedAcceptance"]["targets"].pop(),
            "public URL": lambda value: value["gates"]["publicUrls"]["urls"][0].update(reachable=False),
            "publisher identity": lambda value: value["gates"]["publisherIdentity"].update(verified=False),
            "reviewer material": lambda value: value["gates"]["reviewerMaterials"]["files"][0].update(sha256="0" * 64),
            "current-rule": lambda value: value["gates"]["currentRules"]["sources"].pop(),
            "portal preflight": lambda value: value["gates"]["portalPreflight"].update(scannerAccepted=False),
        }
        original = copy.deepcopy(self.evidence)
        for expected, mutate in mutations.items():
            with self.subTest(gate=expected):
                self.evidence = copy.deepcopy(original)
                mutate(self.evidence)
                self.assert_rejected(expected)

    def test_stale_evidence_is_rejected(self) -> None:
        self.evidence["generatedAt"] = "2026-08-01T00:00:00Z"
        self.assert_rejected("generation time is stale")

    def test_archive_binding_drift_is_rejected(self) -> None:
        self.evidence["gates"]["portalPreflight"]["archiveSha256"] = "0" * 64
        self.assert_rejected("portal preflight is not bound")


if __name__ == "__main__":
    unittest.main()
