param([string]$CodexBinary)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$workspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot "../..")).ProviderPath
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ("lili launcher " + [Guid]::NewGuid().ToString("N"))
$pluginRoot = Join-Path $fixtureRoot "plugin with spaces"
$captureFile = Join-Path $fixtureRoot "captured-input.txt"
$codexRoot = Join-Path $fixtureRoot "host-data"
$pluginData = Join-Path $codexRoot "plugins/data/lili-lili-local"
$payload = '{"hook_event_name":"SessionStart","session_id":"launcher-test","cwd":"path with spaces"}'

function Invoke-DeclaredHook {
    param([string]$DeclaredRoot = $pluginRoot)

    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = Join-Path $env:SystemRoot "System32/cmd.exe"
    $startInfo.Arguments = '/C "' + $handler.commandWindows + '"'
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.EnvironmentVariables.Clear()
    $environment = @{
        PATH = $env:PATH
        SystemRoot = $env:SystemRoot
        LOCALAPPDATA = $fixtureRoot
        TEMP = $fixtureRoot
        TMP = $fixtureRoot
        HOME = $fixtureRoot
        CODEX_HOME = $codexRoot
        PLUGIN_ROOT = $DeclaredRoot
        PLUGIN_DATA = $pluginData
        LILI_TEST_CAPTURE = $captureFile
    }
    foreach ($entry in $environment.GetEnumerator()) {
        $startInfo.EnvironmentVariables[$entry.Key] = $entry.Value
    }

    $hookProcess = New-Object Diagnostics.Process
    $hookProcess.StartInfo = $startInfo
    try {
        [void]$hookProcess.Start()
        $stdout = $hookProcess.StandardOutput.ReadToEndAsync()
        $stderr = $hookProcess.StandardError.ReadToEndAsync()
        $hookProcess.StandardInput.Write($payload)
        $hookProcess.StandardInput.Close()
        if (-not $hookProcess.WaitForExit(30000)) {
            & (Join-Path $env:SystemRoot "System32/taskkill.exe") /PID $hookProcess.Id /T /F | Out-Null
            [void]$hookProcess.WaitForExit(5000)
            throw "The declared Windows hook command timed out: stdout=$($stdout.GetAwaiter().GetResult()); stderr=$($stderr.GetAwaiter().GetResult()); captured=$(Test-Path -LiteralPath $captureFile)"
        }
        return @{
            ExitCode = $hookProcess.ExitCode
            Stdout = $stdout.GetAwaiter().GetResult()
            Stderr = $stderr.GetAwaiter().GetResult()
        }
    } finally {
        $hookProcess.Dispose()
    }
}

try {
    New-Item -ItemType Directory -Path $pluginRoot, $pluginData -Force | Out-Null
    Copy-Item (Join-Path $workspaceRoot "plugins/lili/*") $pluginRoot -Recurse -Force
    $forwarderRoot = Join-Path $pluginRoot "bin/x86_64-pc-windows-msvc"
    New-Item -ItemType Directory -Path $forwarderRoot -Force | Out-Null
    $fixtureSource = @'
using System;
using System.IO;
using System.Text;

public static class LauncherFixture
{
    public static int Main(string[] arguments)
    {
        if (String.Join(" ", arguments) != "--integration-id lili-session-v1 --plugin-hook --json-stdin")
            return 91;
        Console.InputEncoding = new UTF8Encoding(false);
        File.WriteAllText(@"CAPTURE_PATH", Console.In.ReadToEnd());
        return 0;
    }
}
'@
    $fixtureSource = $fixtureSource.Replace("CAPTURE_PATH", $captureFile.Replace('"', '""'))
    Add-Type -OutputAssembly (Join-Path $forwarderRoot "lili-hook.exe") -OutputType ConsoleApplication -TypeDefinition $fixtureSource

    $hooks = Get-Content (Join-Path $workspaceRoot "plugins/lili/hooks/hooks.json") -Raw | ConvertFrom-Json
    $handler = $hooks.hooks.SessionStart[0].hooks[0]
    $result = Invoke-DeclaredHook
    if ($result.ExitCode -ne 0 -or $result.Stdout -ne "" -or $result.Stderr -ne "") {
        throw "The declared Windows hook failed: exit=$($result.ExitCode); stdout=$($result.Stdout); stderr=$($result.Stderr)"
    }
    $captured = [IO.File]::ReadAllText($captureFile).TrimEnd([char[]]"`r`n")
    if ($captured -ne $payload) {
        throw "The Windows launcher did not preserve the hook payload"
    }
    Write-Output "Windows hook launcher passed"

    if ($CodexBinary) {
        $pythonSource = @'
import json
import os
from pathlib import Path
import shutil
import sys

workspace, executable, fixture, plugin, host = map(Path, sys.argv[1:])
sys.path.insert(0, str(workspace / "scripts"))
from test_local_marketplace import CodexRunner
from test_hook_trust import dispatch_installed_plugin_hook

catalog = fixture / "catalog"
shutil.copytree(workspace / "marketplace" / "local", catalog)
staged = catalog / "plugins" / "lili"
shutil.copytree(plugin, staged)
error_path = fixture / "hook-stderr.txt"
hooks_path = staged / "hooks" / "hooks.json"
hooks = json.loads(hooks_path.read_text(encoding="utf-8"))
for groups in hooks["hooks"].values():
    groups[0]["hooks"][0]["commandWindows"] += f' 2> "{error_path}"'
hooks_path.write_text(json.dumps(hooks), encoding="utf-8")

runner = CodexRunner(executable.resolve(), host.resolve(), "0.147.0")
runner.environment.update({"SystemRoot": os.environ["SystemRoot"]})
runner.verify_version()
runner.json(["plugin", "marketplace", "add", str(catalog)])
installed = runner.json(["plugin", "add", "lili@lili-local"])
try:
    result = dispatch_installed_plugin_hook(
        workspace, executable, host, Path(installed["installedPath"]), fixture
    )
    print(json.dumps(result))
finally:
    if error_path.exists():
        print(error_path.read_text(encoding="utf-8", errors="replace"), file=sys.stderr)
'@
        $pythonScript = Join-Path $fixtureRoot "dispatch.py"
        [IO.File]::WriteAllText($pythonScript, $pythonSource)
        python $pythonScript $workspaceRoot $CodexBinary $fixtureRoot $pluginRoot $codexRoot
        if ($LASTEXITCODE -ne 0) {
            throw "Installed Windows hook dispatch failed"
        }
    }
} finally {
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}
