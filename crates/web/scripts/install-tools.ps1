# Dot-source on Windows x64. Portable, pinned binaries; no installer/elevation.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'This portable-tool helper is Windows-only; use the Unix tool instructions in HANDOFF.md.' }
if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../target')) }
$env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
$env:CARGO_INCREMENTAL = '0'
$tools = Join-Path $env:CARGO_TARGET_DIR 'tools'
$downloads = Join-Path $tools 'downloads'
New-Item -ItemType Directory -Force $downloads | Out-Null
function Get-Tool([string]$Name, [string]$Url, [string]$Sha256) {
    $archive = Join-Path $downloads "$Name.tar.gz"
    if (!(Test-Path -LiteralPath $archive)) { Invoke-WebRequest $Url -OutFile $archive }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $Sha256) { throw "$Name archive hash mismatch; remove the bad archive and retry." }
    & tar -xf $archive -C $tools
    if ($LASTEXITCODE -ne 0) { throw "$Name extraction failed." }
}
Get-Tool 'binaryen' 'https://github.com/WebAssembly/binaryen/releases/download/version_131/binaryen-version_131-x86_64-windows.tar.gz' '2F4EDAC1703A2F695254D6FF52EDE03481E67DB1F094915763D863158C17D9BC'
Get-Tool 'bindgen' 'https://github.com/wasm-bindgen/wasm-bindgen/releases/download/0.2.130/wasm-bindgen-0.2.130-x86_64-pc-windows-msvc.tar.gz' '844D23F1B56097549B936DD2E0D561FADEBC1E8C9E985D5068B6B919654104AD'
$env:WASM_OPT = Join-Path $tools 'binaryen-version_131/bin/wasm-opt.exe'
$env:WASM_BINDGEN = Join-Path $tools 'wasm-bindgen-0.2.130-x86_64-pc-windows-msvc/wasm-bindgen.exe'
$env:PATH = "$tools/bin;$env:PATH"
& rustup target add wasm32-unknown-unknown
if ($LASTEXITCODE -ne 0) { throw 'WASM Rust target installation failed.' }
if (!(Test-Path -LiteralPath "$tools/bin/wasm-split-cli.exe")) {
    & cargo install wasm_split_cli_support --version 0.2.5 --locked --features build-binary --root $tools
    if ($LASTEXITCODE -ne 0) { throw 'wasm-split-cli installation failed.' }
}
& $env:WASM_BINDGEN --version
& $env:WASM_OPT --version
