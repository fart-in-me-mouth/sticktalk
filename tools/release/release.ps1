# One command builds the client, creates a per-user NSIS installer, signs and publishes it.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[0-9A-Za-z.+-]{1,64}$')][string]$Version,
    [Parameter(Mandatory)][ValidatePattern('^https://[^/]+/?$')][string]$Server,
    [string]$Key = "$env:USERPROFILE/.sticktalk-secrets/update-signing.pk8",
    [string]$MakeNsis = 'makensis.exe',
    [string]$Output = "$env:USERPROFILE/.cargo-target/sticktalk-releases"
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/../..").Path
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-server'
function CheckExit([string]$Step) { if ($LASTEXITCODE -ne 0) { throw "$Step failed ($LASTEXITCODE)." } }
if (!(Test-Path -LiteralPath $Key)) { throw 'Signing key missing. Follow tools/release/README.md first.' }
$keyPath = (Resolve-Path -LiteralPath $Key).Path
if ($keyPath.StartsWith($repo + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'The private signing key must be OUTSIDE the repository.' }
$nsisCommand = (Get-Command $MakeNsis -ErrorAction Stop).Source
if (!$env:STICKTALK_RELEASE_TOKEN -or $env:STICKTALK_RELEASE_TOKEN.Length -lt 32) { throw 'Set STICKTALK_RELEASE_TOKEN (32+ random characters) in this shell, not in a script.' }
if (!$env:STICKTALK_UPDATE_PUBLIC_KEY) { throw 'Set STICKTALK_UPDATE_PUBLIC_KEY to the base64 public key; the client must embed it.' }
New-Item -ItemType Directory -Force -Path $Output | Out-Null
$out = (Resolve-Path -LiteralPath $Output).Path
$installer = Join-Path $out "StickTalk-$Version-Setup.exe"
$manifest = Join-Path $out "StickTalk-$Version-manifest.json"
$env:STICKTALK_DEFAULT_SERVER = $Server.TrimEnd('/')
Push-Location $repo
try {
    $metadata = (& cargo metadata --no-deps --format-version 1 | ConvertFrom-Json)
    CheckExit 'Cargo metadata'
    $clientVersion = ($metadata.packages | Where-Object name -eq 'sticktalk').version
    if ($clientVersion -ne $Version) { throw "Client Cargo version is $clientVersion, not $Version. Ask the lead to bump the workspace version first." }
    & cargo build --release -p sticktalk --bin sticktalk
    CheckExit 'Windows client build'
    & cargo build --release -p sticktalk-server --bin sticktalk-release
    CheckExit 'Signing helper build'
    & $nsisCommand "/DCLIENT=$env:CARGO_TARGET_DIR/release/sticktalk.exe" "/DOUTPUT=$installer" "/DVERSION=$Version" "$PSScriptRoot/installer.nsi"
    CheckExit 'NSIS installer build'
    & "$env:CARGO_TARGET_DIR/release/sticktalk-release.exe" sign --key $keyPath --artifact $installer --version $Version --server $Server --output $manifest
    CheckExit 'Manifest signing'
    $envelope = [Convert]::ToBase64String([IO.File]::ReadAllBytes($manifest))
    $headers = @{Authorization="Bearer $env:STICKTALK_RELEASE_TOKEN"; 'X-StickTalk-Manifest'=$envelope}
    Invoke-WebRequest -UseBasicParsing -Uri "$($Server.TrimEnd('/'))/api/update/publish" -Method Post -Headers $headers -ContentType 'application/octet-stream' -InFile $installer -TimeoutSec 120 | Out-Null
    Write-Output "Published $Version. Installer: $installer"
} finally { Pop-Location }
