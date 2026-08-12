{pkgs}: let
  inherit (pkgs.stdenv) isDarwin isLinux;
  rustVersion = "1.97.0";
  wasmBindgenVersion = "0.2.126";
  cargoTauriVersion = pkgs.cargo-tauri.version;
  trunkVersion = pkgs.trunk.version;
  nodeMajor = 24;
  # Oxalica's Linux toolchain propagates Nix GCC and its Cargo wrapper exports
  # Nix libsecret. Keep those implementation details out of native builds and
  # the processes that Cargo launches.
  systemLinkingRustToolchain = name: toolchain:
    pkgs.runCommand name {} ''
      mkdir -p "$out/bin"
      for path in ${toolchain}/*; do
        if [[ "$(basename "$path")" != bin && "$(basename "$path")" != nix-support ]]; then
          ln -s "$path" "$out/$(basename "$path")"
        fi
      done
      for binary in ${toolchain}/bin/*; do
        if [[ "$(basename "$binary")" != cargo ]]; then
          ln -s "$binary" "$out/bin/$(basename "$binary")"
        fi
      done
      ${
        if isLinux
        then ''
          cargo_entry=${toolchain}/bin/cargo
          if [[ -L "$cargo_entry" ]]; then
            cargo_binary="$(readlink -f "$cargo_entry")"
          else
            mapfile -t cargo_candidates < <(
              sed -n \
                -e 's|^exec "\(/nix/store/[^"]*/bin/cargo\)"[[:space:]].*$|\1|p' \
                -e 's|^exec \(/nix/store/[^[:space:]]*/bin/cargo\)[[:space:]].*$|\1|p' \
                "$cargo_entry"
            )
            if (( ''${#cargo_candidates[@]} != 1 )); then
              echo "expected exactly one Cargo binary in $cargo_entry" >&2
              exit 1
            fi
            cargo_binary="''${cargo_candidates[0]}"
          fi
          if [[ "$cargo_binary" != /nix/store/*/bin/cargo || ! -x "$cargo_binary" ]]; then
            echo "invalid Cargo binary: $cargo_binary" >&2
            exit 1
          fi
          cargo_magic="$(od -An -tx1 -N4 "$cargo_binary" | tr -d '[:space:]')"
          if [[ "$cargo_magic" != 7f454c46 ]]; then
            echo "Cargo target is not an ELF binary: $cargo_binary" >&2
            exit 1
          fi
          cat > "$out/bin/cargo" <<EOF
          #!${pkgs.runtimeShell}
          unset LD_LIBRARY_PATH
          exec "$cargo_binary" "\$@"
          EOF
          chmod +x "$out/bin/cargo"
        ''
        else ''
          ln -s ${toolchain}/bin/cargo "$out/bin/cargo"
        ''
      }
    '';
  rustToolchain = systemLinkingRustToolchain "lili-rust-${rustVersion}" (
    pkgs.rust-bin.stable.${rustVersion}.default.override {
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
    }
  );
  fuzzRustToolchain = systemLinkingRustToolchain "lili-fuzz-rust-2026-08-01" (
    pkgs.rust-bin.nightly."2026-08-01".minimal.override {
      extensions = ["rust-src"];
    }
  );
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
in
  assert pkgs.wasm-bindgen-cli_0_2_126.version == wasmBindgenVersion; {
    inherit
      cargoCrap
      darwinEnv
      cargoTauriVersion
      fuzzRustToolchain
      isDarwin
      isLinux
      nodeMajor
      rustToolchain
      rustVersion
      trunk
      trunkVersion
      wasmBindgenVersion
      ;

    mkDevShell = pkgs.mkShellNoCC;

    buildTools = [
      pkgs.binaryen
      pkgs.cargo-tauri
      pkgs.nodejs_24
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
