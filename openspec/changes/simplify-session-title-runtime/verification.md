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

## Visibility Follow-up Verification

- Replaced scattered scoped visibility in the action runtime and application-state helpers with private module ownership and parent-level imports/re-exports.
- Moved the supervisor below the process implementation, and the title runtime below the supervisor. Their internal fields, execution trait, audit helper, and process helpers are private.
- The reducer owns the notification incarnation field without exposing it across the crate.
- The three related library suites passed 183 tests, and workspace all-target/all-feature Clippy passed.
- An independent temporary consumer crate compiled against the existing public types and spawn API. Thirteen negative compilation probes were rejected with the expected privacy errors: supervisor fields, process helper methods, notification incarnation, internal scheduling/restoration imports, and the private execution module.
- No public protocol or runtime behavior was changed. No remote operation was performed.
