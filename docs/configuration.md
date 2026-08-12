# Configuration

This guide covers the supported path from launching Lili to receiving Codex Session notifications and running local programs for pet or notification interactions. Read [Security and operations](security-and-operations.md) before enabling native actions.

## 1. Choose how to run Lili

For desktop development from a checkout, use the pinned native toolchain:

```text
nix run .#dev
```

`nix run .#dev-web` is a browser fixture and cannot read pet packages, receive Session events, edit Codex configuration, or run actions.

For a stable local installation, build or unpack a complete release:

```text
nix run .#build
```

The assembled release is written below `release/lili-<version>-<platform>/`. It contains:

- the desktop bundle below `bundles/`;
- `bin/lili` for integration management;
- the matching `bin/lili-hook` forwarder;
- the built-in pet files under `pets/lili/`;
- this guide and an action example.

Launch the platform desktop bundle. On macOS, for example:

```text
export LILI_RELEASE="/absolute/path/to/unpacked-lili-release"
open "$LILI_RELEASE/bundles/macos/Lili.app"
```

The remaining command examples use a POSIX shell. In Windows PowerShell, set the release root and invoke the `.exe` explicitly:

```powershell
$env:LILI_RELEASE = "C:\absolute\path\to\unpacked-lili-release"
& "$env:LILI_RELEASE\bin\lili.exe" integrate inspect
```

Keep the release directory at a stable absolute path after installing Session integration. The generated Codex configuration references its `bin/lili-hook` by absolute path. If the release moves, generate and install a new plan from the new location.

## 2. Select the Codex home

Lili uses `${CODEX_HOME}` when it is set to an absolute path. Otherwise it uses `~/.codex`. The same root controls pet discovery, action configuration, local state, forwarding, and Codex integration files.

The desktop application, integration command, hook, and Codex process must resolve the same root. The default needs no configuration. For a custom root, provide the same environment value to every process:

```text
export CODEX_HOME="/absolute/path/to/codex-home"
```

The PowerShell equivalent is `$env:CODEX_HOME = "C:\absolute\path\to\codex-home"`.

Applications launched from a desktop shell may not inherit terminal environment variables. On macOS, a direct launch that preserves the override is:

```text
CODEX_HOME="/absolute/path/to/codex-home" "$LILI_RELEASE/bundles/macos/Lili.app/Contents/MacOS/lili"
```

Use the default root unless all launch paths can consistently provide the override.

## 3. Configure pet packages

Lili always has an embedded Lili fallback. Every external package is discovered one directory below `${CODEX_HOME}/pets/`. A valid package at `${CODEX_HOME}/pets/lili/` replaces the embedded fallback:

```text
${CODEX_HOME}/pets/lili/pet.json
${CODEX_HOME}/pets/lili/spritesheet.webp
${CODEX_HOME}/pets/<other-pet-id>/pet.json
${CODEX_HOME}/pets/<other-pet-id>/spritesheet.webp
```

To install the release copy as the external default package:

```text
mkdir -p "${CODEX_HOME:-$HOME/.codex}/pets/lili"
cp "$LILI_RELEASE/pets/lili/pet.json" "${CODEX_HOME:-$HOME/.codex}/pets/lili/pet.json"
cp "$LILI_RELEASE/pets/lili/spritesheet.webp" "${CODEX_HOME:-$HOME/.codex}/pets/lili/spritesheet.webp"
```

The legacy singular `${CODEX_HOME}/pet/` directory is not scanned or migrated. Move any package that still uses it into `${CODEX_HOME}/pets/<pet-id>/`, then restart Lili.

A v2 manifest uses camel-case field names:

```json
{
  "id": "lili",
  "displayName": "Lili",
  "description": "A local desktop companion.",
  "spriteVersionNumber": 2,
  "spritesheetPath": "spritesheet.webp"
}
```

The atlas must be a transparent PNG or WebP image with exact dimensions `1536x2288`: eight `192x208` columns by eleven rows. The package path must be an ordinary directory rather than a symbolic link, `pet.json` must be an ordinary file, and the atlas must resolve inside the package directory.

Restart Lili after adding or replacing a package. Select an installed package from the tray menu under **Pet**. Invalid packages are skipped and reported under **Diagnostics**; the embedded Lili package remains available.

## 4. Enable Session notifications

Run integration commands with the release `bin/lili`, not the executable inside the desktop bundle. This ensures the planned hook path names the matching sibling `bin/lili-hook`.

Start Lili, then inspect the current Codex configuration:

```text
"$LILI_RELEASE/bin/lili" integrate inspect
```

Generate a plan and review the complete JSON before applying it:

```text
"$LILI_RELEASE/bin/lili" integrate plan > ./lili-plan.json
cat ./lili-plan.json
"$LILI_RELEASE/bin/lili" integrate install --plan ./lili-plan.json
"$LILI_RELEASE/bin/lili" integrate inspect
```

Only install a plan whose `status` is `ready`. The plan shows the exact target files, expected hashes, backup paths, `notify` argv, hook additions, trust requirements, and synthetic verification command. Installation refuses a stale plan if either target file changes after planning.

An existing non-Lili `notify` command produces `status: "conflict"`. If both commands are required, create a coexistence plan explicitly and review the preserved command before installation:

```text
"$LILI_RELEASE/bin/lili" integrate plan --coexist > ./lili-plan.json
cat ./lili-plan.json
"$LILI_RELEASE/bin/lili" integrate install --plan ./lili-plan.json
```

The installation may update `${CODEX_HOME}/config.toml` and `${CODEX_HOME}/hooks.json`, creates timestamped backups for changed existing files, and records managed provenance in `${CODEX_HOME}/lili/integration.json`. Permission notifications remain observer-only; Lili never approves or denies a request.

After installation, restart Codex and start a new Session so it reads the updated configuration. A synthetic verification event may appear during installation. If Lili is temporarily stopped, supported events are written to the bounded local spool and consumed on the next start.

## 5. Configure interaction actions

Native actions are optional and are loaded from `${CODEX_HOME}/lili/actions.toml` when Lili starts. Copy the release example, then replace every placeholder executable path:

```text
mkdir -p "${CODEX_HOME:-$HOME/.codex}/lili"
cp "$LILI_RELEASE/examples/actions.toml" "${CODEX_HOME:-$HOME/.codex}/lili/actions.toml"
```

The three supported triggers are:

| Trigger | Accepted interaction |
| --- | --- |
| `pet_click` | One pet click that is not part of a drag or double click |
| `pet_double_click` | One accepted pet double click |
| `notification_activate` | Activation of one immutable Session notification |

Each `command` is an argv array. Lili starts the executable directly without a shell and never interpolates notification text into argv. Prefer an absolute executable path:

```toml
version = 1

[[action]]
id = "open-session-context"
trigger = "notification_activate"
command = ["/absolute/path/to/lili-action", "notification"]
timeout_ms = 5000
debounce_ms = 400

[action.filters]
providers = ["codex"]
notification_kinds = ["attention", "failure", "completion"]

[action.concurrency]
mode = "reject"
max_parallel = 1
queue_capacity = 0

[action.working_directory]
policy = "codex_home"
```

Filters support `providers`, `notification_kinds`, and `project_labels`. They require a notification context, so omit `[action.filters]` from pet click and pet double-click actions.

Working-directory policies are `application`, `codex_home`, and `explicit`. An `explicit` policy also requires an existing absolute `path`. Actions receive only a minimal process environment; add fixed values explicitly when needed:

```toml
[action.working_directory]
policy = "explicit"
path = "/absolute/path/to/working-directory"

[action.environment.allow]
LILI_ACTION_PROFILE = "local"
```

`reject` concurrency requires `queue_capacity = 0`. `queue` concurrency requires a positive capacity. `max_parallel` is between 1 and 16, `queue_capacity` is at most 64, `timeout_ms` is between 1 and 120000, and `debounce_ms` is at most 60000.

Lili writes one `InteractionContextV1` JSON document to the action's standard input. A notification activation has this shape:

```json
{
  "version": 1,
  "interactionId": "00000000-0000-0000-0000-000000000000",
  "acceptedAtMs": 0,
  "trigger": "notification_activate",
  "pet": {
    "petId": "lili",
    "label": "Lili",
    "lifecycle": "review"
  },
  "notification": {
    "notificationId": "notification-id",
    "eventId": "event-id",
    "provider": "codex",
    "sessionId": "session-id",
    "turnId": "turn-id",
    "kind": "completion",
    "occurredAtMs": 0,
    "projectLabel": "project",
    "summary": {
      "text": "Display-safe bounded summary",
      "truncated": false,
      "redacted": false
    }
  }
}
```

Pet interactions use the same envelope with `notification: null`. The configured executable decides how to parse the JSON and what local operation to perform. Lili bounds runtime, output capture, concurrency, and process-tree cleanup, but the executable still has the current operating-system user's authority.

Restart Lili after editing `actions.toml`. Invalid entries are disabled independently; valid entries continue to load. Review the effective redacted action configuration and diagnostic codes from the tray **Diagnostics** view.

## 6. Verify and troubleshoot

Use this sequence after configuration:

1. Confirm the desktop pet is visible. Use tray **Show** if it was hidden.
2. Confirm the tray integration label reports **Integration: Installed**.
3. Run `"$LILI_RELEASE/bin/lili" integrate inspect` and review `warnings`, notify ownership, configured hooks, and detected Codex version.
4. Start a new Codex Session and produce a completion or attention event.
5. Activate the notification or interact with the pet, then inspect action feedback and tray **Diagnostics**.

If the pet package does not load, confirm the manifest field names, atlas dimensions and transparency, and package paths. Lili falls back to the embedded package instead of rendering an invalid image.

If Session notifications do not arrive, confirm Lili and Codex use the same absolute `${CODEX_HOME}`, keep Lili running for direct delivery, restart Codex after integration changes, and inspect compatibility warnings. Events received while the desktop endpoint is unavailable are recovered only if the hook can write to the same local spool.

If an action does not run, restart Lili after editing the file, confirm the executable exists and is executable, remove filters from pet-triggered actions, and review action diagnostics. Remember that the inherited environment is cleared and the default Unix `PATH` is only `/usr/bin:/bin`.

## 7. Uninstall or reset

Remove managed Session integration before deleting the release directory that contains `lili-hook`:

```text
"$LILI_RELEASE/bin/lili" integrate uninstall
```

Only a result with `complete: true` confirms that owned entries were removed or safely restored. Pet packages are deliberately retained. For backup handling, local-state reset, or complete data removal, follow [Security and operations](security-and-operations.md#backup-reset-and-uninstall).
