# Lili Appearance Design

## Scope

This design covers the first Appearance surface for the Lili desktop pet. The page is intentionally limited to Pet configuration and preview.

## Layout

- A desktop application shell with Lili branding and an Appearance title, presented inside a rounded custom window frame.
- An always-on-top frameless native window with a dedicated drag region, branded minimize and close controls, and a target initial size of 1180 x 860 px; macOS uses the same shared `NSPanel` policy as the desktop companion windows.
- A left navigation rail with one visible entry: `Pet`.
- A center preview workbench with explicit scene controls.
- A right-side installed Pet list used to change the selected package; each item shows a looping Idle atlas thumbnail.
- Pet imagery is presentation-only: browser image selection, swipe, and drag interactions are disabled while the surrounding controls remain operable.
- No Active pet summary and no additional configuration forms.

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

## Saved design assets

- `lili-appearance.png`: static high-fidelity design draft.
- `lili-appearance.html`: self-contained interactive preview with Pet and scene switching.

The prototype uses the repository fallback Pet frames as embedded preview assets so it can be reviewed without filesystem access to the application data directory.
