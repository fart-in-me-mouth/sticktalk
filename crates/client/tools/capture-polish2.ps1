param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Output = (Join-Path $PSScriptRoot '../screenshots')
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
$screens = [System.Collections.Generic.List[string]]::new()
foreach ($source in @('src/headless.rs','src/ui/settings/fixtures.rs','src/ui/audit.rs')) {
    $content = Get-Content -LiteralPath (Join-Path $root $source) -Raw
    $arrays = [regex]::Matches($content, 'const \w*SCREENS:\s*&\[&str\]\s*=\s*&\[(.*?)\];', [System.Text.RegularExpressions.RegexOptions]::Singleline)
    foreach ($array in $arrays) {
        foreach ($name in [regex]::Matches($array.Groups[1].Value, '"([^"]+)"')) {
            $screen = $name.Groups[1].Value
            if ($seen.Add($screen)) { $screens.Add($screen) }
        }
    }
}
if ($screens.Count -lt 200) { throw 'Incomplete screenshot inventory.' }
$null = New-Item -ItemType Directory -Path $Output -Force
$outputPath = (Resolve-Path -LiteralPath $Output).Path
$arguments = "--mock --screenshots `"$outputPath`" --sizes 1280x800,1920x1080 --screens $($screens -join ',')"
# This entry point returns before bootstrap: no OS window, browser, audio, or notifications.
$process = Start-Process -FilePath $Exe -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardError "$outputPath/capture.stderr.txt"
if (!$process.WaitForExit(300000)) { Stop-Process -Id $process.Id; throw 'Headless capture timed out.' }
if ($process.ExitCode -ne 0) { throw "Headless capture exited $($process.ExitCode)." }
$manifest = Get-Content -LiteralPath "$outputPath/manifest.json" -Raw | ConvertFrom-Json
if ($manifest.screens.Count -ne $screens.Count * 2) { throw 'Screenshot matrix is incomplete.' }
@{ binary_sha256=(Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash; scenes=$screens.Count; captures=$manifest.screens.Count; window_backend='none' } | ConvertTo-Json | Set-Content -LiteralPath "$outputPath/polish2-capture.json"
"Rendered $($manifest.screens.Count) PNGs, $($screens.Count) scenes, both sizes, no OS window."
