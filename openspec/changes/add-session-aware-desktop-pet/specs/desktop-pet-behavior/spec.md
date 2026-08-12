## Purpose

Defines the visible desktop pet window, session-driven animation state, interaction behavior, notification presentation, and accessibility guarantees.

## ADDED Requirements

### Requirement: Provide an ambient desktop pet window
The desktop application SHALL present a transparent, frameless pet window that can remain above ordinary windows, can be hidden or restored from a tray control, and does not expose filesystem or process authority to rendered page content.

#### Scenario: Application starts normally
- **WHEN** the desktop application finishes native initialization
- **THEN** a valid pet is rendered without an opaque page background or visible browser chrome

#### Scenario: User hides the pet
- **WHEN** the user chooses Hide from the tray control
- **THEN** the pet window is hidden, session event ingestion continues, and the tray can restore it

### Requirement: Restore a visible window position
The system SHALL persist the pet position in logical display coordinates and SHALL clamp or relocate it to a visible work area when displays, scale factors, or work areas change.

#### Scenario: Previous display is disconnected
- **WHEN** the persisted position is outside all current work areas
- **THEN** the pet is placed within the primary display work area with a visible margin

### Requirement: Map lifecycle state to pet animation
The system SHALL map idle to row 0, attention required to waiting row 6, an active turn to running row 7, an unread successful completion to review row 8, and an unacknowledged failure to failed row 5. Temporary direct interactions SHALL use waving row 3 or jumping row 4 and then return to the highest-priority lifecycle state.

#### Scenario: Turn begins and completes
- **WHEN** a session transitions from active to successfully completed
- **THEN** the pet changes from running to review and displays one unread completion notification

#### Scenario: Interaction occurs during waiting
- **WHEN** the user triggers a temporary wave while a session still requires attention
- **THEN** the wave completes and the pet returns to waiting with the attention notification intact

### Requirement: Support gaze and drag behavior
The pet SHALL look toward the pointer while hover tracking is active, SHALL use running-left or running-right during horizontal drag movement, and SHALL persist the final clamped position when dragging ends.

On macOS, the desktop pet window SHALL be backed by a non-activating `NSPanel` configured for desktop-companion behavior. It SHALL remain outside accessibility-driven virtual-workspace window trees rather than being assigned to one emulated workspace. Controlled dragging SHALL derive each native window target from the pointer's absolute screen position and the window origin captured at drag start, rather than accumulating relative window deltas.

#### Scenario: macOS uses a desktop-companion panel
- **WHEN** the pet window is created on macOS
- **THEN** its native window is an `NSPanel` that remains available across application deactivation and Spaces without activating the application merely to display the pet

#### Scenario: Virtual workspace changes
- **WHEN** an accessibility-driven window manager switches between emulated workspaces
- **THEN** the pet remains at its absolute visible position because it is exposed as an unmanaged companion popup rather than a normal or dialog window assigned to one workspace

#### Scenario: Dragging follows an absolute screen target
- **WHEN** pointer movement events are sparse, coalesced, or delivered after the native window has moved
- **THEN** each movement resolves the same absolute window origin from the drag-start anchor and current screen pointer coordinates without accumulating position error

#### Scenario: User grabs the visible pet
- **WHEN** the primary pointer presses and moves within the pet sprite
- **THEN** controlled native window movement starts from the pet hit region without making the transparent window margin draggable or blocking animation rendering

#### Scenario: User grabs an inactive pet
- **WHEN** the non-activating macOS pet panel is not the active window and the primary pointer presses and moves within the pet sprite
- **THEN** the first mouse press reaches the rendered pet and starts the same controlled drag without a preliminary activation click

#### Scenario: User drags the pet to the right
- **WHEN** horizontal drag velocity is positive beyond the movement threshold
- **THEN** the pet displays running-right until the drag slows or ends

#### Scenario: Native window dragging owns pointer movement
- **WHEN** the platform drag loop moves the window without delivering continuous WebView pointer events
- **THEN** native window movement keeps the running-left or running-right animation responsive without flickering back to the lifecycle animation between adjacent movement samples

#### Scenario: Drag ends near a work-area boundary
- **WHEN** the pointer is released with part of the pet outside the visible work area
- **THEN** the final window position is clamped so the configured visible portion remains reachable

### Requirement: Present privacy-safe session notifications
The system SHALL render pet-anchored notification cards with event type, project label, relative time, and a bounded display-safe summary. Raw prompts, full assistant messages, commands, and approval arguments SHALL be hidden by default.

#### Scenario: Completion contains a long assistant message
- **WHEN** a completion event includes provider text beyond the display-safe bound
- **THEN** the card shows a redacted or truncated summary and never expands automatically to the raw message

#### Scenario: Multiple notifications are queued
- **WHEN** more than one unread event exists
- **THEN** cards are ordered by attention priority and recency and each can be dismissed independently

### Requirement: Respect reduced motion and keyboard access
The system SHALL honor the operating-system reduced-motion preference, expose keyboard-reachable notification and tray actions, and provide accessible labels independent of sprite imagery.

#### Scenario: Reduced motion is enabled
- **WHEN** the operating system requests reduced motion
- **THEN** the pet uses stable representative frames and state changes without looping movement while notifications remain fully usable

#### Scenario: Notification is operated by keyboard
- **WHEN** focus reaches a notification card and the user activates or dismisses it
- **THEN** the same action or dismissal semantics apply as for pointer input

### Requirement: Recover the rendered shell without losing native ingestion
The application SHALL keep session ingestion and lifecycle aggregation in the native runtime and SHALL recover the UI after a renderer reload or stream reconnect without duplicating notifications.

#### Scenario: Renderer reloads during an active turn
- **WHEN** the WebView reloads or reconnects
- **THEN** it receives a current state snapshot followed by newer events and renders the active state once
