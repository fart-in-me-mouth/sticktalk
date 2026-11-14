$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../../../..')).Path
Set-Location -LiteralPath $root
$env:CARGO_TARGET_DIR = $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { (Join-Path $PSScriptRoot '../../../../target') })
$env:CARGO_INCREMENTAL = '0'
$target = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
if ($target -eq [IO.Path]::GetPathRoot($target)) { throw 'Target must not be a filesystem root' }
if (Test-Path -LiteralPath (Join-Path $target 'debug')) { throw 'Clean debug first; use only one profile tree.' }
$cargo = (Get-Command cargo).Source
$process = Start-Process -FilePath $cargo -ArgumentList @('test','--profile','server','-p','sticktalk','--test','voice','--no-run','-j2') -WorkingDirectory $root -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $PSScriptRoot 'optimized-build.stdout.log') -RedirectStandardError (Join-Path $PSScriptRoot 'optimized-build.stderr.log')
$peak = 0L
try {
    while (-not $process.HasExited) {
        $bytes = [long](Get-ChildItem -LiteralPath $target -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
        $peak = [Math]::Max($peak, $bytes)
        Add-Content -LiteralPath (Join-Path $PSScriptRoot 'optimized-budget.log') -Value ("{0:o} bytes={1} peak={2}" -f (Get-Date), $bytes, $peak)
        if ($bytes -gt 9000000000L) {
            taskkill /PID $process.Id /T /F | Out-Null
            throw 'Stopped conservatively before the 10 GB budget.'
        }
        Start-Sleep -Seconds 5
        $process.Refresh()
    }
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) { throw "Optimized build failed ($($process.ExitCode)); inspect its logs." }
} finally {
    if (-not $process.HasExited) { taskkill /PID $process.Id /T /F | Out-Null }
    $process.Dispose()
}
