# Toolchain and version ownership

Lili uses Nix Flakes as the only supported toolchain entry point.

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
