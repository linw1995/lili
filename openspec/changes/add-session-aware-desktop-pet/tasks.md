## 1. Nix Toolchain, Workspace, and Shared Application Shell

- [x] 1.1 Define version ownership: Cargo workspace `package.version` is the application release version, `flake.lock` pins system/build tools, `Cargo.lock` pins Rust dependencies, and `package-lock.json` pins npm dependencies.
- [x] 1.2 Create a thin root `flake.nix`, committed `flake.lock`, and modular `nix/outputs.nix`, `nix/toolchain.nix`, `nix/apps.nix`, `nix/dev-shells.nix`, and `nix/checks.nix` files with deduplicated Flake inputs; place every imported module in the Git index before the first Git-Flake evaluation.
- [x] 1.3 Pin the Rust release and compose Cargo, rustc, rustfmt, Clippy, rust-analyzer, rust-src, LLVM tools, `wasm32-unknown-unknown`, and the required Darwin cross target.
- [x] 1.4 Pin Node.js 24, Tauri CLI, Binaryen, Trunk, wasm-bindgen CLI, formatting, linting, and repository-hook tools; assert wasm-bindgen CLI compatibility with the Rust dependency.
- [x] 1.5 Implement the Darwin `mkShellNoCC` environment using host `xcrun`, SDK, clang, linker, and Swift library paths, plus Linux GTK/WebKit/pkg-config build inputs.
- [x] 1.6 Expose stable Flake apps for `dev`, `dev-web`, `build`, `build-app`, `build-css`, `format`, `lint`, `prek`, and `e2e`, with Trunk offline mode and normalized color handling.
- [x] 1.7 Keep the default development shell lightweight, move Playwright/browser dependencies to the E2E app or shell, and add `.envrc` with `use flake`.
- [x] 1.8 Add Flake checks for clean lockfiles, canonical version propagation, expected tool versions, required output names, and evaluation of `aarch64-darwin`, `aarch64-linux`, and `x86_64-linux`.
- [x] 1.9 Document the selective Flake input update procedure and add a test proving normal development and build commands do not rewrite any lockfile.
- [x] 1.10 Create the Rust workspace with `lili-core`, `lili-pet`, `lili-session`, `lili-actions`, `lili-app-state`, `lili-server`, `lili-ui`, `lili-web`, and `lili` members, pinned shared dependencies, formatting, lint, and test configuration.
- [x] 1.11 Add the Leptos SSR shell, hydration entry, static asset build, and fixture-only Web entry point with a deterministic health and SSR marker test.
- [x] 1.12 Implement the single Axum router with snapshot, event-stream, approved pet-asset, settings, interaction, and diagnostics routes plus strict body limits and security headers.
- [x] 1.13 Port linw1995's ephemeral HTTPS loopback, certificate pinning, signed mutating-request transport, replay protection, and narrow Tauri capability registration under Lili names.
- [x] 1.14 Add a frameless transparent Tauri window, tray lifecycle, packaged Web assets, clean native shutdown, and a desktop smoke-verification entry point driven by the Flake build applications.

## 2. Codex V2 Pet Compatibility

- [x] 2.1 Define the v2 manifest, atlas geometry, row frame counts, exact frame durations, direction order, and immutable `PetDefinition` domain types with contract tests.
- [x] 2.2 Implement `CODEX_HOME` resolution and confined package discovery that rejects absolute, traversal, escaping-link, duplicate-ID, and oversized manifest cases.
- [x] 2.3 Implement streaming PNG/WebP metadata and transparency validation for exact `1536x2288` atlases without decoding unbounded images into memory.
- [x] 2.4 Add an embedded known-good v2 fallback package and selection persistence by identifier with startup revalidation and package-specific diagnostics.
- [x] 2.5 Serve only approved atlas assets through opaque server identities with MIME, cache, CSP, and package-change invalidation tests.
- [x] 2.6 Implement the renderer frame scheduler for all standard rows and the screen-coordinate 16-direction lookup with deadzone, timer, and wraparound tests.
- [x] 2.7 Add golden compatibility tests against valid custom packages plus wrong-version, wrong-dimension, malformed-image, opaque-background, and escaping-path fixtures.
- [x] 2.8 Wire the approved active atlas into the SSR and hydrated shell with the exact six-frame idle loop so `nix run .#dev` visibly exercises the compatibility layer.
- [x] 2.9 Move opaque atlas delivery outside the signed API namespace so authenticated native image requests work in the desktop WebView, and make desktop smoke wait for decoded atlas dimensions.
- [x] 2.10 Mark only the pet sprite as the native window-movement hit region and grant the pet window the narrow movement capability, leaving drag animation and placement persistence to task 7.4.

## 3. Normalized Session Domain and Reducer

- [x] 3.1 Define versioned provider input, normalized session event, session/turn/event identity, display-safe project context, source capability, notification, and view snapshot types.
- [x] 3.2 Implement payload normalization with additive-field tolerance, required-field validation, summary redaction and truncation, stable fallback event identity, and raw-payload exclusion from errors.
- [x] 3.3 Implement the serialized session reducer with per-turn monotonic terminal states, new-turn generations, deduplication, attention resolution, notification acknowledgement, and snapshot revisions.
- [x] 3.4 Implement deterministic concurrent-session presentation priority and minimum animation dwell without coupling notification ordering to the currently displayed session.
- [x] 3.5 Add table-driven and property tests for duplicated, reordered, stale, concurrent, missing-field, and new-provider-field event sequences.
- [x] 3.6 Add bounded persistence for selected pet, window placement, unread normalized notifications, and reducer metadata with version rejection and atomic replacement tests.

## 4. Private Event Delivery and Offline Recovery

- [x] 4.1 Define the bounded authenticated forwarding protocol, rotating instance credentials, nonce replay window, acknowledgement semantics, and platform endpoint abstraction.
- [x] 4.2 Implement owner-only Unix domain socket delivery with peer checks on macOS/Linux and named-pipe delivery with current-user ACLs on Windows.
- [x] 4.3 Implement the native ingestion actor that verifies messages, reduces accepted events once, publishes snapshots, and exposes aggregate rejection diagnostics.
- [x] 4.4 Implement atomic owner-only offline spooling, claim-by-rename consumption, age/count/byte limits, priority-aware eviction, crash recovery, and aggregate drop metrics.
- [x] 4.5 Implement `lili-hook` modes for JSON argv and JSON stdin with strict input bounds, short connection deadlines, spool fallback, deterministic exit codes, and no approval output.
- [x] 4.6 Add adversarial tests for forged MACs, replayed nonces, wrong peer ownership, partial frames, oversized messages, symlinked spool paths, concurrent forwarders, and interrupted spool writes.
- [x] 4.7 Measure hook forwarding and offline fallback latency and enforce a regression budget that keeps lifecycle hooks independent of renderer responsiveness.

## 5. ChatGPT/Codex Notification Adapters

- [x] 5.1 Add golden fixtures for supported Codex `agent-turn-complete`, `SessionStart`, prompt/activity, `PermissionRequest`, `Stop`, and `SessionEnd` payloads keyed by tested Codex versions.
- [x] 5.2 Implement the `notify` adapter for terminal turn events, including thread, turn, client, working-directory, and bounded assistant-summary normalization when fields are present.
- [x] 5.3 Implement observer-only lifecycle adapters that map available hooks to session start, active, attention, stop, and end events while advertising unsupported distinctions instead of inferring them.
- [x] 5.4 Verify that `PermissionRequest` forwarding returns no decision, never acknowledges a permission, and remains bounded when Lili is stopped, hung, or restarting.
- [x] 5.5 Add adapter compatibility diagnostics showing Codex version, discovered surfaces, missing lifecycle coverage, last accepted event, and safe remediation guidance.
- [x] 5.6 Add end-to-end fixture runs that invoke the packaged hook binary exactly as Codex does and observe one normalized notification in the native reducer.

## 6. Reversible Codex Integration Management

- [x] 6.1 Implement `lili integrate inspect` to resolve the effective Codex home and version, parse relevant configuration without credentials, and report existing Lili and non-Lili integrations.
- [x] 6.2 Implement a deterministic install plan that shows exact `notify` and hook changes, target files, backups, trust requirements, and verification command before mutation.
- [x] 6.3 Implement atomic install with timestamped backups, unique Lili markers, preservation of unrelated configuration, idempotent re-run behavior, and rollback on verification failure.
- [x] 6.4 Detect an existing non-Lili `notify` command as a hard conflict and add an explicit coexistence dispatcher mode that isolates failures and preserves original argv semantics.
- [x] 6.5 Implement provenance-aware uninstall that removes only Lili-owned entries, restores a prior notification command when still safe, and leaves pet packages and unrelated hooks untouched.
- [x] 6.6 Add round-trip tests for empty, commented, reordered, existing-hook, conflicting-notify, repeated-install, modified-after-install, uninstall, and corrupted configuration cases.

## 7. Pet UI, Notifications, and Window Behavior

- [x] 7.1 Build the SSR pet shell and hydrated renderer from `PetPresentationState`, with no raw provider event interpretation in browser code.
- [x] 7.2 Add snapshot-first event-stream reconnection using monotonic revisions, keep-alives, stale-event rejection, and WebView reload tests without duplicate cards.
- [x] 7.3 Implement lifecycle animation mapping, temporary wave and jump overlays, deterministic return to reducer state, and interruption rules for attention and failure.
- [x] 7.4 Implement pointer gaze, hit-region tracking, click versus double-click disambiguation, drag velocity, running-left/right selection, and final window-position commit.
- [x] 7.5 Implement logical display-coordinate persistence, scale conversion, work-area clamping, display disconnect handling, and a reachable visible margin on every platform.
- [x] 7.6 Implement pet-anchored notification cards with priority/recency ordering, immutable session context, bounded safe summaries, independent activation, dismissal, and unread state.
- [x] 7.7 Add tray controls for show, hide, always-on-top, pet selection, integration status, settings, diagnostics, and quit while native event ingestion continues when hidden.
- [x] 7.8 Add reduced-motion rendering, keyboard operation, screen-reader labels, focus behavior, contrast checks, and automated accessibility assertions.
- [x] 7.9 Keep directional drag animation responsive by replacing modal native drag loops with controlled window movement and smoothing sparse pointer samples.
- [x] 7.10 Keep the built-in fallback palette consistent across standard animation and look-direction rows, with deterministic color-drift QA.
- [x] 7.11 Back the macOS pet window with `NSPanel` and move controlled drags to absolute screen-coordinate targets anchored at pointer down.
- [ ] 7.12 Expose the macOS pet as a non-activating unmanaged companion popup so accessibility-driven virtual workspace switches do not hide it.

## 8. Declarative Interaction Actions

- [x] 8.1 Define `actions.toml` v1, supported interaction triggers, event filters, argv commands, working-directory policies, environment allowlist, timeout, debounce, and concurrency modes.
- [x] 8.2 Implement tolerant multi-entry loading that disables invalid or duplicate actions independently and exposes the effective redacted configuration with precise diagnostics.
- [x] 8.3 Define `InteractionContextV1` and bind accepted interactions to immutable notification or pet snapshots before asynchronous dispatch.
- [x] 8.4 Implement direct executable spawning without a shell or event-to-argv interpolation, using a minimal environment and bounded JSON stdin.
- [x] 8.5 Implement global and per-action concurrency, debounce, queue/reject policy, timeout, bounded output capture, and cross-platform process-tree termination.
- [x] 8.6 Implement the bounded privacy-preserving execution audit and action result feedback without mutating session state or automatically dismissing failed notification actions.
- [x] 8.7 Bind notification activation, pet click, and pet double-click to configured actions and verify each accepted interaction dispatches at most once under concurrent session updates.
- [x] 8.8 Add injection, missing executable, nonzero exit, timeout, output flood, environment leak, rapid-click, concurrency saturation, and application-shutdown tests.

## 9. Security, Privacy, and Fault Verification

- [x] 9.1 Add tests proving Web and fixture builds cannot access local IPC credentials, arbitrary pet paths, action execution, Codex configuration mutation, or native process APIs.
- [x] 9.2 Add structured logging and diagnostics that redact raw prompts, full assistant messages, commands, approval arguments, credentials, MAC secrets, and inherited environment values.
- [x] 9.3 Fuzz manifest, provider payload, forwarding frame, spool record, action configuration, and interaction context parsers with bounded-memory assertions.
- [x] 9.4 Add failure-injection tests across config replacement, credential rotation, socket restart, spool claim, reducer acceptance, WebView reload, child spawn, and shutdown phases.
- [x] 9.5 Document the trust boundary, local data layout, retained metadata, integration changes, action authority, backup and uninstall procedure, and unsupported private Codex surfaces.

## 10. Cross-Platform and Release Acceptance

- [x] 10.1 Add browser end-to-end scenarios for package selection, every animation state, look-direction quadrants, concurrent sessions, notification ordering, action results, reload recovery, and reduced motion with strict console diagnostics.
- [x] 10.2 Add packaged macOS acceptance for transparency, always-on-top behavior, certificate pinning, tray lifecycle, logical positioning, Unix socket ownership, hook delivery, action timeout, and clean quit.
- [x] 10.3 Add packaged Windows acceptance for transparency, always-on-top behavior, certificate pinning, tray lifecycle, DPI positioning, named-pipe ACLs, hook delivery, action process-tree cleanup, and clean quit.
- [x] 10.4 Add supported Linux acceptance for Web/Tauri availability, Unix socket ownership, Codex CLI hook delivery, tray behavior, and documented compositor limitations.
- [x] 10.5 Run `nix flake check --no-build`, explicit evaluation for every declared system, formatting, Clippy with warnings denied, workspace tests, dependency audit, release Web build, Tauri builds, fuzz smoke corpus, browser end-to-end tests, and OpenSpec strict validation through Flake-provided commands in CI.
- [x] 10.6 Verify installation against the supported Codex version matrix using generated or captured public schemas and fail release when a required adapter fixture no longer normalizes.
- [x] 10.7 Produce signed or platform-standard release bundles through `nix run .#build` containing the canonical application version, desktop binary, matching hook forwarder, fallback pet, Web assets, license notices, and integration documentation, with no development paths or private test data.
- [x] 10.8 Specify separate native coverage and CRAP Flake commands, report formats, production-only analysis scope, threshold 30 failure semantics, and the prohibition on baselines, allow lists, or optimistic missing coverage.
- [x] 10.9 Preserve and upload the generated CRAP Markdown report when the threshold gate fails.
- [x] 10.10 Specify shared native Flake application environments for clean Linux hosts while keeping non-native commands lightweight.
- [x] 10.11 Reuse one native dependency definition across development shells and every Flake application that compiles or runs the desktop workspace.
- [x] 10.12 Add Flake contract checks that reject native application wrappers missing the shared environment and verify the Linux native dependency closure.
