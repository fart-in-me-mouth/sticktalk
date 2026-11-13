> Historical fixture-only baseline. Current real-server evidence and gaps: [live/REPORT.md](live/REPORT.md).

# Web shell implementation proof

## Build and budgets

- Leptos CSR, isolated `crates/web` workspace, Trunk release, wasm-opt `-Oz`.
- Content revision: `99d22dbe7ae1ce3bee7a`.
- Chromium 158.0.8127.18, headless only. Test and debugging servers bind **127.0.0.1 only**.

| Measurement | Actual bytes | Budget |
|---|---:|---:|
| wasm, uncompressed | 666,660 | — |
| wasm, Brotli quality 11 | 227,714 | 1,500,000 |
| Complete precached shell transfer, including worker | 417,712 | 2,500,000 |
| Measured cold first load including shell installation | 764,695 | 2,500,000 |
| All built source assets, uncompressed | 985,601 | — |
| Bundled WOFF2 fonts (all four subsets) | 84,144 | included above |

Cold-load bytes are the response bodies actually sent by the loopback test
server, including duplicate app-shell installation requests (48 requests).
HTTP headers are excluded. This is intentionally more conservative than the
single-copy precache inventory. The test server negotiates Brotli.

## Lighthouse

| Headless chat route | Performance | Accessibility | Best practices |
|---|---:|---:|---:|
| Mobile, simulated throttling | 87 | 96 | 100 |
| Desktop, 1440×900 | 100 | 91 | 100 |

Current Lighthouse: **12.8.2**. Legacy PWA category:
**100**, Lighthouse **11.7.1**.
The PWA category was removed in Lighthouse 12; it is not a current-version score.

Mobile metrics: {"first-contentful-paint": "2.9 s", "largest-contentful-paint": "3.2 s", "total-blocking-time": "130 ms", "cumulative-layout-shift": "0.004", "interactive": "3.2 s"}.

Desktop metrics: {"first-contentful-paint": "0.6 s", "largest-contentful-paint": "0.7 s", "total-blocking-time": "0 ms", "cumulative-layout-shift": "0", "interactive": "0.7 s"}.

Raw evidence: [mobile](lighthouse-mobile.json), [desktop](lighthouse-desktop.json),
[legacy PWA](lighthouse-pwa-legacy.json), [audit summary](lighthouse-summary.json),
[browser results and size inventory](browser-results.json).

## Screenshots and review

**140 PNGs**, each dimension validated, at **1440×900,
390×844, 412×915 and 820×1180**. All paths are in `crates/web/screenshots/`.
Open the [complete screenshot index](index.html); each thumbnail links to its
original PNG. `review-01.jpg` through `review-19.jpg` are contact sheets used
to review the whole matrix. Main screens, role tabs, profiles, devices, members,
modals, menus, loading and disconnected/empty states are covered.

Phone extras: `phone-left-panel-*`, `phone-right-panel-*`,
`phone-long-press-*`, and `phone-keyboard-open-*` for both phone dimensions.
Long-press and swipe use CDP touch input. **Keyboard proof is labeled emulation**
of the visualViewport contract, not an OS keyboard or physical-device capture.

Review corrections include server-rail pills, wrong server-profile initialization,
message-specific action sheets, one-to-one DM participants/details, tablet call
tile overflow, fitted stream viewing, and mobile-only share limitations.
No broken images, document horizontal overflow, or browser exceptions remain.

## Validation passed

```text
trunk build --release
cargo test                         (3 tests)
cargo clippy --all-targets -- -D warnings
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings
cargo fmt --all -- --check
node tests/browser.mjs
node tests/audit.mjs
python scripts/gallery.py
```

Rust build output used `C:/Users/dwhee/.cargo-target/sticktalk-web`; native
test/dev debug symbols and incremental compilation were disabled to avoid
exhausting the owner's disk. No other task's cache was touched.

## Not connected / not certified

- As requested: **fixture UI only**, no client-core/native-client dependency.
  Auth/Google, real sessions, message persistence, uploads/downloads, GIF search,
  WebRTC/media, push notifications and server actions need the later adapter.
- The crate is an isolated workspace to avoid changing files outside the assigned
  crate; integrating into the root workspace is a lead follow-up.
- iOS/Android installation and real keyboard, safe-area hardware, 4G interaction
  and 60-fps scrolling have not been certified on physical phones. The mobile
  Lighthouse interactive time above is the lab result, not a sub-2-second claim.
- Default spec colors intentionally remain unchanged. `text.muted`, small
  white-on-danger badges, and 30%-opacity offline members fail WCAG contrast;
  desktop split-device chevrons also use the prescribed 16px segment and fail the desktop target-size audit. Inspect the raw accessibility findings. Owner approval is needed to change
  these authoritative visual tokens.
