## Purpose

Defines supported ChatGPT/Codex session event ingestion and the stable, privacy-preserving lifecycle behavior that drives the desktop pet across provider-version changes.

## ADDED Requirements

### Requirement: Ingest events through supported external surfaces
The system SHALL accept Codex `notify` payloads and configured lifecycle-hook payloads through a dedicated forwarder, and SHALL NOT derive events by reading Codex credentials, internal databases, rollout logs, or private application memory.

#### Scenario: Agent turn completion is forwarded
- **WHEN** Codex invokes the configured `notify` command with an `agent-turn-complete` payload
- **THEN** the forwarder emits one normalized terminal turn event containing the available thread, turn, project, and summary metadata

#### Scenario: Lifecycle hook is forwarded
- **WHEN** a configured session, prompt, stop, permission, or session-end hook invokes the forwarder
- **THEN** the forwarder emits the corresponding normalized lifecycle event and exits without deciding, approving, denying, or otherwise changing the Codex session

### Requirement: Normalize provider events into a versioned model
The system SHALL normalize accepted payloads into a versioned event envelope with a provider, event type, session identifier, optional turn identifier, occurrence time, project context, display-safe summary, and source capabilities. Unknown provider fields SHALL be ignored, while missing required identity or type fields SHALL produce a bounded diagnostic instead of a state transition.

#### Scenario: New provider field is present
- **WHEN** a supported payload includes an unrecognized additive field
- **THEN** the known event fields are still normalized successfully

#### Scenario: Required identity is absent
- **WHEN** a payload cannot identify a session or event type
- **THEN** the event is rejected, no pet state changes, and no raw payload is written to ordinary logs

### Requirement: Deliver events over a private local channel
The system SHALL authenticate event delivery to the running application over a user-scoped local endpoint with owner-only access, a bounded message size, and replay-resistant instance credentials.

#### Scenario: Authorized forwarder delivers an event
- **WHEN** a forwarder owned by the same user presents current instance credentials and a valid event
- **THEN** the application acknowledges the event after durable handoff or in-memory acceptance

#### Scenario: Unauthorized or oversized delivery is attempted
- **WHEN** credentials are absent or invalid, peer ownership is invalid, or the payload exceeds the configured bound
- **THEN** the application rejects the delivery without changing session or pet state

### Requirement: Preserve bounded events while the UI is unavailable
The forwarder SHALL atomically spool normalized events when the application endpoint is unavailable, and the application SHALL consume unexpired events on startup. The spool SHALL have count, byte, and age limits and SHALL discard the oldest terminal events before dropping an attention-required event.

#### Scenario: Application starts after an offline completion
- **WHEN** a valid completion event was spooled within the retention window
- **THEN** the application consumes it once and presents it as an unread session notification

#### Scenario: Spool reaches its bound
- **WHEN** adding an event would exceed a count or byte limit
- **THEN** the eviction policy preserves the newest and highest-attention events and records only aggregate drop counts

### Requirement: Apply events idempotently and monotonically
The system SHALL deduplicate repeated deliveries and SHALL prevent a stale nonterminal event from reopening a terminal turn. A new turn identifier for the same session SHALL start a new lifecycle generation.

#### Scenario: Event is delivered twice
- **WHEN** the same source event is received more than once
- **THEN** it produces one state transition and at most one visible notification

#### Scenario: Stale active event follows completion
- **WHEN** an active event for a turn arrives after that turn has completed or failed
- **THEN** the terminal state is retained

### Requirement: Aggregate concurrent sessions deterministically
The system SHALL track each session independently and SHALL select the ambient pet state by priority: attention required, newest unacknowledged failure, newest unread completion, any active turn, then idle. Notification cards SHALL remain associated with their original session and turn even when the ambient state changes.

#### Scenario: One session runs while another needs input
- **WHEN** at least one session is active and another has an unresolved attention event
- **THEN** the pet displays waiting behavior and exposes the attention notification first

#### Scenario: Attention is resolved while another session is active
- **WHEN** the attention event is resolved or its turn ends and another turn remains active
- **THEN** the pet transitions to running behavior without losing queued terminal notifications

### Requirement: Install integration without silently replacing user configuration
The integration workflow SHALL show the exact configuration changes before applying them, preserve unrelated settings and hooks, create a recoverable backup, and require explicit conflict resolution before replacing or chaining an existing `notify` command.

#### Scenario: Clean integration install
- **WHEN** the user accepts a preview with no conflicting Lili integration
- **THEN** configuration is updated atomically and a verification event confirms delivery

#### Scenario: Existing notification command conflicts
- **WHEN** Codex already has a different `notify` command
- **THEN** the installer leaves it unchanged until the user explicitly chooses a supported coexistence strategy
