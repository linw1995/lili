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
      coverage_script="${packages.coverage}/bin/coverage"
      ${pkgs.lib.optionalString toolchain.isLinux ''
        grep -F 'export PKG_CONFIG_PATH=' "$coverage_script"
        grep -F 'export LD_LIBRARY_PATH=' "$coverage_script"
        grep -F 'export GI_TYPELIB_PATH=' "$coverage_script"
        grep -F 'export XDG_DATA_DIRS=' "$coverage_script"
        grep -F '${pkgs.glib}' "$coverage_script"
        grep -F '${pkgs.gtk3}' "$coverage_script"
        grep -F '${pkgs.libdatrie}' "$coverage_script"
        grep -F '${pkgs.libselinux}' "$coverage_script"
        grep -F '${pkgs.libsepol}' "$coverage_script"
        grep -F '${pkgs.libsysprof-capture}' "$coverage_script"
        grep -F '${pkgs.libthai}' "$coverage_script"
        grep -F '${pkgs.libxdmcp}' "$coverage_script"
        grep -F '${pkgs.pango}' "$coverage_script"
        grep -F '${pkgs.libsoup_3}' "$coverage_script"
        grep -F '${pkgs.util-linuxMinimal}' "$coverage_script"
        grep -F '${pkgs.webkitgtk_4_1}' "$coverage_script"
        if grep -F 'webkitgtk' "${packages.crap}/bin/crap"; then
          echo "lightweight CRAP app unexpectedly references WebKitGTK" >&2
          exit 1
        fi
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
