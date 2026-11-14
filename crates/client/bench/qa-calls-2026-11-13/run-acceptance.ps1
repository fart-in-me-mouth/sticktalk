param(
    [ValidateSet('synthetic', 'wgc', 'wgc-motion')][string]$Source = 'synthetic',
    [ValidateSet('480', '720', '1080', '1440', 'source')][string]$Quality = '1080',
    [ValidateRange(10, 600)][int]$Seconds = 600,
    [ValidateSet(15,30,60)][int]$Fps = 60,
    [string]$Name = ''
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../../../..')).Path
Set-Location -LiteralPath $root
$env:CARGO_TARGET_DIR = $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { (Join-Path $PSScriptRoot '../../../../target') })
$env:CARGO_INCREMENTAL = '0'
if (Test-Path -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'debug')) {
    throw 'Clean the debug profile before optimized qualification; never retain both trees.'
}
# Use the existing optimized binary: no compiler, second target, window or
# background build can contaminate a measurement. The server profile retains
# unwinding required by the SFU containment boundary, without overriding it.
$binary = Get-ChildItem -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'server/deps') -Filter 'voice-*.exe' |
    Sort-Object LastWriteTime -Descending | Where-Object {
        # The server ALSO has tests/voice.rs. Never accept its zero-test exit
        # as a qualification pass after a full workspace validation build.
        @(& $_.FullName --list) -match '^live_calls2_two_viewers_ten_minutes: test$'
    } | Select-Object -First 1
if (-not $binary) { throw 'First build: cargo test --profile server -p sticktalk --test voice --no-run -j2' }
if (-not $Name) { $Name = "$Source-$Quality-${Fps}fps-$Seconds" }
if ($Name -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Output name must be a simple directory name.' }
$env:STICKTALK_VIDEO_TRACE = '1'
$env:STICKTALK_CALLS2_SOURCE = $Source
$env:STICKTALK_CALLS2_QUALITY = $Quality
$env:STICKTALK_CALLS2_SECONDS = "$Seconds"
$env:STICKTALK_CALLS2_FPS = "$Fps"
$env:STICKTALK_CALLS2_OUTPUT = Join-Path $PSScriptRoot $Name
New-Item -ItemType Directory -Force -Path $env:STICKTALK_CALLS2_OUTPUT | Out-Null
@{
    source = $Source; quality = $Quality; fps = $Fps; seconds = $Seconds; viewers = 2
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
$fixture = $null
if ($Source -eq 'wgc-motion') {
    $env:STICKTALK_AGENT_WINDOW = 'secondary'
    $fixtureBinary = Join-Path $env:CARGO_TARGET_DIR 'server/examples/motion_fixture.exe'
    if (-not (Test-Path -LiteralPath $fixtureBinary)) { throw 'Build sticktalk-media --example motion_fixture first' }
    $fixture = Start-Process -FilePath $fixtureBinary -ArgumentList @("$($Seconds+120)") -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $env:STICKTALK_CALLS2_OUTPUT 'fixture.log') -RedirectStandardError (Join-Path $env:STICKTALK_CALLS2_OUTPUT 'fixture.stderr.log')
    Start-Sleep -Seconds 3
    if ($fixture.HasExited) { throw 'Motion fixture failed; inspect fixture.stderr.log' }
}
try {
$process = Start-Process -FilePath $binary.FullName -ArgumentList @('--exact', 'live_calls2_two_viewers_ten_minutes', '--ignored', '--nocapture') -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
$result = $process.ExitCode
if ($null -eq $result) { throw 'Native test ExitCode unavailable; refusing false-green acceptance' }
$process.Dispose()
} finally { if ($fixture) { if (-not $fixture.HasExited) { Stop-Process -Id $fixture.Id }; $fixture.Dispose() } }
Get-Content -LiteralPath $stdout, $stderr | Set-Content -LiteralPath $log
if ($result -eq 0) {
    foreach ($role in @('sharer', 'viewer_b', 'viewer_c')) {
        $metrics = Join-Path $env:STICKTALK_CALLS2_OUTPUT "$role.metrics.json"
        if (-not (Test-Path -LiteralPath $metrics)) { throw "Missing $role measurements" }
        $data = Get-Content -LiteralPath $metrics -Raw | ConvertFrom-Json
        if ($data.role -ne $role -or $data.seconds -lt $Seconds) { throw "Incomplete $role measurement" }
        $minimum = ($data.fps_by_second | Measure-Object -Minimum).Minimum
        if (($data.frames / $data.seconds) -lt ($Fps * 58 / 60) -or $minimum -lt ($Fps * 45 / 60) -or [math]::Abs($data.drift_ms) -ge 20) { throw "FPS/drift gate failed for $role" }
        if ($role -eq 'sharer' -and $data.encoder_reconfigurations -ne 0) { throw 'Encoder reconfiguration thrash' }
    }
}
Get-ChildItem -LiteralPath $env:STICKTALK_CALLS2_OUTPUT -Filter '*.log' |
    Select-String -Pattern '^CALLS2|^capture stats:|^private_bytes' |
    ForEach-Object { $_.Line }
exit $result
