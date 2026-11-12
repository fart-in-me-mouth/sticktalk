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

