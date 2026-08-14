import json
import os
import platform
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
PLUGIN_ROOT = WORKSPACE_ROOT / "plugins" / "lili"
POSIX_LAUNCHER = PLUGIN_ROOT / "hooks" / "forward"
WINDOWS_LAUNCHER = PLUGIN_ROOT / "hooks" / "forward.ps1"
SUPPORTED_POSIX_TARGETS = {
    ("Darwin", "arm64"): "arm64-apple-darwin",
    ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
}


class PluginLauncherContractTests(unittest.TestCase):
    def test_posix_launcher_uses_only_declared_targets(self) -> None:
        policy = json.loads(
            (WORKSPACE_ROOT / "marketplace" / "lili" / "package-policy.json").read_text(
                encoding="utf-8"
            )
        )
        declared = set(policy["declaredExecutables"])
        self.assertIn("hooks/forward", declared)
        for target in SUPPORTED_POSIX_TARGETS.values():
            self.assertIn(f"bin/{target}/lili-hook", declared)
        self.assertIn("bin/x86_64-pc-windows-msvc/lili-hook.exe", declared)

    def test_unsupported_hosts_have_no_fallback_target(self) -> None:
        posix = POSIX_LAUNCHER.read_text(encoding="utf-8")
        windows = WINDOWS_LAUNCHER.read_text(encoding="utf-8")
        self.assertIn("Darwin:arm64)", posix)
        self.assertIn("Linux:x86_64)", posix)
        self.assertIn(
            '*)\n        fail "Lili plugin does not support this host" 64', posix
        )
        self.assertNotIn("Darwin:x86_64)", posix)
        self.assertNotIn("Linux:aarch64)", posix)
        self.assertIn(
            "$architecture -ne [Runtime.InteropServices.Architecture]::X64", windows
        )
        self.assertIn(
            'Fail-LiliLauncher "Lili plugin does not support this host" 64', windows
        )

    def test_launchers_do_not_construct_commands_from_stdin(self) -> None:
        posix = POSIX_LAUNCHER.read_text(encoding="utf-8")
        windows = WINDOWS_LAUNCHER.read_text(encoding="utf-8")
        for forbidden in ("eval ", "source ", "curl ", "wget ", "read "):
            self.assertNotIn(forbidden, posix)
        for forbidden in (
            "Invoke-Expression",
            "Invoke-WebRequest",
            "Start-Process",
            "ReadToEnd",
            "Get-Content",
        ):
            self.assertNotIn(forbidden, windows)
        self.assertIn(
            'exec "$forwarder" --integration-id lili-session-v1 --plugin-hook --json-stdin',
            posix,
        )
        self.assertIn("$OutputEncoding = [Text.UTF8Encoding]::new($false)", windows)
        self.assertIn(
            '$input | & $forwarderPath --integration-id "lili-session-v1" --plugin-hook --json-stdin',
            windows,
        )

    def test_posix_launcher_preserves_stdin_with_spaces_in_root(self) -> None:
        target = SUPPORTED_POSIX_TARGETS.get((platform.system(), platform.machine()))
        if target is None:
            self.skipTest("current host is outside the published POSIX target matrix")
        with tempfile.TemporaryDirectory(
            prefix="lili plugin launcher "
        ) as temporary_directory:
            plugin_root = Path(temporary_directory) / "package with spaces"
            launcher = plugin_root / "hooks" / "forward"
            launcher.parent.mkdir(parents=True)
            shutil.copy2(POSIX_LAUNCHER, launcher)
            launcher.chmod(0o755)
            forwarder = plugin_root / "bin" / target / "lili-hook"
            forwarder.parent.mkdir(parents=True)
            forwarder.write_text(
                "#!/bin/sh\n"
                'test "$1" = "--integration-id" || exit 91\n'
                'test "$2" = "lili-session-v1" || exit 92\n'
                'test "$3" = "--plugin-hook" || exit 93\n'
                'test "$4" = "--json-stdin" || exit 94\n'
                "/bin/cat\n",
                encoding="utf-8",
            )
            forwarder.chmod(0o755)
            environment = os.environ.copy()
            environment["PLUGIN_ROOT"] = str(plugin_root)
            payload = b'{"text":"$(touch should-not-exist)"}\n'
            result = subprocess.run(
                [str(launcher)],
                input=payload,
                capture_output=True,
                check=False,
                cwd=plugin_root,
                env=environment,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, payload)
            self.assertEqual(result.stderr, b"")
            self.assertFalse((plugin_root / "should-not-exist").exists())

    def test_posix_launcher_resolves_uname_before_restricting_path(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="lili plugin host utility "
        ) as temporary_directory:
            root = Path(temporary_directory)
            plugin_root = root / "package"
            launcher = plugin_root / "hooks" / "forward"
            launcher.parent.mkdir(parents=True)
            shutil.copy2(POSIX_LAUNCHER, launcher)
            launcher.chmod(0o755)
            forwarder = (
                plugin_root / "bin" / "x86_64-unknown-linux-gnu" / "lili-hook"
            )
            forwarder.parent.mkdir(parents=True)
            forwarder.write_text("#!/bin/sh\n/bin/cat\n", encoding="utf-8")
            forwarder.chmod(0o755)
            utility_root = root / "host utilities"
            utility_root.mkdir()
            uname = utility_root / "uname"
            uname.write_text(
                "#!/bin/sh\n"
                'case "$1" in\n'
                "    -s) printf 'Linux\\n' ;;\n"
                "    -m) printf 'x86_64\\n' ;;\n"
                "    *) exit 1 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            uname.chmod(0o755)
            environment = os.environ.copy()
            environment["PATH"] = str(utility_root)
            environment["PLUGIN_ROOT"] = str(plugin_root)
            payload = b"{}\n"
            result = subprocess.run(
                [str(launcher)],
                input=payload,
                capture_output=True,
                check=False,
                env=environment,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, payload)
            self.assertEqual(result.stderr, b"")

    def test_posix_launcher_fails_closed_without_packaged_target(self) -> None:
        target = SUPPORTED_POSIX_TARGETS.get((platform.system(), platform.machine()))
        if target is None:
            self.skipTest("current host is outside the published POSIX target matrix")
        with tempfile.TemporaryDirectory(
            prefix="lili plugin missing target "
        ) as temporary_directory:
            plugin_root = Path(temporary_directory) / "package"
            launcher = plugin_root / "hooks" / "forward"
            launcher.parent.mkdir(parents=True)
            shutil.copy2(POSIX_LAUNCHER, launcher)
            launcher.chmod(0o755)
            environment = os.environ.copy()
            environment["PLUGIN_ROOT"] = str(plugin_root)
            started = time.monotonic()
            result = subprocess.run(
                [str(launcher)],
                input=b"{}\n",
                capture_output=True,
                check=False,
                env=environment,
            )
            elapsed = time.monotonic() - started
            self.assertEqual(result.returncode, 66)
            self.assertEqual(result.stdout, b"")
            self.assertEqual(
                result.stderr, b"Lili plugin forwarder is missing or invalid\n"
            )
            self.assertLess(elapsed, 1.0)

    def test_posix_launcher_rejects_symlinked_forwarder(self) -> None:
        target = SUPPORTED_POSIX_TARGETS.get((platform.system(), platform.machine()))
        if target is None:
            self.skipTest("current host is outside the published POSIX target matrix")
        with tempfile.TemporaryDirectory(
            prefix="lili plugin symlink target "
        ) as temporary_directory:
            plugin_root = Path(temporary_directory) / "package"
            launcher = plugin_root / "hooks" / "forward"
            launcher.parent.mkdir(parents=True)
            shutil.copy2(POSIX_LAUNCHER, launcher)
            launcher.chmod(0o755)
            external = Path(temporary_directory) / "external-hook"
            external.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            external.chmod(0o755)
            forwarder = plugin_root / "bin" / target / "lili-hook"
            forwarder.parent.mkdir(parents=True)
            forwarder.symlink_to(external)
            environment = os.environ.copy()
            environment["PLUGIN_ROOT"] = str(plugin_root)
            result = subprocess.run(
                [str(launcher)],
                input=b"{}\n",
                capture_output=True,
                check=False,
                env=environment,
            )
            self.assertEqual(result.returncode, 66)
            self.assertEqual(result.stdout, b"")
            self.assertEqual(
                result.stderr, b"Lili plugin forwarder is missing or invalid\n"
            )


if __name__ == "__main__":
    unittest.main()
