# Profiles and cosmetics

Animated avatars, profile banners, profile colors and bio, per-server
profiles, display name styles, badges, avatar decorations, profile effects,
nameplates and profile frames. Everything is free for every user; there is no
shop. Contracts: `crates/proto/src/profiles.rs`. Server:
`crates/server/src/profiles.rs` (+ `profiles/media.rs`, `profiles/badges.rs`).
Web: `crates/web/src/live/profiles.rs`, `crates/web/src/live/ui/profiles.rs`,
`crates/web/public/profiles.js`, `crates/web/styles/profiles.css`, artwork in
`crates/web/public/cosmetics/` and `crates/web/public/badges/`, fonts in
`crates/web/public/fonts/names/`. Migration: `202611231001_profiles.sql`.
Browser test: `node crates/web/tests/profiles.mjs` (needs `CARGO_TARGET_DIR`
with `web-dist`, `STICKTALK_SERVER_EXE`; screenshots go to `STICKTALK_QA_DIR`).

## Wire protocol (all additive)

- **`UserSummary.style`** (`UserStyle`, omitted when empty): the global look of
  a name and avatar. Every user summary carries it, so every place that draws
  a user can draw it, with no extra request.
  - `avatar`: `ImageMeta {id, animated, crop}` for the avatar whose ID it
    carries. A client ignores it when `avatar_id` is a different upload (an
    old client set a new avatar with `UpdateProfile`).
  - `name`: `NameStyle {font, color, color2, effect}`.
  - `decoration`, `nameplate`: catalog IDs.
- **`Member.server_style`** (`UserStyle`, omitted when empty): what the
  member's server profile overrides in that server. The server nickname stays
  `Member.nickname`.
- **Commands:** `Command::Launch(LaunchCommand::Profiles(ProfilesCommand))`,
  all answered with `Ack`:
  - `update(ProfileEdit)`: the global profile as a whole: pronouns, about me,
    banner color, primary and accent colors, name style, decoration,
    nameplate, effect and frame. It also writes the existing `user_profiles`
    row, so `ProfileUpdated` and the old profile routes stay correct.
  - `update_server {guild_id, edit: ServerProfileEdit}`: a server profile as a
    whole; every field is optional and `None` falls back to the global one.
    Members only.
  - `set_image {guild_id?, slot: avatar|banner, image?: ImageRef {id, crop?}}`:
    `guild_id` none is the global image; `image` none removes it.
  - `set_badges {shown}`: up to six earned badge IDs, in order.
  - `reset {guild_id, user_id, avatar, banner, reason?}`: a moderator removes
    a member's server avatar and/or banner.
  - Display name and status still use `UpdateProfile`; the server nickname
    still uses `SetNickname` (and its Manage Nicknames rule for others).
- **Events:** no new events. A changed look sends `SelfUpdated`, `UserUpdated`
  (to viewers who can see the profile) and `MemberUpsert` in every server the
  user is in; a changed server look sends `MemberUpsert` in that server; text
  changes send the existing `ProfileUpdated`.
- **Audit:** `AuditAction::MemberProfileReset` ("Reset a server profile"),
  with `server_avatar` / `server_banner` changes and the reason.
- **Uploads:** avatars, banners and server images are `UploadPurpose::Avatar`
  uploads (the uploads table's purpose check is not rebuilt). An animated APNG
  is stored with content type `image/apng`.

## REST routes

| Route | Who | Returns |
| --- | --- | --- |
| `GET /api/v1/users/{id}/card?guild=` | anyone who may see the profile | `ProfileCard`: user, pronouns, about me, banner color, created, `look` (banner, colors, effect, frame), shown `badges`, and with `guild` (the viewer must be a member) the member's `server` profile and `member_since`. |
| `GET /api/v1/users/@me/profiles` | self | `MyProfiles`: the global profile and look, every earned badge (shown ones first), the shown IDs, whether they were chosen, and every non-empty server profile. |
| `GET /api/v1/badges/{id}/icon` | signed in | A custom badge's 64x64 PNG. |
| `GET /api/v1/files/{id}` and `/thumbnail` | (existing) | Banners and server avatars are readable by anyone who may see the profile (global) or by members of that server. |

Rate limits: profile commands 30/min (burst 10) per user; the routes share the
REST bucket (120/min, burst 30).

## Limits and validation (server)

| What | Limit |
| --- | --- |
| About me | 190 characters, no control characters except line breaks |
| Pronouns | 40 characters |
| Avatar or banner file | 10 MB; PNG, APNG, JPEG, WebP (still or animated) or GIF; sniffed, never trusted from the name or type |
| Still image | each side up to 4096 px, up to 16 megapixels |
| Animation | each frame up to 1920x1080, up to 500 frames, up to 250 million pixels over all frames |
| Crop | zoom 100–400 %, focal point 0–100 % on each axis |
| Shown badges | 6 |
| Custom badges | 100; name 40 characters, description 120; icon 256 KB, fitted to 64x64 PNG |
| Name colors | any RGB (the owner removed the readability rule on 2026-11-28: how a name reads on the chat is the person's choice); a gradient needs two colors |
| Profile colors | any RGB; the card text and a black scrim are chosen so text reaches 7:1 on both colors (`card_theme`), so its 80 % secondary text still reads |
| Cosmetic IDs | must be in the catalogs below |

Every image is validated by really decoding every frame (`profiles/media.rs`),
one frame at a time, stopping at the first cap exceeded. The upload's
thumbnail becomes a still first-frame rendition: JPEG, transparent pixels over
the avatar color, up to 256x256 (or 960x384 for an image at least twice as
wide as tall, a banner).

**Animation flag.** The validator records, per stored file, whether an upload
moves; `set_image` reads that record. A server restarted between the upload
and its use (at most an hour, the upload's lifetime) falls back to the content
type: GIF and APNG count as moving, WebP as still.

## Behavior

- **Animated avatars.** Lists (messages, member list, DM list, friends, user
  panel, call tiles, search, inbox) show the still rendition. The animation is
  a CSS background that only applies while the row is hovered or focused, so
  the browser fetches and decodes it only then. A profile card plays it. With
  reduced motion it never plays.
- **Cropping.** Choosing a picture opens a crop dialog (drag or arrow keys to
  move, zoom slider or +/-). A still picture is cropped in the browser to
  512x512 (avatar, WebP or PNG) or 1500x600 (banner, JPEG) before upload. An
  animation is uploaded as it is and the crop (`Crop`) is applied by CSS where
  it is shown (object-fit cover, focal point, zoom), so it keeps moving and is
  never re-encoded. The crop preview draws with the same mapping as that CSS.
  The page's CSP allows no `blob:` images, so the preview is a canvas painted
  from `createImageBitmap`.
- **Profile banner.** Still or animated, cropped to 5:2, falls back to the
  banner color.
- **Profile colors.** Primary and accent tint the card as a vertical gradient;
  light colors switch the text to dark, otherwise a scrim is added as needed.
- **Bio.** Rendered with the shared message renderer (`markdown()` in
  `ui.rs`): the Markdown subset, links and emoji. Custom server emoji will
  appear once that renderer draws them; nothing else needs to change here.
- **Per-server profiles.** Nickname, avatar, banner, bio, pronouns, colors,
  name style, decoration, nameplate, effect and frame, each optional. Shown
  wherever the member appears in that server: messages, the member list, the
  profile card. Leaving the server deletes it (a trigger on `guild_members`).
  The editor is in Settings > Profiles > Server Profiles and in the server
  menu (Edit Server Profile), with a server selector and a side-by-side
  preview.
- **Moderation.** On a member's card: Reset Nickname (Manage Nicknames, the
  existing command and audit entry), Remove Server Avatar and Remove Server
  Banner (Time Out Members). The member must be below the moderator. Both ask
  first and are audited.
- **Display name styles.** Six bundled OFL fonts (Script, Pixel, Future,
  Serif, Handwritten, Display; `public/fonts/names/NOTICE.md`), one or two
  colors, and one effect: solid, gradient, glow, outline, or shimmer on hover.
  Fonts are `@font-face` rules, so the browser fetches one only when a name in
  it is on screen; the service worker never precaches them. Without a style a
  name keeps its role color.
- **Badges.** Built in: Instance Administrator (awarded), Server Owner (owns a
  server with 50 or more members), Early Member (account created before the
  cutoff, default 2027-01-01 UTC), Member for N Years (evolves at 1, 2, 3 and
  5 years), Bug Reporter and Contributor (awarded). Computed when a card is
  read; no timers. Up to six shown, in the order chosen (Settings > Profiles >
  Badges); until chosen, the first six earned.
- **Decorations** (14): Halo, Orbit, Sprout, Crown, Cat Ears, Starfield,
  Ember, Frost, Pixel, Prism, Bubbles, Headset, Hearts, Lightning.
  **Effects** (7, profile card only): Fireflies, Snowfall, Aurora, Confetti,
  Rain, Sparkle, Bubbles. **Nameplates** (9, member list and direct messages):
  Dusk, Forest, Ocean, Ember, Candy, Circuit, Stars, Waves, Plaid.
  **Frames** (6): Gilded, Neon, Vine, Pixel, Ribbon, Frost. All are original
  static SVG files (`public/cosmetics/`, generated by a script, see below) or
  CSS. Each is selectable globally and per server, with previews.
- **Viewer settings** (Settings > Appearance, stored on the device):
  "Show plain names" turns off other people's name styles (role colors stay);
  "Reduce cosmetics" hides other people's decorations, nameplates, effects
  and frames. Your own always show.

## Efficiency

- Nothing animates in a list unless that row is hovered or focused. A profile
  card's effect plays two cycles when it opens and again while hovered. All
  motion is `transform`/`opacity` CSS (compositor only); no script drives an
  animation, and reduced motion turns all of it off.
- Artwork, fonts and the editor script are fetched only when shown and are
  not in the service worker's install list. The SVGs are static (no SVG
  animation runs inside an image).
- `UserStyle` is boxed on `UserSummary` and `Member` (it is about 90 bytes),
  so the protocol enums that carry users stay small. A summary without a
  style serializes exactly as before.

## Admin CLI (the instance administrator)

Run against the server's data directory (it may be running):

```
sticktalk-server --data-dir <dir> admin badge list
sticktalk-server --data-dir <dir> admin badge create --name "Helper" --description "Helps out" --icon icon.png
sticktalk-server --data-dir <dir> admin badge award <instance_admin|bug_reporter|contributor|c<ID>> <username>
sticktalk-server --data-dir <dir> admin badge revoke <badge> <username>
sticktalk-server --data-dir <dir> admin badge delete <ID>
sticktalk-server --data-dir <dir> admin badge early-cutoff 2027-01-01
```

## Artwork

The decorations, effects, nameplate patterns and badge icons were drawn for
StickTalk by `crates/web/scripts/cosmetics.py` (simple geometric SVG:
rings, stars, leaves, flames, hearts, a crown, ears, a headset, bolts,
particles). They copy no one else's art. Change the script and run it again
rather than editing the SVG files by hand.
