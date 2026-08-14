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
        plugin-archive = mkApp "plugin-archive" "Build the deterministic universal Lili plugin";
        plugin-supply-chain = mkApp "plugin-supply-chain" "Generate plugin dependency, license, and vulnerability evidence";
        plugin-inspect = mkApp "plugin-inspect" "Inspect the final plugin release";
        marketplace-check = mkApp "marketplace-check" "Validate Marketplace materials and runtime boundaries";
        marketplace-roundtrip = mkApp "marketplace-roundtrip" "Test the final plugin archive through a local Marketplace";
        marketplace-trust = mkApp "marketplace-trust" "Validate plugin hook trust and invalidation";
        build-css = mkApp "build-css" "Build the Lili stylesheet";
        format = mkApp "format" "Format the Lili workspace";
        format-check = mkApp "format-check" "Check workspace formatting";
        lint = mkApp "lint" "Lint the Lili workspace";
        test = mkApp "test" "Run workspace tests";
        coverage = mkApp "coverage" "Generate native coverage reports";
        crap = mkApp "crap" "Generate and enforce the CRAP metric report";
        audit = mkApp "audit" "Audit Rust dependencies";
        license-check = mkApp "license-check" "Enforce dependency license policy";
        web-build = mkApp "web-build" "Build the release Web application";
        spec-validate = mkApp "spec-validate" "Validate the OpenSpec change strictly";
        workflow-lint = mkApp "workflow-lint" "Lint GitHub Actions workflows";
        codex-matrix = mkApp "codex-matrix" "Verify the supported Codex installation matrix";
        fuzz = mkApp "fuzz" "Run a bounded parser fuzz target";
        fuzz-smoke = mkApp "fuzz-smoke" "Run the bounded fuzz smoke corpus";
        prek = mkApp "prek" "Run repository hooks";
        e2e = mkApp "e2e" "Run browser end-to-end tests";
        desktop-smoke = mkApp "desktop-smoke" "Run the desktop smoke verifier";
        macos-acceptance = mkApp "macos-acceptance" "Run packaged macOS acceptance";
        linux-acceptance = mkApp "linux-acceptance" "Run supported Linux acceptance";
      };
    in {
      inherit apps packages;
      devShells = import ./dev-shells.nix {inherit pkgs toolchain;};
      checks = import ./checks.nix {inherit apps packages pkgs root supportedSystems toolchain;};
    }
  )
