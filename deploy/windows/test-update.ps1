# Requires an ephemeral TEST key already embedded in the helper/package binaries.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$TestKey,
    [string]$TargetDir = "$env:USERPROFILE/.cargo-target/sticktalk-release-eng",
    [switch]$RealClient,
    [string]$MakeNsis = "$env:USERPROFILE/.cargo-target/sticktalk-release-eng/tools/nsis-3.13/makensis.exe"
)
$ErrorActionPreference = 'Stop'
$env:PYTHONUTF8 = '1'
$extra = @()
if ($RealClient) { $extra += '--real-client' }
& python "$PSScriptRoot/test-update.py" --key $TestKey --target $TargetDir --nsis $MakeNsis @extra
if ($LASTEXITCODE -ne 0) { throw "Signed update integration failed ($LASTEXITCODE)" }
