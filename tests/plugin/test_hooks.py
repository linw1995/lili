import json
import unittest
from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
HOOKS_PATH = WORKSPACE_ROOT / "plugins" / "lili" / "hooks" / "hooks.json"
EXPECTED_EVENTS = {
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "Stop",
    "SessionEnd",
}
EXPECTED_COMMAND = '"${PLUGIN_ROOT}/hooks/forward"'
EXPECTED_WINDOWS_COMMAND = (
    'powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass '
    '-File "${PLUGIN_ROOT}\\hooks\\forward.ps1"'
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
                self.assertEqual(handler["commandWindows"].count("${PLUGIN_ROOT}"), 1)

    def test_every_event_is_bounded_and_synchronous_on_reviewed_codex(self) -> None:
        for event, groups in self.document["hooks"].items():
            with self.subTest(event=event):
                handler = groups[0]["hooks"][0]
                self.assertEqual(handler["timeout"], 1)
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


if __name__ == "__main__":
    unittest.main()
