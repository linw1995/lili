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
        if (-not $hookProcess.WaitForExit(10000)) {
            $hookProcess.Kill()
            throw "The declared Windows hook command timed out"
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
    Copy-Item (Join-Path $workspaceRoot "plugins/lili/hooks") $pluginRoot -Recurse
    $forwarderRoot = Join-Path $pluginRoot "bin/x86_64-pc-windows-msvc"
    New-Item -ItemType Directory -Path $forwarderRoot -Force | Out-Null
    Add-Type -OutputAssembly (Join-Path $forwarderRoot "lili-hook.exe") -OutputType ConsoleApplication -TypeDefinition @'
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
        File.WriteAllText(Environment.GetEnvironmentVariable("LILI_TEST_CAPTURE"), Console.In.ReadToEnd());
        return 0;
    }
}
'@

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
} finally {
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}
