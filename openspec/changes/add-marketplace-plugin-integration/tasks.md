## 1. Contract and Submission Preflight

- [x] 1.1 Capture the supported Codex, ChatGPT, plugin manifest, hook schema, target platform, and public submission requirements in versioned fixtures dated from the official review.
- [ ] 1.2 Validate a minimal skills-only package containing Codex hooks through local Marketplace tooling and the submission portal preflight, and record any scanner or package restrictions before expanding the artifact.
- [x] 1.3 Define the final plugin identity, publisher fields, category, capabilities, country availability, desktop prerequisite, surface-specific copy, and prohibited endorsement language.
- [x] 1.4 Define the supported plugin/application/IPC compatibility matrix and the exact migration and rollback states.

## 2. Plugin Package and Skill

- [x] 2.1 Add `plugins/lili/.codex-plugin/plugin.json` with release-derived versioning, confined component paths, final interface metadata, legal links, and production asset references.
- [x] 2.2 Add a minimal `lili-setup` skill that handles setup, compatibility, trust, status, migration, and troubleshooting without rewriting configuration or accessing credentials and private session data.
- [x] 2.3 Add production icons and logos, deterministic asset validation, reviewed license provenance, and no screenshots for the UI-less plugin.
- [x] 2.4 Add schema and policy checks that reject path escapes, missing files, placeholders, metadata drift, endorsement claims, undeclared binaries, and accidental MCP, UI, authentication, or network configuration.

## 3. Marketplace Lifecycle Hooks

- [x] 3.1 Add plugin-bundled `SessionStart`, `UserPromptSubmit`, `PermissionRequest`, `Stop`, and `SessionEnd` observer hooks using `${PLUGIN_ROOT}`, bounded timeouts, asynchronous execution where supported, and a Windows command override.
- [x] 3.2 Add POSIX and PowerShell launchers that select only packaged supported targets, preserve stdin, quote plugin paths safely, ignore provider content during command construction, and fail closed without `PATH` fallback.
- [ ] 3.3 Extend release builds to produce the required signed or platform-standard `lili-hook` binaries from the same source version and assemble them into a deterministic universal plugin archive.
- [ ] 3.4 Add launcher and hook tests for paths with spaces, unsupported hosts, missing or tampered binaries, bounded exit behavior, empty stdout, non-decision permission handling, and concurrent invocation.

## 4. Integration Status and Migration

- [ ] 4.1 Update Lili diagnostics to report supported Codex plugin availability, installed and enabled state, hook source, trust state, plugin version, desktop version, IPC compatibility, last accepted plugin event, and safe remediation.
- [ ] 4.2 Reclassify `lili integrate plan` and `install` as explicit legacy/fallback operations while preserving inspect, conflict preview, backups, atomicity, coexistence, and provenance-aware uninstall.
- [ ] 4.3 Implement the install, trust, verify, deduplicated overlap, and legacy-cleanup migration workflow with rollback on any failed precondition.
- [ ] 4.4 Add migration tests for clean plugin adoption, unrelated hooks and notify commands, untrusted hooks, incompatible versions, concurrent sessions, failed verification, repeated migration, and modified legacy provenance.
- [ ] 4.5 Ensure supported plugin removal and rollback never delete the desktop application, pet packages, actions, spool, unrelated hooks, or legacy configuration.

## 5. Privacy, Listing, and Review Materials

- [ ] 5.1 Publish matching website, support, privacy-policy, and terms content covering local data categories, purposes, recipients, retention, deletion controls, security boundaries, and the absence of telemetry or remote transfer.
- [ ] 5.2 Add final short and long descriptions, starter prompts, release notes, availability, prerequisite instructions, and a dated Marketplace compliance checklist with no inaccurate ChatGPT lifecycle or OpenAI endorsement claims.
- [ ] 5.3 Add at least five reproducible positive reviewer cases covering setup, trusted delivery, offline recovery, migration, and diagnostics, with fixture data and expected workflow and result shapes.
- [ ] 5.4 Add at least three reproducible negative reviewer cases covering unsupported ChatGPT lifecycle observation, permission approval or secret extraction, remote transfer, and unsupported hosts.
- [ ] 5.5 Add automated consistency checks across the manifest, skill, documentation, policies, submission metadata, release contents, and observed runtime data handling.

## 6. Marketplace and Packaged Acceptance

- [ ] 6.1 Add a repo-local Marketplace catalog and automated clean-home install, list, enable, disable, update, rollback, and remove round trips using supported plugin commands and the final archive.
- [ ] 6.2 Verify that untrusted plugin hooks are skipped, explicit trust enables them, hook changes invalidate trust, and no workflow bypasses review.
- [ ] 6.3 Run the versioned Codex fixture matrix through plugin-attributed hooks and verify normalized delivery, deduplication, bounded offline spooling, recovery, and no model-visible hook output.
- [ ] 6.4 Extend packaged macOS, Windows, and supported Linux acceptance to install the plugin from the local Marketplace and exercise the declared operating-system and architecture targets against the matching desktop bundle.
- [ ] 6.5 Add archive checksums, binary signing verification where applicable, dependency and license inventory, vulnerability scanning, and final-package inspection for secrets, private fixtures, development paths, and undeclared network endpoints.
- [ ] 6.6 Update the repository OpenSpec check to validate every active change instead of the previously hard-coded change only.
- [ ] 6.7 Gate `submission-ready` on strict OpenSpec validation, all automated and packaged acceptance, reachable public URLs, verified matching publisher identity, final reviewer materials, current-rule revalidation, and successful submission portal preflight.
