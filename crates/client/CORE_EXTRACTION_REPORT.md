# B8 step 1: client-core extraction

## Layout / API

`crates/client-core` now owns `Model` and `ClientState`, reducers/history/read/
permissions/mentions, protocol DTOs and `protocol::run`, `Http<A>`, resumable
uploads, markdown AST/cache, timestamp formatting, OAuth PKCE/state/exchange,
and mock protocol/auth/REST fixtures plus their image. Native `crates/client`
keeps iced views/editor/geometry, bounded UI queues, image/file caches, keyring,
file dialogs, browser launch and OAuth loopback listener. No protocol changes.

Boundaries: `Transport`, `HttpTransport`, `Clock`, `Entropy`, cancellable
`Spawner`, streaming `UploadSource`, `RestService` and bounded `EventSink`.
Native adapters live in `client::platform`; feature `web` provides web-sys
WebSocket/fetch/Blob/Web Crypto/timer/spawn_local adapters. Resource references
and queue handles are generic rather than OS types. [Full API/boundaries](../client-core/README.md).

## Validation

Passed on x86_64-pc-windows-msvc:

- `cargo test -p sticktalk-client-core -p sticktalk`: **44 unit + 2 live e2e**.
  Includes 18 moved tests, deterministic transport/heartbeat/retry-expiry and
  upload-offset tests, native cancellation, all existing editor/history/layout
  checks and the real Phase 1/mock-Google/GIF/upload/reconnect harness.
- `cargo clippy -p sticktalk-client-core -p sticktalk --all-targets -- -D warnings`
- `cargo fmt --all -- --check`; `git diff --check`
- `cargo check -p sticktalk-client-core --target x86_64-pc-windows-msvc`
- `tools/check-client-core.ps1`: **wasm web check passed**.
- wasm `--no-default-features` check and wasm `--features web --all-targets`
  clippy with `-D warnings` passed. The wasm dependency tree has no Tokio,
  reqwest, tungstenite, iced or keyring. Portable code has no filesystem,
  threads, OS time or `Instant`.

The live harness and all OAuth mock listeners bind **127.0.0.1 only**, confirmed
in source and the active three-listener table. No elevation, firewall changes
or installers were used.

## Visual parity

**100/100 PNGs byte-identical; zero changed pixels**, 50 screens at 1280x800 and
1920x1080, using the actual iced tiny-skia view tree without an OS window.
[Hashes and baseline details](core-screenshot-comparison.json).
Baseline is parent source with only its mock clock frozen; the new headless-only
`STICKTALK_TEST_UNIX_MS` override freezes the same time. Normal desktop time is
unchanged. [1280 sample](screenshots/core-extraction/guild-chat-1280x800.png),
[1920 sample](screenshots/core-extraction/guild-chat-1920x1080.png). Full before/
after PNG sets are retained at the paths in the comparison JSON rather than
committing another 34 MB of unchanged images.

## Idle measurements

Windows **debug builds**, same default features, 1280x800 `--mock`; existing
sampler, ten-second warmup and 60 one-second samples. GPU = D3D12; software =
forced tiny-skia. Each native measurement used guarded non-primary placement
and closed immediately afterward. [Placement evidence](core-window-placement.json)
confirms no focus/foreground changes. The owner continued using the PC.

| Run | Mean working set MiB | Mean private commit MiB | One-core idle CPU | Raw |
|---|---:|---:|---:|---|
| GPU before | 164.30 | 159.66 | 0.102% | [perf-core-before-gpu.json](perf-core-before-gpu.json) |
| GPU after | 176.13 | 154.25 | 0.077% | [perf-core-after-gpu.json](perf-core-after-gpu.json) |
| GPU repeat before | 141.48 | 154.48 | 0.231% | [perf-core-repeat-before-gpu.json](perf-core-repeat-before-gpu.json) |
| GPU repeat after | 152.01 | 154.30 | 0.122% | [perf-core-repeat-after-gpu.json](perf-core-repeat-after-gpu.json) |
| Software before (default features) | 47.08 | 12.54 | 0.000% | [perf-core-before-tiny-default.json](perf-core-before-tiny-default.json) |
| Software after (default features) | 70.61 | 11.58 | 0.000% | [perf-core-after.json](perf-core-after.json) |

Idle CPU remains near zero and private commit is flat/reduced. **Working-set RAM
parity is not established**: after samples are higher, with substantial run-to-
run residency variation even for the identical baseline binary. No new idle
polling, animation, runtime, retained-body or queue budget was added, but these
uncontrolled debug measurements cannot prove unchanged RAM. The existing
<60 MB target is not met by the default GPU renderer; no renderer changes were
made to mask that pre-existing limitation. Controlled release profiling remains
needed. `perf-core-before.json` is an earlier *no-default-features* software
sample and is intentionally not paired with the default-feature after sample.

Final measured native binary: 55,364,608 bytes (debug), SHA-256
`41B83F0DACA6813C28591CD71B1E6F7669A30A19CDC4EEDDFD3FD549D20FF89B`.

## Not included

Leptos/PWA UI, browser session/CSRF server integration and web-redirect OAuth
are subsequent B8 work. Browser adapters were compile/clippy checked, not tested
in a real browser (no browser or owner-screen window was opened for screenshots).
Release resource-budget certification and proof of RAM parity remain unverified
for the reasons above. No visuals or renderer behavior were intentionally changed.
