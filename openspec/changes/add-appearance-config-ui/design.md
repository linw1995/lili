## Context

See `proposal.md` for the motivation and scope. The current desktop runtime already has the boundaries needed for this surface:

- `lili-ui` renders the Pet and notification surfaces from native presentation state.
- `lili-server` owns the SSR shell, the authenticated loopback routes, the snapshot endpoint, and the signed mutation boundary.
- `lili-app-state` owns the selected Pet, the validated `PetCatalog`, the active approved asset, and the persisted selected identifier.
- `lili-pet` owns Pet v2 package discovery, atlas validation, frame timing, lifecycle animation mappings, and fallback behavior.
- `lili/src/lib.rs` creates the Tauri windows and tray menu and currently reuses the native Pet selection path.

The saved visual design is intentionally narrower than the existing runtime: a left rail with only `Pet`, a center scene preview, and a right installed-Pet list. The design assets live under `docs/design/` and use embedded fallback frames so review does not require access to application storage.

## Goals / Non-Goals

**Goals:**

- Add a custom-framed desktop settings window that uses the existing signed loopback and SSR/WebView delivery path.
- Make the current validated Pet catalog visible and let users select a Pet by stable identifier.
- Make the seven agreed preview scenes directly inspectable in one Appearance surface.
- Reuse the same approved atlas and animation mappings used by the transparent desktop Pet.
- Keep preview state presentation-only and independent from Session reduction, notification acknowledgement, and action execution.
- Preserve the current selected-Pet persistence and embedded fallback semantics without adding a migration.
- Verify the surface in the browser fixture and in packaged desktop acceptance.

**Non-Goals:**

- Do not add Notifications, Interactions, Connection, or any other settings page in this change.
- Do not add an Active pet summary, window toggles, notification controls, action editors, integration controls, or package authoring/import UI.
- Do not allow the preview to emit real Session events, invoke configured executables, acknowledge notifications, or move the transparent Pet window.
- Do not expose application paths, loopback credentials, raw provider payloads, or action configuration to the renderer.
- Do not change the Pet v2 package format or replace the existing tray selection behavior.

## Decisions

### Use a dedicated settings WebView on the existing loopback origin

Add an `appearance` Tauri WebviewWindow with a custom frameless shell: native decorations disabled, a rounded branded frame rendered by the page, an explicit drag region, branded minimize and close controls, resizable behavior, visible task-switcher participation where the platform permits it, and focus only when explicitly opened. Start it at 1180 x 860 px so the three-column desktop layout is not clipped on first open. Navigate it through a third one-shot loopback bootstrap path that establishes the same HttpOnly session cookie and certificate pinning as the existing Pet and notification windows.

The SSR shell will route `/appearance` to a dedicated `AppearancePage` component instead of extending the transparent `AppSurface` renderer. The page will contain the saved left navigation rail, the center preview, and the right Pet list. The tray menu and the Pet context menu will gain one `settings` action that opens or focuses this window; no second transport or local file URL will be introduced.

Alternatives considered:

- A Tauri-command-only settings SPA would duplicate the loopback and fixture behavior and move native authority closer to the page.
- A temporary local HTML file would lose the authenticated asset and API boundary and would not exercise the normal SSR path.
- Reusing the transparent Pet window would make a settings surface difficult to focus, resize, and test and would mix two unrelated interaction models.

### Add a bounded Appearance view model and one signed selection mutation

Add a read-only Appearance view model containing the available Pet summaries, the selected Pet identifier, and opaque approved asset identities needed by the list and preview. Expose it through a dedicated API route so the existing Pet and notification snapshot contracts remain unchanged.

Add one signed mutation that accepts a validated Pet identifier. The native selection path will revalidate the package, persist only the identifier through `AppStateStore`, replace the active approved asset, publish the new Pet presentation, and return the refreshed Appearance view model. The tray and settings window will use the same selection service so selection semantics cannot diverge.

The approved asset registry will be generation-scoped and keyed by opaque identifiers. It may load or cache validated atlas bytes for listed packages, but it will never accept a renderer-supplied path. Revalidation issues a new generation and invalidates prior asset identities. Existing package size and atlas validation limits remain the upper bound for memory and response size.

Alternatives considered:

- Returning only the active asset would make unselected list entries impossible to preview and would require placeholder thumbnails.
- Returning absolute package paths would violate the existing renderer trust boundary.
- Adding every Appearance field to `ViewSnapshot` would couple settings-only data to the live Session stream and enlarge an unrelated contract.

### Reuse the Pet renderer through a preview-only scene model

Create an `AppearancePage` and a focused preview component in `lili-ui`. A local `PreviewScene` value will map the seven controls to the existing lifecycle and transient animation vocabulary:

- `Idle` → idle lifecycle.
- `Running` → the activity-reminder/running lifecycle representation.
- `Review` → review lifecycle plus a bounded completion card.
- `Attention` → attention lifecycle plus a bounded attention card.
- `Failed` → failed lifecycle plus a bounded failure card.
- `Waiting` → waiting lifecycle plus a bounded attention card.
- `Click` → the representative transient click animation.

The preview component will reuse the atlas frame scheduler and approved asset URL logic. Notification cards in the preview will be read-only fixture data with no activation or dismissal handlers. Scene and selected-Pet signals will live only in the Appearance WebView and will not be written to `AppState`.

If common frame code is currently private to the live Pet component, extract the reusable frame/scene mapping into `lili-pet` or a small shared module rather than duplicating timing tables. The live renderer remains the authority for actual Session-driven animation.

Alternatives considered:

- Replaying synthetic provider events would exercise too much native state and risk changing unread notifications during a visual preview.
- Duplicating CSS-only sprite rows would drift from native frame timing and the Pet v2 contract.
- Rendering only static screenshots would not let users compare states or verify package switching.

### Keep selection persistence and runtime isolation explicit

The selection mutation will reuse the existing identifier-based persistence and fallback behavior. A missing or invalid package remains a diagnostic condition and falls back to the embedded Pet. A Pet selection will update the active transparent Pet and any future Appearance view model, but it will not acknowledge notifications or reset the Session reducer.

The Appearance WebView will receive the narrow signer capability required for its API mutation and no Pet-drag, notification-control, action, or acceptance capabilities. GET requests remain cookie-protected; the selection mutation remains origin-checked and signed by the existing fetch wrapper.

Alternatives considered:

- Keeping a second selection state only in the settings window would make tray and restart behavior inconsistent.
- Writing selection directly from browser code would bypass native package validation and persistence ordering.

## Risks / Trade-offs

- [Multiple validated atlases increase memory or startup work] → Keep the existing package and encoded-size bounds, build a generation-scoped approved registry, and load only metadata plus the assets required by the current view; add measurements to fixture and desktop checks.
- [Settings and transparent Pet windows can drift after a selection] → Publish the active presentation through the existing watch channel, refresh the Appearance view model after a successful mutation, and keep one native selection service for tray and settings.
- [A malformed or removed package can leave a stale list entry] → Revalidate before mutation, issue generation-scoped asset identities, remove invalid entries from the refreshed response, and preserve the embedded fallback with diagnostics.
- [Preview controls could accidentally trigger native behavior] → Use a separate preview scene model and read-only card component, grant only the signed API capability, and assert that fixture scene changes do not alter reducer, notification, or action-audit state.
- [The settings window may behave differently across platforms] → Keep the custom frame within the WebView, preserve the existing loopback/certificate path, and add packaged macOS, Windows, and supported Linux acceptance coverage for opening, focusing, closing, and relaunching the surface.

## Migration Plan

1. Land the Appearance view model, routes, preview component, and settings-window lifecycle behind the new OpenSpec change.
2. Keep the existing selected-Pet persistence field and tray selection path; no database migration or package migration is required.
3. Add the settings entry to the tray/context menu and verify that existing Pet and notification windows continue to start with the same bootstrap paths.
4. Ship the saved design assets and browser fixture coverage with the release.
5. If the surface must be rolled back, remove its menu entry and settings window creation while retaining the existing selected-Pet data and tray selection behavior; no data cleanup is required.

## Open Questions

None. The surface scope, navigation shape, Pet selection semantics, preview scenes, and runtime isolation are resolved for implementation.
