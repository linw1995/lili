# Build

Lili has two separate build paths: local development through the pinned Nix Flake and release delivery through GitHub CD. Application setup belongs in the [Configuration guide](configuration.md), not in either build path.

## Local development build

Nix Flakes are the supported local toolchain entry point on macOS and Linux:

```text
nix run .#dev
nix run .#dev-web
nix run .#lint
nix run .#fuzz
nix flake check
```

`nix run .#dev` starts the native Tauri application. `nix run .#dev-web` starts a fixture-only browser build; it cannot read local forwarding credentials, load arbitrary pet paths, mutate Codex configuration, execute actions, or access native process APIs.

Build a complete release for the current macOS or Linux host with:

```text
nix run .#build
```

The command runs the supported Codex matrix gate, creates the platform-standard Tauri bundles, and writes both an unpacked release and a compressed archive under `release/`. This local command does not publish either output.

### Linux desktop support

The Web application and Tauri desktop are supported on X11 and on Wayland compositors that implement the layer, activation, tray, transparency, and pointer behavior used by GTK/WebKit. Run `nix run .#linux-acceptance` inside a graphical session to validate the installed compositor and desktop environment.

Linux window managers remain authoritative. A compositor may ignore always-on-top hints, omit the legacy tray protocol, restrict programmatic window positioning, or render transparent windows differently. Lili treats these as compositor capabilities: session ingestion and the Web application remain available, while unavailable desktop affordances are reported by acceptance instead of bypassing compositor policy. Headless CI must provide a virtual X11 session and a tray host.

### Toolchain and version ownership

- `Cargo.toml` `workspace.package.version` owns the application release version and the required `v<version>` CD tag.
- `flake.lock` pins Nix inputs and system/build tools.
- `Cargo.lock` pins Rust dependencies.
- `package-lock.json` pins npm dependencies.

Application metadata must repeat the Cargo workspace version where required by a downstream tool. `nix flake check` rejects mismatches in `package.json` and `lili/tauri.conf.json`.

### Selective updates

Update one Flake input at a time and review the resulting lockfile diff:

```text
nix flake lock --update-input nixpkgs
nix flake check --no-build
```

Replace `nixpkgs` with the intended input name. Do not run broad lockfile updates as part of ordinary development or build commands.

Rust and npm dependency updates are separate operations. Use Cargo and npm commands that explicitly update their own lockfile, then verify all checks through the Flake.

Normal development and build applications use locked dependency resolution. `scripts/check-lockfiles.sh` can wrap a command and fail if it mutates any lockfile.

### Coverage and CRAP reports

`nix run .#coverage` runs the locked workspace test graph with LLVM coverage instrumentation and writes LCOV, Cobertura, HTML, and Markdown reports under `target/coverage/result/`. CI uploads the directory as the `coverage` artifact and submits `lcov.info` to Codecov using the repository policy in `codecov.yml`.

`nix run .#crap` consumes the same LCOV report, writes `target/coverage/result/crap.md`, and rejects production functions above the default CRAP threshold of 30. Generate coverage before running the CRAP gate. Verification-only platform acceptance binaries and build scripts are excluded from the metric.

### Release assembly

The archive contains the desktop and hook binaries, fallback pet, release Web assets, configuration, build, and security documentation, action example, project license, reviewed third-party notices, and a SHA-256 file manifest. The assembler rejects source test fixtures and files that contain the current development workspace path.

`nix run .#license-check` enforces the dependency license allowlist and verifies that `THIRD_PARTY_NOTICES.html` matches the locked workspace graph. Run `scripts/generate-third-party-notices.sh` after an accepted dependency update, review the resulting license texts, and commit the updated artifact with the lockfile change.

Platform signing remains an external trust operation. When Tauri receives a configured signing identity, the manifest records `signed`; otherwise a standard local bundle records `platform-standard`. Set `LILI_REQUIRE_SIGNED=1` in a protected release environment to reject an unsigned macOS archive.

## GitHub CD build

Following the same separation used by `linw1995/coco`, [CD](../.github/workflows/CD.yaml) is a thin trigger workflow and [Publish release](../.github/workflows/Publish-Release.yaml) owns the reusable build and publication implementation. Pushes to `master` exercise the production build without publishing, while a pushed tag matching `v*` publishes a release. Manual dispatch also builds without publishing unless **publish-release** is explicitly selected. Publication refuses a ref that is not a tag or whose name does not exactly match `Cargo.toml`'s workspace version as `v<version>`.

[CI](../.github/workflows/CI.yaml) runs independently for pull requests, manual dispatches, and pushes to `master`. As in Coco, CI and CD do not wait on one another; CD reruns the production release assembly and its compatibility and manifest gates rather than reusing mutable CI binaries.

The workflow builds independently on these GitHub-hosted runners:

| Runner | Release platform | Bundles |
| --- | --- | --- |
| `macos-14` | `arm64-apple-darwin` | macOS application and DMG |
| `ubuntu-latest` | `x86_64-unknown-linux-gnu` | Debian package and AppImage |
| `windows-latest` | `x86_64-pc-windows-msvc` | NSIS installer |

The reusable workflow uses a native-runner matrix. macOS and Linux share the repository's composite Nix setup action and call the same `nix run .#build` release assembler used locally. Windows uses a separate composite setup action backed by `nix/windows-toolchain.json` and calls `scripts/build-release.ps1`, which applies the same Codex compatibility, archive-content, manifest, and checksum gates. CI uses those same setup actions rather than maintaining another toolchain installation path.

Each platform job uploads its `.tar.gz` archive and `.sha256` checksum as a one-day workflow artifact. Only after all platform jobs succeed does the `publish` job download the complete set, reject missing or unexpected files, verify every checksum, create the GitHub Release for the existing tag, and attach every archive and checksum. Unlike Coco's intentionally partial multi-architecture container publication, Lili cannot publish a partial desktop release.

CD does not rewrite lockfiles or application versions. Prepare a release by updating and reviewing the canonical version and repeated downstream metadata, run the local checks, commit those changes, and then push the matching version tag. The workflow generates release notes from the repository history.

GitHub-hosted builds currently produce `platform-standard` archives. Platform signing requires protected signing credentials and must be added as a separate reviewed trust operation; the workflow must not claim an unsigned archive is signed.
