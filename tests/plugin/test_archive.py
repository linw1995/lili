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
from inspect_plugin_release import InspectionError, inspect_release, scan_entry


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def forwarder_fixture(target: str) -> bytes:
    if target == "arm64-apple-darwin":
        contents = bytearray(32)
        contents[:4] = b"\xcf\xfa\xed\xfe"
        contents[4:8] = (0x0100000C).to_bytes(4, "little")
        return bytes(contents)
    if target == "x86_64-unknown-linux-gnu":
        contents = bytearray(64)
        contents[:7] = b"\x7fELF\x02\x01\x01"
        contents[18:20] = (62).to_bytes(2, "little")
        return bytes(contents)
    if target == "x86_64-pc-windows-msvc":
        contents = bytearray(128)
        contents[:2] = b"MZ"
        contents[0x3C:0x40] = (64).to_bytes(4, "little")
        contents[64:68] = b"PE\0\0"
        contents[68:70] = (0x8664).to_bytes(2, "little")
        return bytes(contents)
    raise AssertionError(f"unsupported test target: {target}")


def supply_chain_fixture() -> dict:
    return {
        "schemaVersion": 1,
        "product": "Lili",
        "component": "plugin",
        "version": "0.1.0",
        "generatedAt": "2026-08-14T00:00:00Z",
        "lockfile": {"path": "Cargo.lock", "sha256": "1" * 64},
        "dependencyInventory": {
            "scope": "test",
            "packageCount": 1,
            "packages": [
                {
                    "name": "lili",
                    "version": "0.1.0",
                    "source": "workspace",
                    "checksum": None,
                    "license": "Apache-2.0",
                }
            ],
        },
        "licensePolicy": {
            "result": "passed",
            "tool": "cargo-deny 0.19.0",
            "configuration": "deny.toml",
            "configurationSha256": "2" * 64,
        },
        "vulnerabilityScan": {
            "tool": "cargo-audit 0.22.1",
            "result": "passed",
            "vulnerabilityCount": 0,
            "databaseAdvisoryCount": 1,
            "databaseCommit": "abc123",
            "databaseUpdatedAt": "2026-08-14T00:00:00Z",
            "informationalWarningCount": 0,
            "informationalWarnings": [],
        },
    }


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
            binary.write_bytes(forwarder_fixture(target))
            binary.chmod(0o755)
            contents = binary.read_bytes()
            (target_root / "manifest.json").write_text(
                json.dumps(
                    {
                        "schemaVersion": 2,
                        "product": "Lili",
                        "component": "lili-hook",
                        "version": "0.1.0",
                        "reportedVersion": "0.1.0",
                        "platform": target,
                        "fileName": target_policy["fileName"],
                        "signatureKind": "platform-standard",
                        "signatureVerifier": {
                            "arm64-apple-darwin": "codesign --verify --strict",
                            "x86_64-unknown-linux-gnu": "ELF format and SHA-256 integrity",
                            "x86_64-pc-windows-msvc": "Get-AuthenticodeSignature",
                        }[target],
                        "signatureStatus": {
                            "arm64-apple-darwin": "unsigned-allowed",
                            "x86_64-unknown-linux-gnu": "not-applicable",
                            "x86_64-pc-windows-msvc": "unsigned-allowed",
                        }[target],
                        "size": len(contents),
                        "sha256": hashlib.sha256(contents).hexdigest(),
                    }
                ),
                encoding="utf-8",
            )
        self.supply_chain = Path(self.temporary_directory.name) / "supply-chain.json"
        self.supply_chain.write_text(
            json.dumps(supply_chain_fixture()), encoding="utf-8"
        )

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def test_archive_is_complete_and_byte_reproducible(self) -> None:
        first = Path(self.temporary_directory.name) / "first.zip"
        second = Path(self.temporary_directory.name) / "second.zip"
        first_manifest = build_archive(
            self.root, self.forwarders, first, self.supply_chain
        )
        second_manifest = build_archive(
            self.root, self.forwarders, second, self.supply_chain
        )
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

    def test_final_release_inspection_binds_supply_chain_and_archive(self) -> None:
        archive = Path(self.temporary_directory.name) / "lili-plugin-0.1.0.zip"
        manifest = build_archive(
            self.root, self.forwarders, archive, self.supply_chain
        )
        result = inspect_release(
            archive.resolve(),
            archive.with_suffix(".manifest.json").resolve(),
            archive.with_suffix(".zip.sha256").resolve(),
            self.supply_chain.resolve(),
        )
        self.assertEqual(result["sha256"], manifest["archiveSha256"])
        self.assertEqual(result["result"], "passed")
        self.supply_chain.write_text(
            self.supply_chain.read_text(encoding="utf-8") + "\n", encoding="utf-8"
        )
        with self.assertRaisesRegex(InspectionError, "supply-chain reference drifted"):
            inspect_release(
                archive.resolve(),
                archive.with_suffix(".manifest.json").resolve(),
                archive.with_suffix(".zip.sha256").resolve(),
                self.supply_chain.resolve(),
            )

    def test_final_release_scan_rejects_secrets_paths_and_urls(self) -> None:
        cases = [
            (b"-----BEGIN PRIVATE KEY-----", "private key"),
            (b"/home/runner/work/lili/lili", "GitHub Linux workspace"),
            (b"tests/fixtures/private.json", "private fixture marker"),
            (b"https://collector.invalid/upload", "undeclared network URL"),
        ]
        for payload, message in cases:
            with self.subTest(message=message):
                with self.assertRaisesRegex(InspectionError, message):
                    scan_entry("bin/test", payload, set())

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
                self.supply_chain,
            )

    def test_wrong_forwarder_architecture_is_rejected(self) -> None:
        wrong_machines = {
            "arm64-apple-darwin": (4, 8, (0x01000007).to_bytes(4, "little")),
            "x86_64-unknown-linux-gnu": (18, 20, (183).to_bytes(2, "little")),
            "x86_64-pc-windows-msvc": (68, 70, (0xAA64).to_bytes(2, "little")),
        }
        for target, (start, end, machine) in wrong_machines.items():
            with self.subTest(target=target):
                target_root = self.forwarders / target
                binary = target_root / TARGETS[target]["fileName"]
                original = binary.read_bytes()
                contents = bytearray(original)
                contents[start:end] = machine
                binary.write_bytes(contents)
                manifest_path = target_root / "manifest.json"
                manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
                manifest["sha256"] = hashlib.sha256(contents).hexdigest()
                manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
                with self.assertRaisesRegex(ArchiveError, "architecture"):
                    build_archive(
                        self.root,
                        self.forwarders,
                        Path(self.temporary_directory.name) / f"wrong-{target}.zip",
                        self.supply_chain,
                    )
                binary.write_bytes(original)
                manifest["sha256"] = hashlib.sha256(original).hexdigest()
                manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

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
            self.assertEqual(manifest["schemaVersion"], 2)
            self.assertEqual(
                manifest["signatureVerifier"],
                {
                    "arm64-apple-darwin": "codesign --verify --strict",
                    "x86_64-unknown-linux-gnu": "ELF format and SHA-256 integrity",
                }[host_target],
            )


if __name__ == "__main__":
    unittest.main()
