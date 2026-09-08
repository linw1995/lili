# Verification

Verified on 2026-09-08.

- `nix run .#format-check` passed.
- `nix run .#lint` passed after the final action-only acceptance changes.
- `nix run .#test` passed for the full workspace, all targets, and the acceptance feature. Apple linker warnings about missing incremental archive members were non-fatal; the rebuild completed and every test passed.
- `cargo check -p lili --features acceptance --bin lili-macos-acceptance` passed.
- The action-only packaged acceptance passed with `actionContext=true` and `notificationState=true`. Its audit contained exactly one successful `open-session-context` execution and the retained timeout fixture, and the recorded context contained provider `codex` plus Session `01912f9d-3109-722d-a391-8e7b42ab1d31`.
- `nix run .#spec-validate` passed all five active changes.
- `git diff --check` passed.

The standard Marketplace macOS acceptance was attempted but stopped before application launch because the installed Codex version was `0.153.4` while the reviewed contract remains `0.147.0`. No compatibility bypass was used, and this result is not represented as a successful Marketplace acceptance.
