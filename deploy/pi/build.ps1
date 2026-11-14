[CmdletBinding()]
param([switch]$InstallTools)
$ErrorActionPreference='Stop'
if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../target')) }
$env:CARGO_INCREMENTAL = '0'
# A Windows CMake generator must not leak into Linux-target native builds.
$env:CMAKE_GENERATOR = 'Ninja'
$env:ZIG_GLOBAL_CACHE_DIR=Join-Path $env:CARGO_TARGET_DIR 'zig-cache/global'
$env:ZIG_LOCAL_CACHE_DIR=Join-Path $env:CARGO_TARGET_DIR 'zig-cache/local'
if ($InstallTools) {
    & python -m pip install --user ziglang==0.13.0 cargo-zigbuild==0.24.1
    if ($LASTEXITCODE -ne 0) { throw 'Tool installation failed.' }
}
$scripts = & python -c "import sysconfig;print(sysconfig.get_path('scripts','nt_user'))"
$zig = & python -c "import ziglang,pathlib;print(pathlib.Path(ziglang.__file__).parent)"
if ($LASTEXITCODE -ne 0) { throw 'Run with -InstallTools to install Zig and cargo-zigbuild.' }
$env:PATH="$scripts;$zig;$env:PATH"
& rustup target add aarch64-unknown-linux-gnu
if ($LASTEXITCODE -ne 0) { throw 'Target installation failed.' }
Push-Location "$PSScriptRoot/../.."
try {
    & cargo zigbuild --locked --profile server -p sticktalk-server --bin sticktalk-server --target aarch64-unknown-linux-gnu.2.36
    if ($LASTEXITCODE -ne 0) { throw 'ARM64 build/link failed.' }
    $binary="$env:CARGO_TARGET_DIR/aarch64-unknown-linux-gnu/server/sticktalk-server"
    $elf=[IO.File]::ReadAllBytes($binary)
    if ($elf[0] -ne 127 -or [Text.Encoding]::ASCII.GetString($elf,1,3) -ne 'ELF' -or $elf[4] -ne 2 -or $elf[5] -ne 1 -or [BitConverter]::ToUInt16($elf,18) -ne 183) { throw 'Output is not a little-endian ARM64 ELF binary.' }
    Get-Item -LiteralPath $binary | Select-Object FullName,Length
    Write-Output ("SHA256: " + (Get-FileHash -Algorithm SHA256 -LiteralPath $binary).Hash)
} finally {Pop-Location}
