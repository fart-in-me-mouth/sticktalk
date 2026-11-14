"""Summarize the actual checked build and headless measurements."""
from pathlib import Path
import os
import json

root = Path(__file__).resolve().parents[1]
out = root / 'screenshots'
proof = json.loads((out / 'browser-results.json').read_text(encoding='utf8'))
audits = json.loads((out / 'lighthouse-summary.json').read_text(encoding='utf8'))
sizes = proof['sizes']
wasm = next(item for item in sizes['assets'] if item['file'].endswith('.wasm'))
fonts = sum(item['bytes'] for item in sizes['assets'] if item['file'].endswith('.woff2'))
mobile = audits['mobile']['categories']
desktop = audits['desktop']['categories']
text = f'''# Web shell implementation proof

## Build and budgets

- Leptos CSR, isolated `crates/web` workspace, Trunk release, wasm-opt `-Oz`.
- Content revision: `{sizes['revision']}`.
- Chromium {proof['browser']}, headless only. Test and debugging servers bind **127.0.0.1 only**.

| Measurement | Actual bytes | Budget |
|---|---:|---:|
| wasm, uncompressed | {wasm['bytes']:,} | — |
| wasm, Brotli quality 11 | {wasm['brotli']:,} | 1,500,000 |
| Complete precached shell transfer, including worker | {sizes['firstLoadBytes']:,} | 2,500,000 |
| Measured cold first load including shell installation | {proof['firstLoad']['transferBytes']:,} | 2,500,000 |
| All built source assets, uncompressed | {sizes['raw']:,} | — |
| Bundled WOFF2 fonts (all four subsets) | {fonts:,} | included above |

Cold-load bytes are the response bodies actually sent by the loopback test
server, including duplicate app-shell installation requests ({proof['firstLoad']['requestCount']} requests).
HTTP headers are excluded. This is intentionally more conservative than the
single-copy precache inventory. The test server negotiates Brotli.

## Lighthouse

| Headless chat route | Performance | Accessibility | Best practices |
|---|---:|---:|---:|
| Mobile, simulated throttling | {mobile['performance']} | {mobile['accessibility']} | {mobile['best-practices']} |
| Desktop, 1440×900 | {desktop['performance']} | {desktop['accessibility']} | {desktop['best-practices']} |

Current Lighthouse: **{audits['mobile']['version']}**. Legacy PWA category:
**{audits['pwaLegacy']['categories']['pwa']}**, Lighthouse **{audits['pwaLegacy']['version']}**.
The PWA category was removed in Lighthouse 12; it is not a current-version score.

Mobile metrics: {json.dumps(audits['mobile']['metrics'], ensure_ascii=False)}.

Desktop metrics: {json.dumps(audits['desktop']['metrics'], ensure_ascii=False)}.

Raw evidence: [mobile](lighthouse-mobile.json), [desktop](lighthouse-desktop.json),
[legacy PWA](lighthouse-pwa-legacy.json), [audit summary](lighthouse-summary.json),
[browser results and size inventory](browser-results.json).

## Screenshots and review

**{len(proof['screenshots'])} PNGs**, each dimension validated, at **1440×900,
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

Rust build output used `{os.environ.get("CARGO_TARGET_DIR", root.parents[1] / "target")}`; native
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
'''
(out / 'REPORT.md').write_text(text, encoding='utf8')
print(out / 'REPORT.md')
