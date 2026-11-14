# Dot-source to stage Firefox's real Cisco H.264 encoder in the task cache.
# No installer, admin rights, registry changes, browser windows or fake codec.
$ErrorActionPreference = 'Stop'
if (!$env:CARGO_TARGET_DIR) { throw 'Set the one task CARGO_TARGET_DIR first.' }
if ($env:OS -ne 'Windows_NT' -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'This pinned helper is for Windows x64. Stage the matching official Mozilla OpenH264 GMP with MOZ_GMP_PATH on other hosts.'
}
$target = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
$plugin = Join-Path $target 'tools/gmp-gmpopenh264/2.6.0'
$archive = Join-Path $target 'tools/downloads/openh264-win64-652bdb7719f30b52b08e506645a7322ff1b2cc6f.zip'
# Mozilla toolkit/content/gmp-sources/openh264.json, OpenH264 2.6.0 Windows x64.
$sha512 = '77991110c1d195616e936d27151d02e4d957be6c20a4f3b3511567868b5ddffc6abbfdc668d17672f5d681f12b20237c7905f9b0daaa6d71dcdac4b38f2448b2'
New-Item -ItemType Directory -Force (Split-Path $archive), $plugin | Out-Null
if (!(Test-Path -LiteralPath $archive)) {
    Invoke-WebRequest 'https://ciscobinary.openh264.org/openh264-win64-652bdb7719f30b52b08e506645a7322ff1b2cc6f.zip' -OutFile $archive
}
if ((Get-Item -LiteralPath $archive).Length -ne 514215 -or (Get-FileHash -LiteralPath $archive -Algorithm SHA512).Hash -ne $sha512) {
    throw 'OpenH264 archive length/hash mismatch; do not load it.'
}
Expand-Archive -LiteralPath $archive -DestinationPath $plugin -Force
if (!(Test-Path -LiteralPath (Join-Path $plugin 'gmpopenh264.dll'))) { throw 'OpenH264 GMP DLL missing.' }
$env:MOZ_GMP_PATH = $plugin
Write-Output "Real OpenH264 2.6.0 staged: $plugin"
