## ADDED Requirements

### Requirement: Configure a dedicated title trigger
The system SHALL accept an optional `session_title` action in version 1 configuration, preserving existing interaction configuration and payloads. Only provider filters SHALL be supported for this trigger. The first enabled matching entry in configuration order SHALL be selected, with no cascading execution after failure.

#### Scenario: Multiple actions match
- **WHEN** two enabled title actions match the same Session
- **THEN** only the first configured action is selected

#### Scenario: An entry uses an unsupported filter
- **WHEN** a title action contains nonempty notification-kind or project-label filters
- **THEN** that entry is disabled with a precise diagnostic and valid entries remain available

#### Scenario: Existing configuration has no title action
- **WHEN** the application loads existing interaction-only configuration
- **THEN** interaction behavior remains unchanged and no title executable runs

### Requirement: Exchange a bounded versioned title protocol
The system SHALL send a dedicated JSON request containing version 1, requestId, trigger, provider, and sessionId through stdin with a 16 KiB limit. It SHALL execute fixed argv directly without interpolation, using the configured working directory and minimal environment. It SHALL accept only successful process output containing one valid version 1 JSON object with a required string-or-null title and no unknown fields, within the existing 16 KiB output bound.

#### Scenario: A title is returned
- **WHEN** the executable exits successfully with a valid title result
- **THEN** whitespace is normalized, remaining control characters are removed, and the title is bounded to 256 Unicode scalar values and displayed as plain text

#### Scenario: A lookup finds no title
- **WHEN** a successful result contains null or a whitespace-only title
- **THEN** the application preserves the current notification title or its existing label fallback

#### Scenario: Output violates the protocol
- **WHEN** output has an unsupported version, unknown or missing fields, invalid encoding, trailing data, or exceeds the output bound
- **THEN** it is rejected and the notification remains available with its current title or fallback presentation

### Requirement: Schedule title work independently of notification delivery
The system SHALL publish notifications before asynchronous lookup and SHALL schedule only for created or updated unread notifications and restored unread notifications. It SHALL coalesce identical in-flight keys, debounce per key, share the global child-process limit, and honor bounded per-action queue or reject policies without automatic retry loops.

#### Scenario: A title action is slow
- **WHEN** a notification arrives while the selected executable is still running
- **THEN** the notification is immediately visible with fallback presentation and the runtime remains responsive

#### Scenario: Multiple Sessions request titles
- **WHEN** eligible events arrive for distinct Session identities
- **THEN** they do not debounce each other and executions remain within global and per-action limits

#### Scenario: The same Session receives repeated events
- **WHEN** a lookup for the same key is already queued or running
- **THEN** subsequent eligible events reuse that lookup rather than enqueueing duplicates

### Requirement: Bound title cache capacity with LRU eviction
The system SHALL retain at most 256 successful nonempty titles in an in-memory LRU cache keyed by provider, sessionId, actionId, and configuration generation. It SHALL NOT use TTLs or expiry timers or cache null/empty results or failures. A hit SHALL promote the entry and directly update the requesting notification without executing a process. Configuration changes and restart SHALL clear the cache. Cache eviction or invalidation SHALL NOT change titles already applied to notifications.

#### Scenario: A cached title is reused
- **WHEN** an eligible notification requests a title whose key is cached
- **THEN** the cached title updates that notification, the entry becomes most recently used, and no executable starts

#### Scenario: Cache capacity is exceeded
- **WHEN** insertion would exceed 256 entries
- **THEN** the least recently used entry is evicted and notifications already using its title retain that title

#### Scenario: A lookup returns no usable title
- **WHEN** execution fails or returns a null or empty title
- **THEN** no cache entry is inserted, the current notification title is preserved, and a later eligible event may retry under execution limits

### Requirement: Update bound notification titles and reject stale results
The system SHALL bind lookup consumers to notification IDs and latest-request tokens and validate unread state, Session identity, selected action, and configuration generation before applying cached or asynchronous output. Valid results SHALL directly update only bound notifications' title fields and SHALL NOT change lifecycle, summaries, notification ordering, acknowledgement, or immutable interaction context. Applied titles SHALL remain independent of cache retention and follow the existing notification persistence lifecycle, with missing title fields supported in older stored notifications.

#### Scenario: Another Session becomes primary
- **WHEN** a result arrives after the primary Session changes
- **THEN** only still-valid notifications bound to the request receive that title

#### Scenario: The target notification was dismissed or removed
- **WHEN** a title result arrives after its target notification was dismissed or removed
- **THEN** the result does not update, recreate, reopen, or acknowledge that notification

#### Scenario: The request is obsolete
- **WHEN** a newer request superseded the notification token or the selected action or configuration generation changed
- **THEN** the old result is discarded

#### Scenario: A notification survives a cache reset
- **WHEN** the cache is cleared or the application restores a retained notification after restart
- **THEN** its previously applied title remains available independently of the empty cache

### Requirement: Isolate query execution failures and diagnostics
The system SHALL enforce existing timeout and process cleanup limits, cancel outstanding work during shutdown, and retain bounded diagnostics containing request identity, outcome, and output counts without raw output or title text. Query results SHALL NOT invoke click feedback or interaction completion effects. User interaction endpoints SHALL NOT dispatch title queries.

#### Scenario: A process times out or fails to start
- **WHEN** title execution exceeds its deadline or cannot be started
- **THEN** applicable process cleanup occurs, diagnostics record the failure, and notification delivery and Session lifecycle remain unaffected

#### Scenario: An executable returns a title successfully
- **WHEN** a valid title result is accepted
- **THEN** no interaction success feedback is shown and no notification is dismissed

#### Scenario: Title text contains markup
- **WHEN** a valid result contains markup-like text
- **THEN** the UI escapes it as text and executes no content from the result


### Requirement: Log execution failures through the runtime logger
The system SHALL emit one structured `warn` event per failed title execution attempt in addition to bounded diagnostics. The event SHALL include action_id, request_id, trigger, a stable failure_kind, duration_ms, and exit_code when available. Failures SHALL include spawn errors, process I/O errors, timeouts, nonzero exits, output overflow, and invalid response protocols. Logs SHALL exclude title text, raw stdout/stderr, request bodies, environment values, and unfiltered error strings that could contain those values.

#### Scenario: A title execution fails
- **WHEN** a title execution attempt ends with an execution or protocol failure
- **THEN** one structured warning identifies the action, request, failure kind, elapsed duration, and available exit code without changing notification behavior

#### Scenario: Coalesced consumers share a failure
- **WHEN** one failed execution serves multiple bound notifications
- **THEN** the runtime emits one warning for that attempt rather than one warning per notification

#### Scenario: No execution failure occurred
- **WHEN** the result is successfully null or empty, a cache hit occurs, scheduling is debounced or saturated, an obsolete result is discarded, or normal shutdown cancels work
- **THEN** no execution-failure warning is emitted

#### Scenario: Failed output contains sensitive text
- **WHEN** malformed output or a process error contains title text or raw executable output
- **THEN** the warning contains only the categorized failure and approved metadata, with none of that text
