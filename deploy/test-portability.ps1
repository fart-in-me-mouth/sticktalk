# Read-only portability checks; no UI, listeners, installs or credentials.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $repo
$savedTarget = $env:CARGO_TARGET_DIR
try {
    $files = & git ls-files '*.ps1' '*.mjs' '*.py' '*.toml'
    if ($LASTEXITCODE -ne 0) { throw 'Cannot enumerate tracked tools.' }
    foreach ($file in $files) {
        $text = Get-Content -LiteralPath $file -Raw
        if ($text -match '(?i)[a-z]:[/\\]Users[/\\][^/\\\r\n]+[/\\]') { throw "Hardcoded user profile in $file" }
        if ($file.EndsWith('.ps1')) {
            $tokens = $null
            $errors = $null
            [System.Management.Automation.Language.Parser]::ParseFile((Join-Path $repo $file), [ref]$tokens, [ref]$errors) | Out-Null
            if ($errors) { throw "PowerShell parse error in ${file}: $errors" }
        } elseif ($file.EndsWith('.mjs')) {
            & node --check $file
            if ($LASTEXITCODE -ne 0) { throw "JavaScript parse error in $file" }
        }
    }
    foreach ($doc in (& git ls-files '*.md')) {
        if ((Get-Content -LiteralPath $doc -Raw) -match '(?m)^\$env:CARGO_TARGET_DIR\s*=\s*''\$env:') { throw "Single-quoted environment variable in $doc" }
    }
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    $metadata = & cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $expected = Join-Path $repo 'target'
    if ([IO.Path]::GetFullPath($metadata.target_directory) -ne $expected) { throw "Default Cargo target is not $expected" }
    foreach ($private in @('target/probe', 'probe.sqlite3', 'probe.sqlite3-wal', 'probe.pk8', 'probe.age', '.env', 'sticktalk.env', 'sticktalk-backup-key.txt', 'secrets/probe')) {
        & git check-ignore --quiet -- $private
        if ($LASTEXITCODE -ne 0) { throw "Private/build path not ignored: $private" }
    }
    Write-Output "PASS: $($files.Count) tracked tooling/config files; syntax, no literal user profiles, default Cargo target and private-file exclusions."
} finally {
    if ($null -eq $savedTarget) { Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue }
    else { $env:CARGO_TARGET_DIR = $savedTarget }
    Pop-Location
}
