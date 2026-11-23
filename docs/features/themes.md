# Themes, appearance and accessibility (task `themes`)

Light, dark, darker and system themes; 22 preset color themes; custom themes
with an editor, sharing and account sync; server themes; the appearance and
accessibility controls; text-to-speech; Streamer Mode; and alternative app
icons. Everything is free for every user and server.

Code:
- `crates/proto/src/themes.rs`: the theme engine (palettes, presets,
  derivation with enforced contrast, share codes, validation) and the wire
  contracts. Used by the server to validate and by the web app to paint.
- `crates/server/src/themes.rs` (+ `themes/tests.rs`), migration
  `202611231401_themes.sql`.
- Web: `crates/web/src/live/themes.rs` (behavior), `src/live/ui/themes.rs`
  (views), `public/preferences.js` (every per-device preference, Streamer
  Mode, app icon, GIF autoplay), `public/theme-boot.js` (before first paint),
  `public/speech.js` (speech synthesis), `styles/themes.css`,
  `public/fonts/chat/` (OpenDyslexic, Atkinson Hyperlegible, OFL),
  `public/app-icons/` (drawn by `scripts/app-icons.py`),
  `scripts/theme-lint.mjs` (+ `theme-lint-allow.json`).
- Windows app: `crates/desktop/src/appearance.rs`.
- Browser test: `node crates/web/tests/themes.mjs` (needs `CARGO_TARGET_DIR`
  with `web-dist`, `STICKTALK_SERVER_EXE`; screenshots to `STICKTALK_QA_DIR`).

## Theme-correct surfaces: the checklist for new UI

A surface added after this work is right in every theme when it follows these
rules. Run `node crates/web/scripts/theme-lint.mjs` after each merge: it exits
non-zero on any color written into `styles/*.css`, `src/live/**/*.rs`,
`public/*.js` or `index.html` instead of taken from a token.

1. **Colors come only from tokens.** Use the Tailwind theme names
   (`bg-background`, `bg-sidebar`, `bg-card`, `bg-popover`, `bg-muted`,
   `bg-accent`, `bg-secondary`, `bg-field`, `bg-panel`, `bg-rail`,
   `text-foreground`, `text-subtle`, `text-muted-foreground`, `text-primary`,
   `text-destructive`, `text-warning`, `border`, `bg-online`, `bg-idle`,
   `bg-dnd`, `bg-offline`, ...) or `var(--bg-chat)`, `var(--text-primary)`,
   etc. in CSS. Never `#hex`, `rgb()`, `white`/`black` or Tailwind palette
   colors (`bg-zinc-800`) for UI chrome.
2. **Text on accent, danger or warning fills uses `text-primary-foreground`**
   (`--accent-text`). It is dark on the default theme and white on light ones;
   every theme guarantees 4.5:1 for it on all three fills.
3. **Text on a surface uses `text-foreground`, `text-subtle` or
   `text-muted-foreground`.** All three are AA on every surface token, in every
   theme (including each gradient stop). Don't lower their opacity.
4. **Large surfaces use the surface tokens, not new colors.** `bg-rail`,
   `bg-sidebar`/`bg-card` and `bg-background` also carry a theme's gradient;
   a new full-height pane on one of them gets it for free.
5. **Content is not chrome.** Photos, video letterboxes (`bg-black`), scrims
   (`bg-black/60`), shadows, cosmetics and confetti may keep fixed colors; wrap
   CSS artwork in `/* theme-lint: artwork */ ... /* theme-lint: end */`, and
   add anything else to `theme-lint-allow.json` with a reason.
6. **A person's color (role, name style) is drawn with `class="role-color"`
   and `style` from `ui::themes::role_style(...)`**, so light themes,
   saturation and "role colors as a dot / off" apply to it.
7. **Motion:** every animation and transition must stop under
   `:root[data-motion="reduce"]` (a global rule already shortens them all;
   don't fight it with `!important`). Never use `@media
   (prefers-reduced-motion)` directly: the setting can override the device.
8. **Don't rename tokens or the component classes** (`button primary`,
   `icon-button`, `field`, `live-checkbox`, `live-modal`, `menu`, ...).
9. **Check it:** `tests/themes.mjs` has an `audit(page, label)` function that
   measures every visible text element against what is painted behind it; use
   it on the new surface in Light, Darker and a gradient preset.

## Tokens and how a theme is applied

- `docs/UI_SPEC.md` has three palette tables: dark (default), light (addition)
  and darker (addition). `scripts/finalize.mjs` turns each into a block of
  custom properties (`:root`, `[data-theme-base="light"]`,
  `[data-theme-base="darker"]`, and the light block again inside
  `@media (prefers-color-scheme: light)` for `[data-theme-base="system"]`).
  Five tokens the web app used to hard-code were added to the dark table
  with their existing values (`bg.panel`, `bg.field`, `bg.control`,
  `bg.control-hover`, `brand.avatar`), so the default dark theme is
  pixel-identical; `styles/app.css` now maps the shadcn names onto them.
- **The static bases** (Dark, Light, Darker, Sync with system) are drawn by
  that sheet alone: choosing one sets one attribute. "Sync with system" follows
  the OS setting live through the media query, with no script running.
- **Everything else** (a preset, a custom or shared theme, a server theme, or
  a base with saturation or contrast boost) is derived in Rust by
  `sticktalk_proto::derive` and handed to `preferences.js`, which puts the
  declarations into one constructed style sheet (`:root:root:root{...}`).
  Changing a theme is a sheet swap: a style recalculation, no re-render, no
  layout (only colors change). The test checks the page is not rebuilt.
- **Before first paint:** `preferences.js` keeps a small snapshot (attributes,
  custom properties, the derived declarations, the theme color and the app
  icon) in `localStorage` (`sticktalk-boot`). `public/theme-boot.js`, a
  blocking script in `<head>`, replays it before the login page or the
  skeleton is painted, so there is no flash. It refuses values that could
  break out of a declaration.
- `<meta name="theme-color">` follows the rail color of what is shown. The
  Windows app's title bar follows too (see below).

## Derivation (`crates/proto/src/themes.rs`)

A theme is `ThemeSpec { base: dark|light, accent, colors: 0..5, angle,
intensity }`. From the base palette:
1. Surfaces keep their lightness (so the base's hierarchy and contrast hold)
   and take the hue and some chroma of the colors' average (OKLCH), scaled by
   intensity. The three large surfaces get a gradient, one stop per color.
2. Saturation (Accessibility) scales the chroma of accents, status colors and
   tints.
3. Every requirement below is enforced by moving lightness until it is met:
   text primary/secondary/muted on every surface and every gradient stop
   (4.5:1; 7:1 with contrast boost), accent as link text (4.5:1) and as the
   focus ring on the selected row (3:1), mention chip text (4.5:1), the ink
   on accent/danger/warning fills (4.5:1), danger and warning as text (4.5:1),
   presence dots (3:1), the switch knob on its track (3:1), text on the
   mention highlight. If the chosen accent had to move, the editor says so
   and offers the adjusted color; if something can't be met (it never
   happens for valid input in the tests, which include hostile colors), it
   warns.

### Contrast report

`cargo test -p sticktalk-proto themes -- --nocapture` fails if any theme is
below its target and prints the worst ratio of each requirement. Every pair
listed above is checked for the 3 bases, the 22 presets, both again with
contrast boost and low saturation, and 15 hostile custom themes. Excerpt
(worst case over all surfaces; "margin" is the smallest ratio ÷ target):

| Theme | text.primary | text.secondary | text.muted | accent (links) | accent.text | danger | status dots | margin |
|---|---|---|---|---|---|---|---|---|
| Dark | 7.88 | 5.71 | 5.21 | 8.52 | 10.43 | 4.71 | 3.25 | 1.05x |
| Light | 8.72 | 7.23 | 5.12 | 6.04 | 7.63 | 5.05 | 3.09 | 1.03x |
| Darker | 9.36 | 7.04 | 6.41 | 10.23 | 10.43 | 5.65 | 3.90 | 1.26x |
| Aurora | 7.87 | 5.78 | 5.26 | 9.96 | 12.09 | 4.75 | 3.28 | 1.06x |
| Midnight Blue | 7.87 | 5.74 | 5.27 | 5.94 | 7.25 | 4.72 | 3.26 | 1.04x |
| Ember | 7.99 | 5.76 | 5.27 | 6.76 | 8.14 | 4.78 | 3.30 | 1.06x |
| Grape | 8.05 | 5.77 | 5.27 | 5.85 | 6.97 | 4.83 | 3.34 | 1.03x |
| Forest | 7.72 | 5.67 | 5.16 | 8.46 | 10.57 | 4.61 | 3.18 | 1.02x |
| Crimson Dusk | 8.07 | 5.80 | 5.29 | 5.73 | 6.85 | 4.83 | 3.33 | 1.01x |
| Ocean Deep | 7.82 | 5.64 | 5.16 | 8.27 | 10.20 | 4.67 | 3.23 | 1.04x |
| Sunset | 8.02 | 5.74 | 5.24 | 9.05 | 10.93 | 4.77 | 3.29 | 1.06x |
| Steel | 7.89 | 5.70 | 5.24 | 8.32 | 10.22 | 4.69 | 3.24 | 1.04x |
| Neon Night | 7.84 | 5.76 | 5.23 | 8.59 | 10.47 | 4.72 | 3.26 | 1.05x |
| Coffee | 7.93 | 5.74 | 5.23 | 7.71 | 9.40 | 4.72 | 3.26 | 1.05x |
| Galaxy | 7.91 | 5.76 | 5.24 | 7.59 | 9.24 | 4.73 | 3.26 | 1.05x |
| Daylight Mint | 8.79 | 7.22 | 5.10 | 4.90 | 6.13 | 5.10 | 3.12 | 1.01x |
| Sky | 8.72 | 7.26 | 5.09 | 5.29 | 6.70 | 5.06 | 3.10 | 1.03x |
| Peach | 8.67 | 7.21 | 5.08 | 5.00 | 6.35 | 5.02 | 3.07 | 1.02x |
| Lavender | 8.64 | 7.21 | 5.08 | 5.57 | 7.10 | 5.03 | 3.06 | 1.02x |
| Rose Garden | 8.61 | 7.21 | 5.13 | 5.14 | 6.58 | 4.99 | 3.07 | 1.01x |
| Lemonade | 8.73 | 7.27 | 5.11 | 4.94 | 6.22 | 5.07 | 3.10 | 1.03x |
| Sage | 8.79 | 7.20 | 5.07 | 5.60 | 7.08 | 5.08 | 3.11 | 1.04x |
| Cotton Candy | 8.65 | 7.20 | 5.07 | 6.14 | 7.88 | 5.02 | 3.07 | 1.02x |
| Sand | 8.66 | 7.28 | 5.14 | 5.60 | 7.09 | 5.03 | 3.10 | 1.03x |
| Glacier | 8.71 | 7.19 | 5.13 | 5.02 | 6.36 | 5.08 | 3.11 | 1.04x |

The browser test also measures the text actually rendered (every visible
text node against the nearest opaque background):

| Theme (in the browser) | Text elements measured | Lowest ratio | Below AA |
|---|---|---|---|
| dark | 32 | 4.83 | 0 |
| light | 32 | 5.21 | 0 |
| darker | 32 | 4.83 | 0 |
| preset aurora | 32 | 4.82 | 0 |
| preset sky | 32 | 5.18 | 0 |
| preset midnight | 32 | 4.82 | 0 |
| preset ember | 32 | 4.79 | 0 |
| preset neon | 32 | 4.83 | 0 |
| preset peach | 32 | 5.18 | 0 |
| preset lemonade | 32 | 5.17 | 0 |
| preset glacier | 32 | 5.23 | 0 |
| custom theme | 32 | 4.82 | 0 |
| server theme ocean | 32 | 4.85 | 0 |
| contrast boost | 32 | 5.82 | 0 |

## Presets

Dark-based: Aurora, Midnight Blue, Ember, Grape, Forest, Crimson Dusk, Ocean
Deep, Sunset, Steel, Neon Night, Coffee, Galaxy. Light-based: Daylight Mint,
Sky, Peach, Lavender, Rose Garden, Lemonade, Sage, Cotton Candy, Sand,
Glacier. Each is two or three key colors plus an accent (`PRESETS`); ids are
stable because servers save them.

## Wire protocol (all additive)

- `Command::Appearance(AppearanceCommand)`, `CommandResult::Appearance(
  AppearanceResult::Library)`, `Event::Appearance(AppearanceEvent)`. The event
  goes only to streams that sent `AppearanceCommand::Subscribe` (the web app
  does after every Ready; the answer is the library, no REST request).
- `AppearanceEvent::Tts { channel_id, message_id }`: sent right after a
  `/tts` message's `MessageCreated`, to the same people (not the sender),
  through the same privacy gate (hidden message requests, age-restricted
  channels). A flag on `MessageExtra` was tried first and dropped: it made
  every `Message`, `Event` and `CommandResult` 8 bytes larger, which tipped a
  deep debug-build test over its stack.
- `Permissions::SEND_TTS_MESSAGES` = bit 56, on for `@everyone` (the migration
  updates existing servers; a trigger covers new ones). It is listed as "Send
  Text-to-Speech Messages" in the role editor and the channel permission
  editor (Text group).

| Command | Who | Notes |
|---|---|---|
| `Subscribe` | anyone | answers `Library { themes, settings, servers }` |
| `SaveTheme { id?, name, spec }` | self | create (`id` none) or replace; name 1–32 chars, no control characters; spec validated; 20 per account (`limit_exceeded`) |
| `DeleteTheme { id }` | self | `not_found` for anyone else's |
| `SetServerTheme { guild_id, theme? }` | Manage Server | `Preset(id)` (must exist) or `Custom(spec)`; none removes; members get `ServerTheme` |
| `SetSettings { server_themes, server_overrides }` | self | per-server overrides only for servers you are in (others dropped), at most 200, no duplicates |
| `SendTts(MessageDraft)` | Send Messages + Send TTS Messages | the ordinary send path; text required; 10/min burst 3 |

Theme commands share 30/min burst 10 per user. Tables: `user_themes`,
`guild_themes`, `theme_settings` (all deleted with their user or server).

## Features

1. **Themes.** Settings › Appearance › Theme: Dark, Light, Darker, Sync with
   system. Applied before first paint; the PWA theme color and the Windows
   title bar follow.
2. **Color themes.** 22 presets in a picker of miniature windows drawn with
   each theme's own tokens.
3. **Custom themes.** Create Theme / Customize This Theme opens the editor:
   base, accent, up to five background colors, direction, intensity, live
   preview, contrast warnings with a one-click fix, Try It (apply without
   saving), Save (synced to every device), and a share code with Copy Link.
   A pasted code or an opened `/?theme=<code>` link shows a preview first (Use
   Theme, Save to My Themes, Customize); the code is removed from the address.
   Codes are base64url of a versioned binary form, at most 256 characters,
   and every field is validated as untrusted input (names lose control
   characters and are cut to 32 characters).
4. **Server themes.** Server Settings › Theme (Manage Server): a preset, one of
   your custom themes, or Customize. Members see it only while that server is
   in view. Appearance › Server Themes: a default and a per-server choice
   (use my default / show / keep my own), synced to the account.
5. **Appearance and accessibility** (per device, `preferences.js`).
   - Appearance: message display, chat font scaling, chat font (Inter,
     System, Serif, Monospace, Atkinson Hyperlegible, OpenDyslexic; the two
     bundled fonts are fetched only when chosen and are never precached),
     space between messages and between groups, zoom level (CSS `zoom`),
     show send button (with a keyboard; touch screens always show it),
     website previews, and the profile toggles (plain names, reduce
     cosmetics).
   - Accessibility: saturation (role colors, name styles, theme accents and
     status colors; optionally color and custom themes too), increase
     contrast (AAA text, stronger dividers), role colors in names / as a dot /
     off, always underline links, always show focus, reduced motion (follow
     device / on / off; every animation and transition honors it), Super
     Reaction animations (moved here from Expression), and when to play GIFs,
     animated emoji, stickers, animated avatars and profile effects (always /
     on hover or focus / never).
   - Role names stay readable on light themes: CSS keeps the role color's
     lightness in the theme's readable range (`--name-l-min/max`) with
     relative color syntax; the default dark theme is untouched.
6. **Text-to-speech.** "Speak Message" in the message menu; `/tts <text>`
   sends a message read aloud to the people viewing that conversation (not
   the sender) if they allow it ("Allow playback of /tts messages"); read
   notifications aloud for the conversation in view or for all conversations;
   speech rate with a preview. Nothing is read in Do Not Disturb or while
   deafened in a call; a backlog over four utterances is dropped. Uses the
   browser's speech synthesis; tests replace it with
   `globalThis.__sticktalkSpeech`.
7. **Streamer Mode.** Settings › Streamer Mode, or Ctrl+Alt+Shift+S. While on,
   each individually: hides account details (email, sign-in methods,
   username in My Account), invite links and codes everywhere (also invite
   links in messages), personal notes, message request contents; turns off
   notification popups and sounds; blurs or hides message previews in the
   inbox. A hidden value is revealed by clicking it, once. "Turn on while I
   share my screen" turns it on when a share starts in a call and back off
   when it stops (or the call ends), unless you changed it meanwhile. A small
   "Streamer Mode" pill shows while it is on (and opens the settings).
8. **App icons.** Twelve original color and style variants of the StickTalk
   mark (`scripts/app-icons.py` draws them from simple shapes: Mint, Ocean,
   Violet, Rose, Amber, Graphite, Snow, Sunset, Aurora, Outline, Midnight,
   Lime). Choosing one changes at once the in-app logo, the favicon, the
   `apple-touch-icon` link and the page's manifest link (each icon has its
   own manifest with the same `id`), and before first paint after a reload.

### What each platform does with the app icon

| Where | What changes | When |
|---|---|---|
| Browser tab, in-app logo | the chosen icon | at once |
| Windows app (`crates/desktop`) | window and taskbar button icon (`WM_SETICON` through tao) | at once, no restart, no admin rights, no new window. A **pinned** taskbar shortcut and the Start menu keep the installed icon (Windows takes those from the shortcut file, which is not rewritten). |
| Installed PWA, Chrome/Edge desktop and Android | the manifest link changes; Chrome checks an installed app's manifest when it is launched and, for an icon change, asks the user to confirm the update. | after the browser's own update check and the user's consent; StickTalk cannot force it |
| Installed PWA, iOS/iPadOS home screen | nothing: Safari copies the icon when the app is added and never updates it. Remove and add the app again to change it. | never |
| Firefox (no installable PWAs on desktop) | the tab icon only | at once |

### Windows app

The page tells the shell (`window.ipc`, only from the server's pages, every
message validated in `appearance.rs`): `{"shell":"theme","dark":bool,
"caption":"#rrggbb"}` sets `DWMWA_USE_IMMERSIVE_DARK_MODE`, the caption color
and the caption text color (Windows 11; older Windows ignores the colors);
`{"shell":"icon","size":32,"rgba":[...]}` (16–64 px square, exact length) sets
the window icon. The page draws the icon into a canvas and sends its pixels,
so the shell needs no image decoder. `__sticktalkHost` now advertises
`theme` and `icon`.

## Efficiency

- No timers, no polling. The OS theme and reduced-motion settings are
  followed through media query `change` events; GIF autoplay through `play`,
  pointer and focus events.
- A theme change is one style sheet swap. Presets' picker swatches are
  derived once per session. Server-theme sync is a string comparison per
  render and calls into JS only when the shown theme changes.
- Speech synthesis, the chat fonts and the app icons load only when used; the
  service worker does not precache `/public/app-icons/` or `/public/fonts/chat/`.

## Not included / notes for the owner

- Send Text-to-Speech Messages defaults to on for everyone (as Discord's
  does); a server can turn it off per role or channel.
- Custom themes are not part of "Your Data" exports yet.
- Changing a server theme is not written to the audit log.
- iOS cannot change an installed home-screen icon (see the table above).
