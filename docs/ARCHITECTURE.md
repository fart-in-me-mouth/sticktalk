# StickTalk foundation architecture

The opening Phase 1 status below is historical foundation context. For the
current Oracle/SFU/web + private Pi topology, portable build commands and known
release gates, start with [HANDOFF.md](../HANDOFF.md).

Status: Phase 1 server implemented, protocol version **1**. The native
axum/tokio server provides accounts, REST snapshots, WebSocket chat/presence,
SQLite persistence, media and bounded replay. Google sign-in is optional and
server-configured. The client is being implemented separately; voice/video
remain reserved, not implemented. `PROTOCOL.md` is normative for wire behavior;
`crates/proto` is authoritative for Rust field names and types. Change both in
the same change. Never independently invent client/server DTOs.

## 1. Product priority and boundaries

Minimize **user RAM, CPU, disk, and bandwidth** before decorative effects or
feature breadth. Native Rust throughout; no Electron, browser runtime, webview,
JavaScript service, continuous redraw loop, background polling, or duplicate
runtime/socket per view. Phase 1 includes accounts, relationships, presence,
DMs/group DMs, guilds/categories/text channels, invitations/moderation,
messages/replies/attachments/mentions/reactions/typing, and read state.

Phase 2 is voice/video/screen sharing through a small `str0m` sans-IO WebRTC
SFU. Audio capture/playback uses `cpal` and Opus; each sender uploads one stream,
not one per recipient. Signaling will use the existing WebSocket. The reserved
voice envelopes are **not implemented** and return `unsupported_feature` now.
Full configurable roles/channel overrides are Phase 3. Do not ship either phase
as part of the initial implementation.

## 2. Workspace and toolchain

* Stable Rust, edition 2021, `x86_64-pc-windows-msvc`, rustfmt and clippy.
  `rust-toolchain.toml` selects stable. `Cargo.lock` belongs in source control.
* Rustup was installed with winget. Existing Visual Studio C++ tools and Windows
  SDK suffice; successful native linking is the final capability check.
* `aarch64-unknown-linux-gnu` standard library is installed. This **does not**
  install an ARM GNU linker, Linux sysroot, or cross C compiler. Prefer building
  the server natively on the Oracle ARM VM (`cargo build --profile server -p
  sticktalk-server`). SQLite/image dependencies may require native build tools.
* Cargo defaults to the ignored `./target` directory. Keep the checkout outside
  OneDrive, or set `CARGO_TARGET_DIR` to a local non-synced directory. See
  `HANDOFF.md` for native build prerequisites. Never sync databases/uploads/caches.
* `crates/proto`: serde + bitflags only at runtime, no tokio, HTTP, GUI, database,
  or cryptography. `rmp-serde` and JSON are test dependencies here; applications
  should opt into the workspace dependencies when implementing transport.
* `crates/server`: `sticktalk-server`; axum/tokio/sqlx/Argon2 service, with
  ring RS256 verification and reqwest/rustls for Google JWKS.
* `crates/client`: `sticktalk`; iced **0.14** is the verified stable baseline.
  Use reactive rendering and the lean `tiny-skia` renderer first, with defaults
  disabled. Enable image codecs/fonts only as actually needed. Benchmark any
  GPU backend before enabling it; software rendering is not always cheaper.
* Desktop release: optimization 3, fat LTO, one codegen unit, abort-on-panic, stripping.
  `cargo build -p sticktalk --profile release-small` chooses size optimization
  `s`; compare binary size/idle RAM/startup before choosing distribution builds.
  The server must use `--profile server`, which retains unwinding for the SFU's
  per-peer panic containment. Validation must never rely on panics.

Sources verified when establishing this contract: [iced release/API](https://docs.rs/iced/0.14.0/iced/),
[iced reactive rendering FAQ](https://book.iced.rs/faq),
[named MessagePack encoding](https://docs.rs/rmp-serde/1.3.1/rmp_serde/encode/fn.to_vec_named.html).

## 3. Resource budgets (initial defaults, measure before tuning)

| Resource | Budget/behavior |
|---|---|
| Client runtimes/sockets | One tokio runtime and one WebSocket; one bounded network reducer |
| Redraws | OS/input/state changes only; no per-frame timer; idle animation disabled |
| Message fetch | 50 per page maximum; keyset before/after ID, never offset |
| Message LRU | At most 1,000 messages **and** 8 MiB estimated retained data, global; smaller limit wins |
| Rendered message widgets | Visible rows plus ~20 overscan rows; do not build widgets for all cached messages |
| Thumbnail RAM | 16 MiB decoded pixels, max 64 entries; free GPU handles on eviction |
| Thumbnail disk | 128 MiB, immutable ID key, LRU cleanup at startup/after writes; per-account namespace |
| Original files | Download only on explicit user action to their selected destination |
| Lazy images | Visible rows only; 2 concurrent thumbnail fetches; cancel stale requests |
| History requests | 1 per channel, 2 globally; coalesce scroll requests; cancel on navigation |
| Network queues | 128 inbound/outbound items and 2 MiB each, whichever first; no unbounded mpsc |
| Client reconnect | Jittered exponential backoff 1, 2, 4, 8…30 seconds, reset after 60 seconds healthy |
| Idle heartbeat | 30 seconds, 75-second timeout; no other periodic presence/REST polling |
| Replay per session | 2,048 events, 2 MiB encoded data, 10 minutes, whichever expires first |
| Replay server global | 64 MiB; evict oldest disconnected streams first, force resync rather than grow |
| Server outbound/socket | 128 frames or 2 MiB; slow consumer is disconnected/resynced |
| SQLite pool | 4 connections initially, one serialized writer; no connection per user |
| CPU-heavy work | Bounded blocking jobs: 2 password hashes, 1 thumbnail encoder initially |
| Upload | 25 MiB/file; 4 attachments/message; stream to disk, never buffer whole body |

Idle client target: <100 MiB resident memory and <0.5% of one logical CPU over
a 60-second idle capture on the reference Windows machine. These are targets,
**not measured client performance claims**. Record release executable
size, cold-start time, 60-second idle CPU, memory after a 10,000-message scroll,
cache bounds and bytes transferred. Include an image-heavy channel test. Text
layout is cached only for visible/cached messages, keyed by content/width/font.
Drop/compact inaccessible data immediately on logout, removal, block, or resync.

Phase 1 service limits keep Ready bounded: 10 guild memberships/user; 32 channels
(categories included)/guild; 1,000 members/guild; 64 open DM/group sidebar
entries/user; 16 participants/group; 100 relationship records/user (requests,
friends and blocks together); 5 sessions/user; 100 custom presence interests/
connection. Channel/user metadata has byte limits in `PROTOCOL.md`. Reject a
mutation exceeding a recipient's limit atomically, not by truncating Ready.
Ready must fit the 512 KiB frame limit even at the configured maxima. Later
larger limits require a negotiated paged bootstrap protocol, not bigger caches.

## 4. Server layout and execution model

One axum + tokio binary exposes plain HTTP and `/api/v1/ws` on a configured
loopback/private port (default `127.0.0.1:8080`). Caddy terminates public TLS,
redirects HTTP to HTTPS, and proxies WebSocket upgrades. Never expose bearer
tokens over the public cleartext port. Bind private/loopback and firewall it.
Trust forwarded client IP only from the configured Caddy peer, not arbitrary
`X-Forwarded-For`. Use an unprivileged dedicated systemd service account on the
Oracle ARM VM; local disk for SQLite and uploads, not a network filesystem.

Implemented modules: `state` (config, IDs, rates, streams, lifecycle), `http`
(auth and snapshots), `google` (Google auth), `files`, `ws`, `commands` (mutations,
authorization and events), and `db` (storage/projections). `AUDIT.md` in the
server crate records hardening and bounded local validation.
HTTP and WS handlers call the same service/authorization layer. Never implement
an HTTP mutation alternative to a WS command. Auth and upload lifecycle are the
only REST mutations. Parameterize all SQL; no `query!` compile-time DB required.
Embed ordered migrations with `sqlx::migrate!`; migrate before accepting traffic.

### Transactions, snapshots and events

Start with one bounded mutation dispatcher/serialization gate. Under this gate:
authorize against current DB state, check receipt, write a transaction, compute
affected account projections/read counts, commit, assign per-session sequence
numbers and enqueue events, then send the command response. For bulk snapshots
and Ready, hold the same gate through the SQLite read snapshot and cursor
capture, then release before serialization/network I/O. REST receives the
cursor of the bearer session's stream. This avoids an ambiguous snapshot/live
race. Never hold a DB transaction/gate while awaiting network/file IO or hashing.
Prepare uploaded media/hash outside it; recheck references/permissions inside.

Events are **per login session stream**, not a global database revision. Each
stream has a random 128-bit hex `stream_id`, an increasing `u64` sequence, a
bounded replay ring, and its connection's optional interests. A login session
has at most one active socket; a second socket using that token replaces the
first. Separate device logins get separate streams. Retain disconnected streams
and interests for up to ten minutes. Share immutable encoded payloads where
possible; personalized reactions/read state must not leak between users.
On server restart streams are lost; persisted auth sessions remain valid, and
clients obtain a new Ready. No durable event log is required in Phase 1.

Allocate sequences to everything admitted to that stream, including ephemeral
typing/presence. Skip expired typing during replay **without renumbering**;
therefore clients require strictly increasing sequences, not contiguous ones.
Replay rechecks access and may omit now-inaccessible historical content. If a
safe filtered replay cannot be produced, resync. Removal events purge local
content before rendering. Duplicate/lower sequences are ignored. Commands are
not considered durable based on socket delivery alone: success response means
the transaction committed. Persistent command receipts make bounded retries
safe even across a crash after commit/before response (see protocol).

Use full replacement/upsert events rather than fragile positional UI patches.
Durable message creates/updates/deletes and read state reach all sessions with
access, including sender sessions. Presence defaults to friends and open DM
recipients; selected-guild member/presence events are optional interests. Do not
broadcast every guild member's every heartbeat to every client. Heartbeat ack
is connection control and is not itself an application event.

## 5. IDs and time

All entity IDs share one Snowflake allocator: positive signed-63-bit values
represented as `Id(u64)` on the wire. Layout from low to high: sequence 12 bits,
worker 10 bits, elapsed milliseconds 41 bits. Epoch is 2025-01-01T00:00:00Z
(`1735689600000` ms). `id = (elapsed << 22) | (worker << 12) | sequence`.
Worker is configured 0..1023; initial single server uses 0. Reserve zero (start
sequence at 1 if epoch+worker would produce zero). Never truncate elapsed time.
If clock moves backward, wait until the previous timestamp or fail allocation
as `unavailable`; never produce duplicates or spin. On 4096 IDs/ms, asynchronously
wait for the next millisecond. On startup seed the allocator above the maximum
persisted ID across all entity tables; refuse allocation until clock catches up
after rollback. Multiple writers require unique workers and a coordination plan.
Sequence ordering is roughly chronological across workers; ID order is the
pagination order, never wall-clock text. Unix times are UTC integer milliseconds.

## 6. SQLite schema outline

Use `PRAGMA foreign_keys=ON`, `journal_mode=WAL`, `busy_timeout=5000`,
`synchronous=NORMAL` on the pool. NORMAL may lose the last committed transactions
on machine power failure; choose FULL if that durability cost is unacceptable.
Use SQLite backup API (or a consistent stopped-service copy), never copy only
the live main DB and ignore WAL. Store IDs/times as INTEGER, opaque hashes as
BLOB, enums as checked TEXT, bit masks as INTEGER (bits 0..62). Migration SQL
must encode uniqueness/FKs/checks; the following is the complete logical outline.

| Table | Columns and constraints |
|---|---|
| `users` | `id PK`, `username UNIQUE COLLATE BINARY` (canonical lowercase ASCII), `password_hash TEXT NOT NULL` (PHC Argon2id, or empty sentinel for Google-only users), `display_name NULL`, `avatar_id NULL FK uploads`, `status_text NULL`, `desired_presence TEXT`, `created_at` |
| `oauth_identities` | `provider` (google), `subject`, `user_id FK users CASCADE`; PK `(provider,subject)`, UNIQUE `(provider,user_id)`; never identify/merge users by email |
| `oauth_signup_tickets` | `token_hash BLOB(32) PK`, `subject UNIQUE`, `expires_at`; 32-byte CSPRNG ticket, SHA-256 at rest, ten minutes, consumed in account/session transaction |
| `sessions` | `token_hash BLOB(32) PK`, `user_id FK users CASCADE`, `created_at`, `expires_at`, `last_used_at`; token plaintext never stored |
| `relationships` | `user_id`, `other_id`, `kind`, `since`; PK `(user_id,other_id)`, FKs users CASCADE, no self; two reciprocal rows for friendship or incoming/outgoing request, blocking row private to blocker |
| `guilds` | `id PK`, `owner_id FK users`, `name`, `icon_id NULL FK uploads`, `created_at` |
| `guild_members` | `guild_id FK guilds CASCADE`, `user_id FK users CASCADE`, `role`, `joined_at`; PK `(guild_id,user_id)`; owner role must match guild owner (service invariant, exactly one) |
| `guild_bans` | `guild_id FK guilds CASCADE`, `user_id FK users`, `actor_id FK users`, `reason NULL`, `created_at`; PK `(guild_id,user_id)` |
| `invites` | `id PK`, `code_hash BLOB(32) UNIQUE`, `code TEXT` (required to list own guild invite codes; protect DB access), `guild_id FK guilds CASCADE`, `creator_id FK users`, `expires_at`, `max_uses`, `uses`; enforce positive bounds, consume use inside join transaction |
| `channels` | `id PK`, `kind`, `guild_id NULL FK guilds CASCADE`, `parent_id NULL FK channels ON DELETE SET NULL`, `name NULL`, `topic NULL`, `position`, `owner_id NULL FK users`, `created_at`; enforce guild-vs-DM and parent-category invariants in service |
| `dm_pairs` | `low_user_id`, `high_user_id`, `channel_id UNIQUE FK channels CASCADE`; PK `(low_user_id,high_user_id)`, low<high; one permanent direct channel/pair |
| `dm_members` | `channel_id FK channels CASCADE`, `user_id FK users CASCADE`, `joined_at`, `visible BOOL`; PK `(channel_id,user_id)`; direct retains both rows on sidebar close; group leave deletes row |
| `messages` | `id PK`, `channel_id FK channels CASCADE`, `author_id FK users`, `content`, `created_at`, `edited_at NULL`, `reply_to NULL FK messages ON DELETE SET NULL`, `deleted_at NULL`; deleted messages retain only tombstone metadata, blank content and null reply |
| `message_mentions` | `message_id FK messages CASCADE`, `user_id FK users`; PK `(message_id,user_id)`; mentions are explicit IDs, never parsed by server |
| `uploads` | `id PK`, `owner_id FK users`, `purpose`, `channel_id NULL FK channels CASCADE`, `filename`, `content_type`, `byte_size`, `storage_key UNIQUE`, `width NULL`, `height NULL`, `thumbnail_key NULL`, `thumbnail_width/height/size/type NULL`, `created_at`, `expires_at NULL`; pending uploads have expiry; attached ones null it |
| `message_attachments` | `message_id FK messages CASCADE`, `attachment_id UNIQUE FK uploads`, `position`; PK `(message_id,attachment_id)`; one attachment cannot be attached to two messages |
| `reactions` | `message_id FK messages CASCADE`, `user_id FK users`, `emoji TEXT`; PK `(message_id,user_id,emoji)`; Unicode exact sequence, max 20 distinct emoji per message |
| `read_states` | `user_id FK users CASCADE`, `channel_id FK channels CASCADE`, `last_read_id NULL`, `unread_count`, `mention_count`; PK `(user_id,channel_id)`; no FK for last_read_id (deleted cursor remains meaningful) |
| `command_receipts` | `session_hash FK sessions CASCADE`, `nonce INTEGER`, `request_hash BLOB(32)`, `result_msgpack BLOB`, `created_at`, `expires_at`; PK `(session_hash,nonce)`; max 256 unexpired durable successes/session |
| `upload_daily_usage` | `user_id FK users CASCADE`, `utc_day INTEGER` (floor UTC milliseconds / 86400000), `charged_bytes INTEGER`; PK `(user_id,utc_day)`; preserves daily bandwidth accounting even if uploads are deleted |

Required secondary indexes: sessions `(user_id,expires_at)`; relationships
`(other_id,kind)`; guild_members `(user_id,guild_id)`; guild_bans `(guild_id,user_id)`;
channels `(guild_id,parent_id,position,id)`; dm_members `(user_id,visible,channel_id)`;
messages `(channel_id,id)` including tombstones; message_mentions `(user_id,message_id)`;
reactions `(message_id,emoji)`; uploads `(expires_at)` and `(owner_id,created_at)`;
receipts `(expires_at)`. Existing PK indexes need not be duplicated.

Compute latest visible message via the channel/ID index; it is not a separately
mutable client counter. Maintain read counts transactionally on create/edit/
delete/read. Count live messages by other authors with `id > last_read_id`
(null treated as zero); mention count is the subset explicitly mentioning the
reader. Recompute touched counts if simplifying correctness initially, but do
not scan all history on every Ready. Own sends do not implicitly mark older
incoming messages read. MarkRead advances monotonically only.

Delete messages into tombstones (no original text, mentions, reactions or file
links); history skips them, but replies and read IDs remain resolvable. Existing
replies show a deleted placeholder. Guild/channel deletion cascades logical
records; file GC happens after commit. A two-phase file lifecycle prevents DB
rows from referencing half-written files. Periodic server housekeeping can use
a coarse timer (e.g. 60 seconds); the client must not poll it. Expired receipts,
sessions and unattached uploads are pruned in bounded batches. Garbage-collect
daily upload usage older than two UTC days. Reserve the maximum accepted upload
size atomically at upload start, settle to actual bytes received on completion
or abort; a crash may conservatively retain the reservation for that day. This
prevents concurrent uploads and delete/reupload loops bypassing the daily cap.
Garbage-collect
unreferenced former avatars/icons after commit. Keep no deleted original files.

## 7. Authentication, authorization, abuse resistance

### Optional Google sign-in

`--google-client-id` / `STICKTALK_GOOGLE_CLIENT_ID` enables the feature;
unset returns 404 `feature_disabled` on Google routes. `GET /api/v1/meta`
exposes only the public client ID, server version and protocol version. The
client performs installed-app browser/loopback PKCE/state and token exchange;
the server never receives a Google client secret or exchanges authorization
codes. See `GOOGLE_LOGIN.md` and `deploy/README.md` for desktop setup.

The verifier pins Google's HTTPS JWKS URL, disables redirects/proxies, uses
rustls, caps fetches at five seconds/64 KiB/32 keys, honors Cache-Control max-age
and Age (at most one day), and coalesces fetches in a separate mutex. Missing
cache headers default to five minutes. No-cache/no-store responses have zero
reuse lifetime. Refresh attempts (including unknown kid and failures/cancellation)
are globally limited to once/minute; expired keys fail closed during that
backoff or a fetch failure. Each ID token is capped at 12 KiB, uses RS256 with
2048–8192-bit RSA, and must have Google's issuer, exactly the configured audience,
nonempty bounded sub, verified email, exp/iat with 60-second clock skew and
iat before exp. No token-controlled key URL, email-based linking, access token,
refresh token, email, profile name or Google picture is stored.

Existing Google subjects receive the same five-session/30-day hashed bearer
sessions as password login. New subjects get at most one outstanding ten-minute
hashed signup ticket each (4,096 globally); re-sign-in rotates it. Completion
atomically checks username uniqueness, creates user/identity/session and consumes
the ticket. Failed username choice keeps the ticket usable until expiry.
Link requires a bearer session plus a verified Google token; subject/account
conflicts never merge accounts. Unlink requires a password if Google is linked.
A Google-only user can set a password with their session; replacing an existing
password requires the current password. Hashing stays outside the gate, then
session/hash are rechecked under it. Empty password sentinels run the dummy hash
on password-login attempts and can never authenticate.

Google endpoints share the password login IP bucket (10/minute, burst 5), plus
10/minute burst 5 per verified subject or authenticated account; completion also
uses the canonical-name bucket (5/minute, burst 3). Auth bodies are limited to
16 KiB, responses are no-store, and existing HTTP admission/deadline controls
remain intact. Expired tickets are also pruned by bounded housekeeping.

### Password/session and authorization controls

* Register/login: Argon2id v19, m=19,456 KiB, t=2, p=1 minimum baseline; random
  16-byte salt, PHC string; benchmark on ARM and increase work only within a
  bounded login latency/memory budget. Use constant-time verification, a dummy
  hash on nonexistent usernames, and identical invalid-credential responses.
* Sessions: CSPRNG 32 bytes, unpadded base64url (43 chars), SHA-256 token hash
  stored server-side; 30-day absolute lifetime, no sliding refresh. Login makes
  a new session, logout revokes just that session and closes its WS. At most 5
  sessions; return limit_exceeded rather than silently revoke another device.
* Do not log tokens, passwords, auth DTO `Debug`, message bodies, invite codes,
  or private file paths. Use OS credential storage for persisted client tokens
  (Windows Credential Manager; platform equivalent on Linux/macOS), never a
  plaintext settings file. Clear account RAM/disk thumbnail namespace on logout.
* Permissions are server-enforced on every command, fetch and file read; IDs,
  client flags, channel lists and previous subscriptions grant no authority.
  Return not_found for inaccessible entities; forbidden for known resources
  with insufficient action permission. Do not leak membership through errors.
* Limits: per-IP unauth auth bucket 10/minute burst 5, plus per-canonical-name
  5/minute burst 3; per-user durable commands 120/minute burst 20; sends
  60/minute burst 10; uploads 10/minute burst 2; 250 MiB uploaded/day/user;
  authenticated REST reads 120/minute burst 30; typing 1/4 seconds/channel;
  WS control frames 10/second burst 20. Applicable buckets all apply, across
  sessions, with retry_after_ms and no busy retries. Bad frames count too.
  Bound IP/name limiter entries by idle TTL and global cap. Protect storage with
  1 GiB attached/pending bytes per uploader and 10 pending uploads/user.
* Native clients don't need CORS. Disable broad CORS. Reject unexpected browser
  Origin on WS; absent Origin is valid for native apps. No bearer in URL/query.
* Close malformed or oversized frames early; bounded deserializer depth (32)
  and frame size (512 KiB), no MessagePack extension objects or compression in
  v1. Enforce limits after decoding too. Never trust filenames/MIME/ID inputs.

### Permissions

`Permissions` is a numeric u64 mask in both formats; unknown bits are retained
on decode but **never grant rights**. Member=287, Admin=8191, Owner=16383.
Owner includes admin. Owner/admin/member are the only guild assignments now.
No per-channel overrides; all guild members can see all text channels.
Admin can manage channels/messages/guild metadata/invites, kick/ban ordinary
members; only owner may assign admins, transfer ownership or delete guild.
No one may kick/ban the owner or an equal/higher role; owner can demote/remove
admins. Owner must transfer before leaving a nonempty guild (or delete it).
`set_member_role(owner)` is invalid; transfer is the only ownership operation.
DMs have a separate participant/owner ACL, not fake guild membership. Group
owners manage name/recipients; any participant can leave; owner must transfer
before leaving if others remain. Last participant leaving deletes the group.

## 8. Files and thumbnails

REST multipart upload streams to a server-generated temporary key. Cap file
content at 26,214,400 bytes (25 MiB; the UI may say “25 MB”) and multipart
overhead at 64 KiB. Validate sniffed MIME, discard path components/control chars
from filename, atomically rename on success, then publish DB metadata. Generic
message files use safe octet-stream download when unknown; avatars/icons must
decode as images. Support PNG/JPEG/WebP inputs initially; reject other claimed
image formats and SVG. Max decoded image 16 million pixels / 16,384 per edge;
decompression bomb or decode failure returns unsupported_media_type.

Thumbnail is static, orientation-corrected, metadata-stripped WebP or JPEG,
max 256x256 preserving aspect ratio (no upscale), max 64 KiB. For avatar/icon
use the thumbnail endpoint too. Never fetch remote link previews on the server.
Return metadata, not embedded file bytes/base64/remote URLs in messages. Pending
uploads expire in one hour; message-purpose upload is bound to one authorized
channel. Attach only own pending uploads with matching purpose/channel, inside
the message/profile/guild mutation transaction. The uploader may delete a
pending upload; attached uploads are deleted by unlink/GC, not arbitrary REST.

Downloads recheck current access to their message/profile/guild; they are not
public bearer URLs. `Cache-Control: private, max-age=86400, immutable`, ETag
`"<id>-original"` or `"<id>-thumbnail"`, with authorization checked **before**
304. `X-Content-Type-Options: nosniff`; originals use attachment disposition.
Support one byte range for originals with 206/416; no ranges for thumbnails.
Disk cache hits need no recurring revalidation; purge cached media on known
access removal/logout and never share caches between accounts. Access cannot
revoke a file the user has deliberately saved elsewhere.

## 9. Native client responsibilities

One iced application owns the reducer/model. A single async network service
uses one tokio runtime and emits bounded state-change messages to the GUI.
Do not create a runtime inside subscriptions/tasks or an endless per-frame
subscription. Use the iced tokio executor consistently. Heartbeat/reconnect/
typing-expiry timers are coarse event sources; no global 16ms ticker.

Render Ready immediately without history. Display guild/DM sidebar and unread
counts, lazily load selected channel topic/history and paged members. Subscribe
to only the selected guild and visible member presence interests. Preserve
scroll anchor across prepend pages and edits; cache measured row heights in a
bounded structure. Never instantiate the entire history in a scrollable column.

Parse URLs and mention syntax locally; selecting `@username` inserts `<@id>` and
sends `mention_ids` from selected users, not arbitrary `@` text. Escape markup in rendering, make links user-activated,
and do not automatically fetch arbitrary link metadata. Resolve missing names
with the bounded bulk users endpoint. Render unknown/deleted reply targets as
placeholders; fetch a specific message only on demand.

Apply WS events serially, persist only a cursor consistent with retained state,
and handle snapshot/live races as described in the protocol. A restarted client
with no retained model must identify without resume, even if it saved a cursor.
Optimistic messages are keyed by nonce until mapped to real message IDs; never
duplicate on a response/event race. Do not automatically retry an operation
outside the server's ten-minute dedup window. On full resync clear stale
projections/history/typing, keep only authorized media after re-evaluation,
replace Ready, and lazily refetch selected view. On auth failure stop reconnect
and present login, rather than looping with an invalid token.

## 10. Implementation acceptance checklist

Server engineer: embedded migrations; auth/session storage; bounded Snowflake
allocator; dispatcher/projections/receipts; REST routes/ACL; WS state machine/
replay/backpressure; media pipeline; moderation/limits; graceful shutdown;
tests for races, authorization, replay loss, duplicate nonce, file traversal,
cross-channel reply/attachment and counter accuracy. Drain committed mutations
on shutdown, close sockets with 1001; startup cleanup removes orphan temp files.

Client engineer: iced shell, single network service, bounded reducer/caches,
virtualized list, lazy downloads, all command/error views, reconnect/resync,
credential storage, idle/resource instrumentation and Windows packaging.

Foundation gates (no server/client functional claims):
`cargo build --workspace`, `cargo test --workspace`,
`cargo clippy --workspace -- -D warnings`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all -- --check`. Do not commit from the foundation task.
