# Lili

Lili is a local desktop pet that renders Codex v2 pet packages, observes supported Codex session notifications, and can run user-configured native actions when the pet or a notification is activated.

Lili is named after my cat.

Optional Pet v2 packages are discovered under Lili's platform-native application data directory at `pets/<pet-id>`. Lili includes an embedded default, so no external package is required; invalid application-owned packages are skipped. The desktop runtime does not scan `CODEX_HOME`.

## Configuration

Follow the [Configuration guide](docs/configuration.md) to launch an unpacked release, install a compatible pet, review and enable Session integration, configure pet and notification actions, verify the result, and uninstall safely.

## Build

See the [Build guide](docs/build.md) for the separate local development and GitHub CD build paths, toolchain ownership, checks, and release artifacts.

## Operations and security

Read [Security and operations](docs/security-and-operations.md) before enabling Codex integration or interaction actions. It documents the trust boundary, retained local data, exact integration changes, action authority, backup behavior, and uninstall procedure. The step-by-step setup remains in the [Configuration guide](docs/configuration.md).

The implementation intentionally supports documented `notify` and lifecycle-hook surfaces. It does not read Codex credentials, private databases, rollout logs, process memory, or private desktop and marketplace APIs.

## Plugin and policies

Read the [plugin overview](docs/marketplace.md), [support guide](docs/support.md), [privacy policy](docs/privacy-policy.md), and [terms of service](docs/terms-of-service.md) for the Marketplace surface and its local-data boundaries.

## License

Lili is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE). Licenses for distributed Rust dependencies are listed in `THIRD_PARTY_NOTICES.html`.
