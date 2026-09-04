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

On Linux, install the distribution's native Tauri development packages first. Debian and Ubuntu require `build-essential`, `file`, `libayatana-appindicator3-dev`, `libgtk-3-dev`, `librsvg2-dev`, `libssl-dev`, `libwebkit2gtk-4.1-dev`, `libxdo-dev`, `pkg-config`, and `xvfb`. The Flake deliberately does not replace the host compiler, glibc, GTK, or WebKitGTK: Linux release binaries target the runner's native system libraries.

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
- `flake.lock` pins Nix inputs and platform-neutral build tools. Native compilers, SDKs, and desktop libraries come from the target system.
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

Each native release archive contains the desktop and matching hook binary, fallback pet, release Web assets, configuration, build, and security documentation, action example, project license, reviewed third-party notices, and a SHA-256 file manifest. The native assembler also emits `release/forwarders/<target>/` with the exact `lili-hook` binary and a sidecar recording its Cargo-derived version, native `--version` result, SHA-256 digest, size, target, signature kind, verifier, and verification result.

The release workflow collects exactly the three published forwarder targets and assembles `lili-plugin-<version>.zip`. The plugin archive uses sorted paths, a fixed ZIP timestamp, normalized file modes, and DEFLATE level 9 in the locked aggregation environment so identical inputs produce identical bytes across runs while remaining within Marketplace size limits. Its external manifest records every entry digest, forwarder signature evidence, and the digest of the matching supply-chain report; a sibling SHA-256 file covers the archive itself. The supply-chain report records the non-development dependency closure, SPDX license expressions, registry checksums, the enforced license policy, and the current RustSec scan result including informational warnings. Final inspection rejects checksum or mode drift, missing signing evidence, vulnerabilities, secrets, private fixture markers, development paths, and undeclared network URLs.

To reproduce the aggregation locally, first place the three native forwarder artifacts under one directory using their target names, then run:

```text
nix run .#plugin-supply-chain -- \
  --output release/lili-plugin-0.1.0.supply-chain.json
nix run .#plugin-archive -- \
  --forwarders /absolute/path/to/forwarders \
  --output release/lili-plugin-0.1.0.zip \
  --supply-chain release/lili-plugin-0.1.0.supply-chain.json
nix run .#plugin-inspect -- \
  --archive /absolute/path/to/release/lili-plugin-0.1.0.zip \
  --manifest /absolute/path/to/release/lili-plugin-0.1.0.manifest.json \
  --checksum /absolute/path/to/release/lili-plugin-0.1.0.zip.sha256 \
  --supply-chain /absolute/path/to/release/lili-plugin-0.1.0.supply-chain.json
```

The archive is not a Marketplace submission candidate merely because assembly and inspection pass. Before portal submission, create a private evidence JSON that follows `marketplace/lili/submission-readiness.json` and binds every gate to the same archive SHA-256 and committed source revision. Evidence must include current timestamps, exact successful automation and packaged-acceptance runs for every declared target, unauthenticated URL checks, the matching verified publisher account, hashes for every reviewer material, a fresh review of every required official rule source, and a successful portal draft and scanner preflight with no unresolved restriction.

Run the final fail-closed gate from the clean source revision named by that evidence:

```text
nix run .#submission-ready -- \
  --evidence /absolute/path/to/submission-evidence.json \
  --archive /absolute/path/to/release/lili-plugin-0.1.0.zip \
  --manifest /absolute/path/to/release/lili-plugin-0.1.0.manifest.json \
  --checksum /absolute/path/to/release/lili-plugin-0.1.0.zip.sha256 \
  --supply-chain /absolute/path/to/release/lili-plugin-0.1.0.supply-chain.json
```

The command runs strict validation for every active OpenSpec change before validating the evidence and archive. Missing, stale, failed, mismatched, duplicated, or differently bound evidence returns a nonzero status. Portal account identifiers and draft identifiers belong in the private release evidence, not in the public plugin archive.

`nix run .#license-check` enforces the dependency license allowlist and verifies that `THIRD_PARTY_NOTICES.html` matches the locked workspace graph. Run `scripts/generate-third-party-notices.sh` after an accepted dependency update, review the resulting license texts, and commit the updated artifact with the lockfile change.

Platform signing remains an external trust operation. When Tauri or the native toolchain applies a verifiable identity, the platform build records `signed` only after `codesign --verify --strict` or `Get-AuthenticodeSignature` succeeds; otherwise a standard local bundle or forwarder records `platform-standard` with an explicit `unsigned-allowed` result. Linux records signature verification as not applicable and binds the ELF forwarder by format and SHA-256. Set `LILI_REQUIRE_SIGNED=1` in a protected release environment to reject an unsigned macOS application, macOS forwarder, or Windows forwarder.

## GitHub CD build

Following the same separation used by `linw1995/coco`, [CD](../.github/workflows/CD.yaml) is a thin trigger workflow and [Publish release](../.github/workflows/Publish-Release.yaml) owns the reusable build and publication implementation. Pushes to `master` and manual dispatches exercise the production build without publishing, while a pushed tag matching `v*` publishes a release. Publication refuses a ref that is not a tag or whose name does not exactly match `Cargo.toml`'s workspace version as `v<version>`.

[CI](../.github/workflows/CI.yaml) runs independently for pull requests, manual dispatches, and pushes to `master`. As in Coco, CI and CD do not wait on one another; CD reruns the production release assembly and its compatibility and manifest gates rather than reusing mutable CI binaries.

The GitHub CD workflow publishes the macOS desktop release only. It still uses Linux and Windows runners to build the native plugin forwarders required by the universal Marketplace plugin archive:

| Runner | CD role | Output |
| --- | --- | --- |
| `macos-14` | Desktop release | macOS application and DMG |
| `ubuntu-latest` | Plugin forwarder | `x86_64-unknown-linux-gnu/lili-hook` |
| `windows-latest` | Plugin forwarder | `x86_64-pc-windows-msvc/lili-hook.exe` |

The reusable workflow uses a native-runner matrix. The macOS job uses the repository's composite Nix setup action and calls the same `nix run .#build` release assembler used locally. Linux uses the Nix setup action to build only `lili-hook`; Windows uses a separate composite setup action backed by `nix/windows-toolchain.json` to build only `lili-hook.exe`. CI uses those same setup actions rather than maintaining another toolchain installation path.

The macOS job uploads its `.tar.gz` archive and `.sha256` checksum. The Linux and Windows jobs upload only their separate short-lived forwarder inputs. Only after the macOS archive and all three native forwarders succeed does the aggregation job build and upload the universal plugin ZIP, checksum, and manifest. The `publish` job then downloads the complete desktop and plugin set, rejects missing or unexpected files, verifies every checksum and ZIP structure, creates the GitHub Release for the existing tag, and attaches the macOS archive plus the plugin artifacts.

CD does not rewrite lockfiles or application versions. Prepare a release by updating and reviewing the canonical version and repeated downstream metadata, run the local checks, commit those changes, and then push the matching version tag. The workflow generates release notes from the repository history.

GitHub-hosted builds currently produce `platform-standard` archives. Platform signing requires protected signing credentials and must be added as a separate reviewed trust operation; the workflow must not claim an unsigned archive is signed.
