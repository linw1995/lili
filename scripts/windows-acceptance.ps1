$ErrorActionPreference = "Stop"

for ($attempt = 1; $attempt -le 3; $attempt++) {
    cargo tauri build --bundles nsis -- --locked
    if ($LASTEXITCODE -eq 0) {
        break
    }
    if ($attempt -eq 3) {
        exit $LASTEXITCODE
    }
    Write-Warning "Tauri build attempt $attempt failed; retrying pinned tool downloads"
    Start-Sleep -Seconds (10 * $attempt)
}
cargo build --locked --release --package lili --features acceptance --bin lili-hook --bin lili-action-tree-fixture --bin lili-windows-acceptance
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$installer = Get-ChildItem -Path "target/release/bundle/nsis" -Filter "*.exe" | Select-Object -First 1
if ($null -eq $installer) {
    throw "NSIS installer was not produced"
}

& "target/release/lili-windows-acceptance.exe" `
    "target/release/lili.exe" `
    "target/release/lili-hook.exe" `
    "target/release/lili-action-tree-fixture.exe" `
    $installer.FullName
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
