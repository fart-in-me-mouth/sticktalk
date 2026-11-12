param(
    [string]$Binary = "$env:USERPROFILE/.cargo-target/sticktalk-server/release/sticktalk-server.exe",
    [int]$Seconds = 60
)
$ErrorActionPreference = 'Stop'
$root = Join-Path $env:USERPROFILE '.cargo-target/sticktalk-server'
$data = Join-Path $root ('rss-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $data | Out-Null
$probe = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$probe.Start()
$port = $probe.LocalEndpoint.Port
$probe.Stop()
$server = Start-Process -FilePath $Binary -ArgumentList @('--bind',"127.0.0.1:$port",'--data-dir',$data) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $data 'stdout.log') -RedirectStandardError (Join-Path $data 'stderr.log')
try {
    $ready = $false
    for ($i=0; $i -lt 150; $i++) {
        if ($server.HasExited) { throw 'Server exited before health check.' }
        try { $null = Invoke-RestMethod "http://127.0.0.1:$port/api/v1/health"; $ready=$true; break } catch { Start-Sleep -Milliseconds 200 }
    }
    if (!$ready) { throw 'Server health check timed out.' }
    Start-Sleep -Seconds 2
    $cpuStart = (Get-Process -Id $server.Id).CPU
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $samples = for ($i=0; $i -lt $Seconds; $i++) {
        Start-Sleep -Seconds 1
        $p = Get-Process -Id $server.Id
        [double]$p.WorkingSet64
    }
    $cpuEnd = (Get-Process -Id $server.Id).CPU
    $watch.Stop()
    [pscustomobject]@{
        platform = [Runtime.InteropServices.RuntimeInformation]::OSDescription
        architecture = [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
        seconds = [math]::Round($watch.Elapsed.TotalSeconds,2)
        idle_rss_min_mib = [math]::Round(($samples | Measure-Object -Minimum).Minimum/1MB,2)
        idle_rss_max_mib = [math]::Round(($samples | Measure-Object -Maximum).Maximum/1MB,2)
        idle_rss_mean_mib = [math]::Round(($samples | Measure-Object -Average).Average/1MB,2)
        cpu_seconds = [math]::Round($cpuEnd-$cpuStart,4)
        percent_one_cpu = [math]::Round(100*($cpuEnd-$cpuStart)/$watch.Elapsed.TotalSeconds,3)
        executable_bytes = (Get-Item -LiteralPath $Binary).Length
        data_directory = $data
    } | ConvertTo-Json
} finally {
    # Only terminate the exact test process created above; never touch other servers.
    if (!$server.HasExited) { Stop-Process -Id $server.Id; $server.WaitForExit() }
}
