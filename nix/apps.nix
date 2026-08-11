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

  lint = mkWorkspaceApp {
    name = "lint";
    runtimeInputs = [toolchain.rustToolchain];
    text = ''
      exec cargo clippy --locked --workspace --all-targets --all-features "$@" -- -D warnings
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
}
