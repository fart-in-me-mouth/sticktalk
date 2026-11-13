# Local dependency changes

Original licenses are retained in each dependency.

## pocketstation-webrtc-audio-processing-sys 2.1.1

Source-vendored from the exact locked registry package. In addition to that
package's upstream Windows support:

- Discover MSVC compiler environment with cc's installed-tool discovery, inside
  the build script, instead of requiring a pre-sourced VsDevCmd shell.
- If present, discover the documented portable Meson/Ninja/libclang bundle at
  `%USERPROFILE%/.cargo-tools/sticktalk-call-hq/python`. An already configured
  compiler/libclang environment remains supported. No system PATH/registry edits.
- Short native source/build directories (`w`, `b`) inside Cargo OUT_DIR prevent
  MSVC MAX_PATH errors with a normal task target. No second hidden target.
- Ninja parallelism follows Cargo NUM_JOBS (workspace default four).

The normal PowerShell build still requires installed VS 2022 C++/Windows SDK and
Python tools (Meson 1.7.2, Ninja 1.13.0, libclang 18.1.1 portable bundle as in
media README). This fixes discovery; it does not download or install tools
implicitly. libclang 18 parses bindgen headers only with its documented STL
version guard override; native C++ is built with supported MSVC. A newer native
libclang configured via LIBCLANG_PATH is preferred for distribution builds.

Workspace CMAKE_GENERATOR defaults to Visual Studio 17 2022, overridable by the
caller. This fixes opusic-sys/cmake-rs guessing VS 2026 while the installed CMake
does not implement that generator. No Opus implementation/codec was replaced.

## neteq 0.9.1

Source-vendored from the exact locked registry package. Added `set_start_delay`
to NetEq/DelayManager. The host calls it only on initial stream registration and
device reset. Upstream always started at 80 ms even on clean links with a 20 ms
floor; the wrapper now starts at its configured floor, retaining the adaptive
quantile estimator, time stretching and 80 ms ceiling. Empty histogram quantiles
no longer undo the chosen startup target before the first 500 ms sample. Actual
packet ingress time, not decoder scheduling time, feeds the delay estimator.
This is not a claim of
Chromium NetEq equivalence. The wrapper also accepts both 10/20 ms Opus and uses
the actual packet sample count for decoding, FEC/PLC and jitter timestamps.

Vendored dependencies are excluded from first-party workspace targets, avoiding
unrelated demo/audio-server dependencies and device-opening upstream examples.

Audio round 2 adds a `NetEq::set_base_minimum_delay` delegation. The host lowers
the otherwise hard-coded 20 ms floor only after validating a 10 ms Opus packet,
and restores 20 ms for 20 ms packets. Explicit jitter multipliers remain separate.
This avoids silently clamping a requested 10 ms startup floor back to 20 ms.
The histogram uses 5 ms buckets (not a hidden 20 ms quantile floor). Buffer
filtering, duration reporting and RTP clocks count per-channel samples; decode
and time-stretch storage count interleaved samples. Accelerate allocations and
concealment overlap now respect this distinction. Stretch splice positions and
lengths are channel-aligned. A complete 10 ms leftover from a 20 ms packet is
played normally rather than spuriously concealed. First-party tests exercise
both packet durations through the native stereo decoder/mixer without devices.
