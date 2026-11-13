# Desktop UI audit

Branch: `t3/desktop-ui-polish-audit`
Scope: native client only. Call/voice panels, stream view, share picker, camera tiles, remote-control banner, media/SFU/web crates excluded.

## Fixed

- Reproducible 940×600 autocomplete panic: composer height no longer feeds back from the shrinking history viewport. Suggestions have a bounded, keyboard-following scroll area.
- Font-measured, grapheme-safe ellipsis replaces character-count guesses in server/category/channel labels, message authors/replies, member names/statuses, uploads, and settings identities. Labels cannot wrap over fixed-height neighboring rows.
- Reactions and pending attachment cards wrap instead of disappearing off the right edge.
- Quick switcher retains its search field, bounds the result list, scrolls the keyboard selection into view, shows conversation-type icons, and separates long server names from channel names.
- Home and Add Server remain accessible with 100+ servers; only the server icons scroll.
- Member list virtualizes fixed-height rows with four-row overscan, including correct name/status line heights. Huge-list fixtures retain 1,006 members without building 1,006 avatar/name widgets.
- Narrow emoji/GIF pickers keep their close/search/footer controls visible. Empty search, failed GIF fetch, and failed image fetch have clear recovery actions.
- Search results identify the actual scope (loaded messages in this conversation), include a count, and have an actionable empty state.
- Server-address configuration sits inside the scrollable login card with helper text; Enter advances login fields and completes username signup. Busy inputs are disabled. Fresh installations now require an explicit administrator-provided address rather than silently defaulting to localhost; saved addresses and packaged defaults still work.
- Minimum-height notification settings keep both the complete mute-duration dropdown and the Done footer visible. Category creation uses category-specific labels. Empty, disabled topic buttons no longer paint black strips.
- Invite creation cannot copy its loading placeholder or submit duplicate creation through the loading controls; failed creation exposes retry instead of claiming to still be loading.
- Profile popouts have relationship-aware Message/Add Friend/Accept/Unblock actions; own Edit Profile opens Profiles, not My Account. Friend-list More opens a context menu.
- Tab/Shift+Tab wrap inside dialogs, pickers, and popovers. Context menus support arrow navigation; closing restores composer focus. Alt+Up in an empty composer no longer also edits a message. Navigation/quick switching cannot discard unsaved settings.
- Image viewer nested menus and update confirmation get keyboard focus; Esc dismisses update confirmation and nested viewer menus.
- Edited suffix stays inline for paragraphs; toast and filename widths are bounded.
- Settings fixes: readable toggle descriptions, no empty description spacer, wrapping color swatches, narrower inner role navigation and single-line role tabs, ellipsized table identities, and one scroll surface on narrow server-profile pages.
- Filled danger actions and mention badges use accessible red; actions retain red on hover and use white labels at 5.70:1 (rest) / 5.05:1 (hover). Disabled secondary backgrounds correctly use 40% opacity.
- Expanded headless audit unifies the formerly separate main/settings suites; adds long names, huge lists, empty/error/busy/focus states, missing creation/confirmation forms, and 150% physical-pixel captures.

## Evidence and reproduction

Ten important before/after pairs are in `crates/client/screenshots/ui-audit/before/` and `after/`. The complete capture inventory and review notes are alongside them. Full intermediate matrices are kept locally, not added as hundreds of redundant binary assets.

```powershell
$env:CARGO_TARGET_DIR = "C:/Users/dwhee/.cargo-target/sticktalk-ui-audit"
$env:CARGO_INCREMENTAL = "0"
cargo build -p sticktalk
& "$env:CARGO_TARGET_DIR/debug/sticktalk.exe" --mock --screenshots <dir> --audit-only --sizes 940x600,1280x800,1920x1080
& "$env:CARGO_TARGET_DIR/debug/sticktalk.exe" --mock --screenshots <dir> --audit-only --sizes 1280x800 --scale-factor 1.5
```

`--screens name,name` selects a subset. The renderer traverses the real iced tree using offscreen tiny-skia; it opens no OS windows. Real GPU capture is available only in mock secondary-window mode with `--native-capture <png>`; `STICKTALK_AGENT_SCALE=1.5` tests application scaling without changing Windows display settings.

## Validation

- Captured/reviewed 190 screen/state fixtures at three logical sizes (570 PNGs), plus 16 scaled and two native GPU captures. Ten before/after pairs and a hashed inventory are retained.
- `cargo test -p sticktalk`: 112 passed, one existing hardware/file-video benchmark ignored. Includes 89 desktop binary tests, 17 library tests, five live integration tests, and one native audio transport test.
- `cargo clippy -p sticktalk --all-targets -- -D warnings` and `cargo fmt --all -- --check`: passed. Existing dependency/build-script warnings remain outside the touched crate.
- Updated the pre-existing live Google-auth test to use fresh one-use challenges, operation-bound reauthentication proofs, and the rotated password session. No authentication/server behavior was weakened; the mock authorization server and all listeners remain loopback-only, with no browser launch.
- Native debug mock idle over 60 seconds: 0.078% of one core; working set 140.8 MiB; private bytes 165.6 MiB. Secondary captures did not activate the client. Owned windows were closed.
- One non-incremental target directory stayed below the 10 GB budget (observed peak 9.29 GiB) and was removed after validation. No release build, UAC, firewall prompt, push, or out-of-scope crate change.

## Limits / not claimed

- Native Windows input automation could not connect to its helper (`failed to connect native pipe`, OS error 2), including retry and kernel reinitialization. Secondary-window creation was verified without activation. Real pointer-hover, drag-resize, right-click, and physical keyboard/DPI interaction checks are **not claimed**; native iced event/layout tests and offscreen 100%/150% captures cover the implemented paths instead.
- Search still searches loaded channel history; this audit does not add a server-wide search endpoint.
- Existing unavailable privacy/emoji/audit-log controls remain honestly unavailable; implementing backend features is outside client polish scope.
- The mandated muted token remains only 3.67:1 on chat, and prescribed offline/muted opacity plus arbitrary role colors can fail AA. Instructions/actionable descriptions now use secondary text. Changing those global contract colors requires an explicit spec decision.
- Deliberate accessibility deviation: filled destructive surfaces use darker semantic red tokens, retaining the specified bright danger color for foregrounds and dividers.
- The measured debug wgpu mock window exceeded the ~100 MB working-set target; no RAM improvement is claimed. No polling or animation subscriptions were added.
