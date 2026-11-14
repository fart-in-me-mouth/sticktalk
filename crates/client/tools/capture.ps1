param([string]$Exe = (Join-Path $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { (Join-Path $PSScriptRoot '../../../target') }) 'release/sticktalk.exe'))
$ErrorActionPreference='Stop'
$root=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
# This process never constructs an OS window. Hidden also suppresses any console host.
$p=Start-Process -FilePath $Exe -ArgumentList "--mock --screenshots `"$root/screenshots`" --sizes 1280x800,1920x1080" -WindowStyle Hidden -PassThru
if(!$p.WaitForExit(180000)){Stop-Process -Id $p.Id;throw 'Headless capture timed out'}
$p.WaitForExit()
$p.Refresh()
if($p.ExitCode -ne 0){throw 'Headless capture failed'}
