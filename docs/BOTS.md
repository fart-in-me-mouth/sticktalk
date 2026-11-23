# Building bots and webhooks for StickTalk

StickTalk is self-hosted, so every server (instance) has its own apps. An
**app** has a **bot**: an account marked **APP** that cannot sign in, has no
password, and acts only with its **bot token**. A server's members add the
app through its install link; the bot then joins with a role holding exactly
the permissions they granted, and nothing more.

A **webhook** is simpler: a secret URL that posts messages into one channel.

Everything is free. A complete bot in Rust is in
[`crates/server/examples/reference_bot.rs`](../crates/server/examples/reference_bot.rs);
the server's own tests run it (`cargo test -p sticktalk-server --test apps`).

Contents: [Create an app](#1-create-an-app) · [Authentication](#2-authentication) ·
[Connecting](#3-connecting) · [Intents and events](#4-intents-and-events) ·
[Sending messages](#5-sending-messages) · [Commands](#6-application-commands) ·
[Interactions](#7-interactions) · [Components and forms](#8-components-and-forms) ·
[Webhooks](#9-webhooks) · [Rate limits](#10-rate-limits) · [Errors](#11-errors) ·
[What bots can't do](#12-what-bots-cant-do)

## 1. Create an app

User Settings › **Developer** › *New App*. Give it a name (up to 32
characters), an icon, a description, the **permissions** its install link asks
for, and the **intents** (event classes) its bot needs. The bot token is shown
**once**: copy it then. *Reset Token* replaces it and disconnects the bot at
once; the old token stops working immediately.

*Install Link* copies `https://<your server>/?app=<app id>`. Anyone with
**Manage Server** in a server can open it (if the app is *public*; otherwise
only you) and sees a consent screen listing every permission and whether the
app may read message content. They can untick any permission. They can never
grant a permission they don't have, and only the server's owner can grant
**Administrator**, after confirming it.

The instance administrator may limit who can create apps, turn the feature off,
or turn one app off everywhere:

```
sticktalk-server admin apps list
sticktalk-server admin apps creation everyone|allowlist|disabled
sticktalk-server admin apps allow <username>     # with the allowlist policy
sticktalk-server admin apps disable <app id>     # and: enable <app id>
```

## 2. Authentication

A bot token looks like `stkb_` followed by 43 URL-safe base64 characters. The
server stores only its SHA-256. Send it:

- on the WebSocket, as `Identify.token`;
- on REST, as `Authorization: Bot stkb_…` (`Bearer stkb_…` also works).

A bot token works **only** on the WebSocket and under `/api/v1/bot/`. Every other
route refuses it (`401 unauthenticated`), and a person's session token never
works as a bot's. A disabled app, a deleted app or a reset token gets `401`.

## 3. Connecting

The WebSocket is the same one the apps use: `wss://<server>/api/v1/ws`, with the
subprotocol `sticktalk.v1`. Frames are **binary MessagePack with named fields**
(`rmp_serde::to_vec_named`); the types are in
[`crates/proto`](../crates/proto/src/apps.rs) and
[`docs/PROTOCOL.md`](PROTOCOL.md). JSON below shows the same maps.

1. The server sends `{"type":"hello","data":{"version":1,"heartbeat_interval_ms":30000,…}}`.
2. Send Identify within 10 seconds:

   ```json
   {"type":"identify","data":{"version":1,"token":"stkb_…","resume":null,"launch_features":true}}
   ```
3. The server answers `ready` (your servers, channels and roles) or `resumed`.
4. Send `{"type":"heartbeat","data":{"nonce":<n>,"last_seq":<last event seq>}}`
   every 30 seconds; the server closes the connection after 75 seconds of silence.
5. Commands are `{"type":"command","data":{"nonce":<unique u63>,"command":{…}}}`;
   each gets a `response` with the same nonce: `{"status":"ok","data":…}` or
   `{"status":"err","data":{"code":…,"message":…,"field":…}}`.

One connection per token: a second Identify with the same token replaces the
first (close code 4009). Identify is limited to 6 a minute. Bots never join calls
(voice signals from a bot are ignored).

## 4. Intents and events

Events arrive as `{"type":"event","data":{"seq":…,"event":{"type":…,"data":…}}}`.
A bot always receives its servers' structure (servers, channels, roles), its own
member record and `apps` events (interactions). Everything else needs an
**intent**, chosen in the Developer section:

| Intent (bit) | Events |
|---|---|
| Server Members (`1<<1`) | `member_upsert`, `member_removed` in every server; `GET /bot/guilds/{id}/members` |
| Moderation (`1<<2`) | `ban_upsert`, `ban_removed` |
| Messages (`1<<3`) | `message_created`, `message_updated`, `message_deleted` |
| Reactions (`1<<4`) | `reaction_updated` |
| Typing (`1<<5`) | `typing` |
| Message Content (`1<<7`) | the text, files and embeds of every message |

Without **Message Content**, a message event carries an empty `content`, no
`attachments` and no `extra`, except for the bot's own messages and messages
that mention it. Each server grants Message Content separately on the consent
screen; asking for the intent is not enough. Read states, presence,
notification settings and similar per-person events are never sent to bots.

## 5. Sending messages

Bots can use the ordinary message commands (`send_message`, `edit_message`,
`delete_message`, `add_reaction`, `typing`, …) with their member's permissions.
For embeds and components use `apps` › `send`:

```json
{"type":"apps","data":{"type":"send","data":{"channel_id":123,"draft":{
  "content":"Build passed",
  "embeds":[{"title":"#42 main","url":"https://ci.example.com/42","color":3066993,
             "fields":[{"name":"Time","value":"3m 12s","inline":true}],
             "footer":{"text":"CI"},"timestamp":"2026-11-23T17:00:00Z",
             "image":{"url":"https://ci.example.com/badge.png"}}],
  "components":[{"components":[{"type":"button","data":{"style":"link","label":"Open","url":"https://ci.example.com/42"}}]}],
  "allowed_mentions":{"parse":["users"]}
}}}}
```

REST equivalent: `POST /api/v1/bot/channels/{id}/messages` with the `draft` as
the JSON body (returns the `Message`). `PATCH` and `DELETE`
`/api/v1/bot/channels/{c}/messages/{m}` edit and delete the bot's own messages.
Files: `PUT /api/v1/bot/uploads/{channel}` (multipart, one `file` part) returns
an `Attachment` whose ID `send_message` can attach.

Limits: text up to 4,000 characters; up to 10 embeds and 6,000 characters in
all of them (title 256, description 4,096, 25 fields of 256/1,024, author 256,
footer 2,048); links are `http(s)` only. A message needs text, an embed, a
component or a file.

**Images** in embeds are never hot-linked. The server fetches each `image` and
`thumbnail` URL once (at most 6 per message, 30 a minute per bot) through the
same fetcher as link previews: public addresses only, no redirects to private
ones, 5 MiB cap, image types only; it re-encodes the image and serves it from
`/api/v1/channels/{c}/messages/{m}/app-images/{slot}` to people who can read
the channel. A `message_updated` follows when it arrives. Failed images are
dropped.

**Mentions.** With `allowed_mentions` omitted, a bot's mentions follow its
permissions like a member's. With it, only what it lists notifies anyone:
`parse` may hold `users`, `roles` and `everyone`; `users` and `roles` list IDs.
Other mentions stay in the text but notify no one.

Everything a bot posts passes the server's checks: AutoMod (text, embeds and
component labels), timeouts of its member, slowmode (unless its role has Manage
Messages or Manage Channels) and channel permissions. Its actions are in the
audit log under the bot, which carries the app's name.

## 6. Application commands

Register **slash commands** (typed in the composer), **user commands** (a
member's menu › Apps) and **message commands** (a message's menu › Apps).
`set_commands` replaces all of one scope's commands; commands keeping their
kind and name keep their ID.

```json
{"type":"apps","data":{"type":"set_commands","data":{"guild_id":null,"commands":[
  {"kind":"chat_input","name":"weather","description":"Forecasts",
   "options":[
     {"kind":"sub_command","name":"today","description":"Today","options":[
        {"kind":"string","name":"city","description":"City","required":true,"autocomplete":true,"max_length":40}]},
     {"kind":"sub_command","name":"week","description":"Next 7 days","options":[
        {"kind":"string","name":"city","description":"City","required":true},
        {"kind":"integer","name":"days","description":"Days","min_value":1,"max_value":7,
         "choices":[{"name":"Three","value":{"type":"integer","value":3}}]}]}]},
  {"kind":"chat_input","name":"purge","description":"Admins only","default_member_permissions":1024},
  {"kind":"user","name":"High five"},
  {"kind":"message","name":"Translate"}]}}}
```

`guild_id: null` is global (every server the app is in); a server ID registers
commands for that server only. REST: `PUT /api/v1/bot/commands` or
`PUT /api/v1/bot/guilds/{id}/commands` with the list; `GET` the same paths.

Rules: up to 100 commands per scope; slash command and option names are 1–32
lowercase letters, digits, `-` or `_`; descriptions 1–100 characters; menu
commands have a name of up to 32 characters and nothing else. Up to 25 options,
required before optional; subcommands and groups (one level of groups) can't be
mixed with other options; up to 25 choices of the option's type, not with
autocomplete. Option kinds: `string`, `integer`, `number`, `boolean`, `user`,
`channel`, `role`, `mentionable`, `attachment`, `sub_command`, `sub_command_group`.

Who may use a command: members need **Use Application Commands** in the
channel (on for @everyone by default) and the command's
`default_member_permissions`. Server Settings › Integrations can allow or deny
each command (or all of an app's) per role and per channel; Manage Server can
always use everything.

## 7. Interactions

When someone uses a command, presses a button, picks from a select or submits a
form, the bot receives:

```json
{"type":"apps","data":{"type":"interaction","data":{
  "id":91,"token":"<43 characters>","app_id":7,"guild_id":10,"channel_id":11,
  "user":{"id":2,"username":"alex",…},"member_permissions":…,"app_permissions":…,
  "data":{"type":"command","data":{"command_id":55,"name":"weather","kind":"chat_input",
    "options":[{"name":"today","options":[{"name":"city","value":{"type":"string","value":"Oslo"}}]}],
    "resolved":{}}},
  "created_at":1795000000000}}}
```

The server already checked every option against your definition and resolved
users, roles, channels, messages and files into `resolved`: you never need to
trust an ID. `data.type` is `command`, `autocomplete`, `component` or
`modal_submit`.

Answer within **3 seconds** (autocomplete: 2) with `respond`:

```json
{"type":"apps","data":{"type":"respond","data":{
  "answer":{"interaction_id":91,"token":"<token>"},
  "response":{"type":"message","data":{"content":"Sunny, 21 °C","ephemeral":true}}}}}
```

| `response.type` | Does |
|---|---|
| `message` | posts a message as the bot (shows "used /weather"); with `"ephemeral": true` only the person sees it, and it is not stored |
| `defer` | shows "*app* is thinking…" to the person; finish with `edit_original` within 15 minutes (`{"ephemeral":true}` keeps the answer private) |
| `defer_update` | components: acknowledge now, change the message later |
| `update_message` | components: replace the message (or ephemeral message) the component is on |
| `modal` | opens a form; its submission arrives as a `modal_submit` interaction |
| `autocomplete` | `{"choices":[{"name":"Oslo","value":{"type":"string","value":"oslo"}}]}`, up to 25 |

After answering, for 15 minutes: `edit_original`, `delete_original` and up to
10 `follow_up` messages (each `{answer, draft}`). REST:
`POST /api/v1/bot/interactions/{id}/{token}/callback` (body: the response),
`PATCH`/`DELETE …/original`, `POST …/followup`.

If no answer arrives in time, the person sees "The app didn't respond in time"
and later answers are refused. If the bot is not connected, using its command
fails at once with `app_offline`. Interactions go only to the bot's WebSocket:
StickTalk never calls a URL you supply.

## 8. Components and forms

Messages can carry up to 5 rows; a row holds up to 5 **buttons** or one
**select**.

```json
{"components":[
  {"type":"button","data":{"style":"primary","label":"Vote","custom_id":"vote"}},
  {"type":"button","data":{"style":"danger","label":"Close","custom_id":"close","disabled":true}},
  {"type":"button","data":{"style":"link","label":"Results","url":"https://example.com/r"}}]}
{"components":[{"type":"select","data":{"kind":"string","custom_id":"color","min_values":1,"max_values":2,
  "options":[{"label":"Red","value":"r"},{"label":"Blue","value":"b"}]}}]}
```

Button styles: `primary`, `secondary`, `success`, `danger`, `link` (opens its
URL; no interaction). Select kinds: `string` (1–25 options), `user`, `role`,
`channel`. `custom_id`s are 1–100 characters, unique in the message.

The server checks every press before the bot hears of it: the component exists
on that message (which the bot sent), is enabled, and the values are among the
options or are real members, roles or channels the person can see. A component
interaction includes `message` (the message it is on).

**Forms** (`modal`): a `custom_id`, a `title` (up to 45 characters) and 1–5
text inputs (`short` or `paragraph`, label up to 45, `min_length`,
`max_length` up to 4,000, `required`, `placeholder`, pre-filled `value`). The
server checks lengths and required fields; each form can be submitted once.

## 9. Webhooks

Channel settings › **Webhooks** (Manage Webhooks) creates one for a text,
announcement or forum channel or a thread: name, avatar, and a secret URL shown
once. *Copy URL* again needs *Rotate*, which replaces the secret (the old URL
stops at once). Changes are in the audit log.

```
POST https://<server>/api/v1/webhooks/<id>/<secret>?wait=true&thread_id=<thread>
Content-Type: application/json

{"content":"Deployed v1.2 to production",
 "username":"Release Bot",
 "avatar_url":"https://example.com/rocket.png",
 "embeds":[{"title":"v1.2","description":"3 fixes","color":5763719}],
 "allowed_mentions":{"parse":["users"],"users":["123"]}}
```

- `204` (or `200` with the message when `wait=true`). `thread_id` posts in a
  thread of the webhook's channel; in a forum, `thread_name` starts a new post.
- Multipart: a `payload_json` part (the JSON above) and up to ten `files[n]` parts.
- `PATCH` and `DELETE …/messages/<message id>` edit and delete the webhook's own
  messages; `GET` on the URL returns the webhook (never its secret).
- Default mentions: users only. `@everyone`/`@here` notify only if
  `allowed_mentions.parse` has `everyone` **and** the webhook's creator may
  mention everyone there; role pings only if allowed.
- Limits: 30 posts a minute per webhook (burst 5), 30 a minute per channel,
  120 requests a minute per address; JSON up to 64 KiB; 15 webhooks per channel,
  100 per server. AutoMod applies, including to embeds.
- A webhook posts as its own APP account, which is not a member of the server:
  it can post in its channel and that channel's threads, and read nothing.

**Slack-style** `POST …/slack`: `text`, `username`, `icon_url` and
`attachments` (`color` (hex or good/warning/danger), `pretext`, `author_name`,
`author_link`, `title`, `title_link`, `text`, `fields` (`title`, `value`,
`short`), `image_url`, `thumb_url`, `footer`, `ts`) become text and embeds.
`<url|label>` becomes a link; `<!here>`/`<!channel>` are written out and ping no
one. JSON or `payload=` form encoding; answers `ok`. Blocks are ignored.

**GitHub-style** `POST …/github` (Content type `application/json`): `ping`,
`push`, `pull_request` (opened, reopened, ready for review, closed, merged),
`issues` (opened, closed, reopened), `issue_comment` (created), `release`
(published), `star` (created), `create` and `delete` become an embed. Other
events are accepted with `204` and not shown. Signatures are not checked: the
secret URL is the credential.

## 10. Rate limits

Limits are per bot (separate from people's) and answer `429` /
`rate_limited` with `retry_after_ms` and a `Retry-After` header.

| What | Limit |
|---|---|
| All bot commands | 600 a minute, burst 60 |
| Bot REST calls | 300 a minute, burst 50 |
| `send` | 60 a minute, burst 10 (plus the ordinary per-account limits) |
| `set_commands` | 10 a minute, burst 5 |
| Identify | 6 a minute |
| Embed images | 30 a minute |
| People: commands, buttons and forms together | 60 a minute; autocomplete 120 |
| Pending interactions | 16 per person, 4,096 in all |

## 11. Errors

Errors use StickTalk's codes (`docs/PROTOCOL.md`); `field` says which rule:

| `code` / `field` | Meaning |
|---|---|
| `unauthenticated` | bad, reset or disabled token |
| `forbidden` / `bot` | bots can't use that command |
| `forbidden` / `app_disabled`, `apps_disabled` | turned off by the administrator |
| `forbidden` / `automod`, `timeout`, `verification` | the server's moderation refused the message |
| `forbidden` / `managed_role` | an app's role changes only with its app |
| `forbidden` / `permissions`, `administrator` | the install asked for more than allowed |
| `invalid_request` / `commands`, `options`, `embeds`, `components`, `modal`, `response` | the payload broke a rule (the message says which) |
| `invalid_request` / `interaction` | unknown, expired, or answered too late |
| `conflict` | the interaction was already answered |
| `unavailable` / `app_offline` | the bot isn't connected |
| `rate_limited` | see `retry_after_ms` |

## 12. What bots can't do

Bots have no friends, DMs, group DMs, calls, profile or account settings, data
export, folders, notification settings, server creation or joining by invite;
they can't own a server, and their managed role can't be given to anyone else.
They can use what a member with the same permissions could: messages,
reactions, pins, threads and forum posts, polls, channels, roles, invites,
slowmode, kicks, bans and timeouts. Removing an app (Integrations › Remove)
removes its member, its role, its commands and their overrides, and nothing
else: its messages and the server's webhooks stay. Kicking or banning its bot
removes the app too; its role then stays as an ordinary role.
