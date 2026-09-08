## Context

See `proposal.md` for motivation. Lili's existing `interaction-actions` contract already loads `actions.toml`, binds notification activation to an immutable event snapshot, invokes configured argv without a shell, writes one bounded `InteractionContextV1` JSON document to child stdin, and isolates action failures from Session and notification state.

The configured executable runs with the current user's operating-system authority. Lili intentionally does not interpret its behavior after process startup. The action context contains `notification.provider` as the source, the clicked notification's immutable `sessionId`, and a display-safe project label, but not the original cwd, process identity, terminal identity, workspace-manager state, or window identity.

## Goals / Non-Goals

**Goals:**

- Provide one accurate, copyable configuration recipe for invoking a user-owned workspace-focus executable from a Codex notification.
- Keep the documented stdin payload synchronized with the existing `InteractionContextV1` version 1 serializer.
- Verify Lili passes the clicked notification's immutable Session context to an inert stub exactly as the existing action contract promises.
- Make installation, authority, failure, and rollback boundaries explicit.

**Non-Goals:**

- Implementing or packaging a workspace-focus helper.
- Registering Codex Sessions outside the existing Lili event integration.
- Persisting Session-to-workspace mappings in Lili or a new companion registry.
- Resolving cwd, repositories, worktrees, workspaces, processes, terminals, windows, or desktop focus.
- Testing or guaranteeing what a user-owned executable does after Lili starts it.
- Adding hooks, accepting trust, or editing live Lili action configuration automatically.

## Decisions

### Treat the external executable as an opaque action consumer

The example configures one absolute executable and fixed argv. It omits `[action.filters]`, so the executable reads `notification.provider` from stdin and decides whether to handle the source. It also omits `[action.concurrency]` because the schema already defaults to reject mode, one parallel execution, and zero queue. Lili supplies only its existing bounded JSON stdin and does not add routing fields, environment discovery, interpolation, or special behavior for workspace automation. Any additional Session registration, local state, application inspection, or desktop control is owned and reviewed separately by the operator.

Shipping a first-party helper was rejected because it would make its storage, process discovery, compatibility, security, packaging, and support surface part of the Lili project. A generic resolver framework was rejected for the same reason: it recreates application integration inside a project that already provides the correct extension point.

### Document the existing payload rather than define a new protocol

The recipe reproduces the canonical `InteractionContextV1` version 1 notification shape from the current Rust type and serialization tests. It identifies required and nullable fields and the existing 16 KiB bound. The external executable uses `notification.provider` to distinguish the source and may use `notification.sessionId` for routing, but Lili does not promise cwd or another workspace locator.

No `SessionStart`, workspace-resolver, registry, inventory, or focus protocol is defined by this change. If a user-owned executable needs those contracts, they belong to that executable's own project or configuration.

### Verify the Lili boundary with an inert stub

Automated and packaged acceptance uses a test-only executable that records bounded stdin and exits without inspecting applications or changing desktop focus. Assertions cover example loading, inherited defaults, direct argv, immutable clicked `provider` and `sessionId`, single execution under the default debounce/concurrency policy, and unchanged notification state after success or failure.

The tests do not model the external executable's routing algorithm. Doing so would incorrectly imply that Lili owns or supports that behavior.

### Keep installation manual and reversible

Documentation tells the operator to copy and edit the example, replace the executable placeholder with an absolute reviewed path, restart Lili, and inspect the effective action configuration. It does not install an executable, mutate action files, or claim that the executable is safe merely because Lili can invoke it.

Rollback removes only the operator-added action entry and restarts Lili. Removing or cleaning up any state owned by the external executable remains outside Lili's rollback procedure.

## Risks / Trade-offs

- [Users may infer that Lili supports the script's desktop automation] → State repeatedly that Lili guarantees only bounded invocation and immutable stdin context.
- [Documentation drifts from `InteractionContextV1`] → Derive or validate the canonical example against the current serializer in automated tests.
- [The configured executable performs unsafe operations] → Require an explicit absolute path and operator review; retain Lili's direct-argv, minimal-environment, timeout, output, debounce, and concurrency controls.
- [The external executable cannot map `sessionId` to a workspace] → Report this as an external configuration limitation and do not add hidden Lili data sources or fallback behavior.
- [The unfiltered action receives a notification source it does not support] → Require the executable to inspect `notification.provider` and handle unsupported sources according to its own documented policy.
- [A failing action disrupts notification state] → Reuse acceptance coverage proving action outcomes neither acknowledge nor discard the notification.

## Migration Plan

1. Update the example and documentation without changing live configuration.
2. Validate the example with the existing action parser and a test-only inert executable.
3. Run unit and packaged acceptance proving the clicked immutable context reaches the stub and notification state remains unchanged.
4. The operator separately installs and reviews any desired executable, copies the example, substitutes its absolute path, restarts Lili, and inspects effective action diagnostics.
5. To roll back, the operator removes only the copied action entry and restarts Lili; external executable cleanup follows that executable's own procedure.
