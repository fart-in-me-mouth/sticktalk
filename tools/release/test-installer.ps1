# Silent, no-app-launch smoke test. Refuses to touch an existing installation.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Installer,
    [Parameter(Mandatory)][string]$Client,
    [string]$InstallDir = "$env:TEMP/StickTalk-InstallerTest-$PID",
    [string]$Scratch = (Join-Path $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot '../../target' }) 'installer-test')
)
$ErrorActionPreference = 'Stop'
function Assert([bool]$Condition, [string]$Message) { if (!$Condition) { throw $Message } }
$Installer = (Resolve-Path -LiteralPath $Installer).Path
$Client = (Resolve-Path -LiteralPath $Client).Path
$InstallDir = [IO.Path]::GetFullPath($InstallDir)
$tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
Assert ($InstallDir.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) 'Installer smoke tests must use a temporary prefix, never the normal app directory.'
$start = Join-Path ([Environment]::GetFolderPath('Programs')) 'StickTalk.lnk'
$desktop = Join-Path ([Environment]::GetFolderPath('Desktop')) 'StickTalk.lnk'
$uninstallKey = 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/StickTalk'
$protocolKey = 'HKCU:/Software/Classes/sticktalk'
$aumidKey = 'HKCU:/Software/Classes/AppUserModelId/StickTalk.Desktop'
$runKey = 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Run'
foreach ($path in @((Join-Path $InstallDir 'sticktalk.exe'), (Join-Path $InstallDir 'Uninstall.exe'), $start, $desktop, $uninstallKey, $protocolKey, $aumidKey)) {
    if (Test-Path -LiteralPath $path) { throw "Refusing to disturb existing installation: $path" }
}
if (Get-ItemProperty -LiteralPath $runKey -Name StickTalk -ErrorAction SilentlyContinue) { throw 'Existing StickTalk autostart preference: refusing test.' }
New-Item -ItemType Directory -Force -Path $Scratch | Out-Null
$uninstallCopy = Join-Path (Resolve-Path -LiteralPath $Scratch).Path 'test-uninstall.exe'
Assert (!(Test-Path -LiteralPath $InstallDir)) 'Test prefix must not already exist.'
$installed = $false
try {
    $p = Start-Process -FilePath $Installer -ArgumentList "/S /DESKTOP=1 /D=$InstallDir" -WindowStyle Hidden -Wait -PassThru
    $installed = Test-Path -LiteralPath (Join-Path $InstallDir 'Uninstall.exe')
    Assert ($p.ExitCode -eq 0) "Silent install failed: $($p.ExitCode)"
    Assert ((Get-FileHash -LiteralPath (Join-Path $InstallDir 'sticktalk.exe')).Hash -eq (Get-FileHash -LiteralPath $Client).Hash) 'Installed client differs from built binary.'
    Assert (Test-Path -LiteralPath (Join-Path $InstallDir 'sticktalk-maintenance.exe')) 'Maintenance helper missing.'
    foreach ($name in @('sticktalk.ico','vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll')) {
        Assert (Test-Path -LiteralPath (Join-Path $InstallDir $name)) "App-local runtime/icon missing: $name"
    }
    $shell = New-Object -ComObject Shell.Application
    foreach ($lnk in @($start,$desktop)) {
        Assert (Test-Path -LiteralPath $lnk) "Shortcut missing: $lnk"
        $folder = $shell.NameSpace((Split-Path $lnk))
        $item = $folder.ParseName((Split-Path $lnk -Leaf))
        Assert ($item.ExtendedProperty('System.AppUserModel.ID') -eq 'StickTalk.Desktop') "Shortcut AUMID missing: $lnk"
    }
    $command = (Get-Item -LiteralPath "$protocolKey/shell/open/command").GetValue('')
    Assert ($command -eq ('"' + (Join-Path $InstallDir 'sticktalk.exe').Replace('/','\') + '" "%1"')) 'Protocol command must quote executable and payload.'
    Assert (Test-Path -LiteralPath $aumidKey) 'AUMID registration missing.'
    Assert (Test-Path -LiteralPath $uninstallKey) 'Uninstall registration missing.'
    Assert (!(Get-ItemProperty -LiteralPath $runKey -Name StickTalk -ErrorAction SilentlyContinue)) 'Autostart was incorrectly enabled.'
    Write-Output 'PASS silent install: client hash, helper, desktop/start shortcuts, both AUMIDs, protocol, uninstall entry, autostart off.'
} finally {
    if ($installed) {
        Copy-Item -LiteralPath (Join-Path $InstallDir 'Uninstall.exe') -Destination $uninstallCopy -Force
        # _?= must be last/unquoted for NSIS; run a scratch copy so installed files
        # can be deleted without a detached temporary uninstaller process.
        $p = Start-Process -FilePath $uninstallCopy -ArgumentList "/S _?=$InstallDir" -WindowStyle Hidden -Wait -PassThru
        Assert ($p.ExitCode -eq 0) "Silent uninstall failed: $($p.ExitCode)"
        foreach ($path in @((Join-Path $InstallDir 'sticktalk.exe'),(Join-Path $InstallDir 'sticktalk-maintenance.exe'),(Join-Path $InstallDir 'Uninstall.exe'),$start,$desktop,$uninstallKey,$protocolKey,$aumidKey)) {
            Assert (!(Test-Path -LiteralPath $path)) "Uninstall left $path"
        }
        Assert (!(Test-Path -LiteralPath $InstallDir)) 'Uninstall left an installation directory or payload files.'
        Remove-Item -LiteralPath $uninstallCopy -Force
        Write-Output 'PASS silent uninstall: binaries, shortcuts and registrations removed; existing cache preserved.'
    }
}
Write-Output "Installer size: $((Get-Item -LiteralPath $Installer).Length) bytes"
