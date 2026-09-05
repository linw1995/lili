## Why

Lili can currently select a pet from the tray and render the active desktop surface, but it has no dedicated place to inspect available packages or verify how a pet behaves across its supported states. A focused Appearance surface gives users a safe, local preview loop without turning the WebView into a session or process authority.

## What Changes

- Add a dedicated Appearance settings surface using the existing signed loopback and SSR/WebView architecture.
- Keep a left navigation shell with exactly one visible configuration entry: `Pet`.
- Show the discovered, validated Pet packages in a right-side selectable list and persist the selected Pet identifier.
- Add a center preview with explicit scene controls for `Idle`, `Running`, `Review`, `Attention`, `Failed`, `Waiting`, and `Click`.
- Reuse approved Pet v2 assets and existing animation/lifecycle mappings so preview output matches the desktop Pet renderer.
- Keep preview state local to the Appearance surface; it MUST NOT ingest live session events, execute configured actions, acknowledge notifications, or mutate session state.
- Save the approved design draft and interactive prototype under `docs/design/` for implementation and review.

## Capabilities

### New Capabilities

- `appearance-configuration`: Dedicated Pet selection and scene-preview behavior for the Appearance settings surface.

### Modified Capabilities

None.

## Impact

- `lili-ui`: add the Appearance shell, Pet list, scene controls, preview renderer, and accessibility semantics.
- `lili-server`: expose bounded read-only Appearance data and a signed Pet-selection mutation route.
- `lili-app-state` and `lili-pet`: reuse available package summaries, approved assets, selected-identifier persistence, and fallback behavior.
- `lili/src/lib.rs`: create and manage the non-transparent settings WebView, signed navigation, lifecycle, and tray entry point.
- Web fixture and browser acceptance tests: cover Pet selection, every preview scene, fallback behavior, reload, and isolation from live session state.
- `docs/design/lili-appearance.png` and `docs/design/lili-appearance.html`: reviewable visual design and interactive prototype.
- No new third-party dependency is required.
