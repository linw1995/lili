## Context

The completed `add-session-aware-desktop-pet` change implemented `lili-hook`, authenticated local IPC, offline spooling, Codex lifecycle adapters, and a reversible direct-config installer. The direct installer writes `notify` and lifecycle hook entries into user-level Codex configuration.

Codex 0.147.0 and the current OpenAI plugin contract support plugin-bundled lifecycle hooks. Plugin hooks run with `PLUGIN_ROOT` and `PLUGIN_DATA`, support a Windows-specific command override, load alongside other hook sources, and remain disabled until the user trusts the exact hook definition. The universal Plugin Directory is shared by ChatGPT and Codex, but individual capabilities may remain surface-specific.

The current public submission flow accepts skills-only and MCP-backed plugins. Lili does not need an MCP server: its value is local desktop presentation of Codex lifecycle state, and exposing local IPC through a remote server would expand the trust boundary without adding user value. A minimal setup and diagnostics skill gives the plugin a reviewable user workflow while Codex-only lifecycle hooks provide deterministic integration.

This design uses the following official contracts as of 2026-08-14:

- https://developers.openai.com/plugins/concepts/plugins
- https://developers.openai.com/plugins/build/plugins
- https://learn.chatgpt.com/docs/hooks
- https://developers.openai.com/plugins/deploy/connect-chatgpt
- https://developers.openai.com/plugins/deploy/submission
- https://developers.openai.com/plugins/app-guidelines
- https://developers.openai.com/plugins/guides/security-privacy

## Goals / Non-Goals

**Goals:**

- Use one Marketplace-managed plugin as the default Codex-facing install, update, trust, and removal unit.
- Keep hook execution local, observer-only, bounded, authenticated, and compatible with Lili's existing normalized event protocol.
- Make the package truthful and useful on every supported surface, with explicit disclosure that automatic lifecycle forwarding currently requires Codex hooks and the separately installed Lili desktop application.
- Produce evidence that can be reused directly in local Marketplace testing and the public plugin review submission.
- Preserve a recoverable migration path for users of the existing direct-config integration.

**Non-Goals:**

- Observe ChatGPT chat lifecycle events through undocumented APIs or imply that ChatGPT currently exposes Codex hooks.
- Bundle the Tauri desktop application into the plugin or replace native application distribution.
- Add an MCP server, OAuth connection, remote telemetry, cloud synchronization, custom plugin UI, or browser iframe.
- Modify user or project Codex configuration during normal plugin installation.
- Automatically trust plugin hooks or bypass Codex hook trust review.
- Promise approval, featured placement, or an OpenAI Verified badge; those remain OpenAI decisions outside the package contract.

## Decisions

### Use a skills-only public submission with Codex-specific hooks

The plugin contains one setup and diagnostics skill plus lifecycle hooks. The skill explains prerequisites, checks the installed Lili and Codex versions through supported commands, reports hook trust and delivery status, and gives safe remediation steps. It does not install software, rewrite Codex configuration, inspect credentials, or read private session storage without explicit user direction.

The package does not declare `.app.json`, `.mcp.json`, MCP tools, authentication, or UI resources. Its Marketplace copy states that installation is available from the shared directory, while automatic Session notifications are currently a Codex-only feature. This matches the official model where a universal plugin may contain surface-specific capabilities.

Alternatives considered:

- An MCP server cannot receive local Codex lifecycle events and would introduce hosting, authentication, retention, and policy obligations unrelated to the product.
- A hooks-only package lacks the reviewable workflow required by the current skills-only submission path and gives ChatGPT users no useful installed behavior.
- Claiming ChatGPT lifecycle support based only on universal discovery would be misleading and fail the accuracy standard.

### Package hooks and native forwarders as a self-contained bridge

Use this package layout:

```text
plugins/lili/
  .codex-plugin/plugin.json
  skills/lili-setup/SKILL.md
  hooks/hooks.json
  hooks/forward
  hooks/forward.ps1
  bin/<supported-target>/lili-hook[.exe]
  assets/
```

`hooks/hooks.json` uses `${PLUGIN_ROOT}` for all package paths, provides `commandWindows` for Windows, runs observer hooks asynchronously where the Codex event contract permits, and applies short timeouts. The POSIX and PowerShell launchers select only an exact packaged target from a closed operating-system and architecture matrix; they never resolve a forwarder from `PATH`, evaluate provider data as shell text, download code, or mutate Codex configuration.

The packaged `lili-hook` binaries are built from the same source and application version as the desktop release. macOS and Windows forwarders use the applicable release signing path. The plugin archive includes only the target binaries supported by the corresponding desktop release matrix, and unsupported hosts fail closed with a bounded local diagnostic.

`lili-hook` remains responsible for size limits, normalization, authenticated local delivery, replay protection, and owner-only spooling. The launchers only select and execute the correct binary with stdin preserved. They produce no model-visible output and never return an approval decision.

Alternatives considered:

- Resolving `lili-hook` from `PATH` permits path shadowing and makes installation sensitive to the user's shell environment.
- Hard-coding application bundle paths breaks unpacked releases, custom installation roots, and portable Linux distribution.
- Reimplementing forwarding in shell or PowerShell would duplicate security-sensitive parsing and IPC behavior.

### Replace direct notification configuration with plugin Stop coverage

The Marketplace path uses documented lifecycle hooks, including `Stop`, whose payload contains the session, turn, working directory, and last assistant message needed by the existing completion adapter. It does not add the top-level Codex `notify` setting, so plugin installation cannot conflict with another user's notification command.

The direct-config integration remains available only as a version-gated fallback when the installed Codex lacks plugin support or policy prevents Marketplace installation. CLI help, diagnostics, and documentation label it as legacy. The fallback retains its existing preview, conflict, backup, coexistence, and provenance semantics.

Alternatives considered:

- Continuing to configure both `notify` and plugin hooks would preserve unnecessary conflicts and duplicate ownership.
- Removing the direct-config implementation immediately would strand older Codex installations and prevent rollback.

### Migrate with overlap and deduplication instead of an event gap

Migration installs and enables the plugin, asks the user to review and trust its hooks, verifies one synthetic and one real lifecycle event, and only then removes Lili-owned direct configuration. During the overlap, existing stable event identities and reducer deduplication ensure one user-visible notification per lifecycle event. Unrelated hooks and notification commands remain untouched.

Plugin removal is delegated to the Plugin Directory or supported `codex plugin remove` command. Removing the plugin removes only its cached package and enabled state; it does not delete the desktop application, pet packages, action configuration, spool, or legacy configuration. `lili integrate uninstall` continues to remove only provenance-matched legacy entries.

Alternatives considered:

- Removing legacy integration before verification creates a silent notification gap if hooks remain untrusted or the package is incompatible.
- Letting plugin removal edit legacy files would mix ownership models and make rollback unsafe.

### Keep plugin, application, and protocol versions explicit

The workspace package version remains the only edited release version and is copied into the plugin manifest and forwarder metadata. The local IPC envelope retains its own schema version. Compatibility is accepted only when the plugin forwarder schema is within the desktop application's declared range; an incompatible pair spools the normalized event and exposes a remediation diagnostic instead of silently discarding it.

Marketplace refresh installs an immutable cached copy, so hook commands always resolve from the active `PLUGIN_ROOT`. Upgrade tests cover old-plugin/new-app and new-plugin/old-app pairs within the supported compatibility window, changed-hook trust invalidation, rollback to the prior package, and concurrent sessions during refresh.

Alternatives considered:

- Assuming plugin and desktop versions update atomically is incompatible with separate distribution channels.
- Coupling compatibility directly to equal semantic versions would force unnecessary lockstep upgrades.

### Treat review readiness as a release gate, not a documentation aspiration

A plugin release is `submission-ready` only when all install-surface metadata is final, public URLs are reachable, publisher identity matches the listing, policy text matches observed local data handling, assets pass format checks, and the package passes the official local install and evaluation flow. The listing does not include screenshots because the plugin has no custom UI.

The repository stores reproducible reviewer materials: starter prompts, at least five positive cases, at least three negative cases, fixture prerequisites, expected behavior, expected result shapes, release notes, country availability, and a dated compliance checklist. Negative cases cover unsupported ChatGPT lifecycle observation, requests to approve permissions, requests to reveal raw conversations or credentials, and unsupported hosts.

Before every submission, maintainers re-check current official documentation and record the review date because Marketplace rules are versioned external contracts. Automated checks reject placeholder publisher metadata, missing legal links, inaccurate surface claims, unapproved endorsement language, or drift between manifest, submission material, and runtime behavior.

Alternatives considered:

- Treating local installation as sufficient ignores identity, privacy, listing, test-case, and policy review requirements.
- Claiming OpenAI Verified status at initial submission would be inaccurate; verification is awarded separately based on trust, testing, usage, and sustained value.

## Risks / Trade-offs

- [The public submission portal may add or narrow rules for hook-bearing skills-only packages] → Run a portal preflight with the final package shape before declaring implementation complete, pin the observed requirements date, and stop publication rather than stripping the safety boundary.
- [A universal package containing native binaries is larger and receives stricter security review] → Ship only declared target artifacts, generate checksums and SBOM/license evidence, sign supported binaries, scan the final archive, and disclose why native local IPC is required.
- [Hook trust prevents events immediately after installation] → Make trust state visible in the setup skill and Lili diagnostics, require explicit review, and never bypass it automatically.
- [Separate plugin and desktop updates can create protocol skew] → Version the IPC contract independently, test an explicit compatibility window, spool recoverable events, and report actionable diagnostics.
- [Legacy and plugin hooks can overlap during migration] → Preserve stable event identities, verify deduplication under concurrency, and remove only provenance-owned legacy entries after plugin delivery succeeds.
- [ChatGPT users may assume universal discovery means ChatGPT lifecycle forwarding] → Repeat the Codex-only lifecycle boundary in short description, long description, setup skill, support documentation, and negative review tests.
- [Hook payloads may contain conversation content] → Reuse bounded normalization and redaction before storage, emit no hook stdout, collect no telemetry, and keep the privacy policy aligned with exact local retention.
- [Unsupported CPU or operating-system variants cannot run bundled forwarders] → Fail closed, provide a local diagnostic, and advertise only the tested platform matrix.

## Migration Plan

1. Add the plugin source tree, manifest validation, setup skill, hook configuration, platform launchers, and local repo Marketplace entry without changing the existing default documentation.
2. Extend release builds to produce signed target forwarders and a deterministic universal plugin archive whose version matches the desktop application.
3. Validate clean install, disabled and untrusted states, explicit trust, lifecycle delivery, offline spooling, upgrades, removal, and concurrent legacy overlap against the supported Codex matrix.
4. Update Lili diagnostics and documentation to recommend Plugin Directory installation and mark direct configuration as legacy/fallback.
5. Migrate existing users with install, trust, verify, provenance-aware legacy uninstall, and a final no-duplicate event check.
6. Complete publisher verification, public policy and support pages, reviewer fixtures, compliance review, and submission portal preflight before marking the artifact submission-ready.

Rollback disables or removes the plugin through supported plugin management, verifies the cached hook is no longer active, and restores the existing direct-config integration only through its reviewed plan flow. Plugin rollback never deletes Lili application data or user pet packages.
