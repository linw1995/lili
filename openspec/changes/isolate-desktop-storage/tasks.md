## 1. Storage Contract and Shared Resolver

- [x] 1.1 Add a shared `ApplicationPaths` abstraction for Lili persistent data, SQLite, configuration, runtime, and Pet roots; verify platform defaults use the Lili application identity and never fall back to `CODEX_HOME`, the current directory, Documents, or Desktop.
- [x] 1.2 Add the shared storage crate to the workspace and make it usable by both the Tauri desktop binary and standalone `lili-hook`; verify both processes resolve the same database and relative file layout on every supported platform.
- [x] 1.3 Define application directory creation, database ownership, runtime permissions, and sidecar handling; verify the database, WAL/SHM files, credentials, and endpoint metadata remain owner-only on Unix.

## 2. SQLite Foundation and Schema

- [x] 2.1 Add Diesel, Diesel migrations, and bundled SQLite dependencies through the pinned workspace/Flake toolchain; verify lockfiles and clean builds resolve the same SQLite implementation across supported targets.
- [x] 2.2 Implement database opening and initialization with embedded migrations, `foreign_keys = ON`, WAL mode, a 5000ms desktop busy timeout, and a short Hook-specific busy timeout with a bounded initialization deadline; verify first-open behavior and bounded connection errors.
- [x] 2.3 Add one initial schema for application metadata, compact reducer snapshots, inbound spool, and latest plugin evidence; verify JSON validity, state checks, unique spool identities, indexes, and retention-related constraints.
- [x] 2.4 Add Diesel models, generated schema, typed codecs, and repositories for the compact projection, spool, and latest plugin evidence; verify business CRUD uses query-builder APIs and raw SQL appears only in the migration or fixed connection PRAGMAs.
- [x] 2.5 Add initial-migration, rollback, constraint, and database-integrity tests; verify a failed migration or transaction does not leave a partially initialized application store.

## 3. Persisted Application State and Pet Resources

- [x] 3.1 Replace `AppStateStore` JSON persistence with a SQLite repository for application metadata, selected Pet, window placement, reducer revision, one state notification, and bounded retired-turn replay tombstones per Session; verify the restored reducer snapshot matches the compact persisted state.
- [x] 3.2 Persist reducer transitions by replacing the current projection in one short database operation; verify failed persistence restores the in-memory reducer and does not append event history.
- [x] 3.3 Remove the separate legacy selection-file path and make selected Pet state database-backed while retaining only the embedded fallback when the selected app-owned package is unavailable; verify `${CODEX_HOME}/lili/selected-pet.json` is ignored and untouched.
- [x] 3.4 Refactor Pet catalog discovery to scan only the application-owned `pets/<id>` file tree while preserving Pet v2 validation; verify packages under `${CODEX_HOME}/pets` and `${CODEX_HOME}/pet` are ignored and untouched.
- [x] 3.5 Move action configuration to the application-owned `actions.toml` path and update the action context to stop modeling `CODEX_HOME`; verify missing, malformed, and valid configuration behavior from the new root.

## 4. Runtime, Spool, and Forwarder Storage

- [x] 4.1 Keep rotating transport credentials and endpoint metadata in owner-only runtime files while moving plugin evidence into the SQLite repository; verify atomic credential replacement and authenticated evidence reload.
- [x] 4.2 Implement the inbound spool as a SQLite repository with normalized event payloads, priority/age bounds, claim leases, acknowledgements, and eviction; verify crash recovery and bounded retention.
- [x] 4.3 Update the desktop runtime to share one database connection/repository composition with the ingestion actor and recover pending spool claims on startup; verify no transaction remains open during socket or WebView I/O.
- [x] 4.4 Update `lili-hook` to resolve only Lili application paths, insert or claim spool records through SQLite, and pass plugin attribution explicitly from the hook launcher; verify forwarding works when `CODEX_HOME` is absent or inaccessible.
- [x] 4.5 Run concurrent direct and plugin hook delivery against one database; verify WAL locking, busy-timeout behavior, single-winner claims, stable event identities, replay protection, and no duplicate delivery.

## 5. Explicit Codex Integration Boundary

- [x] 5.1 Isolate `resolve_codex_home` and Codex inspection/configuration access inside explicit integration CLI paths; verify desktop launch, tray diagnostics, background ingestion, and hook forwarding do not invoke those functions.
- [x] 5.2 Update direct integration and Marketplace hook commands for the new forwarder contract without adding runtime `CODEX_HOME` access; verify explicit `lili integrate inspect` remains the only normal path that reads Codex configuration.
- [x] 5.3 Remove automatic migration, cleanup, fallback, and legacy-path probing from desktop startup and storage repositories; verify old `${CODEX_HOME}/lili`, `${CODEX_HOME}/pets`, and `${CODEX_HOME}/pet` trees remain byte-for-byte unchanged.

## 6. Breaking Behavior and Failure Boundaries

- [x] 6.1 Add clean-start tests that set `CODEX_HOME` to an existing sentinel or protected directory and assert that desktop startup, database initialization, Pet discovery, actions, and forwarding create no files below it.
- [x] 6.2 Add database-open, migration, permission, corruption, and storage-integrity diagnostics; verify Lili fails closed without trying `CODEX_HOME`, the project directory, Documents, or Desktop as a fallback.
- [x] 6.3 Add explicit diagnostics and release behavior for ignored old data and required user-initiated integration reconfiguration; verify startup does not rewrite existing hook or plugin configuration.

## 7. Verification and Packaged Acceptance

- [x] 7.1 Add unit and integration coverage for the single schema migration, transaction rollback, JSON constraints, compact projection bounds, spool retention, cross-process claims, and restart recovery.
- [x] 7.2 Update macOS, Windows, and Linux desktop acceptance fixtures to provision isolated Lili application roots and SQLite stores while keeping Codex configuration in a separate sentinel root; verify platform-native paths, permissions, WAL sidecars, and hook delivery.
- [x] 7.3 Run the complete Flake-provided Rust checks, tests, Web tests, packaged desktop smoke/acceptance, plugin checks, and `openspec validate --changes --strict --no-interactive`; verify all gates pass without modifying lockfiles or user Codex data.

## 8. Documentation and Release Surface

- [x] 8.1 Rewrite configuration and security documentation to describe the platform-native Lili directory, SQLite database, Pet file tree, actions file, runtime files, and explicit integration boundary; verify release manifests contain the updated guides.
- [x] 8.2 Update Marketplace/plugin skill text, diagnostics wording, privacy policy, and support material so they describe SQLite-backed local state without claiming desktop storage or hook forwarding uses Codex directories; verify consistency checks pass.
- [x] 8.3 Add a release note for the intentional breaking storage change, including a fresh SQLite schema, ignored old data, and explicit integration reinstallation requirements; verify the note is included in the release artifact checklist.
