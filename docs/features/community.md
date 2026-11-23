# Community tools (task `community`)

Rules screening, member applications (with interviews), onboarding, channel
opt-in ("Browse Channels"), this instance's server directory, server templates
and server insights. Every feature is free for every server, with no
eligibility thresholds.

Code: `crates/proto/src/community.rs` (wire), `crates/server/src/community.rs`
and `community/{admission,onboarding,directory,templates,insights,routes,tests}.rs`
(server), `crates/web/src/live/community.rs` and `live/ui/community.rs` (web),
`crates/web/tests/community.mjs` (browser test). Migration
`202611231201_community.sql`. No new permission bits.

## Permissions

| Action | Who |
| --- | --- |
| Rules screening, applications form, onboarding, directory listing, templates | Manage Server (owner and Administrators have everything) |
| Onboarding answers that give roles | also Manage Roles; each role below the configuring person's top role (the owner has no ceiling) and with none of `ONBOARDING_FORBIDDEN`: Administrator, Manage Server, Manage Roles, Ban, Kick, Time Out (Moderate) Members, Manage Channels, Manage Messages |
| Review applications, open interviews, `GET .../applications` | Kick Members |
| Insights | owner, View Audit Log or Manage Server |
| Accept rules, answer onboarding, Browse Channels, guide tasks | the member themself |
| Apply, withdraw | anyone not banned, not a member, and (for a server with a verification level) with an account older than 5 minutes |
| Directory controls (feature, hide, delist, turn off) | the instance administrator, with `sticktalk-server admin directory ...` |

Every check runs on the server; each has a server test.

## Wire protocol (all additive)

- `Command::Community(CommunityCommand)`, `CommandResult::Community(CommunityResult)`,
  `Event::Community(CommunityEvent)`. Events go **only** to connections that
  sent `CommunityCommand::Subscribe` (the web app does after every Ready).
- New `AuditAction`s (appended): `application_approve`, `application_reject`,
  `admission_update`, `onboarding_update`, `listing_update`, `template_create`,
  `template_update`, `template_delete`. Onboarding role changes are logged as
  `member_roles_update` by the member, with the reason "Chosen in onboarding".
- Refusals use existing error codes with `field`: `screening` (a pending member
  tried to talk), `application` (the server takes applications, or a recent
  rejection), `rules`, `answers`, `roles`, `responses`, `requirements`,
  `directory`, `template`, `verification`, `invites_paused`.

| Command | Result | Notes |
| --- | --- | --- |
| `SetAdmission { guild_id, screening, description, rules, reaccept, applications, questions }` | `Admission` | ≤ 16 rules of ≤ 300 chars (short Markdown); screening needs ≥ 1 rule; ≤ 5 questions (short ≤ 150, paragraph ≤ 1000, single choice with 2–10 options). Any change to the rules raises `rules_version`; `reaccept` then marks every member except Manage Server holders as pending again. Turning screening off releases everyone. |
| `AcceptRules { guild_id, rules_version }` | `Me` | `Conflict` when the version is stale (the member must read the new rules). |
| `Apply { via: Invite{code} \| Discovery{guild_id}, answers, accept_rules }` | `Application` | One application per person and server; 2/min per person; 500 waiting per server (`LimitExceeded`); a declined applicant may apply again after 24 h. |
| `Withdraw { guild_id }` | `Application` | Only a waiting application. |
| `Review { guild_id, user_id, approve, reason? }` | `Ack` | Approval adds the member (bans, the member cap and the applicant's server cap are checked again); audited with the reason. |
| `Interview { guild_id, user_id }` | `Channel` | A group conversation "Interview · <server>" with the applicant and the reviewer, made once; each later reviewer who opens it is added (≤ 16 people). |
| `SetOnboarding(Onboarding)` | `Onboarding` | ≤ 15 questions (≤ 5 before joining), ≤ 12 answers each (label ≤ 50, optional emoji, ≤ 5 roles, ≤ 10 channels); opt-in channel list; welcome screen (≤ 300 chars, ≤ 5 highlighted channels with a line each); guide (≤ 5 tasks: Read or Talk in a channel; ≤ 5 resource channels). The server numbers questions, answers and tasks. |
| `Respond { guild_id, responses, finish }` | `Me` | Validates every answer; adds the roles of new answers and removes those of dropped ones; adds and removes the answers' opt-in channels; `finish` requires the required questions and marks the flow done. |
| `SetChannels { guild_id, opted_in, show_all }` | `Me` | Only the server's opt-in channels. |
| `CompleteTask { guild_id, task_id }` | `Me` | Read tasks only, for a channel the member can see. Talk tasks are ticked by the server when the member sends a message there. |
| `JoinDiscovered { guild_id }` | `Guild` | A listed, qualifying server; applications servers refuse with `application`. Paused invites refuse it and joins count toward raid detection, as with invites. |
| `SetListing { guild_id, listing? }` | `Listing` | Category, description (≤ 300), ≤ 5 tags (≤ 20 chars, letters, digits, spaces, hyphens; lowercased), language tag, ≤ 5 featured custom emoji of the server. `None` removes the listing. ≤ 1,000 listings per instance. |
| `CreateTemplate`, `SyncTemplate { code }`, `DeleteTemplate { code }` | `Template` / `Ack` | ≤ 5 templates per server; 16-character share code. |
| `CreateFromTemplate { template: Code \| Builtin, name }` | `Guild` | 6/min per person. |

Events: `Updated { guild_id }` (admission or onboarding changed: fetch again,
to all members), `Me` (your state, your devices), `Application` (your
application changed), `Queue { guild_id, pending }` (reviewers).

## REST routes (cookie or bearer; reads only)

| Route | Who | Returns |
| --- | --- | --- |
| `GET /api/v1/guilds/{g}/community` | member | `CommunityState`: admission, onboarding, your state, waiting applications (reviewers only) |
| `GET /api/v1/guilds/{g}/applications` | Kick Members | `ApplicationQueue`: all waiting (oldest first, ≤ 500) and the latest 50 decided |
| `GET /api/v1/users/@me/applications` | anyone | your applications (≤ 50) |
| `GET /api/v1/invites/{code}/admission` | holder of the code | `JoinGate`: name, description, member count, rules (when screening), questions (when applying), your application |
| `GET /api/v1/directory?q=&category=` | anyone signed in | `DirectoryPage`: featured, recently active, results (≤ 24 each), category counts |
| `GET /api/v1/directory/{g}` | anyone signed in | `DirectoryPreview`: entry, join gate, welcome screen |
| `GET /api/v1/guilds/{g}/listing` | Manage Server | `ListingAdmin`: listing, requirement checklist, whether the directory is on |
| `GET /api/v1/guilds/{g}/templates` | Manage Server | templates, each with `out_of_sync` |
| `GET /api/v1/templates` | anyone signed in | the five starter templates |
| `GET /api/v1/templates/{code}` | holder of the code | the template, without its source server or creator |
| `GET /api/v1/guilds/{g}/insights?days=` | owner, View Audit Log or Manage Server | `Insights` for 1–120 days (default 30) |

A listed server's icon (`/api/v1/files/{id}/thumbnail`) and featured emoji
(`/api/v1/emoji/{id}`) can be loaded by anyone signed in, so the directory can
show them.

## Behavior

- **Screening is a timeout-style hold.** A member with `rules_pending` or
  `questions_pending` keeps only View Channel and Read History.
  `moderation::restrict` (used by `permissions::evaluate`) and the batched
  evaluator in `capacity.rs` apply it, so sending, reacting, typing, joining
  voice and speaking are refused everywhere; the moderation message check says
  why (`screening`), and its DM check refuses DMs to fellow members who are not
  friends while any shared server holds the person. The owner is never held.
  Members who joined before screening was turned on are not held unless the
  rules change with `reaccept`.
- **Required pre-join questions** hold a new member the same way until they
  are answered. Other questions never block.
- **Applicants are not members.** They have no access at all until approved.
  Answers are kept with the questions as they were asked. Decided
  applications are kept 30 days.
- **Role grants are checked twice:** when onboarding is saved (against the
  person saving it) and when a member answers (against the person who last
  saved it, at their current rank; a role that became dangerous or is now at or
  above that rank is skipped). Removing an answer removes only roles it gave
  that the member still has.
- **Opt-in channels.** A channel in `opt_in_channel_ids` is hidden from a
  member's channel list unless they chose it (an answer or Browse Channels) or
  turned on Show All Channels. It stays visible while it is open or has unread
  mentions, and mentions still notify. The owner, Administrators and people
  with Manage Channels always see every channel. Opt-in is a list preference, not a
  permission: the channel's permissions decide who may read it. Opt-in works
  whether or not the onboarding questions are turned on.
- **Directory.** A listing shows only while: the directory is on, the listing
  is not hidden or delisted, and the server still meets the requirements (rules
  screening with rules, or applications; a verification level; no
  age-restricted channel that is a default channel). The list, member counts,
  online counts (members with a live connection who are not invisible) and the
  week's message counts are rebuilt at most once a minute, only when someone
  opens the directory, or as soon as a listing changes. Search matches every
  word against the name and tags. Hidden servers cannot be joined from the
  directory; delisted ones cannot be listed again by their owner.
- **Templates** capture channels (nesting, kinds, topics, slowmode, age
  restriction), roles (except the reserved legacy Admin role, which every new
  server gets), role overwrites (member overwrites are dropped), admission,
  onboarding, AutoMod rules and the verification level, with role and channel
  references as local numbers. Building validates the snapshot again as
  untrusted input (every name, the 250-role and 500-channel limits, parents
  before children, nothing inside a forum, known permission bits only, no
  Administrator on @everyone, AutoMod's own validator and limits, onboarding's
  validator) inside the command's transaction: a server is created whole or
  not at all. `out_of_sync` compares a fresh capture with the stored one.
- **Insights** are daily rollups updated as events happen: joins (with their
  source: invite code, directory, template link), leaves (any removal),
  messages per channel (threads count toward their channel; AutoMod alerts do
  not), voice minutes per channel (counted when a session ends), and onboarding
  steps reached (joined, each question, rules, finished), once per member.
  Retention comes from a join log (`insight_joins`): of the people who joined
  on a day, how many were still members and had sent a message 1 and 7 days
  later, reported once the whole cohort is that old. Rows older than 120 days
  are dropped by the first event of a server each day; there is no timer.
  Reading insights never touches the messages table. Only counts are exposed.

## Web app

- **Server Settings › Community:** Rules Screening, Applications, Review
  Applications, Onboarding (with a preview that updates as fields change),
  Discovery (requirement checklist and listing form), Templates (create, copy
  link, sync, delete) and Insights (stat tiles; member growth, messages and
  voice as inline SVG bar charts by day or week, each with a table for screen
  readers; join sources and active channels as ranked bars; retention table;
  onboarding funnel with drop-off).
- **New members:** the onboarding flow opens by itself once per session when
  there is something to do (pre-join questions, rules, the other questions, the
  welcome screen). While held, the composer shows why, with a button back into
  the flow. The server menu has Channels & Roles, Browse Channels and Server
  Rules; the channel list has Server Guide, Channels & Roles and Browse
  Channels rows.
- **Join a Server** shows when the invite's server asks for an application
  (Apply to Join) or for rules. My Applications lists your applications with
  their status, withdraw, and the interview.
- **Discover** (compass at the bottom of the server rail): featured and
  recently active servers, categories, search, a preview with Join or Apply.
- **Create a Server** offers five starter templates (Friends, Gaming, Study
  Group, Club or Community, Project Team) and a template link field; a
  `?template=<code>` link opens the template's preview.
- `public/live.js` gained one generic hook: a form or field with
  `data-live-change="<action>"` sends that action when it changes.

## Decisions to revisit

- Rules are a list of short Markdown items shown on join; the member ticks one
  box for all of them (Discord's model), not one per rule.
- "Pre-join" questions are asked right after joining, before the member can
  talk, rather than before the membership exists.
- Members with a role are not exempt from screening (unlike the verification
  level), so onboarding roles cannot be used to skip the rules.
- A reviewer's interview is a group DM: it outlives the decision, and anyone
  in it can leave as with any group.
- Insights count the owner's join only for servers built from a template.
- The directory has no separate "online" ranking; recently active means
  messages in the last 7 days.
