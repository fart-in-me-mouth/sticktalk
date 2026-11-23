# Navigation: search, Quick Switcher, inbox, saved messages, pins, Favorites, folders

Everything here is free for every account. All data is private to the account
that owns it, stored on the server so it follows the account across devices,
bounded, and checked against what the account may read *now*.

Code: `crates/proto/src/nav.rs`, `crates/server/src/nav.rs` (+ `nav/tests.rs`),
`crates/server/src/search.rs`, `crates/web/src/live/nav.rs`,
`crates/web/src/live/ui/navui.rs`, `crates/web/src/nav_logic.rs`,
`crates/web/public/nav.js`, `crates/web/tests/nav.mjs`.
Migration: `crates/server/migrations/202611230101_nav.sql`.

## Limits

| What | Limit |
| --- | --- |
| Pinned conversations (DMs, groups, channels) | 50 (`MAX_PINNED_CONVERSATIONS`) |
| Favorites | 200 (`MAX_FAVORITES`) |
| Favorite categories | 20 (`MAX_FAVORITE_CATEGORIES`), names 1–32 characters |
| Server folders | 10, up to 10 servers in folders (existing `ServerFolder` rules) |
| Rail order entries | 20 |
| Saved messages (bookmarks and reminders together) | 200 (`MAX_SAVED_MESSAGES`) |
| Reminder time | from now to 366 days ahead |
| Search / mention / saved pages | 1–50 per page (default 25) |
| Unreads inbox | 50 most recently active conversations, 3 newest unread messages each |
| Mark as read | 100 channels per request |

Writes are rate limited (`nav-write`, 60/min, burst 20); search 30/min; inbox
reads 60/min.

## REST routes (all need a session; writes need the CSRF header in a browser)

| Route | Body / query | Answer |
| --- | --- | --- |
| `GET /api/v1/users/@me/nav` | | `Snapshot<NavSettings>`. Also opts this stream in to the events below. |
| `PUT /api/v1/users/@me/nav/pins` | `SetPinned { channel_ids }` (complete, ordered) | `NavSettings` |
| `PUT /api/v1/users/@me/nav/favorites` | `SetFavorites { categories, favorites }` (complete) | `NavSettings` |
| `PUT /api/v1/users/@me/nav/rail` | `SetRail { folders, order }` (complete) | `NavSettings` |
| `GET /api/v1/users/@me/saved` | `?before=<id>&limit=` | `Snapshot<SavedPage>` newest first |
| `PUT /api/v1/users/@me/saved/{message_id}` | `SaveMessage { channel_id, remind_at?, clear_reminder? }` | `SavedMessage` |
| `DELETE /api/v1/users/@me/saved/{message_id}` | `?reminder_only=true` clears only the reminder | `SavedSummary` |
| `GET /api/v1/users/@me/mentions` | `?before=&limit=&everyone=true&roles=true` | `Snapshot<MessageSearchPage>` |
| `GET /api/v1/users/@me/unreads` | | `Snapshot<InboxUnreads>` |
| `POST /api/v1/users/@me/unreads/ack` | `AckChannels { channel_ids }` | `AckResult { read_states }`; also `ReadStateUpdated` events |
| `GET /api/v1/users/@me/messages/search` | `?q=&before=&limit=&channel_id=` | `Snapshot<MessageSearchPage>`: DMs and groups only |

`NavSettings` = `pinned`, `favorite_categories`, `favorites` (entries carry an
optional `category_id`), `folders` (the existing `ServerFolder`), `rail`
(`RailItem::Guild(id)` / `RailItem::Folder(id)`, servers not listed follow in
join order) and `saved` (`SavedSummary { count, pending, due }`).

## Permissions and privacy

- Pins and Favorites accept only conversations the account can see now
  (`VIEW_CHANNEL`, or DM/group membership); categories are refused. A
  conversation that later becomes hidden is left out of every answer, so its ID
  never leaks; a deleted channel is removed by foreign key.
- The rail accepts only servers the account belongs to; a server cannot be both
  loose and in a folder. Folders keep their existing validation
  (`folders::set`), and both are written in one transaction.
- Saving needs `VIEW_CHANNEL + READ_HISTORY` on the message's own channel and a
  message that is not deleted. A saved message whose source is later deleted or
  unreadable is listed as `message: null`: no content, author or attachments.
- Mentions and unreads list only conversations whose history may be read now.
  Mentions exclude your own messages and blocked authors. `everyone=false`
  drops @everyone/@here, `roles=false` drops role mentions; explicit mentions
  always count.
- DM search covers DMs and groups the account is a member of (closed ones too);
  `channel_id` outside that set is `404`. Server search is unchanged and still
  applies `VIEW_CHANNEL + READ_HISTORY` per channel before the limit.
- Server tests: `crates/server/src/nav/tests.rs` (one per area above).

## Events (WebSocket)

Appended to `Event`; sent **only** to streams that fetched
`GET /api/v1/users/@me/nav` (the web app does after every Ready), so older
clients never see a variant they cannot decode:

- `NavUpdated(NavSettings)`: after any pins/favorites/rail write (other devices).
- `SavedUpdated(SavedSummary)`: after a save/unsave or when reminders fire.
- `ReminderDue(SavedMessage)`: a reminder's time has come (`message` null when
  it can no longer be read).

## Reminders

One task (`nav::spawn`, started with the router) sleeps until the earliest
`remind_at` that has not fired, or until a reminder is set or cleared
(`Notify`). It fires all due reminders in batches of 100, marks them fired,
sends the in-app event, and sends a Web Push through the existing push path
(`push::reminder`: generic "Reminder" text, only when the owner has no focused
app, not in Do Not Disturb, and only for a message they may still read). It
never scans on a schedule; with no reminders set it does nothing.

## Search syntax (server and the web suggestions)

Words (quoted phrases allowed), `from:<user>`, `mentions:<user>`, `in:<channel>`
(for DMs: a group name or the other person's username), `has:link|file|image|video`
(video is new), `before:`, `after:`, `during:` (new, one day) with `YYYY-MM-DD`.

## Web app

- Header: search box (desktop) / search button (phone) with filter and people
  suggestions as you type, results panel with highlighted terms, paging and
  jump-to-message; Inbox button with a badge (mentions + due reminders).
- Ctrl+K / Cmd+K: Quick Switcher over people, DMs, groups, servers and channels
  already in the client. `#`, `@`, `*` narrow to channels, people, servers.
  Unread and mentioned items rank first. Phones: "Find or start a conversation"
  at the top of the sidebar.
- Inbox: Unreads (by server, Mark as Read per channel and for all), Mentions
  (filters), Saved (due reminders first, Remind Me, Clear Reminder, Remove).
  Phone: the Notifications tab opens it.
- Message actions: Save Message, Remind Me (presets or a custom time).
- Conversation options (the "…" on a row, right-click, long-press, Shift+F10):
  Pin to Top, Add to Favorites, Mark as Read, Notification Settings.
- Rail: Favorites entry (star) under Home; servers and folders in the saved
  order. Drag a server onto another to make a folder, onto a folder to add it,
  above/below to reorder. Right-click/long-press/Shift+F10 gives the same
  moves as a menu (Move Up/Down, Create Folder, Add to …, Remove from folder),
  plus folder name, color, collapse and ungroup. Folders show unread and
  mention badges.
