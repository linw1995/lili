{
  pkgs,
  toolchain,
}: let
  workspaceEnv = toolchain.darwinEnv + toolchain.wasmEnv;
  mkWorkspaceApp = {
    name,
    runtimeInputs ? toolchain.buildTools,
    text,
  }:
    pkgs.writeShellApplication {
      inherit name runtimeInputs;
      text = ''
        ${workspaceEnv}
        ${text}
      '';
    };
in {
  dev = mkWorkspaceApp {
    name = "dev";
    text = ''
      exec cargo tauri dev "$@" -- --locked
    '';
  };

  dev-web = mkWorkspaceApp {
    name = "dev-web";
    text = ''
      trunk build --locked --config Trunk.web.toml
      exec cargo run --locked --package lili-web -- "$@"
    '';
  };

  build = mkWorkspaceApp {
    name = "build";
    text = ''
      exec cargo tauri build "$@" -- --locked
    '';
  };

  build-app = mkWorkspaceApp {
    name = "build-app";
    text = ''
      exec cargo tauri build --bundles app "$@" -- --locked
    '';
  };

  build-css = mkWorkspaceApp {
    name = "build-css";
    runtimeInputs = [pkgs.nodejs_24];
    text = ''
      exec npm run build:css -- "$@"
    '';
  };

  format = mkWorkspaceApp {
    name = "format";
    runtimeInputs = [pkgs.alejandra toolchain.rustToolchain];
    text = ''
      alejandra flake.nix nix
      exec cargo fmt --all "$@"
    '';
  };

  format-check = mkWorkspaceApp {
    name = "format-check";
    runtimeInputs = [pkgs.alejandra toolchain.rustToolchain];
    text = ''
      alejandra --check flake.nix nix
      exec cargo fmt --all -- --check
    '';
  };

  lint = mkWorkspaceApp {
    name = "lint";
    runtimeInputs = [toolchain.rustToolchain];
    text = ''
      exec cargo clippy --locked --workspace --all-targets --all-features "$@" -- -D warnings
    '';
  };

  test = mkWorkspaceApp {
    name = "test";
    runtimeInputs = [toolchain.rustToolchain];
    text = ''
      test_tmp="$(mktemp -d /tmp/lili-tests.XXXXXX)"
      trap 'rm -rf -- "$test_tmp"' EXIT
      export TMPDIR="$test_tmp"
      cargo test --locked --workspace --all-targets
    '';
  };

  audit = mkWorkspaceApp {
    name = "audit";
    runtimeInputs = [pkgs.cargo-audit toolchain.rustToolchain];
    text = ''
      exec cargo audit
    '';
  };

  web-build = mkWorkspaceApp {
    name = "web-build";
    text = ''
      exec trunk build --locked --release --config Trunk.web.toml
    '';
  };

  spec-validate = mkWorkspaceApp {
    name = "spec-validate";
    runtimeInputs = [pkgs.openspec];
    text = ''
      exec openspec validate add-session-aware-desktop-pet --strict
    '';
  };

  workflow-lint = mkWorkspaceApp {
    name = "workflow-lint";
    runtimeInputs = [pkgs.actionlint];
    text = ''
      exec actionlint
    '';
  };

  fuzz = mkWorkspaceApp {
    name = "fuzz";
    runtimeInputs = [pkgs.cargo-fuzz toolchain.fuzzRustToolchain];
    text = ''
      if [[ $# -eq 0 ]]; then
        cargo fuzz list
        exit 0
      fi
      exec cargo fuzz run "$@"
    '';
  };

  fuzz-smoke = mkWorkspaceApp {
    name = "fuzz-smoke";
    runtimeInputs = [pkgs.cargo-fuzz toolchain.fuzzRustToolchain];
    text = ''
      corpus_root="$(mktemp -d)"
      trap 'rm -rf -- "$corpus_root"' EXIT
      for target in pet_manifest provider_payload forwarding_frame spool_record action_config interaction_context; do
        mkdir -p "$corpus_root/$target"
        mkdir -p "$corpus_root/artifacts/$target"
        cargo fuzz run "$target" "$corpus_root/$target" -- \
          -artifact_prefix="$corpus_root/artifacts/$target/" \
          -runs=64
      done
    '';
  };

  prek = mkWorkspaceApp {
    name = "prek";
    runtimeInputs = [pkgs.prek];
    text = ''
      exec prek run --all-files "$@"
    '';
  };

  e2e = mkWorkspaceApp {
    name = "e2e";
    runtimeInputs = toolchain.buildTools ++ [pkgs.playwright-test];
    text = ''
      export PLAYWRIGHT_BROWSERS_PATH="${pkgs.playwright-driver.browsers}"
      ${pkgs.lib.optionalString toolchain.isLinux ''
        export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
      ''}
      exec playwright test "$@"
    '';
  };

  desktop-smoke = mkWorkspaceApp {
    name = "desktop-smoke";
    text = ''
      trunk build --locked
      exec cargo run --locked --package lili -- --desktop-smoke
    '';
  };

  macos-acceptance = mkWorkspaceApp {
    name = "macos-acceptance";
    text =
      if toolchain.isDarwin
      then ''
        cargo tauri build --bundles app -- --locked
        cargo build --locked --release --package lili --bin lili-hook --bin lili-macos-acceptance
        exec target/release/lili-macos-acceptance \
          target/release/bundle/macos/Lili.app/Contents/MacOS/lili \
          target/release/lili-hook
      ''
      else ''
        echo "macOS acceptance requires macOS" >&2
        exit 2
      '';
  };

  linux-acceptance = mkWorkspaceApp {
    name = "linux-acceptance";
    runtimeInputs = toolchain.buildTools ++ pkgs.lib.optionals toolchain.isLinux [pkgs.xvfb-run];
    text =
      if toolchain.isLinux
      then ''
        cargo tauri build --bundles deb -- --locked
        cargo build --locked --release --package lili --bin lili-hook --bin lili-linux-acceptance
        bundle="$(find target/release/bundle/deb -maxdepth 1 -type f -name '*.deb' -print -quit)"
        test -n "$bundle"
        acceptance=(target/release/lili-linux-acceptance target/release/lili target/release/lili-hook "$bundle")
        if [[ "''${LILI_ACCEPTANCE_HEADLESS:-}" == "1" ]]; then
          exec xvfb-run -a "''${acceptance[@]}"
        fi
        exec "''${acceptance[@]}"
      ''
      else ''
        echo "Linux acceptance requires Linux" >&2
        exit 2
      '';
  };
}
