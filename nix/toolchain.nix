{pkgs}: let
  inherit (pkgs.stdenv) isDarwin;
  rustVersion = "1.97.0";
  wasmBindgenVersion = "0.2.126";
  rustToolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
    extensions = [
      "clippy"
      "llvm-tools-preview"
      "rust-analyzer"
      "rust-src"
      "rustfmt"
    ];
    targets =
      ["wasm32-unknown-unknown"]
      ++ pkgs.lib.optionals isDarwin ["x86_64-apple-darwin"];
  };
  trunk = pkgs.writeShellApplication {
    name = "trunk";
    text = ''
      if [[ -v NO_COLOR ]]; then
        export NO_COLOR=true
      fi
      export TRUNK_OFFLINE=true
      exec ${pkgs.trunk}/bin/trunk "$@"
    '';
  };
in
  assert pkgs.wasm-bindgen-cli_0_2_126.version == wasmBindgenVersion; {
    inherit rustToolchain rustVersion trunk wasmBindgenVersion;

    buildTools = [
      pkgs.binaryen
      pkgs.cargo-tauri
      pkgs.nodejs_24
      pkgs.pkg-config
      rustToolchain
      pkgs.wasm-bindgen-cli_0_2_126
      trunk
    ];

    qualityTools = [
      pkgs.alejandra
      pkgs.prek
      pkgs.taplo
    ];
  }
