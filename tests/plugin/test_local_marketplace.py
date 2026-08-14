import json
import os
import shutil
import stat
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKSPACE_ROOT / "scripts"))

from test_local_marketplace import (
    MarketplaceRoundTripError,
    extract_archive,
    run_round_trip,
)


class LocalMarketplaceRoundTripTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        self.archive = self.root / "lili-plugin-0.1.0.zip"
        self.write_archive(self.archive)

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def write_archive(self, destination: Path) -> None:
        plugin_root = WORKSPACE_ROOT / "plugins" / "lili"
        policy = json.loads(
            (WORKSPACE_ROOT / "marketplace" / "lili" / "package-policy.json").read_text(
                encoding="utf-8"
            )
        )
        binary_headers = {
            "bin/arm64-apple-darwin/lili-hook": b"\xcf\xfa\xed\xfe",
            "bin/x86_64-unknown-linux-gnu/lili-hook": b"\x7fELF",
            "bin/x86_64-pc-windows-msvc/lili-hook.exe": b"MZ",
        }
        executables = set(policy["declaredExecutables"])
        with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for relative in sorted(policy["allowedPackageFiles"]):
                source = plugin_root / relative
                contents = source.read_bytes() if source.is_file() else binary_headers[relative] + b"fixture"
                mode = 0o755 if relative in executables and not relative.endswith(".ps1") else 0o644
                info = zipfile.ZipInfo(relative, (1980, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = (stat.S_IFREG | mode) << 16
                archive.writestr(info, contents)

    def test_catalog_template_matches_lifecycle_contract(self) -> None:
        catalog = json.loads(
            (
                WORKSPACE_ROOT
                / "marketplace"
                / "local"
                / ".agents"
                / "plugins"
                / "marketplace.json"
            ).read_text(encoding="utf-8")
        )
        lifecycle = json.loads(
            (WORKSPACE_ROOT / "marketplace" / "local" / "lifecycle.json").read_text(
                encoding="utf-8"
            )
        )
        self.assertEqual(catalog["name"], lifecycle["marketplaceName"])
        self.assertEqual(catalog["plugins"][0]["name"], "lili")
        self.assertEqual(catalog["plugins"][0]["source"]["path"], "./plugins/lili")
        self.assertEqual(lifecycle["pluginSelector"], "lili@lili-local")
        self.assertEqual(
            {operation["command"][-1] for operation in lifecycle["operations"].values()},
            {"add", "remove"},
        )

    def test_final_archive_shape_extracts_with_executable_modes(self) -> None:
        destination = self.root / "extracted"
        manifest = extract_archive(self.archive.resolve(), destination)
        self.assertEqual(manifest["name"], "lili")
        self.assertEqual(manifest["version"], "0.1.0")
        self.assertTrue((destination / "hooks" / "forward").stat().st_mode & stat.S_IXUSR)

    def test_archive_path_escape_is_rejected(self) -> None:
        archive = self.root / "unsafe.zip"
        with zipfile.ZipFile(archive, "w") as output:
            output.writestr("../plugin.json", "{}")
        with self.assertRaisesRegex(MarketplaceRoundTripError, "unsafe"):
            extract_archive(archive.resolve(), self.root / "unsafe")

    @unittest.skipUnless(
        os.environ.get("LILI_RUN_CODEX_MARKETPLACE") == "1",
        "live Codex Marketplace acceptance is opt-in",
    )
    def test_clean_home_live_round_trip(self) -> None:
        codex = shutil.which(os.environ.get("LILI_CODEX", "codex"))
        self.assertIsNotNone(codex, "Codex executable is unavailable")
        result = run_round_trip(WORKSPACE_ROOT, self.archive.resolve(), Path(codex))
        self.assertEqual(result["result"], "passed")
        self.assertEqual(result["releaseVersion"], "0.1.0")
        self.assertEqual(result["derivedUpdateVersion"], "0.1.1")


if __name__ == "__main__":
    unittest.main()
