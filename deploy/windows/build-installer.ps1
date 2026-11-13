# Build only. Never publish, install, elevate or launch the application.
[CmdletBinding()]
param(
    [string]$Version,
    [string]$Output = "$env:USERPROFILE/.cargo-target/sticktalk-release-eng-out",
    [string]$TargetDir = "$env:USERPROFILE/.cargo-target/sticktalk-release-eng",
    [string]$MakeNsis,
    [switch]$SkipBuild,
    # Only for local installer/update fixtures; never used by the publishing script.
    [switch]$TestFixture
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/../..").Path
$env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath($TargetDir)
$env:CARGO_INCREMENTAL = '0'
function CheckExit($step) { if ($LASTEXITCODE -ne 0) { throw "$step failed ($LASTEXITCODE)" } }
Push-Location $repo
try {
    $metadata = & cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    CheckExit 'Cargo metadata'
    $actual = ($metadata.packages | Where-Object name -eq 'sticktalk').version
    if (!$Version) { $Version = $actual }
    if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') { throw 'Version must be a SemVer.' }
    if (!$TestFixture -and $Version -ne $actual) { throw "Cargo version is $actual, not $Version. Bump the workspace version first." }
    if (!$SkipBuild) {
        if (!$env:STICKTALK_UPDATE_PUBLIC_KEY) { throw 'Set STICKTALK_UPDATE_PUBLIC_KEY before building. See deploy/RELEASING.md.' }
        if (!$env:STICKTALK_DEFAULT_SERVER) { throw 'Set STICKTALK_DEFAULT_SERVER before building.' }
        & cargo build --release -p sticktalk --bin sticktalk -p sticktalk-desktop-platform --bin sticktalk-maintenance --bin sticktalk-package
        CheckExit 'Release build'
    }
    if (!$MakeNsis) {
        $command = Get-Command makensis.exe -ErrorAction SilentlyContinue
        if ($command) { $MakeNsis = $command.Source }
        else {
            # Official portable compiler; no installer, registry changes or admin.
            $tools = Join-Path $env:CARGO_TARGET_DIR 'tools'
            $MakeNsis = Join-Path $tools 'nsis-3.13/makensis.exe'
            if (!(Test-Path -LiteralPath $MakeNsis)) {
                New-Item -ItemType Directory -Force -Path $tools | Out-Null
                $zip = Join-Path $tools 'nsis-3.13.zip'
                Invoke-WebRequest 'https://downloads.sourceforge.net/project/nsis/NSIS%203/3.13/nsis-3.13.zip' -OutFile $zip
                # Some SourceForge mirrors return a download HTML interstitial.
                if ((Get-Item -LiteralPath $zip).Length -lt 1000000) {
                    $html = Get-Content -LiteralPath $zip -Raw
                    $redirect = [regex]::Match($html, 'content="5; url=([^"]+)"').Groups[1].Value.Replace('&amp;', '&')
                    if ($redirect -notmatch '^https://downloads\.sourceforge\.net/project/nsis/') { throw 'Unexpected NSIS download redirect.' }
                    Invoke-WebRequest $redirect -OutFile $zip
                }
                # Pin the official portable archive: reviewed separately from each build.
                $expected = 'BA63DFFC4410EE89193E1CB5A41989991BD77C61068DA17E3156D136B7B0B3D8'
                if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash -ne $expected) { throw 'NSIS archive hash mismatch.' }
                Expand-Archive -LiteralPath $zip -DestinationPath $tools -Force
            }
        }
    }
    $MakeNsis = (Resolve-Path -LiteralPath $MakeNsis).Path
    New-Item -ItemType Directory -Force -Path $Output | Out-Null
    $installer = Join-Path (Resolve-Path -LiteralPath $Output).Path "StickTalk-Setup-$Version.exe"
    $release = Join-Path $env:CARGO_TARGET_DIR 'release'
    & $MakeNsis "/DCLIENT=$release\sticktalk.exe" "/DMAINTENANCE=$release\sticktalk-maintenance.exe" "/DOUTPUT=$installer" "/DVERSION=$Version" "$repo/tools/release/installer.nsi"
    CheckExit 'NSIS'
    Get-Item -LiteralPath $installer | Select-Object FullName, Length
} finally { Pop-Location }

