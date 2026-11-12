# Vendored wrapper provenance

Source: `webrtc-audio-processing`2.1.0 from crates.io, upstream Git
`c14d7af1760baff83e8210fee336a0cae0faaa7d`, https://github.com/tonarino/webrtc-audio-processing.
Original `COPYING` license text preserved (trailing whitespace normalized).

Changes:
- sys dependency aliases published `pocketstation-webrtc-audio-processing-sys`2.1.1
  for source-bundled MSVC portability and aggregate-output ABI fixes;
- examples/dev dependencies omitted from this source-only vendor;
- `Processor::set_stream_delay_ms` exposes its existing runtime atomic delay
  setting without reapplying/resetting APM config;
- `verify_native_layout` calls the sys package's native aggregate layout witness;
- workspace rustfmt normalization only, no changes to native DSP algorithms.

Native WebRTC/Abseil source comes from the pinned sys package/build subproject,
not an opaque downloaded DLL. Windows symbols are unprefixed: link only one APM
implementation. See media README for per-process build tools/static CRT and the
libclang18 parsing workaround. No source registry cache files are patched.
