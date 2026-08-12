{
  pkgs,
  toolchain,
}: let
  shellArgs = {
    packages = toolchain.buildTools ++ toolchain.qualityTools;
    buildInputs = toolchain.nativeBuildInputs;
    hardeningDisable = pkgs.lib.optionals toolchain.isDarwin ["zerocallusedregs"];
    shellHook = ''
      ${toolchain.darwinEnv}
      ${toolchain.wasmEnv}
      export LILI_WORKSPACE_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd -P)"
    '';
  };
in {
  default = toolchain.mkDevShell shellArgs;

  coverage = toolchain.mkDevShell (shellArgs
    // {
      packages = shellArgs.packages ++ toolchain.coverageTools;
    });

  crap = toolchain.mkDevShell (shellArgs
    // {
      packages = shellArgs.packages ++ toolchain.crapTools;
    });

  e2e = toolchain.mkDevShell (shellArgs
    // {
      packages = shellArgs.packages ++ [pkgs.playwright-test];
      shellHook =
        shellArgs.shellHook
        + ''
          export PLAYWRIGHT_BROWSERS_PATH="${pkgs.playwright-driver.browsers}"
          ${pkgs.lib.optionalString toolchain.isLinux ''
            export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
          ''}
        '';
    });
}
