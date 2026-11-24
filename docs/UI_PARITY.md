# UI parity spec: screen-by-screen, from the owner's real Discord screenshots

The lead derived this from screenshots of the owner's Discord client (Nov 2026). **It extends `docs/UI_SPEC.md`.** Tokens, the mint accent, and the copy rules there still apply.
- Wherever Discord uses blurple, we use **accent** (mint): primary buttons, toggles, active-tab underline, links, mention chips (accent at 20%), and the "Reveal" links.
- Never copy Discord's logo, artwork, Nitro or Boost branding, or wording that names Discord features we don't have.

Goal: **as good as or better than Discord.** It must look dense, calm and precise. Every pixel decision below is deliberate. When unsure, match Discord's proportions.

---

## 0. Global patterns (used everywhere)
- **Tooltip.** bg `#111214`, text.primary, 14 Medium, padding 8/12, radius 6, small 8 px arrow pointing at the target. It appears **above** the target after a 400 ms hover delay, instantly when moving between adjacent targets, and is never animated. Every icon-only button has one ("Deafen", "User Settings", "Invite to Join", "Member List", "Pinned Messages"…).
- **Icon button.** 32×32 hit area, 20 px icon, text.secondary. Hover: text.primary plus bg.hover with radius 6. Active/toggled: text.primary plus bg.selected.
- **Split button** (mic, headphones): icon button plus an attached 16 px-wide chevron segment that opens a device menu. The two segments share one hover background.
- **Dropdown menu** (server menu, context menus):
  - bg `#111214`, radius 8, padding 6, min-width 220, 1px divider border, shadow.
  - Rows 32 h, radius 4, padding 0/8. Label is 14 Medium, with an optional 18 px icon **right-aligned**.
  - Hover: accent bg with accent.text. Danger rows: danger text, and on hover danger bg with white text.
  - Separators: 1px divider with 4 px vertical margin.
  - Checkbox rows have the checkbox right-aligned.
- **Toggle switch.** 40×24, radius 12. Off: track `#72767d` at 60% with a white knob on the left. On: accent track with a white knob on the right and a small ✓ in the knob. Click target is the whole row.
- **Secondary button** ("Edit"): bg `#3A3A41`, hover `#45454D`, text.primary 14 Medium, h 38, min-w 96, radius 8.
- **Row-action buttons** in tables (pencil, "…"): 40×40 squares, radius 8, bg `#2B2B30`, hover `#3A3A41`, 20 px icon.
- **Section header inside settings content:** 20 Semibold for a page section ("Account Info", "Password & Security"), and 12 Semibold uppercase muted for small labels. 1px divider between sections, with 32 px vertical margin.
- **Avatar with status.** The status dot sits in a **cut-out ring** in the parent background color: the ring is 3 px at 32 px avatars and 5 px at 80 px. This is a mask, not a dot drawn on top.
  - Online: filled dot.
  - Idle: crescent moon.
  - DND: dot with a horizontal bar.
  - Offline/invisible: hollow ring.
  - Streaming (later): purple.
- **Role colors.** A member's name renders in the color of their highest colored role, everywhere: messages, member list, mentions in their color tint, popouts. With no colored role, the name uses text.primary.
- **Owner crown.** A 14 px gold crown after the owner's name in the member list.

## 1. Main window
### 1.1 Server rail (72 px)
- Home button (our logo) at the top. Under it, a 32×2 divider, then server icons.
- Icons are 48 px. Default shape is a circle. Hover and selected shape is a **squircle** with radius 16. The shape change is instant, with no animation loop.
- Left pill indicator: 8 px dot when unread, 20 px tall on hover, 40 px tall when selected. White, 4 px wide, rounded on the right side.
- Mention badge: red pill bottom-right of the icon, with a white count, and a 3 px ring in the rail color.
- Server **folders**, later: a stacked mini-grid of 4 icons.
- At the bottom: "+" (Add a Server), then "Explore" is **omitted**.

### 1.2 Channel sidebar (240 px)
- **Header**, 48 h: server name 16 Semibold, then a chevron that flips when the menu is open, then right-aligned icon buttons: "Invite People" (user-plus) and, later, an optional banner.
  - Click opens the **server menu** (§0 dropdown), with these items:
    - Invite to Server
    - Server Settings
    - Create Channel
    - Create Category
    - Create Event (later)
    - separator
    - Notification Settings
    - Privacy Settings
    - separator
    - Edit Per-server Profile
    - Hide Muted Channels ☐
    - separator
    - Leave Server (danger, non-owner only)
  - Omit Server Boost and App Directory.
- **Top shortcuts** (later): Events, then a divider.
- **Category header:** "Text Channels ⌄", 12 Semibold, text.muted, not uppercase. Hover makes it text.primary. A "+" on the right appears on hover with the tooltip "Create Channel". Clicking it collapses the category; collapsed categories still show unread and selected channels.
- **Channel row,** 34 h, radius 6, margin 0/8:
  - Icon 20 px: `#` for text, a speaker for voice. A locked variant marks private channels.
  - Name 16 Medium, ellipsized.
  - On hover or selected, show right-aligned icon buttons "Create Invite" (user-plus) and "Edit Channel" (gear). They're hidden otherwise.
  - Muted channels: 40% opacity.
- **Nested rows:** any channel or category can sit inside any other, to any depth. Each level moves its rows in 12 px, up to 8 levels (96 px); deeper rows keep that indent so names stay readable. At every level channels are listed before categories. A nested category header has 8 px above it instead of 16. People who manage channels get a "+" ("Create channel in …") on a channel row on hover, focus or when it is selected, as on category headers. They can also drag any row to reorder it, nest it or take it out to the top level (`docs/features/channel-drag.md`); Move Channel in the channel's options menu is the keyboard, screen-reader and touch path.
- **Voice channel row,** with the same base styling. Connected users are listed under it, indented 36 px:
  - Rows 28 h: 24 px avatar, name 14, and right-aligned state icons (muted mic, deafened headphones, red **LIVE** pill when streaming, camera icon when video is on).
  - A speaking user gets a 2 px accent ring around their avatar. It toggles on voice activity events, with no animation loop.
- **Voice Connected panel,** above the user panel, shown only while in a call:
  - "Voice Connected" in 14 Semibold online-green with a signal-strength icon (tooltip shows ping).
  - Below it, "<channel> / <server>" in 12 muted. Clicking it jumps to the channel.
  - Right: a disconnect button (phone-down icon, hover danger).
  - Below: a 4-button row of 32 h tiles with bg.hover: Camera, Share Screen, Activities (later), Soundboard (later).
  - While streaming, a "Screen 1" row with the stream title and a stop button.
- **Game Activity panel,** later, above the Voice panel: game icon, game name linked, and buttons "Invite to Join" and "Stream <Game>". See §6 activity detection.
- **User panel,** 52 h, bg `#141417`, radius 8, margin 8:
  - Avatar 32 with status.
  - Display name 14 Semibold, ellipsized.
  - Subline 12 muted that **alternates on hover**: status text by default, @username on hover.
  - Right: split mic, split headphones, gear. Muted mic shows the slashed icon in **danger** color with a danger-tinted bg.

### 1.3 Channel header (48 h)
- Left: `#` icon, the name 16 Semibold, a divider, then the topic in 14 muted, ellipsized. Clicking the topic opens the full topic.
- Right, in order:
  - Threads (later)
  - Notification Settings (bell; slashed when muted)
  - Pinned Messages
  - Member List toggle (accent when shown)
  - **Search box**: 240 w, collapsing to 144 when narrow, placeholder "Search <server>", search icon right, bg `#121215`, radius 6, h 28
  - Inbox (later)

### 1.4 Messages
Same as UI_SPEC §3, plus:
- **Name line:** name in role color, 16 Medium. Then the optional **server tag chip** (later): small icon plus 4-letter tag, bg `#2B2B30`, radius 4, 11 Semibold. Then the **APP** badge for bots (accent bg, 10 Bold, radius 3). Then the timestamp, 12 muted.
- **Image attachments:** max 550×350, radius 8, click opens the viewer (BACKLOG B1). Spoilered images are blurred, with a "SPOILER" pill.
- **Mention chip:** accent at 20% bg with accent text, radius 3. Hover is 40% bg. Clicking opens the user's profile popout.
- **System hint row** above the composer, like "X supports game invites. Dismiss": 12 px, muted, with an accent link. Later.

### 1.5 Composer
- Left: "+" circle button, which opens a menu: Upload a File, Create Thread (later).
- Right icon buttons: **GIF** (the "GIF" glyph in a rounded rect), **Stickers** (later), **Emoji** (smile, which turns into a random grayscale→color emoji on hover; that's a static swap, not an animation). No gift button and no apps button.

### 1.6 Member list (240 px)
- Group headers "Online — 6" and "Offline — 15", 12 Semibold muted, 24 px top margin. With roles: one group per **hoisted** role ("Moderators — 2"), then Online, then Offline.
- **Activity section,** later, at the top: "Activity — 1" plus a gear icon. Cards are radius 8, bg `#232327`, padding 8, 1 px border. Each card shows: the user's mini avatar and name; the game name 14 Semibold; "2w ago · 🔥 Trending" 12 muted; and the game art 64 px on the right.
- **Member row,** 42 h, radius 6, hover bg.hover:
  - avatar 32 with status
  - name in role color, 16 Medium
  - badges: server tag, APP, crown
  - optional subline 12 muted: activity ("🎮 HELLDIVERS 2") or custom status, ellipsized
- **Offline members:** 60% opacity, rising to 100% on hover. (30% left their names at about 2.5:1 contrast, below what is readable.)
- **Click opens the profile popout** to the left of the list.

### 1.7 Profile popout (340 w)
- **Overall:** bg `#232327`, radius 8, shadow, 1 px border. Appears next to the clicked element and is clamped to the window.
- **Banner:** 105 h. Uses the user's banner color, or their banner image (later). "…" icon button top-right on the banner, with a dark translucent bg.
- **Avatar:** 80, overlapping the banner at its bottom-left (x 16), with a 6 px ring in the popout bg and the status cut-out.
- **Display name** 20 Bold. **Username** 14 with optional pronouns. Then badges.
- **Rows** below:
  - "N Mutual Servers" and "N Mutual Friends" (clickable)
  - Custom status
  - "Member since"
  - **Roles** header plus role chips. Each chip has a colored dot and the name, bg `#2B2B30`, radius 4. Admins see a "+" to add a role.
- **Primary action button,** full width, h 36: "Add Friend" or "Message". Our own popout shows "Edit Profile".
- **Bottom:** "Message @name" input. Typing there and pressing Enter sends a DM.

## 2. User Settings (full screen)
- **Left nav,** bg.sidebar:
  - At the top, a profile header: avatar 40, display name 16 Semibold, "Edit Profiles ✎" link 12 muted.
  - A **search box** that filters the nav items.
  - Group headers 12 Semibold muted.
  - Rows 34 h with an 18 px icon. A selected parent expands to show its **sub-items** with a 1 px vertical guide line, and the selected sub-item gets a white 2 px bar on that line.
- **Content area**
  - A top bar with the page title (16 Semibold) and a close × at the right edge. Esc also closes.
  - Content is max 700 w, centered in the remaining space. Sections are separated by dividers.
  - **Field rows:** label 16 Semibold left; value right-aligned 16 text.secondary; then a secondary "Edit" button. Masked values ("********@gmail.com") have an accent **Reveal** link.
  - **Chevron rows:** label left; value plus ">" right; the whole row is clickable.
- **Our nav groups**
  - **User Settings:** My Account (Account Info / Password & Security / Sign-in Methods / Account Status), Profiles, Privacy & Safety, Devices (logged-in sessions with "Log out" per device, which needs a server endpoint), Notifications.
  - **App Settings:** Voice & Video, Appearance (theme, message density "Cozy/Compact", chat font scale), Accessibility (reduced motion is moot since we don't animate; saturation; always underline links), Keybinds, Language (later), Advanced (developer mode → "Copy ID" in menus).
  - At the bottom: Log Out (danger), and the version string in muted text.
- **Omit:** Nitro, Server Boost, Subscriptions, Gift Inventory, Billing, Family Center.

## 3. Server Settings (full screen)
- **Left nav,** same component as §2.
  - A header with the server name (12 Semibold muted).
  - Groups:
    - Server Profile, Engagement (system messages channel, default notifications)
    - **Expression:** Emoji, Stickers (later), Soundboard (later)
    - **People:** Members, Roles, Invites, Access (who can join)
    - **Moderation:** Audit Log, Bans, AutoMod (later)
  - A divider, then **Delete Server** in danger text with a trash icon, owner only. It opens a confirm modal that requires typing the server name.
- **Server Profile page**
  - Two columns: form (max 560) and a **live preview card** on the right (320 w, sticky).
  - The preview card shows the banner color, the server icon as a squircle 64 overlapping the banner, the name 16 Bold, "● 5 Online ● 21 Members" in 12 with green and grey dots, and "Est. Apr 2025".
  - Form fields:
    - **Name** input.
    - **Icon:** helper text "We recommend an image of at least 512x512", then "Change Server Icon" (primary) and "Remove Icon" (danger text on secondary bg).
    - **Banner color:** a grid of 10 gradient swatches (5×2, 104×64, radius 8), with a 2 px white ring when selected.
    - **Description:** textarea with a 0/120 counter.
  - **Unsaved changes bar:** when the form is dirty, a bar slides in at the bottom ("Careful — you have unsaved changes!", Reset link, "Save Changes" button in success green). It's a position change only, not a continuous animation.
- **Roles page**
  - Title, plus the description "Use roles to group your server members and assign permissions."
  - A **"Default Permissions"** card: icon, title, "@everyone • applies to all server members", and a chevron. It opens the @everyone editor.
  - Search input "Search Roles", plus a **Create Role** primary button on the right.
  - Help text: "Members use the color of the highest role they have on this list. Drag roles to reorder them."
  - Table header "Roles — 8 | Members". Rows 56 h:
    - a shield icon tinted with the role color
    - the role name
    - the member count with a person icon
    - right: the pencil button, then the "…" button (§0 row-action buttons)
  - Rows can be reordered by **drag and drop**; a drop line shows the target.
- **Edit Role page**
  - Inner left column: "← Back" and "+". Below it, a role list with color dots; the selected role uses bg.selected.
  - Right panel:
    - "Edit Role — <name>" 16 Semibold, plus "…".
    - **Tabs:** Display / Permissions / Links (omit) / Manage Members. The active tab is text.primary with a **2 px accent underline**.
    - **Display tab:** role name, color picker (preset swatches plus a custom hex), "Display role members separately" toggle, "Allow anyone to @mention this role" toggle.
    - **Permissions tab:** "Search permissions" input. Sections ("General Server Permissions", "Membership", "Text Channel Permissions", "Voice Channel Permissions", "Advanced") have an accent "Clear permissions" link. Each permission row has a title 16 Semibold, a description 14 muted, and a toggle on the right, with a divider between rows.
    - **Manage Members tab:** search, member rows, and an "Add Members" button.
- **Members page,** already built: add a **Roles** column showing chips, and filter by role.

## 4. Behaviors that make it feel native
- Right-click works everywhere: messages, users, channels, servers, the rail and links. Every entity has a context menu.
- Hovering a channel or member shows its actions. Clicking a name or avatar opens the profile popout. Ctrl+K opens a **Quick Switcher** that jumps to channels, DMs and servers, with fuzzy search.
- Keyboard:
  - Alt+↑/↓ moves between channels.
  - Alt+Shift+↑/↓ jumps to unread channels.
  - Ctrl+Shift+M toggles mute.
  - Ctrl+Shift+D toggles deafen.
  - Esc marks the channel read.
- Drag files anywhere onto the chat to upload, with a full-pane drop overlay.

## 5. Scope for launch vs later
- **Launch (with voice):**
  - Everything in §0–§4 not marked "later".
  - **Roles:** colors, hoisting, ordering, the full permission set, and channel permission overrides, including private channels. This is a proto, server and client feature.
  - Profile popouts, banners as colors, mutual servers.
  - Quick Switcher, pins, notification settings.
  - Devices/sessions page.
- **Later:**
  - threads, events, server tags, stickers, soundboard, custom profile banners and images
  - game activity detection (§6), audit log, AutoMod, server folders, bots/webhooks (the APP badge)
- **Never:** Nitro, Boosts, gifts, billing, the app directory.

## 6. Game activity detection (later, desktop only)
- Detect games from a bundled list of process names plus user-added games in Settings → Activity.
- Use the Windows process-start/stop event subscription (WMI `Win32_ProcessStartTrace`). It's **event-driven, not polling**. Fall back to a 60 s check only if events are unavailable.
- Presence gets `activity { name, started_at }` (additive proto). The member list subline shows "🎮 <Game>" and the profile popout shows "Playing <Game> for 1h".
- A per-user privacy toggle controls whether the activity is shared.
