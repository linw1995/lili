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

$isWindows = [Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [Runtime.InteropServices.OSPlatform]::Windows
)
$architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if (-not $isWindows -or $architecture -ne [Runtime.InteropServices.Architecture]::X64) {
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

$startInfo = [Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $forwarderPath
$startInfo.Arguments = "--integration-id lili-session-v1 --plugin-hook --json-stdin"
$startInfo.UseShellExecute = $false
$startInfo.RedirectStandardInput = $true
$process = [Diagnostics.Process]::new()
$process.StartInfo = $startInfo
try {
    if (-not $process.Start()) {
        Fail-LiliLauncher "Lili plugin forwarder could not start" 67
    }
    [Console]::OpenStandardInput().CopyTo($process.StandardInput.BaseStream)
    $process.StandardInput.Close()
    $process.WaitForExit()
    $exitCode = $process.ExitCode
} catch {
    Fail-LiliLauncher "Lili plugin forwarder could not complete" 67
} finally {
    $process.Dispose()
}
exit $exitCode
