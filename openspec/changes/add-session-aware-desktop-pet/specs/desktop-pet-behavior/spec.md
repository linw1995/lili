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
The system SHALL map idle to row 0, attention required to waiting row 6, a recent activity reminder to the standard running row 7, an unread successful completion to review row 8, and an unacknowledged failure to failed row 5. Temporary direct interactions SHALL use waving row 3 or jumping row 4 and then return to the highest-priority lifecycle state.

#### Scenario: Turn begins and completes
- **WHEN** a session emits an activity event and then successfully completes
- **THEN** the pet changes from the activity reminder to review and displays one unread completion notification

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
The system SHALL render pet-anchored notification cards in a separate transparent native window with event type, project label, relative time, and a bounded display-safe summary. Cards SHALL fill from the bottom upward, with the newest notification occupying the bottom slot closest to the pet and older notifications ordered upward by recency regardless of lifecycle priority. The notification window SHALL anchor to the visible sprite boundary with a 4 logical pixel visual gap, SHALL follow the pet window, share its visibility and always-on-top policy, remain within the selected display work area, and SHALL NOT depend on content overflowing the pet WebView bounds. On macOS, the notification window SHALL use the same non-activating companion panel and cross-Space collection behavior as the Pet window without installing a second Pet context-menu handler. Raw prompts, full assistant messages, commands, and approval arguments SHALL be hidden by default.

#### Scenario: Notification content is shorter than the maximum stack
- **WHEN** one or more cards occupy less than the 142 pixel scroll bound
- **THEN** the native notification window shrinks to the measured content plus bounded visual padding and reanchors, avoiding a large transparent native hit-test region

#### Scenario: User right-clicks an application surface
- **WHEN** a context-menu gesture occurs on a notification, transparent background, or the Lili context-menu window itself
- **THEN** the preloaded surface's root context-menu handler and native AppKit event handling suppress the browser menu, and no Reload, inspection, or additional Pet context action is exposed

#### Scenario: Initial blank menu document finishes loading
- **WHEN** the hidden context-menu WebView reports its initial `about:blank` page as finished
- **THEN** the popup remains unready until the exact authenticated context-menu URL finishes loading

#### Scenario: Notification WebView is still initializing
- **WHEN** a context-menu gesture occurs on the notification window before authenticated navigation or hydration completes
- **THEN** a document-start blocker suppresses the browser menu on every supported platform

#### Scenario: User right-clicks the Pet sprite
- **WHEN** a true secondary-button gesture occurs on the Pet sprite
- **THEN** the bounded Lili context menu opens without exposing browser reload or inspection actions

#### Scenario: First interaction is a Pet right-click
- **WHEN** the non-activating Pet panel has received no prior left click or DOM pointer movement and the user right-clicks the visible sprite
- **THEN** native event coordinates match the sprite hit region and open the Lili context menu on that first gesture

#### Scenario: Pet moves while notifications are visible
- **WHEN** the pet window moves within or between display work areas
- **THEN** the notification window remains anchored above the pet, falls below it with cards top-aligned when the upper edge has insufficient space, preserves the 4 pixel visual gap, and stays fully inside the selected work area

#### Scenario: Notification window changes display scale
- **WHEN** moving the companion surfaces causes the notification window to receive a new scale factor
- **THEN** placement is recomputed using its new physical size so centering and work-area clamping remain correct

#### Scenario: Notification document finishes after native placement
- **WHEN** native placement is computed before the authenticated notification page finishes loading
- **THEN** the retained above/below mode is reapplied to the final document so card alignment matches the native window position

#### Scenario: Hidden notification window receives new content
- **WHEN** unread content causes the notification window to be shown while another application has focus
- **THEN** the window appears without taking focus until the user clicks it or uses the explicit keyboard focus route

#### Scenario: Pet visibility changes
- **WHEN** the user hides or restores the pet while unread notifications exist
- **THEN** the separate notification window is hidden or restored with the pet without affecting native session ingestion

#### Scenario: Notification window receives a close shortcut
- **WHEN** unread notifications exist and the operating system requests that the notification window close
- **THEN** the close is prevented and the notification window remains reconciled with the visible Pet and unread state

#### Scenario: Pet receives focus over the notification window
- **WHEN** Pet and notification windows overlap through the Pet transparent margin and the Pet becomes focused
- **THEN** the notification window is raised again so its Open and Dismiss controls remain interactive

#### Scenario: macOS Space changes with unread notifications
- **WHEN** the user switches Spaces while Pet and notification windows are visible
- **THEN** both non-activating companion panels remain together at their anchored positions without activating the application

#### Scenario: Completion contains a long assistant message
- **WHEN** a completion event includes provider text beyond the display-safe bound
- **THEN** the card shows a redacted or truncated summary and never expands automatically to the raw message

#### Scenario: Multiple notifications are queued
- **WHEN** more than one unread event exists
- **THEN** cards fill upward from the bottom in reverse-recency order and each can be dismissed independently

### Requirement: Respect reduced motion and keyboard access
The system SHALL honor the operating-system reduced-motion preference, expose keyboard-reachable notification and tray actions, and provide accessible labels independent of sprite imagery.

#### Scenario: Reduced motion is enabled
- **WHEN** the operating system requests reduced motion
- **THEN** the pet uses stable representative frames and state changes without looping movement while notifications remain fully usable

#### Scenario: Notification surface is active
- **WHEN** the separate notification WebView is loaded
- **THEN** it updates relative time on a low-frequency clock and does not run the Pet animation, gaze, or click polling loop

#### Scenario: Notification is operated by keyboard
- **WHEN** focus reaches a notification card and the user activates or dismisses it
- **THEN** the same action or dismissal semantics apply as for pointer input

#### Scenario: Keyboard focus crosses companion windows
- **WHEN** the Pet has keyboard focus and unread notifications are visible
- **THEN** `Alt+N` focuses the first notification control and `Escape` from the notification surface returns focus to the Pet

### Requirement: Recover the rendered shell without losing native ingestion
The application SHALL keep session ingestion and lifecycle aggregation in the native runtime and SHALL recover the UI after a renderer reload or stream reconnect without duplicating notifications.

#### Scenario: Renderer reloads during an active turn
- **WHEN** the WebView reloads or reconnects
- **THEN** it receives a current state snapshot followed by newer events and renders the active state once
