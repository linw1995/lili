## Context

The title runtime uses immutable supervisor ownership and watch channel versions to avoid storing redundant selection and completion state.

## Goals / Non-Goals

Goals:

- Remove duplicated information and redundant state encodings.
- Preserve observable behavior and explicit resource/lifecycle guarantees.
- Preserve ownership invariants and regression contracts.

Non-goals:

- Removing the user-selected bounded LRU policy.
- Merging notification and process lifetimes into one undifferentiated task.
- Dropping configuration replacement, cancellation, persistence, or audit guarantees.
- Changing public trigger or JSON contracts.
- Treating a passing test suite as proof of concurrency correctness or a speedup.

## Decisions

### 1. Remove action identity from the per-supervisor query key

Current keys contain `(action_id, provider, session_id)`. The supervisor owns an immutable action collection, title selection depends only on provider, and a replacement supervisor gets a new cache and cancellation channel. Within one supervisor, provider therefore determines the selected action. Use `(provider, session_id)` without changing cache ownership or `same_configuration` checks.

Preserve provider separation even when textual Session IDs match. Do not apply this simplification if title selection later becomes dependent on notification fields or actions become mutable in place.

### 2. Let watch notifications represent completion

A watch receiver already distinguishes an unread update from its initial value. Use `watch::Receiver<Option<String>>`, wait for `changed()`, then read the result. Publishing `None` still advances the watch version, so a completed empty result does not need an outer `Some` wrapper.

Keep publication before removing the pending entry, and keep all pending receiver clones at their unread version. Channel closure without a result remains a no-title outcome. Tests must cover multiple waiters receiving null, completion before a waiter resumes, cancellation, and invalid output.

### 3. Retain action collection indexing

Retain the action map and separate title-order index. They preserve indexed action lookup and configuration-order title selection without introducing per-interaction sorting or a new collection dependency.

### 4. Preserve distinct asynchronous lifetimes

The supervisor tracks process requests, coalesced results, cache entries, and child cleanup. The application tracks notification consumers, persistence, and presentation publication. A process can finish before its notification update finishes, and one process can serve multiple consumers.

Shutdown must wait for publication, concurrent callers must share requests, and old results must not update recreated notifications. These structures are not duplicate copies of the same state; do not collapse them merely because both are named `pending`.

### 5. Keep early filtering unless a measured benefit justifies its removal

Retain the application prefilter so unmatched sources do not create unnecessary notification tasks.

## Preserved Contracts and Validation

Retain configuration replacement, the shared execution interface, lifecycle serialization, request tokens, and persistence rollback. Verify coalesced null replies, per-Session debounce, provider separation, cancellation/reaping, and notification publication during shutdown through the regular regression suite.

Run the full workspace suite and all-feature lint after integration, followed by focused packaged notification acceptance. Passing tests do not establish a performance gain or exhaustively cover thread interleavings.

## Visibility Ownership Follow-up

Private parent modules own internal access; public re-exports define the external boundary. The process implementation owns the supervisor child module, and the supervisor owns its title-runtime child. This lets descendants use genuinely private process/supervisor fields without crate-wide field visibility. The crate root explicitly re-exports the existing public API.

The application-state root privately imports title dispatch and scheduling helpers. Internal free functions in private modules do not become public methods on the exported application-state types. Notification representation and its incarnation marker are owned by the reducer; the marker remains private while the crate root preserves the public `Notification` name.
