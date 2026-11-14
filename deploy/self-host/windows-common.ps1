# Helpers shared by setup-windows.ps1 and uninstall-windows.ps1 (dot-sourced).
function Get-Tailscale {
    $cmd = Get-Command tailscale -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $exe = Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
    if (Test-Path -LiteralPath $exe) { return $exe }
    throw 'Tailscale is not installed. Install it from https://tailscale.com/download/windows and sign in, then re-run.'
}
# Windows PowerShell 5.1 turns any native stderr line into a terminating error under
# ErrorActionPreference=Stop; run Tailscale with native errors judged by exit code.
function Invoke-Tailscale($ts, [string[]]$arguments, [switch]$Quiet) {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $out = & $ts @arguments 2>&1
        if (!$Quiet) { $out | ForEach-Object { Write-Host "$_" } }
        return $LASTEXITCODE
    } finally { $ErrorActionPreference = $saved }
}
function Get-ServeConfig($ts) {
    $json = (& $ts serve status --json 2>$null) -join "`n"
    if (!$json.Trim()) { return [pscustomobject]@{} }
    return $json | ConvertFrom-Json
}
function Get-Prop($object, [string]$name) {
    if ($null -eq $object) { return $null }
    $p = $object.PSObject.Properties[$name]
    if ($p) { return $p.Value }
    return $null
}
# True when this HTTPS port is served only by StickTalk's backend (ours to reuse).
function Test-OurPort($serve, [string]$tsHost, [int]$port, [string]$backend) {
    $web = Get-Prop (Get-Prop $serve 'Web') "${tsHost}:$port"
    if (!$web) { return $false }
    $handlers = @((Get-Prop $web 'Handlers').PSObject.Properties)
    return $handlers.Count -eq 1 -and $handlers[0].Name -eq '/' -and (Get-Prop $handlers[0].Value 'Proxy') -eq $backend
}
# Funnel can only use 443, 8443 and 10000. Never take a port another service uses.
function Select-TailscalePort($ts, [string]$tsHost, [string]$backend, [int]$requested) {
    $serve = Get-ServeConfig $ts
    $tcp = Get-Prop $serve 'TCP'
    $used = if ($tcp) { @($tcp.PSObject.Properties.Name) } else { @() }
    $candidates = if ($requested) { @($requested) } else { @(443, 8443, 10000) }
    foreach ($p in $candidates) { if (Test-OurPort $serve $tsHost $p $backend) { return $p } }
    foreach ($p in $candidates) { if ($used -notcontains "$p") { return $p } }
    throw "Tailscale HTTPS port(s) $($candidates -join ', ') are already used by other services on this machine (see 'tailscale serve status'). Free one, or pass -PublicUrl."
}
# Only called for a port Test-OurPort confirmed, so nothing else is touched.
function Clear-TailscalePort($ts, $serve, [int]$p) {
    $funnel = Get-Prop $serve 'AllowFunnel'
    if ($funnel -and @($funnel.PSObject.Properties | Where-Object { $_.Name -like "*:$p" -and $_.Value }).Count) {
        Invoke-Tailscale $ts @('funnel', "--https=$p", 'off') -Quiet | Out-Null
    }
    Invoke-Tailscale $ts @('serve', "--https=$p", 'off') -Quiet | Out-Null
}
function Remove-TailscaleRoutes($ts, [string]$tsHost, [string]$backend) {
    $serve = Get-ServeConfig $ts
    foreach ($p in 443, 8443, 10000) {
        if (Test-OurPort $serve $tsHost $p $backend) {
            Clear-TailscalePort $ts $serve $p
            Write-Host "Removed Tailscale HTTPS port $p"
        }
    }
}
function Test-MsvcLib {
    foreach ($d in ("$env:LIB" -split ';')) { if ($d -and (Test-Path -LiteralPath (Join-Path $d 'msvcrt.lib'))) { return $true } }
    return $false
}
# Load an x64 MSVC environment whose libraries are actually installed.
function Import-Msvc {
    if ((Get-Command link.exe -ErrorAction SilentlyContinue) -and (Test-MsvcLib)) { return }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (!(Test-Path -LiteralPath $vswhere)) { throw 'Install Visual Studio 2022 Build Tools with "Desktop development with C++" (see HANDOFF.md section 3).' }
    $vc = 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64'
    $paths = @(& $vswhere -products * -version '[17.0,18.0)' -requires $vc -property installationPath) + @(& $vswhere -products * -requires $vc -property installationPath)
    foreach ($p in ($paths | Where-Object { $_ } | Select-Object -Unique)) {
        $bat = Join-Path $p 'VC\Auxiliary\Build\vcvars64.bat'
        if (!(Test-Path -LiteralPath $bat)) { continue }
        cmd /c "`"$bat`" >nul 2>&1 && set" | ForEach-Object { if ($_ -match '^([^=]+)=(.*)$') { Set-Item -LiteralPath "env:$($matches[1])" -Value $matches[2] } }
        if (Test-MsvcLib) { return }
    }
    throw 'No complete x64 MSVC toolset found. Install Visual Studio 2022 Build Tools with "Desktop development with C++".'
}
function Stop-StickTalk([string]$InstallDir, [string]$TaskName) {
    Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    $launcher = [IO.Path]::GetFullPath((Join-Path $InstallDir 'run-server.cmd'))
    Get-CimInstance Win32_Process -Filter "Name='cmd.exe' OR Name='conhost.exe'" |
        Where-Object { $_.CommandLine -and $_.CommandLine.Contains($launcher) } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    $exe = [IO.Path]::GetFullPath((Join-Path $InstallDir 'bin\sticktalk-server.exe'))
    Get-Process sticktalk-server -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | ForEach-Object {
        Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
        $_.WaitForExit(10000) | Out-Null
    }
}
function Wait-Http([string]$url, [int]$seconds) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        try { if ((Invoke-WebRequest -UseBasicParsing -TimeoutSec 5 $url).StatusCode -eq 200) { return $true } } catch { }
        Start-Sleep -Milliseconds 500
    }
    return $false
}
