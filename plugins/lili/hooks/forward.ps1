$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Fail-LiliLauncher {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Message,
        [Parameter(Mandatory = $true)]
        [int]$ExitCode
    )

    [Console]::Error.WriteLine($Message)
    exit $ExitCode
}

try {
    if ([string]::IsNullOrWhiteSpace($PSScriptRoot)) {
        Fail-LiliLauncher "Lili plugin launcher requires an absolute package path" 65
    }
    $resolvedPluginRoot = (Resolve-Path -LiteralPath (Join-Path -Path $PSScriptRoot -ChildPath "..")).ProviderPath
    if ([string]::IsNullOrWhiteSpace($env:PLUGIN_ROOT)) {
        Fail-LiliLauncher "Lili plugin root is unavailable" 65
    }
    $providedRoot = (Resolve-Path -LiteralPath $env:PLUGIN_ROOT).ProviderPath
} catch {
    Fail-LiliLauncher "Lili plugin root is invalid" 65
}

$trimCharacters = [char[]]@(
    [IO.Path]::DirectorySeparatorChar,
    [IO.Path]::AltDirectorySeparatorChar
)
$resolvedPluginRoot = $resolvedPluginRoot.TrimEnd($trimCharacters)
$providedRoot = $providedRoot.TrimEnd($trimCharacters)
if (-not [StringComparer]::OrdinalIgnoreCase.Equals($resolvedPluginRoot, $providedRoot)) {
    Fail-LiliLauncher "Lili plugin root does not match the active package" 65
}

if ([string]::IsNullOrWhiteSpace($env:PLUGIN_DATA) -or
    -not [IO.Path]::IsPathRooted($env:PLUGIN_DATA)) {
    Fail-LiliLauncher "Lili plugin data root is unavailable" 65
}
$pluginDataName = Split-Path -Leaf $env:PLUGIN_DATA.TrimEnd($trimCharacters)
if ($pluginDataName -notmatch '^lili-[A-Za-z0-9._-]{1,123}$') {
    Fail-LiliLauncher "Lili plugin data root does not identify the active package" 65
}
Remove-Item Env:LILI_PLUGIN_CODEX_HOME -ErrorAction SilentlyContinue
if (-not [string]::IsNullOrWhiteSpace($env:CODEX_HOME) -and
    [IO.Path]::IsPathRooted($env:CODEX_HOME)) {
    $env:LILI_PLUGIN_CODEX_HOME = $env:CODEX_HOME
}

$hostIsWindows = [Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [Runtime.InteropServices.OSPlatform]::Windows
)
$architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if (-not $hostIsWindows -or $architecture -ne [Runtime.InteropServices.Architecture]::X64) {
    Fail-LiliLauncher "Lili plugin does not support this host" 64
}

$forwarder = Join-Path -Path $resolvedPluginRoot -ChildPath "bin\x86_64-pc-windows-msvc\lili-hook.exe"
try {
    $forwarderItem = Get-Item -LiteralPath $forwarder -Force
} catch {
    Fail-LiliLauncher "Lili plugin forwarder is missing or invalid" 66
}
if (-not $forwarderItem.PSIsContainer -and
    ($forwarderItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) {
    $forwarderPath = $forwarderItem.FullName
} else {
    Fail-LiliLauncher "Lili plugin forwarder is missing or invalid" 66
}

$utf8 = [Text.UTF8Encoding]::new($false)
$reader = [IO.StreamReader]::new([Console]::OpenStandardInput(), $utf8, $false)
try {
    $payload = $reader.ReadToEnd()
} finally {
    $reader.Dispose()
}

$startInfo = [Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $forwarderPath
$startInfo.Arguments = '--integration-id lili-session-v1 --plugin-hook --json-stdin'
$startInfo.UseShellExecute = $false
$startInfo.RedirectStandardInput = $true
if ($null -ne $startInfo.PSObject.Properties["StandardInputEncoding"]) {
    $startInfo.StandardInputEncoding = $utf8
} else {
    # Windows PowerShell uses Console.InputEncoding for redirected process input.
    [Console]::InputEncoding = $utf8
}

$forwarderProcess = [Diagnostics.Process]::new()
$forwarderProcess.StartInfo = $startInfo
try {
    if (-not $forwarderProcess.Start()) {
        Fail-LiliLauncher "Lili plugin forwarder could not start" 67
    }
    $forwarderProcess.StandardInput.Write($payload)
    # The native forwarder reads to EOF before parsing the event.
    $forwarderProcess.StandardInput.Close()
    $forwarderProcess.WaitForExit()
    $forwarderExitCode = $forwarderProcess.ExitCode
} finally {
    $forwarderProcess.Dispose()
}
exit $forwarderExitCode
