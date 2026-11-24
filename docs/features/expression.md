# Expression (task `expression`)

Custom server emoji end to end, emoji and stickers across servers and in
direct messages, stickers (server and built-in), Super Reactions and custom
typing indicators. Everything is free for every account and server.

Code: `crates/proto/src/expression.rs` (wire), `crates/server/src/expression.rs`
(+ `expression/library.rs`, `expression/stickers.rs`, `expression/tests.rs`),
`crates/server/src/custom_emoji.rs` (emoji upload and assets),
`crates/web/src/live/expression.rs` and `live/ui/expression.rs` (web),
`crates/web/public/expression.js` (press-and-hold on reaction chips),
`crates/web/styles/expression.css`, `crates/web/public/stickers/*.svg` (the
built-in pack, original artwork). Migration `202611230801_expression.sql`.

## Permissions

| Bit | Name | Grants |
| --- | --- | --- |
| 28 | `MANAGE_EMOJI` (shown as **Manage Expressions**) | Upload, rename and delete emoji **and stickers**. Existing bit. |
| 44 | `USE_EXTERNAL_EMOJI` | Use custom emoji from other servers in this server's channels (messages, reactions, poll answers). |
| 45 | `USE_EXTERNAL_STICKERS` | Send stickers from other servers in this server's channels. |

Bits 44 and 45 are on for `@everyone` by default: the migration adds them to
every existing `@everyone` role, and a trigger adds them to each new one. A
channel or role overwrite can deny them. Owners and Administrators have
everything. Direct messages and group DMs have no such permission: membership
of the source server is enough.

## Limits

- Per server: **250 still + 250 animated** custom emoji (`MAX_GUILD_STATIC_EMOJI`,
  `MAX_GUILD_ANIMATED_EMOJI`; `MAX_GUILD_EMOJI` is now their sum, 500), **60 stickers**.
- Emoji uploads: PNG, JPEG, GIF, WebP, animated GIF/WebP/APNG, at most 256 KB,
  at most 2048×2048, decoded in full (every frame, at most 120 frames and 32 MB
  decoded) and re-encoded at most 128×128 (animated ones as GIF). SVG and
  anything that does not decode are refused.
- Stickers: PNG, APNG, GIF or WebP (still or animated), at most 512 KB, at most
  1024×1024, every frame decoded (at most 600 frames, 256 MB decoded). A still
  image is re-encoded as PNG at most 320×320; an animated one must already be
  at most 320×320 and is stored as sent, with a still first frame beside it.
  Shown at 160×160. Name 2–30 letters, digits, spaces, `_`, `-`; one related
  Unicode emoji; optional description up to 100 characters (alt text).
- Up to **3 stickers** on one message, with optional text. 50 favorite stickers.
- Typing indicator: up to 3 emoji (Unicode, or custom ones from a server you
  are in), a motion (`bounce`, `wave`, `pulse`, `float`, `still`), and a verb:
  `is typing`, `is writing`, or a custom verb of 1–20 letters with single
  spaces, apostrophes or hyphens between letters.
- Super Reactions are unlimited. Normal and Super Reactions together share the
  existing limit of 20 distinct emoji per message.

## Using emoji and stickers across servers

- Anyone may use a server's custom emoji or stickers **anywhere** (other
  servers, DMs, group DMs) while they are a member of that server. The server
  checks it at send time, for messages, edits, reactions, Super Reactions and
  poll answers.
- In a message, a `<:name:id>` the author may not use (not a member, external
  use denied, unknown or deleted) is rewritten to `:name:`. A reaction with
  one is refused (`invalid_request`).
- **Grants.** Using an emoji or sticker in a conversation records it for that
  conversation (`emoji_grants`, `sticker_grants`). The image routes then serve
  it to anyone who can read a conversation it was used in, so recipients who
  are not in the source server see it. Grants are removed with the emoji,
  sticker or channel.
- Deleted emoji degrade to `:name:`: messages keep the token (the name is in
  it) and `deleted_emoji` keeps the last name for reactions. A message keeps a
  copy of each sticker's name and format; once the sticker is deleted the copy
  is marked `deleted` and the app shows its name.

## Wire protocol (additive)

- `Message.extra.stickers: Vec<StickerItem>` and
  `Message.extra.super_reactions: Vec<ReactionSummary>`, both omitted when
  empty. Super Reactions are kept apart from `Message.reactions`, so older apps
  never see them.
- `Command::Expression(ExpressionCommand)`,
  `CommandResult::Expression(ExpressionResult)`,
  `Event::Expression(ExpressionEvent)`: the event is sent **only** to
  connections that sent `ExpressionCommand::Subscribe` (the web app does after
  every Ready).
- `AuditAction::{StickerCreate, StickerUpdate, StickerDelete}` and
  `AuditTarget::Sticker`, appended.
- The web app now sends `Identify.text_features = true`, so it receives
  `GuildEmojiUpdated` (and the folder/slowmode text events, which the shared
  model already handles).

| Command | Who | Notes |
|---|---|---|
| `Subscribe` | anyone | opts this connection in to `Event::Expression` |
| `AddSuperReaction { channel_id, message_id, emoji }` / `RemoveSuperReaction` | Add Reactions | emoji as for reactions; custom emoji stored as `<:emoji:ID>` |
| `SendStickers { channel_id, sticker_ids, content, reply_to, mention_ids }` | Send Messages (+ Use External Stickers for another server's stickers in a server channel) | 1–3 distinct stickers; the ordinary send path (slowmode, mentions, AutoMod, timeouts, DM blocks). Answers `CommandResult::Message`. |
| `UpdateSticker { guild_id, sticker_id, name, emoji, description }` | Manage Expressions | audited |
| `DeleteSticker { guild_id, sticker_id }` | Manage Expressions | audited |
| `SetTypingStyle { style }` | self | `None` restores the standard indicator |
| `SetStickerFavorites { sticker_ids }` | self | built-in stickers or those of servers you are in |

Emoji rename and delete keep using `LaunchCommand::RenameEmoji` / `DeleteEmoji`.

Events: `StickersUpdated { guild_id, stickers }` (to the server's members),
`SuperReactionUpdated(ReactionUpdate)` (one copy per viewer, `me` is about that
viewer; viewers play the burst when `added`), `TypingStyle { user_id, style }`
(sent just before each `Typing` event from someone with a custom indicator,
to the same audience), `SettingsUpdated(ExpressionSettings)` (your other
sessions).

REST (cookie or bearer auth; every `/api` response is `no-store` by the
browser boundary):

| Route | Who | Returns |
|---|---|---|
| `GET /api/v1/users/@me/expression` | self | `ExpressionLibrary`: each server's emoji and stickers, the built-in pack, your settings |
| `GET /api/v1/guilds/{id}/expression` | Manage Expressions | `GuildExpressionAdmin`: emoji with creator and time, stickers, creators, slot maxima |
| `POST /api/v1/guilds/{id}/emoji?name=` (body: image) | Manage Expressions | existing route, new limits and JPEG |
| `GET /api/v1/guilds/{id}/stickers` | member | `Vec<Sticker>` |
| `POST /api/v1/guilds/{id}/stickers?name=&emoji=&description=` (body: image) | Manage Expressions | `Sticker`; audited |
| `GET /api/v1/stickers/{id}`, `/still` | member of its server, or reader of a conversation it was sent in | the image (`nosniff`, `default-src 'none'; sandbox`) |
| `GET /api/v1/emoji/{id}`, `/preview` | member of its server, or reader of a conversation it was used in | existing routes, wider access |
| `GET /api/v1/emoji/resolve?ids=1,2` | as above, per emoji | `Vec<EmojiInfo>`, at most 100 IDs; the server's name and icon only for its members; deleted ones as `available: false` with their name |

## Web app

- **Picker** (`ui/picker.rs`, task `picker-tabs`): one dialog for Emoji
  (`dialog:emoji`), Stickers (`dialog:stickers`), GIFs (`dialog:gif`) and
  Kaomoji (`dialog:kaomoji`). Wherever it is opened (the composer's buttons,
  its More Options menu, a link), it shows the same four tabs in that order:
  a segmented control (`role=tablist`, one tab stop, Left/Right/Home/End move
  and open, the open tab has its own background; 44 px tall on phones). The
  composer's emoji button opens Emoji; each other entry opens its own tab.
  Pressing the open tab again keeps the search and the scroll.
  - **One frame.** The core draws the dialog at its final size (640 px tall,
    or less on a short screen; a bottom sheet of the same height on a phone)
    and the tabs the moment a picker opens. The tabs' contents are one lazily
    loaded module (`load_picker_body`); until it arrives the frame shows the
    search field's outline and nothing else. Switching tabs never changes
    the dialog's size or place: only the list inside scrolls.
  - **Focus.** Opening a tab by pointer puts focus in its search field (not on
    touch screens, where the keyboard would cover the list); moving along the
    tabs with the arrow keys keeps focus on the tabs.
  - **Emoji**: search, then a category bar (Recently Used, each server's emoji
    with its icon, the eight bundled categories with icons and names as
    tooltips) and one list of all of them, as in Discord and Slack. A choice
    in the bar scrolls its section to the top; the bar marks the section at
    the top as the list scrolls (`aria-current`). Recently Used holds custom
    and Unicode emoji (stored on the device); custom emoji insert as `:name:`
    (or the full token when another server's emoji has that name), and
    `:name:` becomes the token when sent, outside code. Searching shows one
    list of matches (custom, then bundled) and hides the bar.
  - **Drawing as it scrolls.** The first category's first 120 emoji are drawn
    at once; every other section is drawn when it comes within two rows of
    view (`IntersectionObserver` on the undrawn part, which keeps the exact
    height of its rows so nothing moves): 96 more, or the whole section when
    it is reached from below after a jump. No scroll polling, no timers; a
    scroll event only re-marks the bar, at most once a frame.
  - **Add Reaction** (`dialog:emoji-react:{message}`, the sparkles button for
    a Super Reaction) is the same frame with the title "Add Reaction" in place
    of the tabs: a reaction is an emoji, so it offers emoji only, with the
    Super Reaction switch, search and the category bar.
  - **Stickers** and **GIFs** are described below and in `content.md`;
    **Kaomoji** lists the text faces by mood.
  - Script: `public/expression.js` (`picker`), loaded when a picker first
    shows. With no picker open it holds no listener on the list and no
    observer.
- **Completion**: `:sti` lists matching custom emoji with their images before
  bundled ones.
- **Rendering**: custom emoji inline in messages, reactions, poll answers and
  the typing indicator; a message of only emoji (up to 27) is jumbo. Clicking a
  custom emoji in a message says which server it is from (or that it is from a
  server you are not in). Emoji not in your servers are resolved once, in
  batches of up to 100, only when they are on screen.
- **Animated emoji and stickers** animate; with reduced motion only their still
  first frame shows until the message or button is hovered or focused (the
  moving image is a lazy image that is not even fetched until then).
- **Stickers** (`dialog:stickers`, the sticker button beside the emoji button):
  built-in pack and each server, search (name, emoji, description), favorites
  (synced), recently used (device), the same category bar as Emoji (Favorites,
  Recently Used, each pack), click to send, `+` to put up to 3 on one message
  with the composer text. Typing an emoji that is a sticker's related
  emoji shows suggestions above the composer.
- **Super Reactions**: the sparkles button on a message, or the Super Reaction
  switch in the reaction picker; on an existing chip, Shift/Alt+click or press
  and hold (touch) uses the other kind. A Super Reaction chip sits beside the
  normal chip of the same emoji with its own count. Viewers see one burst
  (two pseudo-elements animating transform and opacity, 900 ms, once); a
  re-render during it resumes it with a negative delay rather than restarting.
  Reduced motion, or Settings › Expression › "Play Super Reaction animations"
  off (stored on the device), shows a static highlight instead.
- **Typing indicator**: Settings › Expression. Shown to others only when that
  person is the only one typing; otherwise the standard text. The motion is a
  CSS animation on elements that exist only while someone is typing, and none
  under reduced motion.
- **Server settings › Emoji / Stickers** (Manage Expressions): upload, slots
  used, who added each, edit (rename; sticker name, emoji, description) and
  delete.
- The role editor and channel permission editor list Manage Expressions, Use
  External Emoji and Use External Stickers.
- `markdown::text` rendering of emoji is `ui::expression::text(a, text)`, and
  `ui::expression::plain(text)` gives one-line `:name:` text. Thread and forum
  names (another task, not merged yet) should render through these.

## Not included

- AutoMod does not check display names today, so there is no AutoMod check to
  apply to the custom typing verb; the verb is limited to letters instead.
- Uploaded animated stickers are stored as sent (after a full decode and the
  size checks); they are not re-encoded, because the image library cannot
  write APNG or animated WebP.
