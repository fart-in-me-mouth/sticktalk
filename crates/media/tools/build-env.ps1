# Process-local build environment; no registry, installer or system PATH edits.
$env:CARGO_TARGET_DIR = 'C:/mhq'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CMAKE_GENERATOR = 'Visual Studio 17 2022'
$env:CARGO_INCREMENTAL = '0'
# Static native APM AND C/C++ CRT: no app-local VC runtime installer/DLL needed.
$env:RUSTFLAGS = '-C target-feature=+crt-static'
$env:CARGO_BUILD_JOBS = '4'
$toolRoot = Join-Path $env:USERPROFILE '.cargo-tools/sticktalk-call-hq'
$env:NASM = Join-Path $toolRoot 'nasm-2.16.03/nasm.exe'
$tools = Join-Path $toolRoot 'python'
$env:PYTHONPATH = $tools
$env:PATH = "$tools/bin;$env:PATH"
$env:LIBCLANG_PATH = "$tools/clang/native"
# Bindgen parses headers only; native DSP is compiled with supported MSVC.
$env:BINDGEN_EXTRA_CLANG_ARGS = "-D_ALLOW_COMPILER_AND_STL_VERSION_MISMATCH"
$vs = 'C:/Program Files/Microsoft Visual Studio/2022/Community/Common7/Tools/VsDevCmd.bat'
if (-not (Test-Path -LiteralPath $vs)) { throw 'VS 2022 developer environment is required' }
$env:PATH = 'C:/Program Files (x86)/Microsoft Visual Studio/Installer;' + $env:PATH
$command = '"' + $vs + '" -no_logo -arch=x64 -host_arch=x64 >nul && set'
& cmd.exe /d /s /c $command | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') { [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process') }
}
if ($LASTEXITCODE -ne 0) { throw 'Could not initialize the per-process MSVC environment' }
