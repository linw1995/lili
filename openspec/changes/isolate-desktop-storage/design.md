## Context

See `proposal.md` for the motivation. The current desktop entry point resolves `CODEX_HOME` before loading application state, Pet packages, actions, native runtime credentials, plugin evidence, and offline spool data. The standalone hook forwarder resolves the same directory so it can read credentials and enqueue events. The integration CLI also resolves `CODEX_HOME` to inspect or install Codex configuration.

These are three different responsibilities but currently share one path root:

1. The desktop application needs private Lili state and local runtime services.
2. The hook forwarder needs to reach those private Lili runtime services quickly and while the desktop is offline.
3. Explicit integration commands need to inspect or update Codex configuration.

Only the third responsibility should touch `CODEX_HOME`. The first two must work without reading Codex files, without using the current working directory, and without falling back to a protected user folder.

## Goals / Non-Goals

**Goals:**

- Make the desktop runtime and hook forwarder independent of `CODEX_HOME`.
- Use one deterministic, platform-native Lili storage layout shared by Tauri and standalone native binaries.
- Use SQLite as the authoritative store for the compact latest per-Session state projection, plugin evidence, and offline recovery data, with database-level concurrency and integrity guarantees.
- Keep Pet assets, human-edited action configuration, and short-lived runtime credentials separated from the database and protected with appropriate local permissions.
- Preserve the Pet v2 manifest and atlas contract while changing the package location to an Lili-owned directory.
- Make Codex access explicit, auditable, and limited to user-invoked integration commands.
- Make the breaking fresh-state behavior observable and testable.

**Non-Goals:**

- Migrating, importing, copying, deleting, or inspecting any previous Lili or Pet data under `CODEX_HOME`.
- Maintaining compatibility with the old `${CODEX_HOME}/lili`, `${CODEX_HOME}/pets`, or singular `${CODEX_HOME}/pet` layouts.
- Reading Codex credentials, databases, rollout files, or private session storage from the desktop runtime or hook forwarder.
- Providing a user-configurable arbitrary storage-root environment variable in the production path resolver.
- Changing the normalized session event protocol or the Pet v2 image geometry.

## Decisions

### Use a shared application path resolver

Introduce a small platform-neutral path abstraction, such as `ApplicationPaths`, in a crate that can be linked by both the Tauri desktop binary and `lili-hook`. It resolves the Lili application identity `dev.linw1995.lili` to platform-native application directories. The Tauri process and standalone hook must use this same resolver rather than independently guessing paths.

The resolver exposes separate roots for persistent data, editable configuration, and short-lived runtime data. The production resolver has no `CODEX_HOME`, current-directory, or Documents-folder fallback. Unit and acceptance tests construct the same abstraction with an explicit temporary root instead of changing production environment semantics.

The expected layout is:

```text
application-data/
  lili.sqlite3
  pets/<pet-id>/
application-config/
  actions.toml
application-runtime/
  credentials.json
  endpoint/
```

The exact platform parent is delegated to the operating system application-data conventions, while the relative layout and application identity remain stable and are covered by path-contract tests.

Alternatives considered:

- Keeping `CODEX_HOME` as the default would preserve the privacy problem and make the desktop application depend on another product's storage policy.
- Using the current working directory would make development, packaging, and launchers resolve different state roots.
- Giving users a general environment override would make accidental Documents-folder access easy to reintroduce and would require a new compatibility contract.

### Use SQLite as the authoritative structured storage layer

Create one SQLite database under the Lili application data root. Use Diesel's generated schema and query builder for repository operations, `diesel_migrations` for embedded schema migrations, and the bundled SQLite library so the desktop and hook binaries do not depend on a system SQLite installation.

Every connection applies the same initialization contract: foreign keys are enabled, WAL mode is enabled for writable connections, and `busy_timeout` is set to 5 seconds. The database is owner-only, and its WAL/SHM sidecars remain inside the same protected application data directory.

The schema follows the reference storage pattern: stable query fields use typed columns and constraints, while the bounded session projection uses one validated JSON snapshot. The initial schema contains only:

- `app_state`: one row for selected Pet, window placement, reducer revision, and a compact reducer snapshot containing the current state and at most one latest notification per Session;
- `inbound_spool`: normalized offline events with priority, lease/claim metadata, and retention timestamps; rows exist only until delivery is committed or retention evicts them;
- `plugin_evidence`: the latest authenticated plugin diagnostics and bounded metadata.

The reducer remains the in-memory authority for rendering. A reducer transition replaces the current application projection in one short database update. Event history, a persisted deduplication log, and separate session/turn/notification tables are intentionally excluded because they duplicate the reducer projection and are not required after restart. Hook claims, acknowledgements, and spool eviction also use short transactions. No transaction may remain open while waiting for a socket, process, WebView, or external command.

Business CRUD uses Diesel repositories rather than handwritten SQL. Raw SQL is limited to embedded migration files and the fixed connection PRAGMAs. Schema constraints enforce valid identities, JSON validity, state values, unique spool identities, and bounded timestamps. The only multi-row retention policy applies to the temporary inbound spool; the durable application projection is replaced rather than appended.

Alternatives considered:

- Independent JSON files provide simple atomic snapshots but cannot make reducer state, event history, and spool claims consistent across the desktop and concurrent hook processes.
- A process-local mutex would not coordinate standalone hook processes; SQLite locking and transactions are the cross-process authority.
- Storing Pet spritesheets or `actions.toml` as database blobs would remove useful file-level validation and human editability without improving state consistency.

### Keep database and file resources separate

SQLite stores structured and queryable application state. Pet manifests and spritesheets remain confined files under the application-owned Pet root, where existing v2 validation can inspect file metadata and asset paths. `actions.toml` remains in the application configuration root because it is user-authored configuration. Rotating runtime credentials and endpoint metadata remain owner-only files in the runtime root because they are short-lived transport material rather than application history.

The database path resolver and file path resolver are returned together by `ApplicationPaths`, so all processes agree on the storage boundary without making every crate derive paths independently.

### Inject application paths into the desktop composition

Resolve application paths after the Tauri application is built, open the SQLite database once for the desktop composition, and pass repositories plus file paths into state loading, Pet catalog construction, action loading, runtime setup, spool recovery, and diagnostics. The desktop startup path must not call `resolve_codex_home`, run Codex inspection, or construct a Codex-rooted path.

The domain crates remain path-agnostic: stores receive explicit repositories, database connections, or an `ApplicationPaths` projection. Constructors named or implemented around `for_codex_home` are removed from desktop-owned stores. This keeps path ownership at the composition boundary and prevents a future caller from silently restoring the old coupling.

If the database cannot be created, opened, migrated, or validated, the desktop process fails closed with a bounded diagnostic or runs only the explicitly supported embedded fallback mode. It never retries with `CODEX_HOME`, the project directory, or a user Documents path.

Alternatives considered:

- Letting every crate resolve its own directories would recreate inconsistent roots between the desktop and hook processes.
- Keeping a global resolver in the Pet crate would make an unrelated asset module responsible for application lifecycle storage.

### Keep Pet format compatibility but remove location compatibility

Pet manifests, atlas geometry, animation rows, and validation rules remain unchanged. The catalog scans only the Lili-owned `application-data/pets` root. A missing or invalid package selects the embedded Lili Pet and records a local diagnostic.

The catalog never scans `${CODEX_HOME}/pets`, `${CODEX_HOME}/pet`, symlinked aliases to those paths, or arbitrary paths from a manifest. Existing packages in those locations remain untouched and are not surfaced by the desktop UI.

Alternatives considered:

- Retaining a read-only Codex Pet fallback would still trigger protected-directory access and would make the new isolation guarantee false.
- Replacing the v2 format would create an unnecessary asset migration while solving a storage-boundary problem.

### Make the hook forwarder use Lili runtime storage only

The hook forwarder uses the shared application path resolver to load credentials, deliver to the local endpoint, and enqueue offline records. It does not resolve `CODEX_HOME`, inspect Codex plugin state, or discover the installed plugin by reading Codex files.

Plugin attribution is passed as an explicit, validated hook argument or equivalent package-owned metadata. The hook launcher does not need to query Codex to determine its own plugin identity. Direct integration hooks and Marketplace hooks therefore share the same local transport without sharing a Codex filesystem dependency.

Credential rotation remains an owner-only atomic file replacement. Spool insertion, claims, acknowledgements, eviction, and concurrent hook delivery use the SQLite repository with authenticated frames and replay protection. The hook never holds a database transaction while waiting for the desktop endpoint.

Alternatives considered:

- Reading Codex plugin state from each hook invocation is slow, privacy-sensitive, and unnecessary when the hook source is already known by the launcher.
- Passing a temporary socket path through every hook invocation would complicate installation and make offline recovery less reliable.

### Keep Codex access behind explicit integration commands

`lili integrate inspect`, assessment, planning, installation, cleanup, and uninstall remain the only supported operations that may resolve or access `CODEX_HOME`. They must receive the Codex root through their explicit command boundary and must not be called by ordinary desktop startup, tray setup, diagnostics refresh, or hook forwarding.

The integration CLI may provision or update Codex hook configuration when the user explicitly invokes it. The resulting hook command points to the Lili forwarder, which uses application-owned storage and does not inherit a requirement to inspect Codex files at runtime.

Alternatives considered:

- Removing all integration commands would prevent the product from receiving supported Codex events.
- Allowing the desktop process to inspect Codex configuration for convenience would reintroduce the same authorization prompt and violate the ownership boundary.

### Treat the storage change as a clean break

The new release does not read old state, Pet selection, actions, runtime credentials, evidence, spool files, or package directories. It does not delete or rewrite them either. The first launch uses defaults and creates only the new Lili-owned directories.

Existing Codex integration configuration is not silently rewritten by desktop startup. If the new hook command or explicit integration workflow requires reinstallation, the user performs that operation through the supported integration command or Plugin Directory flow. The release notes and diagnostics state that old Lili data is intentionally ignored.

Rollback means running the previous application version against its untouched old files; the new application does not provide a migration or rollback bridge.

## Risks / Trade-offs

- [Users lose automatic access to old state and external Pet packages] → State the breaking change prominently, keep old files untouched, and provide the embedded Pet as the clean default.
- [The Tauri resolver and standalone hook resolver can diverge on one platform] → Centralize the resolver, assert the same application identity and relative layout in platform tests, and run packaged hook-to-desktop acceptance.
- [SQLite migrations or lock contention can prevent startup or event delivery] → Embed migrations, configure WAL and a bounded busy timeout, keep transactions short, fail closed on migration errors, and test concurrent desktop/hook access.
- [Database corruption can affect more state than one JSON file] → Enforce JSON validity, unique spool identities, bounded projections, owner-only permissions, and explicit integrity diagnostics.
- [An existing hook may stop working until the new runtime contract is provisioned] → Make explicit integration verification report the required reinstallation state; never silently mutate Codex configuration during desktop startup.
- [Application-data permissions can still be changed or denied by the user] → Fail closed, report a bounded diagnostic, and do not search for a fallback path.
- [A user-configured action may intentionally execute in a protected directory] → Keep action execution explicit, argv-only, and user-configured; do not use action working directories as a startup storage mechanism.
- [Removing automatic Codex Pet discovery reduces ecosystem convenience] → Preserve the Pet v2 format and provide an explicit app-owned package installation path in a later change if needed.

## Migration Plan

There is no migration plan by design. On first launch after this change, Lili creates a new application-owned SQLite database, applies its embedded initial migrations, and ignores existing Lili/Codex-rooted files without reading or deleting them. Users who need Codex event delivery must explicitly run the current integration or Plugin Directory workflow and complete its verification. Users who need old state or Pet packages can continue using the previous application version; the new version does not import them.
