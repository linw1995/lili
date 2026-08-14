import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKSPACE_ROOT / "scripts"))

from build_plugin_archive import TARGETS, ArchiveError, build_archive


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class PluginArchiveTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name) / "workspace"
        self.root.mkdir()
        shutil.copy2(WORKSPACE_ROOT / "Cargo.toml", self.root / "Cargo.toml")
        (self.root / "lili").mkdir()
        shutil.copy2(
            WORKSPACE_ROOT / "lili" / "tauri.conf.json",
            self.root / "lili" / "tauri.conf.json",
        )
        shutil.copytree(WORKSPACE_ROOT / "plugins", self.root / "plugins")
        (self.root / "marketplace" / "lili").mkdir(parents=True)
        for name in ("package-policy.json", "submission.json"):
            shutil.copy2(
                WORKSPACE_ROOT / "marketplace" / "lili" / name,
                self.root / "marketplace" / "lili" / name,
            )
        self.forwarders = Path(self.temporary_directory.name) / "forwarders"
        for target, target_policy in TARGETS.items():
            target_root = self.forwarders / target
            target_root.mkdir(parents=True)
            binary = target_root / target_policy["fileName"]
            binary.write_bytes(target_policy["magics"][0] + f"lili-{target}".encode())
            binary.chmod(0o755)
            contents = binary.read_bytes()
            (target_root / "manifest.json").write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "product": "Lili",
                        "component": "lili-hook",
                        "version": "0.1.0",
                        "reportedVersion": "0.1.0",
                        "platform": target,
                        "fileName": target_policy["fileName"],
                        "signatureKind": "platform-standard",
                        "size": len(contents),
                        "sha256": hashlib.sha256(contents).hexdigest(),
                    }
                ),
                encoding="utf-8",
            )

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def test_archive_is_complete_and_byte_reproducible(self) -> None:
        first = Path(self.temporary_directory.name) / "first.zip"
        second = Path(self.temporary_directory.name) / "second.zip"
        first_manifest = build_archive(self.root, self.forwarders, first)
        second_manifest = build_archive(self.root, self.forwarders, second)
        self.assertEqual(first.read_bytes(), second.read_bytes())
        self.assertEqual(first_manifest["archiveSha256"], digest(first))
        self.assertEqual(
            first_manifest["archiveSha256"], second_manifest["archiveSha256"]
        )

        policy = json.loads(
            (self.root / "marketplace" / "lili" / "package-policy.json").read_text(
                encoding="utf-8"
            )
        )
        with zipfile.ZipFile(first) as archive:
            self.assertEqual(archive.namelist(), sorted(policy["allowedPackageFiles"]))
            self.assertTrue(
                all(
                    info.date_time == (1980, 1, 1, 0, 0, 0)
                    for info in archive.infolist()
                )
            )
            self.assertTrue(
                all(
                    info.compress_type == zipfile.ZIP_DEFLATED
                    for info in archive.infolist()
                )
            )
            self.assertEqual(archive.testzip(), None)

    def test_tampered_forwarder_is_rejected(self) -> None:
        binary = self.forwarders / "arm64-apple-darwin" / "lili-hook"
        contents = bytearray(binary.read_bytes())
        contents[-1] ^= 1
        binary.write_bytes(contents)
        with self.assertRaisesRegex(ArchiveError, "checksum drifted"):
            build_archive(
                self.root,
                self.forwarders,
                Path(self.temporary_directory.name) / "tampered.zip",
            )

    def test_native_manifest_writer_records_reported_version(self) -> None:
        host_target = {
            ("Darwin", "arm64"): "arm64-apple-darwin",
            ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
        }.get((platform.system(), platform.machine()))
        if host_target is None:
            self.skipTest("native manifest writer fixture is unavailable on this host")
        with tempfile.TemporaryDirectory(
            prefix="lili forwarder manifest "
        ) as directory:
            root = Path(directory)
            binary = root / "lili-hook"
            binary.write_text(
                "#!/bin/sh\nprintf 'lili-hook 0.1.0\\n'\n", encoding="utf-8"
            )
            binary.chmod(0o755)
            output = root / "manifest.json"
            result = subprocess.run(
                [
                    "node",
                    str(WORKSPACE_ROOT / "scripts" / "write-forwarder-manifest.mjs"),
                    str(binary),
                    str(output),
                    "0.1.0",
                    host_target,
                    "platform-standard",
                ],
                capture_output=True,
                check=False,
                env=os.environ.copy(),
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            manifest = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(manifest["version"], "0.1.0")
            self.assertEqual(manifest["reportedVersion"], "0.1.0")
            self.assertEqual(manifest["sha256"], digest(binary))


if __name__ == "__main__":
    unittest.main()
