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
      pkgs = import inputs.nixpkgs {
        inherit system;
        overlays = [inputs.rust-overlay.overlays.default];
      };
      toolchain = import ./toolchain.nix {inherit pkgs;};
    in {
      packages = import ./apps.nix {inherit pkgs toolchain;};
      devShells = import ./dev-shells.nix {inherit pkgs toolchain;};
      checks = import ./checks.nix {inherit pkgs root supportedSystems toolchain;};
    }
  )
