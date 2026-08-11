# Lili

Lili is a local desktop pet that renders Codex v2 pet packages, observes supported Codex session notifications, and can run user-configured native actions when the pet or a notification is activated.

The default pet package is loaded from `${CODEX_HOME}/pet/lili`, which normally resolves to `~/.codex/pet/lili`. Invalid or missing packages fall back to the embedded Lili asset.

## Development

The Nix flake is the supported command and toolchain boundary:

```text
nix run .#dev
nix run .#dev-web
nix run .#lint
nix run .#fuzz
nix flake check
```

`dev-web` is a fixture-only build. It cannot read local forwarding credentials, load arbitrary pet paths, mutate Codex configuration, execute actions, or access native process APIs.

## Operations and security

Read [Security and operations](docs/security-and-operations.md) before enabling Codex integration or interaction actions. It documents the trust boundary, retained local data, exact integration changes, action authority, backup behavior, and uninstall procedure.

The implementation intentionally supports documented `notify` and lifecycle-hook surfaces. It does not read Codex credentials, private databases, rollout logs, process memory, or private desktop and marketplace APIs.
