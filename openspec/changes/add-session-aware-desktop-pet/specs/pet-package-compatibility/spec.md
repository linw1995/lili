## Purpose

Defines how Lili discovers, validates, selects, and renders Codex v2 pet packages without changing the package format or accepting malformed sprite geometry.

## ADDED Requirements

### Requirement: Discover Codex v2 pet packages
The system SHALL use `${CODEX_HOME}/pet/lili/` as Lili's fixed default user package location, discover additional user pet packages from `${CODEX_HOME}/pets/<pet-id>/`, default `CODEX_HOME` to the platform Codex home, and require `pet.json` and the manifest-referenced spritesheet to remain inside the package directory.

#### Scenario: Valid package is discovered
- **WHEN** a package directory contains a valid `pet.json` and referenced spritesheet
- **THEN** the pet is listed by its identifier, display name, and description

#### Scenario: Default Lili package is discovered
- **WHEN** `${CODEX_HOME}/pet/lili/` contains a valid v2 package
- **THEN** the package is considered before additional packages with the same identifier

#### Scenario: Escaping asset path is rejected
- **WHEN** `spritesheetPath` is absolute, traverses a parent directory, or resolves through a link outside the package
- **THEN** the package is rejected without reading the escaped asset

### Requirement: Validate the v2 manifest and atlas
The system SHALL accept only `spriteVersionNumber: 2` packages whose PNG or WebP atlas is exactly `1536x2288`, consists of `192x208` cells in an 8-column by 11-row grid, and has a decodable transparent image surface.

#### Scenario: Compatible v2 package loads
- **WHEN** the manifest fields and atlas geometry satisfy the v2 contract
- **THEN** the package becomes selectable and renderable

#### Scenario: Unsupported or malformed package fails closed
- **WHEN** the version, file type, image dimensions, manifest fields, or decode operation is invalid
- **THEN** the package is excluded and the UI reports a package-specific diagnostic while retaining a usable fallback pet

### Requirement: Render standard animation rows exactly
The system SHALL render standard rows with the v2 frame counts and timing sequences: idle row 0 columns 0-5 at `280, 110, 110, 140, 140, 320 ms`; reserved neutral look cell at row 0 column 6; running-right row 1 columns 0-7 at `120 ms` per frame and `220 ms` for the final frame; running-left row 2 with the same columns and timing; waving row 3 columns 0-3 at `140 ms` per frame and `280 ms` for the final frame; jumping row 4 columns 0-4 at `140 ms` per frame and `280 ms` for the final frame; failed row 5 columns 0-7 at `140 ms` per frame and `240 ms` for the final frame; waiting row 6 columns 0-5 at `150 ms` per frame and `260 ms` for the final frame; running row 7 columns 0-5 at `120 ms` per frame and `220 ms` for the final frame; and review row 8 columns 0-5 at `150 ms` per frame and `280 ms` for the final frame.

#### Scenario: Standard animation plays
- **WHEN** the behavior state selects a standard animation
- **THEN** only that row's used frames play in order with the contract-defined durations, the neutral cell remains outside the idle loop, and unused cells are never displayed

#### Scenario: Animation timer crosses a frame or loop boundary
- **WHEN** elapsed monotonic time reaches an exact frame boundary or exceeds one or more complete loops
- **THEN** the scheduler selects the next frame at the boundary and wraps by the contract-defined loop duration without accumulating drift

#### Scenario: Development application starts with the fallback pet
- **WHEN** the application starts without a valid user package
- **THEN** the SSR shell references only the approved opaque fallback asset identity and the hydrated view displays the six-frame idle loop instead of a placeholder

#### Scenario: Desktop WebView loads the approved atlas
- **WHEN** the authenticated desktop WebView resolves the active opaque asset identity through a native image request that cannot carry API signature headers
- **THEN** the atlas is served from the cookie-protected non-API asset route while signed API enforcement remains unchanged

#### Scenario: Fallback animation state changes
- **WHEN** the built-in fallback transitions between standard animation rows or look-direction rows
- **THEN** the pet retains a visually consistent tabby palette without a perceptible red, yellow, saturation, or brightness jump between states

### Requirement: Render all look directions in clockwise order
The system SHALL map rows 9 and 10 to the 16 clockwise look directions from `000` through `337.5` degrees, where `000` means up and the no-vector deadzone falls back to idle.

#### Scenario: Pointer direction selects a look cell
- **WHEN** the pointer vector from the pet center falls outside the deadzone
- **THEN** the nearest 22.5-degree look cell is displayed using screen-coordinate direction semantics

#### Scenario: Pointer enters the deadzone
- **WHEN** the pointer vector is within the configured deadzone
- **THEN** no directional cell is displayed and the pet resumes its applicable non-look animation

#### Scenario: Pointer crosses a direction midpoint
- **WHEN** the pointer vector crosses the midpoint between adjacent 22.5-degree directions
- **THEN** the selected cell changes to the nearest clockwise screen-coordinate direction, with midpoint ties resolving clockwise

### Requirement: Persist pet selection safely
The system SHALL persist the selected pet identifier rather than an arbitrary asset path and SHALL revalidate the package on each application start.

#### Scenario: Selected package disappears
- **WHEN** the persisted pet identifier no longer resolves to a valid package
- **THEN** the application uses the built-in fallback, preserves the invalid identifier for diagnostics, and remains operable

### Requirement: Serve only approved pet assets
The system SHALL expose a validated atlas through an opaque generation-scoped asset identity, return its validated image MIME type with private immutable caching and the application CSP, and never expose an arbitrary filesystem path to the renderer.

#### Scenario: Approved atlas is requested
- **WHEN** the renderer requests the active generation's opaque asset identity
- **THEN** the server returns the validated atlas with its image MIME type, private immutable cache policy, CSP, and content-sniffing protection

#### Scenario: Package generation changes
- **WHEN** package state is revalidated after a package change
- **THEN** a new opaque identity is issued and the prior identity no longer resolves

#### Scenario: Unknown asset identity is requested
- **WHEN** a request supplies an identity that is not registered for the active generation
- **THEN** the server returns not found without interpreting the identity as a path
