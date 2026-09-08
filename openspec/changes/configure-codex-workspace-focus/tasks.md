## 1. Existing Contract Documentation

- [x] 1.1 Audit the current `InteractionContextV1` Rust type, serializer, input-size bound, and immutable notification binding, then document the canonical version 1 notification JSON with required and nullable fields; verify the documented fixture deserializes through the existing action context decoder.
- [ ] 1.2 Document that stdin supplies the source as `notification.provider` and the stable Session routing identity as `notification.sessionId`, while cwd, workspace, process, terminal, window, and focus semantics are absent; verify no documentation claims Lili discovers or guarantees those values.
- [ ] 1.3 Document the existing action authority boundary: fixed direct argv, JSON stdin, minimal environment, working-directory policy, timeout, output cap, debounce, concurrency, audit, and failure isolation; verify the security documentation identifies all post-spawn behavior as operator-owned.

## 2. Configuration Recipe

- [ ] 2.1 Add a minimal `notification_activate` example containing only `id`, `trigger`, and an absolute user-executable `command`, with no filter or concurrency blocks; verify the unchanged action parser enables it with empty filters and the stable default timeout, debounce, reject mode, one parallel execution, and zero queue.
- [ ] 2.2 Add copy, path-substitution, restart, effective-configuration inspection, synthetic invocation, real-notification verification, troubleshooting, and action-only rollback instructions; verify every state-changing step is explicitly user-run and preserves unrelated actions, hooks, trust, plugin state, and application data.
- [ ] 2.3 Explain that any Session registry, workspace resolution, application inspection, or desktop focus belongs to the external executable and requires its own documentation and cleanup; verify the recipe neither ships nor implies a supported implementation.

## 3. Boundary Verification

- [ ] 3.1 Add or reuse an inert test executable that records bounded stdin mechanically and performs no application inspection or desktop mutation; verify tests compare its decoded `provider`, `sessionId`, and remaining input with the clicked immutable notification snapshot.
- [ ] 3.2 Verify inherited default debounce/concurrency, rapid activation, concurrent primary-Session changes, action success, nonzero exit, timeout, and spawn failure preserve at-most-once dispatch and leave the underlying notification and Session state unchanged.
- [ ] 3.3 Extend packaged desktop acceptance to load the documented action recipe with the inert stub and verify direct argv plus the canonical stdin payload without asserting any external workspace or window behavior.
- [ ] 3.4 Run formatting, lint, targeted unit tests, packaged acceptance, documentation-drift checks, and `openspec validate configure-codex-workspace-focus --strict`; record only results actually observed.
