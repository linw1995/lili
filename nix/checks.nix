{
  apps,
  packages,
  pkgs,
  root,
  supportedSystems,
  toolchain,
}: let
  cargo = builtins.fromTOML (builtins.readFile (root + /Cargo.toml));
  package = builtins.fromJSON (builtins.readFile (root + /package.json));
  tauri = builtins.fromJSON (builtins.readFile (root + /lili/tauri.conf.json));
  windowsToolchain = builtins.fromJSON (builtins.readFile (root + /nix/windows-toolchain.json));
  version = cargo.workspace.package.version;
  wasmBindgenDependency = cargo.workspace.dependencies."wasm-bindgen";
  requiredApps = [
    "dev"
    "dev-web"
    "build"
    "build-app"
    "plugin-archive"
    "marketplace-check"
    "build-css"
    "format"
    "format-check"
    "lint"
    "test"
    "coverage"
    "crap"
    "audit"
    "license-check"
    "web-build"
    "spec-validate"
    "workflow-lint"
    "codex-matrix"
    "fuzz"
    "fuzz-smoke"
    "prek"
    "e2e"
    "macos-acceptance"
    "linux-acceptance"
  ];
  missingApps = builtins.filter (name: !(builtins.hasAttr name apps)) requiredApps;
  nativeApps =
    [
      "dev"
      "build"
      "build-app"
      "lint"
      "test"
      "coverage"
      "codex-matrix"
      "desktop-smoke"
      "marketplace-check"
    ]
    ++ pkgs.lib.optionals toolchain.isDarwin ["macos-acceptance"]
    ++ pkgs.lib.optionals toolchain.isLinux ["linux-acceptance"];
  lightweightApps = builtins.filter (name: !(builtins.elem name nativeApps)) requiredApps;
  missingNativeEnvironments =
    builtins.filter (
      name: !(packages.${name}.nativeWorkspace or false)
    )
    nativeApps;
  unexpectedNativeEnvironments =
    builtins.filter (
      name: packages.${name}.nativeWorkspace or false
    )
    lightweightApps;
in {
  version-contract = assert package.version == version;
  assert tauri.version == version;
    pkgs.runCommand "lili-version-contract-${version}" {} ''
      touch "$out"
    '';

  output-contract = assert missingApps == [];
  assert supportedSystems == ["aarch64-darwin" "aarch64-linux" "x86_64-linux"];
    pkgs.runCommand "lili-output-contract" {} ''
      touch "$out"
    '';

  windows-toolchain-contract = assert windowsToolchain.rust == toolchain.rustVersion;
  assert windowsToolchain.node == toolchain.nodeMajor;
  assert windowsToolchain.tauriCli == toolchain.cargoTauriVersion;
  assert windowsToolchain.trunk == toolchain.trunkVersion;
  assert windowsToolchain.wasmBindgenCli == toolchain.wasmBindgenVersion;
    pkgs.runCommand "lili-windows-toolchain-contract" {} ''
      touch "$out"
    '';

  toolchain-contract = assert wasmBindgenDependency == "=${toolchain.wasmBindgenVersion}";
    pkgs.runCommand "lili-toolchain-contract" {
      nativeBuildInputs = toolchain.buildTools;
    } ''
      rustc --version | grep -F "rustc ${toolchain.rustVersion}"
      node --version | grep -E '^v24\.'
      wasm-bindgen --version | grep -F "wasm-bindgen ${toolchain.wasmBindgenVersion}"
      touch "$out"
    '';

  native-app-contract = assert missingNativeEnvironments == [];
  assert unexpectedNativeEnvironments == [];
    pkgs.runCommand "lili-native-app-contract" {} ''
      ${pkgs.lib.optionalString toolchain.isLinux ''
        test ! -e ${toolchain.rustToolchain}/nix-support
        grep -F 'unset LD_LIBRARY_PATH' ${toolchain.rustToolchain}/bin/cargo
        if grep -F 'export LD_LIBRARY_PATH' ${toolchain.rustToolchain}/bin/cargo; then
          echo "Cargo unexpectedly exports Nix runtime libraries to child processes" >&2
          exit 1
        fi
        ${pkgs.lib.concatMapStringsSep "\n" (name: ''
            script="${packages.${name}}/bin/${name}"
            if grep -E 'export (CC|CXX|LD|LIBRARY_PATH|CPATH|PKG_CONFIG_PATH|LD_LIBRARY_PATH|GI_TYPELIB_PATH|XDG_DATA_DIRS)=' "$script"; then
              echo "${name} unexpectedly overrides the native Linux toolchain" >&2
              exit 1
            fi
            if grep -E '/nix/store/[^/]+-(glibc|glib|gtk|webkitgtk|libsoup|libayatana-appindicator)' "$script"; then
              echo "${name} unexpectedly references Nix Linux runtime libraries" >&2
              exit 1
            fi
          '')
          nativeApps}
      ''}
      touch "$out"
    '';

  lockfile-contract =
    pkgs.runCommand "lili-lockfile-contract" {
      nativeBuildInputs = [pkgs.git pkgs.nodejs_24 toolchain.rustToolchain];
    } ''
      cp -R ${root} source
      chmod -R u+w source
      cd source
      bash ./scripts/check-lockfiles.sh cargo metadata --locked --no-deps --format-version 1
      touch "$out"
    '';
}
