# StickTalk independent server review

Reviewed 2026-11-11 on Windows x86-64 against ARCHITECTURE.md, PROTOCOL.md,
and the public proto types. Direct source edits in `crates/server/**` and `deploy/**` were the only intentional source changes.
No client/proto edits, commits or new server dependencies were made. Direct
source edits were limited to server/deploy. Cargo.lock also changed during the
concurrent client work; no intentional lockfile edits were made, and the shared
lockfile was left intact rather than reverting concurrent changes.
Cargo used `C:/Users/dwhee/.cargo-target/sticktalk-audit`.

## Verdict

The original **6 unit + 6 integration tests passed independently**, but missed
the confirmed issues below. Each finding has a fix and regression coverage.
No cross-account guild/DM/group message read/edit/delete/reaction bypass was
found in the exercised matrices. This is a bounded review/test exercise, not
a certification or proof of absence of other bugs. Severity describes pre-fix
impact, not an outstanding vulnerability.

## Findings by severity

| Severity | What / where | Fix / regression evidence |
|---|---|---|
| **High: integrity** | `files.rs::UploadGuard::drop`: cancellation after SQLite commit but before `complete=true` could delete already-published files and leave valid metadata pointing at missing originals/thumbnails. | Cleanup checks publication under the gate before unlinking. `cancellation_after_upload_commit_preserves_published_files` models this window, and separately verifies failed uploads still clean staging/refund unused daily reservations. |
| **High: availability** | `http.rs`/`files.rs`: no global HTTP admission/handler deadline or upload concurrency bound; per-uploader quotas did not bound total file admission across accounts. | 128 concurrent responses (held through streamed bodies), 15-second ordinary/120-second upload deadlines, 16 upload slots. Atomic global logical upload admission defaults to 10 GiB, including thumbnails/in-flight maximum reservations, configurable with `--max-total-upload-mb`. Real TCP body-stall, exhausted-slot, and rejected-global-reservation tests cover the controls. This is **not a filesystem quota**. |
| **Medium: resource/lifecycle** | `state.rs::cursor` had no global stream-entry cap; initial WS replay could pin buffers without a whole-batch deadline and ignored replacement/shutdown during writes. | At most 4,096 stream entries, evicting oldest disconnected streams first; existing active-socket cap remains 1,024. One 10-second deadline for the entire initial replay, observing close/shutdown signals. Removed empty per-command file-GC tasks; actual orphan scans use the existing one-worker gate. `rest_streams_are_offline_and_metadata_is_bounded`, real constrained-receive-buffer `slow_initial_replay_has_one_deadline`, and existing replay/backpressure tests pass. |
| **Medium: projections/privacy** | `commands.rs` broadcast profile updates to every shared-guild account, not each session's selected-guild projection. Friend/DM/group visibility changes omitted initial/offline presence; `db.rs` briefly treated REST-only streams as online. | Per-session profile delivery filtering; before/after base-presence comparison for affected visibility changes, including blocks/custom interests/subscription clearing; `ever_connected` distinguishes actual connection grace. Tests cover separate sessions with different selected guilds, remove-friend, DM reopen/close, custom-only block, unrelated observers, and REST-only presence. The profile overbroadcast was within otherwise profile-accessible guilds, not a demonstrated unrelated-guild disclosure. |
| **Medium: receipt authorization** | `commands.rs::result_access` checked enclosing guild/channel access but not removed nested member/invite/reaction-message/guild-channel entities in stored results. | Rechecks target membership, invite existence/audience, live reaction message, and every returned guild channel. Removed targets yield `not_found`, without re-executing the receipt. `receipts_cannot_return_removed_nested_entities` plus existing persistent-receipt/revocation tests cover this. |
| **Medium: installer local race** | `deploy/install.sh` downloaded executable installer text to predictable shared `/tmp/sticktalk-rustup.sh`, allowing local replacement/symlink attacks. | Private unpredictable `mktemp`, quoted paths, EXIT cleanup trap. Regression invariants reject the old path; shell syntax passes. No package installation was executed. |
| **Medium: graceful shutdown** | `state.rs::serve`: Axum's HTTP graceful drain does not itself await upgraded WS tasks, so executable runtime exit could race close writers. Pre-auth sockets also waited for Identify timeout on service shutdown. | Await all socket permits with a 15-second bound after signaling shutdown/draining the gate; pre-auth reads close 1001 on shutdown. `graceful_shutdown_drains_authenticated_and_pre_auth_sockets` exercises both socket states. |
| **Low: profile contract** | `db.rs::profile_access` denied bulk profile/avatar lookup for departed authors whose messages remain readable. | Author-of-readable-live-message access, without presence or arbitrary user-directory access; migration `0005_author_lookup.sql` adds `(author_id,channel_id)`. Former-author/outsider regression passes. |
| **Low: transport contract** | `ws.rs` accepted positional arrays for nested structs; pre-auth size/depth errors had wrong close classification. Setup failure could retain a ghost stream or classify storage failure as revoked credentials. | Raw bounds scan, named-shape validation, then typed decode; size close 1009, failed-setup detachment, internal-vs-auth failure classification. Tests cover nested structs/patches/resume/placements, malicious length/depth, trailing bytes/extensions, 2,048 random inputs, injected Ready failure, and oversized masked headers rejected before payload allocation. |
| **Low: cross-channel errors** | Visible cross-channel mutation targets returned `not_found` instead of documented `invalid_request`. | A shared command-target validator distinguishes visible wrong-channel IDs from hidden/missing IDs, retaining `not_found` for the latter to avoid an oracle. Authorized/outsider edit/delete/read/add/remove-reaction regression passes. |
| **Low: media/counters/login** | `files.rs` upscaled tiny thumbnails; `db.rs::read` silently cast counts to u32; malformed login usernames returned a different error category. | Clamp thumbnail bounds to original dimensions; checked count conversion with `limit_exceeded`; login returns `invalid_credentials` for malformed canonical names. Tests cover 2x3 image, valid-CRC oversized-edge/16-million-pixel PNG headers, MIME spoofing/SVG/path basenames, overflow and uniform login errors. |
| **Low: deployment availability** | No explicit Caddy header/body/time bounds, systemd memory ceiling, or failure-safe backup restart helper. | Caddy limits, `MemoryHigh=768M`/`MemoryMax=1G`, and `deploy/backup.sh`: private backup directories, stopped-service SQLite backup + matching files, quick-check, EXIT restart trap. Encryption example enables `pipefail`. Invariants/shell syntax/Caddy adaptation-validation pass; no live VM restore/systemd execution was performed. |

## Checks not requiring changes

- **Passwords/sessions:** Argon2id v19, m=19,456 KiB, t=2, p=1; random 16-byte
  salt; dummy-hash verification for unknown valid usernames. Two blocking hash
  permits remain held by closures even on cancellation. Tokens are 32 random
  bytes/43 base64url characters, SHA-256 stored, with `subtle` comparison after
  indexed lookup. Absolute 30-day expiry, five sessions/account, per-token socket
  replacement and logout were traced/tested. Logout now closes before its SQL
  await so cancellation cannot strand an already-revoked socket.
- **ACL:** all 40 command branches and REST catalog handlers were inspected.
  New guild-role/hierarchy and nonparticipant guild/DM/group matrices exercise
  send/edit/delete/react/typing/read/management denial. Bulk users fail wholly
  if any target is inaccessible; guild enumeration lists only own memberships;
  files reauthorize **before** 304. Direct/group participant checks are separate
  from guild roles. Existing group owners cannot delete other authors' messages.
- **Presence/blocks:** invisible is offline to others; presence is restricted to
  self, friends/visible DMs and selected-guild interests. Either-direction block
  prevents direct sends/reopen/typing/new group invitations, not historical
  shared-room reads/reactions (the documented policy).
- **Invites:** 128-bit CSPRNG code, hashed lookup, expiry/use/ban/quota checks,
  creator/manager-only code audiences, transactional consumption. Real simultaneous
  WS joins to a one-use invite produce exactly one success and one exhausted
  error. Existing tests cover moderation/revoke/unban and safe revoked replay.
- **Uploads:** generated storage keys and sanitized display basenames; streamed
  25-MiB file/64-KiB overhead bound; per-user pending/storage/daily reservations;
  publication/attachment ACL rechecks. One blocking decoder, signature-based
  PNG/JPEG/WebP, per-edge/pixel/allocation bounds. Unknown files download as
  octet-stream attachments with nosniff; no remote preview fetch or SVG rendering.
- **SQL/panics/logs:** bound parameters, no client SQL interpolation. Fixed-header,
  fixed-Argon and OS-signal `expect` calls are not client-controlled. Guild
  compaction indexing relies on an already-authorized nonempty membership;
  heartbeat stream indexing is under the same gate as its active-stream check.
  No input-reachable unwrap/slice panic was identified; this is not exhaustive
  codec fuzzing. Application logs/errors expose no passwords, tokens, invite
  codes, message bodies, SQL or private file paths. Caddy access logging stays off.
- **Bounds/snapshots:** 512-KiB WS message/frame, raw depth 32, collection scanner,
  128-item/2-MiB socket output, 2,048-event/2-MiB/10-minute stream replay, 64-MiB
  global replay, 20,000 limiter entries, four SQLite connections, one mutation/
  snapshot gate. Existing tests exercise shared payload bytes, replay loss,
  slow consumers, restart receipts, Ready/cursor and message-before-read ordering.

## Protocol spot-check and discrepancies

All 40 commands, 23 event variants, and REST catalog routes have implementations.
Source spot-checking is **not** exhaustive testing of every field combination.
The catalog below records each reviewed entry and its service/routing family.

Behavioral mismatches found and fixed: profile/presence projections and snapshots,
former-author lookup, nested positional MessagePack, pre-auth size close codes,
nested receipt access, visible cross-channel error codes, thumbnail upscaling,
login error category, count overflow, and upgraded-socket shutdown. Voice remains
intentionally `unsupported_feature`, not a missing Phase 1 feature.

Documentation-only discrepancy left untouched: ARCHITECTURE's opening/workspace
paragraphs still describe the original executables as stubs. That historical
status is stale. This audit did not edit `docs/**` or the normative wire types.
Extra admission ceilings are documented in server/deployment READMEs and use
existing unavailable/limit error codes.

### Commands

| Entry | Checked documented behavior |
|---|---|
| update_profile | Own account; username_taken; pending avatar-purpose image owned by caller only |
| set_presence | Own desired status; offline invalid; private self result may say invisible |
| send_friend_request | Existing canonical username; self invalid; blocked is generic forbidden; existing incoming returns conflict (explicit accept required); outgoing/friend returns already_exists |
| accept_friend_request | Must be incoming, not blocked; stale/missing request not_found |
| decline_friend_request | Remove incoming only; absent is idempotent ack, wrong direction conflict |
| cancel_friend_request | Remove outgoing only; absent ack, wrong direction conflict |
| remove_friend | Remove reciprocal friendship; absent ack; pending row conflict |
| block_user | Remove both friendship/request rows atomically, create caller-only block; existing block returns existing row |
| unblock_user | Remove own block only, absent ack; no friendship restoration |
| open_dm | New direct requires mutual friendship; existing pair can reopen after friendship ends; either-direction block denies with blocked; reopen not duplicate |
| create_group_dm | 2..15 unique other users, all caller's friends, none blocked with any proposed participant; caller auto-added owner |
| add_group_recipient | Group owner only; target caller's friend, no block with participants; already participant idempotent |
| remove_group_recipient | Group owner only, cannot remove self (use leave/transfer); absent target idempotent |
| leave_group_dm | Participant; owner with others must transfer first (owner_required); last member deletes group; no rejoin except a new owner invitation |
| transfer_group_ownership | Owner to existing participant; self idempotent |
| close_dm | Direct only, hides sidebar for caller (does not erase history, revoke pair, or hide for peer); already hidden ack |
| create_guild | Caller owner; create default text `general` position 0; icon must be own pending guild_icon upload |
| update_guild | manage_guild; pending guild_icon owned by caller |
| delete_guild | Owner only; owner_required; cascade channels, history, members, invites, bans |
| join_guild | Banned denies; existing member returns current detail without consuming use (but code must still resolve valid invite); invalid_invite/invite_expired/invite_exhausted/banned |
| leave_guild | Member; owner must transfer first, even sole owner must use delete; owner_required |
| transfer_guild_ownership | Owner to existing member; old owner becomes admin; self idempotent; owner_required |
| set_member_role | Owner only; admin/member only; cannot alter owner; owner_required/hierarchy_violation |
| kick_member | kick_members and lower role; self use leave; hierarchy_violation; absent member not_found |
| ban_member | ban_members, lower role; may ban a nonmember known user; hierarchy_violation; removes membership, existing ban no-op (reason unchanged) |
| unban_member | ban_members; absent ban no-op; does not restore membership |
| create_invite | create_invites; max 100 active invites/guild, 10/creator/guild |
| revoke_invite | Creator or manage_guild; no-op if already absent but same guild authority required |
| create_channel | manage_channels; kind category/text only; append to sibling list |
| update_channel | Guild manage_channels; group owner can change name only; direct rename/topic invalid; category topic cannot set |
| reorder_channels | manage_channels; complete permutation of **all** guild channels; unique dense sibling positions, valid parents; atomic |
| delete_channel | Guild manage_channels only; deleting category moves children to end of root preserving order; deleting text deletes its history; no group/DM deletion through this command |
| send_message | send_messages (+attach_files if needed), participant for DM; own pending channel-bound files; reply must be live same-channel; invalid_attachment/blocked |
| edit_message | Author only, still has channel access/send rights; attachments/reply immutable; no time limit; same content/mentions no-op, else edited_at now |
| delete_message | Author or guild manage_messages; tombstone same-channel repeated delete ack; absent ID not_found |
| add_reaction | add_reactions, live message; already reacted idempotent added=true with current count/me |
| remove_reaction | Remove own only; already absent idempotent added=false with current count/me |
| typing | send rights; expires server_time+8000ms, coalesce 1/4 seconds/channel, ephemeral; no self echo required |
| mark_read | Message or tombstone in same channel, existing access; monotonically max old/new; no future/arbitrary IDs |
| subscribe_guild | Replaces this connection's selected guild/interests; selected membership required; null clears; presence users must be guild members; response is initial effective presence snapshot |

### Events

| Entry | Checked documented behavior |
|---|---|
| self_updated | All own streams for profile/desired-presence changes |
| user_updated | Streams whose profile projection includes that user (relationships, DM peers, selected-guild member interests); update cached author/profile labels |
| relationship_upsert | Affected account's streams, kind personalized |
| relationship_removed | Affected account's streams; delete relationship, not shared history |
| presence_updated | Authorized base friends/visible DM peers and selected custom interests; never invisible to others |
| guild_upsert | All current guild members; personalized role/permissions; includes name/icon/count/ownership changes |
| guild_joined | Newly joined/creating account's streams; atomically add sidebar and read state |
| guild_removed | Leaving/kicked/banned user, or all on delete; atomically purge all guild children/history/interests/media |
| member_upsert | Selected-guild subscriber streams for join/role/profile changes; own streams too for own role; all get corresponding guild_upsert when count or their role changes |
| member_removed | Selected-guild subscribers still in guild; departing account gets guild_removed instead |
| invite_upsert | Creator and current manage_guild streams; create/use-count changes; invite codes never broadcast to ordinary peers |
| invite_removed | Same privileged invite audience; revoke/expiry/exhaustion; expiry cleanup may be delayed up to housekeeping interval, join always checks expiry immediately |
| ban_upsert | Current ban_members streams on new ban; never unrelated members/target account |
| ban_removed | Current ban_members streams on unban |
| channel_upsert | All authorized guild members or visible DM/group participants; full replacement metadata; needed user profiles precede new DM |
| channel_removed | Channel deletion to guild members; own closed DM=left, removed group member=kicked, leaving member=left; purge sidebar/history/typing/read state/media |
| channels_reordered | All guild members, complete post-change placement list; category deletion emits removal then reordered |
| message_created | All current guild/channel or DM/group member streams, including author; no history subscription needed |
| message_updated | Same access audience; full post-edit state including personalized reactions |
| message_deleted | Same access audience; remove body or show reply placeholder, remove cached reactions/attachments |
| reaction_updated | Same access audience, absolute post-change summary; count=0 removes emoji from list |
| typing | Same access audience except typing author; respect block rule for direct; no storage/history; expire locally without polling |
| read_state_updated | Only affected reader's streams; absolute counts, never infer counts solely from message events |

### REST

| Entry | Checked documented behavior |
|---|---|
| GET `/health` | `HealthResponse {version:String,protocol_version:u16}`; no internal config details |
| POST `/auth/register` | 201 `AuthResponse` |
| POST `/auth/login` | `AuthResponse` |
| POST `/auth/logout` | 204; revoke caller token and close socket |
| GET `/me` | `Snapshot<SelfUser>` |
| GET `/users?ids=1,2,...` | `Snapshot<Vec<UserSummary>>`, sorted by ID |
| GET `/relationships` | `Page<Relationship>` |
| GET `/guilds` | `Page<GuildSummary>` |
| GET `/guilds/{guild_id}` | `Snapshot<GuildDetail>` |
| GET `/guilds/{guild_id}/channels` | `Page<ChannelSummary>` (ID pagination; client sorts position) |
| GET `/guilds/{guild_id}/members` | `Page<Member>` |
| GET `/guilds/{guild_id}/invites` | `Page<Invite>`; members see own, admin/owner all |
| GET `/guilds/{guild_id}/bans` | `Page<Ban>` |
| GET `/dms` | `Page<ChannelSummary>` of visible direct/groups only |
| GET `/channels/{channel_id}` | `Snapshot<Channel>` |
| GET `/channels/{channel_id}/messages` | `Page<Message>` of nondeleted messages |
| GET `/channels/{channel_id}/messages/{message_id}` | `Snapshot<Message>` |
| GET `/read-states` | `Page<ReadState>`; text/visible DM channels only |
| POST `/uploads` | 201 `UploadResponse {attachment:Attachment,expires_at:UnixMillis}` |
| DELETE `/uploads/{attachment_id}` | 204; own pending upload only, repeat missing own ID not_found |
| GET `/files/{attachment_id}` | Original binary, 200/206/304; typed Content-Type/Length/Disposition and ETag |
| GET `/files/{attachment_id}/thumbnail` | Thumbnail binary, 200/304, with Content-Type/Length/ETag |
| GET `/ws` | 101, then Hello/Identify |


## Load test

`examples/loadtest.rs` seeds a disposable database and starts a **separate release
server process**. 300 actual authenticated binary-MessagePack WS clients occupy
30 guilds of ten members, await Ready, send heartbeats, and each send one roughly
200-character chat message every ten seconds (staggered), for 60 seconds. A
two-second receive-only drain checks delivery. Fixture passwords are intentionally
not login-capable; production registration/rate rules are not disabled.

Non-self fan-out latency uses a shared client monotonic clock and timestamps in
actual messages; it includes scheduling, loopback transport, SQLite commit, fan-out
and receiver decode. Server-only CPU uses Win32 GetProcessTimes; RSS is working
set from GetProcessMemoryInfo, sampled once/second, **not Linux RSS**. Client CPU/
memory is excluded. Other host work can affect scheduling. CPU percent refers
to **one logical core**, on a 16-logical-CPU host. Initial history is empty; there
are no uploads/history/member-panel subscriptions or image/hash stress load.

Final production-release run (60-second send phase; 60.515-second CPU/RSS sampling interval):

| Measurement | Observed |
|---|---:|
| Simultaneous clients / guilds | 300 / 30 |
| Messages / successful responses | 1,800 / 1,800 |
| Chat rate | 30.0 messages/second |
| Non-self deliveries / expected | 16,200 / 16,200 |
| Fan-out p50 / p99 / maximum | 2.221 / 3.519 / 6.738 ms |
| Server RSS minimum / mean / peak | 17.33 / 21.11 / 23.42 MiB |
| Server CPU | 5.609375 CPU-seconds; 9.27% of one logical CPU |
| RSS samples | 61 |
| Executable bytes | 8,172,032 |

Release executable SHA-256: e745061dc10db37555ec86932a3d747041f42c6b7f534750f59faf6c085ce7e8.

All 1,800 commands succeeded and all 16,200 non-self deliveries arrived within
the two-second drain. No rejected command, unexpected disconnect or detected
sequence regression occurred in this run. The JSON retains unrounded values.

Reproduce from the repository root:

```powershell
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-audit'
$cargo = "$env:USERPROFILE/.cargo/bin/cargo.exe"
& $cargo build --release -p sticktalk-server --example loadtest
& $cargo build --release -p sticktalk-server
& "$env:CARGO_TARGET_DIR/release/examples/loadtest.exe" --server "$env:CARGO_TARGET_DIR/release/sticktalk-server.exe" --seconds 60
```

The harness kills/waits only for its child and removes only its temporary
fixture, never the user's real data. Raw results: `loadtest-results.json`.

## Validation / limits

Caddy timeout/body settings were checked against [the official global options](https://caddyserver.com/docs/caddyfile/options) and the actual adapter/validator, not only text inspection.

Passed after fixes:
- `cargo test -p sticktalk-server`: **25 unit + 9 integration tests**, no failures.
- `cargo clippy -p sticktalk-server --all-targets -- -D warnings`.
- `cargo fmt --all -- --check` (read-only, including concurrent client source).
- `cargo build --release -p sticktalk-server --bins --example loadtest`.
- Production-only release build: cargo build --release -p sticktalk-server, exit 0.
- Final 300-client/30-guild/60-second load harness: exit 0.
- `bash -n deploy/install.sh`, `bash -n deploy/backup.sh` (Git Bash).
- Caddy v2.11.9: `caddy adapt --validate --config deploy/Caddyfile`, exit 0.

Command output is retained in audit-validation.log and audit-deploy-validation.log.


- This is not an ARM/Linux capacity or SLA claim. No live Oracle firewall,
  systemd sandbox, TLS issuance or backup/decrypt/restore drill was performed.
  Caddy adaptation/validation checks configuration, not public connectivity.
- A 60-second empty-history text test is not a long-history soak, image-heavy
  or password-hash benchmark. Read counters still recount touched history;
  large retained histories require separate benchmarking.
- Open registration/durable database growth remain product policy. Logical
  upload admission does **not** reserve OS free space against SQLite/WAL, other
  apps, backups or files waiting for GC. Use a dedicated volume/quota, monitor
  free space, leave headroom and restrict network access where appropriate.
- Application quotas are not volumetric DDoS/TCP-connection protection; Caddy,
  host/cloud firewall controls and operational monitoring remain necessary.
- A full oversized payload can cause TCP reset before a best-effort WS close
  reaches the peer. The close-code test sends only the oversized frame header
  to distinguish early application rejection from unread-payload TCP behavior.
- Concurrent client work was not modified; workspace fmt is a point-in-time
  check, not client correctness validation.
# B2/B4/B5/B9 operational follow-up

The original audit below is historical. The resumable REST API now defaults to
unlimited file size, with persisted offsets, streamed whole-file SHA-256,
post-network authorization, fsync-before-offset commit, 24-hour abandoned-partial
GC, disk headroom reservations, and optional quotas. Legacy multipart retains its
25-MiB compatibility bound and daily hardening. Both paths share pending/disk/quota
accounting. `If-Range` now protects resumed original downloads. See the leading
operational section in `docs/PROTOCOL.md` for exact routes/DTOs and expiry behavior.

Publication keeps generated `.part` storage keys unchanged: cancellation cannot
strand a rename between filesystem and SQLite commit. Startup/minutely orphan
scans recognize partial reservations. Hashing uses 64-KiB buffers and one disk
gate; oversized/duplicate chunks cannot advance an offset. GC skips a busy disk
gate so large hash jobs cannot block session-revocation maintenance. Filesystem
free-space admission is not an OS quota against other processes or backup writes.

Ed25519 release verification uses ring; only the public key is configured on the
server. The exact signed UTF-8 manifest string commits to the installer's hash,
size, version, target and HTTPS content-addressed URL. Publication requires an
independent constant-time-checked publishing token, streams the body, verifies
signature/size/hash, and rejects changing an already-published version's hash.
Release-token Debug formatting is redacted. This is not Authenticode, and client
verification/downgrade prevention remains the client engineer's responsibility.

The local admin CLI deliberately opens SQLite directly, **not** `App::open`, so
it cannot run server-startup cleanup against another process's active upload.
One-time recovery codes are random 256-bit values, SHA-256-at-rest, expire after
30 minutes, and are consumed transactionally with password replacement/session
revocation. Disabling revokes password and Google sessions and blocks issuance
(including a SQLite trigger). Removed sockets close at heartbeat/maintenance;
normal authenticated mutations reject revoked sessions immediately.

Validation (all build output under `C:/Users/dwhee/.cargo-target/sticktalk-server`):

- `cargo test -p sticktalk-server -p sticktalk-proto`: **68 tests**, passed
  (47 server unit, 9 server integration, 12 protocol).
- `cargo clippy -p sticktalk-server -p sticktalk-proto --all-targets -- -D warnings`.
- `cargo fmt --all -- --check`, including a read-only workspace check.
- `deploy/pi/build.ps1`: Windows cargo-zigbuild 0.24.1 + Zig 0.13.0, release
  ARM64 Linux **link**, ELF64/AArch64 verification; glibc 2.36 target baseline.
  Final ELF: 10,341,344 bytes, SHA-256
  `5c80dbb7bc4441ccf12e32b3dcb31ac175f69b82c02422a8b7d0fd78f76604ca`;
  dynamic loader `/lib/ld-linux-aarch64.so.1`, highest GLIBC symbol version 2.34.
- Signing helper keygen/sign smoke test with an ephemeral external key, deleted
  after use. Real HTTP update routes and download bytes tested locally.
- Bash syntax checks for the three Pi shell scripts; PowerShell parser checks
  for Pi build/deploy and release scripts. Caddy adaptation and validation with
  an ephemeral test certificate (no listener/server started).

Limits: no physical Pi/systemd/Tailscale certificate issuance, remote deployment,
off-device restore drill, or DERP/media performance test. The Pi guide explicitly
hands actual SFU bind/UDP/MTU/bitrate/relay integration to the separate SFU work.
The NSIS installer script has not been compiled here: NSIS is not installed and
the upstream portable download returned HTML rather than a ZIP. No production
signing key, publishing token, release upload, or client-source edit was made.

