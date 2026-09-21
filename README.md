# Lili

Lili is an extensible desktop pet app inspired by the pet experience in the ChatGPT desktop app. It supports compatible Pet v2 packages, Codex Session integration, and user-configured local actions.

Lili is named after my cat.

[![Lili pet and notification demo](https://linw1995.github.io/lili/media/lili-demo.gif)](https://linw1995.github.io/lili/)

[Website and interactive demo](https://linw1995.github.io/lili/) · [Build and deploy](docs/website.md)

## Make it your own

- **Desktop pet interactions.** Animated states, pointer-following gaze, and click, double-click, and drag reactions. A built-in tabby is ready to use.
- **Custom pets.** Install compatible Pet v2 packages and select a companion from the tray.
- **Session integration.** Connect supported Codex completion, attention, and failure events to pet states and notifications.
- **Custom actions.** Bind pet clicks, double clicks, and notification activation to your own executables with structured context.

The website is built for [repository GitHub Pages](docs/website.md#github-pages). To try the interactive demo locally:

```sh
nix develop --command npm run preview:site
```

Open `http://127.0.0.1:4173`. The entire website is a Leptos application compiled from Rust to WebAssembly. Its landing page and interactive demo share one component tree, reusing the desktop pet renderer, notification card, and preview controller. No native actions run. The animation above is recorded from the same demo during the Pages build and published with the website. See the [website guide](docs/website.md#checks-and-readme-media) to regenerate it locally.

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
