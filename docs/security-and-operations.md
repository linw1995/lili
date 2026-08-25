# Security and Operations

For ordered setup and verification steps, see the [Configuration guide](configuration.md). This document defines the authority, data retention, and recovery boundaries behind those steps.

## Trust boundary

Lili separates display code from native authority.

- The WebView and `lili-web` fixture build receive presentation snapshots, approved opaque pet asset identities, settings, and interaction endpoints. They do not receive filesystem paths, forwarding credentials, action configuration values, or process handles.
- The desktop process owns Pet discovery, SQLite state, authenticated local forwarding, spool recovery, and action execution. Only explicitly invoked integration commands own Codex configuration changes.
- Provider payloads, pet packages, action configuration, persisted state, spool records, and browser requests are untrusted inputs. Native parsers apply size, schema, path, identity, replay, and ownership checks before state mutation.
- The local forwarding endpoint is restricted to the current operating-system user. Instance credentials rotate when the desktop service starts, messages carry a nonce and MAC, and accepted nonces cannot be replayed inside the replay window.
- The WebView communicates with an ephemeral HTTPS loopback origin. Desktop navigation pins its generated certificate, mutating requests require a narrow native signature, and the WebView has no general shell or filesystem command.

This boundary protects Lili from page content and malformed inputs. It is not an operating-system sandbox for a user-approved action executable.

## Local data layout

The desktop runtime and Hook use the platform-native Lili application root. `CODEX_HOME` is not a desktop storage root. The paths below that use `CODEX_HOME` belong only to explicitly invoked integration commands that manage Codex configuration.

| Path | Lifetime | Contents |
| --- | --- | --- |
| `<LILI_DATA>/lili.sqlite3` | Persistent | Application metadata, one latest state projection per Session, unconsumed offline spool records, and the latest plugin evidence. |
| `<LILI_DATA>/pets/<id>/` | User managed | Validated Pet v2 manifests and spritesheets owned by Lili. |
| `<LILI_DATA>/config/actions.toml` | User managed | Action identifiers, filters, executable argv, limits, working-directory policy, and explicit environment additions. |
| `<LILI_DATA>/runtime/forwarding.json` | Current desktop instance | Instance identifier, local endpoint, and secret used to authenticate forwarding. Owner-only and removed on orderly shutdown. |
| `<XDG_RUNTIME_DIR>/lili-<hash>/endpoint.sock` or `/tmp/lili-<hash>/endpoint.sock` (recorded in runtime credentials) | Current desktop instance on Unix | Short local forwarding socket inside an owner-only runtime directory; the `/tmp` form is the bounded fallback when no suitable XDG runtime directory exists. Windows uses a user-scoped named pipe. |
| `${CODEX_HOME}/lili/integration.json` | Until complete uninstall; integration only | Managed integration provenance, file hashes, owned hook commands, prior notify argv, backup paths, and install timestamp. |
| `${CODEX_HOME}/config.toml.lili-backup-<timestamp>` | Until manually removed; integration only | Pre-install configuration backup when `config.toml` was updated. |
| `${CODEX_HOME}/hooks.json.lili-backup-<timestamp>` | Until manually removed; integration only | Pre-install configuration backup when `hooks.json` was updated. |

`<LILI_DATA>` is `~/Library/Application Support/dev.linw1995.lili/` on macOS, `$XDG_STATE_HOME/dev.linw1995.lili/` or `~/.local/state/dev.linw1995.lili/` on Linux, and `%LOCALAPPDATA%\dev.linw1995.lili\` on Windows. Deleting `lili.sqlite3` while Lili is stopped resets application state; it does not touch Codex configuration or user Pet files.

## Retained metadata and excluded content

Lili retains only the metadata needed for display and recovery:

- the latest provider, event, session, and optional turn identities for each persisted Session projection;
- the latest normalized lifecycle kind and occurrence time for each persisted Session projection;
- a bounded project label and display-safe summary for the presentation-driving Session notification;
- unread state for at most one presentation-driving notification per Session;
- action identifier, trigger, event identity, timing, outcome, exit code, and output byte counts;
- unconsumed normalized spool records until they are delivered or evicted;
- aggregate expired, limit, and malformed-drop counters for the temporary spool.

The action audit is memory-only and bounded to recent entries. Captured child stdout and stderr are used only to classify the current result; their content is not included in the audit or diagnostics response.

Structured logs and diagnostics exclude raw prompts, complete assistant messages, command argv, approval arguments, credentials, MAC secrets, and inherited environment values. Spool records contain normalized events, never the original provider payload. The action configuration and integration provenance necessarily retain user-authored executable configuration on disk; they are not copied into ordinary logs or browser diagnostics.

## Codex integration changes

Lili provides read-only inspection plus an explicit legacy fallback plan, install, and uninstall workflow. The Codex plugin is the primary installation path; direct configuration remains available only when the plugin is unavailable or Marketplace policy prevents its use.

```text
lili integrate inspect
lili integrate plan --legacy-fallback
lili integrate plan --legacy-fallback --coexist
lili integrate install --legacy-fallback --plan <plan.json>
lili integrate uninstall
```

The plan is immutable, identifies itself as `legacy_fallback`, and includes expected file hashes. Installation rejects old or differently classified plans and stops if either target changes after planning.

An accepted plan can make these changes:

- update `${CODEX_HOME}/config.toml` with a direct `notify` argv for the packaged `lili-hook` binary;
- update `${CODEX_HOME}/hooks.json` with marked observer-only handlers for `SessionStart`, `UserPromptSubmit`, `PermissionRequest`, `Stop`, and `SessionEnd`;
- create timestamped backups before updating existing files;
- write `${CODEX_HOME}/lili/integration.json` only after marker validation and a synthetic delivery check succeed.

Existing non-Lili `notify` configuration is a conflict by default. `--coexist` must be selected explicitly; it preserves the previous argv and dispatches the two notification commands independently. Codex may still require the user to trust newly configured hooks.

`PermissionRequest` is observation-only. The hook returns no approval or denial and Lili never becomes a Codex authorization authority.

Plugin removal and migration rollback invoke only the supported `codex plugin remove <plugin@marketplace> --json` operation. They do not call legacy uninstall and do not remove or edit the desktop application, pet packages, actions, state, spool, unrelated hooks, notification commands, or Lili-owned legacy configuration. Legacy cleanup remains a separate provenance-gated operation after verified migration.

## Action authority

Actions are opt-in native programs configured in `<LILI_DATA>/config/actions.toml`.

Lili resolves an executable, passes argv directly without a shell, clears the inherited environment, adds a minimal platform environment plus explicit configured values, and sends one bounded `InteractionContextV1` JSON document on standard input. Event text is never interpolated into argv. Timeouts, debounce, concurrency, queue capacity, output capture, and process-tree termination are bounded.

These controls prevent shell interpretation and accidental ambient environment leakage. They do not restrict what the selected executable can do with the current user's operating-system permissions. A configured action may read files, use the network, or modify data if that executable could do so when launched directly by the user. Review the executable path, argv, working directory, and explicit environment values before enabling an action.

An action result cannot acknowledge a Codex permission, change source session state, or dismiss its notification.

## Backup, reset, and uninstall

Use this order for a complete removal:

1. Run `lili integrate uninstall` while the installed Lili binary is still available.
2. Review the JSON result. `complete: true` means owned notify and hook entries were removed or restored and integration provenance was deleted. If `complete` is false, resolve the reported conflicts before editing or deleting provenance manually.
3. Quit Lili and remove the application bundle or installed binaries.
4. After `complete: true`, remove the Lili application root if local state is no longer needed. This deletes the SQLite database, actions, spool data, runtime remnants, and application-owned Pet packages. Do not remove Codex directories to bypass an incomplete integration uninstall.
5. Review timestamped `*.lili-backup-*` files under `${CODEX_HOME}` and remove them only after confirming the active Codex configuration.
6. Existing `${CODEX_HOME}/pets/` and `${CODEX_HOME}/lili/` data is not migrated or deleted by the new desktop runtime.

For a state-only reset, quit Lili and remove `<LILI_DATA>/lili.sqlite3`. Leave `<LILI_DATA>/pets/` and `${CODEX_HOME}/lili/integration.json` intact unless you explicitly intend to remove those user-managed resources.

## Unsupported Codex surfaces

Lili supports documented `notify` payloads and explicitly configured lifecycle hooks covered by its versioned compatibility fixtures. It intentionally does not:

- read `auth.json` or any Codex credential store;
- read private SQLite databases, rollout JSONL, conversation history, or process memory;
- call private desktop RPC, IPC, automation, marketplace, or plugin APIs;
- infer unsupported lifecycle states from unrelated hooks;
- send prompts, start or resume turns, approve permissions, or mutate a Codex session;
- promise compatibility for an untested Codex version without reporting missing coverage.

When a documented provider surface changes, update the adapter and its versioned fixtures. Do not bypass the adapter by consuming a private surface.
