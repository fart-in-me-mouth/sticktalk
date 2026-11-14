# StickTalk self-hosting on Windows: build, install, keep running, publish with
# Tailscale Funnel. Per user: no administrator rights and no Windows service.
# Re-run with the same options to update or reconfigure. See deploy/SELF_HOST.md.
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'StickTalk\Server'),
    [string]$BuildDir = (Join-Path $env:LOCALAPPDATA 'StickTalk\build'),
    [ValidateRange(1024, 65535)][int]$Port = 8080,
    # Funnel: public internet. Tailnet: only your Tailscale devices. Local: this PC only.
    [ValidateSet('Funnel', 'Tailnet', 'Local')][string]$Exposure = 'Funnel',
    # 0 picks the first Tailscale HTTPS port (443, 8443, 10000) not used by another service.
    [ValidateSet(0, 443, 8443, 10000)][int]$HttpsPort = 0,
    # Your own HTTPS origin instead of Tailscale (Caddy + DuckDNS, Cloudflare Tunnel...).
    [string]$PublicUrl,
    # Calls use UDP, which Funnel cannot carry. Tailnet: calls between Tailscale devices.
    # Public: you forward UDP 50000-50100 on your router to this PC and pass your public IP.
    [ValidateSet('Tailnet', 'Public', 'Local')][string]$Voice = 'Tailnet',
    [string]$VoiceAdvertisedIp,
    [string]$TaskName = 'StickTalk Server',
    [switch]$SkipWeb,
    [switch]$SkipBuild,
    [switch]$NoStartupTask
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'This script is for Windows; on Linux or a Raspberry Pi use deploy/self-host/setup-linux.sh.' }
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
. (Join-Path $PSScriptRoot 'windows-common.ps1')
function Say($text) { Write-Host "==> $text" -ForegroundColor Cyan }

# ---- Public address -------------------------------------------------------
$ts = $null
$tsHost = $null
$tsIp = $null
if ($Exposure -ne 'Local' -or $Voice -eq 'Tailnet') {
    $ts = Get-Tailscale
    $status = & $ts status --json | ConvertFrom-Json
    if ($status.BackendState -ne 'Running') { throw 'Tailscale is not connected. Open Tailscale, sign in, then re-run.' }
    $tsHost = $status.Self.DNSName.TrimEnd('.')
    $tsIp = @($status.Self.TailscaleIPs | Where-Object { $_ -match '^\d+\.\d+\.\d+\.\d+$' })[0]
}
$backend = "http://127.0.0.1:$Port"
# Routes this install published before that the new settings no longer use: an
# old -Port's route, or ours at all when switching to Local or -PublicUrl.
$stateFile = Join-Path $InstallDir 'install.json'
$stale = @()
if (Test-Path -LiteralPath $stateFile) {
    $previous = (Get-Content -LiteralPath $stateFile -Raw | ConvertFrom-Json).Backend
    if ($previous -and $previous -ne $backend) { $stale += $previous }
}
if ($PublicUrl -or $Exposure -eq 'Local') { $stale += $backend }
if ($stale) {
    try {
        if (!$ts) { $ts = Get-Tailscale; $tsHost = ((& $ts status --json | ConvertFrom-Json).Self.DNSName).TrimEnd('.') }
        foreach ($b in $stale) { Remove-TailscaleRoutes $ts $tsHost $b }
    } catch { Write-Warning "Could not check Tailscale for old StickTalk routes: $($_.Exception.Message)" }
}
$tsPort = $null
if ($PublicUrl) {
    $u = [Uri]$PublicUrl
    if ($u.Scheme -ne 'https' -or $u.AbsolutePath -ne '/' -or $u.Query) { throw '-PublicUrl must be an https origin such as https://chat.example.org' }
    $PublicUrl = $u.GetLeftPart([UriPartial]::Authority)
} elseif ($Exposure -eq 'Local') {
    $PublicUrl = $backend
} else {
    $tsPort = Select-TailscalePort $ts $tsHost $backend $HttpsPort
    $PublicUrl = if ($tsPort -eq 443) { "https://$tsHost" } else { "https://${tsHost}:$tsPort" }
}
switch ($Voice) {
    'Tailnet' { if (!$tsIp) { throw 'No Tailscale IPv4 address; use -Voice Local or -Voice Public.' }; $voiceBind = $tsIp; $voiceAdvertised = $tsIp }
    'Public' { if (!$VoiceAdvertisedIp) { throw '-Voice Public needs -VoiceAdvertisedIp <your public IPv4>.' }; $voiceBind = '0.0.0.0'; $voiceAdvertised = $VoiceAdvertisedIp }
    'Local' { $voiceBind = '127.0.0.1'; $voiceAdvertised = '127.0.0.1' }
}
Say "Public address: $PublicUrl"

# ---- Build ------------------------------------------------------------------
$env:CARGO_TARGET_DIR = $BuildDir
$env:CARGO_INCREMENTAL = '0'
$serverExe = Join-Path $BuildDir 'server/sticktalk-server.exe'
$webDist = Join-Path $BuildDir 'web-dist'
if (!$SkipBuild) {
    Import-Msvc
    New-Item -ItemType Directory -Force $BuildDir | Out-Null
    Say 'Building the server (first build takes a while)'
    Push-Location $repo
    try { cargo build --locked --profile server -p sticktalk-server --bin sticktalk-server; if ($LASTEXITCODE) { throw 'Server build failed.' } }
    finally { Pop-Location }
    if (!$SkipWeb) {
        if (!(Get-Command node -ErrorAction SilentlyContinue)) { throw 'Node.js 22+ is needed to package the web app (or pass -SkipWeb).' }
        Say 'Building the web app (phones and browsers)'
        . (Join-Path $repo 'crates/web/scripts/install-tools.ps1')
        & (Join-Path $repo 'crates/web/scripts/build.ps1')
    }
}
if (!(Test-Path -LiteralPath $serverExe)) { throw "No server build at $serverExe. Run without -SkipBuild." }
$withWeb = !$SkipWeb -and (Test-Path -LiteralPath (Join-Path $webDist 'index.html'))

# ---- Install ------------------------------------------------------------------
Say "Installing to $InstallDir"
Stop-StickTalk $InstallDir $TaskName
foreach ($d in 'bin', 'data', 'logs', 'web') { New-Item -ItemType Directory -Force (Join-Path $InstallDir $d) | Out-Null }
Copy-Item -LiteralPath $serverExe -Destination (Join-Path $InstallDir 'bin/sticktalk-server.exe') -Force
# Keep earlier hashed web assets: open tabs may still request them.
if ($withWeb) { Copy-Item -Path (Join-Path $webDist '*') -Destination (Join-Path $InstallDir 'web') -Recurse -Force }
$webArg = if ($withWeb) { ' --web-root "%~dp0web"' } else { '' }
$envFile = Join-Path $InstallDir 'server.env'
if (!(Test-Path -LiteralPath $envFile)) {
    @'
# Optional StickTalk server settings, one KEY=VALUE per line; lines starting with # are ignored.
# Same names as /etc/sticktalk.env in deploy/ORACLE.md (Google sign-in, GIFs, upload limits...).
# The address, port, data folder and voice settings are managed by setup-windows.ps1:
# re-run it to change those. Restart after editing: Stop-ScheduledTask / Start-ScheduledTask.
# STICKTALK_MAX_TOTAL_UPLOAD_MB=10240
'@ | Set-Content -LiteralPath $envFile -Encoding Ascii
}
$launcher = Join-Path $InstallDir 'run-server.cmd'
@"
@echo off
rem Generated by deploy/self-host/setup-windows.ps1. Re-run setup to change settings.
setlocal
set RUST_LOG=sticktalk_server=info
set LOG=%~dp0logs\server.log
rem Optional KEY=VALUE settings (Google, GIFs, limits...). Flags below take precedence.
if exist "%~dp0server.env" for /f "usebackq eol=# tokens=1* delims==" %%A in ("%~dp0server.env") do set "%%A=%%B"
:run
for %%F in ("%LOG%") do if %%~zF GTR 10485760 move /y "%LOG%" "%LOG%.1" >nul
"%~dp0bin\sticktalk-server.exe" --bind 127.0.0.1:$Port --public-url $PublicUrl --data-dir "%~dp0data"$webArg --voice-bind-ip $voiceBind --voice-advertised-ip $voiceAdvertised >> "%LOG%" 2>&1
rem Restart after a crash (a few seconds' pause). Stopping the task ends this loop.
waitfor /t 5 StickTalkRestartDelay >nul 2>&1
goto run
"@ | Set-Content -LiteralPath $launcher -Encoding Ascii
# Remembered so re-runs and uninstall-windows.ps1 find this install's own routes.
@{ Backend = $backend; Port = $Port; TaskName = $TaskName; PublicUrl = $PublicUrl } | ConvertTo-Json | Set-Content -LiteralPath $stateFile -Encoding Ascii

# ---- Keep it running ----------------------------------------------------------
$user = "$env:USERDOMAIN\$env:USERNAME"
if ($NoStartupTask) {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    Say "Starting once (no startup task): $launcher"
    Start-Process -FilePath "$env:SystemRoot\System32\conhost.exe" -ArgumentList '--headless', $env:ComSpec, '/d', '/c', "`"$launcher`"" -WindowStyle Hidden
} else {
    Say "Registering startup task '$TaskName' (runs hidden whenever you sign in to Windows)"
    $action = New-ScheduledTaskAction -Execute "$env:SystemRoot\System32\conhost.exe" -Argument "--headless $env:ComSpec /d /c `"$launcher`"" -WorkingDirectory $InstallDir
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew
    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description "StickTalk server at $PublicUrl" -Force | Out-Null
    Start-ScheduledTask -TaskName $TaskName
}
if (!(Wait-Http "$backend/api/v1/health" 60)) { throw "The server did not start. See $(Join-Path $InstallDir 'logs\server.log')." }
Say 'Server is running locally'

# ---- Publish ----------------------------------------------------------------------
if ($tsPort) {
    $verb = if ($Exposure -eq 'Funnel') { 'funnel' } else { 'serve' }
    Say "Configuring Tailscale $verb on HTTPS port $tsPort"
    $serve = Get-ServeConfig $ts
    if (Test-OurPort $serve $tsHost $tsPort $backend) { Clear-TailscalePort $ts $serve $tsPort }
    if (Invoke-Tailscale $ts @($verb, '--bg', "--https=$tsPort", $backend)) { throw "tailscale $verb failed. Enable HTTPS certificates (and Funnel) for your tailnet in the Tailscale admin console; see deploy/SELF_HOST.md." }
}
if ($PublicUrl -ne $backend -and !(Wait-Http "$PublicUrl/api/v1/health" 90)) {
    Write-Warning "$PublicUrl is not answering yet. A new Tailscale certificate can take a minute; DNS for a new Funnel address can take up to 10 minutes."
}
Write-Host ''
Write-Host 'StickTalk is self-hosted.' -ForegroundColor Green
Write-Host "  Web app / desktop client server address: $PublicUrl"
Write-Host "  MCP connector URL for AI assistants:     $PublicUrl/mcp"
Write-Host "  Data: $(Join-Path $InstallDir 'data')   Logs: $(Join-Path $InstallDir 'logs\server.log')"
if ($PSBoundParameters.ContainsKey('PublicUrl') -or ($tsPort -and $Exposure -eq 'Funnel')) {
    Write-Host '  Registration is open: anyone with the address can create an account. Share it only with friends.' -ForegroundColor Yellow
}
Write-Host "  Stop or remove: powershell -ExecutionPolicy Bypass -File `"$(Join-Path $PSScriptRoot 'uninstall-windows.ps1')`" -InstallDir `"$InstallDir`""
