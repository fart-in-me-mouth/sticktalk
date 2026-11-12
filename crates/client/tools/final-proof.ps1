param([string]$Exe='C:/Users/dwhee/.cargo-target/sticktalk-client/release/sticktalk.exe')
$ErrorActionPreference='Stop'
$env:PATH="$env:USERPROFILE/.cargo/bin;$env:PATH"
$env:CARGO_TARGET_DIR='C:/Users/dwhee/.cargo-target/sticktalk-client'
$root=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$log=Join-Path $root 'VALIDATION.txt'
"Final client verification $(Get-Date -Format o)" | Set-Content $log
function Check([string[]]$Command,[bool]$Required=$true){
 "`n> cargo $($Command -join ' ')" | Tee-Object -FilePath $log -Append
 & cargo @Command 2>&1 | Tee-Object -FilePath $log -Append
 $code=$LASTEXITCODE
 "Exit: $code" | Tee-Object -FilePath $log -Append
 if($Required -and $code -ne 0){throw "Failed: cargo $($Command -join ' ')"}
}
Check @('test','-p','sticktalk')
Check @('clippy','-p','sticktalk','--all-targets','--','-D','warnings')
Check @('fmt','-p','sticktalk','--','--check')
Check @('fmt','--all','--','--check') $false
Check @('build','-p','sticktalk','--release')
$hash=(Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
"Final release SHA256: $hash" | Tee-Object -FilePath $log -Append
function Headless([string]$Arguments,[int]$Timeout=180000){
 $err=Join-Path $env:CARGO_TARGET_DIR 'headless-proof-stderr.txt'
 $p=Start-Process -FilePath $Exe -ArgumentList $Arguments -WindowStyle Hidden -RedirectStandardError $err -PassThru
 if(!$p.WaitForExit($Timeout)){Stop-Process -Id $p.Id;throw "Headless timed out: $Arguments"}
 $p.WaitForExit();$p.Refresh()
 if($p.ExitCode -ne 0){throw "Headless failed: $Arguments : $(Get-Content $err -Raw)"}
 "Passed: $Arguments" | Tee-Object -FilePath $log -Append
}
Headless "--mock --bench-scroll `"$root/scroll-after-gpu.json`" --gpu"
Headless "--mock --bench-scroll `"$root/scroll-after-tiny.json`""
Headless "--mock --screenshots `"$root/screenshots`" --sizes 1280x800,1920x1080"
& "$PSScriptRoot/measure.ps1" -Exe $Exe -Renderer 'wgpu/dx12' -Arguments "--mock --size 2560x1440 --agent-window-report `"$root/perf-final-placement-gpu.json`"" -Output perf-final-gpu.json
& "$PSScriptRoot/measure.ps1" -Exe $Exe -Renderer 'tiny-skia' -Arguments "--mock --size 2560x1440 --agent-window-report `"$root/perf-final-placement-tiny.json`"" -Output perf-final-tiny.json
$stats=Join-Path $root 'perf-final-history-stats.json'
if(Test-Path -LiteralPath $stats){throw 'Use a fresh history stats path; archive/remove the previous proof explicitly first.'}
& "$PSScriptRoot/measure.ps1" -Exe $Exe -Renderer 'wgpu/dx12' -Arguments "--mock --screen scroll-5000 --size 2560x1440 --stats `"$stats`" --agent-window-report `"$root/perf-final-placement-history.json`"" -WaitFor $stats -Output perf-final-history.json
& "$PSScriptRoot/measure.ps1" -Exe $Exe -Renderer 'wgpu/dx12' -Arguments "--mock --screen image-heavy --agent-window-report `"$root/perf-final-placement-images.json`"" -Output perf-final-images.json
python "$PSScriptRoot/stamp_proof.py"
"Proof measurements complete: $(Get-Date -Format o)" | Add-Content $log
