param([switch]$Full)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location $root
try {
    # Each top-level crate owns an out-of-OneDrive artifact directory.
    $env:CARGO_TARGET_DIR = Join-Path $env:USERPROFILE '.cargo-target/sticktalk-b6-client'
    if ($Full) { cargo test -p sticktalk } else { cargo test -p sticktalk --test live -- --nocapture }
    if ($LASTEXITCODE) { throw 'Client tests failed' }
    if ($Full) {
        cargo clippy -p sticktalk --all-targets -- -D warnings
        if ($LASTEXITCODE) { throw 'Client clippy failed' }
        foreach ($crate in @('server', 'proto')) {
            $env:CARGO_TARGET_DIR = Join-Path $env:USERPROFILE ".cargo-target/sticktalk-b6-$crate"
            cargo test -p "sticktalk-$crate"
            if ($LASTEXITCODE) { throw "$crate tests failed" }
            cargo clippy -p "sticktalk-$crate" --all-targets -- -D warnings
            if ($LASTEXITCODE) { throw "$crate clippy failed" }
        }
        cargo fmt --all -- --check
        if ($LASTEXITCODE) { throw 'Formatting failed' }
    }
} finally { Pop-Location }
