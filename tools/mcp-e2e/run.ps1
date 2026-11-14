# MCP connector end-to-end check with the official MCP TypeScript SDK client.
# Loopback only, no windows, no browser; a fresh database each run.
param([string]$SdkVersion = '1.32.2', [int]$Port = 18080)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $root 'target' }
$env:CARGO_INCREMENTAL = '0'
Push-Location $root
try { cargo build --locked -p sticktalk-server --bin sticktalk-server; if ($LASTEXITCODE) { throw 'Server build failed' } }
finally { Pop-Location }

$work = Join-Path $env:CARGO_TARGET_DIR 'mcp-e2e'
$deps = Join-Path $work 'deps'
$data = Join-Path $work "data-$PID"
New-Item -ItemType Directory -Force $deps, $data | Out-Null
$sdkPackage = Join-Path $deps 'node_modules/@modelcontextprotocol/sdk/package.json'
if (!(Test-Path $sdkPackage) -or (Get-Content $sdkPackage -Raw | ConvertFrom-Json).version -ne $SdkVersion) {
    Push-Location $deps
    try {
        if (!(Test-Path package.json)) { npm.cmd init -y | Out-Null }
        # Third-party code: never run its install scripts.
        npm.cmd install --ignore-scripts --no-audit --no-fund "@modelcontextprotocol/sdk@$SdkVersion"
        if ($LASTEXITCODE) { throw 'SDK install failed' }
    } finally { Pop-Location }
}

$base = "http://127.0.0.1:$Port"
$exe = Join-Path $env:CARGO_TARGET_DIR 'debug/sticktalk-server.exe'
$server = Start-Process -FilePath $exe -NoNewWindow -PassThru -RedirectStandardError (Join-Path $work 'server.log') -ArgumentList @(
    '--bind', "127.0.0.1:$Port", '--public-url', $base, '--data-dir', $data,
    '--voice-bind-ip', '127.0.0.1', '--voice-advertised-ip', '127.0.0.1', '--min-free-disk-gb', '0')
try {
    $deadline = (Get-Date).AddSeconds(30)
    while ($true) {
        try { Invoke-RestMethod "$base/api/v1/health" | Out-Null; break }
        catch { if ((Get-Date) -gt $deadline -or $server.HasExited) { throw "Server did not start; see $work/server.log" }; Start-Sleep -Milliseconds 200 }
    }
    node (Join-Path $PSScriptRoot 'e2e.mjs') $base (Join-Path $deps 'node_modules/@modelcontextprotocol/sdk/dist/esm') $data (Join-Path $PSScriptRoot 'seed.py')
    if ($LASTEXITCODE) { throw 'MCP end-to-end checks failed' }
} finally {
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    $server.WaitForExit(5000) | Out-Null
    Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
}
