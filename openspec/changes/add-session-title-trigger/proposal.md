## Why

Session notifications need an optional user-customized title. Existing actions accept interactions but do not define a query result that can update presentation. A dedicated `session_title` trigger provides a bounded request and response contract while keeping notification delivery responsive.

## What Changes

- Add the opt-in `session_title` action trigger to the existing version 1 configuration.
- Define versioned JSON stdin and stdout contracts for title requests and results.
- Select one matching action deterministically and execute it asynchronously through shared process supervision.
- Coalesce requests, bound caching and concurrency, and reject stale results.
- Directly update bound notification titles and retain them independently of the capacity-only LRU cache, with the existing label as fallback.
- Expose bounded diagnostics and structured runtime warnings for execution failures, and document configuration, failure behavior, and compatibility.

## Capabilities

### New Capabilities

- `session-title-actions`: Configuration, execution, scheduling, result validation, and presentation for the `session_title` trigger.

### Modified Capabilities

None. Existing interaction trigger contracts remain unchanged.

## Impact

- `lili-actions`: trigger-aware configuration, request/result types, reusable process supervision, and diagnostics.
- `lili-app-state`: native scheduling, bounded LRU title cache, notification title updates, and request identity checks.
- `lili-core` and `lili-ui`: optional title presentation with stable fallback behavior.
- `lili-server` and desktop entry points: consistent presentation publication and diagnostics.
- Action examples, configuration documentation, unit tests, and packaged desktop acceptance.
- Retained notifications gain an optional title field with backward-compatible decoding; no separate persistent cache or external dependency is planned.
