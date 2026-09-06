## Purpose

Defines the local Appearance surface where users choose a validated Pet package and preview its supported desktop states without coupling preview interactions to live session processing or native action execution.

## ADDED Requirements

### Requirement: Render the focused Pet Appearance surface

The system SHALL provide a dedicated always-on-top Appearance surface with a Lili application shell inside a custom frameless window frame, a left navigation list containing exactly one visible configuration entry named `Pet`, a center live-preview area, and a right-side Pet list. The surface SHALL NOT present Notifications, Interactions, Connection, Active pet summary, or unrelated configuration forms as additional navigation entries or controls.

#### Scenario: Appearance surface opens

- **WHEN** the user opens Appearance from the desktop application
- **THEN** the surface shows `Pet` as the selected and only visible navigation entry, shows the live preview in the center, shows the installed Pet list on the right, and exposes the custom frame's drag, minimize, and close affordances

#### Scenario: No other configuration page exists

- **WHEN** the current release has no implementation for another settings page
- **THEN** the navigation still renders the single `Pet` entry without exposing dead links, placeholder page content, or controls that imply another page is available

### Requirement: List and select validated Pet packages

The system SHALL list every Pet package that has passed the existing Pet v2 validation boundary using its display name and stable identifier, mark the persisted selection, and allow the user to select another listed package. A selection SHALL be represented by the Pet identifier rather than a filesystem path.

#### Scenario: Valid packages are listed

- **WHEN** the application has one or more validated packages in its Lili-owned Pet root
- **THEN** the right-side list contains each available package, each entry renders a looping Idle atlas thumbnail, and exactly one package is marked selected when a valid selection exists

#### Scenario: Pet list previews Idle state

- **WHEN** the user views the installed Pet list
- **THEN** every listed package renders its approved asset in a fixed thumbnail viewport using the Idle scene, without requiring a scene change or triggering native runtime state

#### Scenario: Only the embedded fallback is available

- **WHEN** no external package passes validation
- **THEN** the list contains the embedded fallback Pet, which remains selectable and renderable

#### Scenario: User selects a different Pet

- **WHEN** the user activates an unselected Pet in the right-side list
- **THEN** the selected marker moves to that Pet, the preview changes to use that package, and the selected identifier is persisted for the next application start

#### Scenario: Selected package disappears or becomes invalid

- **WHEN** the persisted identifier no longer resolves to a valid package
- **THEN** the application selects the embedded fallback, keeps the surface usable, and records the package-specific condition through the existing diagnostics boundary

### Requirement: Preview supported Pet scenes

The system SHALL expose explicit, keyboard-reachable scene controls for `Idle`, `Running`, `Review`, `Attention`, `Failed`, `Waiting`, and `Click`. Selecting a scene SHALL update the center preview to the corresponding representative lifecycle, notification, or transient interaction presentation using the same approved Pet asset and animation mappings as the desktop renderer. Notification-bearing scenes SHALL reuse the production notification-card component and styles with controls disabled for preview.

#### Scenario: Initial preview is useful

- **WHEN** the Appearance surface finishes loading with a selected Pet
- **THEN** the `Idle` scene is selected and the center preview renders the selected Pet without requiring a live Session event

#### Scenario: User switches scenes

- **WHEN** the user selects any scene control
- **THEN** the selected scene is visibly marked and the Pet image, lifecycle label, notification treatment, and supporting preview copy update to that scene without navigating away

#### Scenario: Notification-bearing scene is selected

- **WHEN** the user selects `Review`, `Attention`, or `Failed`
- **THEN** the preview shows the corresponding bounded completion, attention, or failure notification treatment while keeping the Pet and notification relationship legible

#### Scenario: Transient click scene is selected

- **WHEN** the user selects `Click`
- **THEN** the preview shows the representative click animation state without dispatching a native action or changing a live Session notification

#### Scenario: Preview uses the selected package

- **WHEN** the user switches Pet and then selects a scene
- **THEN** the scene preview uses the newly selected approved asset and never requests an arbitrary path or a package that failed validation

### Requirement: Keep Appearance preview isolated from runtime state

The system SHALL keep selected scene state and preview-only presentation state local to the Appearance surface. Preview interactions SHALL NOT ingest provider events, acknowledge or dismiss notifications, execute configured actions, or mutate the native Session reducer.

#### Scenario: Live Session event arrives during preview

- **WHEN** a normalized Session event is accepted while the user is viewing or changing a preview scene
- **THEN** the native Session presentation continues to update through its normal path, while the preview remains on the user-selected scene and no preview action is dispatched

#### Scenario: Pet selection races with a Session update

- **WHEN** the user selects a Pet at the same time that a Session event is reduced
- **THEN** the selected Pet and the Session event are each applied through their own state boundaries, with no lost event, duplicate notification, or preview-driven acknowledgement

#### Scenario: Web content sends an invalid selection

- **WHEN** a renderer submits an unknown identifier, an absolute path, or a path traversal value as the Pet selection
- **THEN** the request is rejected without reading the supplied path, changing the active Pet, or mutating persisted state

### Requirement: Preserve secure and accessible Appearance interactions

The system SHALL serve the Appearance surface through the existing authenticated local WebView boundary, expose only bounded Pet metadata and approved asset identities to the renderer, and make Pet and scene controls operable by keyboard with a visible selected state and concise accessible names.

#### Scenario: Renderer requests Pet metadata

- **WHEN** the Appearance renderer loads its data
- **THEN** it receives display-safe Pet metadata and an approved asset identity, but no application root, arbitrary filesystem path, loopback credential, action command, or provider payload

#### Scenario: User operates controls by keyboard

- **WHEN** keyboard focus reaches the Pet list or scene controls
- **THEN** the user can select a Pet or scene without a pointer, and the selected state is exposed through native focus and accessibility semantics

#### Scenario: Appearance renderer reconnects

- **WHEN** the Appearance WebView reloads or reconnects
- **THEN** it receives the current selected Pet and a usable default scene, and the reconnect does not duplicate Session notifications or execute preview actions
