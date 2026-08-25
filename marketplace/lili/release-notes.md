# Lili 0.1.0

Initial Marketplace package for setting up and diagnosing the separately installed Lili desktop companion.

## Included

- A guided setup and compatibility workflow for Lili desktop and Codex.
- Explicit trust guidance for packaged Codex lifecycle hooks.
- Local forwarding for supported Codex session lifecycle events after hook trust is granted.
- Diagnostics for plugin state, hook source, trust, versions, IPC compatibility, and the last accepted plugin event.
- Safe migration guidance from the legacy direct Codex configuration, including rollback and cleanup previews.
- Bounded local spooling when the desktop application is temporarily unavailable.

## Breaking storage change

- Application metadata, one latest state projection per Session, latest plugin evidence, and unconsumed offline events now live in a versioned SQLite database under Lili's platform-native application data directory. Event history and the in-memory deduplication cache are not persisted.
- Pet assets remain in the Lili-owned `pets/` file tree; user action configuration remains in the Lili-owned `config/actions.toml`; runtime credentials remain owner-only instance files.
- The desktop runtime and Hook no longer use `CODEX_HOME` for storage or forwarding.
- Existing `${CODEX_HOME}/lili`, `${CODEX_HOME}/pets`, and `${CODEX_HOME}/pet` data is intentionally ignored. It is not migrated, deleted, or backward-compatible.
- Users must explicitly rerun the supported integration or Plugin Directory workflow if a Hook requires reconfiguration.

## Boundaries

- ChatGPT supports setup, compatibility, migration, and troubleshooting guidance only. ChatGPT lifecycle observation is not supported.
- The plugin does not install or update the Lili desktop application.
- The plugin does not approve permission requests, access credentials, expose an MCP server, or transfer session data to a remote service.
- Plugin installation does not imply hook trust. Users must review and explicitly trust the packaged hook definitions.

## Prerequisites

Install and start Lili desktop 0.1.0 or later, use a compatible Codex release, install and enable the plugin through a supported plugin workflow, explicitly trust its hooks, and confirm the setup diagnostics report compatible local delivery.
