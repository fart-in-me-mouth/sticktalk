# Stop a self-hosted StickTalk server, remove its startup task and its Tailscale
# address. Keeps your data unless -DeleteData. Never touches other Tailscale routes.
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'StickTalk\Server'),
    [ValidateRange(1024, 65535)][int]$Port = 8080,
    [string]$TaskName = 'StickTalk Server',
    [switch]$DeleteData
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'windows-common.ps1')
Stop-StickTalk $InstallDir $TaskName
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
Write-Host "Stopped StickTalk and removed the '$TaskName' startup task."
try {
    $ts = Get-Tailscale
    $tsHost = ((& $ts status --json | ConvertFrom-Json).Self.DNSName).TrimEnd('.')
    Remove-TailscaleRoutes $ts $tsHost "http://127.0.0.1:$Port"
} catch { Write-Warning "Tailscale routes not changed: $($_.Exception.Message)" }
if ($DeleteData) {
    # Only delete a directory this setup created.
    if (!(Test-Path -LiteralPath (Join-Path $InstallDir 'run-server.cmd'))) { throw "$InstallDir does not look like a StickTalk install; not deleting it." }
    Remove-Item -LiteralPath $InstallDir -Recurse -Force
    Write-Host "Deleted $InstallDir (server, web app, database and uploads)."
} else {
    Write-Host "Kept $InstallDir (database and uploads). Re-run setup-windows.ps1 to start again."
}
