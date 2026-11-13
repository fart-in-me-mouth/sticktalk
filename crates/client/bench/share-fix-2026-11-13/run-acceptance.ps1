param(
    [ValidateSet('synthetic', 'wgc')][string]$Source = 'synthetic',
    [ValidateSet('1080', '1440', 'source')][string]$Quality = '1080',
    [ValidateRange(10, 600)][int]$Seconds = 600,
    [string]$Name = ''
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../../../..')).Path
Set-Location -LiteralPath $root
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-share-fix'
$env:CARGO_INCREMENTAL = '0'
if (Test-Path -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'debug')) {
    throw 'Clean the debug profile before optimized qualification; never retain both trees.'
}
# Use the existing optimized binary: no compiler, second target, window or
# background build can contaminate a measurement. The server profile retains
# unwinding required by the SFU containment boundary, without overriding it.
$binary = Get-ChildItem -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'server/deps') -Filter 'voice-*.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $binary) { throw 'First build: cargo test --profile server -p sticktalk --test voice --no-run -j2' }
if (-not $Name) { $Name = "$Source-$Quality-$Seconds" }
if ($Name -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Output name must be a simple directory name.' }
$env:STICKTALK_VIDEO_TRACE = '1'
$env:STICKTALK_CALLS2_SOURCE = $Source
$env:STICKTALK_CALLS2_QUALITY = $Quality
$env:STICKTALK_CALLS2_SECONDS = "$Seconds"
$env:STICKTALK_CALLS2_OUTPUT = Join-Path $PSScriptRoot $Name
New-Item -ItemType Directory -Force -Path $env:STICKTALK_CALLS2_OUTPUT | Out-Null
@{
    source = $Source; quality = $Quality; seconds = $Seconds; viewers = 2
    commit = (git rev-parse HEAD); binary = $binary.Name
    binary_sha256 = (Get-FileHash -LiteralPath $binary.FullName -Algorithm SHA256).Hash
    profile = 'server'; utc_started = (Get-Date).ToUniversalTime().ToString('o')
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $env:STICKTALK_CALLS2_OUTPUT 'run.json')
$log = Join-Path $PSScriptRoot "$Name.log"
# PowerShell 5 treats native stderr as terminating errors under Stop, even
# with *> redirection. Trace output must never kill the coordinator and leave
# decoder children behind. Keep native streams out of PowerShell's error lane.
$stdout = Join-Path $PSScriptRoot "$Name.stdout.log"
$stderr = Join-Path $PSScriptRoot "$Name.trace.log"
$process = Start-Process -FilePath $binary.FullName -ArgumentList @('--exact', 'live_calls2_two_viewers_ten_minutes', '--ignored', '--nocapture') -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
$process.WaitForExit()
$result = $process.ExitCode
$process.Dispose()
Get-Content -LiteralPath $stdout, $stderr | Set-Content -LiteralPath $log
Get-ChildItem -LiteralPath $env:STICKTALK_CALLS2_OUTPUT -Filter '*.log' |
    Select-String -Pattern '^CALLS2|^capture stats:|^private_bytes' |
    ForEach-Object { $_.Line }
exit $result
