# Lili

Lili is a local desktop pet that renders Codex v2 pet packages, observes supported Codex session notifications, and can run user-configured native actions when the pet or a notification is activated.

Lili is named after my cat.

Optional pet packages are discovered under `${CODEX_HOME}/pets/<pet-id>`. Lili includes an embedded default, so no external pet package is required; invalid external packages are skipped.

## Configuration

Follow the [Configuration guide](docs/configuration.md) to launch an unpacked release, install a compatible pet, review and enable Session integration, configure pet and notification actions, verify the result, and uninstall safely.

## Build

See the [Build guide](docs/build.md) for the separate local development and GitHub CD build paths, toolchain ownership, checks, and release artifacts.

## Operations and security

Read [Security and operations](docs/security-and-operations.md) before enabling Codex integration or interaction actions. It documents the trust boundary, retained local data, exact integration changes, action authority, backup behavior, and uninstall procedure. The step-by-step setup remains in the [Configuration guide](docs/configuration.md).

The implementation intentionally supports documented `notify` and lifecycle-hook surfaces. It does not read Codex credentials, private databases, rollout logs, process memory, or private desktop and marketplace APIs.

## License

Lili is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE). Licenses for distributed Rust dependencies are listed in `THIRD_PARTY_NOTICES.html`.
