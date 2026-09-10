## Adopted Scope

Implemented the smaller `(provider, session_id)` key and single-option watch reply. The selected action ID is now borrowed for lookup rather than allocated into a temporary string. Configuration-instance isolation, LRU bounds, debounce, coalescing, notification binding, and lifecycle protections remain unchanged.

The action map and title-order index remain in place. Their consolidation was experimentally viable but offered no production line-count reduction and introduced different lookup/sorting costs without a measured benefit.

## Observed Verification

Implementation commit: `460d399`.

- Promoted all four supplemental probes into regular library tests.
- The three scoped library suites passed 183 tests; scoped all-target Clippy passed.
- The full workspace suite with all targets and acceptance support passed 445 tests.
- Workspace all-target, all-feature Clippy passed with warnings denied.
- Formatting and strict OpenSpec validation passed.
- The macOS release bundle and acceptance executable built successfully.
- Focused packaged notification acceptance passed with the rendered-fallback handshake and strict result-file check. Its audit contained one successful title query, the successful existing notification activation action, and the expected activation timeout. Plain-text rendering and notification dismissal behaved as expected.
- Persistence, configuration replacement, cancellation/reaping, and shutdown/publication regressions passed in the workspace suite.

Only local validation was performed for this adoption. No remote operation, CI rerun, or live user configuration change was performed. Other operating systems were not exercised locally.
