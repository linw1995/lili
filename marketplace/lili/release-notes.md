# Lili 0.1.0

Initial Marketplace package for setting up and diagnosing the separately installed Lili desktop companion.

## Included

- A guided setup and compatibility workflow for Lili desktop and Codex.
- Explicit trust guidance for packaged Codex lifecycle hooks.
- Local forwarding for supported Codex session lifecycle events after hook trust is granted.
- Diagnostics for plugin state, hook source, trust, versions, IPC compatibility, and the last accepted plugin event.
- Safe migration guidance from the legacy direct Codex configuration, including rollback and cleanup previews.
- Bounded local spooling when the desktop application is temporarily unavailable.

## Boundaries

- ChatGPT supports setup, compatibility, migration, and troubleshooting guidance only. ChatGPT lifecycle observation is not supported.
- The plugin does not install or update the Lili desktop application.
- The plugin does not approve permission requests, access credentials, expose an MCP server, or transfer session data to a remote service.
- Plugin installation does not imply hook trust. Users must review and explicitly trust the packaged hook definitions.

## Prerequisites

Install and start Lili desktop 0.1.0 or later, use a compatible Codex release, install and enable the plugin through a supported plugin workflow, explicitly trust its hooks, and confirm the setup diagnostics report compatible local delivery.
