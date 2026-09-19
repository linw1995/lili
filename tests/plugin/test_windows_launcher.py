import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKSPACE_ROOT / "scripts"))

from test_hook_trust import dispatch_installed_plugin_hook
from test_local_marketplace import CodexRunner, load_json, resolve_executable


@unittest.skipUnless(os.name == "nt", "requires Windows PowerShell")
class WindowsLauncherTests(unittest.TestCase):
    def test_installed_hook_forwards_stdin_through_the_host_shell(self) -> None:
        lifecycle = load_json(WORKSPACE_ROOT / "marketplace" / "local" / "lifecycle.json")
        executable = resolve_executable(os.environ.get("CODEX_BIN", "codex"))
        with tempfile.TemporaryDirectory(prefix="lili launcher ", ignore_cleanup_errors=True) as temporary:
            root = Path(temporary).resolve()
            catalog = root / "catalog"
            plugin = catalog / "plugins" / "lili"
            capture = root / "events.jsonl"
            project = root / "project with spaces"
            application_home = root / "application-home"
            project.mkdir()
            application_home.mkdir()
            shutil.copytree(WORKSPACE_ROOT / "marketplace" / "local", catalog)
            shutil.copytree(WORKSPACE_ROOT / "plugins" / "lili", plugin)
            forwarder = plugin / "bin" / "x86_64-pc-windows-msvc" / "lili-hook.exe"
            forwarder.parent.mkdir(parents=True)
            started = root / "forwarder.started"
            source = root / "forwarder.rs"
            source.write_text(
                """use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if arguments != "--integration-id lili-session-v1 --plugin-hook --json-stdin" {
        return Err("unexpected forwarder arguments".into());
    }
    std::fs::write(STARTED_PATH, b"started")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut input = String::new();
        let result = std::io::stdin().read_to_string(&mut input).map(|_| input);
        let _ = sender.send(result);
    });
    let input = receiver.recv_timeout(std::time::Duration::from_secs(5))??;
    let mut output = std::fs::OpenOptions::new().create(true).append(true).open(CAPTURE_PATH)?;
    writeln!(output, "{}", input)?;
    Ok(())
}
""".replace("CAPTURE_PATH", json.dumps(str(capture), ensure_ascii=False))
                .replace("STARTED_PATH", json.dumps(str(started), ensure_ascii=False)),
                encoding="utf-8",
            )
            built = subprocess.run(
                ["rustc", "--edition=2021", str(source), "-o", str(forwarder)],
                capture_output=True,
                text=True,
                timeout=90,
            )
            self.assertEqual(built.returncode, 0, built.stdout + built.stderr)
            runner = CodexRunner(executable, root / "host-data", lifecycle["codexVersion"])
            runner.environment["SystemRoot"] = os.environ["SystemRoot"]
            runner.verify_version()
            runner.json(["plugin", "marketplace", "add", str(catalog)])
            installed = runner.json(["plugin", "add", lifecycle["pluginSelector"]])
            installed_root = Path(installed["installedPath"])

            # Native acceptance also checks the installed executable before dispatch.
            subprocess.run(
                [str(installed_root / "bin" / "x86_64-pc-windows-msvc" / "lili-hook.exe"),
                 "--integration-id", "lili-session-v1", "--plugin-hook", "--json-stdin"],
                input=b"{}",
                check=True,
                timeout=60,
            )
            capture.unlink()
            started.unlink()
            hooks = load_json(installed_root / "hooks" / "hooks.json")
            command = hooks["hooks"]["SessionStart"][0]["hooks"][0]["commandWindows"]
            payload = {"hook_event_name": "SessionStart", "cwd": str(project), "label": "caf\u00e9"}
            legacy_environment = {
                **runner.environment,
                "LOCALAPPDATA": str(application_home),
                "PLUGIN_ROOT": str(installed_root),
                "PLUGIN_DATA": str(runner.codex_home / "plugins" / "data" / "lili-lili-local"),
                "PSExecutionPolicyPreference": "Restricted",
                "TEMP": str(root),
                "TMP": str(root),
            }
            powershell = Path(os.environ["SystemRoot"]) / "System32" / "WindowsPowerShell" / "v1.0" / "powershell.exe"
            legacy = subprocess.run(
                [str(powershell), "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", command],
                input=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
                env=legacy_environment,
                capture_output=True,
                timeout=10,
            )
            self.assertEqual(legacy.returncode, 0, legacy.stderr.decode("utf-8", errors="replace"))
            self.assertEqual(legacy.stdout, b"")
            self.assertEqual(legacy.stderr, b"")
            self.assertEqual(json.loads(capture.read_text(encoding="utf-8")), payload)
            capture.unlink()
            started.unlink()
            with patch.dict(os.environ, {"LOCALAPPDATA": str(application_home)}):
                try:
                    result = dispatch_installed_plugin_hook(
                        WORKSPACE_ROOT, executable, runner.codex_home, installed_root, project
                    )
                except Exception:
                    print(f"fixture_started={started.exists()}; payload_captured={capture.exists()}", file=sys.stderr)
                    raise
            self.assertEqual(result["result"], "passed")
            self.assertIs(result["bypassUsed"], False)
            events = [
                json.loads(line)
                for line in capture.read_text(encoding="utf-8").splitlines()
                if line.strip()
            ]
            starts = [event for event in events if event["hook_event_name"] == "SessionStart"]
            self.assertEqual(len(starts), 1)
            self.assertTrue(os.path.samefile(starts[0]["cwd"], project))


if __name__ == "__main__":
    unittest.main()
