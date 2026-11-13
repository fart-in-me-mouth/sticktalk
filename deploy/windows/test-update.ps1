# Requires an ephemeral TEST key already embedded in the helper/package binaries.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$TestKey,
    [string]$TargetDir = "$env:USERPROFILE/.cargo-target/sticktalk-release-eng",
    [string]$MakeNsis = "$env:USERPROFILE/.cargo-target/sticktalk-release-eng/tools/nsis-3.13/makensis.exe"
)
$ErrorActionPreference = 'Stop'
$env:PYTHONUTF8 = '1'
& python "$PSScriptRoot/test-update.py" --key $TestKey --target $TargetDir --nsis $MakeNsis
if ($LASTEXITCODE -ne 0) { throw "Signed update integration failed ($LASTEXITCODE)" }
