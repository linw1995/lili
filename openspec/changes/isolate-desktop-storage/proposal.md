## Why

The desktop runtime currently treats `CODEX_HOME` as Lili's storage root, so normal startup reads and writes Codex-adjacent state, Pet packages, actions, runtime credentials, evidence, and spool files. This couples an independently distributed desktop application to protected user data and can trigger macOS Documents-folder authorization during development.

The storage boundary must be corrected before more desktop features are added. The new boundary is intentionally breaking: Lili will start with fresh application-owned storage and will not migrate or read the previous Codex-rooted data layout.

## What Changes

- **BREAKING** Remove `CODEX_HOME` resolution from the normal desktop runtime and from the hook forwarder.
- Add a shared application-storage path resolver used by the Tauri desktop process and standalone hook binary.
- Store structured desktop state, reducer/session records, plugin evidence, and offline spool data in a versioned SQLite database under the platform-native Lili application data directory.
- Keep Pet assets in an application-owned file tree, keep human-edited actions in `actions.toml`, and keep short-lived runtime credentials and endpoint metadata in owner-only runtime files.
- Preserve the Pet v2 manifest and atlas format while removing the `${CODEX_HOME}/pets` location contract.
- Limit Codex filesystem access to explicit `lili integrate` commands that the user invokes; startup and event forwarding must not inspect Codex files.
- **BREAKING** Ignore existing `${CODEX_HOME}/lili`, `${CODEX_HOME}/pets`, and legacy singular Pet directories without reading, deleting, or migrating them.
- Update plugin/direct-hook setup, diagnostics, tests, packaged acceptance, release documentation, and privacy wording for the new boundary.

## Capabilities

### New Capabilities

- `desktop-storage-isolation`: Defines the application-owned storage boundary, explicit Codex integration boundary, fresh-state behavior, and isolation guarantees for the desktop runtime and event forwarder.

### Modified Capabilities

None. No archived main specifications exist yet; this capability is the breaking replacement contract for the active storage assumptions in the earlier desktop-pet and plugin changes.

## Impact

- Refactors path construction across `lili`, `lili-app-state`, `lili-pet`, `lili-actions`, `lili-session`, and hook-forwarding code.
- Adds a shared path/storage abstraction so Tauri and standalone hook processes resolve the same Lili-owned directories without consulting `CODEX_HOME`.
- Adds a bundled SQLite/Diesel storage layer with embedded migrations, schema constraints, WAL, busy-timeout configuration, and short repository transactions.
- Changes plugin and direct integration verification so only explicit integration commands inspect or mutate Codex configuration.
- Requires new isolation, permissions, concurrency, offline-spool, and packaged desktop acceptance coverage.
- Changes user-visible configuration and release documentation. Existing Lili state and external Pet packages are deliberately not carried forward.
