# Non-incremental, release-only qualification with a conservative 10 GB target cap.
param([switch]$SkipPassedTests)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../../../..')).Path
Set-Location -LiteralPath $root
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-calls2'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_BUILD_JOBS = '2'
$target = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
if ($target -ne 'C:\Users\dwhee\.cargo-target\sticktalk-calls2') { throw 'Unexpected task target' }
if (Test-Path -LiteralPath (Join-Path $target 'debug')) { throw 'Remove the debug tree before release qualification' }
$cargo = (Get-Command cargo).Source
$script:peak = 0L
$script:budgetHit = $false
$budgetLog = Join-Path $PSScriptRoot 'target-budget.log'
function Invoke-BudgetedCargo([string]$Name, [string[]]$CargoArgs) {
    $stdout = Join-Path $PSScriptRoot "$Name.stdout.log"
    $stderr = Join-Path $PSScriptRoot "$Name.stderr.log"
    $process = Start-Process -FilePath $cargo -ArgumentList $CargoArgs -WorkingDirectory $root -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    try {
        while (-not $process.HasExited) {
            $bytes = 0L
            if (Test-Path -LiteralPath $target) {
                # Linkers rename/remove temporary files during enumeration.
                try {
                    $bytes = [long](Get-ChildItem -LiteralPath $target -File -Recurse -Force -ErrorAction Ignore | Measure-Object Length -Sum).Sum
                } catch {
                    $bytes = $script:peak
                }
            }
            $script:peak = [Math]::Max($script:peak, $bytes)
            Add-Content -LiteralPath $budgetLog -Value ("{0:o} {1} bytes={2} peak={3}" -f (Get-Date), $Name, $bytes, $script:peak)
            # Reserve 1 GB for in-flight compiler/linker writes between checks.
            if ($bytes -gt 9000000000L) {
                $script:budgetHit = $true
                taskkill /PID $process.Id /T /F | Out-Null
                throw 'Stopped conservatively before the 10 GB target budget'
            }
            Start-Sleep -Seconds 5
            $process.Refresh()
        }
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) { throw "$Name failed with exit code $($process.ExitCode); see its logs" }
    } finally {
        if (-not $process.HasExited) { taskkill /PID $process.Id /T /F | Out-Null }
        $process.Dispose()
    }
}
$measurementStarted = $false
try {
    Invoke-BudgetedCargo 'clippy-release' @('clippy','--release','-p','sticktalk-media','-p','sticktalk','--all-targets','-j','2','--','-D','warnings')
    if (-not $SkipPassedTests) {
        Invoke-BudgetedCargo 'tests-release' @('test','--release','-p','sticktalk-media','-p','sticktalk','-j','2','--','--test-threads=1')
    }
    # The actual iced view tree, tiny-skia only: no winit/OS window is constructed.
    $screens = 'voice-call,focused-stream,stream-stats,share-picker,share-picker-windows,share-quality,voice-consent,controlling-banner,request-control,camera-tiles,camera-chooser'
    $shots = Start-Process -FilePath (Join-Path $target 'release/sticktalk.exe') -ArgumentList @('--mock','--screenshots','crates/client/screenshots/calls2','--screens',$screens) -WorkingDirectory $root -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput (Join-Path $PSScriptRoot 'screenshots.stdout.log') -RedirectStandardError (Join-Path $PSScriptRoot 'screenshots.stderr.log')
    if ($shots.ExitCode -ne 0) { throw 'Headless screenshot rendering failed' }
    $measurementStarted = $true
    Invoke-BudgetedCargo 'gpu-release' @('test','--release','-p','sticktalk','--test','video','-j','2','--','--ignored','--nocapture','--test-threads=1')
    Invoke-BudgetedCargo 'acceptance-release' @('test','--release','-p','sticktalk','--test','voice','-j','2','live_calls2_two_viewers_ten_minutes','--','--ignored','--exact','--nocapture')
} finally {
    if ($measurementStarted -or $script:budgetHit) {
        # First command after qualification, including failed qualification.
        & $cargo clean --release 2>&1 | Out-File -LiteralPath (Join-Path $PSScriptRoot 'clean-release.log')
    }
    Add-Content -LiteralPath $budgetLog -Value "Peak target bytes=$script:peak; hard budget=10000000000; CARGO_INCREMENTAL=0"
}
