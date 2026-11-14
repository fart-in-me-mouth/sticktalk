# Transfer committed source/history, never a sweep of the working directory.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Output)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$destination = [IO.Path]::GetFullPath($Output)
if ($destination -eq $repo -or $destination.StartsWith($repo + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Output must be outside the repository.' }
function CheckExit([string]$Step) { if ($LASTEXITCODE -ne 0) { throw "$Step failed ($LASTEXITCODE)" } }
Push-Location $repo
try {
    $dirty = & git status --porcelain --untracked-files=all
    CheckExit 'Git status'
    if ($dirty) { throw 'Commit source changes and track nonsecret required assets first. No untracked files are copied; ignored secrets/build output are excluded.' }
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    $bundle = Join-Path $destination 'sticktalk.bundle'
    if (Test-Path -LiteralPath $bundle) { throw 'sticktalk.bundle exists; choose a fresh output directory.' }
    & git bundle create $bundle --branches --tags HEAD
    CheckExit 'Bundle creation'
    & git bundle verify $bundle
    CheckExit 'Bundle verification'
    $check = Join-Path $destination ('verify-' + [Guid]::NewGuid().ToString('N'))
    try {
        & git clone --quiet $bundle $check
        CheckExit 'Bundle clone'
        $expected = & git rev-parse HEAD
        $actual = & git -C $check rev-parse HEAD
        CheckExit 'Clone HEAD'
        if ($expected -ne $actual) { throw 'Cloned HEAD differs from the handoff commit.' }
        & git -C $check fsck --no-dangling
        CheckExit 'Cloned object integrity'
        $files = & git -C $check status --porcelain
        CheckExit 'Clone status'
        if ($files) { throw 'Bundle clone is not clean.' }
    } finally {
        # Verify the exact absolute target before recursive deletion.
        $resolved = [IO.Path]::GetFullPath($check)
        if (!$resolved.StartsWith($destination.TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolved -Leaf) -notmatch '^verify-[a-f0-9]{32}$') { throw 'Refusing cleanup outside output.' }
        if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
    }
    $hash = (Get-FileHash -LiteralPath $bundle -Algorithm SHA256).Hash
    @("HEAD=$expected", "SHA256=$hash", 'No extra ZIP: all build-required assets are tracked. Ignored/untracked files and target directories are not packaged.', 'History is included: review it for credentials before sharing. Transfer recovery credentials separately and encrypted.') | Set-Content -LiteralPath (Join-Path $destination 'HANDOFF-CONTENTS.txt')
    Get-Item -LiteralPath $bundle | Select-Object FullName, Length
    Write-Output "Verified clean clone at $expected; SHA256 $hash"
} finally { Pop-Location }
