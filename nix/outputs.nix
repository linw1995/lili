{
  inputs,
  root,
}: let
  supportedSystems = [
    "aarch64-darwin"
    "aarch64-linux"
    "x86_64-linux"
  ];
in
  inputs.utils.lib.eachSystem supportedSystems (
    system: let
      playwrightOverlay = final: prev: {
        inherit (inputs.playwright.packages.${system}) playwright-driver playwright-test;
      };
      pkgs = import inputs.nixpkgs {
        inherit system;
        config.allowUnfree = true;
        overlays = [
          inputs.rust-overlay.overlays.default
          playwrightOverlay
        ];
      };
      toolchain = import ./toolchain.nix {inherit pkgs;};
      packages = import ./apps.nix {inherit pkgs toolchain;};
      mkApp = name: description: {
        type = "app";
        program = "${packages.${name}}/bin/${name}";
        meta = {inherit description;};
      };
      apps = {
        dev = mkApp "dev" "Run Lili in Tauri development mode";
        dev-web = mkApp "dev-web" "Run the fixture-only Lili web application";
        build = mkApp "build" "Build the complete Lili desktop release";
        build-app = mkApp "build-app" "Build the Lili desktop app bundle";
        build-css = mkApp "build-css" "Build the Lili stylesheet";
        format = mkApp "format" "Format the Lili workspace";
        lint = mkApp "lint" "Lint the Lili workspace";
        prek = mkApp "prek" "Run repository hooks";
        e2e = mkApp "e2e" "Run browser end-to-end tests";
      };
    in {
      inherit apps packages;
      devShells = import ./dev-shells.nix {inherit pkgs toolchain;};
      checks = import ./checks.nix {inherit pkgs root supportedSystems toolchain;};
    }
  )
