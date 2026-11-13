# Non-voice launch QA

Branch: `t3/qa-text-launch-readiness`. Synthetic accounts only; all services bound to loopback. No pushes, external OAuth/browser, OS notifications/sounds, media/SFU/call-UI edits, or admin operations.

**Not a full launch certification.** Implemented flows are exercised against the real TCP/SQLite/blob server through the desktop transport and shared `client-core` reducer. Several requested features do not exist yet; they are explicitly listed below, not counted as passing.

## Fixed bugs

1. **Uploaded MP4/WebM files never became inline videos.** The server classified every non-image as octet-stream. Bounded container sniffing now preserves video MIME types without trusting extensions or claimed MIME; image-only avatar/icon rules remain enforced. Red/green regression with small deterministic FFmpeg fixtures. Existing stored octet-stream video metadata is not backfilled.
2. **Inline attachment playback rejected files over 128 MiB and predownloaded originals.** Attachment playback now uses a clip-scoped 256-bit loopback capability proxy, authenticated upstream Range/HEAD requests, streaming backpressure, bounded concurrency, and cancellation on decoder teardown. No bearer token in decoder URLs or persistent original-file playback cache. The bounded GIF preview cache is unchanged.
3. **Ordinary members had incorrect channel controls.** They cannot fetch raw ACLs, and the client ignored implicit send dependencies. A personalized `/channels/{id}/permissions` snapshot now supplies effective rights without exposing ACLs. Navigation/live channel updates refresh it; read-only composers and reaction/reply/edit/upload affordances honor it. Keyboard sends/reactions are guarded too. Category/member overrides and revocation are exercised end-to-end.
4. **Real long server names wrapped in the fixed-height header.** Actual shaped-width, grapheme-safe ellipsis replaces character-count guessing; results are bounded/cached. Tests cover wide/narrow/combining-character names and zero-width bounds.
5. **The device notification default was persisted but ignored.** The real message-event path now applies the local default when no server/channel override exists; explicit overrides retain precedence. Tests mock the OS effect.
6. **Real native clients could crash while reconciling rich-text messages.** The link wrapper shared the stateless/empty widget tag and indexed a nonexistent child. It now has its own state tag and reconciles children safely. A deterministic empty/stateless-tree regression failed before the fix; both real default-GPU clients then passed.
7. **Deep history pagination failed at the real REST rate limit.** The first 2,048-message soak reached every send/unread/cache assertion, then failed on HTTP 429 during pagination. Native and browser transports now honor bounded server backoff for safe empty GETs only (three retries, at most 10 seconds total, original request deadline retained). Mutations are never replayed. A 45-read real-server regression, policy unit tests, and the complete 2,048-message rerun pass.
8. **Developer mode used two inconsistent state flags.** Settings did not control newer context menus, and the rail toggle was not persisted. All menus now use the same persisted preference; a reducer/serialization regression verifies both directions.
9. **The saved Quick Switcher binding was ignored, and active shortcuts intercepted keybind recording.** Dispatch now matches the persisted non-voice shortcut exactly, and recording takes precedence over execution. Native event/reducer tests cover changed bindings, old-binding removal, extra modifiers, recording Ctrl+K, and Escape cancellation. Voice shortcut behavior is not modified in this task.

10. **Long unbroken drafts were clipped horizontally inside a tall, mostly blank composer.** The real error-toast capture exposed default word-only wrapping for pasted tokens/URLs. The editor now falls back to glyph wrapping. A rasterized real-widget regression painted zero later glyph scanlines before the fix and passes after it.

The existing comprehensive live test had stale OAuth assumptions (replayed login nonce, missing fresh reauthentication proofs, ignored password-session rotation). It now tests the current hardened flow, honoring actual auth rate limits rather than disabling them.

## Coverage

| Feature | Status | Evidence / remaining limitation |
|---|---|---|
| Register, login, invalid credentials, logout | Pass | Real REST/client-core, socket termination, token denial |
| Google login | Pass | Loopback authorization/JWKS/token mock, RSA signatures, PKCE/state, invalid claims, single-use nonce/signup ticket |
| Google link/unlink, set password | Pass | Fresh operation/session-bound proofs; last-login-method denial; rotated session |
| Change password; revoke other sessions | Pass | Real accounts test verifies old caller and second-device tokens/sockets revoked; unrelated users retain access |
| Session list and revocation | Pass | Opaque own-device IDs; cross-account denial; single/all-other logout |
| Account deletion | Pass | Fresh proof, revoked token, relationship removal, other clients resync without losing their accounts |
| Display name, avatar, bio, pronouns, custom status, banner color | Pass | Real upload/profile events/REST snapshots and reconnect |
| Banner image | Not-done | Profile protocol/UI supports a banner color, not an image |
| Online / idle / DND / invisible | Pass | Live reciprocal projection; invisible appears offline to peers, private desired status syncs to other devices |
| Server create/edit name/icon/delete/leave | Pass | Real three-user lifecycle, live fan-out, purpose-bound icon upload and clear |
| Invite create/max uses/revoke | Pass | Bounds, one-use exhaustion, join and revoked-code denial |
| Invite expiry after real elapsed time | Pass | Production-minimum TTL 60 seconds; real join after 61 seconds returns InviteExpired |
| Kick, ban/unban | Pass | Live removal, history/access denial, banned rejoin denial and later allowed rejoin |
| Server folders / rail ordering | Not-done | Not present in the implemented feature model |
| Text / voice-channel metadata / categories | Pass | Create/edit/topic/reorder/delete; category deletion rehomes children. No call or stream testing in this task |
| Slowmode / NSFW flag | Not-done | Absent from channel protocol and UI |
| Roles create/edit/reorder/color/assignment | Pass | Real settings E2E plus server hierarchy/escalation matrix |
| Channel overrides and server-side permissions | Fixed | Owner/member/outsider E2E; category denial, member allow, hidden-channel purge; existing service bit-matrix tests |
| Client permission affordances | Fixed | Personalized masks, disabled composer, hidden/disabled actions, shortcut tests, real-data read-only captures |
| Friends request/accept/decline/cancel/remove/block/unblock | Pass | Live tests plus real-server group/relationship policy suite |
| DMs and group DMs | Pass | Send/history, close/reopen, recipients, transfer/leave, block and nonparticipant denials |
| Message send/edit/delete/reply | Pass | Real reducer fan-out, cross-user denial, idempotent retries/restart receipts |
| @user / @role / @everyone / @here | Pass | Live user mention/unreads and server expansion/authorization tests; mention rendering in real desktop capture |
| Unicode reactions and pins | Pass | Personalized reactions, own/remove behavior, pin ACL/deletion consistency, real launch tests |
| Custom server emoji upload/use/reactions | Not-done | No custom-emoji protocol/storage/UI; Unicode-only emoji crate and reaction validation |
| Long names and unbroken drafts | Fixed | Shaped grapheme ellipsis and WordOrGlyph composer wrapping; native/raster regressions |
| Markdown, code blocks, spoilers, quotes, links | Pass | Shared parser tests, native layout tests, real-data message rendering. No claim of exhaustive language-grammar highlighting |
| Typing and read/unread state | Pass | Real events, mention counts, read reset, resume |
| Jump and bounded history pagination | Pass | Around-message/date and older/newer regions through real client-core |
| Thousands of messages / restart unreads | Pass | 2,048 real sends; all history IDs exactly once; jump around midpoint; retained cache ≤1,000 messages / 8 MiB; read at message 1,001 leaves 1,047 unreads after a fresh client. Complete gate: 2,051.16 s |
| Message search | Not-done | Current UI searches cached messages only; no full-history server search endpoint |
| Images / arbitrary files / streamed download | Pass | Real multipart/resumable uploads, thumbnail/original/ranges, atomic destination replacement; 26 MiB round trip |
| Upload >=2 GiB | Pass | Actually uploaded **2,147,483,649 bytes** to temporary server storage, then verified tail ranges and ACL revocation; 111.39 s upload/range verification in debug |
| Resume / upload progress / cancel | Pass | Real offset conflict/restart and upload worker/checkpoint tests; cancellation race/progress reducer tests. No manual multi-GB cancel timing claim |
| Inline video >128 MiB | Fixed | Native muted/windowless decoder produced 160x90 frame after **15,687,680 / 135,266,304 bytes**; real-server range tests |
| Image viewer / zoom | Pass | Existing native viewer/pan/zoom/navigation/cancellation tests and headless visual fixtures; not manually clicked in the two real-data windows |
| Emoji picker | Pass | Unicode artwork/search/skin-tone/autocomplete tests |
| GIF picker | Pass | Mock KLIPY categories/search/trending/favorites; no provider/CDN calls in QA |
| Server/channel notification settings and mute | Pass | Durable, device-synced REST/live events, precedence, expiry, inaccessible-scope filtering |
| Device notification defaults/sounds | Fixed | Local default now used; OS notification and sound dispatch mocked |
| Separate master desktop-notification switch | Not-done | UI has default levels and sound control, not an independent master enable switch |
| Preferences non-voice fields across an actual process restart | Pass | Two sequential real desktop processes, isolated home; every non-voice Preferences field changed from defaults and compared to actual settings.json and reloaded app state. Voice settings intentionally untouched |
| Developer mode / non-voice custom shortcut behavior | Fixed | One shared durable developer flag; saved Quick Switcher chord now executes and records correctly |
| Every setting's restart persistence / side effects | Not-done | GIF autoplay uses a separate feature cache and was not process-restart-tested. Privacy toggles are local preferences, not server-enforced account policy. Startup/OS notification/sound effects mocked; custom mute/deafen dispatch belongs to voice and is not changed |
| Real desktop, two accounts | Pass | Real `sticktalk.exe`, not `--mock`; both compose/send, render images/replies/reactions/mentions; secondary monitor, foreground unchanged, processes closed |
| Empty/read-only real-data UI, both sizes | Pass | Native 1280x800 and 1920x1080 captures inspected; correct read-only composer and real empty state |
| Loading/error/reconnect UI | Pass | Native error toast from an overlength composer send; actual cut TCP proxy produces captured reconnect banner; real replay/resume test; loading fixtures. No exhaustive fault-injection screenshot matrix |
| Idle working set/private bytes/CPU | Not-done | No release idle-performance claim; debug functional QA only |

## Validation and artifacts

The history sender uses a fresh genuine login every 200 durable commands to remain within the documented 256/session receipt cap; the recipient remains connected for all 2,048 messages, then reconnects to verify persistence. The real per-user 60/min send and 120/min REST limits are not disabled or injected away.

- One target directory only: `C:/Users/dwhee/.cargo-target/sticktalk-qa-text`; `CARGO_INCREMENTAL=0`; workspace profiles unchanged. Peak measured size 9.8 GiB; final build 7.57 GiB. `cargo clean --target-dir` removed 17,140 files / 7.6 GiB after validation; the target directory no longer exists.
- Native captures: `crates/client/screenshots/qa-live/`. Window reports verify a non-primary monitor and no focus activation. Captures use iced's native render-surface screenshot hook, not mock fixtures or screen-wide capture.
- Headless parity suite: 136 PNGs (68 fixture views × two sizes) in `crates/client/screenshots/qa-parity/`, with SHA-256 manifest. All 27 core UI-spec screens at both sizes visually reviewed in `qa-parity-review/` contact sheets. These supplement, not replace, the real-data captures.
- Deterministic video fixtures: `crates/server/tests/fixtures/clip.{mp4,webm}` (160x90, silent, generated locally with FFmpeg).
### Reproduction commands

```powershell
$env:CARGO_TARGET_DIR = "C:/Users/dwhee/.cargo-target/sticktalk-qa-text"
$env:CARGO_INCREMENTAL = "0"
cargo test -p sticktalk -p sticktalk-server -p sticktalk-client-core --no-fail-fast
cargo clippy -p sticktalk -p sticktalk-server -p sticktalk-client-core --all-targets -- -D warnings
cargo fmt --all -- --check
cargo check -p sticktalk-client-core --target wasm32-unknown-unknown --features web
# Individual opt-in gates (desktop gates require a secondary monitor):
cargo test -p sticktalk --test live files_above_two_gib_have_no_size_ceiling -- --ignored --exact --nocapture
cargo test -p sticktalk --test live thousands_of_messages_paginate_jump_and_keep_unreads_across_client_restart -- --ignored --exact --nocapture
cargo test -p sticktalk --test live desktop_two_real_clients_render_and_send -- --ignored --exact --nocapture
cargo test -p sticktalk --test live native_decoder_plays_video_above_128_mib_before_full_download -- --ignored --exact --nocapture
cargo test -p sticktalk --test preferences_live -- --ignored --nocapture
cargo test -p sticktalk --bin sticktalk video::qa_native_formats -- --ignored --nocapture
& "$env:CARGO_TARGET_DIR/debug/sticktalk.exe" --mock --screenshots crates/client/screenshots/qa-parity
```

### Final results (2026-11-13)

- **290 passed, 0 failed, 8 normally ignored** across the three touched crates, including real-server integrations. All six relevant opt-in gates above were also run separately and passed. The two other ignored gates were not run: `file_1080p60_decodes_at_least_58_fps_for_sixty_seconds` and `hybrid_multigb_stream_flat_rss` (pre-existing codec/storage memory performance acceptance).
- Strict Clippy, workspace format check, and browser/WASM shared-core compilation passed. Third-party dependency/build-script warnings remain; no project Clippy warnings were waived.
- Two real desktop clients compose/send successfully; eight real-data captures include both empty/read-only sizes, actual validation feedback, and a real disconnected TCP proxy. Native MP4 and WebM fixtures both decode 160x90 frames. The >128 MiB decoder gate begins rendering after 15,687,680 of 135,266,304 bytes.
- The 2 GiB + 1-byte upload/range/ACL gate passed (111.39 s upload/range verification); the 2,048-message history/restart gate passed (2,051.16 s total). These are debug functional timings, not release performance claims.
- All 136 parity PNG hashes/dimensions verified. Refreshed core contact sheets and real-data captures were visually inspected. All native QA processes closed; the exact task target directory was deleted after the final validation.

No tests in this report should be interpreted as proving absent features.
