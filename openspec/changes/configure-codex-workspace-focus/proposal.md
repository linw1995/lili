## Why

Lili already supports bounded user-configured actions for notification activation, but the current example does not make the ownership boundary or workspace-routing use case explicit. A focused configuration recipe should show how to invoke a user-owned script with the clicked notification context without turning desktop automation into a Lili product capability.

## What Changes

- Add a minimal `notification_activate` action example that invokes an operator-selected executable by absolute path without redundant filters or concurrency overrides.
- Document the existing `InteractionContextV1` stdin payload exactly, including `notification.provider` as the source, the immutable clicked-notification `sessionId`, optional fields, size bound, and direct-argv boundary.
- Document that Session registration, workspace resolution, window discovery, and focus behavior belong entirely to the configured external executable and are not implemented, persisted, or guaranteed by Lili.
- Add stub-based verification that the example loads, inherits the existing action defaults, and receives the clicked notification's immutable source and Session context once.
- Add explicit installation, security review, troubleshooting, and rollback guidance without modifying live hooks, trust, actions, or unrelated configuration.
- Do not add a helper executable, Session registry, workspace resolver, window backend, new payload schema, Lili runtime behavior, or release dependency.

## Capabilities

### New Capabilities

None. This change documents and verifies an existing `interaction-actions` capability.

### Modified Capabilities

None. The existing action configuration, execution, payload, and failure contracts remain unchanged.

## Impact

- Updates example action configuration and operator documentation.
- May add documentation-drift and packaged acceptance fixtures using an inert stub executable.
- Does not change Lili application code, plugin hooks, storage, schemas, action authority, release binaries, or supported platforms.
- Leaves all workspace and desktop automation semantics to a separately installed, user-reviewed executable.
