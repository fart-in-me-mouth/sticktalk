# webrtc-audio-processing-sys
[![Crates.io](https://img.shields.io/crates/v/webrtc-audio-processing-sys.svg)](https://crates.io/crates/webrtc-audio-processing-sys)
[![Docs.rs](https://docs.rs/webrtc-audio-processing-sys/badge.svg)](https://docs.rs/webrtc-audio-processing-sys)
[![Build Status](https://travis-ci.org/tonarino/webrtc-audio-processing.svg?branch=master)](https://travis-ci.org/tonarino/webrtc-audio-processing)
[![dependency status](https://deps.rs/repo/github/tonarino/webrtc-audio-processing/status.svg)](https://deps.rs/repo/github/tonarino/webrtc-audio-processing)

A wrapper around [PulseAudio's repackaging of WebRTC's AudioProcessing module](https://www.freedesktop.org/software/pulseaudio/webrtc-audio-processing/).

## Building

### Dynamic linking

By default the build will attempt to dynamically link with the library installed via your OS's package manager.

You can specify an include path yourself by setting the environment variable `WEBRTC_AUDIO_PROCESSING_INCLUDE`.

### Packages

```sh
sudo apt install webrtc-audio-processing-dev # Ubuntu/Debian
sudo pacman -S webrtc-audio-processing # Arch
```

### Static linking

Static linking can be enabled with the `bundled` feature flag.

The following tools are needed in order to use the `bundled` feature flag:

* meson
* ninja


## MSVC builds with the portability patch

Build on the target architecture from a Visual Studio developer environment
selecting the matching x64 or ARM64 compiler. Install Meson, Ninja, pkg-config,
LLVM/libclang and Rust's llvm-tools component. The supported qualification here
uses native runners; this patch does not establish cross-compilation.

Use Ninja 1.12 or later on Windows: earlier versions do not support the long
paths produced by nested Cargo/Meson builds and can repeatedly regenerate the
build manifest. The current CI candidate pins `meson==1.7.2 ninja==1.13.2` and
records their actual versions. Native Windows x64 debug/release qualification passes with that pin;
ARM64 qualification remains pending. The version requirement alone is not a
successful-build guarantee.
See [Ninja's long-path support](https://github.com/ninja-build/ninja/releases/tag/v1.12.0).

Native ARM64 requires `clang-cl.exe` from an ARM64 LLVM installation. The
unchanged engine uses GNU inline assembly for FPCR access, which `cl.exe` rejects.
Use clang-cl inside the matching Visual Studio developer environment so SDK,
linker and MSVC runtime discovery remain available; set both `CC` and `CXX` to
its absolute path. For example, after setting `LIBCLANG_PATH` to native LLVM:

```powershell
$env:CC = Join-Path $env:LIBCLANG_PATH 'clang-cl.exe'
$env:CXX = $env:CC
& $env:CXX --version # Target must be aarch64-pc-windows-msvc
```

Do not substitute an emulated x64 LLVM installation. The repository's
`ci/enter-msvc.ps1` verifies both libclang and clang-cl PE architecture, then
checks the compiler target. CI also records Meson's effective C/C++ compiler
identity and version and inspects emitted archives and executable CRT imports.
The ARM64 compiler correction requires native execution before qualification.
Windows x64 continues using `cl.exe`. See [clang-cl's SDK setup](https://clang.llvm.org/docs/UsersManual.html#clang-cl)
and [MSVC compatibility scope](https://clang.llvm.org/docs/MSVCCompatibility.html).

MSVC-compatible compilers also need short build paths. For a native build in PowerShell, use a fresh
output directory near the workspace drive root:

```powershell
$env:CARGO_TARGET_DIR = Join-Path (Split-Path -Qualifier (Get-Location).Path) "pks-$PID"
if (Test-Path $env:CARGO_TARGET_DIR) { throw 'Build directory already exists' }
$null = New-Item -ItemType Directory -Path $env:CARGO_TARGET_DIR
cargo test --features bundled,experimental-aec3-config --lib
```

Select the matching native Rust/MSVC toolchain first. This native command omits
the redundant `--target` directory component; it does not cross-compile. CI
verifies the Rust host against the requested architecture and inspects every
resulting executable/archive machine type. It validates Cargo's output against
the declared directory and reports both unresolved and normalized source path
lengths. It does not change machine-wide Windows long-path settings.

The wrapper and bundled engine use C++20 on MSVC and the same release CRT:
`/MD` by default or `/MT` when Cargo enables `crt-static`. Keep `WEBRTC_WIN`
consistent across the engine, wrapper and generated bindings. The patch does
not alter engine sources, AEC3 configuration or audio processing.

COFF symbol prefixing is disabled for both the engine and wrapper on MSVC.
Exactly one WebRTC APM implementation may be linked into each final native
binary/module. The sys crate retains `links = "webrtc-audio-processing-2"`,
which rejects competing Cargo packages with that same links key; it does not
detect differently named or manually linked APM libraries. Do not combine such
libraries without independently implementing and qualifying symbol isolation.
On ELF and Mach-O the existing prefixing remains enabled, and a missing wrapper
archive now fails explicitly. No archive is renamed on Windows, so cc's `.lib`
and `.a` aliases cannot diverge through partial symbol rewriting.

Windows x64 and ARM64 compile/link, installed import, engine tests, CRT inspection
and link-symbol inspection are required before this patch is qualified. Debug
and release configurations both need coverage. This patch alone is not evidence
of Windows runtime support.


### Rust/native aggregate boundary

C++ values with constructors cannot safely use bindgen's ordinary C aggregate
return convention on every architecture. This fork constructs stream and AEC3
configurations and statistics into explicitly supplied, aligned Rust storage.
C++ starts object lifetimes with placement construction; the value types must
remain trivially destructible. Native tests compare compiler sizes/alignment
and verify actual constructed fields, configuration validity and processing.
The top-level sys Rust function signatures are retained. Raw generated C++
namespace helper names change to `_into` functions; applications should use the
Rust wrapper API.

The six-target CI runs both sys and wrapper tests in debug and release. Its
cumulative build budget is 1,200 seconds and its job ceiling is 25 minutes,
including retained timeout cleanup. These are build limits, not audio processing
latency promises. ARM64 ABI qualification remains pending until all tests pass.

References: [Microsoft ARM64 return conventions](https://learn.microsoft.com/en-us/cpp/build/arm64-windows-abi-conventions?view=msvc-170#return-values)
and [bindgen's C++ calling-convention limitations](https://rust-lang.github.io/rust-bindgen/cpp.html).
