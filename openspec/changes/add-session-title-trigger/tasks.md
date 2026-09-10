## 1. Configuration and Protocol

- [x] 1.1 Separate configured triggers from interaction-only triggers and add `session_title` loading, provider-only filter validation, deterministic first-match selection, and effective-configuration diagnostics.
- [x] 1.2 Add dedicated version 1 request and response types with required fields, strict output decoding, bounded UTF-8 handling, null/empty handling, and title normalization.
- [x] 1.3 Verify existing configuration defaults, serialized interaction contexts, interaction endpoint validation, and click completion effects remain unchanged.

## 2. Execution and Scheduling

- [x] 2.1 Refactor shared process primitives to accept bounded query input while preserving direct argv, minimal environment, timeout, output bounds, process-tree cleanup, and global accounting.
- [ ] 2.2 Add native scheduling after unread-notification publication and restoration, deterministic selection, per-key debounce, in-flight coalescing, and bounded queue/reject behavior.
- [x] 2.3 Implement the 256-entry success-only LRU cache with hit promotion, capacity eviction, configuration invalidation, and no TTL; keep null/empty results and failures uncached and implement shutdown cancellation.
- [ ] 2.4 Bind lookup consumers to notification IDs and guard application with unread state, Session identity, latest-request tokens, selected action, and configuration generation; discard obsolete results.

## 3. Presentation and Diagnostics

- [ ] 3.1 Directly update optional notification titles and retain them through existing notification persistence with backward-compatible missing-field defaults; cache eviction, empty results, and failures must preserve applied titles, summaries, ordering, lifecycle, acknowledgement, and immutable interaction snapshots.
- [ ] 3.2 Render title text safely across native and browser surfaces and publish only meaningful visible changes.
- [ ] 3.3 Extend bounded diagnostics for query request identity and outcomes without raw streams or title content; keep query completion separate from click feedback.

- [ ] 3.4 Emit structured runtime `warn` events once per failed execution attempt with action/request identity, trigger, categorized failure, duration, and available exit code; exclude raw output, title text, request bodies, environment values, and unfiltered error strings.

## 4. Verification and Documentation

- [ ] 4.1 Add synthetic executable tests for valid/null output, normalization, malformed output, unknown fields, UTF-8 and output limits, nonzero exit, spawn failure, timeout, and process cleanup.
- [ ] 4.2 Add deterministic scheduling tests for matching priority, coalescing, independent Session debounce, saturation, queue bounds, LRU hit promotion/eviction and configuration invalidation, title persistence/restoration, and shutdown.
- [ ] 4.3 Add race tests for primary-Session changes, dismissal, notification removal/recreation, configuration generation changes, and out-of-order results; verify interaction regression behavior.
- [ ] 4.4 Document the new trigger, exact stdin/stdout schemas, defaults, matching/filter rules, capacity-only cache policy and title retention, failure diagnostics and runtime warning fields, compatibility, and additive configuration/rollback using a synthetic example.
- [ ] 4.5 Run repository-toolchain formatting, lint, targeted unit and presentation tests, and packaged desktop acceptance demonstrating immediate fallback, asynchronous title update, safe rendering, and unchanged click behavior.
- [ ] 4.6 Run `openspec validate add-session-title-trigger --strict` and record only verification actually performed.

- [ ] 4.7 Capture runtime logs in tests to verify warning level and fields for each execution/protocol failure, one warning for coalesced consumers, exclusion of raw content, and no execution-failure warnings for successful empty results or normal scheduling/cancellation outcomes.
