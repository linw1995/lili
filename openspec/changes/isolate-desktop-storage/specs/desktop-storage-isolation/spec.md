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

#### Scenario: Application root is not representable as UTF-8

- **WHEN** the platform resolves an absolute application root containing bytes outside UTF-8
- **THEN** path resolution rejects the root with a bounded explicit diagnostic before creating or opening the SQLite database

#### Scenario: Plugin evidence belongs to another Codex home

- **WHEN** an integration inspection loads authenticated plugin evidence bound to a different selected `CODEX_HOME`
- **THEN** the evidence is rejected and cannot satisfy real-delivery or cleanup verification for the current home

#### Scenario: Plugin evidence has no delivery-time Codex-home identity

- **WHEN** authenticated plugin evidence lacks the home identity recorded by the packaged Hook at delivery time
- **THEN** inspection rejects it for home-specific verification and never retroactively assigns it to the selected `CODEX_HOME`

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

#### Scenario: Notification acknowledgement is interrupted

- **WHEN** the user acknowledges a notification while the desktop is running and the reducer snapshot cannot be persisted
- **THEN** the request fails, the in-memory acknowledgement is rolled back, and a restart does not resurrect a mutation that was reported as successful

#### Scenario: Notification acknowledgement succeeds

- **WHEN** the user acknowledges a notification and the reducer snapshot is persisted successfully
- **THEN** the acknowledgement is durable before the mutation reports success and the notification remains acknowledged after restart

#### Scenario: Two hooks claim one spool record

- **WHEN** two forwarder or recovery processes attempt to claim the same offline event concurrently
- **THEN** exactly one claim succeeds, the other observes a stale or already-claimed record, and no duplicate delivery is created by the claim operation

### Requirement: Enforce database invariants and bounded retention

The storage layer MUST enforce valid JSON, valid spool state values, unique spool event identities, and bounded retention for temporary offline events. The durable application projection MUST be replaced rather than appended, and MUST contain at most one presentation-driving unread notification for each persisted Session rather than a notification history or a separate global notification queue. When a Session has multiple unread notifications, the notification with the highest presentation priority MUST be retained, with the newest notification breaking ties.

#### Scenario: Invalid structured detail is written

- **WHEN** a repository receives malformed JSON or an invalid state value
- **THEN** the write is rejected by validation or a database constraint and the existing record remains unchanged

#### Scenario: More than one notification exists for a Session

- **WHEN** a reducer snapshot is persisted after a Session has produced multiple notifications
- **THEN** the durable projection contains only the highest-priority unread notification for that Session, choosing the newest one when priorities tie, while the in-memory reducer may retain the complete current lifecycle

#### Scenario: A notification belongs to a Session outside the persisted projection

- **WHEN** the reducer has more Sessions than the bounded restart projection can retain
- **THEN** the durable projection retains notifications only for the Sessions present in that projection, with no orphan notification record

#### Scenario: An older state-driving Session competes with newer ended Sessions

- **WHEN** an Active or Attention Session is older than enough ended Sessions to exceed the restart projection bound
- **THEN** the state-driving Session is retained and the restored presentation is recomputed from the retained Sessions and notifications

#### Scenario: A newer terminal notification follows unresolved attention

- **WHEN** a Session has an unresolved Attention notification for an older turn and a newer Completion or Failure notification
- **THEN** the durable projection retains the Attention notification so restart restores the same higher-priority Waiting presentation

#### Scenario: A notification-driving Session competes with active Sessions

- **WHEN** the Session projection is full of Active or Attention Sessions and another Session owns an unread Failure or Completion notification
- **THEN** the notification-driving Session is retained before the projection bound is applied, and restart restores Failed or Review presentation as applicable

#### Scenario: An older turn is retried after restart

- **WHEN** a delayed event for one of a Session's recently retired turn identities arrives after the reducer has been restored
- **THEN** the event is ignored as stale without changing the current turn or recreating its notification, while the durable projection remains bounded

#### Scenario: Retention limit is reached

- **WHEN** offline spool records exceed their configured bound
- **THEN** retention counts all rows toward the byte bound, removes only pending records allowed by the priority and age policy, preserves claimed rows for lease recovery, and never allows pending data to grow without a bound

#### Scenario: A duplicate hook event is already expired

- **WHEN** a Hook retries an event identity whose existing pending spool row is older than the age bound
- **THEN** the expired pending row is removed and counted before the conflict-safe insert, so the retry stores a current bounded record rather than bypassing age retention

#### Scenario: A caller supplies a spool limit above the SQLite hard bound

- **WHEN** a spool store is configured above 256 records, 4 MiB, or 24 hours
- **THEN** the store rejects the configuration instead of allowing Rust and SQLite retention policies to diverge

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

#### Scenario: Plugin hook omits optional Codex-home provenance

- **WHEN** a supported plugin launcher provides the guaranteed plugin environment but no `CODEX_HOME`
- **THEN** the forwarder still delivers or spools the attributed event, does not emit a Codex filesystem diagnostic, and records no home-bound evidence that could later be assigned to an integration assessment

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
