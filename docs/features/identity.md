# Server identity (task `identity`)

Server tags, role styles (gradient and holographic), role icons, the server
banner, animated server icons, the invite background, rich invite cards and
vanity invite links. Discord sells these with Server Boosts; in StickTalk
every server has all of them, free.

Code: `crates/proto/src/identity.rs` (wire), `crates/server/src/identity.rs`
and `identity/{tags,looks,vanity,routes,tests}.rs` (server),
`crates/web/src/live/identity.rs` and `live/ui/identity.rs` (web),
`crates/web/public/identity.js` (cropping, tag editor), `crates/web/styles/identity.css`,
`crates/web/public/tags/*.svg` (32 badge glyphs drawn by
`crates/web/scripts/tag_glyphs.py`, original art), `crates/web/tests/identity.mjs`
(browser test). Migration `202611231601_identity.sql`. No new permission bits.

## Permissions

| Action | Who |
| --- | --- |
| Set or remove the server tag, hide other servers' tags, server images (banner, icon, invite background), vanity link, `GET .../identity` | Manage Server (owner and Administrators have everything) |
| Role style and role icon (`SetRoleStyle`, `SetRoleIcon`, icon upload) | Manage Roles, and the role below your highest role (the owner has no ceiling); never @everyone |
| Wear a tag | a member of that server, for a server that has a tag |
| Tag card (`GET /api/v1/tags/{g}`) | anyone signed in, for a server that has a tag |
| Invite card and invite art | anyone holding a usable invite code |
| Role icon picture | members of its server |
| Revoke a tag, unblock a tag text, release a vanity code | the instance administrator, `sticktalk-server admin identity ...` |

Every check runs on the server and has a test in `identity/tests.rs`.

## Wire protocol (all additive)

- `UserSummary.style.tag` (`WornTag {guild_id, text, glyph, color, color2}`,
  omitted when none): the tag a user wears. Every user summary carries it, so
  names everywhere (messages, member lists, DM lists, friends, profiles, call
  tiles) draw it with no request.
- `Guild.look` (`GuildLook {banner?, icon?, hide_tags}`, boxed, omitted when
  empty): the banner, whether the icon (`icon_id`) moves and its crop (only
  when `icon.id == icon_id`), and whether other servers' tags are hidden here.
- `Role.look` (`RoleLook {style?, icon?}`, boxed, omitted when empty):
  `RoleStyle {paint: gradient|holographic, color2?, color3?}` (a gradient
  starts at `Role.color`), `RoleIcon {rev?, emoji_id?, emoji}`: an uploaded
  picture (`rev`), a custom emoji of the server (`emoji_id`, its name in
  `emoji`) or a Unicode emoji (`emoji`). Plain structs, not enums, keep the
  snapshot decoder in the web app's first load small. `UpdateRole` ignores it
  (an older client cannot clear it).
- `Command::Identity(IdentityCommand)`, every one answered with `Ack`.
  No new result type and no new events: changes travel as `SelfUpdated`/`UserUpdated`/`MemberUpsert`
  (tags), `GuildUpsert` (server look) and `RolesUpdated` (role look).
- `AuditAction::ServerTagUpdate` (`server_tag_update`) and `VanityUpdate`
  (`vanity_update`), appended. Role styles and icons are `role_update`
  (fields `style`, `icon`); server images and tag hiding are `guild_update`
  (fields `banner`, `icon`, `invite_background`, `other_server_tags`).

| Command | Notes |
| --- | --- |
| `SetTag {guild_id, tag?: TagEdit {text, glyph, color, color2, invite_code?}}` | None removes the tag and everyone stops wearing it. 6/min per server. |
| `WearTag {guild_id?}` | None wears no tag. One tag at a time. |
| `SetHideTags {guild_id, hide}` | |
| `SetImage {guild_id, slot: icon\|banner\|invite_background, image?: ImageRef {id, crop?}}` | `image` is the person's own fresh avatar upload (or one this server already uses). |
| `SetRoleStyle {guild_id, role_id, style?}` | |
| `SetRoleIcon {guild_id, role_id, icon?: emoji{emoji} \| custom{emoji_id}}` | A picture is uploaded with the REST route below. |
| `SetVanity {guild_id, code?}` | None releases. `GET .../identity` has the result (`Vanity {code, uses, claimed_at}`). |

## REST routes (cookie or bearer)

| Route | Returns |
| --- | --- |
| `GET /api/v1/guilds/{g}/identity` | `IdentityAdmin`: tag, wearers, look, invite background, vanity (with every use), `vanity_wait_ms`, the server's usable invite codes |
| `GET /api/v1/users/@me/tags` | `MyTags {wearing, available}` |
| `GET /api/v1/tags/{g}` | `TagCard {tag, name, icon_id, member_count, description, join}`; `join` is `none`, `member`, `invite {code, apply}` (the invite the server chose for its tag, while usable) or `directory {apply}` (listed in Discover); never for someone banned |
| `GET /api/v1/invites/{code}/card` | `InviteCard`: name, description, member and online counts, icon, banner, background, tag, joined, apply, vanity. Only what holding the code already grants; no channels or members |
| `GET /api/v1/invites/{code}/art/{icon\|banner\|background}[?still=1]` | the image, for holders of a usable code (`nosniff`, `sandbox`) |
| `GET /api/v1/directory/{g}/art` | `DirectoryArt {banner, tag}` for a listed server |
| `POST /api/v1/guilds/{g}/roles/{r}/icon` (body: PNG or WebP) | `RoleLook`; audited; members get `RolesUpdated` |
| `GET /api/v1/guilds/{g}/roles/{r}/icon?v={rev}` | the 64x64 PNG rendition, members only |
| `GET /invite/{code}` | 303 to `/?invite={code}` (the shareable vanity link) |

Reads share an `identity-rest:{user}` budget (120/min, burst 30); images use
the `file:{user}` budget like `/api/v1/files`.

File access: a server's banner and icon uploads are readable by its members,
by anyone signed in when the server is listed in Discover, and its icon by
anyone signed in when it has a tag (its tag card shows it). The invite
background is only served through an invite.

## Limits and validation (server)

| What | Rule |
| --- | --- |
| Tag text | 1–4 characters: ASCII letters, digits and `! # $ % & * + - = ? @ ^ _ ~`, at least one letter or digit |
| Tag uniqueness | one per instance, ignoring case (`server_tags.tag_key` unique) |
| Reserved tags | `RESERVED_TAGS` (STAFF, ADMIN, MOD, SYS, ROOT, the app's name ST/STK/STIK/TALK…), compared after dropping symbols and folding look-alike digits (0→O, 1→I, 3→E, 4→A, 5→S, 7→T, 8→B), so `M0D` and `AD-M` are reserved; plus tags the administrator revoked (`blocked_tags`) |
| AutoMod | the server's own enabled keyword rules that block, with no exemptions (only Manage Server holders set the tag, and they would otherwise be exempt) |
| Badge | one of the 32 built-in glyphs; the two colors at least 3:1 apart (WCAG non-text contrast) |
| Tag invite | an invite of this server that is usable when saved and when the card is read |
| Role gradient | the role has a color; 2 or 3 stops; each at least 4.5:1 against `bg.selected` (`legible_name_color`, the profile name rule) |
| Role icon picture | PNG or WebP, at most 256 KB and 1024x1024, decoded in full, stored as a 64x64 PNG rendition beside the original |
| Role icon emoji | one Unicode emoji (1–10 characters, pictographic), or a custom emoji of this server |
| Server images | avatar uploads: 10 MB, PNG/APNG/JPEG/WebP/GIF, every frame decoded and capped, still first-frame rendition (`docs/features/profiles.md`) |
| Vanity code | 3–32 lowercase letters, digits and single hyphens, not at either end; stored lowercase, so it matches in any case; unique per instance; `RESERVED_VANITY`; at most 3 changes (claim, change, release) per server in any hour, counted from the audit log; a released or lost code is held 30 days for everyone but the server that held it |

## Behavior

- **Wearing.** The worn tag is copied onto `users.tag` (with `users.tag_guild`)
  so it rides on every user summary. Editing the tag updates every wearer's
  copy at once and queues every wearer in `tag_announce`; once the edit
  commits, the fan-out task (`identity::fanout`) tells each wearer's viewers
  in batches of 64, holding the command gate only while a batch is read and
  sent, with no transaction open across batches. Removing the tag, or the
  administrator revoking it, takes it off everyone the same way. A big
  server's members leaving at once (deleting the server) are queued there
  too, past the first 64.
- **Leaving drops it immediately.** A trigger on `guild_members` clears the
  tag the moment the membership row goes (leave, kick, ban, prune, server
  deletion, account erasure) and queues the user in `tag_drops`;
  `identity::settle`, inside the same command's transaction, sends
  `SelfUpdated` and `UserUpdated` for them.
- **Hiding other tags.** With `hide_tags`, a server's channels and member list
  show only its own tag; other servers' tags stay everywhere else.
- **Vanity links are invites.** Each is an invite row whose hash is the
  lowercase code's, with no expiry, so joining, the preview, rules screening,
  applications, raid pauses and insights' join sources treat it like any
  invite (`identity::find_invite` is used by every invite lookup). The invites
  table caps uses at 1,000, so the count moves into `vanity_codes.uses_base`
  at 900. `RevokeInvite` refuses it (field `vanity`). A deleted server's code
  is held too (a trigger).
- **Role styles in names.** The name takes the style of the member's highest
  role that has a color or a style (Discord's rule for colors). Combination
  rule: **a member's own display name style always wins for their name**;
  with "Show plain names" on, everyone else's name shows their role style
  again. A future "Role colors: in names / dot / off" preference sets
  `html[data-role-colors]`; `dot` and `off` turn role styles off the same way
  they turn role colors off (`identity.css`). Stops pass the same legibility
  floor as solid role colors and are then mixed `--role-ink` (22 %) toward the
  theme's `--text-primary`, so they stay readable on any theme's background;
  a light theme that needs more raises `--role-ink`. Holographic is a fixed
  iridescent sweep; a second sweep fades in and out (opacity only) while the
  row is hovered or focused.
- **Role icons** show after the name in messages and the member list: the
  highest role with an icon, with the role name as its accessible label and
  tooltip. Custom emoji icons use the still preview, so nothing moves in a
  list.
- **Banner.** Shown at the top of the channel list, scrolling away with it
  (no script, no layout change). The server header floats over it on a scrim
  of 78 % sidebar color, so its text keeps the theme's own contrast whatever
  the picture; a scroll-driven opacity turns the header solid as the banner
  leaves (browsers without scroll-driven animations keep the header above the
  banner). An animated banner plays only while hovered.
- **Animated icons.** The rail shows the still first frame; the animation is a
  CSS background applied only while the button is hovered or focused, so it
  is not even fetched until then. Reduced motion never plays it.
- **Cropping.** Choosing a banner (5:2), icon (1:1) or invite background
  (16:9) opens a crop dialog; stills are cropped in the browser, animations
  are uploaded as they are and the crop is applied by CSS, exactly as for
  profile images.
- **Invite cards.** A message linking `<this server>/?invite=CODE` or
  `/invite/CODE` (up to three) shows a card: banner, icon, name, tag, online
  and member counts, description, Join (or Apply, or Joined). Each code is
  fetched once when on screen and kept in a 32-entry cache; the card has its
  final size while loading. The join screen shows the invite background (or
  the banner) behind the card on a scrim.
- **Invite codes and links** render through one helper, `ui::invite_text`
  (codes and copied links: the invite list, the vanity page, a Stage event's
  invite) or `ui::message_link` (links in messages, marked when they contain
  `invite=` or `/invite/`), both `.streamer-invite`, so Streamer Mode hides
  every invite, whatever its form. The tag's invite picker shows only the
  last four characters of secret codes.
- **Tag card.** Clicking a tag in a message (or on a profile, invite or join
  card) opens the card; in member, DM and friend rows the tag is a static chip
  because the row itself opens the profile.
- **Call tiles** show the tag text after the name.
- **Data export.** `settings.json` has `worn_tag`.

## Web app

- Server Settings › Identity: **Server Tag** (text, 32 badges, two colors with
  a contrast warning, live preview, join offer, wearers, "Hide other servers'
  tags"), **Appearance** (banner, server icon, invite background, each with a
  preview, crop, remove), **Vanity URL** (claim, change, copy, uses, release
  with a confirmation, the wait after three changes).
- Role editor: Role Style (solid, gradient with a second and optional third
  color, holographic; preview) and Role Icon (emoji, server emoji, upload,
  remove).
- Profile editor: Server Tag picker (no tag, or one of your servers' tags).
- Profile card: the tag after the name and the member's roles as pills.
- Directory preview: the server's banner.
- Hooks in shared files are one line each: `ui.rs` (names in messages and the
  member list, invite cards, sidebar banner, role editor, join screen, call
  tile data), `navui.rs` (rail icon), `settings.rs` (pages), `profiles.rs`
  (card and editor), `community.rs` (directory banner), `operations.rs`,
  `mod.rs`, `live.js`, `media.js`, `app.css`, and the packager's runtime
  module list and precache exclusions.

## Efficiency

- Snapshot cost: a few small fields per user (only when wearing a tag), per
  server (only with a banner, an animated or cropped icon, or hidden tags) and
  per role (only with a style or icon). Nothing new is sent to a client that
  does not use the feature.
- First load: the settings pages, role controls, tag card and join card are
  reached only from dialogs (split out of the core WASM); `identity.js` and
  the glyph SVGs load on first use and are not precached. The list renderers
  and `identity.css` are small.
- No timers: vanity holds are checked when a code is next claimed; the vanity
  rate limit reads the audit log.
- Measured against the parent commit (`b957ccf`), same build script, Brotli:
  core WASM 1,097,215 → 1,118,006 bytes (+20.8 KB: the decoders for the new
  snapshot fields and the list renderers), `sizes.json` firstLoadBytes
  1,402,930 → 1,427,634 (+24.7 KB, with the CSS), the lazy dialog chunk
  +23 KB. The `id:` actions, settings loaders, invite-card renderer and its
  decoder are `#[lazy]` split modules. `RoleIcon` is a struct and no
  `IdentityResult` exists precisely to keep the snapshot decoder small.

## Admin CLI

```
sticktalk-server --data-dir <dir> admin identity list
sticktalk-server --data-dir <dir> admin identity revoke-tag <guild_id>
sticktalk-server --data-dir <dir> admin identity unblock-tag <TEXT>
sticktalk-server --data-dir <dir> admin identity release-vanity <code>
```

The CLI edits the database while the server may be running. A revoked tag
queues its wearers in `tag_announce`; the running server's housekeeping pass
(every minute) finds them and wakes the fan-out task, so open apps see the
change within a minute. Revoking a tag and releasing a vanity code are in the
server's audit log (`server_tag_update`, `vanity_update`), attributed to the
server's owner with the reason "Revoked/Released by the instance
administrator" (an entry needs an actor, and the administrator has no
account there).

## Decisions to revisit

- Tags are ASCII only, so look-alike letters from other scripts cannot be
  used to imitate reserved words or other servers' tags.
- A tag change is told to every wearer's viewers in batches after it commits
  (the command itself only queues them).
- The admin CLI's revoke is told at the next housekeeping pass (within a
  minute), not instantly: the CLI is another process and the server does not
  watch its database for changes.
- The banner is 5:2 (like profile banners) rather than Discord's 16:9, which
  keeps the channel list visible on short screens.
- The "Role colors: in names / dot / off" preference is not built here (it
  belongs to the accessibility work); role styles already honor its
  `data-role-colors` attribute.
