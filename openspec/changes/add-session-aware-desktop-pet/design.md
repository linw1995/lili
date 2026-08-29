## Context

The repository is empty apart from Git and OpenSpec. Linw1995 provided a reference architecture with Rust domain crates, one Axum router, Leptos SSR with hydrated islands, a normal Web entry point, and a Tauri desktop entry point that serves the same router over an ephemeral signed HTTPS loopback origin. Lili needs that security boundary because session integration and local process execution are native capabilities that must not be exposed directly to WebView JavaScript.

The compatibility target is the installed Codex v2 pet contract: a package under the Lili application data root at `pets/<id>/`, `spriteVersionNumber: 2`, and a transparent `1536x2288` 8x11 atlas with `192x208` cells. The product wording uses ChatGPT/Codex sessions, but the first supported machine-readable inputs are the public Codex `notify` command and lifecycle hooks. Those contracts evolve independently, so provider payloads cannot become the internal model.

## Goals / Non-Goals

**Goals:**

- Keep pet assets interchangeable with existing Codex v2 packages.
- Represent concurrent sessions with a deterministic reducer and a restart-safe state-notification projection.
- Make hook forwarding fast, observer-only, private to the local user, and tolerant of the UI being offline.
- Keep rendering, native session authority, and process execution separated by explicit interfaces.
- Make every configured interaction reproducible and auditable without shell parsing.
- Retain linw1995's shared Router and dual Web/Tauri verification pattern.
- Make Flake-pinned tools and stable `nix run` applications the only supported development and release command surface.

**Non-Goals:**

- Reimplement the ChatGPT or Codex client, start turns, send prompts, or read conversation history.
- Approve or deny permission requests from the pet in the first release.
- Read `auth.json`, Codex state databases, rollout JSONL, process memory, or private Desktop APIs.
- Generate, repair, or install pet artwork; Lili only consumes validated packages.
- Support arbitrary v1 spritesheets or silently coerce nonconforming atlases.
- Provide cloud synchronization, remote action execution, a plugin marketplace, or scripting language.
- Produce fully pure Nix derivations for signed or notarized native bundles in the first release; the Flake pins and drives those builds while platform signing remains an explicit external step.

## Decisions

### Make Nix Flake the toolchain and command-entry authority

Use a thin root `flake.nix` and split maintained logic under `nix/`, initially `outputs.nix`, `toolchain.nix`, `apps.nix`, `dev-shells.nix`, and `checks.nix`. Commit `flake.lock` and `.envrc` with `use flake`. This combines a modular Nix structure with the proven Tauri/WASM shell composition supplied by linw1995.

The Flake inputs are `nixpkgs`, `flake-utils`, and `rust-overlay`, with every nested `nixpkgs` input following the root input. Add the Playwright Flake only for the on-demand E2E closure. The lockfile, not the local channel or registry state, selects revisions.

Pin an explicit Rust stable release rather than floating with the machine toolchain. Compose Cargo, rustc, rustfmt, Clippy, rust-analyzer, rust-src, LLVM tools, `wasm32-unknown-unknown`, and the Darwin cross target used for universal builds. Pin Node.js 24, Tauri CLI, Binaryen, Trunk, and a wasm-bindgen CLI version that exactly matches the Rust dependency. Trunk runs through a wrapper that normalizes `NO_COLOR` and sets offline mode. Cargo and npm dependency graphs remain owned by committed `Cargo.lock` and `package-lock.json`; the Flake does not replace language lockfiles.

Expose stable applications at least for `dev`, `dev-web`, `build`, `build-app`, `build-css`, `format`, `lint`, `prek`, and `e2e`. The default shell contains everyday compile and quality tools. Browser binaries and other large E2E dependencies stay outside it and are realized only through `nix run .#e2e` or a dedicated E2E shell.

Evaluate `aarch64-darwin`, `aarch64-linux`, and `x86_64-linux` explicitly. Darwin uses `mkShellNoCC`, `/usr/bin/xcrun`, the host SDK, and system clang/Swift so Nix does not shadow Xcode with an incompatible SDK. Linux supplies pkg-config and the GTK/WebKit dependencies required by Tauri. Universal macOS output uses the pinned Rust cross target without claiming an `x86_64-darwin` development shell.

The root Cargo workspace `package.version` is the single editable release version. Nix reads it for package metadata, and checks verify that Tauri bundle metadata and the hook-forwarder report the same value. Ordinary commands use `--no-write-lock-file` semantics where available and fail rather than mutate locks. Upgrades update selected Flake inputs deliberately, review the lock diff, print effective tool versions, evaluate all supported systems, and run the stable applications' smoke checks.

Git-backed Flake evaluation excludes completely untracked imported files. During initial implementation or Nix module additions, the new `nix/*.nix` files must enter the Git index before evaluation, for example with intent-to-add, so validation exercises the actual modular Flake rather than reporting a misleading missing-file error.

Alternatives considered:

- `rustup`, `nvm`, and globally installed Tauri/Trunk tools are convenient but allow developer and CI versions to drift independently.
- A monolithic Flake is initially shorter but becomes difficult to review once host shells, E2E closures, packaging, and release checks diverge.
- A fully pure Nix package for the signed desktop bundle would improve hermeticity, but host Xcode, Apple signing/notarization, and platform secrets make it a separate deliverable from toolchain version management.

### Reuse linw1995's layered workspace and one-router/two-entry architecture

Create these workspace members:

- `lili-core`: normalized events, session reducer, notification identities, action contracts, and serializable view snapshots.
- `lili-pet`: package discovery, manifest and image validation, atlas layout, frame timing, and animation selection.
- `lili-session`: provider adapters, local authenticated IPC, offline spool, deduplication, and the ingestion actor.
- `lili-actions`: configuration validation, direct process spawning, limits, and execution audit.
- `lili-app-state`: concrete composition and long-lived native services.
- `lili-server`: the only Axum router constructor, SSR context, snapshot APIs, and event stream.
- `lili-ui`: Leptos SSR shell plus a client-only pet renderer and settings islands.
- `lili-web`: TCP entry point used for browser development and end-to-end tests.
- `lili`: Tauri desktop entry point and packaged `lili-hook` forwarding subcommand or companion binary.

The Web entry uses fixture adapters and cannot execute native actions. The desktop entry runs the real session and action services, packages Web assets, binds an ephemeral HTTPS loopback listener, pins its generated certificate, signs mutating browser requests through a narrow Tauri command, and injects no general shell or filesystem capability into the WebView.

Alternatives considered:

- A Tauri-command-only SPA is smaller, but duplicates transport semantics, weakens SSR/browser acceptance, and pushes native authority closer to page code.
- Copying linw1995's reference implementation wholesale would import storage and editor concerns unrelated to Lili; only the architectural boundaries and build approach are retained.

### Normalize every provider payload before it enters application state

`ProviderEventV1` is a tolerant input envelope. It is converted into a closed `SessionEvent` enum such as `SessionStarted`, `TurnActive`, `AttentionRequired`, `TurnCompleted`, `TurnFailed`, and `SessionEnded`. The normalized envelope carries `EventId`, `SessionKey { provider, session_id }`, optional `turn_id`, timestamps, project context, display-safe summary, and source capability flags.

Codex integrations are split by purpose:

1. The top-level `notify` command supplies the stable `agent-turn-complete` terminal notification.
2. Lifecycle hooks supply session start, prompt/turn activity, permission attention, stop, and session end signals when enabled by the installed Codex version.
3. The normalized forwarding protocol already supports explicit failed or resolved events from future documented adapters without changing reducers or UI contracts.

Unknown additive fields are ignored. Adapter fixtures are pinned to the installed Codex schemas and recorded by product version. A hook that cannot express a lifecycle distinction advertises that limitation rather than fabricating state.

Alternatives considered:

- Reading Codex SQLite or rollout files provides more apparent detail but is private, unstable, and risks leaking conversation content.
- Coupling UI state directly to hook names would make every provider change a rendering migration.

### Keep hook forwarding observer-only and off the UI process

`lili-hook` reads either the one JSON argv used by `notify` or JSON stdin used by lifecycle hooks, validates a strict size bound, normalizes only display-safe fields, sends one message, and exits quickly. Permission hooks return no decision and never wait for user interaction. Hook execution therefore cannot make the pet an approval authority or stall the session behind a WebView round trip.

The forwarder connects to a per-user Unix domain socket on macOS/Linux or named pipe on Windows. At startup, the application creates a random instance secret in an owner-only runtime directory. Each message includes an instance identifier, nonce, and MAC; the server also verifies peer ownership where the platform exposes it. Instance secrets rotate at every app start and accepted nonces have a short replay window.

If the endpoint is unavailable, the forwarder writes an atomic owner-only spool record and exits. The application claims records by rename, validates them again, and removes them only after reducer acceptance. The spool is bounded by age, count, and bytes; eviction uses event priority and age. Raw provider payloads are never spooled.

Alternatives considered:

- Starting the full desktop application synchronously from every hook increases latency and causes launch storms.
- An unauthenticated localhost HTTP port is easier to call but permits cross-user or browser-origin event injection on shared systems.

### Use a serialized reducer for ordering, deduplication, and priority

One native actor owns the session map, event deduplication cache, in-memory notification set, and monotonic snapshot revision. An `EventId` uses a provider-supplied identity when present and otherwise a hash of the normalized source type, session, turn, occurrence time, and stable source discriminator.

Each `(session, turn)` is monotonic: a terminal state cannot return to active, while a different turn identifier starts a new generation. A new snapshot is published only after reduction. The displayed state priority is:

```text
unresolved attention
newest unacknowledged failure
newest unread completion
any active turn
idle
```

Temporary animation overrides never mutate session state. They expire back to the reducer-selected state. Renderer reconnection performs `snapshot -> events after snapshot revision`, preventing a reload gap or duplicate notification.

Alternatives considered:

- Letting the WebView merge events makes reload recovery and concurrency dependent on browser timing.
- The application-owned SQLite store is the authority for the compact latest per-Session projection, plugin evidence, and bounded offline spool. It deliberately does not append event history or persist the in-memory deduplication cache.

### Treat the Codex v2 manifest as an external compatibility boundary

Package loading is two phase: parse and confine manifest paths, then decode metadata and validate exact atlas geometry before exposing an immutable `PetDefinition`. The renderer receives only a server URL for an approved asset plus computed frame descriptors. It never receives an arbitrary filesystem path.

CSS background positioning or a canvas atlas renderer can implement frames, but the frame table is native/domain data and is tested independently. Standard row frame counts and durations are fixed by the contract. Direction selection converts the screen vector to clockwise degrees with up as `000`, rounds to 22.5 degrees, and uses idle inside the deadzone.

Package selection persists only the pet identifier. Missing or invalid packages fall back to an embedded known-good v2 pet and surface diagnostics.

Alternatives considered:

- Repacking every pet on import duplicates the ecosystem and can alter pixel identity.
- Trusting browser image dimensions after load allows malformed assets into persistent selection and produces blank or offset frames.

### Model behavior as reducer state plus temporary interaction overlays

The UI receives `PetPresentationState` rather than interpreting raw sessions. Lifecycle mapping uses `idle`, `waiting`, `running`, `review`, and `failed`. Direct click and double-click overlays use `waving` and `jumping`. Drag direction uses `running-left` or `running-right`. Pointer gaze has precedence only while no temporary or higher-priority lifecycle animation needs to communicate status.

The frameless transparent pet window has a small interactive hit region around the pet. Notification cards render in a separate transparent window anchored to the visible sprite boundary above the pet, with a below-pet fallback near the top work-area edge. This intentionally overlaps the Pet window transparent margin so the visible card-to-sprite gap remains 4 logical pixels. Cards fill from the notification window bottom upward, keeping the newest card closest to the pet and pushing older cards upward regardless of lifecycle priority. Native window coordination keeps the notification surface aligned and above the Pet transparent margin while the pet moves, applies the same visibility and always-on-top policy, and clamps the notification window to the selected display work area. On macOS, both Pet and notification windows use the same non-activating companion `NSPanel` policy so they remain together across Spaces, while only the Pet panel owns the application context-menu action. Every application WebView disables devtools and suppresses the browser context menu before page interaction. A process-local AppKit monitor registers the Pet, notification, and custom-menu window numbers and consumes both `RightMouseDown` and `RightMouseUp`; consuming the release is required because the custom menu can appear under the pointer after the press is consumed, allowing WebKit to otherwise generate a Reload menu on release. The monitor does not change WKWebView class identity. Only a true right-click on the Pet sprite opens the bounded Lili context menu, while right-clicking notification, transparent background, or context-menu content does nothing. This avoids relying on content outside the pet WebView bounds, which native window clipping would discard. Dragging is initiated only from the pet hit region and uses bounded pointer-captured movement rather than a modal platform drag loop so animation rendering continues during movement. The pet position is stored in logical coordinates with display identity and scale, then clamped on display changes.

Reduced motion selects stable representative cells and suppresses looping and gaze animation while retaining state color, labels, cards, and keyboard behavior.

### Execute declarative actions through a native bounded supervisor

`actions.toml` has a schema version and entries with `id`, `trigger`, filters, `command = [executable, args...]`, timeout, debounce, concurrency mode, working directory policy, and explicit environment additions. Event values are never template-expanded into argv. The child receives one `InteractionContextV1` JSON document on stdin.

The supervisor resolves an executable once at configuration load, spawns without a shell, uses a minimal environment, caps input and output, limits global concurrency, and terminates timed-out process trees where platform APIs allow. The clicked notification is cloned into an immutable accepted interaction before dispatch, so concurrent reducer updates cannot retarget the action.

Execution audit is a bounded structured log. It stores action and event identities, timing, result class, exit code, and truncated byte counts; it excludes raw payloads, full environment, and complete conversation text. A failure affects only interaction feedback and never resolves the underlying Codex attention request.

Alternatives considered:

- Shell command strings are convenient but make project names and summaries injection primitives.
- JavaScript-side process plugins expose excessive authority and are difficult to constrain consistently across Web and Tauri builds.

### Make integration installation explicit and reversible

`lili integrate inspect` discovers Codex version and effective configuration, then emits an exact plan. `lili integrate install` applies an accepted plan atomically, preserves formatting where practical, creates a timestamped backup, adds uniquely identifiable Lili hook entries, and verifies delivery with a synthetic event.

An existing non-Lili `notify` command is a hard conflict by default. An optional explicit coexistence mode creates a small stable dispatcher that invokes Lili and the existing command independently; the preview shows both commands and failure policy. Uninstall removes only entries carrying Lili's identifier and restores the prior command when provenance still matches. Hooks remain subject to Codex's own trust prompt.

Alternatives considered:

- Blindly replacing `notify` breaks existing user automation.
- Asking users to hand-edit every file avoids installer risk but makes version checks, rollback, and acceptance testing unreliable.

## Risks / Trade-offs

- [Codex hook payloads and available event types evolve] → Keep versioned adapters, tolerant parsing, capability reporting, and golden fixtures per supported Codex release.
- [Some Codex surfaces expose completion but not full failure or resolution semantics] → Never infer unsupported states; retain provider capability flags and accept richer documented adapters later.
- [A hook can delay an agent session] → Keep the forwarder single-purpose, bounded, nonblocking with short connection deadlines, and spool locally on failure.
- [Transparent always-on-top windows behave differently across platforms] → Isolate window policy by platform and require packaged acceptance on every supported OS.
- [A malicious local process could forge events or replace config] → Use owner-only directories, rotating credentials, peer checks, atomic config reads, and bounded authenticated messages.
- [Configured scripts are intentionally powerful] → Require explicit config, direct argv execution, minimal environment, time and output bounds, visible effective configuration, and an audit trail.
- [Notification text can leak conversation content on screen] → Normalize to display-safe fields, hide raw prompts and commands by default, and bound all summaries.
- [Many concurrent sessions can create animation churn] → Serialize reduction, use priority with minimum dwell times, and keep notifications independent of animation transitions.
- [Flake and language lockfiles can drift as separate version surfaces] → Assign tool/system versions to `flake.lock`, Rust dependencies to `Cargo.lock`, npm dependencies to `package-lock.json`, and enforce clean-lock plus compatibility checks in CI.
- [Nix can inject an incompatible Darwin compiler or SDK] → Use `mkShellNoCC` and explicitly bind the host Xcode SDK/compiler while retaining pinned surrounding tools.
- [The default development shell can become too expensive to realize] → Keep browser/E2E closures in dedicated apps and measure the default shell closure during upgrades.

## Migration Plan

1. Establish the locked Flake, modular Nix outputs, language lockfiles, stable command applications, and clean cross-system evaluation.
2. Establish the workspace, shared Router, fixture Web entry, secure Tauri loopback, transparent pet window, and packaged smoke test through Flake-provided commands.
3. Add the embedded fallback pet, v2 package validator, deterministic animation table, and reduced-motion renderer.
4. Add the normalized event model, reducer, snapshots, fixture event source, and concurrent-session tests.
5. Add authenticated local IPC, bounded spool, `notify` forwarder, lifecycle-hook adapters, and integration diagnostics.
6. Add notification cards, pointer gaze, dragging, persisted placement, tray behavior, and accessibility coverage.
7. Add declarative action configuration, bounded process supervisor, audit, and interaction bindings.
8. Add reversible Codex integration install/uninstall and verify against supported Codex versions.
9. Gate release on clean lockfiles, Flake evaluation, Rust checks, browser end-to-end tests, malformed input tests, and packaged macOS/Windows acceptance.

Rollback first disables or uninstalls the Codex integration, then exits Lili. Removing the application leaves pet packages untouched. Configuration and spool formats are versioned; an incompatible newer file is ignored with a diagnostic rather than rewritten.
