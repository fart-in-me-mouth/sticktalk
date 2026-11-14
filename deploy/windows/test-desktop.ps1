# Native acceptance test. Explicitly opens the installed app on a secondary
# monitor when available. Uses an isolated profile; never sends synthetic input.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Installer,
    [string]$TargetDir = $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot '../../target' })
)
$ErrorActionPreference = 'Stop'
function Assert([bool]$Condition, [string]$Message) { if (!$Condition) { throw $Message } }
$originalLocal = $env:LOCALAPPDATA
$originalAgent = $env:STICKTALK_AGENT_WINDOW
$install = Join-Path $originalLocal 'Programs/StickTalk'
$exe = Join-Path $install 'sticktalk.exe'
$registry = 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/StickTalk'
$start = Join-Path ([Environment]::GetFolderPath('Programs')) 'StickTalk.lnk'
$desktop = Join-Path ([Environment]::GetFolderPath('Desktop')) 'StickTalk.lnk'
foreach ($path in @($install,$registry,$start,$desktop,'HKCU:/Software/Classes/sticktalk','HKCU:/Software/Classes/AppUserModelId/StickTalk.Desktop')) {
    Assert (!(Test-Path -LiteralPath $path)) "Refusing to disturb an existing installation: $path"
}
$profile = Join-Path $TargetDir "native-final-localappdata-$PID"
Assert (!(Test-Path -LiteralPath $profile)) 'Acceptance profile already exists; choose a fresh target.'
$root = Join-Path $profile 'StickTalk'
New-Item -ItemType Directory -Force -Path $root | Out-Null
$probe = Join-Path $TargetDir 'window-probe.exe'
& rustc --edition 2021 -O -C debuginfo=0 "$PSScriptRoot/probe-window.rs" -o $probe
Assert ($LASTEXITCODE -eq 0) 'Native read-only probe build failed.'
Add-Type -AssemblyName System.Windows.Forms
$monitor = [System.Windows.Forms.Screen]::AllScreens | Where-Object { !$_.Primary } | Select-Object -First 1
if (!$monitor) { $monitor = [System.Windows.Forms.Screen]::PrimaryScreen }
$work = $monitor.WorkingArea
$rect = @(($work.Left + 32),($work.Top + 32),($work.Left + 32 + [Math]::Min(1280,$work.Width - 64)),($work.Top + 32 + [Math]::Min(800,$work.Height - 64)))
function SavePlacement([bool]$Max) { @{rect=$rect;maximized=$Max} | ConvertTo-Json -Compress | Set-Content "$root/window.json" -Encoding ascii }
function State($Process) {
    $Process.Refresh()
    Assert (!$Process.HasExited) 'Installed client exited unexpectedly.'
    $observation = & $probe $Process.Id | ConvertFrom-Json
    $window = $observation.windows | Where-Object title -eq 'StickTalk' | Select-Object -First 1
    Assert ($null -ne $window) 'Real StickTalk window missing (helper windows do not count).'
    return $window
}
function StopOwn($Process) {
    if ($Process) {
        $Process.Refresh()
        if (!$Process.HasExited) {
            Assert ([IO.Path]::GetFullPath($Process.Path) -eq [IO.Path]::GetFullPath($exe)) 'Refusing to stop a different executable.'
            Stop-Process -Id $Process.Id
            $Process.WaitForExit()
        }
    }
}
$installed = $false
$app = $null
$results = [ordered]@{profile=$profile;monitor=$monitor.DeviceName}
try {
    $setup = Start-Process -FilePath $Installer -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
    $installed = Test-Path -LiteralPath "$install/Uninstall.exe"
    Assert ($setup.ExitCode -eq 0 -and $installed) 'Default per-user install failed.'
    Assert (!(Test-Path -LiteralPath $desktop)) 'Desktop shortcut must be optional/off by default.'
    $env:LOCALAPPDATA = $profile
    $env:STICKTALK_AGENT_WINDOW = $null # Test real tray/IPC/placement, not dry-run host.
    SavePlacement $false
    $app = Start-Process -FilePath $exe -PassThru
    Start-Sleep 6
    $results.normal = State $app
    Assert $results.normal.visible 'Normal launch remained hidden.'
    Assert ($results.normal.rect[0] -ge $work.Left - 16) 'Window did not restore to selected monitor.'
    $before = $app.TotalProcessorTime.TotalSeconds
    $timer = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep 60
    $app.Refresh()
    $results.idle = @{seconds=$timer.Elapsed.TotalSeconds;cpu_one_core_percent=100*($app.TotalProcessorTime.TotalSeconds-$before)/$timer.Elapsed.TotalSeconds;working_bytes=$app.WorkingSet64;private_bytes=$app.PrivateMemorySize64}
    foreach ($link in @('sticktalk://open','sticktalk://invite/acceptance-fixture')) {
        $second = Start-Process -FilePath $exe -ArgumentList $link -Wait -PassThru
        Assert ($second.ExitCode -eq 0) 'Second-launch forwarding failed.'
    }
    $results.after_links = State $app
    $same = @(Get-Process sticktalk -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe })
    Assert ($same.Count -eq 1 -and $same[0].Id -eq $app.Id) 'Second launch created another installed client.'
    StopOwn $app
    "true`n" | Set-Content "$root/desktop.json" -Encoding ascii
    SavePlacement $false
    $app = Start-Process -FilePath $exe -PassThru
    Start-Sleep 5
    $results.minimized = State $app
    Assert (!$results.minimized.visible) 'Start-minimized preference did not hide the real app window.'
    SavePlacement $true
    $second = Start-Process -FilePath $exe -ArgumentList 'sticktalk://open' -Wait -PassThru
    Start-Sleep 2
    $results.maximized = State $app
    Assert ($results.maximized.visible -and $results.maximized.maximized) 'Opening from tray failed to honor maximized preference.'
    $second = Start-Process -FilePath $exe -ArgumentList 'sticktalk://open' -Wait -PassThru
    Start-Sleep 1
    $results.maximized_again = State $app
    Assert $results.maximized_again.maximized 'Second activation unmaximized the window.'
    StopOwn $app
    'false' | Set-Content "$root/desktop.json" -Encoding ascii
    $app = Start-Process -FilePath $exe -PassThru
    Start-Sleep 5
    $results.restored = State $app
    Assert ($results.restored.visible -and $results.restored.maximized) 'Maximized state did not survive restart.'
    Assert (Test-Path -LiteralPath "$root/Logs/running.flag") 'Crash detection running marker absent.'
    $results | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $TargetDir 'desktop-results.json') -Encoding utf8
    Write-Output 'PASS installed native launch, secondary placement, idle measurement, single instance, links, tray preference, maximized reopen/restart.'
} finally {
    StopOwn $app
    $env:LOCALAPPDATA = $originalLocal
    $env:STICKTALK_AGENT_WINDOW = $originalAgent
    if ($installed) {
        $copy = Join-Path $TargetDir 'native-uninstall.exe'
        Copy-Item -LiteralPath "$install/Uninstall.exe" -Destination $copy -Force
        $uninstall = Start-Process -FilePath $copy -ArgumentList "/S _?=$install" -WindowStyle Hidden -Wait -PassThru
        Assert ($uninstall.ExitCode -eq 0) 'Native acceptance uninstall failed.'
        foreach ($path in @($install,$registry,$start,$desktop,'HKCU:/Software/Classes/sticktalk','HKCU:/Software/Classes/AppUserModelId/StickTalk.Desktop')) {
            Assert (!(Test-Path -LiteralPath $path)) "Uninstall left $path"
        }
        Write-Output 'PASS default-prefix uninstall: no installed payload, shortcuts or registrations remain; owner data retained.'
    }
}
