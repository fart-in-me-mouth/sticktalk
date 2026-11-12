# B6 implementation report — 2026-11-12

## Changes / bugs fixed

- Shared the production transport, reducers, markdown and OAuth flow as a client
  library. Non-persisting authentication keeps scripted clients out of the
  owner's credential store/preferences.
- Normal uploads still used legacy multipart and a 25 MiB cap; saved originals
  also stopped at 25 MiB. Normal uploads now stream/hash through the resumable
  API. Explicit resume reads the server's durable offset; originals stream to
  disk without the obsolete cap. Legacy upload remains testable.
- Members/invites/bans continuation responses cleared the preceding page. They
  now carry the request's `after` cursor and append subsequent pages.
- Added wire-compatible optional `around` ID / `around_timestamp` history
  cursors, bounded indexed queries, both continuation directions, edge filling
  and deleted pivots. Added reply-preview **Jump to message** and header ID/date
  jumps. Explicit jumps replace the region with at most 50 fetched messages.
  Newer/older continuation works from it. Evicted-region reloads retain measured
  slots/scroll anchors. Stale normal/around/newer responses are rejected, and
  failed loads release the landing loader.
- Members rows reserve the same 36 px action column, including the owner.
- Explicit loopback-only Google issuer/JWKS and GIF mock configuration; unchanged
  production defaults. Public binds/origins, credentials, queries and fragments
  are rejected. Token contents never choose an upstream URL.

Each client bug has regression coverage, including 26 MiB transfer, metadata
continuations, stale/failed/region history loads and action-column widget sizes.

## Live E2E coverage

`tests/live.rs` drives three independent **production** subscriptions/reducers
against the actual TCP server with temporary SQLite/files. A second focused test
covers socket interruption/replay. No GUI/browser/sound/notification/keyring I/O.

- Register/login/invalid login/logout and revoked tokens.
- Mock Google authorization/code exchange with PKCE and generated RSA/JWKS;
  signup/existing login, audience/expiry/verification rejection, collision/ticket
  replay, identity ownership, password setup and link/unlink protection.
- Friends/presence; direct and three-party group messages, recipients, group
  ownership/leave, DM close/reopen; servers/invites/join/revoke, categories and
  channel create/update/delete, member snapshots, kick/ban/unban and ACL denial.
- Send/edit/delete/reply/reactions/mentions/markdown, typing, unread/read state.
- Before/after pagination, ID/time regions, 50-message bounds, invalid cursors,
  deleted/edge pivots and continuation in both directions.
- Legacy/resumable uploads, thumbnails, authorized ranges, durable-offset resume,
  conflict/idempotent completion and a streamed **26 MiB** upload/download.
- Socket cut, production backoff, offline delivery, same-stream resume/replay.
- Mock GIF trending/search/categories/items/pagination and private favorites.

Run `powershell -File tools/e2e/run.ps1` (or `-Full`). Real rate limits remain
enabled. Final live suite: **2 passed in 117.01 s**; no failing checks.

## Validation

Separate targets under `C:/Users/dwhee/.cargo-target/sticktalk-b6-` with suffixes
`client`, `server`, and `proto`.

- `cargo test -p sticktalk` — **42** (24 library, 16 binary, 2 live).
- `cargo test -p sticktalk-server` — **58** (48 unit, 10 integration).
- `cargo test -p sticktalk-proto` — **9** wire tests.
- `cargo clippy -p <each package> --all-targets -- -D warnings` — passed.
- `cargo fmt --all -- --check` — passed.
- `cargo run -p sticktalk -- --mock --screenshots crates/client/screenshots/b6`
  — **100 PNGs**, 50 states at both required sizes, offscreen tiny-skia.
  Reviewed both contact sheets and changed layouts at full size.

Logs: `B6_VALIDATION.txt`; screenshot dimensions/hashes:
`screenshots/b6/manifest.json`. No commits, SFU or voice-schema edits.

## Remaining / not fully completed

**The global scrollbar still represents visited slots, not all never-loaded
history.** Reply/ID/date jumps now reach unvisited regions in one bounded request;
evicted visited regions retain their anchors. Arbitrary dragging to an ordinal
in unknown history still needs a bounded extent/rank index, which an around
cursor alone cannot supply. The literal whole-history scrollbar part of task 4
is therefore **not claimed complete**.

No native GUI/live-provider credentials certification or new RAM/CPU performance
claim. External providers were deliberately mocked; `PERF.md` limitations remain.
