{
  apps,
  pkgs,
  root,
  supportedSystems,
  toolchain,
}: let
  cargo = builtins.fromTOML (builtins.readFile (root + /Cargo.toml));
  package = builtins.fromJSON (builtins.readFile (root + /package.json));
  tauri = builtins.fromJSON (builtins.readFile (root + /lili/tauri.conf.json));
  version = cargo.workspace.package.version;
  wasmBindgenDependency = cargo.workspace.dependencies."wasm-bindgen";
  requiredApps = [
    "dev"
    "dev-web"
    "build"
    "build-app"
    "build-css"
    "format"
    "lint"
    "fuzz"
    "prek"
    "e2e"
    "macos-acceptance"
  ];
  missingApps = builtins.filter (name: !(builtins.hasAttr name apps)) requiredApps;
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

  toolchain-contract = assert wasmBindgenDependency == "=${toolchain.wasmBindgenVersion}";
    pkgs.runCommand "lili-toolchain-contract" {
      nativeBuildInputs = toolchain.buildTools;
    } ''
      rustc --version | grep -F "rustc ${toolchain.rustVersion}"
      node --version | grep -E '^v24\.'
      wasm-bindgen --version | grep -F "wasm-bindgen ${toolchain.wasmBindgenVersion}"
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
