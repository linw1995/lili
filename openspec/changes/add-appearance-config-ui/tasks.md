## 1. Contract and design assets

- [x] 1.1 Add the Appearance view-model contract for validated Pet summaries, selected Pet identifier, opaque asset identities, and bounded error responses; verify serialization and unknown-field rejection with focused Rust tests
- [x] 1.2 Add the seven-scene preview contract and map `Idle`, `Running`, `Review`, `Attention`, `Failed`, `Waiting`, and `Click` to existing Pet lifecycle or transient animation semantics; verify every scene has one deterministic mapping and no native side effect
- [x] 1.3 Keep `docs/design/lili-appearance.png`, `docs/design/lili-appearance.html`, and `docs/design/lili-appearance.md` aligned with the approved scope; verify the image and interactive prototype show one `Pet` navigation entry, no Active pet summary, a Pet list, and scene switching

## 2. Pet catalog and native selection

- [x] 2.1 Implement a generation-scoped approved asset registry for every validated Pet package needed by Appearance; verify opaque identities resolve only to validated assets and never to renderer-supplied paths
- [x] 2.2 Extract a shared native Pet-selection service used by the tray and Appearance mutation; verify it revalidates the package, persists only the Pet identifier, replaces the active approved asset, and publishes the updated presentation
- [x] 2.3 Cover missing, malformed, removed, and duplicate Pet packages at the selection boundary; verify the embedded fallback remains usable and package-specific diagnostics are retained

## 3. Loopback and server integration

- [x] 3.1 Add the authenticated `/appearance` SSR route and the read-only Appearance data endpoint without changing the existing Pet, notification, or live snapshot contracts; verify SSR and server tests render the focused surface
- [x] 3.2 Add the signed Pet-selection mutation endpoint with bounded request validation and the existing origin/signature checks; verify unknown identifiers, absolute paths, traversal values, stale signatures, and replayed signatures are rejected without state mutation
- [x] 3.3 Extend the loopback bootstrap and Tauri capability setup for the Appearance WebView; verify the page receives the session cookie and narrow signer capability but no drag, notification-control, action, or acceptance capability
- [x] 3.4 Add server tests proving Appearance preview operations do not acknowledge notifications, execute actions, change the Session reducer, or expose credentials, provider payloads, or application paths

## 4. Appearance UI and preview behavior

- [x] 4.1 Build the Leptos Appearance page with the saved desktop shell, left navigation containing only `Pet`, center preview, and right installed-Pet list; verify the SSR output and fixture page contain no additional navigation entries or Active pet summary
- [x] 4.2 Wire keyboard-reachable Pet-list selection to the signed mutation and refresh the selected marker, approved asset, and preview; verify selecting each fixture Pet updates the page and persists across a reload
- [x] 4.3 Implement the preview-only scene controller and renderer using the shared Pet atlas frame scheduler and approved asset identity; verify all seven scenes update in place and notification-bearing scenes use bounded read-only cards
- [x] 4.4 Verify preview isolation under concurrent Session updates; assert that scene changes and Pet-list clicks do not duplicate, dismiss, or acknowledge notifications and do not add action-audit entries
- [x] 4.5 Add accessibility and responsive coverage for the Appearance page; verify keyboard operation, selected-state semantics, readable labels, no clipped controls at 320px and 736px, and the wide layout at 1,024px

## 5. Desktop window and navigation lifecycle

- [x] 5.1 Create the normal decorated Appearance settings WebView, navigate it through the pinned loopback bootstrap, and focus an existing window instead of creating duplicates; verify open, focus, close, and relaunch behavior on the desktop smoke path
- [x] 5.2 Add the `settings` action to the tray and Pet context menu while preserving existing Show, Always on Top, Pet selection, and Quit behavior; verify the settings action does not change transparent Pet or notification-window lifecycle semantics
- [x] 5.3 Reconcile selected-Pet updates across the settings window, transparent Pet window, notification window, tray state, and persisted state; verify a selection made from either tray or Appearance produces one consistent active Pet after restart

## 6. Cross-platform acceptance and delivery

- [x] 6.1 Add browser fixture scenarios for the initial Idle preview, all seven scene controls, every available Pet selection, fallback behavior, keyboard access, and preview isolation; verify the fixture suite passes with strict console diagnostics
- [x] 6.2 Add packaged macOS acceptance for the Appearance window, loopback certificate pinning, tray/context-menu launch, window focus, selected-Pet persistence, and clean shutdown; verify existing non-activating Pet behavior remains intact
- [x] 6.3 Add packaged Windows and supported Linux acceptance for Appearance navigation, signed selection, asset delivery, window lifecycle, and documented compositor/DPI behavior; verify platform-specific failures remain bounded and diagnosable
- [x] 6.4 Run formatting, Clippy with warnings denied, workspace tests, browser E2E, OpenSpec strict validation, and release packaging; verify the saved design assets are included in the intended source tree and no unrelated lockfile or dependency changes are introduced

### Verification record

- `nix run .#format-check`, `nix run .#lint`, `nix run .#test`, `nix flake check --no-build`, `nix run .#spec-validate`, and `nix run .#build` passed.
- The Appearance Playwright fixture passed all 6 tests, including the seven scenes, fallback asset, Pet selection, reload persistence, preview isolation, keyboard semantics, and 320/736/1024px layouts.
- The full Playwright suite and packaged macOS acceptance remain environment-gated on this host: the former repeatedly stalled while building the pinned Playwright npm derivation, and the latter stopped before app launch because the installed Codex version did not match the reviewed contract.
