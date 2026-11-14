$ErrorActionPreference = 'Stop'
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $PSScriptRoot '../../../target' }
$target = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
$dist = [IO.Path]::GetFullPath((Join-Path $target 'web-dist'))
if (-not $dist.StartsWith($target.TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Distribution escaped task target directory.' }
Push-Location (Join-Path $PSScriptRoot '..')
try {
  # This directory's .cargo config preserves the split relocation/table metadata.
  cargo build --locked -p sticktalk-web --target wasm32-unknown-unknown --profile web-release
  if ($LASTEXITCODE -ne 0) { throw 'WASM compilation failed.' }
  if (Test-Path -LiteralPath $dist) { Remove-Item -LiteralPath $dist -Recurse -Force }
  $env:TRUNK_STAGING_DIR = $dist
  node scripts/assemble.mjs
  if ($LASTEXITCODE -ne 0) { throw 'WASM splitting/packaging failed.' }
} finally { Pop-Location }
