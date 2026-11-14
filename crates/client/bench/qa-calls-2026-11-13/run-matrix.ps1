$ErrorActionPreference = 'Stop'
# Run only after final native compilation/validation. Sequential qualification:
# do not compete for the GPU with another encoder, renderer or build.
$cases = @()
foreach ($source in @('synthetic', 'wgc-motion')) {
    foreach ($quality in @('1080', '1440')) {
        $cases += @{ Source=$source; Quality=$quality; Fps=60; Seconds=600; Name="final-$source-$quality-60-600" }
    }
}
foreach ($fps in @(15,30,60)) {
    foreach ($quality in @('480','720','1080','1440','source')) {
        $cases += @{ Source='synthetic'; Quality=$quality; Fps=$fps; Seconds=120; Name="preset-$quality-$fps-120" }
    }
}
$shell = (Get-Process -Id $PID).Path
$results = @()
foreach ($case in $cases) {
    $started = (Get-Date).ToUniversalTime().ToString('o')
    $process = Start-Process -FilePath $shell -ArgumentList @('-NoProfile','-File',('"' + (Join-Path $PSScriptRoot 'run-acceptance.ps1') + '"'),'-Source',$case.Source,'-Quality',$case.Quality,'-Fps',"$($case.Fps)",'-Seconds',"$($case.Seconds)",'-Name',$case.Name) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $PSScriptRoot "$($case.Name).runner.log") -RedirectStandardError (Join-Path $PSScriptRoot "$($case.Name).runner.stderr.log")
    $process.WaitForExit()
    $results += @{ Case=$case.Name; ExitCode=$process.ExitCode; Started=$started; Finished=(Get-Date).ToUniversalTime().ToString('o') }
    $process.Dispose()
    $results | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'matrix-results.json')
}
if (@($results | Where-Object { $_.ExitCode -ne 0 }).Count) { exit 1 }
