# StickTalk UI Spec (authoritative — the lead reviews screenshots against this)

The UI must look like a polished commercial chat app, not a default-iced debug tool.
**Never ship iced's default widget styles.** Every widget gets a style function that uses the tokens below.

## 1. Tokens (define once in `theme.rs`, never hardcode colors/sizes elsewhere)

### Color (dark, default)
| Token | Hex | Use |
|---|---|---|
| bg.rail | #111113 | guild rail |
| bg.sidebar | #17171A | channel list, member list, user panel |
| bg.chat | #1C1C20 | message pane, header |
| bg.elevated | #26262B | composer, inputs, menus, modals, tooltips |
| bg.hover | #232327 | row hover (messages, channels) |
| bg.selected | #2E2E34 | selected channel/friend row |
| divider | #2A2A2F | 1px separators only (header bottom, sidebar edges) |
| text.primary | #E8EAED | body, names |
| text.secondary | #A8A8B0 | channel names (read), labels |
| text.muted | #74747C | timestamps, placeholders, hints |
| accent | #3DDC97 | primary buttons, focus ring, links, selected pill, own-mention |
| accent.hover | #5BE5AA | |
| accent.text | #08170F | text on accent |
| danger | #F25F5C | delete, kick/ban, DND, "new messages" divider |
| warning | #F2B84B | idle, mention highlight bar |
| mention.bg | #F2B84B @ 8% alpha | message that mentions me |
| online / idle / dnd / offline | #3DDC97 / #F2B84B / #F25F5C / #74747C | presence dots |

### Spacing scale: 4, 8, 12, 16, 24, 32 — nothing else.
### Radius: 4 (badges, small chips), 8 (buttons, inputs, composer, cards), 12 (modals), full circle (avatars, guild icons when not selected; selected guild icon = radius 16 "squircle").
### Typography: bundle **Inter** (OFL, subset Latin + punctuation) as the only UI font.
| Role | Size | Weight |
|---|---|---|
| Modal/page title | 20 | Semibold |
| Section header (CATEGORY names) | 12, uppercase, +0.5 letter-spacing if supported | Semibold, text.muted |
| Username in message | 15 | Semibold |
| Message body | 15 | Regular, line-height ~1.375 |
| Channel/friend row | 15 | Medium |
| Timestamp, hints, badges | 12 | Regular/Semibold for badges |
Emoji: bundle a compact color emoji font or render twemoji-style images (CC-BY, credit in About). No tofu boxes.

### Icons: use **Lucide** (ISC license) SVGs or an icon-font subset — one consistent set, 20px in headers/composer, 16px inline. **No emoji or unicode symbols as UI icons.**

## 2. Layout (window min 940×600)
```
| rail 72 | sidebar 240 | chat (flex)                         | members 240 |
|         | guild header 48 | channel header 48 (# name | topic | icons) |
|  48px   | categories +    | messages (virtualized)                     | grouped by
|  icons, | channels        |                                            | presence
|  8 gap  |                 | composer (margin 16, min h 44, radius 8)   |
|         | user panel 52   |                                            |
```
- Member list auto-hides below 1100px window width (toggle button in header).
- Rail: DM/home button at top, 2px×32px divider, guild icons, "+" (create/join) and nothing else. Selected guild: 4px-wide **white** (text.primary) pill on the left edge, 40px tall; hover: 20px pill; unread: 8px dot. Tooltip with guild name on hover, shown to the right.
- Sidebar header (48): guild name Semibold 15 + chevron → dropdown (invite, settings, create channel, leave).
- Channel row: 32px tall, radius 4, 8px horizontal inner padding, Lucide `hash` icon 18px muted. Read = text.secondary; unread = text.primary Semibold + 8px white pill on the rail-side edge; selected = bg.selected + text.primary; mention badge = danger pill with count, right-aligned.
- User panel (52, bg slightly darker than sidebar #141417): 32px avatar with presence dot, display name 14 Semibold + status text 12 muted (ellipsized), icon buttons (settings) right-aligned.

## 3. Messages
- Group consecutive messages by the same author within 7 minutes. Group head: 40px avatar, name (15 Semibold, colored text.primary) + timestamp (12 muted, "Today at 4:12 PM"/"Yesterday at…"/date). Continuations: no avatar/name, hover shows short time ("4:13 PM") in the avatar gutter.
- Spacing: group top margin 16; continuation gap 2; left padding 16; avatar→text gap 16; content max width = pane width − 72.
- Hover row: bg.hover; action bar floats top-right (elevated bg, radius 8, 1px divider border, icons: react, reply, edit*, delete*, *own/permitted only).
- Reply: small 12px line above body: curved connector + 16px avatar + name + truncated quoted text, clickable.
- "(edited)" suffix 12 muted. Mentions: chip with accent @ 15% bg, accent text; message mentioning me gets mention.bg + 2px warning left bar.
- Reactions: chips below the body, radius 8, bg.elevated, 4/8 padding, emoji + count; my reaction = accent @ 15% bg + 1px accent border. "+" add chip on hover.
- Attachments: image at its own size, fitted into 550×350 and never enlarged; one that would end up a thin strip (a wide screenshot) may run wider, up to the message column, until it is 200 tall. A file too heavy to draw in the list (over 4 MB, or over 3840×2160 pixels) shows its 256-pixel thumbnail at thumbnail size. Radius 8, preserved aspect, placeholder box of exact final size while loading (no layout jump). Files: card with icon, name, size, download button.
- Date separator: centered 12 muted text with divider lines either side. "NEW" divider: 1px danger line with small "NEW" badge right.
- Typing indicator: 24px strip under the composer: "Name is typing…" / "A and B are typing…" / "Several people are typing…". Static three dots — no animation loop.
- "Jump to present" pill appears bottom-center when scrolled up > 1 screen.

## 4. Composer
- bg.elevated, radius 8, inner padding 12 / 16. Left: `plus-circle` (upload). Right: emoji picker button. Placeholder "Message #channel" / "Message @name" in text.muted. Grows with content to max 50% of pane height then scrolls.
- Pending attachments shown above input as 96px cards with remove ×.
- @mention autocomplete popup above composer: elevated card, radius 8, rows 36px with 24px avatar + name + muted username, keyboard ↑↓ Enter/Tab, Esc closes.
- Editing a message inline: same composer style inside the message, hint "escape to cancel • enter to save" in 12 muted.

## 5. Every interactive element has states
hover, pressed, selected/active, disabled (40% opacity), keyboard focus (2px accent ring). Pointer cursor on clickables. Buttons: Primary (accent bg, accent.text, 8 radius, h 38, padding 16), Secondary (bg.elevated), Danger (danger bg, white text), Link (accent text). Inputs: bg #121215, radius 8, h 40, focus border accent.

## 6. Screens that must exist and look finished
Login / Register (centered 480px card on bg.rail with subtle brand mark, server address field with helper text, inline field errors), Home (friends tabs: Online / All / Pending / Blocked + "Add Friend" accent button; DM list in sidebar), DM chat, Guild chat, Create/Join guild modal (two big choice cards), Invite modal (copyable code with copy button + "Copied!" feedback), Guild settings (left nav list + content pane, like a settings page — Overview / Channels / Members / Invites / Bans), User settings (same pattern — My Account / Profile / Appearance placeholder / Log Out in danger), context menus (elevated, radius 8, 32px rows, danger items red), toasts (bottom-center, elevated, auto-dismiss via a single one-shot timer).
- Empty states with an icon + one line + action: no friends, no DMs, empty channel ("This is the beginning of #general").
- Loading: static skeleton blocks in bg.elevated (no shimmer animation — keeps CPU at 0).
- Disconnected: thin banner under the header "Reconnecting…" (warning) → disappears on resume.

## 7. Anti-patterns — reject on sight
Default iced button/text_input/scrollbar looks; boxes and borders around everything; inconsistent paddings; text sizes outside the table; unaligned baselines between avatar/name/timestamp; text overflowing or clipping instead of ellipsis; full-width stretched buttons on wide screens; centered body text; placeholder/lorem copy; emoji used as icons; visible scrollbars thicker than 8px (use thin 4px rounded thumb, no track, shows on hover); white flashes on startup; layout jumps when images load; modals without a dimmed backdrop (#000 @ 60%).

## 7b. Copy / wording rules
- Users call guilds **"servers"**. Never "community" in UI text. Examples: "Create a server", "Server Settings", "Invite people to <name>".
- Copy is plain and functional, Discord-level terse. No taglines, slogans or poetic filler ("Your next conversation is waiting", "Good company makes a community", "A little closer"). Headings say what the page is; helper text only when it helps a user act.
- No developer/implementation details in the UI (e.g. "stored in Windows Credential Manager", raw localhost URLs as defaults).
- Never insert manual line breaks into prose; let text wrap to its container width.

## 8. Self-review loop (required before reporting done)
1. Render each screen in §6 at **1280×800** and **1920×1080** with realistic mock data (varied name lengths, long messages, a code-ish line, images, reactions, replies, mentions, 3+ guilds, unread + mention badges, 2 people typing).
2. Save PNGs to `crates/client/screenshots/<screen>-<w>x<h>.png`.
3. Open each screenshot yourself and check it against every rule in §1–§7; fix and re-shoot. List any rule you knowingly deviated from and why.
