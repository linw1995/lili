# Lili

Lili is a local desktop companion for supported Codex session lifecycle events. The separately installed desktop application presents a pet, local lifecycle notifications, and optional user-configured native actions.

The Lili plugin is discoverable through supported ChatGPT and Codex plugin surfaces. In Codex, explicitly trusted plugin hooks can forward supported lifecycle events to the local desktop application. In ChatGPT, the plugin provides setup, compatibility, migration, and troubleshooting guidance only. It does not observe ChatGPT conversation lifecycle events.

## Requirements

- A separately installed compatible Lili desktop release.
- Codex `0.147.0` for the currently reviewed lifecycle-hook integration.
- Lili plugin and desktop versions within `>=0.1.0,<0.2.0`.
- Explicit user review and trust of the exact plugin hook definitions.

The plugin does not install the desktop application, approve or deny permission requests, expose an MCP server, read Codex credentials, or transfer session data to a remote Lili service.

## Local operation

Supported hook payloads are normalized before local delivery. Lili keeps bounded identifiers, lifecycle state, timestamps, project labels, and display-safe summaries needed for presentation and recovery. Raw prompts, credentials, approval arguments, process memory, private Codex databases, rollout logs, and complete conversation history are outside the supported integration boundary.

The desktop application stores structured state, evidence, and offline events in a local SQLite database under Lili's platform application data directory. Pet assets and user action configuration are stored in Lili-owned files. Neither the desktop runtime nor the forwarder uses `CODEX_HOME`; only an explicitly invoked integration command inspects or updates Codex configuration.

When the desktop process is unavailable, the local forwarder can retain bounded owner-only spool records for later recovery. Lili has no product telemetry or remote session-data recipient. Optional native actions run only when configured by the user and have the operating-system authority of the selected executable.

## Resources

- [Install and migration guide](configuration.md)
- [Support](support.md)
- [Privacy policy](privacy-policy.md)
- [Terms of service](terms-of-service.md)
- [Security and operations](security-and-operations.md)
- [Source and releases](https://github.com/linw1995/lili)
