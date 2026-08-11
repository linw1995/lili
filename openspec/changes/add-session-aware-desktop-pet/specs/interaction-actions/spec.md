## Purpose

Defines how user-approved configuration maps pet and session-notification interactions to bounded local executable invocations without introducing shell interpretation or implicit approval behavior.

## ADDED Requirements

### Requirement: Configure versioned declarative actions
The system SHALL load a versioned action configuration whose entries have a stable identifier, supported interaction trigger, optional event filters, executable argv array, timeout, concurrency policy, and explicitly allowed environment values.

#### Scenario: Valid notification-click action loads
- **WHEN** an action declares a supported notification-click trigger and a nonempty executable argv array
- **THEN** it is enabled and shown in the effective configuration view

#### Scenario: Invalid action is present
- **WHEN** an entry has an unsupported schema version, trigger, empty command, invalid timeout, or duplicate identifier
- **THEN** that entry is disabled with a precise diagnostic and other valid entries remain available

### Requirement: Execute argv directly without a shell
The system SHALL invoke the configured executable and arguments directly, SHALL NOT evaluate a shell command string, and SHALL NOT interpolate event data into command arguments. Event context SHALL be supplied as bounded JSON on standard input with a documented schema.

#### Scenario: Event contains shell metacharacters
- **WHEN** a project label or summary contains quotes, substitutions, separators, or control characters
- **THEN** those bytes remain JSON data and cannot alter the executable or argv

### Requirement: Minimize child process authority
The child SHALL receive only a minimal platform environment plus explicitly configured variables, SHALL run with the configured or application-default working directory, and SHALL NOT receive Codex credentials or raw provider payloads from Lili.

#### Scenario: Action inspects its context
- **WHEN** a notification-click action starts
- **THEN** its JSON input contains the normalized interaction, session, turn, project, and display-safe event fields but no authentication material

### Requirement: Bind interactions to immutable event snapshots
Clicking a session notification SHALL invoke matching actions at most once per accepted interaction using the notification's original normalized event snapshot, even if another session becomes primary before process startup.

#### Scenario: Primary session changes during a click
- **WHEN** the user clicks a completion card and a newer attention event arrives concurrently
- **THEN** the action receives the clicked completion's session and turn context, not the newer primary session

### Requirement: Bound execution and concurrency
The system SHALL enforce per-action debouncing, a global child-process limit, configured timeout capped by an application maximum, bounded stdout and stderr capture, and deterministic queue or reject behavior.

#### Scenario: Action exceeds its timeout
- **WHEN** a child process remains alive beyond the effective timeout
- **THEN** the process tree is terminated where supported, the pet runtime remains responsive, and the execution is recorded as timed out

#### Scenario: Interaction is repeated rapidly
- **WHEN** repeated clicks occur inside an action's debounce window
- **THEN** at most one execution is accepted and the UI does not emit duplicate success or failure feedback

### Requirement: Isolate action failures from session state
An action spawn error, nonzero exit, timeout, or output overflow SHALL NOT acknowledge a Codex approval, change the source session lifecycle, crash the pet, or discard the underlying notification.

#### Scenario: Configured executable is missing
- **WHEN** an interaction matches an executable that cannot be started
- **THEN** the notification remains available and the UI shows a bounded action failure associated with the configured action identifier

### Requirement: Keep a privacy-preserving execution audit
The system SHALL retain a bounded audit containing action identifier, trigger, event identity, start and finish times, outcome, and truncated output metadata, while excluding raw provider payloads and inherited environment values.

#### Scenario: User reviews recent action runs
- **WHEN** the user opens action diagnostics
- **THEN** recent outcomes can be correlated to action and event identities without revealing Codex credentials or complete conversation content
