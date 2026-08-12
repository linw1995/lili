{pkgs}: let
  inherit (pkgs.stdenv) isDarwin isLinux;
  rustVersion = "1.97.0";
  wasmBindgenVersion = "0.2.126";
  cargoTauriVersion = pkgs.cargo-tauri.version;
  trunkVersion = pkgs.trunk.version;
  nodeMajor = 24;
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
  cargoCrap = pkgs.rustPlatform.buildRustPackage rec {
    pname = "cargo-crap";
    version = "0.2.2";

    src = pkgs.fetchurl {
      url = "https://static.crates.io/crates/${pname}/${pname}-${version}.crate";
      name = "${pname}-${version}.tar.gz";
      hash = "sha256-Ej+k1P4Am2AXD8PVhrcrCdisA/0AAOI/8j2x0ULuOmY=";
    };

    cargoHash = "sha256-vzkGNzQrVOtfpGLniGTdPRQfwA9jn5elXhudrFC7w9g=";
    doCheck = false;
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
  dependencyClosure = packages: let
    expanded = pkgs.lib.unique (packages
      ++ builtins.concatMap (
        package:
          builtins.filter pkgs.lib.isDerivation (
            (package.buildInputs or []) ++ (package.propagatedBuildInputs or [])
          )
      )
      packages);
  in
    if builtins.length expanded == builtins.length packages
    then packages
    else dependencyClosure expanded;
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
  linuxRuntimeInputs = pkgs.lib.optionals isLinux [
    pkgs.glib
    pkgs.gtk3
    pkgs.libsoup_3
    pkgs.webkitgtk_4_1
  ];
  linuxBuildInputs = dependencyClosure (linuxRuntimeInputs
    ++ pkgs.lib.optionals isLinux [
      pkgs.libdatrie
      pkgs.libselinux
      pkgs.libsepol
      pkgs.libsysprof-capture
      pkgs.libthai
      pkgs.libxdmcp
      pkgs.util-linuxMinimal
    ]);
  linuxPkgConfig = pkgs.buildEnv {
    name = "lili-pkg-config";
    paths = linuxBuildInputs;
    pathsToLink = ["/lib/girepository-1.0" "/lib/pkgconfig" "/share"];
    ignoreCollisions = true;
  };
  darwinBuildInputs = pkgs.lib.optionals isDarwin [pkgs.darwin.libiconv];
  nativeBuildInputs = linuxRuntimeInputs ++ darwinBuildInputs;
  nativeEnv =
    pkgs.lib.optionalString isLinux ''
      export PKG_CONFIG_PATH="${linuxPkgConfig}/lib/pkgconfig:${linuxPkgConfig}/share/pkgconfig''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath linuxRuntimeInputs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
      export GI_TYPELIB_PATH="${linuxPkgConfig}/lib/girepository-1.0''${GI_TYPELIB_PATH:+:$GI_TYPELIB_PATH}"
      export XDG_DATA_DIRS="${linuxPkgConfig}/share''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
    ''
    + pkgs.lib.optionalString isDarwin ''
      export LIBRARY_PATH="${pkgs.lib.makeLibraryPath darwinBuildInputs}''${LIBRARY_PATH:+:$LIBRARY_PATH}"
      export CPATH="${pkgs.lib.makeSearchPath "include" darwinBuildInputs}''${CPATH:+:$CPATH}"
    '';
in
  assert pkgs.wasm-bindgen-cli_0_2_126.version == wasmBindgenVersion; {
    inherit
      cargoCrap
      darwinEnv
      cargoTauriVersion
      fuzzRustToolchain
      isDarwin
      isLinux
      nativeBuildInputs
      nativeEnv
      nodeMajor
      rustToolchain
      rustVersion
      trunk
      trunkVersion
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
      pkgs.cargo-about
      pkgs.cargo-deny
      pkgs.prek
      pkgs.python3
      pkgs.taplo
    ];

    coverageTools = [
      pkgs.grcov
      rustToolchain
    ];

    crapTools = [
      cargoCrap
      rustToolchain
    ];
  }
