## Why

The title runtime contains several overlapping representations. Some encode necessary asynchronous lifetimes; others repeat information already available from an immutable supervisor or a watch channel. Independent ablations should identify which representations can be removed without weakening behavior.

## What Changes

- Reduce each title cache/debounce/in-flight key to provider and Session identity, retaining supervisor-instance isolation.
- Replace nested optional watch replies with a single optional title and use the channel's version notification for completion.
- Defer action collection consolidation: retain the existing indexed lookup and interaction match order.
- Retain debounce, request coalescing, notification incarnation checks, cancellation/reaping, and tracked publication during shutdown.
- Keep runtime configuration replacement support unchanged in this proposal.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. This is an internal representation change. Existing configuration, protocol, presentation, persistence, resource bounds, and shutdown requirements remain in force.

## Impact

- Primary implementation scope: `lili-actions/src/title_runtime.rs` and `lili-actions/src/supervisor.rs`.
- Add targeted regression probes without changing established assertions.
- No new dependency, public protocol, configuration field, persistent schema, or UI behavior.
- Experimental evidence and the reproduction script are recorded under `experiments/session-title-ablation/`; generated patches remain ignored local outputs.
