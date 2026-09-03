#!/usr/bin/env python3

import argparse
import http.server
import json
import os
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
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
EXPECTED_NORMALIZED_EVENTS = {
    "sessionStart": "session_started",
    "userPromptSubmit": "turn_started",
    "permissionRequest": "attention_required",
    "stop": "turn_completed",
    "sessionEnd": "session_ended",
}


def sse(events: list[dict]) -> bytes:
    chunks = []
    for event in events:
        event_type = event["type"]
        payload = json.dumps(event, separators=(",", ":"))
        chunks.append(f"event: {event_type}\ndata: {payload}\n\n")
    return "".join(chunks).encode()


class ResponsesServer:
    def __init__(self, permission_command: str):
        function_arguments = json.dumps(
            {"command": permission_command}, separators=(",", ":")
        )
        self.responses = [
            sse(
                [
                    {"type": "response.created", "response": {"id": "resp-1"}},
                    {
                        "type": "response.output_item.done",
                        "item": {
                            "type": "function_call",
                            "call_id": "lili-permission-request",
                            "name": "shell_command",
                            "arguments": function_arguments,
                        },
                    },
                    {
                        "type": "response.completed",
                        "response": {
                            "id": "resp-1",
                            "usage": {
                                "input_tokens": 0,
                                "input_tokens_details": None,
                                "output_tokens": 0,
                                "output_tokens_details": None,
                                "total_tokens": 0,
                            },
                        },
                    },
                ]
            ),
            sse(
                [
                    {"type": "response.created", "response": {"id": "resp-2"}},
                    {
                        "type": "response.output_item.done",
                        "item": {
                            "type": "message",
                            "role": "assistant",
                            "id": "msg-1",
                            "content": [
                                {
                                    "type": "output_text",
                                    "text": "Marketplace hook dispatch completed.",
                                }
                            ],
                        },
                    },
                    {
                        "type": "response.completed",
                        "response": {
                            "id": "resp-2",
                            "usage": {
                                "input_tokens": 0,
                                "input_tokens_details": None,
                                "output_tokens": 0,
                                "output_tokens_details": None,
                                "total_tokens": 0,
                            },
                        },
                    },
                ]
            ),
        ]
        self.requests: list[dict] = []
        self.lock = threading.Lock()
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def do_POST(self) -> None:
                length = int(self.headers.get("Content-Length", "0"))
                require(0 < length <= MAX_MESSAGE_BYTES, "Codex model request is invalid")
                body = self.rfile.read(length)
                try:
                    request = json.loads(body)
                except json.JSONDecodeError as error:
                    raise MarketplaceRoundTripError(
                        "Codex model request is not JSON"
                    ) from error
                require(isinstance(request, dict), "Codex model request root is invalid")
                with owner.lock:
                    index = len(owner.requests)
                    owner.requests.append(request)
                require(index < len(owner.responses), "Codex made an unexpected model request")
                response = owner.responses[index]
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(response)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(response)

            def log_message(self, _format: str, *_arguments) -> None:
                return

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def base_url(self) -> str:
        host, port = self.server.server_address
        return f"http://{host}:{port}/v1"

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, _error_type, _error, _traceback) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


class AppServerClient:
    def __init__(self, runner: CodexRunner, cwd: Path):
        base_url = runner.environment.get("OPENAI_BASE_URL")
        require(isinstance(base_url, str) and base_url, "acceptance model base URL is missing")
        self.process = subprocess.Popen(
            [
                str(runner.executable),
                "-c",
                'model_provider="lili_acceptance"',
                "-c",
                'model_providers.lili_acceptance.name="Lili Acceptance"',
                "-c",
                f"model_providers.lili_acceptance.base_url={json.dumps(base_url)}",
                "-c",
                'model_providers.lili_acceptance.wire_api="responses"',
                "-c",
                'model_providers.lili_acceptance.env_key="OPENAI_API_KEY"',
                "-c",
                "model_providers.lili_acceptance.supports_websockets=false",
                "app-server",
                "--stdio",
            ],
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
        self.output_messages: queue.Queue[bytes] = queue.Queue(maxsize=1024)
        self.output_thread = threading.Thread(target=self._read_stdout, daemon=True)
        self.output_thread.start()
        self.next_identifier = 1
        self.notifications: list[dict] = []
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
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            if stream is not None and not stream.closed:
                stream.close()
        self.output_thread.join(timeout=1)

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

    def start_thread(self) -> str:
        response = self.request(
            "thread/start",
            {
                "approvalPolicy": "never",
                "cwd": str(self.cwd),
                "ephemeral": True,
                "sandbox": "read-only",
            },
        )
        thread = response.get("thread")
        require(isinstance(thread, dict), "thread/start omitted the created thread")
        thread_id = thread.get("id")
        require(isinstance(thread_id, str) and thread_id, "thread/start omitted the thread ID")
        return thread_id

    def dispatch_turn(self, thread_id: str) -> dict[str, dict]:
        turn_id = self._start_turn(
            thread_id, "Exercise the Marketplace permission lifecycle hook."
        )

        runs = {
            "sessionStart": self._completed_hook(thread_id, "sessionStart", turn_id),
            "userPromptSubmit": self._completed_hook(
                thread_id, "userPromptSubmit", turn_id
            ),
            "permissionRequest": self._completed_hook(
                thread_id, "permissionRequest", turn_id
            ),
        }
        self.request(
            "turn/interrupt", {"threadId": thread_id, "turnId": turn_id}
        )
        interrupted = self._notification(
            "turn/completed",
            lambda params: params.get("threadId") == thread_id
            and isinstance(params.get("turn"), dict)
            and params["turn"].get("id") == turn_id,
            "interrupted permission turn completion",
        )
        require(
            interrupted["params"]["turn"].get("status") == "interrupted",
            "Codex permission turn did not interrupt cleanly",
        )
        stop_turn_id = self._start_turn(
            thread_id, "Exercise the Marketplace Stop lifecycle hook."
        )
        self._completed_hook(thread_id, "userPromptSubmit", stop_turn_id)
        runs["stop"] = self._completed_hook(thread_id, "stop", stop_turn_id)
        self._notification(
            "turn/completed",
            lambda params: params.get("threadId") == thread_id
            and isinstance(params.get("turn"), dict)
            and params["turn"].get("id") == stop_turn_id,
            "turn completion after Stop",
        )
        return runs

    def _start_turn(self, thread_id: str, prompt: str) -> str:
        response = self.request(
            "turn/start",
            {
                "threadId": thread_id,
                "input": [{"type": "text", "text": prompt}],
                "approvalPolicy": "on-request",
                "sandboxPolicy": {"type": "readOnly"},
            },
        )
        turn = response.get("turn")
        require(isinstance(turn, dict), "turn/start omitted the created turn")
        turn_id = turn.get("id")
        require(isinstance(turn_id, str) and turn_id, "turn/start omitted the turn ID")
        return turn_id

    def shutdown(self) -> None:
        require(self.process.stdin is not None, "Codex app-server stdin is closed")
        self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired as error:
            raise MarketplaceRoundTripError(
                "Codex app-server did not shut down after stdin closed"
            ) from error
        require(self.process.returncode == 0, "Codex app-server shutdown failed")

    def _completed_hook(
        self, thread_id: str, event_name: str, turn_id: str | None = None
    ) -> dict:
        notification = self._notification(
            "hook/completed",
            lambda params: params.get("threadId") == thread_id
            and (turn_id is None or params.get("turnId") == turn_id)
            and isinstance(params.get("run"), dict)
            and params["run"].get("eventName") == event_name,
            f"{event_name} hook completion",
        )
        run = notification["params"]["run"]
        require(run.get("source") == "plugin", f"Codex dispatched {event_name} outside the plugin")
        require(run.get("handlerType") == "command", f"Codex did not dispatch {event_name} as a command")
        require(run.get("executionMode") == "sync", f"Codex changed {event_name} execution mode")
        require(run.get("status") == "completed", f"Codex did not complete the trusted {event_name} hook")
        return run

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
            message = self._message(10)
            if message.get("id") != identifier:
                if isinstance(message.get("method"), str):
                    self.notifications.append(message)
                continue
            require("error" not in message, f"Codex app-server request failed: {message.get('error')}")
            result = message.get("result")
            require(isinstance(result, dict), "Codex app-server response omitted its result")
            return result

    def _notification(self, method: str, predicate, description: str | None = None) -> dict:
        return self._matching_message(
            lambda message: message.get("method") == method
            and isinstance(message.get("params"), dict)
            and predicate(message["params"]),
            description or method,
        )

    def _matching_message(self, predicate, description: str) -> dict:
        deadline = time.monotonic() + 10
        while True:
            for index, message in enumerate(self.notifications):
                if predicate(message):
                    return self.notifications.pop(index)
            remaining = deadline - time.monotonic()
            require(
                remaining > 0,
                f"Codex app-server notification timed out: {description}",
            )
            try:
                message = self._message(remaining)
            except MarketplaceRoundTripError as error:
                observed = [
                    message.get("method")
                    for message in self.notifications[-10:]
                    if isinstance(message.get("method"), str)
                ]
                errors = [
                    message.get("params")
                    for message in self.notifications[-10:]
                    if message.get("method") == "error"
                ]
                raise MarketplaceRoundTripError(
                    f"Codex app-server notification timed out: {description}; "
                    f"observed={observed}; errors={errors}"
                ) from error
            if isinstance(message.get("method"), str):
                self.notifications.append(message)

    def _message(self, timeout: float) -> dict:
        try:
            line = self.output_messages.get(timeout=timeout)
        except queue.Empty as error:
            raise MarketplaceRoundTripError("Codex app-server message timed out") from error
        require(0 < len(line) <= MAX_MESSAGE_BYTES, "Codex app-server message is empty or too large")
        try:
            message = json.loads(line)
        except json.JSONDecodeError as error:
            raise MarketplaceRoundTripError("Codex app-server returned invalid JSON") from error
        require(isinstance(message, dict), "Codex app-server message root is not an object")
        return message

    def _read_stdout(self) -> None:
        require(self.process.stdout is not None, "Codex app-server stdout is closed")
        while True:
            line = self.process.stdout.readline(MAX_MESSAGE_BYTES + 1)
            self.output_messages.put(line)
            if not line:
                return


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


def dispatched_spool_events(codex_home: Path) -> dict[str, list[dict]]:
    spool = codex_home / "lili" / "spool"
    pending = sorted(spool.glob("*.pending"))
    expected_counts = {
        **{event_type: 1 for event_type in EXPECTED_NORMALIZED_EVENTS.values()},
        "turn_started": 2,
    }
    require(
        len(pending) == sum(expected_counts.values()),
        "trusted hooks produced an unexpected lifecycle event count",
    )
    by_type: dict[str, list[dict]] = {}
    for path in pending:
        event = load_json(path).get("event")
        require(isinstance(event, dict), "spooled hook record omitted its normalized event")
        require(event.get("provider") == "codex", "spooled hook provider drifted")
        event_type = event.get("eventType")
        require(isinstance(event_type, str), "spooled hook event type is missing")
        source = event.get("sourceDiscriminator")
        require(
            isinstance(source, str) and source.startswith("plugin:"),
            f"spooled {event_type} lost plugin attribution",
        )
        require(
            isinstance(event.get("eventId"), str) and event["eventId"],
            f"spooled {event_type} event ID is missing",
        )
        by_type.setdefault(event_type, []).append(event)

    require(
        set(by_type) == set(EXPECTED_NORMALIZED_EVENTS.values()),
        "spooled hook event type set drifted",
    )
    require(
        {event_type: len(events) for event_type, events in by_type.items()}
        == expected_counts,
        "spooled hook event counts drifted",
    )
    return {
        event_name: by_type[event_type]
        for event_name, event_type in EXPECTED_NORMALIZED_EVENTS.items()
    }


def dispatch_installed_plugin_hook(
    workspace_root: Path,
    codex_executable: Path,
    codex_home: Path,
    plugin_root: Path,
    cwd: Path,
) -> dict:
    lifecycle = load_json(workspace_root.resolve() / "marketplace" / "local" / "lifecycle.json")
    selector = lifecycle["pluginSelector"]
    require(plugin_root.resolve().is_dir(), "installed plugin root is missing")
    require(cwd.resolve().is_dir(), "hook dispatch working directory is missing")
    runner = CodexRunner(
        codex_executable.resolve(),
        codex_home.resolve(),
        lifecycle["codexVersion"],
    )
    runner.verify_version()
    if os.name == "nt":
        system_root = os.environ.get("SystemRoot")
        require(isinstance(system_root, str) and system_root, "Windows system root is missing")
        runner.environment["SystemRoot"] = system_root
        temporary = codex_home.resolve() / "acceptance-temp"
        temporary.mkdir(parents=True, exist_ok=True)
        runner.environment.update({"TEMP": str(temporary), "TMP": str(temporary)})
    runner.environment.update(
        {
            "OPENAI_API_KEY": "lili-windows-hook-dispatch",
            "OPENAI_BASE_URL": "http://127.0.0.1:9/v1",
        }
    )
    client = AppServerClient(runner, cwd.resolve())
    try:
        initial = hook_map(client.hooks(), selector, "untrusted")
        client.trust(list(initial.values()))
        hook_map(client.hooks(), selector, "trusted")
        thread_id = client.start_thread()
        run = client._completed_hook(thread_id, "sessionStart")
        client.shutdown()
    finally:
        client.close()
    return {
        "schemaVersion": 1,
        "plugin": selector,
        "pluginRoot": str(plugin_root.resolve()),
        "event": "sessionStart",
        "hookRunId": run["id"],
        "bypassUsed": False,
        "result": "passed",
    }


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

        permission_target = project / "permission-target"
        permission_target.write_text(
            "must remain after interrupted permission turn\n", encoding="utf-8"
        )
        permission_command = f"rm -f {permission_target}"
        with ResponsesServer(permission_command) as responses:
            runner.environment.update(
                {
                    "OPENAI_API_KEY": "lili-marketplace-acceptance",
                    "OPENAI_BASE_URL": responses.base_url,
                }
            )
            client = AppServerClient(runner, project)
            try:
                initial = hook_map(client.hooks(), selector, "untrusted")
                client.trust(list(initial.values()))
                trusted = hook_map(client.hooks(), selector, "trusted")
                trusted_hashes = {
                    event: hook["currentHash"] for event, hook in trusted.items()
                }
                thread_id = client.start_thread()
                dispatched_runs = client.dispatch_turn(thread_id)
                client.shutdown()
            finally:
                client.close()
            require(
                len(responses.requests) == 2,
                "Codex did not complete the deterministic model round trip",
            )
        require(permission_target.is_file(), "permission hook turn mutated the project")
        dispatched_events = dispatched_spool_events(runner.codex_home)

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
            "dispatch-every-trusted-hook",
            "change-hook-definition",
            "observe-trust-invalidated",
            "explicit-retrust",
        ],
        "dispatchedEventIds": {
            event: [item["eventId"] for item in dispatched_events[event]]
            for event in sorted(dispatched_events)
        },
        "dispatchedHookRunIds": {
            event: dispatched_runs[event]["id"] for event in sorted(dispatched_runs)
        },
        "bypassUsed": False,
        "result": "passed",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate Lili plugin hook trust with Codex")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--archive", type=Path)
    source.add_argument("--installed-codex-home", type=Path)
    parser.add_argument("--installed-plugin-root", type=Path)
    parser.add_argument("--dispatch-cwd", type=Path)
    parser.add_argument("--codex", default="codex")
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
    )
    arguments = parser.parse_args()
    try:
        codex = resolve_executable(arguments.codex)
        if arguments.archive is not None:
            require(
                arguments.installed_plugin_root is None and arguments.dispatch_cwd is None,
                "installed dispatch arguments cannot accompany an archive",
            )
            result = run_hook_trust_round_trip(
                arguments.workspace_root,
                arguments.archive,
                codex,
            )
        else:
            require(
                arguments.installed_plugin_root is not None and arguments.dispatch_cwd is not None,
                "installed dispatch requires plugin root and working directory",
            )
            result = dispatch_installed_plugin_hook(
                arguments.workspace_root,
                codex,
                arguments.installed_codex_home,
                arguments.installed_plugin_root,
                arguments.dispatch_cwd,
            )
    except (MarketplaceRoundTripError, OSError, KeyError, TypeError, ValueError) as error:
        print(f"hook trust round trip failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
