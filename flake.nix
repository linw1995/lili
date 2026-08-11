{
  description = "Lili session-aware desktop pet";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    playwright = {
      url = "github:pietdevries94/playwright-web-flake";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  nixConfig.warn-dirty = false;

  outputs = inputs:
    import ./nix/outputs.nix {
      inherit inputs;
      root = ./.;
    };
}
