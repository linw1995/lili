import json
import os
import shutil
import sqlite3
import stat
import sys
import tempfile
import unittest
import zipfile
from contextlib import closing
from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKSPACE_ROOT / "scripts"))

from test_local_marketplace import (
    MarketplaceRoundTripError,
    extract_archive,
    run_round_trip,
)
from test_hook_trust import (
    EXPECTED_NORMALIZED_EVENTS,
    dispatched_spool_events,
    run_hook_trust_round_trip,
)


class HookSpoolTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.database = Path(self.temporary_directory.name) / "lili.sqlite3"
        migration = WORKSPACE_ROOT / "lili-storage/migrations/00000000000001_create_storage/up.sql"
        with closing(sqlite3.connect(self.database)) as connection:
            connection.executescript(migration.read_text(encoding="utf-8"))
            for index, event_type in enumerate(
                [*EXPECTED_NORMALIZED_EVENTS.values(), "turn_started"]
            ):
                event = {
                    "provider": "codex",
                    "eventId": f"event-{index}",
                    "eventType": event_type,
                    "sourceDiscriminator": "plugin:lili@lili-local",
                }
                connection.execute(
                    "INSERT INTO inbound_spool "
                    "(provider, event_id, payload_json, priority, occurred_at_ms, "
                    "inserted_at_ms, status, attempts) VALUES (?, ?, ?, 0, 1000, 1000, 'pending', 0)",
                    ("codex", event["eventId"], json.dumps(event)),
                )
            connection.commit()

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def test_reads_every_lifecycle_event_from_the_application_database(self) -> None:
        events = dispatched_spool_events(self.database)
        self.assertEqual(set(events), set(EXPECTED_NORMALIZED_EVENTS))
        self.assertEqual(len(events["userPromptSubmit"]), 2)
        self.assertEqual(sum(map(len, events.values())), 6)

    def test_missing_lifecycle_event_is_rejected(self) -> None:
        with closing(sqlite3.connect(self.database)) as connection:
            connection.execute("DELETE FROM inbound_spool WHERE event_id = 'event-0'")
            connection.commit()
        with self.assertRaisesRegex(MarketplaceRoundTripError, "event count"):
            dispatched_spool_events(self.database)

    def test_missing_plugin_attribution_is_rejected(self) -> None:
        with closing(sqlite3.connect(self.database)) as connection:
            connection.execute(
                "UPDATE inbound_spool SET payload_json = "
                "json_remove(payload_json, '$.sourceDiscriminator') WHERE event_id = 'event-0'"
            )
            connection.commit()
        with self.assertRaisesRegex(MarketplaceRoundTripError, "plugin attribution"):
            dispatched_spool_events(self.database)

    def test_missing_database_is_not_created(self) -> None:
        self.database.unlink()
        with self.assertRaisesRegex(MarketplaceRoundTripError, "database is missing"):
            dispatched_spool_events(self.database)
        self.assertFalse(self.database.exists())


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

    @unittest.skipUnless(
        os.environ.get("LILI_RUN_CODEX_MARKETPLACE") == "1",
        "live Codex Marketplace acceptance is opt-in",
    )
    def test_live_hook_trust_round_trip(self) -> None:
        codex = shutil.which(os.environ.get("LILI_CODEX", "codex"))
        self.assertIsNotNone(codex, "Codex executable is unavailable")
        result = run_hook_trust_round_trip(
            WORKSPACE_ROOT,
            self.archive.resolve(),
            Path(codex),
        )
        self.assertEqual(result["result"], "passed")
        self.assertFalse(result["bypassUsed"])


if __name__ == "__main__":
    unittest.main()
