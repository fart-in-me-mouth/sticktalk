# StickTalk server — Phase 1 implementation

`sticktalk-server` implements the Phase 1 REST routes, WebSocket commands and
events in [`docs/PROTOCOL.md`](../../docs/PROTOCOL.md), using the existing public
`sticktalk-proto` types. Optional Google auth uses additive REST types without
changing existing wire shapes; see the protocol Google authentication section.

## Implemented

- Optional Google RS256/JWKS sign-in, hashed ten-minute signup tickets, explicit
  account linking/unlinking, login-method status and password setup/replacement.
- Registration/login/logout, Argon2id credentials, hashed opaque 30-day sessions,
  account/profile updates, friends/requests/blocks and effective presence.
- Direct and group DMs, recipient/ownership management, guilds, invitations,
  owner/admin/member permissions, moderation, categories and text channels.
- Messages/replies/explicit mentions, edits, tombstones, personalized Unicode
  reactions, typing, read/unread/mention state and keyset history pagination.
- All REST snapshots/collections, lazy message/profile/member lookup, streamed
  multipart uploads, pending-file deletion and authenticated original/thumbnail
  downloads, ETags and single original-file byte ranges.
- PNG/JPEG/WebP sniffing and bounded decoding; orientation-corrected,
  metadata-free JPEG thumbnails, generated with `spawn_blocking`. Generic files
  download as octet-stream; SVG and unsupported claimed image formats are refused.
- Required binary named-map MessagePack transport, Hello/Identify/Ready,
  heartbeat/timeouts, single active socket per token, correlated results, durable
  ten-minute nonce receipts, resume/replay, safe full-resync fallback and bounded
  slow-consumer disconnection.
- CLI/env configuration, embedded SQLite migrations, WAL, a four-connection pool,
  one mutation/snapshot gate, Snowflake allocation with persisted high-water,
  account-wide token buckets, quotas, upload reservations, bounded maintenance
  and graceful shutdown.

Voice is deliberately **not implemented**: every authenticated Phase 2 voice
envelope is rejected as `unsupported_feature`. No SFU, TURN or media UDP is
started. Phase 3 configurable roles/overrides are likewise outside Phase 1.
There are no intentional Phase 1 endpoint/command omissions.

## Execution and safety notes

- `http.rs`: REST/auth/snapshots; `google.rs`: Google verification/account flows;
  `ws.rs`: transport/state machine;
  `commands.rs`: authorization, mutations and event projections; `db.rs`:
  parameterized queries/projections; `files.rs`: upload/media/download pipeline;
  `state.rs`: configuration, IDs, bounded streams, rates and lifecycle.
- Mutation state and successful retry receipts commit together. Events are
  admitted after commit, before the initiating response. REST and Ready share
  that same gate/cursor boundary. Password hashing, image work, filesystem and
  network I/O do not run while holding the database mutation gate.
- Each socket has a 128-item/2-MiB outbound budget. Immutable encoded event bytes
  are shared between matching session sequences. Replay is limited to 2,048
  events, 2 MiB and ten minutes per stream, with a 64-MiB global budget.
- Revocations invalidate potentially unsafe replay rather than replaying old
  private bodies. Such reconnects receive a full replacement Ready. Deleted
  messages also invalidate old body-bearing replay. Optional member/presence
  interests are scoped to their connection.
- CPU-heavy concurrency is two password jobs and one media job. WebSocket
  read buffers are 8 KiB; the service accepts at most 1,024 simultaneous sockets.
- Extra internal migration tables track in-flight upload reservations and the
  allocator high-water. An internal invite notification marker ensures expiry/
  exhaustion removal events are emitted once. These are not wire changes.
- Read counters are recomputed for the touched channel for correctness. Metadata
  sets have Phase 1 quotas; history uses the `(channel_id,id)` index and only
  `limit+1` records. Larger installations should benchmark the write/read-counter
  workload before raising quotas or adding features.
- Run one process per local data directory. TLS is provided by Caddy; keep the
  application listener private. This is open-registration software, not an
  independently audited Internet service.

## Build and run (Windows PowerShell)

```powershell
$env:CARGO_TARGET_DIR = '$env:USERPROFILE/.cargo-target/sticktalk-server'
& "$env:USERPROFILE/.cargo/bin/cargo.exe" build -p sticktalk-server --release
& "$env:CARGO_TARGET_DIR/release/sticktalk-server.exe" --bind 127.0.0.1:8080 --data-dir "$env:LOCALAPPDATA/StickTalk/server-data"
```

All settings are listed by `sticktalk-server --help`. Defaults: loopback 8080,
`./sticktalk-data`, public URL `http://localhost:8080`, 25-MiB uploads, worker 0,
and trusted proxy `127.0.0.1`. Only that peer's forwarded client IP is trusted.
Keep runtime databases/uploads out of the source checkout and OneDrive.

For native ARM64 Ubuntu/systemd, HTTPS, both Oracle/Ubuntu firewalls, DuckDNS,
backup/restore and updates, use [`deploy/README.md`](../../deploy/README.md).
The deployment script has been checked with `bash -n`; it has **not** been
executed against a live Oracle VM during this Windows implementation task.

## Validation / handoff

The following commands were run with the dedicated server target directory:

```powershell
$env:CARGO_TARGET_DIR = '$env:USERPROFILE/.cargo-target/sticktalk-server'
& "$env:USERPROFILE/.cargo/bin/cargo.exe" build -p sticktalk-server --release
& "$env:USERPROFILE/.cargo/bin/cargo.exe" test -p sticktalk-server
& "$env:USERPROFILE/.cargo/bin/cargo.exe" clippy -p sticktalk-server --all-targets -- -D warnings
& "$env:USERPROFILE/.cargo/bin/cargo.exe" fmt -p sticktalk-server -- --check
& "$env:USERPROFILE/.cargo/bin/cargo.exe" test -p sticktalk-proto
```

The original implementation tests included six unit tests and six integration tests.
The independent audit expands coverage; see AUDIT.md for current counts,
findings, regressions, and the measured 300-client load test. Integration
tests start the actual HTTP/WebSocket service on ephemeral ports with temporary
data directories and `tokio-tungstenite` peers. They cover accounts, friends and
presence, DM edits/deletes/reactions, read state, typing, pagination, guild
invitation/join/channel fan-out, denied permissions, image upload/thumbnail/
range/ACL handling, groups/ownership, moderation, category reordering/removal,
safe replay after revocation, restart-persistent receipts, protocol errors,
heartbeat, empty resume, socket replacement and logout closure. Unit tests
exercise bounds/sharing/backpressure, maximal Ready size, Snowflakes/rollback,
framing and the username-reassignment login race. Proto's eight tests also pass.

The required **workspace-wide** command was also run:

```powershell
& "$env:USERPROFILE/.cargo/bin/cargo.exe" fmt --all -- --check
```

The workspace-wide format check initially found concurrent client edits, but
the client engineer subsequently formatted them and the check now passes. No
client files were edited by this server implementation task. Server-only
formatting also passes.

### Pre-audit release footprint

[`idle-rss.json`](idle-rss.json) records the latest 60-second idle release capture
on Windows 11 x64, using a fresh empty database, no active clients, a health probe
and a two-second warmup. RSS means Windows process working set; results depend on
OS working-set trimming and concurrent system load. This is **not** an ARM64
Linux RSS measurement or a hundreds-of-clients load test. Neither was executed
here; measure them on the actual VM before advertising deployment capacity.

Pre-audit capture (2026-11-11): **9.41 MiB peak idle RSS** over 60.78 seconds.
The measured release executable is 8,116,224 bytes.

Reproduce (the helper starts/stops only its own hidden test process):

```powershell
./crates/server/scripts/measure-idle.ps1
```

The measured release remains below the requested 30-MiB idle target in these
Windows captures. No commits were made, and no client or proto sources were
edited by this implementation task.

## Independent audit follow-up

[AUDIT.md](AUDIT.md) records confirmed issues, fixes, catalog spot-checks,
validation and a separate-process release load test.
[loadtest-results.json](loadtest-results.json) contains measurements;
[examples/loadtest.rs](examples/loadtest.rs) reproduces them.

Additional admission controls: 128 active HTTP responses, 16 uploads,
15-second ordinary/120-second upload handler deadlines, 4,096 retained
session-stream entries and one 10-second initial replay deadline. Global
logical upload admission defaults to 10 GiB via --max-total-upload-mb /
STICKTALK_MAX_TOTAL_UPLOAD_MB, including thumbnails and in-flight reservations.
It is not a filesystem quota: leave room for SQLite/WAL/GC/backups and monitor disk.
Upgraded sockets are explicitly drained on graceful shutdown. No public wire
types, client or proto files changed.

## Google sign-in

Set `--google-client-id` or `STICKTALK_GOOGLE_CLIENT_ID` to a Google **Desktop
app** client ID. Leave unset to disable the Google routes (404 feature_disabled)
and advertise `google_client_id:null` at `/api/v1/meta`. Setup is explained for
owners in [`deploy/README.md`](../../deploy/README.md). No Google client secret
is used by the server, stored in SQLite, or exposed in metadata. Tests generate
an RSA key in memory and fetch JWKS only from an in-process loopback mock.
