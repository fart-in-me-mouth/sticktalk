param([string]$TargetDir = $env:CARGO_TARGET_DIR)
$ErrorActionPreference = 'Stop'
$previousTarget = $env:CARGO_TARGET_DIR
Push-Location (Join-Path $PSScriptRoot '..')
try {
    if ($TargetDir) { $env:CARGO_TARGET_DIR = $TargetDir }
    cargo check -p sticktalk-client-core --target wasm32-unknown-unknown --features web
    if ($LASTEXITCODE) { throw 'client-core wasm check failed (install target with rustup target add wasm32-unknown-unknown)' }
} finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    Pop-Location
}
