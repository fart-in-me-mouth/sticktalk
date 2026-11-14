$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../../../..')).Path
$binary = 'C:/Users/dwhee/.cargo-target/sticktalk-qa-calls/server/sticktalk.exe'
$output = Join-Path $root 'crates/client/screenshots/qa-calls'
$scenes = 'focused-stream,stream-stats,share-picker-windows,share-quality,controlling-banner,request-control,camera-tiles,camera-chooser,voice-call,share-picker,voice-consent,call-empty,call-solo,call-nine,call-muted,call-streaming,voice-quality,call-controls-hover,camera-grid-1,camera-grid-2,camera-grid-3,camera-grid-4,camera-grid-5,camera-grid-6,camera-grid-7,camera-grid-8,camera-grid-9'
function Render($dir, $extra) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $args = @('--mock','--screenshots',('"'+$dir+'"')) + $extra
    $process = Start-Process -FilePath $binary -ArgumentList $args -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $dir 'render.log') -RedirectStandardError (Join-Path $dir 'render.stderr.log')
    $process.WaitForExit()
    $result = $process.ExitCode
    $process.Dispose()
    if ($result -ne 0) { throw "Headless render failed: $result" }
}
Render $output @('--screens',$scenes)
Render (Join-Path $output 'audio') @('--audio-only')
Render (Join-Path $output 'minimum') @('--sizes','940x600','--screens','camera-grid-9,request-control,call-streaming,share-quality')
@{
    commit=(git -C $root rev-parse HEAD)
    binary_sha256=(Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
    utc=(Get-Date).ToUniversalTime().ToString('o')
    png_count=@(Get-ChildItem -LiteralPath $output -Recurse -Filter '*.png').Count
    rendering='offscreen iced / tiny-skia; no OS window'
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'provenance.json')
