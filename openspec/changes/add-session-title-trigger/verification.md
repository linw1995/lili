## Verification

Completed on 2026-09-10 on arm64 macOS through the repository Nix toolchain.

- Workspace Clippy with all targets and all features passed with warnings denied.
- Workspace tests with all targets and acceptance support passed: 438 tests, zero failures.
- Formatting and strict OpenSpec validation passed.
- The release application bundle and acceptance executable built successfully.
- Focused packaged notification-action acceptance passed using an isolated application data directory and a synthetic title executable. The hydrated notification first showed fallback text, then the returned title without changing its identity. Markup-like title text remained plain text. The execution audit recorded one successful title query, the existing successful activation action, and the expected timed-out activation action.
- Full desktop window-placement and appearance acceptance were outside this focused run. Other operating systems were not exercised locally.

Regression coverage includes configuration isolation, strict response decoding, Unicode bounds, shared process supervision, LRU promotion/eviction, per-key debounce, request coalescing, null/failure cache exclusion, cancellation, notification dismissal/recreation, configuration replacement, notification persistence and rollback, backward-compatible decoding, and structured warning fields and output exclusion.

No live user action configuration was changed. No remote repository operation was performed.

## PR follow-up

- Cancellation now explicitly terminates and awaits the child process before completing title shutdown; the regression checks that Unix `waitpid` reports no remaining child after shutdown returns.
- The related action and application-state suites passed (84 tests), and workspace all-feature Clippy passed.
- The packaged notification-action acceptance was rerun successfully with a strict result-file check in addition to the process exit code. Its notification-state assertion now reflects the successful activation action dismissing the notification.

- CI follow-up added fixture title/identity boundary coverage, replaced timing assumptions with an explicit rendered-fallback handshake, and corrected native acceptance assertions to run on the main thread and await asynchronous position/visibility changes. macOS floating-level checks compare the actual AppKit level rather than a lookup key.
- The full local packaged desktop acceptance now passes every native assertion, including window position and Appearance close/reopen, with the strict result-file check. Final workspace regression passed 440 tests; all-feature Clippy passed.
- A second review follow-up keeps title tasks tracked through publication. Shutdown closes admission, cancels query processes, and awaits completion tasks; configuration replacement and shutdown are serialized. The publication-blocking regression and related suites passed (85 tests), with targeted Clippy passing.
