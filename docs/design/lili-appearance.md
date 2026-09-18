# Lili Appearance Design

## Scope

This design covers the Appearance surface for the Lili desktop pet, including Pet configuration, preview, and launch-at-login settings.

## Layout

- A desktop application shell with Lili branding and an Appearance title, presented inside a rounded custom window frame.
- An always-on-top frameless native window with a dedicated drag region and a branded close control at the top left, with a target initial size of 1180 x 900 px; macOS uses the same shared `NSPanel` policy as the desktop companion windows.
- The initial window is capped to the monitor work area and centered within it. The shell fills the available viewport without document or outer-shell scrolling; content scrolls inside the workspace.
- A left navigation rail with one visible entry: `Pet`.
- A center preview workbench with explicit scene controls.
- An open preview stage without a nested desktop window mockup or extra preview-status badge.
- A right-side installed Pet list used to change the selected package; each item shows a looping Idle atlas thumbnail.
- A Startup card below the Pet list keeps launch-at-login settings alongside the preview without adding height above it; narrow layouts stack both cards below the preview.
- Pet imagery is presentation-only: browser image selection, swipe, and drag interactions are disabled while the surrounding controls remain operable.
- Readable text can be selected only inside its own marked text container; chrome and controls remain unselectable.
- No Active pet summary.

## Viewport and scrolling contract

The viewport determines the shell height. The frame fills that height, and flex/grid descendants use zero minimum sizes and `minmax(0, 1fr)` tracks to pass the remaining space down to the workspace. Content must never enlarge the frame. The title bar, close control, navigation, and page heading remain outside scrolling content.

Short windows use compact shell spacing, as narrow windows do, so decorative gutters do not consume the usable preview area.

- Above 900 px, the preview content and settings column own vertical scrolling independently. The preview mode controls remain above the preview scroller. Pet and Startup cards keep their intrinsic heights inside the settings scroller.
- At 900 px and below, the panels stack inside one vertically scrolling workbench. The nested preview and settings containers stop scrolling vertically.
- The complete sprite sheet owns horizontal scrolling only. Its height follows its rows; vertical scrolling belongs to the preview content or compact workbench. New rows, controls, and Pet packages must not introduce another fixed-height vertical scroller.
- Sprite cells keep their complete aspect ratio and dimensions. A small viewport changes the scrollable area, not the image contents. Focused controls remain reachable by keyboard inside their scroll owner.

Browser regressions assert shell bounds and close-control visibility in every preview mode, at wide, narrow, and short viewport sizes, during resizing, and with the maximum 64 Pet packages. They also require the last sprite cell, last Pet option, and Startup control to remain reachable, so clipping cannot masquerade as a layout fix.

## Preview scenes

The center preview exposes the states already represented by the Pet renderer:

- `Idle`
- `Running`
- `Review`
- `Attention`
- `Failed`
- `Waiting`
- `Click`

Scene selection is presentation-only. It does not deliver a Session event, acknowledge a notification, or execute an interaction action. Notification-bearing scenes reuse the production notification card and styles with disabled controls.

## Sprite browsing

Live preview defaults to `Scenes`. The adjacent `Sprites` mode provides complete asset inspection without requiring gestures:

- `Animations` exposes all nine standard animations using the shared frame counts and durations. Playback can be paused or resumed; frame thumbnails and previous/next controls select a frame and pause playback, wrapping at animation boundaries.
- `Look directions` fixes any of the sixteen directions, clockwise from up, or the neutral pose. Pointer movement does not change the selection.
- `Sprite sheet` displays all 88 cells in their original eight-column layout. Labels distinguish animation frames, look directions, the neutral pose, and unused cells. Selecting a cell shows it at full cell size. Narrow windows scroll the sheet horizontally; vertical overflow follows the workspace scrolling contract.

The sprite stage preserves the complete 192 x 208 cell and uses a checkerboard background to reveal transparency. Sprite selection is available through native keyboard-operable buttons. Reduced motion disables automatic playback while retaining manual frame selection and stepping.

Changing the selected Pet preserves the sprite target and paused state, restarting animations at their first frame. Returning to `Scenes` restores the previously selected scene. All sprite state remains local to Appearance and never invokes native actions or mutates Session state.

## Saved design assets

- `lili-appearance.png`: static high-fidelity design draft.
- `lili-appearance.html`: self-contained interactive preview with Pet and scene switching.

The prototype uses the repository fallback Pet frames as embedded preview assets so it can be reviewed without filesystem access to the application data directory. These original design assets illustrate scene preview; sprite browsing is implemented in the application.
