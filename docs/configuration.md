# Configuration

This guide covers the supported path from launching an unpacked Lili release to receiving Codex Session notifications and running local programs for pet or notification interactions. It does not describe building Lili. Read [Security and operations](security-and-operations.md) before enabling native actions.

## 1. Launch an unpacked release

Obtain and unpack the release archive for the current platform. The release root contains:

- the desktop bundle below `bundles/`;
- `bin/lili` for integration management;
- the matching `bin/lili-hook` forwarder;
- the built-in pet files under `pets/lili/`;
- this guide and an action example.

Set `LILI_RELEASE` to that stable absolute location, then launch the platform desktop bundle. On macOS, for example:

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

## 2. Lili application storage

The desktop application and `lili-hook` use Lili-owned platform application storage. They do not use `CODEX_HOME`, the current directory, Documents, or Desktop for desktop state or event forwarding.

Typical application roots are:

| Platform | Application root |
| --- | --- |
| macOS | `~/Library/Application Support/dev.linw1995.lili/` |
| Linux | `$XDG_STATE_HOME/dev.linw1995.lili/`, or `~/.local/state/dev.linw1995.lili/` |
| Windows | `%LOCALAPPDATA%\dev.linw1995.lili\` |

The root contains the SQLite database `lili.sqlite3`, the application-owned `pets/` directory, `config/actions.toml`, owner-only runtime credentials and endpoint metadata, and SQLite WAL/SHM sidecars. The structured state, reducer records, notifications, plugin evidence, and offline spool are stored in SQLite with embedded migrations, WAL, foreign-key checks, and bounded transactions.

Existing `${CODEX_HOME}/lili`, `${CODEX_HOME}/pets`, and `${CODEX_HOME}/pet` paths are intentionally ignored. This release does not migrate, delete, or read them.

`CODEX_HOME` is used only by an explicitly invoked `lili integrate` command to inspect or update Codex configuration. It is not a desktop configuration switch.

## 3. Configure pet packages

Lili always has an embedded Lili fallback. External packages are discovered only below the application root's `pets/` directory. A valid package at `<LILI_DATA>/pets/lili/` replaces the embedded fallback:

```text
<LILI_DATA>/pets/lili/pet.json
<LILI_DATA>/pets/lili/spritesheet.webp
<LILI_DATA>/pets/<other-pet-id>/pet.json
<LILI_DATA>/pets/<other-pet-id>/spritesheet.webp
```

To install the release copy as the external default package:

```text
mkdir -p "$LILI_DATA/pets/lili"
cp "$LILI_RELEASE/pets/lili/pet.json" "$LILI_DATA/pets/lili/pet.json"
cp "$LILI_RELEASE/pets/lili/spritesheet.webp" "$LILI_DATA/pets/lili/spritesheet.webp"
```

Set `LILI_DATA` to the platform application root shown above for a manual package installation. The old Codex package directories are not scanned or migrated.

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

## 4. Enable legacy fallback Session notifications

Run integration commands with the release `bin/lili`, not the executable inside the desktop bundle. This ensures the planned hook path names the matching sibling `bin/lili-hook`.

The Codex plugin is the primary installation path. Use the direct-configuration workflow below only as an explicit legacy fallback when the Lili plugin is unavailable or current policy prevents Marketplace installation. Do not remove a working legacy integration until trusted plugin delivery has been verified.

Start Lili, then inspect the current Codex configuration:

```text
"$LILI_RELEASE/bin/lili" integrate inspect
```

Generate a plan and review the complete JSON before applying it:

```text
"$LILI_RELEASE/bin/lili" integrate plan --legacy-fallback > ./lili-plan.json
cat ./lili-plan.json
"$LILI_RELEASE/bin/lili" integrate install --legacy-fallback --plan ./lili-plan.json
"$LILI_RELEASE/bin/lili" integrate inspect
```

Only install a plan whose `status` is `ready`. The plan shows the exact target files, expected hashes, backup paths, `notify` argv, hook additions, trust requirements, and synthetic verification command. Installation refuses a stale plan if either target file changes after planning.

An existing non-Lili `notify` command produces `status: "conflict"`. If both commands are required, create a coexistence plan explicitly and review the preserved command before installation:

```text
"$LILI_RELEASE/bin/lili" integrate plan --legacy-fallback --coexist > ./lili-plan.json
cat ./lili-plan.json
"$LILI_RELEASE/bin/lili" integrate install --legacy-fallback --plan ./lili-plan.json
```

The installation may update `${CODEX_HOME}/config.toml` and `${CODEX_HOME}/hooks.json`, creates timestamped backups for changed existing files, and records managed provenance in `${CODEX_HOME}/lili/integration.json`. Permission notifications remain observer-only; Lili never approves or denies a request.

### Migrating from legacy fallback to the plugin

Lili's migration assessment keeps the legacy fallback active while the plugin is installed. It never writes Codex trust state. Review and trust the exact hook definitions in Codex yourself, then produce one real plugin-attributed lifecycle event and run `lili integrate assess --plugin <plugin@marketplace> > lili-plugin-migration-assessment.json`. The command verifies current hook trust, performs authenticated synthetic delivery and cross-source deduplication, and saves a separate runtime-authenticated verification receipt. Only its unedited `cleanup_ready` assessment permits provenance-aware cleanup. Cleanup verifies the receipt and assessment digest, starts a fresh Codex hook inspection, and requires every selected plugin hook to remain enabled and trusted, the installed plugin version and real event identity to match the authenticated evidence, and the selected Marketplace identity and `CODEX_HOME` to remain unchanged. A failed install or post-install compatibility check removes only the newly installed plugin; a failed trust or verification precondition leaves the legacy integration unchanged. An unreviewed Codex version may proceed to cleanup only after the same real-delivery and current-trust requirements are satisfied; an unknown version remains blocked.

After installation, restart Codex and start a new Session so it reads the updated configuration. A synthetic verification event may appear during installation. If Lili is temporarily stopped, supported events are written to the bounded local spool and consumed on the next start.

## 5. Configure interaction actions

Native actions are optional and are loaded from `$LILI_DATA/config/actions.toml` when Lili starts. Copy the release example, then replace every placeholder executable path:

```text
mkdir -p "$LILI_DATA/config"
cp "$LILI_RELEASE/examples/actions.toml" "$LILI_DATA/config/actions.toml"
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
policy = "application_data"
```

Filters support `providers`, `notification_kinds`, and `project_labels`. They require a notification context, so omit `[action.filters]` from pet click and pet double-click actions.

Working-directory policies are `application`, `application_data`, and `explicit`. An `explicit` policy also requires an existing absolute `path`. Actions receive only a minimal process environment; add fixed values explicitly when needed:

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

If Session notifications do not arrive, run `lili integrate inspect`, verify that the exact Hook definitions are trusted, keep Lili running for direct delivery, restart Codex after integration changes, and inspect compatibility warnings. Events received while the desktop endpoint is unavailable are recovered in the Lili-owned SQLite spool.

If an action does not run, restart Lili after editing the file, confirm the executable exists and is executable, remove filters from pet-triggered actions, and review action diagnostics. Remember that the inherited environment is cleared and the default Unix `PATH` is only `/usr/bin:/bin`.

## 7. Uninstall or reset

Remove managed Session integration before deleting the release directory that contains `lili-hook`:

```text
"$LILI_RELEASE/bin/lili" integrate uninstall
```

Only a result with `complete: true` confirms that owned entries were removed or safely restored. Pet packages are deliberately retained. For backup handling, local-state reset, or complete data removal, follow [Security and operations](security-and-operations.md#backup-reset-and-uninstall).
