{pkgs}: let
  inherit (pkgs.stdenv) isDarwin;
  rustVersion = "1.97.0";
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
in {
  inherit rustToolchain rustVersion;
}
