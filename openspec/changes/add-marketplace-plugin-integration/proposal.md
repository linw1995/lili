## Why

Lili's current Codex integration rewrites user-level configuration even though the supported plugin model now provides a first-class unit for discovery, installation, trust, updates, and removal across ChatGPT and Codex. Shipping a review-ready plugin makes the default integration safer and easier to operate while preserving the native desktop application as a separately installed local component.

## What Changes

- Add a universal-directory plugin package with a production manifest, a narrowly scoped setup and diagnostics skill, Codex lifecycle hooks, cross-platform hook launchers, listing assets, and legal metadata.
- Make the installed plugin the default Codex Session integration path; plugin hooks forward supported events to the local Lili application without editing `config.toml` or user `hooks.json`.
- State the surface boundary explicitly: the plugin can be discovered in ChatGPT and Codex, while lifecycle event forwarding is a Codex-only capability until ChatGPT exposes an equivalent supported hook surface.
- Keep `lili integrate` as a legacy migration, diagnostics, and unsupported-marketplace fallback path instead of the primary installer.
- Add safe coexistence and migration from direct configuration, including duplicate-event tolerance, provenance-aware cleanup, hook trust review, upgrade compatibility, and removal behavior.
- Add Marketplace readiness gates for package metadata, public policy and support URLs, privacy claims, reproducible review fixtures, positive and negative test cases, cross-platform packaged acceptance, and submission artifacts.
- Exclude an MCP server, remote account connection, custom plugin UI, advertising, commerce, and any claim of OpenAI endorsement or verification.

## Capabilities

### New Capabilities

- `marketplace-plugin-integration`: Packaging, installing, trusting, migrating, validating, and publishing Lili's local Codex Session bridge as a ChatGPT/Codex plugin that meets public Marketplace review requirements.

### Modified Capabilities

None.

## Impact

- Adds a plugin source tree containing `.codex-plugin/plugin.json`, `skills/`, `hooks/`, platform launchers, signed hook forwarders, and install-surface assets.
- Changes integration status and documentation so Marketplace installation is primary and direct Codex configuration management is explicitly legacy or fallback behavior.
- Extends release assembly and CI with plugin archives, version alignment, manifest validation, local marketplace round trips, hook-trust acceptance, and reviewer-ready evidence.
- Requires public publisher, website, support, privacy-policy, and terms-of-service metadata before a release can be marked submission-ready.
- Does not add a remote service, authentication flow, network data transfer, or new provider payload fields.
