# Requires an ephemeral TEST key already embedded in the helper/package binaries.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$TestKey,
    [string]$TargetDir = $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot '../../target' }),
    [switch]$RealClient,
    [string]$MakeNsis = (Join-Path $TargetDir 'tools/nsis-3.13/makensis.exe')
)
$ErrorActionPreference = 'Stop'
$env:PYTHONUTF8 = '1'
$extra = @()
if ($RealClient) { $extra += '--real-client' }
& python "$PSScriptRoot/test-update.py" --key $TestKey --target $TargetDir --nsis $MakeNsis @extra
if ($LASTEXITCODE -ne 0) { throw "Signed update integration failed ($LASTEXITCODE)" }
