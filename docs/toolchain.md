# Toolchain and version ownership

Lili uses Nix Flakes as the only supported toolchain entry point.

## Linux desktop support

The Web application and Tauri desktop are supported on X11 and on Wayland compositors that implement the layer, activation, tray, transparency, and pointer behavior used by GTK/WebKit. Run `nix run .#linux-acceptance` inside a graphical session to validate the installed compositor and desktop environment.

Linux window managers remain authoritative. A compositor may ignore always-on-top hints, omit the legacy tray protocol, restrict programmatic window positioning, or render transparent windows differently. Lili treats these as compositor capabilities: session ingestion and the Web application remain available, while unavailable desktop affordances are reported by acceptance instead of bypassing compositor policy. Headless CI must provide a virtual X11 session and a tray host.

- `Cargo.toml` `workspace.package.version` owns the application release version.
- `flake.lock` pins Nix inputs and system/build tools.
- `Cargo.lock` pins Rust dependencies.
- `package-lock.json` pins npm dependencies.

Application metadata must repeat the Cargo workspace version where required by a downstream tool. `nix flake check` rejects mismatches in `package.json` and `lili/tauri.conf.json`.

## Selective updates

Update one Flake input at a time and review the resulting lockfile diff:

```text
nix flake lock --update-input nixpkgs
nix flake check --no-build
```

Replace `nixpkgs` with the intended input name. Do not run broad lockfile updates as part of ordinary development or build commands.

Rust and npm dependency updates are separate operations. Use Cargo and npm commands that explicitly update their own lockfile, then verify all checks through the Flake.

Normal development and build applications use locked dependency resolution. `scripts/check-lockfiles.sh` can wrap a command and fail if it mutates any lockfile.

## Release assembly

`nix run .#build` runs the supported Codex matrix gate, creates the platform-standard Tauri bundles, and assembles a versioned archive under `release/`. The archive contains the desktop and hook binaries, fallback pet, release Web assets, integration and security documentation, action example, project license, reviewed third-party notices, and a SHA-256 file manifest. The assembler rejects source test fixtures and files that contain the current development workspace path.

`nix run .#license-check` enforces the dependency license allowlist and verifies that `THIRD_PARTY_NOTICES.html` matches the locked workspace graph. Run `scripts/generate-third-party-notices.sh` after an accepted dependency update, review the resulting license texts, and commit the updated artifact with the lockfile change.

Platform signing remains an external trust operation. When Tauri receives a configured signing identity, the manifest records `signed`; otherwise a standard local bundle records `platform-standard`. Set `LILI_REQUIRE_SIGNED=1` in a protected release environment to reject an unsigned macOS archive.
