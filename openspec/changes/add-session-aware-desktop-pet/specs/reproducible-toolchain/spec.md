## Purpose

Defines the reproducible development, validation, and release toolchain for Lili so contributors and CI use the same pinned tools and application version across supported platforms.

## ADDED Requirements

### Requirement: Use a committed Nix Flake as the toolchain authority
The project SHALL commit `flake.nix` and `flake.lock`, and all supported development, validation, and release entry points SHALL obtain their tool versions from the locked Flake rather than an implicitly selected global installation.

#### Scenario: Developer enters the default environment
- **WHEN** a developer runs `nix develop` from a clean checkout
- **THEN** the shell provides the project-pinned Rust, Node.js, Tauri, Trunk, WebAssembly, formatting, and linting tools without modifying `flake.lock`

#### Scenario: Global tool version differs
- **WHEN** a machine has a different global Rust, Node.js, Tauri, or Trunk version
- **THEN** project commands still use the Flake-provided version

### Requirement: Commit language dependency lockfiles
The project SHALL commit `Cargo.lock` and `package-lock.json` alongside `flake.lock`. The Flake SHALL pin system and build tools, while the language lockfiles SHALL pin Rust and npm dependency graphs.

#### Scenario: Clean checkout resolves dependencies
- **WHEN** CI validates a clean checkout with network access disabled after declared dependencies are available in the store or cache
- **THEN** dependency resolution uses the committed lockfiles without selecting newer versions

### Requirement: Provide stable Flake application entry points
The Flake SHALL expose stable commands for desktop development, Web development, desktop build, stylesheet/assets build, formatting, linting, repository hooks, native coverage, CRAP analysis, and end-to-end tests. Command implementations SHALL use the same toolchain composition as the default development shell.

#### Scenario: Developer runs a standard workflow
- **WHEN** a developer invokes a documented `nix run .#<command>` entry point
- **THEN** the command runs with pinned tools and does not require manual PATH construction

#### Scenario: Heavy test dependencies are unused
- **WHEN** a developer enters the default shell or runs a non-E2E command
- **THEN** browser binaries and other E2E-only closures are not required

#### Scenario: Native workspace command runs on clean Linux
- **WHEN** a Flake application compiles or runs the native desktop workspace on a clean Linux host
- **THEN** the application exposes the pinned GLib, GTK, libsoup, and WebKitGTK build and runtime discovery paths without relying on host-installed development packages

#### Scenario: Lightweight command avoids native desktop libraries
- **WHEN** a Flake application does not compile or run the native desktop workspace
- **THEN** the application does not require the native WebView library closure solely because other workspace commands need it

### Requirement: Produce auditable native coverage and CRAP reports
The project SHALL expose separate Flake-provided coverage and CRAP commands. Native coverage SHALL emit LCOV, Cobertura, HTML, and Markdown reports from the locked workspace test graph. CRAP analysis SHALL consume the same LCOV data, report production functions, and reject any production function above the default threshold of 30 without machine-specific baselines, allow lists, or optimistic missing-coverage assumptions.

#### Scenario: CI generates native coverage
- **WHEN** the coverage job runs against a clean checkout
- **THEN** it uploads the complete coverage report directory as an artifact and submits the LCOV report to the configured coverage service

#### Scenario: CRAP analysis passes
- **WHEN** every analyzed production function has a CRAP score at or below 30
- **THEN** the CRAP job writes and uploads the Markdown report and completes successfully

#### Scenario: CRAP analysis rejects a function
- **WHEN** any analyzed production function has a CRAP score above 30
- **THEN** the CRAP job fails after producing annotations and still uploads the Markdown report for diagnosis

#### Scenario: Verification-only code is analyzed
- **WHEN** CRAP analysis discovers test fixtures, acceptance binaries, build scripts, or other verification-only code
- **THEN** those non-production surfaces are excluded without suppressing uncovered production functions

### Requirement: Support declared host systems explicitly
The Flake SHALL evaluate for `aarch64-darwin`, `aarch64-linux`, and `x86_64-linux`. Darwin development SHALL use the pinned surrounding tools with the host Xcode command-line tools and SDK as an explicit platform dependency, while Linux shells SHALL provide the native WebView build libraries.

#### Scenario: Flake outputs are evaluated in CI
- **WHEN** CI evaluates every declared system without building foreign-platform artifacts
- **THEN** required development shells, applications, and checks resolve without missing attributes or evaluation-time impurity

#### Scenario: Developer enters the Darwin shell
- **WHEN** `nix develop` runs on `aarch64-darwin`
- **THEN** the shell uses the host `xcrun` SDK and system compiler while retaining Flake-pinned Rust, Node.js, Tauri, Trunk, and WebAssembly tools

### Requirement: Keep one application release version
The Cargo workspace package version SHALL be the canonical Lili application version. Flake packages, Tauri bundle metadata, the hook-forwarder version, and release artifacts SHALL derive or verify that same value instead of maintaining independent editable version strings.

#### Scenario: Version metadata is validated
- **WHEN** the canonical workspace version changes
- **THEN** Flake and release checks either propagate that value to every artifact or fail with the mismatched surface identified

### Requirement: Make lock updates deliberate and reviewable
Normal development, build, and CI commands SHALL NOT rewrite lockfiles. Toolchain upgrades SHALL use an explicit selective or full Flake update workflow and SHALL validate all declared systems and stable application entry points before acceptance.

#### Scenario: Locked input is unavailable from the current cache
- **WHEN** a normal project command cannot realize a locked input
- **THEN** the command fails without silently updating to a newer input

#### Scenario: Toolchain input is upgraded
- **WHEN** a maintainer intentionally updates a Flake input
- **THEN** the resulting lockfile diff, tool versions, cross-system evaluation, workspace checks, and build entry points are reviewed as one change

### Requirement: Ship an actionable configuration guide
The project SHALL maintain a configuration guide linked from the README and included in release bundles. The guide SHALL distinguish source development from packaged operation and SHALL document Codex home selection, default and additional pet packages, reviewed Session integration installation, interaction action configuration, configuration reload boundaries, verification, troubleshooting, and provenance-aware uninstall using only supported public interfaces.

#### Scenario: User configures a release from a clean environment
- **WHEN** a user opens the release README without prior project knowledge
- **THEN** the linked guide provides an ordered, copyable path to launch Lili, review and install Session integration, configure pet and notification interactions, verify the result, and safely remove the integration

#### Scenario: User edits a startup-loaded configuration
- **WHEN** a user changes a pet package or `actions.toml`
- **THEN** the guide identifies whether Lili must be restarted before the change becomes effective
