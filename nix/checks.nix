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
  requiredApps = [
    "dev"
    "dev-web"
    "build"
    "build-app"
    "build-css"
    "format"
    "lint"
    "prek"
    "e2e"
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

  toolchain-contract =
    pkgs.runCommand "lili-toolchain-contract" {
      nativeBuildInputs = toolchain.buildTools;
    } ''
      rustc --version | grep -F "rustc ${toolchain.rustVersion}"
      node --version | grep -E '^v24\.'
      wasm-bindgen --version | grep -F "wasm-bindgen ${toolchain.wasmBindgenVersion}"
      touch "$out"
    '';
}
