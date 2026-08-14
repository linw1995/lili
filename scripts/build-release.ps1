$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

$workspace = (Get-Location).Path
$metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
$version = ($metadata.packages | Where-Object { $_.name -eq "lili" }).version
$buildTarget = Join-Path $workspace "target/release-build"
$env:CARGO_TARGET_DIR = $buildTarget
$platform = "x86_64-pc-windows-msvc"

cargo build --locked --release --package lili --features release-tools --bin lili-hook --bin lili-codex-matrix
& "$buildTarget/release/lili-codex-matrix.exe" `
    "$buildTarget/release/lili-hook.exe" `
    "$workspace/lili-session/tests/fixtures/codex"

$bundleRoot = Join-Path $buildTarget "release/bundle"
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $bundleRoot
for ($attempt = 1; $attempt -le 3; $attempt++) {
    try {
        cargo tauri build --bundles nsis -- --locked
        break
    }
    catch {
        if ($attempt -eq 3) {
            throw
        }
        Write-Warning "Tauri build attempt $attempt failed; retrying pinned tool downloads"
        Start-Sleep -Seconds (10 * $attempt)
    }
}

$releaseParent = Join-Path $workspace "release"
$releaseName = "lili-$version-$platform"
$releaseRoot = Join-Path $releaseParent $releaseName
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $releaseRoot

New-Item -ItemType Directory -Force `
    "$releaseRoot/bin", `
    "$releaseRoot/bundles/nsis", `
    "$releaseRoot/docs", `
    "$releaseRoot/examples", `
    "$releaseRoot/pets/lili" | Out-Null

Copy-Item "$buildTarget/release/lili.exe", "$buildTarget/release/lili-hook.exe" "$releaseRoot/bin/"
Copy-Item "$bundleRoot/nsis/*.exe" "$releaseRoot/bundles/nsis/"
Copy-Item -Recurse "dist" "$releaseRoot/web"
Copy-Item "lili-pet/assets/fallback/pet.json", "lili-pet/assets/fallback/spritesheet.webp" "$releaseRoot/pets/lili/"
Copy-Item "README.md" "$releaseRoot/"
Copy-Item "docs/build.md", "docs/configuration.md", "docs/security-and-operations.md" "$releaseRoot/docs/"
Copy-Item "examples/actions.toml" "$releaseRoot/examples/"
Copy-Item "LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.html" "$releaseRoot/"

$forwarderSignatureKind = "platform-standard"
$forwarderSignature = Get-AuthenticodeSignature "$buildTarget/release/lili-hook.exe"
if ($forwarderSignature.Status -eq [System.Management.Automation.SignatureStatus]::Valid) {
    $forwarderSignatureKind = "signed"
}
if ($env:LILI_REQUIRE_SIGNED -eq "1" -and $forwarderSignatureKind -ne "signed") {
    throw "release signing was required but the hook forwarder is unsigned"
}

$forwarderRoot = Join-Path $releaseParent "forwarders/$platform"
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $forwarderRoot
New-Item -ItemType Directory -Force $forwarderRoot | Out-Null
Copy-Item "$buildTarget/release/lili-hook.exe" "$forwarderRoot/lili-hook.exe"
node scripts/write-forwarder-manifest.mjs `
    "$forwarderRoot/lili-hook.exe" `
    "$forwarderRoot/manifest.json" `
    $version `
    $platform `
    $forwarderSignatureKind

node scripts/release-manifest.mjs `
    $releaseRoot `
    $version `
    $platform `
    "platform-standard" `
    $workspace

$archive = Join-Path $releaseParent "$releaseName.tar.gz"
Remove-Item -Force -ErrorAction SilentlyContinue $archive
tar.exe -C $releaseParent -czf $archive $releaseName
$hash = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
"$hash  $releaseName.tar.gz" | Set-Content "$archive.sha256"

@{
    release = $archive
    signatureKind = "platform-standard"
    forwarder = $forwarderRoot
    forwarderSignatureKind = $forwarderSignatureKind
} | ConvertTo-Json -Compress
