# Build, sign, and explicitly publish a Windows release. Never elevate.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[0-9A-Za-z.+-]{1,64}$')][string]$Version,
    [Parameter(Mandatory)][ValidatePattern('^https://[^/]+/?$')][string]$Server,
    [string]$Key = "$env:USERPROFILE/.sticktalk-secrets/update-signing.pk8",
    [string]$MakeNsis,
    [string]$Output = "$env:USERPROFILE/.cargo-target/sticktalk-release-eng-out"
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/../..").Path
if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $repo 'target' }
$env:CARGO_INCREMENTAL = '0'
if (!(Test-Path -LiteralPath $Key)) { throw 'Signing key missing. Follow deploy/RELEASING.md.' }
$keyPath = (Resolve-Path -LiteralPath $Key).Path
if ($keyPath.StartsWith($repo + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Private signing key must be OUTSIDE the repository.' }
if (!$env:STICKTALK_RELEASE_TOKEN -or $env:STICKTALK_RELEASE_TOKEN.Length -lt 32) { throw 'Set STICKTALK_RELEASE_TOKEN (32+ random characters) in this shell.' }
if (!$env:STICKTALK_UPDATE_PUBLIC_KEY) { throw 'Set the public key before building.' }
$env:STICKTALK_DEFAULT_SERVER = $Server.TrimEnd('/')
$env:STICKTALK_UPDATE_ORIGIN = $Server.TrimEnd('/')
& "$repo/deploy/windows/build-installer.ps1" -Version $Version -Output $Output -MakeNsis $MakeNsis
if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
$installer = Join-Path (Resolve-Path -LiteralPath $Output).Path "StickTalk-Setup-$Version.exe"
$manifest = Join-Path (Resolve-Path -LiteralPath $Output).Path "StickTalk-$Version-manifest.json"
& "$env:CARGO_TARGET_DIR/release/sticktalk-package.exe" sign $keyPath $installer $Version $Server $manifest
if ($LASTEXITCODE -ne 0) { throw 'Manifest signing failed' }
$headers = @{Authorization="Bearer $env:STICKTALK_RELEASE_TOKEN"; 'X-StickTalk-Manifest'=[Convert]::ToBase64String([IO.File]::ReadAllBytes($manifest))}
Invoke-WebRequest -UseBasicParsing -Uri "$($Server.TrimEnd('/'))/api/update/publish" -Method Post -Headers $headers -ContentType 'application/octet-stream' -InFile $installer -TimeoutSec 120 | Out-Null
Write-Output "Published $Version. Installer: $installer"
