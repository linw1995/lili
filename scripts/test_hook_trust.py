#!/usr/bin/env python3

import argparse
import json
import selectors
import shutil
import subprocess
import tempfile
from pathlib import Path

from test_local_marketplace import (
    CodexRunner,
    MarketplaceRoundTripError,
    extract_archive,
    load_json,
    next_patch_version,
    replace_catalog_plugin,
    require,
    resolve_executable,
)


EXPECTED_EVENTS = {
    "permissionRequest",
    "sessionStart",
    "sessionEnd",
    "userPromptSubmit",
    "stop",
}
MAX_MESSAGE_BYTES = 1024 * 1024


class AppServerClient:
    def __init__(self, runner: CodexRunner, cwd: Path):
        self.process = subprocess.Popen(
            [str(runner.executable), "app-server", "--stdio"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=runner.environment,
        )
        require(
            self.process.stdin is not None
            and self.process.stdout is not None
            and self.process.stderr is not None,
            "Codex app-server pipes are unavailable",
        )
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.next_identifier = 1
        self._send(
            {
                "id": 0,
                "method": "initialize",
                "params": {
                    "clientInfo": {"name": "lili-marketplace-acceptance", "version": "0.1.0"},
                    "capabilities": {"experimentalApi": True},
                },
            }
        )
        self._response(0)
        self._send({"method": "initialized", "params": {}})
        self.cwd = cwd

    def close(self) -> None:
        self.selector.close()
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            if stream is not None:
                stream.close()

    def request(self, method: str, params: dict) -> dict:
        identifier = self.next_identifier
        self.next_identifier += 1
        self._send({"id": identifier, "method": method, "params": params})
        return self._response(identifier)

    def hooks(self) -> list[dict]:
        response = self.request("hooks/list", {"cwds": [str(self.cwd)]})
        data = response.get("data")
        require(isinstance(data, list) and len(data) == 1, "hooks/list returned an invalid workspace set")
        entry = data[0]
        require(entry.get("errors") == [], "hooks/list reported a hook error")
        require(entry.get("warnings") == [], "hooks/list reported a hook warning")
        hooks = entry.get("hooks")
        require(isinstance(hooks, list), "hooks/list omitted hooks")
        return hooks

    def trust(self, hooks: list[dict]) -> None:
        edits = [
            {
                "keyPath": f"hooks.state.{json.dumps(hook['key'])}",
                "value": {"enabled": True, "trusted_hash": hook["currentHash"]},
                "mergeStrategy": "upsert",
            }
            for hook in hooks
        ]
        response = self.request(
            "config/batchWrite",
            {"edits": edits, "reloadUserConfig": True},
        )
        require(response.get("status") == "ok", "Codex rejected the explicit hook trust write")

    def _send(self, message: dict) -> None:
        require(self.process.stdin is not None, "Codex app-server stdin is closed")
        payload = json.dumps(message, separators=(",", ":")).encode() + b"\n"
        require(len(payload) <= MAX_MESSAGE_BYTES, "Codex app-server request is too large")
        try:
            self.process.stdin.write(payload)
            self.process.stdin.flush()
        except OSError as error:
            raise MarketplaceRoundTripError("Codex app-server request failed") from error

    def _response(self, identifier: int) -> dict:
        while True:
            events = self.selector.select(timeout=10)
            require(bool(events), "Codex app-server response timed out")
            require(self.process.stdout is not None, "Codex app-server stdout is closed")
            line = self.process.stdout.readline(MAX_MESSAGE_BYTES + 1)
            require(0 < len(line) <= MAX_MESSAGE_BYTES, "Codex app-server response is empty or too large")
            try:
                message = json.loads(line)
            except json.JSONDecodeError as error:
                raise MarketplaceRoundTripError("Codex app-server returned invalid JSON") from error
            if message.get("id") != identifier:
                continue
            require("error" not in message, f"Codex app-server request failed: {message.get('error')}")
            result = message.get("result")
            require(isinstance(result, dict), "Codex app-server response omitted its result")
            return result


def hook_map(hooks: list[dict], selector: str, expected_status: str) -> dict[str, dict]:
    require(len(hooks) == len(EXPECTED_EVENTS), "Codex did not load every Lili plugin hook")
    by_event = {hook.get("eventName"): hook for hook in hooks}
    require(set(by_event) == EXPECTED_EVENTS, "Codex plugin hook event set drifted")
    for event, hook in by_event.items():
        require(hook.get("source") == "plugin", f"{event} hook source is not plugin")
        require(hook.get("pluginId") == selector, f"{event} hook plugin identity drifted")
        require(hook.get("enabled") is True, f"{event} hook is unexpectedly disabled")
        require(hook.get("trustStatus") == expected_status, f"{event} hook trust status is not {expected_status}")
        require(isinstance(hook.get("currentHash"), str), f"{event} hook hash is missing")
    return by_event


def changed_snapshot(source: Path, destination: Path, version: str) -> None:
    shutil.copytree(source, destination)
    manifest_path = destination / ".codex-plugin" / "plugin.json"
    manifest = load_json(manifest_path)
    manifest["version"] = version
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    hooks_path = destination / "hooks" / "hooks.json"
    hooks = load_json(hooks_path)
    for groups in hooks["hooks"].values():
        groups[0]["hooks"][0]["statusMessage"] = "Forwarding updated event to Lili"
    hooks_path.write_text(json.dumps(hooks, indent=2) + "\n", encoding="utf-8")


def run_hook_trust_round_trip(
    workspace_root: Path,
    archive_path: Path,
    codex_executable: Path,
) -> dict:
    workspace_root = workspace_root.resolve()
    lifecycle = load_json(workspace_root / "marketplace" / "local" / "lifecycle.json")
    selector = lifecycle["pluginSelector"]
    marketplace_name = lifecycle["marketplaceName"]

    with tempfile.TemporaryDirectory(prefix="lili-hook-trust-") as temporary_directory:
        temporary_root = Path(temporary_directory)
        catalog_root = temporary_root / "catalog"
        shutil.copytree(workspace_root / "marketplace" / "local", catalog_root)
        catalog_plugin = catalog_root / lifecycle["archiveDestination"]
        manifest = extract_archive(archive_path.resolve(), catalog_plugin)
        original_version = manifest["version"]
        changed_version = next_patch_version(original_version)
        changed_root = temporary_root / "changed-plugin"
        changed_snapshot(catalog_plugin, changed_root, changed_version)

        project = temporary_root / "project"
        project.mkdir()
        runner = CodexRunner(
            codex_executable.resolve(),
            temporary_root / "codex-home",
            lifecycle["codexVersion"],
        )
        runner.verify_version()
        runner.json(["plugin", "marketplace", "add", str(catalog_root)])
        runner.json(["plugin", "add", selector])

        client = AppServerClient(runner, project)
        try:
            initial = hook_map(client.hooks(), selector, "untrusted")
            client.trust(list(initial.values()))
            trusted = hook_map(client.hooks(), selector, "trusted")
            trusted_hashes = {event: hook["currentHash"] for event, hook in trusted.items()}
        finally:
            client.close()

        replace_catalog_plugin(catalog_plugin, changed_root)
        updated = runner.json(["plugin", "add", selector])
        require(updated.get("version") == changed_version, "changed hook package did not install")

        changed_client = AppServerClient(runner, project)
        try:
            changed_hooks = changed_client.hooks()
            changed_by_event = {hook.get("eventName"): hook for hook in changed_hooks}
            require(set(changed_by_event) == EXPECTED_EVENTS, "changed hook event set drifted")
            for event, hook in changed_by_event.items():
                require(
                    hook.get("trustStatus") in {"untrusted", "modified"},
                    f"{event} retained stale trust: {hook.get('trustStatus')}",
                )
                require(hook.get("currentHash") != trusted_hashes[event], f"{event} hash did not change")

            changed_client.trust(changed_hooks)
            hook_map(changed_client.hooks(), selector, "trusted")
        finally:
            changed_client.close()

        runner.json(["plugin", "remove", selector])
        runner.json(["plugin", "marketplace", "remove", marketplace_name])

    return {
        "schemaVersion": 1,
        "codexVersion": lifecycle["codexVersion"],
        "plugin": selector,
        "initialVersion": original_version,
        "changedVersion": changed_version,
        "operations": [
            "observe-untrusted",
            "explicit-trust",
            "observe-trusted",
            "change-hook-definition",
            "observe-trust-invalidated",
            "explicit-retrust",
        ],
        "bypassUsed": False,
        "result": "passed",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate Lili plugin hook trust with Codex")
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--codex", default="codex")
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
    )
    arguments = parser.parse_args()
    try:
        result = run_hook_trust_round_trip(
            arguments.workspace_root,
            arguments.archive,
            resolve_executable(arguments.codex),
        )
    except (MarketplaceRoundTripError, OSError, KeyError, TypeError, ValueError) as error:
        print(f"hook trust round trip failed: {error}")
        return 1
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
