# Dot-source: . ./deploy/dev-env.ps1 -InstallTools
# Process-local only: no elevation, registry edits, windows or servers.
[CmdletBinding()]
param([switch]$InstallTools)
$ErrorActionPreference = 'Stop'
if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $PSScriptRoot '../target' }
$env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
$env:CARGO_INCREMENTAL = '0'
$tools = Join-Path $env:CARGO_TARGET_DIR 'tools/python'
if ($InstallTools) {
    & python -m pip install --disable-pip-version-check --upgrade --target $tools cmake==3.31.6 meson==1.7.2 ninja==1.13.0 libclang==18.1.1
    if ($LASTEXITCODE -ne 0) { throw 'Python tool installation failed. Install Python 3.11+ with pip first.' }
}
if (!(Test-Path -LiteralPath "$tools/bin/meson.exe")) { throw 'Run . ./deploy/dev-env.ps1 -InstallTools first (Windows x64).' }
$vswhere = "${env:ProgramFiles(x86)}/Microsoft Visual Studio/Installer/vswhere.exe"
if (!(Test-Path -LiteralPath $vswhere)) { throw 'Install VS 2022 Build Tools with C++ and Windows SDK; see HANDOFF.md. This script never elevates.' }
$vs = & $vswhere -latest -version '[17.0,18.0)' -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (!$vs) { throw 'VS 2022 C++ tools not found (Build Tools, Community, Professional or Enterprise).' }
$env:CMAKE_GENERATOR = 'Visual Studio 17 2022'
$env:PYTHONPATH = "$tools;$env:PYTHONPATH"
$env:PATH = "$tools/bin;$tools/cmake/data/bin;$env:PATH"
$env:LIBCLANG_PATH = Join-Path $tools 'clang/native'
# clang 18 parses headers only; native C++ is compiled by MSVC.
$env:BINDGEN_EXTRA_CLANG_ARGS = '-D_ALLOW_COMPILER_AND_STL_VERSION_MISMATCH'
Write-Output "Build environment ready. Target: $env:CARGO_TARGET_DIR"
