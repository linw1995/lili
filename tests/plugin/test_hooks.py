import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
HOOKS_PATH = WORKSPACE_ROOT / "plugins" / "lili" / "hooks" / "hooks.json"
ACCEPTANCE_SOURCE = WORKSPACE_ROOT / "lili" / "src" / "acceptance_marketplace.rs"
HOOK_TRUST_SOURCE = WORKSPACE_ROOT / "scripts" / "test_hook_trust.py"
EXPECTED_EVENTS = {
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "Stop",
    "SessionEnd",
}
EXPECTED_COMMAND = '"${PLUGIN_ROOT}/hooks/forward"'
EXPECTED_WINDOWS_COMMAND = (
    '"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe" '
    '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass '
    '-Command "$input | & (Join-Path $env:PLUGIN_ROOT \'hooks\\forward.ps1\')"'
)


class PluginHooksTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.document = json.loads(HOOKS_PATH.read_text(encoding="utf-8"))

    def test_declares_only_supported_observer_events(self) -> None:
        self.assertEqual(set(self.document), {"description", "hooks"})
        self.assertEqual(set(self.document["hooks"]), EXPECTED_EVENTS)

    def test_every_event_uses_the_confined_launchers(self) -> None:
        for event, groups in self.document["hooks"].items():
            with self.subTest(event=event):
                self.assertEqual(len(groups), 1)
                self.assertEqual(set(groups[0]), {"hooks"})
                self.assertEqual(len(groups[0]["hooks"]), 1)
                handler = groups[0]["hooks"][0]
                self.assertEqual(handler["type"], "command")
                self.assertEqual(handler["command"], EXPECTED_COMMAND)
                self.assertEqual(handler["commandWindows"], EXPECTED_WINDOWS_COMMAND)
                self.assertEqual(handler["command"].count("${PLUGIN_ROOT}"), 1)
                self.assertEqual(handler["commandWindows"].count("$env:PLUGIN_ROOT"), 1)
                self.assertNotIn("powershell.exe -NoLogo", handler["commandWindows"])

    def test_every_event_is_bounded_and_synchronous_on_reviewed_codex(self) -> None:
        for event, groups in self.document["hooks"].items():
            with self.subTest(event=event):
                handler = groups[0]["hooks"][0]
                self.assertEqual(handler["timeout"], 2)
                self.assertIs(handler["async"], False)
                self.assertEqual(handler["statusMessage"], "Forwarding event to Lili")
        session_end = self.document["hooks"]["SessionEnd"][0]["hooks"][0]
        self.assertLessEqual(session_end["timeout"], 3)

    def test_permission_hook_cannot_return_a_decision(self) -> None:
        handler = self.document["hooks"]["PermissionRequest"][0]["hooks"][0]
        self.assertNotIn("decision", handler)
        self.assertNotIn("allow", handler)
        self.assertNotIn("deny", handler)
        self.assertIs(handler["async"], False)

    def test_windows_acceptance_dispatches_the_installed_hook_through_codex(self) -> None:
        acceptance = ACCEPTANCE_SOURCE.read_text(encoding="utf-8")
        hook_trust = HOOK_TRUST_SOURCE.read_text(encoding="utf-8")
        self.assertNotIn('Command::new("powershell.exe")', acceptance)
        self.assertIn('arg("--installed-codex-home")', acceptance)
        self.assertIn('arg("--installed-plugin-root")', acceptance)
        start_turn = hook_trust.index("turn_id = client._start_turn(")
        await_hook = hook_trust.index(
            'run = client._completed_hook(thread_id, "sessionStart", turn_id)'
        )
        self.assertLess(start_turn, await_hook)
        self.assertIn('"SystemRoot": system_root', hook_trust)
        self.assertIn('"LOCALAPPDATA": local_app_data', hook_trust)

    def test_hook_trust_failure_is_reported_on_stderr(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            result = subprocess.run(
                [
                    sys.executable,
                    HOOK_TRUST_SOURCE,
                    "--workspace-root",
                    Path(temporary_directory),
                    "--archive",
                    Path(temporary_directory) / "missing.zip",
                    "--codex",
                    sys.executable,
                ],
                check=False,
                capture_output=True,
            )

        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, b"")
        self.assertIn(b"hook trust round trip failed:", result.stderr)


if __name__ == "__main__":
    unittest.main()
