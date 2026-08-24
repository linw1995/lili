## Purpose

Defines the storage and filesystem ownership boundary that keeps the Lili desktop runtime and event forwarder independent from Codex user data while preserving explicit, user-invoked integration access.

## ADDED Requirements

### Requirement: Resolve desktop storage from Lili-owned application directories

The desktop runtime and event forwarder MUST resolve persistent, configuration, and runtime storage from the Lili application identity and platform-native application directories. They MUST NOT use `CODEX_HOME`, the current working directory, or a fallback under Documents or Desktop.

#### Scenario: Desktop starts with CODEX_HOME set

- **WHEN** the desktop process starts with `CODEX_HOME` set to an existing or protected directory
- **THEN** it resolves all desktop-owned storage outside that directory and does not create, read, or update any Lili file below `CODEX_HOME`

#### Scenario: Application storage cannot be prepared

- **WHEN** the platform application directory cannot be created or fails ownership/permission validation
- **THEN** Lili reports a bounded storage diagnostic and fails closed or enters only the explicitly supported embedded fallback mode without trying another user directory

### Requirement: Keep desktop-owned data separated by purpose

The desktop runtime MUST store application metadata, a compact latest per-Session state projection, offline spool records, and latest plugin evidence in a versioned SQLite database under the Lili application data root. Pet assets MUST remain in the Lili-owned Pet file tree, action configuration MUST remain in the Lili-owned configuration root, and credentials and endpoint metadata MUST remain in owner-only runtime storage where the platform supports it.

#### Scenario: Desktop persists state on shutdown

- **WHEN** the desktop process exits after moving the Pet or changing session state
- **THEN** the persisted state is written only below the Lili application data root and no file is written below `CODEX_HOME`

#### Scenario: Desktop and hook share runtime storage concurrently

- **WHEN** the desktop rotates runtime credentials while multiple hooks deliver events concurrently
- **THEN** both processes use the same Lili-owned runtime root, atomic replacement and authentication remain valid, and no Codex-rooted runtime file is consulted

### Requirement: Open a versioned SQLite database with a fixed connection contract

The desktop and hook processes MUST open the same Lili-owned SQLite database through a shared path resolver. The database MUST use one embedded initial schema, WAL mode for writable connections, and a bounded busy timeout. A new database MUST start at the current schema without reading any previous Codex-rooted storage.

#### Scenario: First launch creates the application database

- **WHEN** Lili starts with no application database and with `CODEX_HOME` set or unset
- **THEN** it creates the database below the Lili application data root, applies embedded migrations, and does not create a database or sidecar below `CODEX_HOME`

#### Scenario: Database migration fails

- **WHEN** opening the Lili database or applying its migrations fails
- **THEN** Lili reports a bounded storage diagnostic and does not fall back to JSON files, `CODEX_HOME`, the current directory, Documents, or Desktop

### Requirement: Commit state transitions and spool claims transactionally

Structured state replacement, spool claims, acknowledgements, and retention decisions MUST use short database transactions. A reducer transition MUST replace the current projection atomically or leave the prior projection visible. Transactions MUST NOT remain open while waiting for sockets, WebViews, processes, or other external work.

#### Scenario: Reducer transition is interrupted

- **WHEN** persisting a reducer transition fails after the projection update is attempted
- **THEN** the database leaves the prior projection visible, and the in-memory reducer restores its previous state

#### Scenario: Two hooks claim one spool record

- **WHEN** two forwarder or recovery processes attempt to claim the same offline event concurrently
- **THEN** exactly one claim succeeds, the other observes a stale or already-claimed record, and no duplicate delivery is created by the claim operation

### Requirement: Enforce database invariants and bounded retention

The storage layer MUST enforce valid JSON, valid spool state values, unique spool event identities, and bounded retention for temporary offline events. The durable application projection MUST be replaced rather than appended.

#### Scenario: Invalid structured detail is written

- **WHEN** a repository receives malformed JSON or an invalid state value
- **THEN** the write is rejected by validation or a database constraint and the existing record remains unchanged

#### Scenario: More than one notification exists for a Session

- **WHEN** a reducer snapshot is persisted after a Session has produced multiple notifications
- **THEN** the durable projection contains only the latest notification for that Session, while the in-memory reducer may retain the complete current lifecycle

#### Scenario: Retention limit is reached

- **WHEN** offline spool records exceed their configured bound
- **THEN** retention removes only records allowed by the priority and age policy and never allows unbounded database growth

### Requirement: Preserve Pet v2 format without Codex location compatibility

The Pet catalog MUST preserve the supported Pet v2 manifest, atlas geometry, animation, and validation contract while scanning only the Lili-owned Pet root. It MUST use the embedded fallback when no valid app-owned package is available.

#### Scenario: Existing Codex Pet package is present

- **WHEN** a valid Pet package exists only under `${CODEX_HOME}/pets/<id>` or `${CODEX_HOME}/pet/<id>`
- **THEN** the desktop ignores it, leaves it untouched, and uses the embedded fallback or an app-owned package instead

#### Scenario: App-owned Pet package is valid

- **WHEN** a valid v2 package exists below the Lili application Pet root
- **THEN** the catalog validates and exposes that package without reading any Codex directory

### Requirement: Forward events without Codex filesystem access

The hook forwarder MUST load Lili runtime credentials, deliver authenticated events, and enqueue offline records using only Lili-owned application paths. It MUST NOT resolve `CODEX_HOME`, inspect Codex configuration, discover plugin identity from Codex files, or read private Codex state during event forwarding.

#### Scenario: Hook forwards while CODEX_HOME is inaccessible

- **WHEN** a supported hook event arrives with `CODEX_HOME` unset, inaccessible, or pointing to a protected directory
- **THEN** the forwarder still delivers the event through the Lili-owned runtime when available and emits no Codex filesystem error

#### Scenario: Hook spools while the desktop is offline

- **WHEN** the Lili endpoint is unavailable during a valid hook invocation
- **THEN** the forwarder writes a bounded authenticated spool record below the Lili application data root and exits within its forwarding budget

#### Scenario: Multiple hooks arrive concurrently

- **WHEN** multiple direct and plugin-attributed hooks forward events at the same time
- **THEN** all records use the shared Lili-owned spool and runtime roots, retain stable identities, and remain deduplicable without consulting Codex files

### Requirement: Restrict Codex filesystem access to explicit integration commands

Only an explicitly invoked `lili integrate` command MAY resolve or read/write `CODEX_HOME`. Ordinary desktop startup, tray diagnostics, background event ingestion, and hook forwarding MUST NOT invoke Codex inspection or configuration mutation.

#### Scenario: Ordinary desktop launch

- **WHEN** Lili starts without an integration subcommand
- **THEN** it does not inspect Codex version, configuration, plugin state, or hook files, even when `CODEX_HOME` is set

#### Scenario: Explicit integration inspection

- **WHEN** the user invokes `lili integrate inspect`
- **THEN** the command may resolve and inspect the selected Codex home, reports bounded errors when access fails, and does not imply that the desktop runtime has access to that directory

### Requirement: Start clean without legacy storage compatibility

The new desktop runtime MUST NOT read, migrate, copy, delete, or rewrite previous Lili state, Pet packages, action configuration, runtime credentials, evidence, or spool records located under `CODEX_HOME`. It MUST create fresh application-owned state instead.

#### Scenario: Previous Lili data exists under CODEX_HOME

- **WHEN** the new release starts and old `${CODEX_HOME}/lili` or Pet directories exist
- **THEN** the old files remain unchanged, the new application starts from default state, and no migration attempt is made

#### Scenario: Existing integration requires the new runtime contract

- **WHEN** an old direct hook or plugin installation is present after the breaking release
- **THEN** desktop startup does not rewrite it; the user must explicitly run the supported installation and verification flow if reconfiguration is required
