param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Output
)
$ErrorActionPreference = 'Stop'
$target = 'C:/Users/dwhee/.cargo-target/sticktalk-polish2'
$null = New-Item -ItemType Directory -Path (Split-Path $Output) -Force
$null = New-Item -ItemType Directory -Path "$target/probe-binaries" -Force
$hash = (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
$binary = "$target/probe-binaries/sticktalk-$hash.exe"
Copy-Item -LiteralPath $Exe -Destination $binary -Force
$saved = @{}
foreach ($name in @('STICKTALK_AGENT_WINDOW','LOCALAPPDATA','ICED_BACKEND','WGPU_BACKEND')) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    $env:STICKTALK_AGENT_WINDOW = 'secondary'
    $env:LOCALAPPDATA = "$target/probe-home"
    $env:ICED_BACKEND = 'wgpu'
    $env:WGPU_BACKEND = 'dx12'
    $placement = "$Output.placement.json"
    $arguments = "--mock --screen scroll-5000 --size 1280x800 --memory-bench `"$Output`" --agent-window-report `"$placement`""
    $process = Start-Process -FilePath $binary -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardError "$Output.stderr.txt" -RedirectStandardOutput "$Output.stdout.txt"
    if (!$process.WaitForExit(600000)) { throw 'Native resource probe timed out.' }
    if ($process.ExitCode -ne 0) { throw "Resource probe exited $($process.ExitCode)." }
    if (!(Test-Path -LiteralPath $Output)) { throw 'Resource probe did not write a report.' }
    if (!(Test-Path -LiteralPath $placement)) { throw 'Secondary placement was not confirmed.' }
    $position = Get-Content -LiteralPath $placement -Raw | ConvertFrom-Json
    if (!$position.shown -or !$position.foreground_unchanged) { throw 'Window placement/focus guard failed.' }
    @{ sha256=$hash; binary_bytes=(Get-Item -LiteralPath $binary).Length; arguments=$arguments; measured_at=(Get-Date -Format o) } | ConvertTo-Json | Set-Content -LiteralPath "$Output.binary.json"
    (Get-Content -LiteralPath $Output -Raw | ConvertFrom-Json).samples |
        Select-Object scenario,@{n='working_set_MiB';e={$_.working_set_bytes/1MB}},@{n='private_MiB';e={$_.private_bytes/1MB}},cpu_percent_one_core,messages_seen |
        Format-Table
} finally {
    if ($process -and !$process.HasExited) { Stop-Process -Id $process.Id }
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
}
