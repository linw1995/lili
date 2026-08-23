# Lili Privacy Policy

Effective date: 2026-08-14

This policy describes the Lili desktop application and Lili ChatGPT/Codex plugin maintained by Jade Lin. Lili is designed as a local desktop companion. It does not require a Lili account and does not operate a remote service that receives session data or product telemetry.

## Scope and surfaces

The plugin can be discovered on supported ChatGPT and Codex surfaces. Automatic lifecycle forwarding is available only through supported Codex hooks. On ChatGPT, the plugin provides setup, compatibility, migration, and troubleshooting guidance and does not observe the lifecycle of the conversation.

The Lili desktop application is installed separately. OpenAI, GitHub, the operating-system vendor, and distribution channels have their own privacy practices for their products and websites.

## Local data categories and purposes

Lili may process and retain the following data on the user's device:

| Category | Examples | Purpose |
| --- | --- | --- |
| Normalized session metadata | Provider, event, session and optional turn identifiers; lifecycle kind; occurrence time; bounded project basename; bounded redacted display summary | Present the pet state and notifications, order events, and deduplicate repeated delivery |
| Application state | Selected pet identifier, window placement, bounded session reducer state, and unread notifications in the local SQLite database | Restore the local desktop experience after restart |
| Offline delivery records | Normalized events and aggregate expired, limit, and malformed-drop counters in the local SQLite database | Recover bounded events while the desktop endpoint is unavailable |
| Integration metadata | Installed/enabled status, versions, hook source, IPC compatibility, last accepted plugin event metadata, legacy provenance, expected hashes, and backup paths | Diagnose compatibility, migrate safely, and remove only Lili-owned legacy entries |
| Local authentication material | Rotating instance identifier, endpoint address, message-authentication secret, and replay nonces | Authenticate the local forwarder to the current desktop instance |
| Pet and action configuration | Pet packages, action identifiers, filters, executable argv, working-directory policy, limits, and explicit environment additions | Render user-selected pets and run user-configured local actions |
| Bounded action audit | Action identifier, trigger, event identifier, timing, outcome, exit code, and output byte counts | Diagnose local action execution without retaining captured output |

Lili does not use these data for advertising, profiling, model training, or sale.

## Excluded data and minimization

The supported integration does not read Codex credentials, `auth.json`, private databases, rollout JSONL, conversation history, or process memory. It does not request or retain raw prompts, approval arguments, command text from hook payloads, inherited environment-secret values, or complete conversation exports.

A supported completion payload can include a last assistant message. Lili applies bounded normalization and redaction before presentation or storage. Users should still avoid placing secrets in content intended for display.

## Recipients and transfers

Lili and its plugin do not send session data, diagnostics, or telemetry to Jade Lin or another remote recipient. Plugin-to-desktop delivery uses an authenticated local endpoint or owner-only local spool.

Optional native actions receive a bounded local interaction context only after the user configures the executable and activates the matching interaction. That executable runs with the current operating-system user's authority and may have its own data practices. Lili does not control what a user-selected executable does after launch.

Visiting GitHub pages, downloading a release, opening a support issue, or using OpenAI products communicates with those third parties under their policies. Information the user voluntarily posts to a support issue is received by GitHub and the repository maintainers.

## Retention

- SQLite application state remains until it is replaced by newer bounded state or the user deletes the Lili application data.
- Unread notifications and bounded reducer metadata remain in application state for restart recovery.
- Offline spool records are bounded to 256 records, 4 MiB total, and 24 hours by default; older and excess records are dropped.
- Runtime forwarding credentials last only for the current desktop instance and are removed on orderly shutdown.
- The action audit is memory-only and bounded to recent entries; captured stdout and stderr content is not included in the audit.
- Legacy integration provenance and timestamped configuration backups remain until successful cleanup or manual removal.
- User-managed pet packages and action configuration remain until the user removes them.

## User controls and deletion

Users can stop processing by quitting Lili and disabling or removing the plugin through supported Codex plugin controls. Plugin removal does not delete the desktop application or local Lili data.

For legacy integration cleanup, run `lili integrate uninstall` and require `complete: true`. To remove persistent Lili data, quit Lili and delete the platform-native Lili application data directory described in the [security and operations guide](security-and-operations.md#local-data-layout). This removes the SQLite database, application-owned Pet packages, action configuration, and runtime files; it does not remove Codex configuration or backups. The [security and operations guide](security-and-operations.md#backup-reset-and-uninstall) contains the complete order and safety checks.

Because Lili has no account or remote session-data store, there is no separate server-side Lili profile to export or delete. Data voluntarily submitted to GitHub or another third party must be managed through that service.

## Security boundaries

Lili uses bounded schemas, owner-only local storage and IPC, rotating local credentials, message authentication, replay protection, atomic writes, path confinement, redaction, and provenance-aware cleanup. Plugin hooks are inactive until the user trusts the exact hook definition in Codex. Permission events are observation-only and never authorize a request.

No security control eliminates all risk. Local administrators, compromised user accounts, user-selected action executables, and third-party software can operate outside Lili's boundary. Users are responsible for reviewing plugin hooks and native actions before enabling them.

## Changes and contact

Material policy changes will update this file and its effective date before a corresponding release is represented as submission-ready. Questions and privacy requests may be opened through [Lili Support](support.md). Do not include secrets or private conversation content.
