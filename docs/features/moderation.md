# Moderation

Audit log, member sanctions (kick with reason, timeouts, prune), AutoMod, raid
response, verification levels, age-restricted channels, slowmode control, the
channel permission editor, and raised capacity limits. Every feature is free
for every server. Contracts: `crates/proto/src/moderation.rs`. Server:
`crates/server/src/moderation.rs` (+ `moderation/audit.rs`,
`moderation/automod.rs`) and `crates/server/src/capacity.rs`. Web:
`crates/web/src/live/moderation.rs` and `crates/web/src/live/ui/moderation.rs`.

## Permissions

| Bit | Name | Grants |
| --- | --- | --- |
| 29 | `VIEW_AUDIT_LOG` | Read the audit log. |
| 30 | `MODERATE_MEMBERS` | Time out and remove timeouts ("Time Out Members"). |

Other checks reuse existing bits: Kick Members (kick, prune), Ban Members,
Manage Server (AutoMod, verification level, pauses), Manage Channels
(slowmode, age restriction), Manage Roles (channel permissions). Owners and
Administrators have everything. Every check runs on the server.

## Wire protocol (all additive)

- **Commands:** `Command::Launch(LaunchCommand::Moderation(ModerationCommand))`
  - `subscribe`: opt this event stream in to `Event::Moderation`. The web
    client sends it after every `Ready`.
  - `kick { guild_id, user_id, reason? }`
  - `timeout { guild_id, user_id, seconds, reason? }`: `seconds` up to
    2,419,200 (28 days). Zero removes the timeout.
  - `prune { guild_id, days, role_ids, reason? }`: `days` must be 1, 7 or 30.
    It returns `ModerationResult::Pruned { count }`.
  - `set_verification_level { guild_id, level }`: `none`, `account_age` or
    `member_age`.
  - `pause_invites { guild_id, seconds }` and `pause_dms { guild_id, seconds }`:
    from 3,600 to 86,400 seconds. Zero resumes.
  - `dismiss_raid_alert { guild_id }`
  - `create_auto_mod_rule(rule)`, `update_auto_mod_rule(rule)` and
    `delete_auto_mod_rule { guild_id, rule_id }`
  - `set_age_restricted { channel_id, age_restricted }`, `confirm_age { channel_id }`
- **Results:** `CommandResult::Moderation(ModerationResult)`. Variants:
  `pruned`, `auto_mod_rule`, `safety`.
- **Events:** `Event::Moderation(ModerationEvent)`. The server sends them only to
  streams that sent `subscribe`, so older clients never receive an unknown variant.
  - `safety_updated(GuildSafety)`: goes to all members. `raid` is set only for
    moderators.
  - `timeout_updated { guild_id, timeout }`: goes only to the member, with the
    reason.
  - `channel_age_restricted { channel_id, age_restricted }`
  - `auto_mod_updated { guild_id }`: goes to members with Manage Server.
  - `slowmode_updated { channel_id, seconds }`: goes to the channel's audience.
- **Member:** `Member.timeout_until` (optional, omitted when not timed out).
  A timeout also changes the member's computed permissions (see below), so
  `GuildUpsert` and `MemberUpsert` are sent when it starts and ends.
- **Refusals** use existing error codes, so old clients can decode them:
  `forbidden` with `field`:
  - `timeout`, `verification`, `automod`, `dms_paused` or `invites_paused`
  - `age_restricted`, on history reads

  The message is the explanation shown to the person.

## REST routes

| Route | Permission | Returns |
| --- | --- | --- |
| `GET /api/v1/guilds/{id}/audit-log?before=&limit=&actor=&action=` | View Audit Log | `AuditLogPage`, newest first. `limit` is 1–100 (default 50). `action` is an `AuditAction` name. |
| `GET /api/v1/guilds/{id}/moderation/me` | member | `ModerationState`: the server's safety settings (the raid alert for moderators only), your timeout and its reason, `send_blocked_until`, and the age-restricted channels you can see and have confirmed. |
| `GET /api/v1/guilds/{id}/automod/rules` | Manage Server | `Vec<AutoModRule>` |
| `GET /api/v1/guilds/{id}/prune?days=&roles=1,2` | Kick Members | `PrunePreview { days, count }` |
| `GET /api/v1/channels/{id}/access` | Manage Roles in the channel | `ChannelAccess`: members, viewers, whether @everyone can see it, which roles can, and a sample of viewers. |
| `GET /api/v1/channels/{id}/messages` | (existing) | Refused with `age_restricted` until the viewer confirms. |

## Behavior

- **Audit log.** Every administrative action writes one entry. The entry is in
  the same transaction as the action, so a refused or rolled-back action leaves
  no entry. Each entry records:
  - the actor, the action and the target
  - the target's name at the time
  - up to 16 before/after changes, each value clipped to 200 characters
  - an optional reason
  - the time

  Existing commands are audited without changing their handlers.
  `moderation::precheck` reads the state a command may change.
  `moderation::record` compares it with the state after the command,
  inside its transaction, and writes nothing when nothing changed.

  The log covers:
  - roles: create, update, delete, reorder and assignment
  - channels: create, update (including slowmode and age restriction), move and delete
  - permission overwrites
  - members: kick, ban, unban, timeout and its removal, prune, and nickname
    changes by others
  - server: settings, ownership transfer, verification level, and invite and DM pauses
  - invites: create and delete
  - custom emoji: create, rename and delete
  - messages: deletion by someone other than the author, pin and unpin
  - AutoMod: rule changes, blocks, alerts and timeouts. These use the AutoMod
    system author, see below.

  Retention: each server keeps its newest 5,000 entries and drops older ones as
  new ones are written.
- **Timeouts.** A timed-out member keeps only View Channel and Read History.
  `permissions::evaluate` applies this, so it covers sending, reacting,
  typing, joining and speaking in voice (voice access is reconciled at once).
  Rules for who can be timed out:
  - not yourself
  - not the owner
  - not an Administrator
  - not someone whose top role is at or above yours

  The timeout survives leaving and rejoining. It expires without polling: one
  task sleeps until the next deadline (timeouts, pauses, raid alerts) and is
  woken when a new one is set.
- **Prune.** Removes members who joined more than N days ago and have not used
  any session for N days. With no roles chosen, only members with no role are
  removed; with roles chosen, only members who have one of them. It never
  removes:
  - the owner or the person pruning
  - anyone online now
  - anyone the person pruning could not kick

  A preview gives the count first.
- **AutoMod.** Rules are checked on send and on edit, before the message is
  stored. Members with Manage Server are never checked. Each rule has exempt
  roles and exempt channels; a channel nested inside an exempt one is exempt too.
  Limits: 10 rules per server, at most one mention-spam rule and one spam rule.

  Triggers:
  - **Keyword:** up to 1,000 keywords and 100 allowed words, each 60
    characters or fewer. Matching ignores case. A keyword matches whole words;
    a `*` at its start or end lets it match inside longer words. A keyword can
    be a phrase.
  - **Mention spam:** more than N unique user and role mentions in one message.
  - **Spam:** the same message N times within 30 seconds, or more than N links.

  Actions:
  - Block, with the rule's message or a default explanation.
  - Alert: post in a chosen text channel as the system author "AutoMod". It has
    a reserved ID below all generated IDs, and its username cannot be
    registered.
  - Time out for a chosen duration.

  At most one alert is sent per member and rule every 10 seconds.
- **Raid response.** Joins are counted per server in a sliding window: up to 64
  timestamps per server and 1,024 servers, all in memory, with no timer. Ten
  joins within 60 seconds raise an alert for moderators, shown as a banner in
  the server and on Safety Setup. The alert clears after an hour or when
  dismissed.

  Security actions:
  - **Pause invites:** joining by invite is refused.
  - **Pause DMs:** direct messages between server members who are not friends
    are refused.

  Each pause lasts 1 to 24 hours, expires by itself, and can be turned off early.
- **Verification levels.**
  - None.
  - Low: the account is older than 5 minutes.
  - Medium: the account is older than 5 minutes and has been a member for more
    than 10 minutes.

  The level gates sending messages in the server and direct messages to its
  members who are not friends. Members with any role, and the owner, are
  exempt. StickTalk has no verified email or phone, so Discord's email and
  phone levels are not offered.
- **Age-restricted channels.** A text or voice channel can be flagged. History
  is refused until the viewer confirms they are an adult. The confirmation is
  stored per user per channel. The web app shows a gate instead of the content
  and fetches nothing until the viewer confirms.
- **Slowmode.**
  - The channel editor offers Off, 5s, 10s, 15s, 30s, 1m, 2m, 5m, 10m, 15m,
    30m, 1h, 2h and 6h.
  - A migration rebuilds the `channel_slowmode` CHECK constraint to allow the
    new values.
  - Members who are limited see their cooldown count down in the composer. A
    one-second ticker runs only while a cooldown is on screen.
- **Channel permission editor.** Add any role or member. Each permission has
  three states: allow, inherit or deny. A summary says who can see the channel.
  "Private channel" is a shortcut (it denies View Channel to @everyone). "Sync
  with parent" applies to nested channels: they already inherit everything
  above them, so syncing removes the channel's own overrides. The old one-role
  "private" form is unchanged.
- **Member menu.** Right-clicking a member, or opening their profile, shows Time
  Out or Remove Timeout, Kick and Ban to members with those permissions. Each
  has an optional reason.

## Capacity limits

| Limit | Before | Now |
| --- | --- | --- |
| Roles per server | 64 | 250 |
| Pins per channel | 50 | 250 |
| Servers per user | 10 | 200 |
| Channels per server | 32 | 500 |

The raised limits keep cost bounded in these ways:

- **Batched evaluation.** Listing a server's channels for a member
  (`db::channels`, and so `Ready`) and a channel's audience (`db::audience`,
  used by every message fan-out) used to evaluate permissions once per channel
  or per member, at several queries each. `capacity.rs` now loads a server's
  roles, overwrites, channel tree and timeouts in a fixed number of queries and
  evaluates in memory with the same rules. A test checks it against
  `permissions::evaluate`. `launch::filter_event` uses one batched audience
  for events with more than two recipients.
- **Nesting bound.** The channel nesting bound (`permissions::MAX_DEPTH`) now
  equals the channel limit. At 64, nesting deeper than 64 levels would have
  dropped outer permission overrides.
- **Pins.** Listed with `LIMIT 250` on a new `(channel_id, pinned_at)` index.
- **Other indexes.** New indexes for role-membership and overwrite-target lookups.
- **Initial snapshot.** At every limit at once, one account's `Ready` is about
  49 MB (`maximum_bootstrap_metadata_fits_the_frame_budget`). Server-to-client
  frames may now be up to `MAX_SERVER_FRAME_BYTES` (64 MiB); client commands
  stay at 512 KiB. Realistic accounts are far smaller. Loading server details
  lazily is the next step if accounts near the maxima appear.
