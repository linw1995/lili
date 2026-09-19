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
        with tempfile.TemporaryDirectory(prefix="lili launcher ") as temporary:
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
            source = root / "forwarder.cs"
            source.write_text(
                """using System;
using System.IO;
using System.Text;

public static class LauncherFixture
{
    public static int Main(string[] arguments)
    {
        if (String.Join(" ", arguments) != "--integration-id lili-session-v1 --plugin-hook --json-stdin")
            return 91;
        Console.InputEncoding = new UTF8Encoding(false);
        File.AppendAllText(CAPTURE_PATH, Console.In.ReadToEnd() + "\\n");
        return 0;
    }
}
""".replace("CAPTURE_PATH", json.dumps(str(capture))),
                encoding="utf-8",
            )
            builder = root / "build.ps1"
            quoted_forwarder = str(forwarder).replace("'", "''")
            quoted_source = str(source).replace("'", "''")
            builder.write_text(
                '$ErrorActionPreference = "Stop"\n'
                f"Add-Type -OutputAssembly '{quoted_forwarder}' "
                "-OutputType ConsoleApplication "
                f"-TypeDefinition (Get-Content -LiteralPath '{quoted_source}' -Raw)\n",
                encoding="utf-8",
            )
            powershell = (
                Path(os.environ["SystemRoot"])
                / "System32"
                / "WindowsPowerShell"
                / "v1.0"
                / "powershell.exe"
            )
            built = subprocess.run(
                [str(powershell), "-NoLogo", "-NoProfile", "-NonInteractive",
                 "-ExecutionPolicy", "Bypass", "-File", str(builder)],
                capture_output=True,
                text=True,
                timeout=60,
            )
            self.assertEqual(built.returncode, 0, built.stdout + built.stderr)
            runner = CodexRunner(executable, root / "host-data", lifecycle["codexVersion"])
            runner.environment["SystemRoot"] = os.environ["SystemRoot"]
            runner.verify_version()
            runner.json(["plugin", "marketplace", "add", str(catalog)])
            installed = runner.json(["plugin", "add", lifecycle["pluginSelector"]])
            installed_root = Path(installed["installedPath"])

            # CLR startup is outside the native forwarder's hook timeout contract.
            subprocess.run(
                [str(installed_root / "bin" / "x86_64-pc-windows-msvc" / "lili-hook.exe"),
                 "--integration-id", "lili-session-v1", "--plugin-hook", "--json-stdin"],
                input=b"{}",
                check=True,
                timeout=60,
            )
            capture.unlink()
            with patch.dict(os.environ, {"LOCALAPPDATA": str(application_home)}):
                result = dispatch_installed_plugin_hook(
                    WORKSPACE_ROOT, executable, runner.codex_home, installed_root, project
                )
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
