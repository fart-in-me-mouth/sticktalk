param([Parameter(Mandatory)][string]$Binary)
$ErrorActionPreference='Stop'
$out=Join-Path $PSScriptRoot '../screenshots/media'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$out=(Resolve-Path -LiteralPath $out).Path
$screens=@('emoji-picker','emoji-search','emoji-autocomplete','reaction-picker','gif-picker','gif-search','gif-favorites','image-viewer','image-viewer-loading','image-viewer-zoom','upload-progress','upload-storage-full','upload-other-channel','restart-update')
foreach($size in @('1280x800','1920x1080')) {
    foreach($screen in $screens) {
        $path=Join-Path $out "$screen-$size.png"
        $process=Start-Process -FilePath $Binary -ArgumentList @('--mock','--capture',('"'+$path+'"'),'--screen',$screen,'--size',$size) -WindowStyle Hidden -Wait -PassThru
        if($process.ExitCode -ne 0){throw "Headless capture failed: $screen ($($process.ExitCode))"}
    }
}
$files=Get-ChildItem -LiteralPath $out -Filter '*.png' | ForEach-Object { @{file=$_.Name;bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash} }
@{renderer='iced-tiny-skia';window_backend='none';files=$files} | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 (Join-Path $out 'manifest.json')
