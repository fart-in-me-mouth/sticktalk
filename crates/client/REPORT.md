# StickTalk client — lead-review handoff

Scope: client-only source/assets/tools/reports and client dependency resolution in Cargo.lock. No proto/server edits; no git commits. Build output stayed in `C:/Users/dwhee/.cargo-target/sticktalk-client`.

## A1–A9

| Item | Status / implementation |
|---|---|
| A1 Copy | Done: server wording; marketing/tagline/security-store copy removed; Active Now / “It's quiet for now.”; natural prose wrapping; functional Credits heading. |
| A2 Login/Register | Done: centered Semibold primary labels, field-aligned muted prefix + accent Register link, blank password placeholder, Register-only helper; remembered server wins over compile-time `STICKTALK_DEFAULT_SERVER`, hidden behind Change server. |
| A3 Google | Done: `/api/meta` + matching build ID/nonempty secret gate; official multicolor mark in SVG wrapper, full-width secondary button/divider on both forms; random IPv4 loopback, PKCE S256/state, bounded parser, system-browser handoff, close-tab HTML, callback foreground request before token response, token exchange and `/auth/google`; username ticket completion; linked/unlinked methods, Link/Unlink, Set/Change password, LastLoginMethod; cancel/2-minute timeout. Stateful mock flows and mock authorization/token-server tests; no real browser/focus side effects in tests. |
| A4 Messages | Partial: 2px mention inset; no empty rich-text block before attachments, 8px text-to-attachment gap; floating More/React/Reply/Edit/Delete controls, own Edit; context quick reactions/+ opens the separate picker, Reply/Edit/Copy Text/Copy Message Link/Mark Unread/Delete; outside/Esc dismissal and keyboard navigation. **The picker currently contains nine bundled choices, not a full Unicode catalog/search.** Additional catalog/search/coverage work remains. **Mark Unread is local to the client session**: protocol v1 exposes only forward MarkRead; no forbidden proto/server change was made. |
| A5 Markdown | Done: cached client-side bold/italic/underline/strike/code/fences/quotes/spoilers/headers/lists/URLs/masked links/escapes; JetBrains Mono OFL subset; required code surfaces, border/radii/padding and horizontal code scrolling. Masked links show the actual URL in confirmation; URLs use the system browser (mocked in tests). |
| A6 Modals | Done: shared 70% black backdrop; right-aligned form footers; dashed camera/Upload circle and username-prefilled server name. Upload form revisions prevent late/cancelled server-icon results from applying to a different form; staged cancelled uploads are discarded. |
| A7 Settings | Done: full-screen sidebar/chat pages, right-aligned 218px nav block, content <=740px, circle close/ESC; member search + avatar/name/username/joined date and permission-gated danger Kick/Ban menus. |
| A8 Avatars | Done: initials scale at 0.4× avatar size; one letter at <=24px. |
| A9 Channel top | Done: beginning-of-#name header on the oldest page; `channel-top` proof at both sizes, without changing normal present-position scrolling. |

## Scrolling priority

Implemented GPU default/fallback, persistent shaped body/layout cache, keyed visible rows + overscan, indexed heights, static fast-drag/missing-history skeletons, landing-region keyset fetches, stale-response suppression and anchored replacements/prepends. Added the 5,000-message `scroll-5000` mock channel and window-free frame profiler. **Final measured render p99 remains above 4 ms; GPU RAM remains above ~90 MB; physical 280 Hz cadence is not certified. Unvisited arbitrary-history jumps need a server count/index API.** [Measurements and limitations](PERF.md).

## Proof

- **96 PNGs**, 48 states at 1280×800 and 1920×1080. All regenerated from the actual iced view tree with offscreen tiny-skia, no OS window. [Review / full list](UI_REVIEW.md), [dimension/hash manifest](screenshots/manifest.json).
- New requested states: google-login, pick-username, sign-in-methods, markdown, masked-link-confirm, context-own-message. Added scroll-5000, scroll-skeleton and expanded account/hover/settings cases.
- Passed: `cargo build -p sticktalk --release`; `cargo test -p sticktalk` (**35 passed**); `cargo clippy -p sticktalk --all-targets -- -D warnings`; `cargo fmt --all -- --check`. [Exact final commands](VALIDATION.txt). Earlier workspace formatting failures were resolved by the concurrent owners of those files; this task did not edit them.
- GPU wheel total p50/p99 **3.89/7.28 ms**; fast drag **1.19/5.63 ms**. Native GPU idle **135.42 MiB WS**, **0.051%** one-core CPU. [All before/after and fallback numbers](PERF.md).

## Remaining integration / deviations

The reaction picker is limited to nine bundled entries, not the requested full Unicode catalog/search. No live Google credentials/browser or real-server end-to-end certification. Mark Unread is session-local because v1 only supports monotonic read receipts. Google's approved gradient mark uses its official PNG fallback inside an SVG wrapper because resvg cannot render the source foreignObject/conic gradient. Arbitrary Unicode/assistive-technology coverage and OS picker/drop integration are not exhaustively tested. Compact continuation hover timestamps use HH:MM to avoid wrapping in the 40px gutter; tight-row action bars sit fully above text. [Asset provenance](ASSETS.md).

All automated screenshots and OAuth tests were window/browser-free. Native residency tests used only non-primary, no-activation mode and closed their own processes immediately after sampling; the owner's running preview was not touched.
