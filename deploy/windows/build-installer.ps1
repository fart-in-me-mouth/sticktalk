# Build only. Never publish, install, elevate or launch the application.
[CmdletBinding()]
param(
    [string]$Version,
    [string]$Output = "$env:USERPROFILE/.cargo-target/sticktalk-release-eng-out",
    [string]$TargetDir = $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot '../../target' }),
    [string]$MakeNsis,
    [string]$RuntimeDir,
    [switch]$SkipBuild,
    # No compiled-in server or update key: people type their own server's address
    # and update by installing a newer release (e.g. from GitHub Releases).
    [switch]$ManualUpdates,
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
    # The desktop app is the WebView shell (crates/desktop), installed as sticktalk.exe.
    $actual = ($metadata.packages | Where-Object name -eq 'sticktalk-desktop').version
    if (!$Version) { $Version = $actual }
    if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') { throw 'Version must be a SemVer.' }
    if (!$TestFixture -and $Version -ne $actual) { throw "Cargo version is $actual, not $Version. Bump the workspace version first." }
    if (!$SkipBuild) {
        if ($ManualUpdates) {
            if ($env:STICKTALK_UPDATE_PUBLIC_KEY -or $env:STICKTALK_UPDATE_ORIGIN) { throw '-ManualUpdates builds have no updater: unset STICKTALK_UPDATE_PUBLIC_KEY and STICKTALK_UPDATE_ORIGIN.' }
        } else {
            if (!$env:STICKTALK_UPDATE_PUBLIC_KEY) { throw 'Set STICKTALK_UPDATE_PUBLIC_KEY before building (or pass -ManualUpdates). See deploy/RELEASING.md.' }
            if (!$env:STICKTALK_DEFAULT_SERVER) { throw 'Set STICKTALK_DEFAULT_SERVER before building (or pass -ManualUpdates).' }
            if ($env:STICKTALK_UPDATE_PUBLIC_KEY -notmatch '^[A-Za-z0-9+/]{43}=$' -or [Convert]::FromBase64String($env:STICKTALK_UPDATE_PUBLIC_KEY).Length -ne 32) { throw 'Update public key must be exactly 32 bytes in base64.' }
            if (!$env:STICKTALK_UPDATE_ORIGIN) { $env:STICKTALK_UPDATE_ORIGIN = $env:STICKTALK_DEFAULT_SERVER }
        }
        if ([bool]$env:STICKTALK_GOOGLE_CLIENT_ID -ne [bool]$env:STICKTALK_GOOGLE_CLIENT_SECRET) { throw 'Google sign-in needs both STICKTALK_GOOGLE_CLIENT_ID and STICKTALK_GOOGLE_CLIENT_SECRET in the build shell, or neither to disable it.' }
        foreach ($value in @($env:STICKTALK_DEFAULT_SERVER,$env:STICKTALK_UPDATE_ORIGIN) | Where-Object { $_ }) {
            $uri = $null
            if (![Uri]::TryCreate($value,[UriKind]::Absolute,[ref]$uri) -or $uri.AbsolutePath -ne '/' -or $uri.Query -or $uri.Fragment -or $uri.UserInfo) { throw 'Server/update origin must be an absolute origin, without path, credentials, query or fragment.' }
            if ($uri.Scheme -ne 'https' -and !($TestFixture -and $uri.Scheme -eq 'http' -and $uri.IsLoopback)) { throw 'Production release origins require HTTPS. Loopback HTTP requires -TestFixture.' }
        }
        # Keep the builder's profile path (their account name) out of shipped panic and
        # assertion locations: Rust via rustc, bundled C/C++ via MSVC's file-name trim.
        $env:CARGO_ENCODED_RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~"
        if ($env:USERPROFILE -notmatch '\s') {
            foreach ($name in 'CFLAGS','CXXFLAGS') { Set-Item "Env:$name" ("$((Get-Item "Env:$name" -ErrorAction SilentlyContinue).Value) /d1trimfile:$env:USERPROFILE".Trim()) }
        }
        & cargo build --locked --release -p sticktalk-desktop --bin sticktalk-desktop -p sticktalk-desktop-platform --bin sticktalk-maintenance --bin sticktalk-package
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
                Invoke-WebRequest -UseBasicParsing 'https://downloads.sourceforge.net/project/nsis/NSIS%203/3.13/nsis-3.13.zip' -OutFile $zip
                # Some SourceForge mirrors return a download HTML interstitial.
                if ((Get-Item -LiteralPath $zip).Length -lt 1000000) {
                    $html = Get-Content -LiteralPath $zip -Raw
                    $redirect = [regex]::Match($html, 'content="5; url=([^"]+)"').Groups[1].Value.Replace('&amp;', '&')
                    if ($redirect -notmatch '^https://downloads\.sourceforge\.net/project/nsis/') { throw 'Unexpected NSIS download redirect.' }
                    Invoke-WebRequest -UseBasicParsing $redirect -OutFile $zip
                }
                # Pin the official portable archive: reviewed separately from each build.
                $expected = 'BA63DFFC4410EE89193E1CB5A41989991BD77C61068DA17E3156D136B7B0B3D8'
                if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash -ne $expected) { throw 'NSIS archive hash mismatch.' }
                Expand-Archive -LiteralPath $zip -DestinationPath $tools -Force
            }
        }
    }
    if (!$RuntimeDir) {
        $vswhere = "${env:ProgramFiles(x86)}/Microsoft Visual Studio/Installer/vswhere.exe"
        if (!(Test-Path -LiteralPath $vswhere)) { throw 'MSVC redistributable directory required; pass -RuntimeDir.' }
        $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        $candidates = Get-ChildItem -LiteralPath "$vs/VC/Redist/MSVC" -Directory | Where-Object Name -Match '^14\.' | Sort-Object Name -Descending
        foreach ($candidate in $candidates) {
            $found = Get-ChildItem -LiteralPath "$($candidate.FullName)/x64" -Directory -Filter 'Microsoft.VC*.CRT' -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($found) { $RuntimeDir = $found.FullName; break }
        }
    }
    if (!(Test-Path -LiteralPath "$RuntimeDir/vcruntime140.dll") -or !(Test-Path -LiteralPath "$RuntimeDir/msvcp140.dll")) { throw 'App-local x64 MSVC runtime DLLs not found.' }
    $RuntimeDir = (Resolve-Path -LiteralPath $RuntimeDir).Path
    $MakeNsis = (Resolve-Path -LiteralPath $MakeNsis).Path
    New-Item -ItemType Directory -Force -Path $Output | Out-Null
    $installer = Join-Path (Resolve-Path -LiteralPath $Output).Path "StickTalk-Setup-$Version.exe"
    $release = Join-Path $env:CARGO_TARGET_DIR 'release'
    $client = Join-Path $release 'sticktalk-desktop.exe'
    foreach ($binary in @($client, (Join-Path $release 'sticktalk-maintenance.exe'))) {
        if (!(Test-Path -LiteralPath $binary)) { throw "Missing $binary. Build without -SkipBuild first." }
    }
    # The remap flags above must have kept the builder's profile path out of the app.
    $profileName = Split-Path -Leaf $env:USERPROFILE
    $bytes = [IO.File]::ReadAllBytes($client)
    $latin1 = [Text.Encoding]::GetEncoding(28591).GetString($bytes)
    foreach ($separator in '\', '/') {
        $needle = "Users$separator$profileName"
        $wide = [Text.Encoding]::GetEncoding(28591).GetString([Text.Encoding]::Unicode.GetBytes($needle))
        if ($latin1.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or $latin1.IndexOf($wide, [StringComparison]::Ordinal) -ge 0) {
            throw "sticktalk-desktop.exe contains the builder's profile path ($needle). Rebuild with this script, not -SkipBuild."
        }
    }
    & $MakeNsis "/DCLIENT=$client" "/DMAINTENANCE=$release\sticktalk-maintenance.exe" "/DOUTPUT=$installer" "/DVERSION=$Version" "/DCRT=$RuntimeDir" "$repo/tools/release/installer.nsi"
    CheckExit 'NSIS'
    Get-Item -LiteralPath $installer | Select-Object FullName, Length
} finally { Pop-Location }

