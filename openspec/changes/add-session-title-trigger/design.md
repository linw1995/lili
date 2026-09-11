## Context

The action configuration currently maps three interaction triggers to executable argv arrays. Execution already provides bounded input/output, minimal environment, timeouts, and process-tree cleanup. Interaction contexts and completion effects are tied to user gestures. Title lookup needs its own request and result semantics while reusing the process primitives.

## Goals / Non-Goals

Goals:

- Provide an optional `session_title` trigger with a small, versioned protocol.
- Keep notification delivery independent of executable latency and failures.
- Apply results only to the bound notifications and configuration generation that requested them.
- Preserve existing action configuration and interaction behavior.

Non-goals:

- Arbitrary notification transformations or additional query triggers.
- Changing notification summaries, lifecycle state, ordering, or acknowledgement.
- Persistent title history, background polling, or a configuration editor.

## Decisions

### 1. Extend configuration without changing interaction payloads

Accept `session_title` in `[[action]]` using the existing `command`, `timeout_ms`, working-directory, environment, and concurrency fields. Keep configuration version 1 and current defaults. Recommend a short explicit timeout in examples:

```toml
version = 1

[[action]]
id = "custom-session-title"
trigger = "session_title"
command = ["/absolute/path/to/title-action"]
timeout_ms = 1000
```

For this trigger, support only `filters.providers`; empty means all sources. Reject nonempty `notification_kinds` or `project_labels` with an entry-local diagnostic, because selection must stay stable for a Session. Among enabled matching entries, the first entry in file order wins. A failure does not cascade to another action.

Represent configured triggers separately from user interaction triggers. Keep `InteractionContextV1` and click routing unchanged; neither the interaction endpoint nor pet gestures may invoke `session_title`. Existing binaries may diagnose the new trigger as unsupported; existing entries retain their current behavior.

### 2. Define a dedicated query protocol

Write one UTF-8 JSON document to stdin and close stdin. Input is capped at 16 KiB and contains only:

```json
{
  "version": 1,
  "requestId": "00000000-0000-0000-0000-000000000000",
  "trigger": "session_title",
  "provider": "example",
  "sessionId": "session-123"
}
```

`requestId` identifies this execution. Session identity is the exact `(provider, sessionId)` pair, never the currently primary Session. The executable returns one JSON document on stdout:

```json
{"version": 1, "title": "Review changes"}
```

Require exit status zero, version 1, and a required `title` field containing a string or null. Reject unknown fields, malformed UTF-8/JSON, trailing non-whitespace data, and output overflow. `null` is a successful lookup with no title. Whitespace-only strings also mean no title. Normalize whitespace to single spaces, remove remaining control characters, and truncate to 256 Unicode scalar values without splitting UTF-8. Treat the result as plain text, never markup.

Reuse the existing 16 KiB output capture bound and timeout/process cleanup behavior. Stderr is diagnostic output and never a title source. Audit records contain outcome, request identity, and byte counts, not title text or captured streams.

### 3. Schedule natively after notification publication

After accepting an event that creates or updates an unread notification, publish the existing presentation first and then schedule lookup. On startup, apply the same scheduling to restored unread notifications. No matching action means no process and unchanged presentation.

Use `(provider, sessionId, actionId, configurationGeneration)` as the request/cache key. Coalesce concurrent requests for the same key. Use per-key debounce with the existing configured interval; activity in one Session must not debounce another. Share the global process limit with interaction actions and honor the selected action's bounded reject/queue policy. Saturation or debounce preserves fallback and does not start an unbounded retry loop; later eligible events may retry.

Use an in-memory LRU cache capped at 256 successful, nonempty title entries, with no TTL or expiry timer. A hit promotes the entry to most recently used and directly updates the requesting notification without spawning a process. Insertion beyond capacity evicts the least recently used entry. Do not cache null/empty results or execution/protocol failures; later eligible events may retry under the existing debounce and concurrency limits. Configuration changes clear the cache; restart starts with an empty cache. Cache eviction affects future lookup reuse only and never clears a notification title already applied.

Do not hold application state locks while spawning, waiting, or reading output. Shutdown cancels outstanding title work and reaps children through the shared supervisor.

### 4. Update bound notification titles directly

Bind each lookup consumer to its immutable notification ID and a latest-request token. Coalesced requests retain a bounded set of consumers drawn from existing unread notifications. Before applying either a cached or asynchronous result, require the notification to remain unread, its Session identity to match, and the selected action, configuration generation, and request token to remain current. Removed or recreated notifications cannot accept an old result. Request tracking is independent of LRU eviction.

A valid nonempty result directly updates each still-valid bound notification's title field. It must not create or reopen a notification, dismiss it, reorder it, alter its summary, or change Session lifecycle. Publish a new presentation revision only when the visible title changes. Do not broadcast results to notifications that were never bound to the request.

Until a title is available, use the existing project label and generic fallback. Null/empty output or failure preserves the notification's current title. Once updated, the title belongs to the notification and remains until that notification is removed or a newer valid result replaces it; cache eviction or invalidation cannot revert it. Retain this optional field through the existing notification persistence path, with a missing field default for previously stored notifications. Keep normalized event data and immutable interaction snapshots unchanged.

### 5. Keep failure reporting separate from click feedback

Record lookup failures in bounded native diagnostics. No lookup outcome emits click-action feedback or invokes the interaction success path. Reuse low-level process execution and accounting, with a dedicated query dispatcher and result decoder.

In addition to diagnostics, emit one structured `warn` event per failed execution attempt through the existing runtime logger. Include `action_id`, `request_id`, `trigger`, a stable `failure_kind`, `duration_ms`, and `exit_code` when available. Cover spawn failures, process I/O failures, timeouts, nonzero exits, output overflow, and invalid response protocols. Use categorized failure reasons rather than raw error strings that may embed executable output. Never log title text, raw stdout/stderr, request bodies, or environment values. Coalesced consumers share one execution warning; do not log once per notification. Successful null/empty results, cache hits, debounce, saturation, stale-result rejection, and normal shutdown cancellation are not execution failures and must not emit these warnings. This adds no new log sink or logging configuration.

## Risks / Trade-offs

- Automatic execution can produce bursts across Sessions. Shared process limits, bounded queues, per-key coalescing, and cache limits bound the work.
- A frequently accessed cache entry may retain an older title until eviction, configuration invalidation, or restart. This is the explicit trade-off of capacity-only LRU caching; there is no automatic freshness guarantee.
- A query result can race with notification dismissal, removal, or a newer request. Identity and generation checks must run immediately before publication.
- Separating configured triggers from interaction triggers touches shared diagnostics and loading. Regression coverage must preserve existing serialized interaction payloads and completion effects.

## Rollout / Rollback

The feature is disabled unless a matching action is configured. Document additive configuration and restart requirements. Removing the entry and restarting prevents future title work. Titles already stored on retained notifications remain; new notifications use existing labels. Previously stored notifications without a title field remain readable. Use synthetic executables for verification.

## Validation Strategy

Cover configuration compatibility, exact protocol decoding, Unicode handling, direct argv, output bounds, timeout cleanup, deterministic selection, per-key debounce, queue/reject behavior, LRU promotion/eviction, notification title retention, and stale-result races. Verify UI text escaping and unchanged click behavior. Run targeted checks through the repository toolchain, then packaged desktop acceptance with a deterministic executable and strict OpenSpec validation.
