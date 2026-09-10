## Context

The baseline is commit `8bbac24964cd3f3e3ba4a71c29396968b1442fb6`. Experiments use an archived copy, leaving application source on the working branch unchanged. See the experiment report and machine-readable results under `experiments/session-title-ablation/`.

## Goals / Non-Goals

Goals:

- Remove duplicated information and redundant state encodings.
- Preserve observable behavior and explicit resource/lifecycle guarantees.
- Base deletion decisions on both experiments and ownership invariants.

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

### 3. Defer action collection consolidation

A configuration-ordered `Vec<Arc<ActionRuntime>>` can replace both the action map and `title_order`. Title selection walks configuration order; interaction matching explicitly sorts action IDs to preserve its prior order. Action lookup becomes a bounded linear scan of at most 128 entries.

Adoption decision: retain the existing map and title-order index for this implementation. The candidate removes one maintained index but does not reduce formatted production line count in the measured patch. Treat it as a state-ownership trade-off, not a demonstrated performance improvement. Implement separately from the first two changes and retain it only if review accepts the bounded lookup trade-off. Do not add a new collection dependency to obtain ordered lookup.

### 4. Preserve distinct asynchronous lifetimes

The supervisor tracks process requests, coalesced results, cache entries, and child cleanup. The application tracks notification consumers, persistence, and presentation publication. A process can finish before its notification update finishes, and one process can serve multiple consumers.

The ablation removing publication joins fails the shutdown/publication regression. Removing request coalescing or incarnation checks also changes required behavior. These structures are not duplicate copies of the same state; do not collapse them merely because both are named `pending`.

### 5. Keep early filtering unless a measured benefit justifies its removal

Removing the application's title-action prefilter still avoids native process execution, but creates a transient notification task for unmatched sources. The resource probe catches this extra scheduling. This is not evidence of an externally visible title failure; it is a resource trade-off with little source-size benefit. Retain the guard for this proposal.

## Larger Simplification Candidates Not Yet Approved

Production currently loads actions during initialization, while tests and the public application-state API exercise configuration replacement. Freezing configuration at construction could remove some generation checks and lifecycle coordination, but would narrow an existing internal contract. It needs a separate decision and experiment covering initialization, restored notifications, shutdown, and all callers. It is not an equivalent ablation and is not part of the implementation tasks below.

The `ExecutionInput` trait currently serves two concrete payload types and keeps process supervision shared. No experiment in this proposal replaces it. Likewise, no evidence here supports removing the lifecycle serialization mutex, request tokens, or persistence rollback.

## Validation and Limits

Promote the supplemental behavior probes before applying representation changes. Run each change independently and then together. Keep negative controls as experiment artifacts, not production patches. Re-run the full workspace suite and all-feature lint after integration; run focused packaged notification acceptance once on the final combined implementation.

The recorded experiments cover three library test suites on arm64 macOS. They do not establish a performance gain, cover other operating systems, or exhaustively explore thread interleavings. Compilation and test elapsed times are execution metadata, not benchmark results.
