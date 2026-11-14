param([Parameter(Mandatory)][string]$Exe,[string]$Renderer='tiny-skia',[string]$Arguments='--mock',[string]$Output='perf-tiny.json',[string]$WaitFor='')
$ErrorActionPreference='Stop'
$root=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if($Renderer.StartsWith('wgpu/')){$env:ICED_BACKEND='wgpu';$env:WGPU_BACKEND=$Renderer.Split('/')[1]}else{$env:ICED_BACKEND='tiny-skia';Remove-Item Env:WGPU_BACKEND -ErrorAction SilentlyContinue}
if($WaitFor -and (Test-Path -LiteralPath $WaitFor)){throw 'Use a fresh -WaitFor stats path; a stale file would mislabel active pagination as idle.'}
$previousAgentWindow=$env:STICKTALK_AGENT_WINDOW
$env:STICKTALK_AGENT_WINDOW='secondary'
# Profile an immutable, hash-named copy so a concurrent build cannot relabel counters or lock linking.
$hash=(Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
$target=if($env:CARGO_TARGET_DIR){$env:CARGO_TARGET_DIR}else{Join-Path $PSScriptRoot '../../../target'}
$binDir=Join-Path $target 'perf-binaries'
$null=New-Item -ItemType Directory -Path $binDir -Force
$runExe=Join-Path $binDir "sticktalk-$hash.exe"
if(!(Test-Path -LiteralPath $runExe)){Copy-Item -LiteralPath $Exe -Destination $runExe}
$p=Start-Process -FilePath $runExe -ArgumentList $Arguments -WindowStyle Hidden -PassThru
try {
 if($WaitFor){$deadline=(Get-Date).AddSeconds(180);while(!(Test-Path -LiteralPath $WaitFor)){if((Get-Date) -gt $deadline){throw 'Stress scenario did not complete'};Start-Sleep -Milliseconds 250}}
 Start-Sleep -Seconds 10
 $p.Refresh(); if($p.HasExited){throw 'Client exited before measurement'}
 $cpu0=$p.TotalProcessorTime.TotalSeconds;$start=Get-Date;$samples=@()
 for($i=0;$i -lt 60;$i++){Start-Sleep -Seconds 1;$p.Refresh();$samples += [pscustomobject]@{working_set=$p.WorkingSet64;private_bytes=$p.PrivateMemorySize64}}
 $p.Refresh();$seconds=((Get-Date)-$start).TotalSeconds
 $result=[pscustomobject]@{renderer=$Renderer;window_mode='secondary-noactivate';arguments=$Arguments;timestamp=(Get-Date -Format o);warmup_seconds=10;seconds=$seconds;cpu_seconds=$p.TotalProcessorTime.TotalSeconds-$cpu0;one_core_cpu_percent=100*($p.TotalProcessorTime.TotalSeconds-$cpu0)/$seconds;working_set_mean=($samples|Measure-Object -Property working_set -Average).Average;working_set_max=($samples|Measure-Object -Property working_set -Maximum).Maximum;private_bytes_mean=($samples|Measure-Object -Property private_bytes -Average).Average;binary_bytes=(Get-Item $runExe).Length;sha256=(Get-FileHash -LiteralPath $runExe -Algorithm SHA256).Hash}
 $result|ConvertTo-Json|Tee-Object -FilePath (Join-Path $root $Output)
} finally {if(!$p.HasExited){Stop-Process -Id $p.Id};$env:STICKTALK_AGENT_WINDOW=$previousAgentWindow}
