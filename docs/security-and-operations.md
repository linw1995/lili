# Security and Operations

## Trust boundary

Lili separates display code from native authority.

- The WebView and `lili-web` fixture build receive presentation snapshots, approved opaque pet asset identities, settings, and interaction endpoints. They do not receive filesystem paths, forwarding credentials, action configuration values, or process handles.
- The desktop process owns pet discovery, persistent state, Codex integration changes, authenticated local forwarding, spool recovery, and action execution.
- Provider payloads, pet packages, action configuration, persisted state, spool records, and browser requests are untrusted inputs. Native parsers apply size, schema, path, identity, replay, and ownership checks before state mutation.
- The local forwarding endpoint is restricted to the current operating-system user. Instance credentials rotate when the desktop service starts, messages carry a nonce and MAC, and accepted nonces cannot be replayed inside the replay window.
- The WebView communicates with an ephemeral HTTPS loopback origin. Desktop navigation pins its generated certificate, mutating requests require a narrow native signature, and the WebView has no general shell or filesystem command.

This boundary protects Lili from page content and malformed inputs. It is not an operating-system sandbox for a user-approved action executable.

## Local data layout

`${CODEX_HOME}` defaults to `~/.codex`. An absolute `CODEX_HOME` override changes every path below.

| Path | Lifetime | Contents |
| --- | --- | --- |
| `${CODEX_HOME}/pet/lili/` | User managed | Default Codex v2 pet manifest and atlas. |
| `${CODEX_HOME}/pets/<id>/` | User managed | Additional compatible pet packages. |
| `${CODEX_HOME}/lili/actions.toml` | User managed | Action identifiers, filters, executable argv, limits, working-directory policy, and explicit environment additions. |
| `${CODEX_HOME}/lili/state.json` | Persistent | Selected pet identifier, logical window placement, bounded reducer metadata, bounded session records, and unread notifications required for restart recovery. |
| `${CODEX_HOME}/lili/selected-pet.json` | Persistent compatibility state | Selected pet identifier only. |
| `${CODEX_HOME}/lili/spool/` | Bounded and recoverable | Normalized events waiting for desktop ingestion plus aggregate drop metrics. |
| `${CODEX_HOME}/lili/runtime/forwarding.json` | Current desktop instance | Instance identifier, local endpoint, and secret used to authenticate forwarding. Owner-only and removed on orderly shutdown. |
| `${CODEX_HOME}/lili/runtime/forwarding.sock` | Current desktop instance on Unix | Owner-only local forwarding socket. Windows uses a user-scoped named pipe. |
| `${CODEX_HOME}/lili/integration.json` | Until complete uninstall | Managed integration provenance, file hashes, owned hook commands, prior notify argv, backup paths, and install timestamp. |
| `${CODEX_HOME}/config.toml.lili-backup-<timestamp>` | Until manually removed | Pre-install configuration backup when `config.toml` was updated. |
| `${CODEX_HOME}/hooks.json.lili-backup-<timestamp>` | Until manually removed | Pre-install hook backup when `hooks.json` was updated. |

`state.json` is persistent so window placement, the selected pet, monotonic reducer metadata, and unread notifications survive an application restart. Deleting it while Lili is stopped resets that recovery state; it is recreated on the next persisted shutdown.

## Retained metadata and excluded content

Lili retains only the metadata needed for display, deduplication, recovery, and audit:

- provider, event, session, and optional turn identities;
- normalized lifecycle kind and occurrence time;
- bounded project label and display-safe summary;
- unread notification state and bounded reducer ordering metadata;
- action identifier, trigger, event identity, timing, outcome, exit code, and output byte counts;
- aggregate ingestion and spool counters.

The action audit is memory-only and bounded to recent entries. Captured child stdout and stderr are used only to classify the current result; their content is not included in the audit or diagnostics response.

Structured logs and diagnostics exclude raw prompts, complete assistant messages, command argv, approval arguments, credentials, MAC secrets, and inherited environment values. Spool records contain normalized events, never the original provider payload. The action configuration and integration provenance necessarily retain user-authored executable configuration on disk; they are not copied into ordinary logs or browser diagnostics.

## Codex integration changes

Lili provides an inspect, plan, install, and uninstall workflow:

```text
lili integrate inspect
lili integrate plan
lili integrate plan --coexist
lili integrate install --plan <plan.json>
lili integrate uninstall
```

The plan is immutable and includes expected file hashes. Installation stops if either target changes after planning.

An accepted plan can make these changes:

- update `${CODEX_HOME}/config.toml` with a direct `notify` argv for the packaged `lili-hook` binary;
- update `${CODEX_HOME}/hooks.json` with marked observer-only handlers for `SessionStart`, `UserPromptSubmit`, `PermissionRequest`, `Stop`, and `SessionEnd`;
- create timestamped backups before updating existing files;
- write `${CODEX_HOME}/lili/integration.json` only after marker validation and a synthetic delivery check succeed.

Existing non-Lili `notify` configuration is a conflict by default. `--coexist` must be selected explicitly; it preserves the previous argv and dispatches the two notification commands independently. Codex may still require the user to trust newly configured hooks.

`PermissionRequest` is observation-only. The hook returns no approval or denial and Lili never becomes a Codex authorization authority.

## Action authority

Actions are opt-in native programs configured in `${CODEX_HOME}/lili/actions.toml`.

Lili resolves an executable, passes argv directly without a shell, clears the inherited environment, adds a minimal platform environment plus explicit configured values, and sends one bounded `InteractionContextV1` JSON document on standard input. Event text is never interpolated into argv. Timeouts, debounce, concurrency, queue capacity, output capture, and process-tree termination are bounded.

These controls prevent shell interpretation and accidental ambient environment leakage. They do not restrict what the selected executable can do with the current user's operating-system permissions. A configured action may read files, use the network, or modify data if that executable could do so when launched directly by the user. Review the executable path, argv, working directory, and explicit environment values before enabling an action.

An action result cannot acknowledge a Codex permission, change source session state, or dismiss its notification.

## Backup, reset, and uninstall

Use this order for a complete removal:

1. Run `lili integrate uninstall` while the installed Lili binary is still available.
2. Review the JSON result. `complete: true` means owned notify and hook entries were removed or restored and integration provenance was deleted. If `complete` is false, resolve the reported conflicts before editing or deleting provenance manually.
3. Quit Lili and remove the application bundle or installed binaries.
4. After `complete: true`, remove `${CODEX_HOME}/lili/` if local Lili state is no longer needed. This deletes actions, state, spool data, and runtime remnants. Do not remove this directory to bypass an incomplete integration uninstall.
5. Review timestamped `*.lili-backup-*` files under `${CODEX_HOME}` and remove them only after confirming the active Codex configuration.
6. Pet packages are deliberately not removed by integration uninstall. Remove `${CODEX_HOME}/pet/lili/` or a package under `${CODEX_HOME}/pets/` only if it is not used elsewhere.

For a state-only reset, quit Lili and remove `${CODEX_HOME}/lili/state.json` and `${CODEX_HOME}/lili/selected-pet.json`. Leave `integration.json` intact unless the integration has been uninstalled or its conflicts have been resolved.

## Unsupported Codex surfaces

Lili supports documented `notify` payloads and explicitly configured lifecycle hooks covered by its versioned compatibility fixtures. It intentionally does not:

- read `auth.json` or any Codex credential store;
- read private SQLite databases, rollout JSONL, conversation history, or process memory;
- call private desktop RPC, IPC, automation, marketplace, or plugin APIs;
- infer unsupported lifecycle states from unrelated hooks;
- send prompts, start or resume turns, approve permissions, or mutate a Codex session;
- promise compatibility for an untested Codex version without reporting missing coverage.

When a documented provider surface changes, update the adapter and its versioned fixtures. Do not bypass the adapter by consuming a private surface.
