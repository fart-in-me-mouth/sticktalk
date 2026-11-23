# Apps, bots, application commands and webhooks (task `apps`)

Apps with bot users, an install consent screen, slash/user/message commands,
interactions with message components and forms, incoming webhooks (with Slack-
and GitHub-style payloads), and the instance administrator's controls. All
free. The developer guide, with complete request and response examples, is
[`docs/BOTS.md`](../BOTS.md); this file is the contract and the integration
notes.

Code: `crates/proto/src/apps.rs` (wire), `crates/server/src/apps.rs` and
`apps/{store,install,registry,interactions,messages,webhooks,rest,admin,tests}.rs`
(server), `crates/server/examples/reference_bot.rs` and
`crates/server/tests/apps.rs` (reference bot, end to end),
`crates/web/src/live/apps.rs` and `live/ui/apps.rs` (web), migration
`202611231701_apps.sql`.

## Permissions

| Bit | Name | Default for @everyone |
| --- | --- | --- |
| 1 << 46 | Manage Webhooks | no |
| 1 << 47 | Use Application Commands | yes (migration + trigger for new servers) |

Installing and removing apps, and command overrides: Manage Server. A timeout
removes Use Application Commands with everything else (`TIMED_OUT_KEEPS`).

## Accounts and credentials

- A bot or webhook is a `users` row with an empty password, `style = {"bot":true}`
  (`UserStyle::bot`, the APP tag) and a username containing `-`, which
  `db::username` rejects: no person can register it, sign in with it, or be
  reset to it by the administrator CLI.
- Its credential is a `sessions` row (expiry year 9999) listed in
  `app_sessions(kind bot|webhook)`. **`db::auth` refuses every such row**, so
  a bot token never works on a person's REST route, the browser cookie path or
  `/mcp`. Bot tokens are `stkb_` + 43 characters, stored as SHA-256, shown
  once, replaced by Reset Token (which closes the bot's connections).
- `apps::gate` runs first in every `commands::execute`: a person can't send
  bot-only or webhook-only `AppCommand`s; a bot may send only an allowlist
  (messages, reactions, typing, pins, threads, polls, channels, roles,
  overwrites, invites, slowmode, kick/ban/timeout, nickname, `apps` bot
  commands); a webhook credential may send only `execute_webhook`,
  `edit_webhook_message`, `delete_webhook_message`. Everything still runs the
  ordinary permission checks of the account.
- Guards (people): an app's managed role can't be deleted, given or taken;
  bots can't be DM'd, added to groups or given a server.
- A webhook account is never a member. `permissions::evaluate` calls
  `apps::principal` only for non-members: in the webhook's channel and the
  threads under it, View, Send, Send in Threads, Embed Links and Attach Files
  (plus Mention Everyone while its creator has it there); nothing elsewhere,
  and thread rules (private, locked) still apply. It can't read history.

## Wire protocol (additive)

`Command::Apps(AppCommand)`, `CommandResult::Apps(AppResult)`,
`Event::Apps(AppEvent)`; `MessageExtra::app: Option<Box<AppMessage>>`
(embeds, components, webhook name/avatar override, the command it answers);
`UserStyle::bot`; new `AuditAction`s `app_install`, `app_remove`,
`command_permissions_update`, `webhook_create`, `webhook_update`,
`webhook_delete` and `AuditTarget::{App, Webhook}`. A person's connection gets
`Event::Apps` only after sending any `AppCommand` (the web app sends
`Subscribe` after Ready); bots always do. Old clients never see a new variant.

| Command | Who | Notes |
| --- | --- | --- |
| `subscribe` | anyone | opt in to `Event::Apps` |
| `create_app(AppDraft)` | the instance policy allows | ≤ 25 per person; returns `token` once |
| `update_app`, `delete_app`, `reset_token` | the app's owner | delete removes it from every server and disables the bot account |
| `install(InstallRequest)` | Manage Server | public app or own app; `permissions` ⊆ what the app asks ⊆ installer's own (owner: anything but Administrator unless `confirm_administrator`); `message_content` only if the app has the intent |
| `uninstall` | Manage Server | removes member, managed role (and its overwrites), commands and overrides |
| `set_command_permissions { guild_id, id, targets }` | Manage Server | `id` is a command or the app (all its commands); ≤ 100 role/channel allow/deny |
| `create_webhook`, `update_webhook`, `rotate_webhook`, `delete_webhook` | Manage Webhooks in the channel | text, announcement, forum, thread; ≤ 15 per channel, 100 per server; secret URL returned on create and rotate only |
| `invoke(Invocation)` | Use Application Commands + overrides + `default_member_permissions` | options validated and resolved; bot must be connected |
| `autocomplete { invocation, request }` | same | answer arrives as `AppEvent::Autocomplete { request }` on that connection |
| `component { channel_id, message_id, custom_id, values }` | can read the message; Use Application Commands | the component must exist on the bot's message (or the person's ephemeral one), be enabled, and values must be valid |
| `submit_modal { interaction_id, values }` | the person the form was shown to | once; lengths and required fields checked |
| `set_commands`, `respond`, `edit_original`, `delete_original`, `follow_up`, `send`, `edit` | bots | see BOTS.md |
| `execute_webhook`, `edit_webhook_message`, `delete_webhook_message` | a webhook's own credential (from its secret URL) | |

`AppEvent`: `interaction` (bots), `ephemeral` / `ephemeral_removed`, `modal`,
`autocomplete`, `failed` (to the invoking connection only), `commands_updated
{ guild_id }` (members), `integrations_updated { guild_id }` (Manage Server or
Manage Webhooks), `apps_updated` (the owner).

### REST

People (cookie or bearer):

| Route | Answer |
| --- | --- |
| `GET /api/v1/applications/@me` | `DeveloperState` (your apps, the policy, whether you may create) |
| `GET /api/v1/applications/{id}/install` | `InstallPreview` (consent screen: app, owner, servers you manage) |
| `GET /api/v1/guilds/{id}/integrations` | `GuildIntegrations` (apps for Manage Server; webhooks you may manage) |
| `GET /api/v1/channels/{id}/webhooks` | the channel's webhooks (Manage Webhooks) |
| `GET /api/v1/channels/{id}/commands` | `CommandCatalog`: what the `/` picker may offer you here |
| `GET /api/v1/channels/{c}/messages/{m}/app-images/{slot}` | a stored embed image or webhook avatar (readers of the channel; `nosniff`, sandbox CSP) |

Bots (`Authorization: Bot stkb_…`): `/api/v1/bot/@me`,
`/bot/guilds/{id}/channels`, `/bot/guilds/{id}/members` (Server Members intent),
`/bot/channels/{id}/messages` (GET history with content withheld as on the
WebSocket; POST send), `/bot/channels/{c}/messages/{m}` (PATCH, DELETE),
`/bot/commands`, `/bot/guilds/{id}/commands` (GET, PUT),
`/bot/interactions/{id}/{token}/{callback,original,followup}`,
`PUT /bot/uploads/{channel}`. Mutations run as commands through
`commands::execute`.

Webhooks (no account; the secret URL): `GET|POST /api/v1/webhooks/{id}/{secret}`
(`?wait=true`, `?thread_id=`), `POST …/slack`, `POST …/github`,
`PATCH|DELETE …/messages/{message}`. Unknown webhook or wrong secret: `404`.

## Behavior

- **Events for bots.** `Core::emit_filtered` asks `apps::view` per stream:
  people's streams are unchanged (except `Event::Apps` needs the opt-in);
  bots' streams drop what their intents don't cover, get member events of every
  server with Server Members, and get message events with content, files and
  extras withheld unless the server granted Message Content (or the bot wrote
  or is mentioned in the message). Which channels a bot may read content in is
  loaded when its stream is registered (Identify, its first command or REST
  call) and kept current from `ChannelUpsert` and new-thread sends; a channel
  it doesn't know is withheld (fail closed).
- **Moderation.** `moderation::precheck` screens `Command::Apps` through
  `apps::outgoing`: the text of every message a bot or webhook would post,
  including embed titles, descriptions, fields, footers and component labels,
  gets the timeout, verification, community gate and AutoMod checks. Messages
  are then posted through `commands::mutate(SendMessage)` (or `EditMessage`,
  or the forum's `CreatePost`), so slowmode, permissions, mentions, read
  states, threads, push and audit run unchanged. Embed-only messages pass
  `validate_message` through a task-local flag (`apps::rich`) set only around
  these validated sends.
- **Mentions.** `apps::messages::mentions` makes disallowed `@everyone`,
  `@here`, `@role` and `<@&id>` inert (a zero-width space after `@`, role
  tokens written as plain names) before the send path parses them, and
  computes the notified users from `<@id>` tokens. Webhooks default to users
  only; bots to their permissions.
- **Images.** Embed images, thumbnails and webhook avatars are queued (≤ 6 per
  message, 30 a minute per author, 256 waiting) and fetched after the command
  commits by the apps timer task through `content::Content::fetch_image` (the
  link-preview fetcher: public addresses only, checked on every hop, pinned
  resolution, caps, re-encoding). Stored in `app_images` (≤ 480 px; avatars
  128, thumbnails 160), then `MessageUpdated`. Deleting a message deletes them
  (trigger).
- **Interactions** are in memory (`apps::interactions`), bounded (4,096; 16
  open per person; 4,096 ephemeral messages), and expire on one timer task
  that sleeps until the next deadline (3 s to answer, 2 s for autocomplete,
  15 minutes for edits and follow-ups) and is woken by `Notify`; with nothing
  pending it sleeps without a deadline. Tokens are random and compared in
  constant time. Ephemeral answers are not stored; they reach only the
  invoking connection. Deferred answers show "thinking" to the person only,
  for public deferrals too.
- **Removal.** Kicking, banning or pruning a bot removes its installation,
  commands and overrides (trigger `apps_bot_left`); Uninstall also deletes the
  managed role. Deleting an app removes it everywhere, deletes its sessions
  and disables the bot account; its messages stay.
- **Admin.** `sticktalk-server admin apps list | disable <id> | enable <id> |
  creation everyone|allowlist|disabled | allow <user> | disallow <user>`.
  Disabling an app adds its bot to `disabled_users` (all credentials refused;
  open connections close at the next housekeeping pass) and hides its
  commands. `creation disabled` turns the feature off: no new apps, no bot
  connects, no commands are offered. Webhooks are not affected.
- **Audit.** Install, removal, command overrides, webhook create/update
  (including rotation)/delete. Bots' own actions (kicks, role changes,
  deletions) are recorded by the existing hooks with the bot as actor; the
  bot's name is the app's name and it carries the APP tag.

## Web app

- Settings › **Developer**: your apps, New App, edit (name, icon,
  description, public, permissions, intents), Reset Token (shown once with
  Copy), Install Link, Delete.
- **Consent screen** for `/?app=<id>`: the app, its owner, a server picker,
  every requested permission (untickable, and disabled where you lack it),
  the Message Content grant, an Administrator confirmation for owners.
- Server Settings › **Integrations**: installed apps with their role,
  permissions and commands; per-command and all-commands role/channel
  overrides; Remove; and the server's webhooks.
- Channel settings › **Webhooks**: create, rename, avatar, Copy URL (after
  create or Rotate), Rotate, Delete.
- Composer: typing `/` opens the command picker: built-ins first, then each
  app's commands under its name, fuzzy matched. Choosing one (Tab/Enter or a
  click) shows its signature; options are typed as `name:value` (quotes keep
  spaces; `@member`, `#channel`, `@role`), with suggestions for the option
  being typed (choices, members, roles, channels, true/false, or the bot's
  autocomplete, which the server drops after 2 s). Text after the command
  fills its first text option. Enter sends when nothing is left to choose;
  a problem (missing required option, wrong type) is shown and the draft
  kept. Built-ins: `/shrug`, `/tableflip`, `/unflip`, `/me`, `/spoiler`,
  `/nick`, `/tts` (the existing one). Unknown `/words` are sent as text.
- User and message commands: "Apps" on a message's actions and on a
  member's profile card, when the channel's apps have such commands (the
  channel's catalog is fetched only in servers that have a bot member).
- Messages: the APP tag on bots and webhooks (messages and member list),
  "*name* used /command" lines, embeds (accent color, author, linked title,
  Markdown description, inline and full-width fields, image, thumbnail,
  footer, time), buttons (five styles; link buttons open in a new tab) and
  selects (with a Choose button), ephemeral messages ("Only you can see
  this · Dismiss message"), "*app* is thinking…", forms in a dialog, and
  "didn't respond in time" as a notice.
- First load: the state struct, the event dispatch, and small hooks that do
  nothing until an app message, a `/`, or one of these pages appears. The
  views (`load_views`), the picker (`compose_lazy`), the send intercept
  (`run_slash`), page data (`refresh`), actions and forms are separate
  lazily loaded chunks (about 90 KB Brotli together).
- Browser test: `crates/web/tests/apps.mjs` (needs the reference bot binary;
  see the file's header).

## Decisions to revisit

- Interaction answers are posted with the bot's own channel permissions (and
  slowmode, unless its role bypasses it), unlike Discord, where interaction
  responses ignore the bot's channel permissions.
- A public `defer` shows "thinking" only to the person who used the command.
- Ephemeral messages are not stored: they disappear on reload, and their embed
  images are not fetched.
- Bots can't open DMs at all (simpler than "only people who interacted").
- Attachment options resolve to the uploaded file's metadata; the bot can't
  download a person's upload yet.
- One connection per bot token (no sharding); one global `apps` timer task.
- Webhook GitHub signatures are not verified; the secret URL is the credential.
