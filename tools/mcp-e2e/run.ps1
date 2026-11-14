# MCP connector end-to-end check with the official MCP TypeScript SDK client.
# Default: builds and starts a throwaway server on loopback (no windows, no browser).
# -Server <url> -DataDir <dir>: test an already-running (e.g. self-hosted) server;
#   the SDK flow seeds test data into that server's database, so only use a test instance.
# -Browser: also run browser.mjs (web app + consent page in headless Chromium).
# -PublicPath: with -Server on the hosting machine itself, resolve the host through
#   public DNS-over-HTTPS (Cloudflare) and pin Node and Chromium to that address, so traffic takes the
#   real internet path (e.g. Tailscale Funnel ingress) instead of the local tailnet.
param(
    [string]$SdkVersion = '1.32.2',
    [string]$PlaywrightVersion = '1.65.0',
    [int]$Port = 18080,
    [string]$Server,
    [string]$DataDir,
    [switch]$Browser,
    [switch]$PublicPath
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $root 'target' }
$env:CARGO_INCREMENTAL = '0'
$work = Join-Path $env:CARGO_TARGET_DIR 'mcp-e2e'
$deps = Join-Path $work 'deps'
New-Item -ItemType Directory -Force $deps | Out-Null
function Install-Package([string]$name, [string]$version) {
    $manifest = Join-Path $deps "node_modules/$name/package.json"
    if ((Test-Path $manifest) -and (Get-Content $manifest -Raw | ConvertFrom-Json).version -eq $version) { return }
    Push-Location $deps
    try {
        if (!(Test-Path package.json)) { npm.cmd init -y | Out-Null }
        # Third-party code: never run its install scripts.
        npm.cmd install --ignore-scripts --no-audit --no-fund "$name@$version"
        if ($LASTEXITCODE) { throw "$name install failed" }
    } finally { Pop-Location }
}
Install-Package '@modelcontextprotocol/sdk' $SdkVersion
$sdk = Join-Path $deps 'node_modules/@modelcontextprotocol/sdk/dist/esm'
if ($Browser) {
    Install-Package 'playwright' $PlaywrightVersion
    $env:PLAYWRIGHT_BROWSERS_PATH = Join-Path $env:CARGO_TARGET_DIR 'browsers'
    node (Join-Path $deps 'node_modules/playwright/cli.js') install chromium
    if ($LASTEXITCODE) { throw 'Chromium download failed' }
}

$proc = $null  # not $server: PowerShell names are case-insensitive (-Server)
if ($Server) {
    if (!$DataDir) { throw '-Server needs -DataDir (the test instance data directory, for seeding).' }
    $base = $Server.TrimEnd('/')
    $data = $DataDir
    if ($PublicPath) {
        $pinHost = ([Uri]$base).Host
        # DNS-over-HTTPS: Windows routes *.ts.net lookups to MagicDNS (NRPT) even
        # with Resolve-DnsName -Server, which would yield the tailnet address.
        $answer = Invoke-RestMethod -Headers @{ accept = 'application/dns-json' } "https://1.1.1.1/dns-query?name=$pinHost&type=A"
        $pinIp = @($answer.Answer | Where-Object { $_.type -eq 1 } | Select-Object -ExpandProperty data)[0]
        if (!$pinIp) { throw "Public DNS has no address for $pinHost" }
        Write-Host "Pinning $pinHost to public address $pinIp"
        $env:E2E_PIN_HOST = $pinHost
        $env:E2E_PIN_IP = $pinIp
        $env:NODE_OPTIONS = "--import " + ([Uri](Join-Path $PSScriptRoot 'pin-dns.mjs')).AbsoluteUri
        $env:E2E_HOST_RESOLVER_RULES = "MAP $pinHost $pinIp"
    }
} else {
    Push-Location $root
    try { cargo build --locked -p sticktalk-server --bin sticktalk-server; if ($LASTEXITCODE) { throw 'Server build failed' } }
    finally { Pop-Location }
    $base = "http://127.0.0.1:$Port"
    $data = Join-Path $work "data-$PID"
    New-Item -ItemType Directory -Force $data | Out-Null
    $exe = Join-Path $env:CARGO_TARGET_DIR 'debug/sticktalk-server.exe'
    $proc = Start-Process -FilePath $exe -NoNewWindow -PassThru -RedirectStandardError (Join-Path $work 'server.log') -ArgumentList @(
        '--bind', "127.0.0.1:$Port", '--public-url', $base, '--data-dir', $data,
        '--voice-bind-ip', '127.0.0.1', '--voice-advertised-ip', '127.0.0.1', '--min-free-disk-gb', '0')
}
try {
    $deadline = (Get-Date).AddSeconds(30)
    while ($true) {
        try { Invoke-RestMethod "$base/api/v1/health" | Out-Null; break }
        catch { if ((Get-Date) -gt $deadline -or ($proc -and $proc.HasExited)) { throw "Server at $base is not answering" }; Start-Sleep -Milliseconds 200 }
    }
    node (Join-Path $PSScriptRoot 'e2e.mjs') $base $sdk $data (Join-Path $PSScriptRoot 'seed.py')
    if ($LASTEXITCODE) { throw 'MCP end-to-end checks failed' }
    if ($Browser) {
        node (Join-Path $PSScriptRoot 'browser.mjs') $base $sdk (Join-Path $deps 'node_modules/playwright')
        if ($LASTEXITCODE) { throw 'Browser checks failed' }
    }
} finally {
    if ($proc) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        $proc.WaitForExit(5000) | Out-Null
        Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
    }
}
