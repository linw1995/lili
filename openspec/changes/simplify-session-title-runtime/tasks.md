## 1. Preserve the Baseline and Evidence

- [x] 1.1 Promote the supplemental probes for coalesced null completion, per-Session failure debounce, provider cache separation, and unmatched-source scheduling into regular tests.
- [x] 1.2 Confirm the implementation baseline and preserve current configuration, interaction ordering, persistence, cancellation/reaping, and shutdown/publication regressions.

## 2. Remove Redundant Query Representations

- [x] 2.1 Reduce title cache, debounce, and pending keys to `(provider, session_id)` while retaining immutable supervisor ownership and configuration identity checks; verify same-ID cross-provider isolation and replacement behavior.
- [x] 2.2 Replace nested optional watch replies with one optional title and wait for channel updates; verify coalesced null results, closed channels, cancellation, and successful cache reuse.
- [x] 2.3 Run both changes together against the scoped suites and lint, then commit the behavior-preserving simplification separately.

## 3. Action Storage Decision

- [x] 3.1 Review the single-collection trade-off: retain the current map and title-order index in this change. The experimental replacement has no production line-count benefit and adds linear lookup and per-interaction sorting. Reconsider only with a separate measured or ownership benefit.

## 4. Final Verification

- [x] 4.1 Re-run the full workspace tests, all-feature Clippy, formatting, and strict OpenSpec validation after applying the selected changes.
- [x] 4.2 Run focused packaged notification acceptance on the final implementation, including fallback-first display, plain-text title update, and click behavior; retain the persistence and shutdown regressions in the workspace suite.
- [x] 4.3 Record actual results and remaining limitations; do not delete the negative-control protections or configuration lifecycle safeguards as part of this refactor.
