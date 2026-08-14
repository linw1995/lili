import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKSPACE_ROOT / "scripts"))

from check_plugin_package import PolicyViolation, validate_workspace


def valid_hooks() -> dict:
    handler = {
        "type": "command",
        "command": '"${PLUGIN_ROOT}/hooks/forward"',
        "commandWindows": (
            '"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe" '
            '-File "${PLUGIN_ROOT}\\hooks\\forward.ps1"'
        ),
        "timeout": 1,
        "async": True,
    }
    return {
        "hooks": {
            event: [{"hooks": [handler]}]
            for event in (
                "SessionStart",
                "UserPromptSubmit",
                "PermissionRequest",
                "Stop",
                "SessionEnd",
            )
        }
    }


class PluginPackagePolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        shutil.copy2(WORKSPACE_ROOT / "Cargo.toml", self.root / "Cargo.toml")
        (self.root / "lili").mkdir()
        shutil.copy2(
            WORKSPACE_ROOT / "lili" / "tauri.conf.json",
            self.root / "lili" / "tauri.conf.json",
        )
        (self.root / "marketplace" / "lili").mkdir(parents=True)
        for name in ("package-policy.json", "submission.json"):
            shutil.copy2(
                WORKSPACE_ROOT / "marketplace" / "lili" / name,
                self.root / "marketplace" / "lili" / name,
            )
        shutil.copytree(WORKSPACE_ROOT / "plugins", self.root / "plugins")
        hooks_path = self.root / "plugins" / "lili" / "hooks" / "hooks.json"
        hooks_path.parent.mkdir(exist_ok=True)
        hooks_path.write_text(json.dumps(valid_hooks()), encoding="utf-8")
        forward = hooks_path.parent / "forward"
        forward.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        forward.chmod(0o755)
        (hooks_path.parent / "forward.ps1").write_text("exit 0\n", encoding="utf-8")

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def manifest(self) -> tuple[Path, dict]:
        path = self.root / "plugins" / "lili" / ".codex-plugin" / "plugin.json"
        return path, json.loads(path.read_text(encoding="utf-8"))

    def write_manifest(self, manifest: dict) -> None:
        path, _ = self.manifest()
        path.write_text(json.dumps(manifest), encoding="utf-8")

    def assert_rejected(self, expected: str) -> None:
        with self.assertRaisesRegex(PolicyViolation, expected):
            validate_workspace(self.root)

    def test_current_complete_shape_passes(self) -> None:
        validate_workspace(self.root)

    def test_bare_windows_interpreter_is_rejected(self) -> None:
        hooks_path = self.root / "plugins" / "lili" / "hooks" / "hooks.json"
        hooks = json.loads(hooks_path.read_text(encoding="utf-8"))
        for groups in hooks["hooks"].values():
            groups[0]["hooks"][0]["commandWindows"] = (
                'powershell.exe -File "${PLUGIN_ROOT}\\hooks\\forward.ps1"'
            )
        hooks_path.write_text(json.dumps(hooks), encoding="utf-8")

        self.assert_rejected("trusted absolute PowerShell path")

    def test_path_escape_is_rejected(self) -> None:
        _, manifest = self.manifest()
        manifest["skills"] = "./skills/../../outside"
        self.write_manifest(manifest)
        self.assert_rejected("escapes plugin root")

    def test_missing_declared_file_is_rejected(self) -> None:
        (self.root / "plugins" / "lili" / "assets" / "logo.png").unlink()
        self.assert_rejected("does not exist")

    def test_placeholder_is_rejected(self) -> None:
        skill = self.root / "plugins" / "lili" / "skills" / "lili-setup" / "SKILL.md"
        skill.write_text(skill.read_text(encoding="utf-8") + "\nTODO\n", encoding="utf-8")
        self.assert_rejected("placeholder token")

    def test_metadata_drift_is_rejected(self) -> None:
        _, manifest = self.manifest()
        manifest["description"] = "Drifted description"
        self.write_manifest(manifest)
        self.assert_rejected("submission metadata drifted")

    def test_endorsement_claim_is_rejected(self) -> None:
        skill = self.root / "plugins" / "lili" / "skills" / "lili-setup" / "SKILL.md"
        skill.write_text(
            skill.read_text(encoding="utf-8") + "\nOpenAI Verified\n",
            encoding="utf-8",
        )
        self.assert_rejected("prohibited endorsement")

    def test_undeclared_executable_is_rejected(self) -> None:
        rogue = self.root / "plugins" / "lili" / "bin" / "rogue"
        rogue.parent.mkdir()
        rogue.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        rogue.chmod(0o755)
        self.assert_rejected("undeclared executable")

    def test_mcp_configuration_is_rejected(self) -> None:
        _, manifest = self.manifest()
        manifest["mcpServers"] = {}
        self.write_manifest(manifest)
        self.assert_rejected("forbidden manifest configuration")

    def test_ui_configuration_is_rejected(self) -> None:
        _, manifest = self.manifest()
        manifest["interface"]["screenshots"] = []
        self.write_manifest(manifest)
        self.assert_rejected("forbidden manifest configuration")

    def test_authentication_configuration_is_rejected(self) -> None:
        auth = self.root / "plugins" / "lili" / "auth.json"
        auth.write_text("{}", encoding="utf-8")
        self.assert_rejected("forbidden plugin configuration")

    def test_network_configuration_is_rejected(self) -> None:
        skill = self.root / "plugins" / "lili" / "skills" / "lili-setup" / "SKILL.md"
        skill.write_text(
            skill.read_text(encoding="utf-8") + "\nhttps://remote.invalid/events\n",
            encoding="utf-8",
        )
        self.assert_rejected("undeclared network endpoint")


if __name__ == "__main__":
    unittest.main()
