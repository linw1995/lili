## Purpose

Defines a safe, truthful, review-ready ChatGPT/Codex plugin distribution path for Lili's local Codex Session bridge while preserving explicit trust, privacy, compatibility, migration, and removal boundaries.

## ADDED Requirements

### Requirement: Package Lili as a valid plugin
The project SHALL produce a deterministic plugin package with a confined `.codex-plugin/plugin.json`, final install-surface metadata, a narrowly scoped setup and diagnostics skill, Codex lifecycle hooks, supported platform launchers and forwarders, and required assets whose versions match the application release.

#### Scenario: Package is assembled for release
- **WHEN** the release pipeline assembles a Lili plugin
- **THEN** every manifest path remains inside the plugin root, every declared file exists, the plugin and application versions agree, and the archive contains no development paths, credentials, private fixtures, or undeclared executable

#### Scenario: Package metadata is incomplete
- **WHEN** publisher metadata, public legal links, surface descriptions, assets, or supported target declarations contain placeholders or disagree with the package
- **THEN** the release is not marked submission-ready

### Requirement: Disclose surface-specific behavior accurately
The plugin SHALL state that it is discoverable through the shared ChatGPT and Codex Plugin Directory, that automatic Session lifecycle forwarding currently runs only on Codex surfaces with supported lifecycle hooks, and that the separately installed Lili desktop application is required for desktop presentation.

#### Scenario: User installs from ChatGPT
- **WHEN** a user reviews or installs the plugin on a ChatGPT surface without Codex lifecycle hooks
- **THEN** the plugin provides accurate setup and compatibility guidance without claiming to observe that ChatGPT conversation

#### Scenario: User enables the plugin in supported Codex
- **WHEN** a user enables and trusts the plugin hooks in a supported Codex version while Lili is installed
- **THEN** supported lifecycle events can reach the local desktop application

### Requirement: Integrate without rewriting Codex configuration
The default Marketplace integration SHALL use plugin-bundled lifecycle hooks and SHALL NOT add, replace, or chain the user-level `notify` command or edit user or project hook configuration.

#### Scenario: Existing integrations are present
- **WHEN** the plugin is installed for a user who already has unrelated notification commands or lifecycle hooks
- **THEN** those integrations remain byte-for-byte unchanged and continue to load alongside the plugin according to Codex policy

#### Scenario: Marketplace integration is unavailable
- **WHEN** the installed Codex version or workspace policy does not support the plugin path
- **THEN** Lili reports the limitation and offers the reviewed direct-config flow only as an explicit version-gated fallback

### Requirement: Preserve hook trust and user authority
Plugin installation or enablement SHALL NOT mark command hooks trusted, bypass hook review, approve or deny a permission request, or cause a state-changing action outside Lili without separate user authorization.

#### Scenario: Hook has not been trusted
- **WHEN** Codex discovers a new or changed Lili hook definition that the user has not trusted
- **THEN** the hook is skipped, Lili reports that review is required, and no automatic trust mutation is attempted

#### Scenario: Permission event is forwarded
- **WHEN** the trusted plugin receives a `PermissionRequest` event
- **THEN** it forwards an observer-only normalized event without emitting an approval decision or blocking the Codex permission flow

### Requirement: Forward events through a bounded local bridge
The plugin hook launchers SHALL execute only the packaged forwarder selected from the declared host matrix, preserve the Codex-provided `PLUGIN_DATA` root for the forwarder to derive the installed plugin selector, accept no caller-selected plugin identity argument, preserve the structured hook input on stdin, emit no model-visible content, perform no download or remote request, and retain the existing limits, authentication, replay protection, redaction, and offline-spool behavior.

#### Scenario: Lili is available
- **WHEN** concurrent supported lifecycle events invoke trusted plugin hooks while the matching desktop application is running
- **THEN** each accepted normalized event reaches the authenticated local reducer at most once per stable event identity without serializing unrelated hooks

#### Scenario: Lili is offline or restarting
- **WHEN** a trusted hook cannot connect within its deadline
- **THEN** the forwarder writes only a bounded owner-only normalized spool record and exits without delaying the Codex lifecycle beyond the configured timeout

#### Scenario: Host target is unsupported
- **WHEN** the launcher cannot select an exact packaged operating-system and architecture target
- **THEN** it fails closed with a bounded local diagnostic and does not execute a binary from `PATH` or another untrusted location

### Requirement: Migrate legacy direct configuration without loss or duplication
The migration workflow SHALL overlap the plugin and provenance-owned legacy integration until plugin trust and delivery are verified, rely on stable event deduplication during overlap, and remove only Lili-owned legacy entries after verification succeeds.

#### Scenario: Migration succeeds during active sessions
- **WHEN** multiple sessions emit events while the trusted plugin and legacy integration temporarily coexist
- **THEN** the user receives one notification per stable event identity and unrelated integrations remain unchanged

#### Scenario: Plugin verification fails
- **WHEN** the plugin remains untrusted, incompatible, or unable to deliver its verification event
- **THEN** the migration preserves the legacy integration and reports the failed precondition without partial cleanup

#### Scenario: Plugin trust changes after assessment
- **WHEN** an installed plugin version or any selected hook trust state changes after a cleanup-ready assessment
- **THEN** cleanup re-inspects the current hooks, rejects the stale assessment, and preserves the legacy integration

#### Scenario: Real delivery is verified on an unreviewed Codex version
- **WHEN** a compatible plugin on an unreviewed Codex version has verified real delivery and every selected hook remains trusted at cleanup time
- **THEN** migration may remove only provenance-owned legacy entries, while unknown Codex versions remain blocked

### Requirement: Support independent plugin and application lifecycle
The plugin and desktop application SHALL expose their release and IPC schema versions, SHALL accept only declared compatible version pairs, and SHALL keep installation, update, rollback, removal, and application-data ownership independent.

#### Scenario: Compatible versions differ
- **WHEN** a supported older plugin forwards to a newer application or a newer plugin forwards to a supported older application
- **THEN** the declared IPC compatibility range permits delivery and diagnostics identify both versions

#### Scenario: Versions are incompatible
- **WHEN** the plugin and application do not share a supported IPC schema
- **THEN** the event is retained only when the existing safe spool contract can represent it and the user receives remediation guidance instead of silent data loss

#### Scenario: Plugin is removed
- **WHEN** the user removes Lili through the Plugin Directory or supported Codex plugin command
- **THEN** plugin hooks stop loading while the desktop application, pet packages, actions, spool, and unrelated or legacy configuration remain untouched

### Requirement: Meet Marketplace listing and privacy requirements
The submission SHALL use a verified and matching publisher identity, clear non-promotional naming, accurate descriptions, production brand assets, reachable website and support URLs, and published privacy and terms documents that match the plugin's actual local data handling.

#### Scenario: User reviews the listing before install
- **WHEN** the Plugin Directory presents Lili
- **THEN** the user can understand its purpose, Codex-only lifecycle boundary, desktop prerequisite, local data categories, purposes, recipients, retention, deletion controls, support path, and terms before installation

#### Scenario: Runtime behavior exceeds disclosure
- **WHEN** review detects undisclosed collection, telemetry, network transfer, raw prompt logging, credential access, or retention behavior
- **THEN** submission fails until behavior or published policy is corrected and revalidated

### Requirement: Provide reproducible review evidence
Every submission-ready release SHALL include realistic starter prompts, at least five reproducible positive cases, at least three reproducible negative cases, fixture prerequisites, expected workflow and result shapes, release notes, availability declarations, and a dated review against the current official plugin rules.

#### Scenario: Reviewer runs a positive case
- **WHEN** a reviewer follows a documented supported setup, trust, delivery, offline recovery, migration, or diagnostics case
- **THEN** the observed skill and hook behavior matches the declared expected result without private context or inaccessible credentials

#### Scenario: Reviewer runs a negative case
- **WHEN** a reviewer requests unsupported ChatGPT lifecycle observation, automatic permission approval, credential or conversation extraction, remote transmission, or execution on an unsupported host
- **THEN** the plugin refuses, clarifies the boundary, or fails safely as documented

### Requirement: Validate installed behavior across supported releases
CI and packaged acceptance SHALL validate the final plugin archive through a local Marketplace install, disabled and untrusted states, explicit hook trust, lifecycle delivery, offline recovery, update, rollback, removal, and legacy migration on every declared host and supported Codex version.

#### Scenario: Local Marketplace round trip succeeds
- **WHEN** a clean supported environment adds the test Marketplace, installs the final archive, enables the plugin, reviews and trusts hooks, exercises fixtures, and removes the plugin
- **THEN** plugin state, hook source attribution, delivery results, and cleanup match the published contract

#### Scenario: Hook definition changes during update
- **WHEN** an update changes a trusted hook command or definition
- **THEN** Codex requires review of the new definition and the acceptance suite rejects any automatic reuse of stale trust
