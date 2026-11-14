param([switch]$Full)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location $root
try {
    # One shared artifact directory for the whole run, honoring the caller.
    if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $root 'target' }
    $env:CARGO_INCREMENTAL = '0'
    if ($Full) { cargo test -p sticktalk } else { cargo test -p sticktalk --test live -- --nocapture }
    if ($LASTEXITCODE) { throw 'Client tests failed' }
    if ($Full) {
        cargo clippy -p sticktalk --all-targets -- -D warnings
        if ($LASTEXITCODE) { throw 'Client clippy failed' }
        foreach ($crate in @('server', 'proto')) {
            cargo test -p "sticktalk-$crate"
            if ($LASTEXITCODE) { throw "$crate tests failed" }
            cargo clippy -p "sticktalk-$crate" --all-targets -- -D warnings
            if ($LASTEXITCODE) { throw "$crate clippy failed" }
        }
        cargo fmt --all -- --check
        if ($LASTEXITCODE) { throw 'Formatting failed' }
    }
} finally { Pop-Location }
