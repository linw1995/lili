{pkgs}: let
  inherit (pkgs.stdenv) isDarwin isLinux;
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
      [
        "wasm32-unknown-unknown"
        "x86_64-pc-windows-msvc"
      ]
      ++ pkgs.lib.optionals isDarwin ["x86_64-apple-darwin"];
  };
  fuzzRustToolchain = pkgs.rust-bin.nightly."2026-08-01".minimal.override {
    extensions = ["rust-src"];
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
  darwinEnv = pkgs.lib.optionalString isDarwin ''
    lili_macos_sdk="$(/usr/bin/xcrun --sdk macosx --show-sdk-path)"
    export SDKROOT="$lili_macos_sdk"
    export CC=/usr/bin/clang
    export CXX=/usr/bin/clang++
    export LD=/usr/bin/ld
    export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/usr/bin/clang
    export CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=/usr/bin/clang
    export CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS="-Lnative=$lili_macos_sdk/usr/lib/swift"
    export CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS="-Lnative=$lili_macos_sdk/usr/lib/swift"
  '';
  wasmEnv = ''
    export CC_wasm32_unknown_unknown="${pkgs.llvmPackages_21.clang-unwrapped}/bin/clang"
    export AR_wasm32_unknown_unknown="${pkgs.llvmPackages_21.llvm}/bin/llvm-ar"
  '';
in
  assert pkgs.wasm-bindgen-cli_0_2_126.version == wasmBindgenVersion; {
    inherit
      darwinEnv
      fuzzRustToolchain
      isDarwin
      isLinux
      rustToolchain
      rustVersion
      trunk
      wasmBindgenVersion
      wasmEnv
      ;

    mkDevShell =
      if isDarwin
      then pkgs.mkShellNoCC
      else pkgs.mkShell;

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

    linuxBuildInputs = pkgs.lib.optionals isLinux [
      pkgs.glib
      pkgs.gtk3
      pkgs.libsoup_3
      pkgs.webkitgtk_4_1
    ];

    darwinBuildInputs = pkgs.lib.optionals isDarwin [pkgs.darwin.libiconv];
  }
