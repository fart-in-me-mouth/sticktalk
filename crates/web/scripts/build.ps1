$ErrorActionPreference = 'Stop'
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $PSScriptRoot '../../../target' }
$target = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
$env:CARGO_TARGET_DIR = $target
$dist = [IO.Path]::GetFullPath((Join-Path $target 'web-dist'))
if (-not $dist.StartsWith($target.TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Distribution escaped task target directory.' }
Push-Location (Join-Path $PSScriptRoot '..')
try {
  # Tailwind CSS and UI SDK are build inputs (package.json dependencies). Test tools
  # (Playwright, Lighthouse) are devDependencies and are not needed to build.
  if (!(Test-Path -LiteralPath 'node_modules/@tailwindcss/cli') -or !(Test-Path -LiteralPath 'node_modules/@ui-sdk/web')) {
    npm.cmd ci --omit=dev --ignore-scripts --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) { throw 'Installing the web build dependencies (npm ci) failed.' }
  }
  # This directory's .cargo config preserves the split relocation/table metadata.
  cargo build --locked -p sticktalk-web --target wasm32-unknown-unknown --profile web-release
  if ($LASTEXITCODE -ne 0) { throw 'WASM compilation failed.' }
  if (Test-Path -LiteralPath $dist) { Remove-Item -LiteralPath $dist -Recurse -Force }
  $env:TRUNK_STAGING_DIR = $dist
  node scripts/assemble.mjs
  if ($LASTEXITCODE -ne 0) { throw 'WASM splitting/packaging failed.' }
} finally { Pop-Location }
