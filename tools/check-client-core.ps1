param([string]$TargetDir = "$env:USERPROFILE/.cargo-target/sticktalk-core-wasm")
$ErrorActionPreference = 'Stop'
$previousTarget = $env:CARGO_TARGET_DIR
Push-Location (Join-Path $PSScriptRoot '..')
try {
    $env:CARGO_TARGET_DIR = $TargetDir
    cargo check -p sticktalk-client-core --target wasm32-unknown-unknown --features web
    if ($LASTEXITCODE) { throw 'client-core wasm check failed (install target with rustup target add wasm32-unknown-unknown)' }
} finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    Pop-Location
}
