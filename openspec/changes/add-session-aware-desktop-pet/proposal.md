## Why

Long-running ChatGPT/Codex sessions can finish or block while the main application is not visible, and the current repository has no ambient way to represent that state or expose safe, user-defined reactions. A lightweight desktop pet can make session state continuously legible while reusing the existing Codex pet ecosystem instead of introducing a proprietary asset format.

## What Changes

- Add a transparent, always-available Tauri desktop pet shell based on the Rust, Axum, Leptos SSR, and signed loopback reference architecture provided by linw1995.
- Load and validate Codex v2 pet packages, render all standard animation rows, and use the 16 look-direction cells for pointer tracking.
- Add a provider-neutral session event model plus Codex `notify` and lifecycle-hook adapters for session start, active work, required attention, completion, failure, and session end.
- Add local authenticated delivery, deduplication, bounded offline spooling, and deterministic aggregation for concurrent sessions.
- Map normalized session state and direct pet interaction to animations, notification cards, drag behavior, and reduced-motion behavior.
- Add declarative interaction actions so events such as clicking a session notification can invoke a configured local executable with bounded, structured context.
- Add an integration installer that previews and safely updates Codex notification/hook configuration without reading credentials or internal session databases.
- Make Nix Flake the mandatory authority for pinned development tools, supported platform shells, build/test entry points, and release-version consistency.

## Capabilities

### New Capabilities

- `pet-package-compatibility`: Discovery, validation, selection, and rendering of Codex v2 pet packages and animation geometry.
- `session-event-integration`: Supported ChatGPT/Codex event ingestion, normalization, delivery, recovery, deduplication, and concurrent-session aggregation.
- `desktop-pet-behavior`: Desktop window lifecycle, pet animation state, pointer gaze, dragging, notifications, accessibility, and failure presentation.
- `interaction-actions`: Safe declarative mapping from pet or session-notification interactions to local executable invocations.
- `reproducible-toolchain`: Flake-pinned toolchain versions, stable developer commands, lockfile policy, cross-platform evaluation, and single-source application versioning.

### Modified Capabilities

None.

## Impact

- Introduces a new Rust workspace with domain, session integration, application state, Axum server, Leptos UI, Web entry point, Tauri desktop, and hook-forwarder boundaries.
- Adds Tauri 2, Leptos, Axum, Tokio, serde, image metadata decoding, platform-local IPC, and cross-platform process-control dependencies.
- Adds a modular `flake.nix`/`nix/` configuration, committed `flake.lock`, `.envrc`, pinned Rust/Node/Tauri/WASM tools, and on-demand end-to-end test dependencies.
- Adds owner-only application data for configuration, cached pet metadata, a bounded event spool, window placement, and execution audit records.
- Integrates with user-level Codex `config.toml` and hook configuration only through an explicit preview-and-apply workflow.
- Requires packaged acceptance coverage on macOS and Windows, with Linux CLI compatibility where the desktop shell is supported.
