import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKSPACE_ROOT / "scripts"))

from check_marketplace_consistency import ConsistencyError, validate_marketplace


class MarketplaceConsistencyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        shutil.copy2(WORKSPACE_ROOT / "Cargo.toml", self.root / "Cargo.toml")
        shutil.copytree(WORKSPACE_ROOT / "marketplace", self.root / "marketplace")
        shutil.copytree(WORKSPACE_ROOT / "plugins", self.root / "plugins")
        shutil.copytree(WORKSPACE_ROOT / "docs", self.root / "docs")
        (self.root / "lili").mkdir()
        shutil.copy2(
            WORKSPACE_ROOT / "lili" / "tauri.conf.json",
            self.root / "lili" / "tauri.conf.json",
        )
        (self.root / "lili" / "tests").mkdir()
        shutil.copy2(
            WORKSPACE_ROOT / "lili" / "tests" / "permission_hook.rs",
            self.root / "lili" / "tests" / "permission_hook.rs",
        )
        (self.root / "lili-session" / "src").mkdir(parents=True)
        for name in ("codex.rs", "forwarding.rs", "spool.rs", "transport.rs"):
            shutil.copy2(
                WORKSPACE_ROOT / "lili-session" / "src" / name,
                self.root / "lili-session" / "src" / name,
            )

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def assert_rejected(self, expected: str) -> None:
        with self.assertRaisesRegex(ConsistencyError, expected):
            validate_marketplace(self.root)

    def test_current_marketplace_materials_pass(self) -> None:
        validate_marketplace(self.root)

    def test_chatgpt_boundary_drift_is_rejected(self) -> None:
        path = self.root / "docs" / "marketplace.md"
        text = path.read_text(encoding="utf-8")
        path.write_text(
            text.replace(
                "It does not observe ChatGPT conversation lifecycle events.",
                "Lifecycle behavior is unspecified.",
            ),
            encoding="utf-8",
        )
        self.assert_rejected("required disclosure drifted")

    def test_review_case_coverage_drift_is_rejected(self) -> None:
        path = self.root / "marketplace" / "lili" / "reviewer-cases" / "positive.json"
        document = json.loads(path.read_text(encoding="utf-8"))
        document["cases"][0]["coverage"] = "diagnostics"
        path.write_text(json.dumps(document), encoding="utf-8")
        self.assert_rejected("positive reviewer coverage drifted")

    def test_runtime_evidence_drift_is_rejected(self) -> None:
        path = self.root / "lili-session" / "src" / "transport.rs"
        text = path.read_text(encoding="utf-8")
        path.write_text(text.replace(".reject_remote_clients(true)", ""), encoding="utf-8")
        self.assert_rejected("required disclosure drifted")


if __name__ == "__main__":
    unittest.main()
