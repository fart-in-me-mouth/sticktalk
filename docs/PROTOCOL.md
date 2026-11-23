# StickTalk protocol v1

This is the normative v1 contract, including the launch extensions in section 11. `crates/proto/src/{ids,permissions,
models,rest,gifs,launch,ws}.rs` defines **every serialized field**; names/types below match
those files. The server implements this contract. All limits
and authorization rules are enforced by the service, not by serde.
See `ARCHITECTURE.md` for implementation, storage, limits and resource budgets.

## 1. Encoding and common rules

* REST base `/api/v1`, JSON UTF-8 except multipart uploads and binary downloads.
  WebSocket `/api/v1/ws`, RFC 6455 binary messages only, subprotocol
  `sticktalk.v1` (required). Version in Hello/Identify must also equal 1.
* One complete `ClientMsg`/`ServerMsg` per binary **message**. Fragmented WS
  frames may be reassembled only within the 512 KiB total bound. No concatenated
  MessagePack values, trailing bytes, compression, text messages or extensions.
* Encode with `rmp_serde::to_vec_named` and decode with `from_slice` plus a
  transport check for complete input consumption. Structs are named maps, not
  positional arrays. Enum tags/fields are snake_case. Ordinary enums are strings.
  Message/command/event/result envelopes are adjacent tags
  `{"type":"variant","data":...}`. Unit variants omit `data`.
  `CommandOutcome` uses `{"status":"ok|err","data":...}`. `Patch<T>` uses
  `{"op":"keep|set|clear","value":...}` with value present only for set.
  Boxes have no wire effect. Option fields serialize null; Vec fields are arrays.
* Unknown map fields are ignored. Unknown variants, wrong types, duplicate
  required fields, unsupported versions, oversized/nested values are rejected;
  no silent interpretation of future commands. Add optional fields compatibly;
  changing required fields/semantics requires a protocol version change.
* IDs are positive `Id(u64)` values <= 2^63-1, numeric in JSON/MessagePack and
  decimal strings in URL/multipart text. Zero invalid. Treat as opaque except
  order; aliases UserId/GuildId/ChannelId/MessageId/AttachmentId all use Id.
  JSON clients must preserve 64-bit integers (the native Rust client does).
* `UnixMillis`: nonnegative UTC epoch milliseconds. `EventSeq`: stream-local
  nonnegative integer, zero is valid before any events. `Nonce`: 1..2^63-1,
  generated randomly with a CSPRNG and never deliberately reused for another
  command. Heartbeat nonces need only be unique while outstanding.
* Auth REST header `Authorization: Bearer <token>`; never put token in URLs.
  Auth/session responses use `Cache-Control: no-store`. API JSON errors are
  always `ApiError`, not HTML, framework default text, or a different wrapper.
* `ResumeCursor {stream_id: String, seq: EventSeq}`: stream_id is 32 lowercase
  hex characters from 16 CSPRNG bytes, scoped to one login token. It is not a
  credential, global sequence or entity ID. `Page<T>`/`Snapshot<T>` use it too.
* Validation limits are measured as UTF-8 **bytes** unless explicitly called
  Unicode scalar values. Do not trim/normalize message content. Identifiers and
  names trim surrounding Unicode whitespace; username canonicalizes ASCII
  lowercase. Uniqueness compares canonical username, not display name.

### Validation reference

| Value | Accepted bounds |
|---|---|
| Username | `[a-z0-9_]{3,32}` after ASCII lowercase/trim; unique; no display-name lookup ambiguity |
| Password | 1..128 UTF-8 bytes; no trimming, case folding, or Unicode normalization |
| Display name | null or 1..64 bytes after trim |
| Status text | null or 1..128 bytes; clear rather than empty |
| Guild/group name | 1..64 bytes after trim |
| Guild channel/category name | 1..64 bytes after trim; no path/control characters |
| Topic | null or 1..512 bytes; category topic must be null |
| Message | <=4,000 Unicode scalar values; non-whitespace text or >=1 attachment; edits cannot make an attachment-free message empty |
| Mention IDs | <=100 explicit unique users with channel access; server-expanded role/everyone/here recipients are bounded by the 1000-member audience (section 12) |
| Attachments | <=4 unique IDs/message; each <=26,214,400 bytes |
| Emoji | One Unicode emoji grapheme cluster, 1..32 UTF-8 bytes; support variation selectors, modifiers and ZWJ; no custom emoji; exact sequence is key, no normalization |
| Reactions | <=20 distinct emoji/message; one reaction/user/emoji; count excludes no one with access |
| Invite | 16 random bytes encoded unpadded base64url; ttl_seconds 60..604800; max_uses 1..1000; uses starts 0 |
| Moderation reason | null or 1..256 bytes after trim |
| Filename | sanitized basename 1..255 bytes; never a filesystem path |
| Pages | default/max 50, minimum 1; before and after mutually exclusive |
| Bulk user IDs | 1..50 unique IDs; comma-separated decimal query values |
| Presence interests | <=100 unique users from the selected guild; none without a guild |

Guild/user/DM/session quotas and all rate limits in ARCHITECTURE apply in
addition. Self-friend/block/DM, duplicate recipient IDs, invalid enum combinations,
cross-guild parents, a channel nested inside itself, and cross-channel message targets
are invalid_request. Limits are atomic, never partial success or truncation.

## 2. Shared entity field reference

Notation `?T` means `Option<T>` (null), `[T]` means `Vec<T>`, and integer widths
are the Rust widths. Every struct has snake_case keys.

| Type | Fields and meaning |
|---|---|
| `UserSummary` | `id:UserId`, `username:String`, `display_name:?String`, `avatar_id:?AttachmentId`, `status_text:?String`; public profile, never password/email/token |
| `SelfUser` | `user:UserSummary`, `desired_presence:PresenceStatus`; private account preference |
| `ProfilePatch` | `username:Patch<String>`, `display_name:Patch<String>`, `avatar_id:Patch<AttachmentId>`, `status_text:Patch<String>`; omitted fields default keep; clear username invalid |
| `Presence` | `user_id:UserId`, `status:PresenceStatus` |
| `Relationship` | `user:UserSummary`, `kind:RelationshipKind`, `since:UnixMillis`; kind is from recipient's perspective |
| `Guild` | `id:GuildId`, `owner_id:UserId`, `name:String`, `icon_id:?AttachmentId`, `member_count:u32` |
| `GuildSummary` | `guild:Guild`, `my_role:MemberRole`, `my_permissions:Permissions` |
| `GuildPatch` | `name:Patch<String>`, `icon_id:Patch<AttachmentId>`; omitted keep; cannot clear name |
| `Member` | `guild_id:GuildId`, `user:UserSummary`, `role:MemberRole`, `permissions:Permissions`, `joined_at:UnixMillis`, `role_ids:[RoleId]`, `nickname:?String` |
| `ChannelSummary` | `id:ChannelId`, `kind:ChannelKind`, `guild_id:?GuildId`, `parent_id:?ChannelId`, `name:?String`, `position:u16`, `recipient_ids:[UserId]`, `owner_id:?UserId` |
| `Channel` | `summary:ChannelSummary`, `topic:?String`; topic fetched lazily, not in Ready |
| `ChannelPatch` | `name:Patch<String>`, `topic:Patch<String>`; omitted keep; cannot clear name of named channel |
| `ChannelPlacement` | `channel_id:ChannelId`, `parent_id:?ChannelId`, `position:u16` |
| `Invite` | `id:Id`, `code:String`, `guild_id:GuildId`, `creator_id:UserId`, `expires_at:UnixMillis`, `max_uses:u32`, `uses:u32` |
| `Ban` | `guild_id:GuildId`, `user:UserSummary`, `actor_id:UserId`, `reason:?String`, `created_at:UnixMillis` |
| `Attachment` | `id:AttachmentId`, `filename:String`, `content_type:String`, `byte_size:u64`, `width:?u32`, `height:?u32`, `thumbnail:?Thumbnail`; dimensions null for non-images |
| `Thumbnail` | `width:u16`, `height:u16`, `byte_size:u32`, `content_type:String`; WebP or JPEG, max 256x256/64 KiB |
| `ReactionSummary` | `emoji:String`, `count:u32`, `me:bool`; me is personalized to viewing account, never the message author by default |
| `Message` | `id:MessageId`, `channel_id:ChannelId`, `author_id:UserId`, `content:String`, `created_at:UnixMillis`, `edited_at:?UnixMillis`, `reply_to:?MessageId`, `attachments:[Attachment]`, `mention_ids:[UserId]`, `reactions:[ReactionSummary]` |
| `MessageDraft` | `channel_id:ChannelId`, `content:String`, `reply_to:?MessageId`, `attachment_ids:[AttachmentId]`, `mention_ids:[UserId]` |
| `MessageEdit` | `channel_id:ChannelId`, `message_id:MessageId`, `content:String`, `mention_ids:[UserId]`; both content/mentions replace, not append |
| `ReadState` | `channel_id:ChannelId`, `last_read_id:?MessageId`, `latest_message_id:?MessageId`, `unread_count:u32`, `mention_count:u32` |
| `ReactionUpdate` | `channel_id:ChannelId`, `message_id:MessageId`, `user_id:UserId` (actor), `added:bool`, `reaction:ReactionSummary` (absolute post-operation count/me, including count=0) |
| `GuildDetail` | `summary:GuildSummary`, `channels:[ChannelSummary]` (authorized guild channels, no member/history bodies) |
| `VoicePayload` | `nonce:Nonce`, `version:u16`, `kind:String`, `payload:[u8]`; legacy reserved bodies still decode; enabled typed body <=72 KiB, see section 10 |

Enums: `PresenceStatus` = online, idle, dnd, invisible, offline;
`RelationshipKind` = friend, incoming_request, outgoing_request, blocked;
`MemberRole` = owner, admin, member; `ChannelKind` = category, text, direct,
group_dm, voice (Phase 2); `RemovalReason` = left, kicked, banned, deleted;
`UploadPurpose` = message, avatar, guild_icon;
`ResyncReason` = stream_expired, cursor_too_old, cursor_ahead, slow_consumer.

### Channel invariants

Guild text/category: guild_id present, recipient_ids empty, owner_id null, name
present. Category topic null. **Nesting:** parent_id is null or any other
category, text or voice channel of the same guild, so channels nest to any
depth (bounded only by the guild channel limit); a channel is never inside
itself, directly or through its descendants. Positions are zero-based/dense
per sibling set.
Direct: guild/parent/name/topic/owner null, position 0, exactly two sorted
recipient_ids **including self**. Group: guild/parent/topic null, position 0,
name present, 1..16 sorted recipient_ids including self, owner one of them.
Categories cannot have messages, typing, reactions or read states.

Ready channels and GuildDetail channels are sorted root siblings by position/id,
with each channel followed, depth first, by its similarly sorted children; DMs
follow by ID.
The client may reorder DM display by latest_message_id without mutating position.

### Permissions numeric bits

| Bit | Name | Allows |
|---|---|---|
| 0 | view_channel | Channel metadata/access |
| 1 | send_messages | Sending/editing own messages, typing |
| 2 | read_history | History/specific message fetch |
| 3 | attach_files | Upload/attach to channel |
| 4 | add_reactions | Add/remove own Unicode reaction |
| 5 | manage_messages | Delete others' messages (not edit them) |
| 6 | manage_channels | Guild channel/category create/update/reorder/delete |
| 7 | manage_guild | Guild name/icon, view/revoke all invites |
| 8 | create_invites | Create invite and list/revoke own invites |
| 9 | kick_members | Kick lower-role members |
| 10 | ban_members | Ban/unban lower-role members, view bans |
| 11 | manage_members | Deprecated legacy bit; use manage_roles (14) |
| 12 | administrator | All currently defined guild action bits, not ownership/hierarchy bypass |
| 13 | owner | Internal owner sentinel; cannot be assigned by a role/overwrite |
| 14 | manage_roles | Manage lower roles and channel overwrites |
| 15 | change_nickname | Change own per-server nickname |
| 16 | manage_nicknames | Change lower members' nicknames |
| 17 | embed_links | Allow link embeds |
| 18 | mention_everyone | Allow broadcast mentions |
| 19 | connect | Join voice |
| 20 | speak | Publish voice |
| 21 | stream | Share screen/system audio |
| 22 | video | Publish camera |
| 23 | mute_members | Server-mute voice members |
| 24 | deafen_members | Server-deafen voice members |
| 25 | move_members | Move voice members |
| 26 | use_voice_activity | Use VAD |
| 27 | priority_speaker | Priority speaker |
| 28..63 | reserved | Never grant unknown permissions; bit 63 not stored in SQLite |

The legacy `Permissions::MEMBER` (16671), `ADMIN` (24575), and `GUILD_OWNER`
(32767) constants retain their numeric values for wire compatibility, but are
**not** authorization presets. Authorization uses the role evaluator in section
11. Owner-only actions and hierarchy take precedence over Administrator.
Clients preserve unknown bits without granting them. DMs authorize by
participant status; group owner alone manages group metadata/recipients. Any
participant may delete their own messages, not other participants' messages.

## 3. REST endpoint contract

All paths below are relative to `/api/v1`. All except health, register/login,
and WS upgrade require bearer auth. Successful GET=200. No content=204 with
empty body. Request/response type names are public proto types. Path and query
IDs are parsed losslessly and validated. No mutations for ordinary live chat
are exposed via REST.

### Common response wrappers and pagination

`Snapshot<T> {data:T,cursor:ResumeCursor}` is a coherent read at that session
stream cursor. `Page<T> {items:[T],users:[UserSummary],next_before:?Id,
next_after:?Id,has_more:bool,cursor:ResumeCursor}`. Users contains deduplicated
message authors (always), plus other referenced users when available; clients
must tolerate unresolved mentions/replies and fetch names lazily. It is empty
when items already embed user summaries or no extra summaries are needed.

`PageQuery {before:?Id,after:?Id,limit:?u16}` serializes as ordinary query keys
(omit nulls). Messages: no cursor or before selects descending newest/older;
after selects ascending newer. Other collections default ascending by key;
after is ascending and before descending. A cursor is **exclusive** and need
not still exist. Sort by message.id, guild.id, channel.id, member.user.id,
relationship.user.id, ban.user.id, invite.id, or read_state.channel_id as
appropriate. Fetch limit+1 to determine has_more. Only the active direction's
next cursor is set, to the last emitted item's key, **only if has_more**; both
null at end/empty. Client reorders displayed message pages chronologically.
Pagination is not a multi-page historical snapshot: reconcile each page with
its own cursor and live events; new inserts may arrive between pages.

| Method/path | Request | Success | Additional expected errors |
|---|---|---|---|
| GET `/health` | none, unauthenticated | `HealthResponse {version:String,protocol_version:u16}`; no internal config details | unavailable (503, before DB/migrations ready) |
| POST `/auth/register` | `RegisterRequest {username,password,display_name:?String}` | 201 `AuthResponse` | username_taken, invalid_request, rate_limited, limit_exceeded |
| POST `/auth/login` | `LoginRequest {username,password}` | `AuthResponse` | invalid_credentials, rate_limited, limit_exceeded |
| POST `/auth/logout` | no body | 204; revoke caller token and close socket | unauthenticated, session_expired |
| GET `/me` | none | `Snapshot<SelfUser>` | common |
| GET `/users?ids=1,2,...` | 1..50 IDs, no PageQuery | `Snapshot<Vec<UserSummary>>`, sorted by ID | invalid_request; not_found if **any** user inaccessible (no partial profile leakage) |
| GET `/relationships` | PageQuery | `Page<Relationship>` | common |
| GET `/guilds` | PageQuery | `Page<GuildSummary>` | common |
| GET `/guilds/{guild_id}` | none | `Snapshot<GuildDetail>` | not_found |
| GET `/guilds/{guild_id}/channels` | PageQuery | `Page<ChannelSummary>` (ID pagination; client sorts position) | not_found |
| GET `/guilds/{guild_id}/members` | PageQuery (user ID keys) | `Page<Member>` | not_found |
| GET `/guilds/{guild_id}/invites` | PageQuery | `Page<Invite>`; members see own, admin/owner all | not_found, forbidden |
| GET `/guilds/{guild_id}/bans` | PageQuery (banned user keys) | `Page<Ban>` | not_found, forbidden |
| GET `/dms` | PageQuery | `Page<ChannelSummary>` of visible direct/groups only | common |
| GET `/channels/{channel_id}` | none | `Snapshot<Channel>` | not_found |
| GET `/channels/{channel_id}/messages` | PageQuery | `Page<Message>` of nondeleted messages | not_found, invalid_request (category) |
| GET `/channels/{channel_id}/messages/{message_id}` | none | `Snapshot<Message>` | not_found (deleted included) |
| GET `/read-states` | PageQuery (channel ID keys) | `Page<ReadState>`; text/visible DM channels only | common |
| POST `/uploads` | multipart below | 201 `UploadResponse {attachment:Attachment,expires_at:UnixMillis}` | invalid_request, not_found, forbidden, payload_too_large, unsupported_media_type, limit_exceeded, unavailable |
| DELETE `/uploads/{attachment_id}` | none | 204; own pending upload only, repeat missing own ID not_found | not_found, conflict (attached) |
| GET `/files/{attachment_id}` | optional Range/If-None-Match | Original binary, 200/206/304; typed Content-Type/Length/Disposition and ETag | not_found, invalid_request (bad/multiple range), 416 unsatisfiable range |
| GET `/files/{attachment_id}/thumbnail` | optional If-None-Match | Thumbnail binary, 200/304, with Content-Type/Length/ETag | not_found (including no thumbnail), invalid_request (Range supplied) |
| GET `/ws` | WS upgrade, subprotocol `sticktalk.v1`, no token in headers required | 101, then Hello/Identify | 400 invalid upgrade, 426 unsupported/missing subprotocol; after upgrade errors use WS |

`AuthResponse {token:String,expires_at:UnixMillis,user:SelfUser}`. Register logs
in automatically; default desired presence online. Token is 43-char unpadded
base64url of 32 random bytes, absolute expiry 30 days, only SHA-256 stored.
Expired token=session_expired; missing/invalid/revoked=unauthenticated. Logout
does not log out other sessions. No refresh/password-reset endpoint in Phase 1.

Upload multipart fields must be ordered `purpose`, optional `channel_id`, then
`file`; exactly one file, no duplicate/unknown fields. Purpose matches
UploadPurpose strings. channel_id required only for message, absent for avatar/
guild_icon. The proto `UploadMetadata` represents the text fields (its Rust
Option is not a literal `null` multipart part). Filename comes from file part.
Cap overhead 64 KiB beyond the 25 MiB file. Validate pending ownership/purpose
again when attaching. The original is immediately readable by its uploader
while pending; after attachment normal entity ACL applies. Pending expires one
hour after upload. Reject quota-exhausting uploads before and during streaming;
clean temporary files on errors/disconnect. The file/thumbnail read ACL and
cache/range headers are specified in ARCHITECTURE. A 416 uses ApiError code
invalid_request plus `Content-Range: bytes */<size>`. Successful 304 has no body.

Common authenticated endpoint errors: invalid_request, unauthenticated,
session_expired, not_found/forbidden where applicable, rate_limited,
internal/unavailable. Inaccessible profile means not self, not relationship,
and no shared guild/DM. Bulk/history access is always rechecked; being in a
previous Ready does not confer access. JSON request limit 16 KiB (auth only).
Framework body/type/route errors must map to ApiError. Caddy's own network/TLS
failure is not an application response; clients must tolerate non-JSON errors.

### Optional Google authentication (additive v1 endpoints)

The following paths are relative to `/api/v1`; these new endpoints also accept
`/api` as a base to match the installed-app Google login spec. Existing non-Google
REST endpoints retain `/api/v1`. No existing request/response fields change.
All responses/errors below use `Cache-Control: no-store`; auth bodies are JSON,
maximum 16 KiB. `UnixMillis` is UTC milliseconds, including ticket expiry.

| Method/path | Authorization / request | 200 response |
|---|---|---|
| GET `/meta` | Public, no body | `MetaResponse {version:String,protocol_version:u16,google_client_id:?String,gifs:GifCapability}` |
| POST `/auth/google` | Public; `GoogleAuthRequest {id_token:String}` | `GoogleAuthResponse` below |
| POST `/auth/google/complete` | Public; `GoogleCompleteRequest {ticket:String,username:String}` | Existing `AuthResponse {token,expires_at,user}` |
| POST `/auth/google/link` | Bearer; `GoogleLinkRequest` (alias of `GoogleAuthRequest`) `{id_token:String}` | `AuthMethodsResponse {has_password:bool,google_linked:bool}` |
| POST `/auth/google/unlink` | Bearer; `GoogleUnlinkRequest {}` | `AuthMethodsResponse` |
| GET `/auth/methods` | Bearer, no body | `AuthMethodsResponse` |
| POST `/auth/password` | Bearer; `SetPasswordRequest {current_password:?String,new_password:String}` | `AuthMethodsResponse` |

`GoogleAuthResponse` is adjacent-tagged (like other protocol enums), exactly:

```json
{"type":"session","data":{"token":"OPAQUE_SESSION","expires_at":123456789,"user":{"user":{"id":1,"username":"alice","display_name":null,"avatar_id":null,"status_text":null},"desired_presence":"online"}}}
{"type":"signup_required","data":{"ticket":"OPAQUE_SIGNUP_TICKET","expires_at":123456789}}
```

Rust variants are `GoogleAuthResponse::Session(AuthResponse)` and
`GoogleAuthResponse::SignupRequired(GoogleSignupRequired {ticket:String,
expires_at:UnixMillis})`. A Google-only user's normal `SelfUser`/Ready is
unchanged; use `/auth/methods` when showing account settings.

The client gets the public Google desktop client ID from `/meta` and hides
Google buttons when it is null. The Google desktop client secret is **never**
returned by this endpoint. Client browser/loopback code handles PKCE S256 and
state; it sends only the resulting ID token, never an authorization code,
Google access/refresh token, email assertion, or Google subject alone.
The server verifies RS256 using Google's cached HTTPS JWKS, issuer (either
`accounts.google.com` or `https://accounts.google.com`), exact string audience,
exp/iat (60-second skew), nonempty subject and boolean `email_verified=true`.
Invalid/malformed/wrong-audience/expired/unverified tokens are 401
`invalid_credentials`. Key-fetch failures or expired-cache refresh backoff
are 503 `unavailable`; unknown keys after a successful fetch are invalid
credentials. Verification never falls back to a different algorithm or email.

An existing provider+subject issues the normal session. An unknown subject
gets a random 43-character signup ticket, ten minutes, SHA-256 stored only.
Only one outstanding ticket per subject exists: requesting a new one replaces
the previous ticket. At most 4,096 unexpired tickets globally. Complete uses
ordinary username canonicalization/limits, returns `username_taken` on collision
and lets the user retry with the same ticket. Success consumes it atomically
with user, identity and session creation. Expired, unknown, consumed or
superseded tickets all return 401 `invalid_signup_ticket`; signing in again
obtains a new ticket. Neither email nor Google display name auto-selects or
merges a StickTalk account.

Linking requires current StickTalk bearer auth and proof of Google control.
Linking the same subject to the same account is idempotent. A subject linked
to another account, or an account with a different Google subject, returns 409
`identity_already_linked`, without exposing the other account. Linking also
invalidates outstanding signup tickets for that subject. Unlink is idempotent
when absent, but linked Google cannot be removed without a password (409
`last_login_method`). There is at most one Google identity per account.

For Google-only users, setting a password accepts `current_password:null`;
password replacement requires a matching current password, otherwise 401
`invalid_credentials`. New passwords use existing 1..128-byte validation and
Argon2id policy. Sessions remain valid; a concurrent password change returns
409 `conflict`. `/auth/methods` and `/auth/password` remain available even if
Google is disabled, allowing a still-authenticated Google-only user to set a
password. Disabling Google does not revoke existing sessions.

All `/auth/google*` routes return 404 `feature_disabled` when the server client
ID is unset. Enabled auth routes share login's per-IP 10/minute burst-5 limiter
(including failed requests), plus 10/minute burst-5 subject/account limits;
completion also uses the 5/minute burst-3 canonical-name limiter. Standard
rate_limited/retry headers, session quota, body/admission limits apply.

### Optional GIF discovery and account favorites (additive v1)

These endpoints accept both `/api/v1` and `/api` bases. All except public `/meta`
require ordinary bearer authentication. All responses, including errors and
favorites, use `Cache-Control: no-store`. These are private picker preferences
and external discovery data, **not** chat snapshots: no `ResumeCursor`, command
nonce, replay receipts, or WS events. Other sessions refetch favorites when
opening the picker, not by polling. Protocol version remains 1.

`MetaResponse.gifs` defaults to disabled when absent (older servers). Wire values:

```json
false
{"mode":"proxy","provider":"klipy"}
{"mode":"direct","provider":"klipy","public_client_key":"EXPLICITLY_PUBLIC_KEY"}
```

Rust: `GifCapability::Disabled` or `GifCapability::Enabled(GifConfig::Proxy
{provider})` / `GifConfig::Direct {provider, public_client_key}`.
`GifProviderName::Klipy` serializes `"klipy"`. Disabled is **false**, never true,
null, or an object with `mode: disabled`. Hide discovery when disabled. Direct
mode exposes only a separately configured public client key; the server-only
API key is never returned. No provider/key is configured by default. Proxy mode
requires the operator's explicit approval setting as well as provider/key.

| Method/path | Request | Success |
|---|---|---|
| GET `/gifs/trending?cursor=...` | `GifQuery {q:?String,cursor:?String}`; q unused | 200 `GifPage` |
| GET `/gifs/search?q=...&cursor=...` | `GifQuery`; q required, trimmed, 1..200 UTF-8 bytes, no controls | 200 `GifPage` |
| GET `/gifs/categories` | none | 200 `GifCategoriesResponse` |
| GET `/gifs/items?ids=klipy:123,klipy:456` | `GifItemsQuery {ids:String}`; 1..24 IDs, <=3300 bytes | 200 `GifPage`; order unspecified, missing/removed IDs may be omitted |
| GET `/gifs/favorites` | none; works in disabled, direct and proxy modes | 200 `GifFavoritesResponse {ids:[String]}` |
| PUT `/gifs/favorites/{id}` | no body; e.g. `/gifs/favorites/klipy:123` | 204; idempotently star for caller's account |
| DELETE `/gifs/favorites/{id}` | no body | 204 even if absent; caller's account only |

Discovery/items return 404 `feature_disabled` in disabled **or direct** mode;
clients must not silently fall back to proxying in direct mode. Unknown routes
or methods are `not_found`. Normal auth errors apply in every mode. Invalid
query/cursor/ID is 400 `invalid_request`. Account rate limiting is 429
`rate_limited`, with `retry_after_ms` and `Retry-After` seconds. Upstream quota
errors also map to 429. Provider timeouts, malformed/oversized/unsafe responses,
redirects and other provider failures are 503 `unavailable`; provider error
bodies, key-bearing paths and secrets are never forwarded. Favorites exceeding
the account limit return 409 `limit_exceeded`; re-starring an existing ID at
the limit still succeeds. No endpoint takes another account's ID.

Exact additional proto models (`crates/proto/src/gifs.rs`):

| Type | Fields |
|---|---|
| `GifMedia` | `url:String,width:u16,height:u16,content_type:String` |
| `GifItem` | `id:String,title:String,preview:GifMedia,send:GifMedia` |
| `GifPage` | `items:[GifItem],next_cursor:?String` |
| `GifCategory` | `title:String,query:String,preview:GifMedia` |
| `GifCategoriesResponse` | `items:[GifCategory]` |
| `GifFavoritesResponse` | `ids:[String]` |

`GIF_PAGE_SIZE=24`, `MAX_GIF_FAVORITES=200`. Cursors are opaque strings, 1..256
bytes without controls; URL-encode without interpreting, incrementing or
truncating them. Omit cursor for the first page; null means no next page. IDs
are **strings, not Snowflakes**: `klipy:` followed by 1..128 ASCII alphanumeric,
underscore or hyphen characters. IDs remain provider-qualified in favorites.
Favorites are newest-added first, breaking ties by ascending ID; duplicate PUT
does not reorder them. SQLite stores **only these IDs and creation timestamps**,
not titles, provider content or media URL copies. Deleting an account cascades
its favorites. Removed content may resolve to no item; retain the ID so the
user can unstar it. ID resolution is bounded to 24 per request.

All returned media URLs are unchanged HTTPS provider CDN URLs, not StickTalk
file URLs. KLIPY CDN hosts: `static.klipy.com`, `static1.klipy.com`,
`static2.klipy.com`; credentials, fragments and nonstandard ports are rejected.
Preview is nano WebM (preferred) or nano MP4, no more than 320x320; no full GIF
preview fallback. Send is `mediumgif`, falling back to tiny WebM/MP4 when
unavailable, with dimensions <=4096 on either axis. `content_type` is
`video/webm`, `video/mp4`, `image/webp`, or send-only `image/gif`. Titles are
1..512 bytes without controls and prefer provider accessibility descriptions.
The category response contains at most eight categories; titles <=128 bytes,
queries <=200. Selecting a category submits **query**, never a provider search
URL. The proxy obtains nano previews using one search per category rather than
returning the provider's large dimensionless category GIFs.

The proxy coalesces concurrent cache misses and retains successful normalized
metadata for ten minutes, at most 128 entries and 8 MiB serialized data. It
does not fetch/cache/rehost media bytes. All picker/favorites requests share a
per-account 30/minute token bucket, burst 10, across login sessions, including
cache hits. All outgoing provider HTTP requests share a conservative 100/hour
budget, including errors and category preview subrequests. Upstream request
deadline 5 seconds; complete refresh 10 seconds inside the existing 15-second
REST deadline; upstream body maximum 1 MiB. Network I/O never holds the chat
mutation gate. Expiration/eviction is demand-driven; no GIF background polling.

**Direct-mode client integration:** use `https://api.klipy.com/v2/featured`,
`/search`, `/categories` and `/posts`, with the designated public key as `key`,
`client_key=sticktalk`, `contentfilter=high`, `limit=24`, and
`media_filter=nanowebm,nanomp4,tinywebm,tinymp4,mediumgif`. Search takes `q`;
pagination translates `cursor` to `pos` and response `next` to `next_cursor`
(empty or `"0"` means end). Convert result `id` to `klipy:<id>`, media
`dims:[width,height]` to `GifMedia`, preferring the renditions above. Categories
use `type=featured&locale=en_US`: `tags[].name/searchterm` map to title/query;
obtain nano preview data via search rather than using `tags[].image`.
Resolve favorite IDs via `/posts?ids=<unqualified comma-separated IDs>` after
stripping the `klipy:` prefix. Still star/unstar on StickTalk, not KLIPY's
customer-history API. Implement provider-required share/analytics behavior in
the client where applicable. Do not forward StickTalk bearer tokens or account
identifiers to the provider. Direct mode necessarily exposes the user's IP to
the provider for **search as well as media**, unlike proxy search. This is the
owner-approved fallback, not the original privacy guarantee. Standard-mode
provider storage restrictions still apply to client media caches; approval is
needed before implementing the disk-cache behavior proposed in GIFS.md.

Sending uses the **existing** `SendMessage(MessageDraft)` with
`content=item.send.url`, no attachments; no new chat message/event schema.
Apply existing permissions and spam checks. Use `Search KLIPY` as the search
placeholder and show `Powered by KLIPY` in the picker. Provider approval,
attribution references and key setup are in `deploy/README.md`.

## 4. WebSocket handshake, session streams and recovery

### Fresh connection

1. Upgrade `/api/v1/ws` with subprotocol `sticktalk.v1`. Server sends
   `ServerMsg::Hello(Hello {version:u16,heartbeat_interval_ms:u32,
   heartbeat_timeout_ms:u32,max_frame_bytes:u32,server_time:UnixMillis})`.
   v1 values: 1, 30000, 75000, 524288, current UTC milliseconds.
2. Within 10 seconds client sends `ClientMsg::Identify(Identify {version:u16,
   token:String,resume:?ResumeCursor})`; fresh clients send resume=null.
   No commands before successful Ready/Resumed. Only one Identify per socket.
3. Server checks token, version, connection quota; closes an older socket for
   this token if present (4009). Different login tokens are independent devices.
4. Under the snapshot gate server attaches stream, emits `Ready`, then releases
   queued newer events. Ready is a full replacement snapshot; its cursor is
   the highest sequence already reflected. Start applying events above it.
5. Client starts heartbeat timer, fetches selected history lazily and issues
   SubscribeGuild for its visible member panel. No initial history polling.

`Ready {cursor,me:SelfUser,guilds:[GuildSummary],channels:[ChannelSummary],
relationships:[Relationship],users:[UserSummary],read_states:[ReadState],
presences:[Presence],voice_states:[GuildVoiceState],roles:[Role],memberships:[Member],
notification_settings:[NotificationSettings]}` is complete at the cursor. Guilds are joined guilds only;
channels are View Channel-authorized guild channels plus visible direct/group channels; relationships
are all own friend/pending/blocked rows. Users provides deduplicated other users
of direct channels needed for unnamed sidebar labels (self and relationship
profiles need not be repeated). Named groups need not preload all participants.
Read states includes every listed text/DM channel, zero counts included.
Presences includes effective self + allowed friends/open-DM participants;
missing presence defaults offline. No guild-wide member arrays, message bodies,
topics, original files, reaction history or password/session secrets. Sort
non-channel arrays by their entity/user/channel IDs for deterministic rendering.

### Heartbeat and presence

Client sends `Heartbeat {nonce:Nonce,last_seq:EventSeq}` every ~30 seconds,
independent of user activity. Server returns `HeartbeatAck {nonce,server_time}`
immediately. Heartbeat does not increment event sequence. Server closes after
75 seconds without a valid client heartbeat; client reconnects after 75 seconds
without an ack (do not treat unrelated traffic as an ack). More than one
heartbeat outstanding is unnecessary. last_seq above current stream high-water
is invalid_sequence; a lower last_seq does not rewind server state.

Client uses focus/input to set idle after five minutes without interaction and
online on activity unless DND/invisible is selected; send only transitions.
SetPresence accepts online/idle/dnd/invisible, not offline. Desired presence is
account-wide (last successful command wins across sessions). Effective presence
is offline when no authenticated socket is connected, otherwise desired; for
other users invisible is always mapped to offline. Self can see invisible.
Connecting/disconnecting one of several sockets must not flap presence. Use a
five-second disconnect grace before broadcasting offline. Heartbeat traffic is
not user activity. Blocked relationships never receive each other's presence.

### Resume

Client keeps the last **applied** event sequence and its stream_id alongside its
retained in-memory model, not just the last received frame. Reconnect using the
same login token and `Identify.resume`. Server validates stream ownership,
retention and `seq <= high_water`. With a usable cursor, replay events with
seq>cursor in increasing order, then `Resumed {cursor:ResumeCursor,replayed:u32}`,
then live events. Resumed.cursor is the replay boundary high-water and replayed
counts actual replayed Event envelopes. Never interleave live events before
Resumed; heartbeat control frames may interleave. Client must not send ordinary
commands until Resumed, then re-establish interests if desired. An exact-current
cursor produces zero replay. Existing interests survive a retained stream.

Ring/stream missing, too old, ahead, or safely filtered replay unavailable:
send `ResyncRequired {reason:ResyncReason}`, immediately followed by a fresh
Ready on the same socket. Do not require another Identify. Rotate stream_id
and reset seq=0 for a new/lost stream; if merely taking a fresh snapshot of an
existing stream, retain its ID and high-water. Reason stream_expired covers
restart/eviction; cursor_ahead is not treated as authentication success proof.
Full Ready resets optional interests to none (base friend/DM presence remains).

Events use `EventEnvelope {seq:EventSeq,event:Event}`. Sequence numbers are
strictly increasing within a stream but may have gaps (expired typing or access
filtering during replay). Ignore duplicates/lower seq. Do **not** invent missed
messages from a numeric gap. Replay covers ten minutes at most, 2048 events or
2 MiB; the global replay budget can shorten that. On removal purge inaccessible
messages/media immediately. Recheck access on replay; never replay old private
bodies to a user removed from their channel. If safe replay is not possible,
take the full replacement snapshot. Replayed typing whose expires_at is past
server_time is omitted. Presence/read upserts are idempotent.

Slow consumer: best effort ResyncRequired(slow_consumer), then close 4010. It is
not safe to queue a Ready behind a full queue. Reconnect may resume only if
retention permits. No busy polling, automatic original-file redownloads, or
unbounded buffering to avoid resync.

### Snapshot/live merge

Bulk replies carry a cursor from the **same token's stream**. Buffer affected
live updates while fetching a page/entity; replace only that fetched subset at
the returned cursor, then reapply buffered events with higher seq in order.
Store bounded per-entity update/delete markers to prevent a late HTTP response
from resurrecting a deletion or overwriting a newer message. Restoring a page
must not remove unfetched newer/older pages. For arbitrary latency, if necessary
events/markers have fallen out of the bounded reducer, abandon the fetch and
refetch/resync; never assume its stale snapshot is current. If stream_id changed
during the request, discard the response. Do not advance the WS applied cursor
just because an HTTP snapshot of **one subset** reports a higher cursor.

## 5. Commands, responses, nonce and retry behavior

`ClientMsg::Command(CommandRequest {nonce,command:Command})`.
One correlated `ServerMsg::Response(CommandResponse {nonce,cursor:ResumeCursor,
replayed:bool,outcome:CommandOutcome})` per accepted command frame, including validation and
authorization failures after identify. `outcome` is ok(CommandResult) or
err(ApiError). Cursor is the stream high-water at result production. For a
durable success, assign/enqueue its own-session events before its response;
unrelated events may interleave. Clients merge result entities using the same
cursor-aware policy as REST; result is also useful for mapping optimistic nonce
to message ID. A result is not permission to overwrite a newer event.

Response is not an event and is not replayed. Event broadcasts include the
initiating session. Sender may see the event before response: deduplicate by
entity ID and retain the pending nonce until response/error/timeout. At most
32 commands in flight/client; bound pending state. No auto-retry of a new nonce
on a lost response: that would duplicate sends.

### Durable retry receipts

For every successful **durable** command (all below except typing and
subscribe_guild and launch logout commands), store `(session_hash,nonce,request_hash,result)` in the **same
transaction** as state mutation. Hash canonical named-MessagePack encoding of
the decoded command using SHA-256 (exact strings/array ordering retained).
Receipts last ten minutes from commit, capped at 256/session. Do not evict
unexpired successes to accept new ones: return limit_exceeded until room exists.
Duplicate nonce + identical command within the window returns the original
result without mutation/new broadcasts, with a **current** response cursor and
replayed=true. Such a result is historical: use it only to resolve operation
identity (e.g. nonce -> message ID), **never** apply its entity state as current.
Use live/REST state for current data. Fresh successes/failures set replayed=false;
different command returns conflict. Receipt lookup precedes rate accounting
for durable duplicates but authentication/active session and current access to
returned entities remain required. If access was revoked, return not_found
without re-executing the mutation or exposing the stored result (replayed=true).
Application failures are not persisted; a caller may retry the same operation
after a rate limit, but changing parameters must use a new nonce. Atomic
idempotent no-op successes also have receipts.

Never retry automatically after ten minutes, with another login token, or when
commit status is unknown beyond that window. Ask the user to inspect current
state/re-send explicitly. SubscribeGuild is replace-state, Typing is ephemeral;
their retry must reflect current UI, not an old offline queue. Do not queue
moderation/friend/profile commands indefinitely while offline. Client clock
skew must not lengthen the ten-minute window; track monotonic elapsed time.

### Command catalog (all fields)

“Common errors” in every row: invalid_request, unauthenticated/session_expired
(normally closes connection), not_found for inaccessible target, forbidden for
known denied action, rate_limited, limit_exceeded, internal, unavailable,
conflict for nonce misuse. Additional errors below are not the only common
ones. No operation returns partial success. Ack means unit `{"type":"ack"}`.

| Command snake_case | Data fields | Success CommandResult | Policy / additional errors |
|---|---|---|---|
| update_profile | ProfilePatch | profile(SelfUser) | Own account; username_taken; pending avatar-purpose image owned by caller only |
| set_presence | `status:PresenceStatus` | presence(Presence) | Own desired status; offline invalid; private self result may say invisible |
| send_friend_request | `username:String` | relationship(Relationship) outgoing | Existing canonical username; self invalid; blocked is generic forbidden; existing incoming returns conflict (explicit accept required); outgoing/friend returns already_exists |
| accept_friend_request | `user_id:UserId` | relationship(Relationship) friend | Must be incoming, not blocked; stale/missing request not_found |
| decline_friend_request | `user_id:UserId` | ack | Remove incoming only; absent is idempotent ack, wrong direction conflict |
| cancel_friend_request | `user_id:UserId` | ack | Remove outgoing only; absent ack, wrong direction conflict |
| remove_friend | `user_id:UserId` | ack | Remove reciprocal friendship; absent ack; pending row conflict |
| block_user | `user_id:UserId` | relationship(Relationship) blocked | Remove both friendship/request rows atomically, create caller-only block; existing block returns existing row |
| unblock_user | `user_id:UserId` | ack | Remove own block only, absent ack; no friendship restoration |
| open_dm | `user_id:UserId` | channel(Channel) direct | New direct requires mutual friendship; existing pair can reopen after friendship ends; either-direction block denies with blocked; reopen not duplicate |
| create_group_dm | `name:String,user_ids:[UserId]` | channel(Channel) group_dm | 2..15 unique other users, all caller's friends, none blocked with any proposed participant; caller auto-added owner |
| add_group_recipient | `channel_id:ChannelId,user_id:UserId` | channel(Channel) | Group owner only; target caller's friend, no block with participants; already participant idempotent |
| remove_group_recipient | `channel_id:ChannelId,user_id:UserId` | channel(Channel) | Group owner only, cannot remove self (use leave/transfer); absent target idempotent |
| leave_group_dm | `channel_id:ChannelId` | ack | Participant; owner with others must transfer first (owner_required); last member deletes group; no rejoin except a new owner invitation |
| transfer_group_ownership | `channel_id:ChannelId,user_id:UserId` | channel(Channel) | Owner to existing participant; self idempotent |
| close_dm | `channel_id:ChannelId` | ack | Direct only, hides sidebar for caller (does not erase history, revoke pair, or hide for peer); already hidden ack |
| create_guild | `name:String,icon_id:?AttachmentId` | guild(GuildDetail) | Caller owner; create default text `general` position 0; icon must be own pending guild_icon upload |
| update_guild | `guild_id:GuildId,patch:GuildPatch` | guild(GuildDetail) | manage_guild; pending guild_icon owned by caller |
| delete_guild | `guild_id:GuildId` | ack | Owner only; owner_required; cascade channels, history, members, invites, bans |
| join_guild | `invite_code:String` | guild(GuildDetail) | Banned denies; existing member returns current detail without consuming use (but code must still resolve valid invite); invalid_invite/invite_expired/invite_exhausted/banned |
| leave_guild | `guild_id:GuildId` | ack | Member; owner must transfer first, even sole owner must use delete; owner_required |
| transfer_guild_ownership | `guild_id:GuildId,user_id:UserId` | guild(GuildDetail) | Owner to existing member; old owner becomes admin; self idempotent; owner_required |
| set_member_role | `guild_id:GuildId,user_id:UserId,role:MemberRole` | member(Member) | Owner only; admin/member only; cannot alter owner; owner_required/hierarchy_violation |
| kick_member | `guild_id:GuildId,user_id:UserId` | ack | kick_members and lower role; self use leave; hierarchy_violation; absent member not_found |
| ban_member | `guild_id:GuildId,user_id:UserId,reason:?String` | ack | ban_members, lower role; may ban a nonmember known user; hierarchy_violation; removes membership, existing ban no-op (reason unchanged) |
| unban_member | `guild_id:GuildId,user_id:UserId` | ack | ban_members; absent ban no-op; does not restore membership |
| create_invite | `guild_id:GuildId,ttl_seconds:u32,max_uses:u32` | invite(Invite) | create_invites; max 100 active invites/guild, 10/creator/guild |
| revoke_invite | `guild_id:GuildId,code:String` | ack | Creator or manage_guild; no-op if already absent but same guild authority required |
| create_channel | `guild_id:GuildId,kind:ChannelKind,name:String,parent_id:?ChannelId,topic:?String` | channel(Channel) | manage_channels in the guild and in the parent; kind category/text/voice; any guild channel may be the parent; append to sibling list |
| set_voice_permission | `channel_id,user_id,connect:bool` | ack | Voice only; Manage Channels and strict target role hierarchy; section 10 |
| update_channel | `channel_id:ChannelId,patch:ChannelPatch` | channel(Channel) | Guild manage_channels; group owner can change name only; direct rename/topic invalid; category topic cannot set |
| reorder_channels | `guild_id:GuildId,placements:[ChannelPlacement]` | ack | manage_channels; complete permutation of **all** guild channels the actor can see; only placements differing from the actor's current view are applied; each sibling set with a change has unique dense positions; parents are any channel of the guild; a move that would put a channel inside itself (through any depth, hidden channels included) is invalid_request; atomic |
| delete_channel | `channel_id:ChannelId` | ack | Guild manage_channels only; children of the deleted channel (any kind) move up one level, to the end of its parent's siblings (root when it had none), preserving order; deleting text deletes its history; no group/DM deletion through this command |
| send_message | MessageDraft | message(Message) | send_messages (+attach_files if needed), participant for DM; own pending channel-bound files; reply must be live same-channel; invalid_attachment/blocked |
| edit_message | MessageEdit | message(Message) | Author only, still has channel access/send rights; attachments/reply immutable; no time limit; same content/mentions no-op, else edited_at now |
| delete_message | `channel_id:ChannelId,message_id:MessageId` | ack | Author or guild manage_messages; tombstone same-channel repeated delete ack; absent ID not_found |
| add_reaction | `channel_id:ChannelId,message_id:MessageId,emoji:String` | reaction(ReactionUpdate) | add_reactions, live message; already reacted idempotent added=true with current count/me |
| remove_reaction | same fields as add_reaction | reaction(ReactionUpdate) | Remove own only; already absent idempotent added=false with current count/me |
| typing | `channel_id:ChannelId` | ack | send rights; expires server_time+8000ms, coalesce 1/4 seconds/channel, ephemeral; no self echo required |
| mark_read | `channel_id:ChannelId,message_id:MessageId` | read_state(ReadState) | Message or tombstone in same channel, existing access; monotonically max old/new; no future/arbitrary IDs |
| subscribe_guild | `guild_id:?GuildId,presence_user_ids:[UserId]` | subscription `{presences:[Presence]}` | Replaces this connection's selected guild/interests; selected membership required; null clears; presence users must be guild members; response is initial effective presence snapshot |

Friend transitions emit own and reciprocal RelationshipUpsert/Removed as
appropriate; blocking is private (the other account sees a removal, never a
“blocked by” event). Block forbids direct sends/reopen/typing and new shared
group invitations in either direction, hides presence, and removes friendship.
Existing common guild/group messages, history and reactions remain visible;
blocking is not a retroactive shared-room content filter. Direct history is
still readable by its pair members but new direct messages are forbidden.
Unblocking does not restore friends or change existing messages.

New group members get all previous group history; removal/leave immediately
revokes group history/files. All guild members get existing guild history;
joining initializes last_read to current latest message with zero counts.
New group recipient likewise starts caught up. On direct creation initialize
null/zero; on reopening preserve existing read state. Close hides own read state
from Ready but retains it; next incoming direct message reopens automatically.
If re-opening would exceed the peer's sidebar quota, send fails limit_exceeded.
Never silently drop a new message or evict another DM to fit Ready.

Successful durable mutations broadcast the events below, only if state really
changed. Deletes and reaction no-ops do not double-count. Response receipt
replays do not rebroadcast. Join/create can return complete GuildDetail without
preloading members/history. Creating/reopening a DM must emit needed UserUpdated
profiles before ChannelUpsert so sidebar names can render immediately.

## 6. Event catalog and routing

All event names are inside `ServerMsg::Event` -> EventEnvelope. Every payload
field is listed here or is the shared entity of that exact type.

| Event | Data | Recipients / behavior |
|---|---|---|
| self_updated | SelfUser | All own streams for profile/desired-presence changes |
| user_updated | UserSummary | Streams whose profile projection includes that user (relationships, DM peers, selected-guild member interests); update cached author/profile labels |
| relationship_upsert | Relationship | Affected account's streams, kind personalized |
| relationship_removed | `user_id:UserId` | Affected account's streams; delete relationship, not shared history |
| presence_updated | Presence | Authorized base friends/visible DM peers and selected custom interests; never invisible to others |
| guild_upsert | GuildSummary | All current guild members; personalized role/permissions; includes name/icon/count/ownership changes |
| guild_joined | `detail:GuildDetail,read_states:[ReadState]` | Newly joined/creating account's streams; atomically add sidebar and read state |
| guild_removed | `guild_id:GuildId,reason:RemovalReason` | Leaving/kicked/banned user, or all on delete; atomically purge all guild children/history/interests/media |
| member_upsert | Member | Selected-guild subscriber streams for join/role/profile changes; own streams too for own role; all get corresponding guild_upsert when count or their role changes |
| member_removed | `guild_id:GuildId,user_id:UserId,reason:RemovalReason` | Selected-guild subscribers still in guild; departing account gets guild_removed instead |
| invite_upsert | Invite | Creator and current manage_guild streams; create/use-count changes; invite codes never broadcast to ordinary peers |
| invite_removed | `guild_id:GuildId,invite_id:Id` | Same privileged invite audience; revoke/expiry/exhaustion; expiry cleanup may be delayed up to housekeeping interval, join always checks expiry immediately |
| ban_upsert | Ban | Current ban_members streams on new ban; never unrelated members/target account |
| ban_removed | `guild_id:GuildId,user_id:UserId` | Current ban_members streams on unban |
| channel_upsert | Channel | All authorized guild members or visible DM/group participants; full replacement metadata; needed user profiles precede new DM |
| channel_removed | `channel_id:ChannelId,reason:RemovalReason` | Channel deletion to guild members; own closed DM=left, removed group member=kicked, leaving member=left; purge sidebar/history/typing/read state/media |
| channels_reordered | `guild_id:GuildId,placements:[ChannelPlacement]` | All guild members, complete post-change placement list; deleting a channel emits removal then reordered |
| message_created | `message:Message,author:UserSummary` | All current guild/channel or DM/group member streams, including author; no history subscription needed |
| message_updated | Message | Same access audience; full post-edit state including personalized reactions |
| message_deleted | `channel_id:ChannelId,message_id:MessageId` | Same access audience; remove body or show reply placeholder, remove cached reactions/attachments |
| reaction_updated | ReactionUpdate | Same access audience, absolute post-change summary; count=0 removes emoji from list |
| typing | `channel_id:ChannelId,user_id:UserId,expires_at:UnixMillis` | Same access audience except typing author; respect block rule for direct; no storage/history; expire locally without polling |
| read_state_updated | ReadState | Only affected reader's streams; absolute counts, never infer counts solely from message events |

On create/edit/delete, emit message/reaction event first and any changed
read_state_updated afterward in the same mutation batch. Unread counts exclude
self-authored/deleted messages and count messages with ID above last_read (null
=0). Mention count is the unread subset mentioning this user. Editing mention
IDs can change mention counts, not unread total; deleting can change both and
latest_message_id. MarkRead emits own state across all devices. Counts are exact
u32 under service quotas; reject impossible overflow as limit_exceeded.

Message attachments/reply are immutable after creation; edits replace content/
mentions only. On author deletion from channel membership, historical authors
remain identifiable by embedded/fetched public UserSummary as permitted by
shared message visibility; bulk profile authorization must permit authors of
messages the caller currently can read. No account deletion is exposed in v1.

Mention composition is client-side: selecting `@username` inserts the canonical
text token `<@DECIMAL_USER_ID>` into content and adds that ID to mention_ids.
The renderer resolves that token to the current display name only when its ID
is in mention_ids; arbitrary `@username` text is plain text. The server preserves validated explicit mention_ids and expands authorized
broadcast/role tokens as specified in section 12. Explicit unrendered user
mentions still count as mentions and remain subject to the explicit limit. Returned mention_ids are sorted;
attachment order is draft order; reaction summaries sort by emoji UTF-8 bytes.

Selecting a guild does not preload all members: REST pages populate the member
panel; subscribed member events keep cached/visible entries current. New
presence interests return their snapshot in subscription result. Do not
subscribe all 1,000 members just to paint an offscreen list. Base presence is
maintained automatically when friends/visible DMs change; emit an effective
presence snapshot for newly visible peers, offline when interest/access ends.
On removal from a guild clear optional interests in that stream. Interest
changes use the current stream; replay can include events from the former
interest set, which the reducer should ignore if no longer relevant.

## 7. Error codes, transport errors and voice signaling

`ApiError {code:ErrorCode,message:String,field:?String,retry_after_ms:?u32}`.
message is safe diagnostic text, not a string for branching/localization; field
is a snake_case/dotted request field path when useful, otherwise null. Do not
include SQL, credentials, filesystem paths or content in errors. retry_after_ms
is required for rate_limited, optional for unavailable, null otherwise.
REST 429 also emits `Retry-After` rounded up seconds. These are the **complete**
v1 ErrorCode values and default HTTP mappings:

| Code | HTTP | Meaning |
|---|---|---|
| invalid_request | 400 | Invalid fields/types/enum combination/cursor/range |
| invalid_credentials | 401 | Username/password pair invalid, no enumeration |
| unauthenticated | 401 | Missing/unknown/revoked token |
| session_expired | 401 | Known expired token |
| forbidden | 403 | Known accessible target, operation not authorized |
| not_found | 404 | Missing/deleted or intentionally hidden inaccessible entity |
| conflict | 409 | Incompatible state, attached pending-delete, nonce reused with different command |
| username_taken | 409 | Canonical username already in use |
| already_exists | 409 | Friend/request already exists |
| blocked | 403 | Direct/group operation disallowed by a block (do not disclose blocker) |
| invalid_invite | 404 | Unknown/revoked invite |
| invite_expired | 410 | Resolved invite expired |
| invite_exhausted | 409 | No uses left |
| banned | 403 | Cannot join guild while banned |
| owner_required | 403 | Ownership-only action or transfer needed before owner leaves |
| hierarchy_violation | 403 | Cannot act on equal/higher role or owner |
| rate_limited | 429 | Token bucket exhausted; retry_after_ms set |
| limit_exceeded | 409 | Entity/storage/in-flight/receipt quota would be exceeded |
| payload_too_large | 413 | HTTP/WS payload or file over size bound |
| unsupported_media_type | 415 | Wrong encoding/MIME or image decoder policy failure |
| invalid_attachment | 400 | File missing/expired, wrong purpose/channel/owner, already consumed |
| version_mismatch | 426 | Unsupported protocol/voice schema version |
| invalid_sequence | 400 | Heartbeat sequence ahead/invalid stream control |
| resync_required | 409 | State cannot be safely incrementally recovered; normally use typed resync_required server message |
| unsupported_feature | 501 | Reserved/unimplemented feature |
| internal | 500 | Unexpected server failure; report generic diagnostic |
| feature_disabled | 404 | Optional Google authentication is not configured |
| invalid_signup_ticket | 401 | Google signup ticket is invalid, expired, superseded or consumed |
| identity_already_linked | 409 | Google identity/account already has a conflicting link; no implicit merge |
| last_login_method | 409 | Unlink would remove the account's only login method |
| unavailable | 503 | Startup/shutdown/temporary storage or clock-allocation failure |

ServerMsg::Error(ApiError) is for uncorrelated connection/protocol/auth errors,
not ordinary command failures. If a command envelope and nonce decode, respond
with its nonce for invalid command data where possible. Unknown enum/unparseable
frame cannot be correlated safely: error then close. A known unauthorized
mutation is a normal correlated failure and does not close the socket.

| WS close | Meaning / client action |
|---|---|
| 1000 | Normal intentional shutdown/logout; no reconnect on local logout |
| 1001 | Service going away; backoff reconnect with resume |
| 1002 | Malformed protocol/frame or repeated Identify; fix client, no busy retry |
| 1003 | Text/nonbinary unsupported |
| 1008 | Invalid pre-auth traffic/Origin/abuse; stop or respect retry guidance |
| 1009 | >512 KiB message, size/depth/collection bounds; do not retry same frame |
| 1011 | Internal server failure; bounded backoff reconnect |
| 4001 | Invalid/expired/revoked auth, including REST logout; clear token/show login |
| 4002 | Protocol version mismatch; client upgrade required |
| 4003 | Channel access changed; queued packets are dropped. Reconnect with resume |
| 4008 | Identify or heartbeat timeout; backoff reconnect |
| 4009 | This token connected elsewhere; stop automatic reconnect to avoid socket duel |
| 4010 | Slow consumer/resync needed; drop backlog, reconnect under bounded policy |

`ClientMsg::Voice(VoicePayload)` now routes to the embedded SFU. Voice failures
are correlated `ServerMsg::Voice` / `VoiceEvent::Error` using payload.nonce,
not CommandResponse/receipts. Unsupported version/kind returns version_mismatch;
invalid signaling does not allocate media. Section 10 defines the enabled schema.
The original four-field outer envelope remains unchanged; old reserved version-0
bodies still deserialize, but are not valid Phase 2 requests.

## 8. Example flows (JSON notation, actual WS uses MessagePack)

Fresh connect: `Hello` -> `Identify {version:1,token,resume:null}` ->
`Ready {cursor:{stream_id:S,seq:0},...}` -> lazy REST history and optional
SubscribeGuild. First durable send:

```json
{"type":"command","data":{"nonce":123,"command":{"type":"send_message","data":{"channel_id":456,"content":"hello","reply_to":null,"attachment_ids":[],"mention_ids":[]}}}}
```

Server commits message + receipt, emits Event(message_created), emits affected
read_state_updated events, then Response nonce=123 outcome ok(message). Client
maps pending send to returned message ID; event and response never create two
rows. If response is lost, reconnect/resume and retry **the same command/nonce**
within ten minutes; stored receipt returns original ID without a new message.

Attachment: POST multipart purpose=message/channel_id/file -> UploadResponse ->
send_message with attachment_ids -> message metadata + preview endpoint -> only
explicit “save/open original” fetches original. Failure before attach leaves a
pending upload; DELETE or one-hour GC removes it.

Resume: applied cursor S:10 -> connection drops -> Identify resume S:10 -> events
11,12 (or ordered gaps for filtered ephemeral data) -> Resumed cursor S:12 ->
live event 13. Expired ring: ResyncRequired(cursor_too_old) -> Ready replacement;
clear stale projections and refetch selected view, not all message histories.

Friend invite: send_friend_request creates outgoing/incoming rows -> recipient
accepts -> reciprocal friend events + initial effective presence snapshots.
Guild invite: creator receives code -> user join_guild(code) -> transaction
checks ban/expiry/uses/quota, inserts member and increments use -> GuildJoined
and caught-up read states to joining account, GuildUpsert count to all members,
MemberUpsert to selected-guild subscriber streams.

Read: only when a message is actually visible/acknowledged, debounce MarkRead
to the greatest visible ID (e.g. 500ms trailing, flush on channel switch). Do not
mark the newest fetched but offscreen message read. Server returns and emits
absolute ReadState to all own devices. No per-second unread polling.

## 9. Required interoperability tests for implementation engineers

The foundation tests exercise every command/event/result shape, control frames,
REST models, Unicode reaction payloads, large numeric IDs, unknown permissions,
patch defaults and a fixed named-map MessagePack fixture. They do not enforce
business authorization or network behavior. Add shared fixture/contract tests
before changing the schema. Server/client integration must additionally cover:

* Bad version/token/Origin, timed-out Identify/heartbeat, replacement socket.
* Fresh Ready vs concurrent mutation and REST snapshot/live merge races.
* Empty replay, partial replay, lost ring/restart, expired typing, access
  revocation during replay, bounded slow-consumer recovery.
* Duplicate nonce before/after crash, changed payload conflict, expiry ambiguity.
* Every role/hierarchy boundary; blocked DM vs shared group; invite use race.
* Cross-channel replies, foreign/reused uploads, image bombs and path traversal.
* Read counts on edits/deletes/own sends, monotonic multi-device read marker.
* Reaction personalization/idempotency and MessagePack frames from both peers.
* Maximal Ready fits frame budget; history/UI/cache queues remain bounded.
# Operational additions: resumable uploads, recovery, releases (B2/B4/B5)

These additions do not change protocol version 1 or existing WS envelopes.
The following upload defaults supersede the original 25-MiB/global quota defaults
elsewhere in this document. `--max-upload-mb 0` (the default) means no per-file
limit on the **resumable API**. Configured sizes must fit signed SQLite integers;
available disk is the practical limit. `--min-free-disk-gb` defaults to 10 GiB.
`--user-quota-mb 0` and `--max-total-upload-mb 0` disable optional logical quotas.
The existing multipart `/api/v1/uploads` is a compatibility path, still capped at
25 MiB (or the smaller configured maximum), with its existing pending/daily/user
anti-abuse bounds. Large/new clients must use the resumable path below, which has
no daily bandwidth ceiling. Both paths preserve existing media validation and ACLs.

## Resumable upload client contract

All routes require the normal session Bearer header. IDs are opaque 32-character
hex strings, **not Snowflakes**. Requests/responses use the additive DTOs in
`crates/proto/src/rest.rs`; existing `UploadResponse`/`Attachment` remain unchanged.

| Method/path | Body/result |
|---|---|
| POST `/api/v1/uploads/resumable` | JSON `CreateUploadRequest` -> 201 `ResumableUpload` |
| GET `/api/v1/uploads/resumable/{upload_id}` | 200 `ResumableUpload` with durable offset |
| PUT `/api/v1/uploads/resumable/{upload_id}` | Raw bytes; `Content-Length` and `Upload-Offset` required -> 200 `ResumableUpload` |
| POST `/api/v1/uploads/resumable/{upload_id}/finish` | Empty body -> 200 `UploadResponse`; idempotent while record exists |
| DELETE `/api/v1/uploads/resumable/{upload_id}` | 204; only unfinished uploads; normal delete-upload route handles published pending attachments |
| GET `/api/v1/guilds/{guild_id}/storage` | Owner only -> `StorageUsage` for the physical instance (not a per-guild quota) |

Create fields: `purpose`, `channel_id` (required only for message files), sanitized
display `filename`, claimed `content_type`, `byte_size`, and lowercase hexadecimal
`sha256` of the **whole file**. MP4 (supported ISO-BMFF `ftyp` brands) and WebM (EBML `DocType=webm`) are
identified from a bounded prefix and returned as `video/mp4` / `video/webm`,
regardless of the claimed MIME. Unknown media remains octet-stream with
attachment disposition/nosniff; this change does not bless executable HTML/SVG or
unvalidated inline video. Avatar/icons must pass the existing bounded image decoder.
The create response contains `upload_id`, `offset`, `byte_size`, `chunk_size`
(16 MiB), `expires_at` in Unix ms, and nullable `completed: UploadResponse`.

Persist the upload ID locally with its file identity/hash. Send sequential 8–16 MiB
chunks (a smaller final chunk is allowed; the server accepts 1 byte–16 MiB).
On disconnect/restart fetch status and resume at the **server's** durable offset.
Offset mismatch or writing a completed upload gives `conflict`; no bytes are
accepted for that request. Offsets commit only after disk sync. A rejected or
interrupted chunk may leave trailing bytes, which the next valid PUT truncates.
Finish checks full length and streamed SHA-256 (`invalid_attachment` on mismatch),
sniffs media, and rechecks the caller's session/channel permissions before commit.
If completion's response is lost, retry finish or read `completed` from status.
Upload publication does not attach it to a message: use the existing send command.
Published unattached files retain the existing one-hour expiry. The resumable
record expires after 24 hours since the last successful chunk (creation for empty
files); completing does not extend that deadline. Finish retry requires unexpired
record **and** attachment. Abandoned partials are collected at startup/minutely.
Lost channel membership prevents resuming/finishing; abandoned data is still GC'd.

Admission reserves the remaining declared size plus thumbnail headroom against
filesystem free bytes and other accepted uploads. Recheck on each chunk/finish.
The error is `unavailable`, HTTP 503, message **"Server storage is full."**
The floor does not stop unrelated applications/SQLite/backup processes writing:
use a dedicated SSD and monitor free space. At most ten pending files per user;
upload/file concurrency is bounded. Disk operations are serialized (not unbounded
buffers). StorageUsage fields, in bytes: `total_bytes`, `available_bytes`,
`committed_bytes` (attachments plus thumbnails), `partial_bytes` (durable received),
`reserved_bytes` (remaining partials plus thumbnail headroom), `min_free_bytes`.
Filesystem usage also includes SQLite, release installers, and non-upload files.
An owner of any guild can read this instance-level operational summary; admins
and ordinary members cannot. No file names, private paths or other-user counts.

Original downloads already support single HTTP byte ranges: `bytes=a-b`, `bytes=a-`,
`bytes=-suffix`; 206 with Content-Range/Length, 416 with `bytes */size` when
unsatisfiable. Multi-range requests are rejected. Authorization precedes even 304.
Thumbnails do not support ranges. `If-Range` mismatch returns full 200; immutable
ETags can be used to safely resume. Downloads are streamed and memory-bounded.

## Account recovery and local admin

`sticktalk-server --data-dir <dir> admin list-users|reset-password <username>|disable-user <username>|enable-user <username>`.
Local filesystem access is the admin authority. Reset prints a random 256-bit
base64url code once; only SHA-256 is stored; replacement invalidates earlier codes.
Codes expire in 30 minutes. Reset issuance and disabling delete all login sessions;
existing WS connections close within the existing 60-second maintenance tick,
and authenticated requests/mutations reject removed sessions immediately.
Disabled accounts cannot password/Google login or redeem reset codes; enable does
not revive old sessions. No account deletion or username reassignment occurs.

POST `/api/v1/auth/reset-password` takes JSON `ResetPasswordRequest { username,
code, new_password }`; success 204, invalid/expired/used code `invalid_credentials`.
It consumes the code atomically with the password change and session revocation;
password policy is 1–128 UTF-8 bytes, same as normal registration. Rate/hash slots
are bounded. No automatic login; use the regular login afterwards.

## Signed Windows update contract

GET `/api/update/latest` (alias `/api/v1/update/latest`) -> `SignedUpdateManifest`:
`{ "manifest": "<exact UTF-8 JSON string>", "signature": "<standard base64>" }`.
404 if no release; `feature_disabled` if no server verification key. No login
required; no-store. **Verify Ed25519 over the exact manifest string bytes before
parsing/trusting its fields.** Never verify a reserialized JSON object. The trusted
32-byte public key is embedded in the client, not accepted from the response.
Do not downgrade an installed version on an older valid manifest.

The signed `UpdateManifest` contains `version`, `target` (currently only
`x86_64-pc-windows-msvc`), `url`, `byte_size`, `sha256` (lowercase hex). `url` is
the configured HTTPS origin + `/api/update/artifact?sha256=<hash>`. Verify installer
size/hash before execution. Signatures authenticate the manifest and, via its
hash, the installer. Server response keys/headers alone are not a trust anchor.

Owner release tooling: `tools/release/release.ps1`, using `sticktalk-release` signing
helper (the current desktop release script uses the compatible standalone
`sticktalk-package` utility; see `deploy/RELEASING.md`). Default **private key outside repo**:
`%USERPROFILE%/.sticktalk-secrets/update-signing.pk8` (Ed25519 PKCS#8).
The server only gets base64 `STICKTALK_UPDATE_PUBLIC_KEY`. Publishing uses a
separate random `STICKTALK_RELEASE_TOKEN` (32+ characters), never a user token.
POST `/api/update/publish` streams the installer bytes; header
`X-StickTalk-Manifest` is standard base64 of the outer signed-envelope JSON.
Requires publishing Bearer, validates signature/origin/target/size/hash, 201 after
publication. A version cannot be overwritten with another hash. Artifact URL is
content-addressed/immutable and previous published artifacts remain available.
Installers are limited to 1 GiB for this administrative path (not user uploads).
The signing key never goes to the Pi or into the repository. No signing key is
created automatically. Windows Authenticode is separate and not supplied here.


## Additive v1 around-history cursors

`GET /api/v1/channels/{channel_id}/messages` also accepts `around=<message_id>`
or `around_timestamp=<UnixMillis>`. These are optional `PageQuery` fields;
older requests remain unchanged. Exactly one of `before`, `after`, `around`,
and `around_timestamp` may be present. `limit` remains 1–50 (default 50).
Around cursors on other collection endpoints are invalid requests. ID zero,
out-of-range IDs, and timestamps outside the signed-63-bit Snowflake time
range (beginning at `SNOWFLAKE_EPOCH_MS`) are rejected.

An ID pivot is inclusive on the older half; it need not still exist (a deleted
reply target is a valid gap). A timestamp resolves to the start of that
millisecond in Snowflake ID order, not a table scan or OFFSET. Results contain
up to N nearest surrounding nondeleted messages, approximately half on either
side; at the edges the other side fills the page. Even limits include the
pivot in the older half; a one-item ID page prefers the pivot/nearest older
message, or the nearest newer message if none is older. Results remain newest
first. Normal history ACL checks and snapshot cursor semantics apply.

For around pages **both** continuation cursors may be populated:
`next_before` is the oldest returned ID if older messages remain, `next_after`
is the newest returned ID if newer messages remain. `has_more` is their OR.
Continue each direction using the existing exclusive before/after queries.
An after continuation is oldest first, as before. Empty channels return an
empty page with neither cursor. The server performs bounded indexed keyset
queries, not a walk through intervening pages. Clients should replace a jump
region, discard stale navigation responses, and preserve measured row slots
when merely reloading an evicted visible region.

Local integration tests can explicitly configure loopback Google issuer/JWKS
and GIF HTTP upstreams; see `tools/e2e/README.md`. Public metadata never returns
these operator-only mock endpoints or the GIF server key.

## 10. Phase 2 voice (embedded server SFU)

`sticktalk-server` embeds `sticktalk-sfu` on its existing Tokio runtime, with one
multiplexed UDP socket and bounded signaling queues. Voice channels use the
existing CreateChannel/UpdateChannel/DeleteChannel commands, including category
placement and Manage Channels authorization. Voice channels cannot contain chat
messages/history/attachments/read states. Text and DM behavior is unchanged.

Voice authorization uses the role evaluator in section 11, including category
and channel overwrites, for View Channel/Connect and every publish/moderation
operation. Canonical Connect is bit **19**, Manage Roles bit **14**; see the
negotiated compatibility projection below. The old boolean command
`set_voice_permission {channel_id,user_id,connect:bool}` remains accepted (Ack):
it edits only Connect in the member overwrite, requires Manage Roles and a
strictly lower target, and never grants other rights. False sets deny; true
clears Connect allow/deny, preserving unrelated overwrite bits. Legacy persisted
Connect denials are migrated into canonical member overwrites; no independent
voice permission store is consulted.

Ready additively includes `voice_states:[GuildVoiceState]` (default [] when absent).
`GuildVoiceState {guild_id,channel_id,participant:VoiceParticipant}` is keyed by
participant.session. Channel-authorized guild members, not just selected-guild
subscribers or call participants, receive `VoiceEvent::GuildState` full replacements and
`GuildLeft {guild_id,session}` removals. Streaming/video are derived from the
participant's Screen/Camera tracks; muted/deafened are explicit. These carry no
SDP, input or control grant. Late guild joins and WS Resumed receive a complete
`GuildSnapshot {states}`; replace the entire previous voice projection. On guild
removal discard all its states. These events are ephemeral, not EventSeq/replay.
Ready is a full replacement including voice; resume does not restore media.

Connect denial, kick/ban/leave, channel/guild deletion, logout, socket replacement,
disconnect, slow consumer, login expiry and SFU handshake/session timeout remove
media access and broadcast removal. The synchronous ACL projection is revoked
before returning the mutation response; queued signals cannot resurrect access.
The SFU checks that projection on signaling, source/recipient media and control
input. Login expiry is also checked against wall clock in the fast ACL. Externally
issued local-admin session revocation is discovered by the existing <=60-second
maintenance tick; mutations/signaling reject removed sessions immediately.

### Envelope and negotiation

The existing outer WS variants and four fields are unchanged:
`VoicePayload {nonce:u64,version:u16,kind:String,payload:[u8]}`. Old reserved
bodies still decode, including version 0; no previously valid wire field was
removed or retyped. For Phase 2, `version=1`, client `kind="signal"` contains a
**named-map MessagePack VoiceSignal**, server `kind="event"` contains a
**named-map MessagePack VoiceEvent**. Both bodies use the usual
`{type:snake_case,data:...}` tagged representation. There is no opaque
application-defined payload in the implemented schema. Encode with
`rmp_serde::to_vec_named`; JSON below is descriptive, not a second transport.
Clients must explicitly opt in; old peers do not understand new enum values.
The runtime-free proto crate defines the types; SFU `encode_signal` and
`encode_event` are the transport helpers.

`nonce` correlates requests and responses; unsolicited events use 0. Voice is
**ephemeral, not durable/replayed**, not part of EventSeq or command receipts.
Reconnect creates a new voice session and fresh ICE/DTLS connection/grants;
never replay an old voice grant. A trusted host binds
`VoiceSession {user_id:UserId,connection_id:Id}` to the authenticated WS
connection and fresh Join lifetime (including same-socket leave/rejoin); a client cannot choose its own identity. Other-session references
are checked against the live same-channel membership. The host must authorize
access to an actual voice channel and disconnect the SFU session on WS loss,
logout, kick, channel deletion, or permission revocation. ACLs are checked by the
SFU for signaling, incoming media, outgoing recipients, and control input.

Limits: SDP 64 KiB, inner signaling body 72 KiB, SDP 33 m-lines (32 media plus
SCTP), four published tracks (one each kind), 32 subscriptions/receiver, three
simulcast RIDs/track, MID/RID 1..16 ASCII alphanumeric/underscore/hyphen, 64
non-join signals/session/second. ICE candidate strings <=1024 bytes and UDP
only. Default 64 sessions total and 16/channel, configurable up to 256 and 32.
Incomplete handshakes expire after 30 seconds. The host must retain the outer
WS frame/rate limits too. Requests rejected for schema/version/access/resource
errors do not allocate an authenticated media session.

### Client VoiceSignal catalog

| type | data |
|---|---|
| join | `channel_id,capabilities:{video:[VideoCodec],hardware_decode:[VideoCodec]}` |
| leave | no data |
| offer | `sdp:String,tracks:[PublishedTrack]` (complete replacement set) |
| ice | `candidate:String` (trickle candidate, standard SDP candidate syntax) |
| state | `muted:bool,deafened:bool` (deafen also suppresses outgoing audio) |
| subscribe | `publisher:VoiceSession,source_mid:String,receiver_mid:String,rid:?String` |
| unsubscribe | `receiver_mid:String` |
| request_keyframe | `receiver_mid:String,fir:bool` (false=PLI, true=FIR) |
| request_control | `sharer:VoiceSession` |
| decide_control | `requester:VoiceSession,allow:bool` (only the sharer) |
| revoke_control | no data (sharer or current controller) |
| stats | no data |

`PublishedTrack {mid,kind,codec:?VideoCodec,layers:[String],surface:?ShareSurface}`.
Kinds: `audio` (Opus voice), `camera`, `screen`, `system_audio` (Opus system/app
sound). Camera and screen have independent m-lines/subscriptions. Codec must be
null for audio, non-null for video; `h264`, `hevc`, `av1` are the video codecs.
`surface` is `monitor` or `window`, required only for screen. This is a server
validated share declaration, not proof of what an OS captures. The native client
must constrain input to the actual shared surface. Layers are ordered low to
high; empty means non-simulcast. A receiver explicitly selects exactly one RID
provided by the publisher, or null for a non-simulcast source. Re-subscribe to
change layer; the SFU requests a keyframe and uses a fresh outgoing SSRC/RTX
pair. No transcoding, spatial/temporal SVC splitting or codec conversion.

The client is always SDP offerer. Native peers and browser RTCPeerConnection
use standard bundled ICE/DTLS/SRTP/SCTP with rtcp-mux. On Join, publish an Offer;
the SFU returns Answer. Before Subscribe, create a recvonly transceiver, offer
it and apply Answer, then reference its MID. A receiver MID cannot also be a
publication, and holds at most one source. Peers must match actual negotiated
codec profiles/formats; H.264 High is offered for the native hardware path,
with baseline/constrained-baseline interoperability configurations available
for browsers. H.264 is mandatory in capabilities. HEVC/AV1 are allowed only if
**all participants** report both support and hardware decode. If an incompatible
participant joins, forwarding of the advanced codec stops immediately and the
publisher receives CodecRequired(h264); it must reconfigure and re-offer.
There is no transparent transcoder. Actual browser/native endpoint integration
still needs to validate this handshake against their real implementations.

Opus is forwarded as compressed RTP without decoding. Audio-level RTP
extensions produce edge-triggered Speaking events; 300 ms of silence expires
the indicator. Mute/deafen are also enforced in forwarding, not merely UI hints.
TWCC feedback/estimates, REMB, PLI/FIR, NACK and negotiated RTX are handled by
str0m per hop. Receiver PLI/FIR is propagated to the selected publisher/RID.
RTX is terminated and regenerated per receiver; compressed payloads are shared.
Sequence numbers are remapped with source ordering preserved and start at ROC 0.

### Server VoiceEvent catalog

| type | data |
|---|---|
| joined | `channel_id,participants:[VoiceParticipant],max_rtp_bytes:u16` |
| participant | `VoiceParticipant {session,muted,deafened,tracks}` full replacement |
| left | `session:VoiceSession` (cleanup, including own session) |
| answer | `sdp:String` |
| speaking | `session,speaking:bool,level_dbov:i8` (negative dBov) |
| bitrate | `mid:String,bps:u64` upstream encoder ceiling; applies alongside RTCP |
| codec_required | `mid:String,codec:VideoCodec` |
| control_requested | `requester:VoiceSession,timeout_ms:u32` (30000) |
| control_denied | `sharer:VoiceSession` |
| control_granted | `sharer,controller:VoiceSession,grant:u64` |
| control_revoked | `sharer:VoiceSession,grant:u64` |
| stats | `VoiceStats` below |
| ack | no data |
| error | `code:ErrorCode,message:String` (safe diagnostics, no SDP/input contents) |

The additional guild events above reach current guild members. Existing
Participant, Left, Speaking and grant/revoke announcements go only to authorized
current channel participants. Answers/errors/stats/bitrate/prompt/denial go to
the relevant session, not the whole guild. Clients must render the persistent
control banner from grant state, and clear stale state on disconnect.

### Tailscale packet/bandwidth policy and statistics

Bind the SFU's configurable UDP range and advertised IP to the Pi's Tailscale
address. One socket multiplexes sessions, using the first available port in the
range. ICE host candidates advertise that address/port. No STUN/TURN server or
external port-forwarding is required for the Tailscale host path.
The server flags/env are `--voice-bind-ip` / `STICKTALK_VOICE_BIND_IP`,
`--voice-advertised-ip` / `STICKTALK_VOICE_ADVERTISED_IP`,
`--voice-udp-port-start` / `STICKTALK_VOICE_UDP_PORT_START` (50000),
`--voice-udp-port-end` / `STICKTALK_VOICE_UDP_PORT_END` (50100),
`--voice-egress-bps` / `STICKTALK_VOICE_EGRESS_BPS` (32000000), and
`--voice-max-datagram-bytes` / `STICKTALK_VOICE_MAX_DATAGRAM_BYTES` (1232).
Development defaults bind UDP to 127.0.0.1 and advertise the HTTP loopback IP,
without enumerating network interfaces. For an explicitly non-loopback deployment
bind, the advertised IP defaults to an assigned Tailscale IPv4 address in 100.64/10,
else the HTTP bind IP. An explicit advertised IP overrides detection. Tests use
injected fake addresses for detection and never bind to wildcard/LAN/Tailscale IPs. Wildcard/multicast advertised addresses are rejected;
IPv6 requires matching explicit bind/advertised addresses. One address is
advertised per server instance. No interface/Tailscale subprocess is polled.

All forwarded SRTP **UDP payloads**, including RTX, are bounded by the configured
1232-byte default. This is the IPv6 allowance at a 1280-byte Tailscale MTU;
IPv4 permits 1252. Admission checks actual incoming UDP bytes, not the old
1060-byte compressed-RTP limit. Standard ~1200-byte browser packets now pass.
str0m's MTU and the final encrypted egress guard use the same configured limit
(1200–1472). Rewritten extensions/authentication/RTX must also fit; packets that
exceed that ceiling are dropped/counted, never IP-fragmented or repacketized.
Native senders should leave headroom (<=1180 compressed bytes for the tested
negotiation). Real browser handshake/playback testing remains separate from the
real-str0m browser-sized packet coverage; arbitrary large RTP is not promised.

`video_egress_bps` caps aggregate outgoing **video UDP bytes ×8**, including
headers, authentication, retransmissions and BWE probes. Default 32 Mbps.
Audio/ICE/DTLS/SCTP/RTCP are not charged against this video budget. A byte token
bucket permits a 100 ms burst (minimum one configured-max-size datagram); over an interval
T, transmitted video <= `cap*T/8 + max(cap/80,max_datagram_bytes)` bytes. There is no growing
media queue to hide congestion. Per-stream pacer queue admission is bounded to 128 packets/128 KiB and rejects an oldest queued age over 250 ms; overflow discards the stale queue and requests a keyframe with a fresh SSRC. Each active video subscription receives a fair
share of the cap, reduced by its receiver's BWE estimate; the minimum receiver
budget (with 10% RTP/SRTP/RTX headroom) is requested from the publisher through REMB and Bitrate. Noncompliant
senders are hard-dropped at the final output boundary, so RTX/probes cannot
escape the cap. Budget includes network overhead: 4 × 8 Mbps **payload** needs
more than 32 Mbps UDP egress. Separate RTCP/handshake/control host limits remain
necessary to bound non-video abuse.

`VoiceStats {tx_bps:u64,rx_bps:u64,tx_fps_milli:u32,rx_fps_milli:u32,
loss_per_million:?u32,rtt_ms:?u32,path:MediaPath,cap_dropped_packets:u64,
mtu_dropped_packets:u64,queue_dropped_packets:u64}`. Published about once/second by str0m while in a call,
and on Stats requests. Bitrates count emitted/accepted UDP bytes (not cap-rejected writes). Queue drops are counted separately from global-cap drops. FPS counts aggregate video RTP frame markers, **not
renderer/decoder output**, and may include RTX markers. Loss/RTT stay null until
measured, not invented zeroes. Counters are session-local. Path is
`unknown|direct|relayed`, supplied by trusted host Tailscale telemetry; ICE host
candidates cannot reveal DERP relaying. Do not label a Tailscale path direct from
its host candidate alone. No stats polling/media work outside a call.

### Remote-control data channel

Input **never travels on the chat WS**. The viewer opens one in-band negotiated
DCEP channel labeled `sticktalk-control-v1`, ordered/reliable, binary named-map
MessagePack. Other labels, duplicate channels and unreliable channels are closed.
Max message is 512 bytes; send buffer 16 KiB, forwarding stops/revokes at 8 KiB.
The server only forwards `ControlData::Input {sharer,grant:u64,sequence:u64,input}`
from the exact approved controller connection, within the same live call and
while that full-monitor/window publication exists and the requester subscribes to it. A fresh random grant is
issued only in response to a pending request and the sharer's explicit Allow;
one pending request and one active controller per sharer. Requests expire after
30 seconds as Deny, with no remember mechanism. Re-offering a share invalidates
the old grant, even if a MID is reused. Session loss, channel closure, revoke,
share stop and either party leaving invalidate grants. No input contents are
logged; only grant/revoke metadata is logged.

`ControlInput` tagged variants:
- `move {x:u16,y:u16}`: divide each by 65535 for normalized [0,1] bounds.
- `button {button:u8,down:bool}`: buttons 0..4.
- `wheel {x:i16,y:i16}`: bounded -4096..4096 relative deltas.
- `key {usage:u16,down:bool}`: USB HID usage 4..0xe7, never arbitrary commands/text.

Sequence strictly increases within a grant; at most 1000 events in each
server-anchored one-second grant window. Invalid matching-grant input, replay,
overrun or data backpressure **revokes**, rather than dropping a key-up and
leaving control active. Unauthorized/stale-grant packets are discarded. On
revoke, the SFU sends data `release_all {grant}` and WS ControlRevoked as a
fallback. The sharer must clear its local grant and release all held keys and
buttons on **either** message, local hotkey/physical mouse, stream stop or any
transport disconnect. It must also ignore already queued input for a revoked
or mismatched grant. Implementing SendInput, UAC restrictions, local bounds,
Ctrl+Alt+Shift+S and the always-on-top banner is client work, not SFU work.


## 11. Launch roles, profiles, pins, notifications and devices

All additions use named maps with the existing v1 numeric ID and permissions
encoding. Existing fields, command tags and permission bits 0..13 are unchanged.
`Member.role_ids`/`nickname` and `Ready.roles`/`memberships`/
`notification_settings` have serde defaults when reading old snapshots.
`Member.role`/`GuildSummary.my_role` remain legacy compatibility labels, not
sources of authority. `Ready.memberships` contains only the caller's member
records; other members remain paged. `roles` contains definitions for every
joined guild. Joining emits `roles_updated` to bootstrap definitions without
another Ready. A role-definition update invalidates cached computed member
permissions; role IDs in messages/member caches are resolved against the newest
role definitions (deleted IDs are ignored). Member changes carry full role IDs
and nickname. Name color is derived from the highest colored role; hoisting is
client presentation, never permission precedence.

### Roles and authorization

`Role {id:RoleId,guild_id,name,color:u32,position:u16,hoist:bool,
mentionable:bool,permissions:Permissions}`. RGB color is 0..0xFFFFFF, zero means
uncolored; name 1..100 trimmed UTF-8 bytes. Maximum **64 roles/guild**, including
@everyone. Greater positions are higher; @everyone has ID=guild_id, name
`@everyone`, position 0, no hoist/mentionable, and applies to every member
implicitly. `role_ids` in server member projections includes @everyone.

The one server entry point is:

```rust
pub async fn permissions::evaluate(
    connection: &mut sqlx::SqliteConnection,
    user: UserId,
    guild: GuildId,
    channel: Option<ChannelId>,
) -> sticktalk_server::Result<Permissions>;
```

Use inside the shared serialization gate/current transaction, not a cached
MemberRole. Voice integration should call it with Some(channel), require
VIEW_CHANNEL | CONNECT for joining/subscription, and require SPEAK/STREAM/VIDEO/
USE_VOICE_ACTIVITY/moderation bits for the corresponding operations. Reauthorize
live sessions when roles/overwrites change; the separate voice host owns SFU
session teardown. This patch does not implement the concurrent voice host.

1. Validate membership and channel/guild association (nonmembers/cross-guild
   channels return not_found). Guild owner gets all known bits. Ownership is
   determined from guilds.owner_id, not an assignable mask.
2. OR @everyone and all assigned role masks. Strip OWNER. Administrator grants
   all known action bits and bypasses all overwrites, but not ownership/hierarchy.
3. For channel evaluation, apply the overwrites of every channel it is nested
   in, outermost first, then its own. At **each scope** apply @everyone deny then allow; aggregate all
   matching role denies/allows and apply deny then allow; apply the member's
   deny then allow last. Role positions do not alter overwrite precedence;
   one role's allow wins another role's deny. This launch inheritance
   contract is explicit at every level of nesting; there is no separate sync flag. The per-scope
   formula follows [Discord's overwrite algorithm](https://docs.discord.com/developers/topics/permissions).
4. Without View Channel, channel message/history/reaction/media capabilities
   are ineffective. Without Send Messages, Attach Files/Embed Links/broadcast
   mention capabilities are ineffective. Without Connect, speaking, stream,
   video, VAD and priority speaker capabilities are ineffective. Guild-level
   permissions remain available for guild management.

`PermissionOverwrite {kind:role|member,target_id,allow:Permissions,
deny:Permissions}`. Maximum **128/channel**; unique (kind,target_id), no overlap
between allow/deny, known bits only, no OWNER/ADMINISTRATOR. Targets must exist
in the same guild. Set requires effective Manage Roles in the channel and
cannot set bits the actor lacks (owner bypass). Overwrites are complete
replacement sets; empty deletes all. Categories use the same endpoint/command.
Manage Roles may edit/delete/assign only roles strictly below the actor's
highest role. Reordering cannot move/change roles at or above that boundary or
move a lower role across it. Owner bypasses hierarchy. Role creation inserts
above @everyone and shifts numeric positions without changing existing relative
order. Kick/ban/manage-nickname compare highest role positions, never numeric
permission-mask magnitude. No Administrator hierarchy bypass.

Every guild channel access, history/file/upload route, message mutation,
read-state projection and channel-bearing event uses the evaluator. Unauthorized
channels are absent from Ready, GuildDetail, channel lists, unread/mention
counts, reorder placements and event audiences. An otherwise visible channel
nested in an invisible one is projected under its nearest visible ancestor
(parent_id=null when there is none), after that ancestor's own children,
including single-channel snapshots/events (no hidden parent-ID leak). ACL changes clear
body-bearing replay and emit per-user additions/removals/read-state projections.
Connections which lose channel visibility close with **4003** to discard queued
pre-revocation packets; reconnect/resume receives safe removals or Ready.
Already-delivered content cannot be recalled. Receipt replay rechecks current
access. Private channel deletion is sent only to its previous audience.

**Migration:** `202611120001_launch.sql` keeps legacy tables/role strings and
numeric bit assignments intact. It materializes @everyone for every old guild
with all six old Member rights plus Change Nickname, Embed Links, Connect,
Speak, Stream, Video and VAD. A migrated Admin role (guild-local ID=guild_id+1)
has Administrator, and old admin members are assigned it. Owner derives from
owner_id. Triggers support new guilds and legacy `set_member_role`/ownership
transfer commands. The migrated Admin role can be edited/deleted like any
other role; the owner-only legacy assignment command recreates it if necessary.
A custom Administrator role need not have the legacy admin label. New launch
commands never trust that label. New tables avoid rebuilding existing data.

### Commands and events

Launch mutations use `Command::Launch(LaunchCommand)`, wire:

```json
{"type":"command","data":{"nonce":123,"command":{"type":"launch","data":{"type":"create_role","data":{"guild_id":10,"name":"Moderators"}}}}}
```

The launch command data catalog (snake_case tags):

| Tag | Data | Result / authorization |
|---|---|---|
| create_role | guild_id,name | role(Role); Manage Roles, below own highest |
| update_role | role:Role | role(Role); full replacement, position immutable; use reorder |
| delete_role | guild_id,role_id | ack; not @everyone; removes assignments/overwrites |
| reorder_roles | guild_id,role_ids:[RoleId] | ack; complete lowest-to-highest list, @everyone first |
| set_member_roles | guild_id,user_id,role_ids | member(Member); complete assignment replacement, @everyone implicit |
| set_overwrites | channel_id,overwrites | ack; complete ACL replacement |
| update_guild_profile | guild_id,description,banner_color:u8 | ack; Manage Server; description <=120 Unicode scalars; preset 0..9 |
| update_user_profile | banner_color:u32,pronouns,about_me | ack; own profile; RGB, pronouns <=40, about-me <=190 Unicode scalars |
| set_nickname | guild_id,user_id,nickname:?String | member(Member); <=32 Unicode scalars/128 bytes; null clears; Change Nickname for self, Manage Nicknames + hierarchy for others |
| pin_message | channel_id,message_id,pinned:bool | ack; effective Manage Messages in guilds, participant ACL in DMs; <=50 pins/channel |
| set_notifications | NotificationSettings | notification_settings(NotificationSettings); own accessible guild/channel only |
| update_device | device_name,platform | ack; current session; trimmed 1..100 / 1..40 UTF-8 bytes |
| logout_session | session_id:String | ack; own session only; may include current device |
| logout_all_others | no data | ack; preserve current token |

All except logout commands use ordinary durable receipts. Logout commands are
terminal/idempotent destructive session operations without receipts (deleting
the current token also deletes its receipts); retry of an already-deleted
session ID returns not_found. Session sockets are closed **after commit**.
`auth/logout` remains the existing REST log-out-current endpoint.

New event variants:
- `roles_updated {guild_id,roles:[Role]}`: full role-definition replacement to
  guild members; member role assignments still use MemberUpsert.
- `profile_updated(UserProfile)`: own/shared-guild/relationship/DM audiences.
- `guild_profile_updated(GuildProfile)`: current guild members.
- `pins_updated {channel_id,message_id,pinned}`: current authorized channel audience.
- `notification_settings_updated(NotificationSettings)`: all own device streams.
- `sessions_updated`: all remaining own streams; refetch session list.

`Identify.launch_features:bool` defaults false and is omitted when false.
Updated clients set it true to receive the new event catalog; sending a launch
command also opts that session in. Old sessions never receive unknown launch
event variants. Changing the flag across reconnect invalidates replay and
requires a fresh compatible snapshot (including missed events when upgrading). Ready's extra named fields remain
safe for old decoders to ignore. Unknown variants are still rejected, never
guessed by legacy clients. Existing login and old command request shapes remain unchanged. Link embed generation remains consumer work; voice authorization
and broadcast/role mention expansion are implemented as described in section 12.

### REST snapshots and profiles

All endpoints are bearer-authenticated GETs returning `Snapshot<T>`:

| Path | T / access |
|---|---|
| /guilds/{g}/roles | Vec<Role>; member |
| /guilds/{g}/profile | GuildProfile; member |
| /guilds/{g}/preview | GuildPreview; member |
| /users/{u}/profile | ProfileDetail; normal profile access, including readable historical authors |
| /channels/{ch}/permissions | Permissions; own effective channel mask, View Channel required; no raw ACL disclosure |
| /channels/{ch}/overwrites | Vec<PermissionOverwrite>; View Channel + Manage Roles |
| /channels/{ch}/pins | Vec<Message>, newest pin first; View Channel + Read History |
| /me/notifications | Vec<NotificationSettings>; own, inaccessible scopes omitted |
| /me/sessions | Vec<DeviceSession>; own unexpired sessions only |

`GuildProfile {guild_id,description,banner_color:u8,created_at:UnixMillis}` uses
the original guild creation date. `GuildPreview {guild:Guild,profile:GuildProfile,
online_count:u32}` includes member_count in Guild; online counts effective
online/idle/DND presence, not offline/invisible or synthetic REST streams.
`UserProfile {user:UserSummary,banner_color:u32,pronouns,about_me,created_at}`.
`ProfileDetail {profile:UserProfile,mutual_servers:[MutualServer],
mutual_friends:[UserSummary]}` includes only servers shared with the caller and
intersection of accepted friend relationships. `MutualServer {guild:Guild,
member_since:UnixMillis}` gives the target member's join date in that guild,
not global account creation. Missing profile customization defaults to zero/empty.

`NotificationSettings {guild_id:?GuildId,channel_id:?ChannelId,
level:all|mentions|nothing,muted:bool,mute_until:?UnixMillis,
suppress_everyone:bool,hide_muted_channels:bool}`. Server scope: guild_id set,
channel_id null. Channel scope: channel_id set, guild_id must match its guild
(null for DMs). Missing channel override inherits server settings; missing server
settings defaults to all/unmuted/unsuppressed/show muted. Settings affect client
notification presentation, **not authorization or delivery of sync events**.
Clients apply mute expiry by timestamp without polling. Server snapshots report
muted=false after expiry; persisted expiry is retained. All writes sync to own
devices; hidden/inaccessible channel settings are omitted from Ready/snapshots.

`DeviceSession {id:String,device_name,platform,last_active:UnixMillis,
place:String,current:bool}` uses an independent random opaque management ID,
never token/token hash. Place is the literal `this device`; no IP lookup,
geolocation or network dependency. Metadata defaults to This device/Unknown;
clients may label the session with update_device. Last activity is updated on
authenticated HTTP/WS commands (writes coalesced to at most once/minute/session),
not by a new background timer. Revocation immediately removes token validity,
closes live WS, drops replay and cascades device metadata/receipts.

### Validation coverage

`launch_tests` covers all 28 known bit positions (including the legacy internal
sentinels), owner/admin/outsider paths, same-guild validation, everyone/aggregate
role/member overwrite precedence, category/channel composition, implicit
text/voice dependencies, hierarchy/escalation/reorder/assignment/deletion,
legacy-data migration, all existing channel command families, hidden channels
in Ready/read counts, HTTP history/message/file/thumbnail/upload ACLs, personalized
fan-out/reorder/delete/parent filtering, replay/receipt revocation and queued
connection closure, Unicode profile limits/mutuals/nicknames, pin lifecycle,
notification cross-device privacy/expiry and session revocation. Proto launch
round trips and maximum Ready tests cover additive defaults and frame budgets.


## 12. Integrated voice roles, migration compatibility and mentions

### One canonical permission set; negotiated legacy projection

The two parallel branches assigned bit 14 differently. Storage, `Role` and
`PermissionOverwrite` masks and all server authorization now exclusively use
section 2's role numbering: **Manage Roles=14, Connect=19**, Speak=20,
Stream=21, Video=22, Mute Members=23, Deafen Members=24, Move Members=25,
Use Voice Activity=26, Priority Speaker=27. Never treat a legacy output mask as
an assignable role. Existing launch databases already use this numbering and
need no mask rewrite. Existing voice databases have no assignable role masks;
the migration bridge translates their per-member denials to bit 19 overwrites.
Migration 9 and 202611120001 retain their original checksums. The merged
migrator also applies missing migration 9 to a launch-only database before the
202611120002 bridge, preserving all channel references with foreign keys off
only on the dedicated startup connection, then checking/restoring enforcement.

`Identify.launch_features` remains false when absent. Such peers receive legacy
GuildSummary/Member masks in Ready, REST, responses and events: bits 0..13
unchanged, bit **14 means Connect**, higher bits omitted. Canonical Manage Roles
is never exposed as legacy Connect. Launch peers (`launch_features:true`, or
issuing a Launch command) receive canonical masks. Encoding/replay caches are
partitioned by capability; changing it invalidates replay. The master voice preset
constants (16671/24575/32767) retain their numeric values and remain legacy
compatibility constants, not evaluator grants. Phase-1 masks (287/8191/16383)
also continue to decode unchanged.
The old voice wire Member/Admin/Owner projections remain 16671/24575/32767.
All existing command/envelope shapes remain accepted; added variants are opt-in.

### Voice publishing, moderation and immediate revocation

Audio publications require Speak; screen and system audio require Stream;
camera requires Video. Offers and each authenticated media-forwarding action,
including paced/RTX encrypted egress, check the synchronous host projection.
`activity {voice_activity:bool,priority:bool}` explicitly enables VAD and priority
mode only with Use Voice Activity/Priority Speaker respectively. Omission uses
ordinary push-to-talk/nonpriority signaling; these permissions cannot attest to
a client's physical capture implementation. The SFU never trusts a client-sent
moderation command directly.

`moderate {session:VoiceSession,action:VoiceModeration}` is authorized by the
host, even when the moderator is not in a call. Action tagged variants are
`mute {muted:bool}`, `deafen {deafened:bool}`, `move {channel_id}`, and
`disconnect` (no data). They require Mute Members, Deafen Members, or Move
Members (move/disconnect), evaluated on the source channel, plus strict member
hierarchy. Moves stay within one guild; both moderator and target must Connect
to the destination, and the moderator also needs destination Move Members.
Moving retains the transport/session, drops all old-room subscriptions/control
grants, emits old-room Left and destination Joined, and requires resubscription.
Server mute/deafen persist per guild membership across call/WS replacement,
apply to its live devices, and cannot be cleared by a self State command. A
moderator must have the matching right in every affected live channel. Removing
the membership cascades these flags. Deafen suppresses
receiving audio as well as sending it; video remains independently permitted.

Every role definition/assignment/deletion, overwrite, category placement,
ownership, membership and login-session change reconciles live voice under the
normal mutation gate, before success is returned. Removing Connect disconnects;
removing Speak/Stream/Video withdraws the relevant tracks/routes (screen stop
also revokes control). The synchronous ACL changes first; queued media is
invalidated by policy generation, including a driver batch waiting to send.
Private voice state is excluded from unauthorized Ready/live/snapshot views.

### Server-expanded mentions and notification/push hooks

Existing `mention_ids` remain explicit user IDs (maximum 100, unique and
channel-authorized). The server additionally recognizes token-boundary
`@everyone`, `@here` and role references `<@&ROLE_ID>` (or unambiguous `@role`
names without whitespace). @everyone/@here require Mention Everyone after
channel overwrites. A role requires its `mentionable` flag or Mention Everyone;
unknown/ambiguous role IDs/names are rejected where recognized. `<@&GUILD_ID>`
is an everyone mention, not a permission bypass. In DMs these broadcast/role
strings are ordinary text. Content limits bound parsing and work.

Expansion intersects current View Channel audience with all members, currently
connected non-invisible members (@here), or assigned role members. Results are
deduplicated with explicit IDs, persisted transactionally and returned in the
existing Message.mention_ids metadata (up to the bounded 1000-member audience).
Edits fully replace/re-expand metadata; unread mention counts and history use
the stored expansion, not a future role/presence lookup. Notification suppression
never removes durable mention counts. Ordinary Unicode @username text is not
implicitly a user mention; clients still supply explicit user IDs.

After a committed create, bounded `App::subscribe_notifications()` hooks receive
IDs, a mentioned flag and an offline push hint; no message bodies, OS popup,
sound or network push provider is invoked. Hooks honor channel-over-guild
All/Mentions/Nothing settings, mute expiry, suppress_everyone (not explicit or
role mentions), blocks, Invisible/DND and current channel access. Default is
Mentions for guilds, All for DMs. Nonconnected eligible recipients have
`push:true`. Consumers must recheck access before fetching bodies/delivering
external pushes; lagged consumers receive a bounded broadcast lag error.
Receipt replay never redelivers a notification. Edits update unread state but
do not send a second push for an existing message.
## 13. Browser/phone server support (additive v1)

Browser schema uses migration `202611120003_browser`; the existing launch and
voice-role migrations retain their versions and checksums. External Web Push
rechecks the same launch notification preferences as in-process notifications
(including channel overrides, mute expiry, broadcast suppression and Invisible).

### Static PWA hosting and browser boundary

`--web-root` / `STICKTALK_WEB_ROOT` selects a trusted, read-only web build
containing `index.html`, the service worker, manifest, scripts, wasm and fonts.
GET `/` serves this build on the API origin; `/api` never falls back to HTML.
No web root leaves hosting disabled. Files stream from disk. Existing `.br`
and `.gz` siblings are negotiated using `Accept-Encoding`, preserving the
original MIME type and `Vary: Accept-Encoding`. Versioned filenames containing
an 8+ hexadecimal hash segment get `public, max-age=31536000, immutable`;
HTML, manifest files, `sw.*`/`sw-*` and `service-worker*` always get `no-cache`.
Requests with `Service-Worker: script` also always get `no-cache`, regardless
of filename. Other unversioned files get `no-cache`. API responses use `no-store`.

All responses carry CSP (`default-src 'none'`, script self +
`'wasm-unsafe-eval'`, same-origin connections including WS, styles/fonts/
workers/manifests self, no objects/frames/base URL), `Referrer-Policy:
no-referrer`, `X-Content-Type-Options: nosniff`, and `Permissions-Policy` allowing
microphone, camera and display-capture **for self only**. Inline script and style
elements remain blocked: the web build externalizes Trunk bootstrap, boot CSS
and generated UI tokens. `style-src-attr 'unsafe-inline'` is the narrow exception
for dynamic avatar/role colors and virtualized layout geometry. CSP img/media sources
add only operator-configured HTTPS origins from `--gif-media-origins` /
`STICKTALK_GIF_MEDIA_ORIGINS` (comma separated, no wildcards/paths). GIF
metadata must use the same-origin proxy under this policy; direct browser
provider API access is not implicitly permitted. No COOP/COEP is sent: this
single-threaded wasm app does not need SharedArrayBuffer isolation, and COEP
would block third-party GIF media without CORP. Serve only trusted builds,
never user uploads, under the root. Native media download routes are unchanged.

### Cookie sessions and CSRF

Existing bearer REST routes and native WS Identify remain unchanged. Browser
routes below are only under `/api/v1` and return the shared proto DTOs:

| Method/path | Request / response |
|---|---|
| GET `/browser/capabilities` | Public `BrowserCapabilities {cookie_sessions,google_redirect,web_push}` |
| POST `/browser/session/register` | Existing `RegisterRequest`; `BrowserSession` |
| POST `/browser/session/login` | Existing `LoginRequest`; `BrowserSession` |
| GET `/browser/session` | Current session's `BrowserSession`, also refreshes the CSRF cookie |
| POST `/auth/logout` | Existing logout; revokes the session and deletes browser cookies |

`BrowserSession {user:SelfUser,expires_at:UnixMillis,csrf_token:String}` never
exposes a bearer credential to script. Login/register require an exact `Origin`
matching configured `public_url`, and use the existing password verification,
session limits and expiry. Cookies are host-only, Path=/, Secure,
SameSite=Strict: `__Host-sticktalk_session` is HttpOnly; the separate
`__Host-sticktalk_csrf` cookie is script-readable. Cookie lifetime is bounded by
the existing 30-day session expiry. HTTPS is required for deployed browser use.
No cookie Domain or insecure-cookie downgrade is supported.

For **every unsafe cookie-authenticated REST call**, including file streaming,
upload lifecycle, legacy `/api/auth/*` aliases and logout, send both:
`Origin: <public origin>` and `X-StickTalk-CSRF: <csrf_token>`. The server checks
an HMAC-bound double submit token against the CSRF cookie and session credential
using constant-time comparison. Duplicate session/CSRF cookie names are invalid.
Explicit Authorization takes precedence over cookies, preserving native clients.
No CORS allowance is granted to foreign origins. Do not cache private API data
in the service worker or put credentials in URLs/localStorage.

Browser WS upgrades use the HttpOnly cookie and **require** the same exact
Origin (including port); missing, null or foreign Origin is forbidden. The cookie
is authenticated before upgrade and again at Identify. Browser sends the usual
`Identify {version:1,token:"",resume}` after Hello. A nonempty Identify token on a
cookie-authenticated connection is rejected. Native Identify still supplies its
bearer token. Subprotocol, frame limits, replay and session revocation are
unchanged. No auth query parameters are accepted.

`ClientMsg::Focus {focused:bool}` is an additive, ephemeral named-map hint:
`{"type":"focus","data":{"focused":true}}`. Both browser and native clients
may send it after Ready/Resumed and on focus/visibility changes. Default false,
never replayed, cleared by disconnect/replacement; keep existing heartbeats so
dead sockets expire. A focused live client suppresses that user's pushes across
sessions. This is a notification hint, not permission/presence/read-state proof.

### Server-side Google redirect + PKCE

Configure **separate Web application OAuth credentials** with
`--google-web-client-id` / `STICKTALK_GOOGLE_WEB_CLIENT_ID` and
`--google-web-client-secret` / `STICKTALK_GOOGLE_WEB_CLIENT_SECRET`. Keep the
secret server-only. Existing desktop client ID/ID-token login remains unchanged.
Redirect capability is disabled unless both web credentials and HTTPS public
origin are configured. Register the exact redirect URI:
`<public_url>/api/v1/browser/google/callback` in Google Cloud.

1. Navigate to GET `/api/v1/browser/google/start`. The server generates random
   state and a PKCE verifier, stores only the state hash with the verifier for
   10 minutes, and redirects to Google's authorization endpoint with S256.
2. A Secure, HttpOnly, host-only `__Host-sticktalk_oauth` state cookie has
   **SameSite=Lax** just for the cross-site top-level Google callback GET.
   Session and signup cookies remain Strict. Never make the session cookie Lax.
3. Callback GET accepts `state` and `code` (or provider `error`). State must
   match the cookie and a live DB row; consumption is atomic and single-use,
   before network I/O. Token exchange uses server-side code + verifier + client
   secret, the fixed redirect URI, bounded responses and no redirects.
4. Existing RS256 Google ID-token verification is reused, with the **web client
   ID audience**, followed by the existing subject-only identity lookup and
   signup-ticket issuance. No email linking or automatic account merging.
5. Existing user: set browser session cookies, delete the state cookie and
   redirect to `/`. New user: put the existing 10-minute signup ticket in a
   Secure/HttpOnly/Strict `__Host-sticktalk_signup` cookie, create a CSRF cookie,
   and redirect to `/?google=signup` (no ticket or credential in the URL).
6. Submit POST `/api/v1/browser/session/google/complete` with
   `GoogleCompleteRequest {ticket:"",username}`, exact Origin, CSRF header and
   signup/CSRF cookies. The server supplies the cookie ticket to the existing
   atomic signup flow. Response is `BrowserSession`; the signup cookie clears.
   Username collision leaves the ticket usable until expiry; reuse is rejected.

Loopback integration tests may explicitly set `--mock-google-token-url` in
addition to the existing mock issuer/JWKS configuration; this is rejected on a
non-loopback HTTP bind. Real browsers/Google are not launched by tests.

### Web Push

| Method/path | Request / response |
|---|---|
| GET `/push/key` | Public `PushKey {public_key}`; base64url uncompressed P-256 VAPID key |
| POST `/push/subscriptions` | `PushSubscribeRequest {endpoint,p256dh,auth}`; 201 `PushSubscription` |
| GET `/push/subscriptions` | This login session's `[PushSubscription]` only |
| DELETE `/push/subscriptions/{id}` | This session only; 204, otherwise not_found |

The JS PushSubscription `getKey("p256dh")` and `getKey("auth")` values are
unpadded base64url (65-byte uncompressed valid P-256 point and 16-byte secret).
Normalize browser PushSubscription JSON to the flat request above.
`PushSubscription {id:String,endpoint:String,created_at:UnixMillis}` never
returns private key material. At most five subscriptions/session; an endpoint
owned by another session conflicts. Logout/account deletion cascades removal;
expired sessions cannot receive pushes even before housekeeping removes rows.

VAPID ES256 PKCS#8 keys are generated once into `<data_dir>/vapid.pk8`; Unix
creation mode is 0600. Back up that file with the database and protect the data
dir with owner-only permissions/Windows ACLs. Corrupt keys fail closed (no
silent rotation). Configure `--vapid-subject` / `STICKTALK_VAPID_SUBJECT` as a
contact `mailto:` or HTTPS URI; default is public_url. JWT audience is the push
endpoint origin, expiry 12 hours. Payload encryption uses RFC 8291 ephemeral
P-256 ECDH, authenticated HKDF-SHA256 and a single AES-128-GCM final record;
VAPID signing keys are never reused as ECDH keys. Each delivery gets fresh
salt/key material. The payload is `PushPayload {url,channel_id,message_id,count,
title,body}` with generic text only, never message contents/author profiles.
IDs have the same u64 precision requirements as all v1 JSON. The server-generated
relative `url` (`/?channel=<decimal>&message=<decimal>`) preserves IDs for a plain
JavaScript service worker: use that URL for notification-click navigation instead
of reconstructing IDs after JSON.parse. Validate that navigation remains same-origin.

Committed DMs/group DMs and explicit mentions enqueue notifications; edits and
command-receipt replay do not. Queue 1024, batch at most 1024 user/channel pairs,
2-second event-driven coalescing, at most one delivery round/user/30 seconds
(up to 25 session subscriptions), push TTL 300 seconds and stable channel Topic.
The worker sleeps on its queue when idle; overload is best-effort/drop, not an
unbounded queue. Focus, current ACL, deleted messages, block, DND, read state,
revoked/expired sessions and a pluggable policy are checked before delivery.
`App::set_push_policy(Arc<dyn push::NotificationPolicy>)` integrates the parallel
notification-settings work; `allows` receives a SQL connection and `PushNotice`
(user/author/channel/message IDs and direct/mentioned flags), and fails closed
on errors. Register this policy at startup; default allows DMs/mentions after
the base checks. It must not acquire the chat mutation gate. No network work
occurs under the chat gate. Provider 404/410 deletes subscriptions.

SSRF protection: only exact configured push-service origins are allowed, no
credentials/fragments, no HTTP or redirects. Defaults: `https://fcm.googleapis.com`,
`https://updates.push.services.mozilla.com`, `https://web.push.apple.com`.
`--push-origins` / `STICKTALK_PUSH_ORIGINS` replaces this allowlist with trusted
HTTPS services; do not allow untrusted DNS or internal services. A separate
`--mock-push-origin` is permitted only with a loopback bind for local tests.
Transient provider failures are best-effort (no durable retry queue); browsers
refresh subscriptions after session expiry/key rotation. Real APNs/FCM/Firefox
and installed-iOS/Android delivery require deployment acceptance testing.

### History extent / scrollbar anchors

GET `/api/v1/channels/{channel_id}/history-extent` returns
`Snapshot<HistoryExtent>` with the usual session cursor:
`HistoryExtent {channel_id,message_count:u64,anchors:[HistoryAnchor]}` and
`HistoryAnchor {rank:u64,message_id}`. Requires View Channel + Read History on
an actual text/DM/group-DM channel; normal not-found/access rules apply.

Count is exact for nondeleted messages. Up to 65 deduplicated anchors are ordered
oldest-first. Rank is approximate, zero-based and interpolated over channel-local
insertion ordinals (deletions leave gaps); first/last anchors are exact endpoints.
Use the nearest rank anchor and existing `messages?around=<message_id>` to fetch
that region directly. Empty channels return zero and no anchors. No OFFSET,
COUNT or full-history scan happens per request: a trigger-maintained count plus
bounded covering primary-key seeks in `history_entries`. Migration performs the
one-time backfill; insert/soft-delete/physical-delete/channel cascades maintain
it transactionally. This index does not cache message bodies. Concurrent
mutations are serialized with the normal snapshot gate. Clients should refresh
the extent after substantial live changes, not poll on an idle timer.

### Security hardening (2026-11)

New additive DTOs: `AccountOperation` (`sign_in`, `link`, `unlink`, `password`,
`delete_account`), `GoogleChallengeRequest/Response`, `ReauthRequest/Response`,
`AccountUpdateResponse`. All routes below accept both API aliases unless noted.

- POST `/api/v1/auth/google/challenge` `{operation}` issues a random nonce valid
  five minutes. `sign_in` is public; other operations require the target bearer
  session. Send it as Google's OIDC `nonce` authorization parameter. The server
  verifies the signed ID token's nonce and atomically consumes its challenge,
  bound to this operation/session. Tokens without a live challenge are rejected.
- POST `/api/v1/auth/reauth` `{operation,current_password?,id_token?}` proves an
  EXISTING login method. Google proof must match the account's linked subject and
  use a fresh challenge for this operation/session. Returns `{proof,expires_at}`.
  The random proof is single-use and expires after five minutes.
- Link/unlink, first-password creation and POST `/api/v1/auth/account/delete`
  require header `x-sticktalk-reauth: <proof>`. Normal password changes may instead
  provide `current_password`; Google reauthentication is also accepted.
- Password updates return `AccountUpdateResponse` with flattened existing auth
  method booleans and `session: AuthResponse`. Persist the rotated token and
  reconnect. ALL previous account tokens (including the current old token), WS
  streams and voice sessions are revoked after the atomic credential transaction.
- Cookie-authenticated password changes return `BrowserSession` and replace the
  Secure/HttpOnly session + CSRF cookies; no bearer token appears in web JSON.
- Browser Google start supports `?operation=sign_in|password|link|unlink|delete_account`
  for existing-account reauth, storing a short-lived HttpOnly proof cookie. After
  password/Google reauth for `link`, `?operation=link_identity` authorizes the NEW
  Google identity. Both callbacks bind nonce/state/PKCE, operation and session.
  No existing identity is inferred from email. Operations never rely on a generic
  'recent login' timestamp.
- Account deletion responds 204, clears credentials/sessions, tombstones authored
  messages and profile as Deleted User, deletes attachment references/files, and
  reserves the former username for 30 days. Direct-message tombstone identities
  remain resolvable. An owned guild transfers to its earliest active remaining
  member, or is removed if empty. Clients reconnect to discard cached content.
  Historical encrypted backups expire after 14 days (see deployment privacy notes).
- Guild storage's host capacity/free/floor fields are zero (not disclosed);
  committed/partial/reserved counts include only the requested guild's channels.
  Host-wide metrics are available solely from local `admin storage-status`.
- Upload daily ingress allowance defaults to 100 GiB, shared across multipart and
  resumable protocols, charged on arriving bytes including retries/aborts.
  Maximum three unfinished uploads/user; idle reservations expire after one hour.
  Zero file-size configuration means unlimited, for BOTH protocols.

## Additive text features: custom server emoji

Text-feature peers may use `RenameEmoji { guild_id, emoji_id, name }` and
`DeleteEmoji { guild_id, emoji_id }` inside `Command::Launch`. Permission
`MANAGE_EMOJI` is bit 28; owners/administrators bypass normally. Changes emit
`GuildEmojiUpdated { guild_id, emoji: GuildEmoji[] }` to text-feature guild
members. Legacy clients never receive this new event.

- `GET /api/v1/guilds/{id}/emoji` returns `Snapshot<Vec<GuildEmoji>>`.
- `POST /api/v1/guilds/{id}/emoji?name=...` accepts a raw PNG/GIF/WebP body
  of at most 262144 bytes. This REST mutation is an asset-upload lifecycle,
  not an alternative command transport. Names are 2–32 ASCII alphanumerics/
  underscores, case-insensitively unique within the guild. Maximum 50/guild.
- Images are normalized to a maximum 128×128 bounding box, preserving alpha.
  GIF/animated WebP is normalized to GIF. Safety limits: 2048px source axes,
  32 MiB total decoded input frames, 120 frames, 256 KiB normalized output.
- `GET /api/v1/emoji/{id}` serves normalized bytes; `/preview` serves a static
  transparent PNG first frame. Both require current guild membership on every
  request, use private/no-store caching, and never expose public asset URLs.
- Message tokens are `<:name:id>`. Unknown/deleted emoji remain text rather
  than loading arbitrary URLs. Reactions accept custom tokens only from the
  message's server; storage uses `<:emoji:id>` so renaming cannot split counts.
  Existing reaction removal remains possible after emoji deletion.
- Desktop playback is hover-only and focused/visible-only. Leaving the row
  drops animation frames and removes its one-shot timer; static previews need
  no periodic task.

## Additive text features: server message search

`GET /api/v1/guilds/{id}/messages/search?q=...&before=<id>&limit=25`
returns `Snapshot<MessageSearchPage>`. `items` and `users` are bounded to
50; `next_before` is an exclusive descending-ID keyset cursor, absent at
end. `has_more` never counts inaccessible messages. The entire read, including
channel permission evaluation, runs under the normal snapshot/mutation gate.
Only text channels with **both View Channel and Read History**, including
category and member overwrites, enter the SQL query before pagination.

`q` is 1–512 UTF-8 bytes and at most 32 terms/filters. Plain terms are ANDed
literal FTS5 prefix phrases; double quotes group a phrase. Raw FTS operators
are never accepted as query syntax. Filters:

- `from:<username|user-id|me>`
- `in:<channel-name|channel-id>` (optional `#`)
- `mentions:<username|user-id|me>`
- `has:link`, `has:file`, `has:image` (multiple filters are ANDed)
- `before:YYYY-MM-DD`, `after:YYYY-MM-DD` (UTC midnight, exclusive)

Duplicate single-value filters, invalid dates, malformed quotes, unknown
`has:` types, and limits outside 1–50 return `invalid_request`. Missing users
or unreadable channels produce an empty page, not cross-server information.
Search has its own bounded rate bucket. FTS5 external content uses a
non-deleted-message view, migration backfill, and insert/update/delete triggers;
soft-deleted text is removed in the same transaction as message deletion.

Desktop Enter submits the header query to a right-hand results panel. The
panel retains one page plus at most 64 keyset cursors, highlights literal terms,
and uses the existing around-message history seek on result activation. No
background polling; canceled/stale responses cannot replace a newer search.

## Additive text features: slowmode

`LaunchCommand::SetSlowmode { channel_id, seconds }` requires Manage Channels
and a guild text channel. Valid seconds: `0,5,10,30,60,300,900,3600,21600`.
Changes persist and emit `SlowmodeUpdated { channel_id, seconds }` only to
text-feature channel viewers. `GET /api/v1/channels/{id}/slowmode` returns a
personalized `Snapshot<SlowmodeStatus>` (interval, bypass flag, server time,
remaining milliseconds). Read access is checked on every fetch.

SendMessage checks the latest persisted send by that author/channel under the
serialized mutation transaction; concurrent devices cannot race the interval.
Manage Messages or Manage Channels bypasses it. Retries of a committed command
retain normal receipt semantics. Edits do not consume the interval, and deleting
the previous message does not evade it. Rejections use `rate_limited`,
`field="slowmode"`, and `retry_after_ms`.

The desktop composer keeps an editable draft during cooldown and ignores sends
until expiry, showing a seconds countdown. The host uses a monotonic local
clock mapped from the server snapshot, not local wall time. Countdown timers
exist only while a restricted channel is focused and the cooldown is nonzero;
no slowmode timer exists when disabled, exempt, expired, or offscreen.

## Additive text features: device notifications and server folders

`Identify.text_features` is an independent optional decoder capability,
defaulting to false and omitted when false. **All new emoji, slowmode, and
folder event variants require this flag**, including for older peers that
already opted into `launch_features`. Capability changes invalidate replay and
force a fresh Ready. The desktop/shared native protocol opts in; the current
web host opts out until its separately-owned UI adopts these events.

`LaunchCommand::SetServerFolders { folders: Vec<ServerFolder> }` replaces only the caller's
private folders. Maximum ten folders, ten total member servers, positive
account-local folder IDs, unique server placement, 1–32 character names without
controls, RGB color 0..0xffffff. Order and collapsed state persist per account.
`GET /api/v1/users/@me/server-folders` returns the caller's cursor-bearing
snapshot. `ServerFoldersUpdated` reaches only that account's opted-in sessions.
Membership foreign keys remove departed/deleted servers from folder contents.

Desktop rail context menus expose Add to Folder/Remove from Folder; folder
right-click opens name/color settings and Ungroup Folder. Collapse/expand is
optimistic, synced to other devices, and rolls back by snapshot on rejection.
No drag-and-drop implementation is claimed.

The local `Preferences.desktop_notifications` master switch defaults true for
existing settings files and persists through the existing settings store.
Disabling it prevents desktop toasts, message sounds, and mention flashes even
when a server/channel override says All; unread counts and badges are unchanged.
It is intentionally device-local, not an account/server notification level.

## Additive: MCP connector (AI assistants)

Outside `/api`, the server serves a remote MCP endpoint and the OAuth 2.1
endpoints that authorize it: `POST /mcp`, `/.well-known/oauth-protected-resource[/mcp]`,
`/.well-known/oauth-authorization-server`, `/oauth/register`, `/oauth/authorize`,
`/oauth/token` and `/oauth/revoke`. [MCP.md](MCP.md) is normative for them. MCP
tool writes call the same command service as WS `Command`s, as the connection's
own session, so permissions, receipts and events are unchanged. Each connection
appears in `DeviceSession` lists with platform `AI connector (MCP)`, and
`LogoutSession` revokes it. MCP bearer tokens (`stka_…`) are not session tokens
and are rejected by `/api`; session tokens are rejected by `/mcp`. Connector
sessions are excluded from the five-session sign-in limit.
