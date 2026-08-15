import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKSPACE_ROOT / "scripts"))

from generate_plugin_supply_chain import SupplyChainError, audit_summary, build_evidence


class PluginSupplyChainTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        shutil.copy2(WORKSPACE_ROOT / "Cargo.toml", self.root / "Cargo.toml")
        shutil.copy2(WORKSPACE_ROOT / "deny.toml", self.root / "deny.toml")
        (self.root / "Cargo.lock").write_text(
            """version = 4

[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
""",
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def metadata(self) -> dict:
        lili_id = "path+file:///workspace/lili#0.1.0"
        serde_id = "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0"
        dev_id = "registry+https://github.com/rust-lang/crates.io-index#dev-only@1.0.0"
        return {
            "packages": [
                {
                    "id": lili_id,
                    "name": "lili",
                    "version": "0.1.0",
                    "manifest_path": "/workspace/lili/Cargo.toml",
                    "source": None,
                    "license": "Apache-2.0",
                },
                {
                    "id": serde_id,
                    "name": "serde",
                    "version": "1.0.0",
                    "manifest_path": "/registry/serde/Cargo.toml",
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "license": "MIT OR Apache-2.0",
                },
                {
                    "id": dev_id,
                    "name": "dev-only",
                    "version": "1.0.0",
                    "manifest_path": "/registry/dev-only/Cargo.toml",
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "license": "MIT",
                },
            ],
            "resolve": {
                "nodes": [
                    {
                        "id": lili_id,
                        "deps": [
                            {"pkg": serde_id, "dep_kinds": [{"kind": None}]},
                            {"pkg": dev_id, "dep_kinds": [{"kind": "dev"}]},
                        ],
                    },
                    {"id": serde_id, "deps": []},
                    {"id": dev_id, "deps": []},
                ]
            },
        }

    def audit(self, vulnerability_count: int = 0) -> dict:
        return {
            "database": {
                "advisory-count": 100,
                "last-commit": "abc123",
                "last-updated": "2026-08-14T00:00:00Z",
            },
            "vulnerabilities": {
                "found": vulnerability_count > 0,
                "count": vulnerability_count,
                "list": [],
            },
            "warnings": {
                "unmaintained": [
                    {
                        "advisory": {"id": "RUSTSEC-2026-0001"},
                        "package": {"name": "serde", "version": "1.0.0"},
                    }
                ]
            },
        }

    def test_evidence_contains_production_inventory_and_scan_result(self) -> None:
        evidence = build_evidence(
            self.root,
            self.metadata(),
            self.audit(),
            "cargo-audit 0.22.1",
            "cargo-deny 0.19.0",
            "2026-08-14T00:00:00Z",
        )
        packages = evidence["dependencyInventory"]["packages"]
        self.assertEqual([package["name"] for package in packages], ["lili", "serde"])
        self.assertEqual(evidence["vulnerabilityScan"]["vulnerabilityCount"], 0)
        self.assertEqual(
            evidence["vulnerabilityScan"]["result"],
            "passed-with-informational-warnings",
        )
        json.dumps(evidence)

    def test_vulnerability_fails_closed(self) -> None:
        with self.assertRaisesRegex(SupplyChainError, "found an advisory"):
            audit_summary(self.audit(vulnerability_count=1))


if __name__ == "__main__":
    unittest.main()
