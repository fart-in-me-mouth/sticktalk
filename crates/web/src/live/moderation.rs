//! Moderation in the web app: per-server moderation state, the data behind the
//! moderation settings pages, and their commands. Views are in `ui/moderation.rs`.
//! Nothing here polls: state is fetched when a server or page is opened and
//! kept current by `Event::Moderation`.
use super::*;
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct State {
    /// The viewer's moderation state per server (bounded by the servers joined).
    pub states: BTreeMap<Id, ModerationState>,
    loading: BTreeSet<Id>,
    /// Slowmode of the open channel, and the local time it was read.
    pub slowmode: Option<(SlowmodeStatus, u64)>,
    pub audit: Option<Audit>,
    pub rules: Option<(Id, Vec<AutoModRule>)>,
    pub prune: Option<PrunePreview>,
    pub access: Option<ChannelAccess>,
    /// The overwrites being edited, for one channel.
    pub overwrites: Option<(Id, Vec<PermissionOverwrite>)>,
    /// A one-second ticker is updating the slowmode countdown.
    pub ticking: std::cell::Cell<bool>,
}
pub(super) struct Audit {
    pub guild: Id,
    pub items: Vec<AuditEntry>,
    pub users: BTreeMap<Id, UserSummary>,
    pub next: Option<Id>,
    pub more: bool,
    pub actor: Option<Id>,
    pub action: Option<AuditAction>,
    pub busy: bool,
}

fn now() -> u64 {
    Browser.unix_ms()
}
fn fail(app: &Shared, e: String) {
    let mut a = app.borrow_mut();
    a.error = e;
    a.touch();
}
fn moderation(a: &mut App, cmd: ModerationCommand) {
    a.command(Command::Launch(LaunchCommand::Moderation(cmd)));
}
fn read(id: &str) -> String {
    document()
        .get_element_by_id(id)
        .and_then(|e| {
            e.dyn_ref::<web_sys::HtmlInputElement>()
                .map(|i| i.value())
                .or_else(|| e.dyn_ref::<web_sys::HtmlSelectElement>().map(|s| s.value()))
                .or_else(|| {
                    e.dyn_ref::<web_sys::HtmlTextAreaElement>()
                        .map(|t| t.value())
                })
        })
        .unwrap_or_default()
}
fn checked(id: &str) -> bool {
    document()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|i| i.checked())
}
/// Which of `candidates` have their checkbox `{prefix}{id}` checked.
fn checked_ids(prefix: &str, candidates: impl Iterator<Item = Id>) -> Vec<Id> {
    candidates
        .filter(|v| checked(&format!("{prefix}{}", v.0)))
        .collect()
}
fn guild_roles(a: &App) -> Vec<Id> {
    a.roles
        .values()
        .filter(|r| Some(r.guild_id) == a.guild && r.id != r.guild_id)
        .map(|r| r.id)
        .collect()
}
/// The value of the checked radio button in group `name`.
fn radio(name: &str) -> String {
    document()
        .query_selector(&format!("input[name='{name}']:checked"))
        .ok()
        .flatten()
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|e| e.value())
        .unwrap_or_default()
}
fn reason(field: &str) -> Option<String> {
    Some(read(field).trim().to_owned()).filter(|r| !r.is_empty())
}

/// After every fresh snapshot: opt in to moderation events and refresh the open server.
pub(super) fn connected(app: Shared) {
    let guild = {
        let mut a = app.borrow_mut();
        moderation(&mut a, ModerationCommand::Subscribe);
        a.moderation.states.clear();
        a.moderation.loading.clear();
        a.guild
    };
    if let Some(g) = guild {
        refresh(app, g);
    }
}
/// Fetch the viewer's moderation state for server `g`.
pub(super) fn refresh(app: Shared, g: Id) {
    let (http, epoch) = {
        let mut a = app.borrow_mut();
        if !a.moderation.loading.insert(g) {
            return;
        }
        (a.http(), a.epoch)
    };
    spawn_local(async move {
        let result = http
            .json::<ModerationState>(&format!("/guilds/{}/moderation/me", g.0))
            .await;
        let mut a = app.borrow_mut();
        a.moderation.loading.remove(&g);
        if a.epoch != epoch || !a.model.guilds.contains_key(&g) {
            return;
        }
        if let Ok(state) = result {
            // The open channel's slowmode status waited for this state.
            let open = a
                .channel
                .filter(|c| state.slowmode.iter().any(|s| s.channel_id == *c));
            a.moderation.states.insert(g, state);
            a.touch();
            drop(a);
            if let Some(c) = open {
                let app = app.clone();
                spawn_local(async move { slowmode(app, c).await });
            }
        }
    });
}
/// A channel was opened: know its server's moderation state and its slowmode.
pub(super) fn channel_opened(app: Shared, c: Id) {
    let (guild, text) = {
        let mut a = app.borrow_mut();
        a.moderation.slowmode = None;
        let channel = a.model.channels.get(&c);
        (
            channel.and_then(|c| c.guild_id),
            channel.is_some_and(|c| c.kind == ChannelKind::Text),
        )
    };
    let Some(g) = guild else { return };
    // Only channels with slowmode have a status to ask for; the server's
    // state lists them (and asks for the open one when it arrives).
    let known = app
        .borrow()
        .moderation
        .states
        .get(&g)
        .map(|state| state.slowmode.iter().any(|s| s.channel_id == c));
    let slowed = match known {
        Some(slowed) => slowed,
        None => {
            refresh(app.clone(), g);
            false
        }
    };
    if !text || !slowed {
        return;
    }
    spawn_local(async move { slowmode(app, c).await });
}
async fn slowmode(app: Shared, c: Id) {
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    if let Ok(s) = http
        .json::<Snapshot<SlowmodeStatus>>(&format!("/channels/{}/slowmode", c.0))
        .await
    {
        let mut a = app.borrow_mut();
        if a.epoch == epoch && a.channel == Some(c) {
            a.moderation.slowmode = Some((s.data, now()));
            a.touch();
        }
    }
}
/// The channel is age-restricted and this viewer has not confirmed yet.
pub(super) fn gated(a: &App, c: Id) -> bool {
    a.model
        .channels
        .get(&c)
        .and_then(|ch| ch.guild_id)
        .and_then(|g| a.moderation.states.get(&g))
        .is_some_and(|s| s.age_restricted.contains(&c) && !s.age_confirmed.contains(&c))
}
pub(super) fn event(a: &mut App, e: &ModerationEvent) {
    match e {
        ModerationEvent::SafetyUpdated(s) => {
            if let Some(state) = a.moderation.states.get_mut(&s.guild_id) {
                let level = state.safety.verification_level != s.verification_level;
                state.safety = s.clone();
                // A new level changes how long this viewer must wait; ask again.
                if level {
                    if let Some(app) = a.owner.upgrade() {
                        let g = s.guild_id;
                        spawn_local(async move { refresh(app, g) });
                    }
                }
            }
        }
        ModerationEvent::TimeoutUpdated { guild_id, timeout } => {
            if let Some(state) = a.moderation.states.get_mut(guild_id) {
                state.timeout = timeout.clone();
            }
        }
        ModerationEvent::ChannelAgeRestricted {
            channel_id,
            age_restricted,
        } => {
            let guild = a.model.channels.get(channel_id).and_then(|c| c.guild_id);
            if let Some(state) = guild.and_then(|g| a.moderation.states.get_mut(&g)) {
                state.age_restricted.retain(|c| c != channel_id);
                if *age_restricted {
                    state.age_restricted.push(*channel_id);
                }
            }
        }
        ModerationEvent::SlowmodeUpdated {
            channel_id,
            seconds,
        } => {
            let guild = a.model.channels.get(channel_id).and_then(|c| c.guild_id);
            if let Some(state) = guild.and_then(|g| a.moderation.states.get_mut(&g)) {
                state.slowmode.retain(|s| s.channel_id != *channel_id);
                if *seconds > 0 {
                    state.slowmode.push(ChannelSlowmode {
                        channel_id: *channel_id,
                        seconds: *seconds,
                    });
                }
            }
            if *seconds == 0 && a.channel == Some(*channel_id) {
                a.moderation.slowmode = None;
                a.touch();
            } else if a.channel == Some(*channel_id) {
                if let Some(app) = a.owner.upgrade() {
                    let c = *channel_id;
                    spawn_local(async move { slowmode(app, c).await });
                }
            }
        }
        ModerationEvent::AutoModUpdated { guild_id } => {
            if a.moderation
                .rules
                .as_ref()
                .is_some_and(|(g, _)| g == guild_id)
            {
                a.moderation.rules = None;
                if let Some(app) = a.owner.upgrade() {
                    let g = *guild_id;
                    spawn_local(async move { load_rules(app, g).await });
                }
            }
        }
    }
}

/// A dialog opened: fetch what its page shows.
pub(super) fn opened(app: Shared, name: &str) {
    let guild = app.borrow().guild;
    let Some(g) = guild else { return };
    let name = name.to_owned();
    match name.as_str() {
        "audit-log" => audit(app, g, true),
        "automod" => {
            refresh_roles(app.clone(), g);
            spawn_local(async move { load_rules(app, g).await });
        }
        value if value.starts_with("automod-rule:") => {
            refresh_roles(app.clone(), g);
            if app
                .borrow()
                .moderation
                .rules
                .as_ref()
                .is_none_or(|(r, _)| *r != g)
            {
                spawn_local(async move { load_rules(app, g).await });
            }
        }
        "safety" => {
            app.borrow_mut().moderation.states.remove(&g);
            refresh(app, g);
        }
        "prune" => {
            app.borrow_mut().moderation.prune = None;
            refresh_roles(app.clone(), g);
            prune_preview(app, g, 7, vec![]);
        }
        value if value.starts_with("channel-perms:") => {
            if let Some(c) = id(value.trim_start_matches("channel-perms:")) {
                {
                    let mut a = app.borrow_mut();
                    a.moderation.overwrites = None;
                    a.moderation.access = None;
                }
                refresh_roles(app.clone(), g);
                load_perms(app, c);
            }
        }
        _ => {}
    }
}
fn refresh_roles(app: Shared, g: Id) {
    let http = app.borrow().http();
    spawn_local(async move {
        if let Ok(p) = http
            .json::<Snapshot<Vec<Role>>>(&format!("/guilds/{}/roles", g.0))
            .await
        {
            let mut a = app.borrow_mut();
            if !a.model.snapshot_valid(&p.cursor) {
                return;
            }
            a.roles.retain(|_, r| r.guild_id != g);
            for r in p.data {
                a.roles.insert(r.id, r);
            }
            a.touch();
        }
    });
}
async fn load_rules(app: Shared, g: Id) {
    let http = app.borrow().http();
    match http
        .json::<Vec<AutoModRule>>(&format!("/guilds/{}/automod/rules", g.0))
        .await
    {
        Ok(rules) => {
            let mut a = app.borrow_mut();
            if a.guild == Some(g) {
                a.moderation.rules = Some((g, rules));
                a.touch();
            }
        }
        Err(e) => fail(&app, e),
    }
}
fn audit(app: Shared, g: Id, reset: bool) {
    let (http, path) = {
        let mut a = app.borrow_mut();
        let keep = a
            .moderation
            .audit
            .as_ref()
            .filter(|v| v.guild == g && !reset);
        let (actor, action, before) = match keep {
            Some(v) => (v.actor, v.action, v.next),
            None => {
                let old = a.moderation.audit.take().filter(|v| v.guild == g);
                (
                    old.as_ref().and_then(|v| v.actor),
                    old.and_then(|v| v.action),
                    None,
                )
            }
        };
        if reset || a.moderation.audit.is_none() {
            a.moderation.audit = Some(Audit {
                guild: g,
                items: vec![],
                users: BTreeMap::new(),
                next: None,
                more: false,
                actor,
                action,
                busy: true,
            });
        }
        if let Some(v) = a.moderation.audit.as_mut() {
            v.busy = true;
        }
        let mut path = format!("/guilds/{}/audit-log?limit=50", g.0);
        if let Some(b) = before {
            path.push_str(&format!("&before={}", b.0));
        }
        if let Some(u) = actor {
            path.push_str(&format!("&actor={}", u.0));
        }
        if let Some(x) = action {
            path.push_str(&format!("&action={}", x.name()));
        }
        a.touch();
        (a.http(), path)
    };
    spawn_local(async move {
        let result = http.json::<AuditLogPage>(&path).await;
        let mut a = app.borrow_mut();
        let Some(view) = a.moderation.audit.as_mut().filter(|v| v.guild == g) else {
            return;
        };
        view.busy = false;
        match result {
            Ok(page) => {
                view.more = page.has_more;
                view.next = page.next_before;
                for u in page.users {
                    view.users.insert(u.id, u);
                }
                view.items.extend(page.items);
                a.touch();
            }
            Err(e) => {
                a.error = e;
                a.touch();
            }
        }
    });
}
fn prune_preview(app: Shared, g: Id, days: u32, roles: Vec<Id>) {
    let http = app.borrow().http();
    let list = roles
        .iter()
        .map(|r| r.0.to_string())
        .collect::<Vec<_>>()
        .join(",");
    spawn_local(async move {
        match http
            .json::<PrunePreview>(&format!("/guilds/{}/prune?days={days}&roles={list}", g.0))
            .await
        {
            Ok(p) => {
                let mut a = app.borrow_mut();
                a.moderation.prune = Some(p);
                a.touch();
            }
            Err(e) => fail(&app, e),
        }
    });
}
fn load_perms(app: Shared, c: Id) {
    let http = app.borrow().http();
    spawn_local(async move {
        let (o, x) = (
            format!("/channels/{}/overwrites", c.0),
            format!("/channels/{}/access", c.0),
        );
        let (overwrites, access) = futures_util::join!(
            http.json::<Snapshot<Vec<PermissionOverwrite>>>(&o),
            http.json::<ChannelAccess>(&x)
        );
        let mut a = app.borrow_mut();
        if a.dialog != format!("channel-perms:{}", c.0) {
            return;
        }
        match (overwrites, access) {
            (Ok(o), Ok(x)) => {
                // Keep unsaved edits when only the summary is refreshed.
                if a.moderation
                    .overwrites
                    .as_ref()
                    .is_none_or(|(ch, _)| *ch != c)
                {
                    a.moderation.overwrites = Some((c, o.data));
                }
                a.moderation.access = Some(x);
            }
            (Err(e), _) | (_, Err(e)) => a.error = e,
        }
        a.touch();
    });
}

/// `data-live="mod:<what>,<arg>"` actions.
pub(super) fn action(app: Shared, arg: &str) {
    let (what, rest) = arg.split_once(',').unwrap_or((arg, ""));
    let guild = app.borrow().guild;
    match what {
        "confirm-age" => {
            if let Some(c) = id(rest) {
                moderation(
                    &mut app.borrow_mut(),
                    ModerationCommand::ConfirmAge { channel_id: c },
                );
            }
        }
        "dismiss-raid" => {
            if let Some(g) = guild {
                moderation(
                    &mut app.borrow_mut(),
                    ModerationCommand::DismissRaidAlert { guild_id: g },
                );
            }
        }
        "remove-timeout" => {
            if let (Some(g), Some(u)) = (guild, id(rest)) {
                moderation(
                    &mut app.borrow_mut(),
                    ModerationCommand::Timeout {
                        guild_id: g,
                        user_id: u,
                        seconds: 0,
                        reason: None,
                    },
                );
            }
        }
        "resume" => {
            if let Some(g) = guild {
                let cmd = if rest == "invites" {
                    ModerationCommand::PauseInvites {
                        guild_id: g,
                        seconds: 0,
                    }
                } else {
                    ModerationCommand::PauseDms {
                        guild_id: g,
                        seconds: 0,
                    }
                };
                moderation(&mut app.borrow_mut(), cmd);
            }
        }
        "audit-more" => {
            if let Some(g) = guild {
                audit(app, g, false);
            }
        }
        "audit-filter" => {
            if let Some(g) = guild {
                {
                    let mut a = app.borrow_mut();
                    let actor = id(&read("audit-actor"));
                    let action = AuditAction::parse(&read("audit-action"));
                    if let Some(v) = a.moderation.audit.as_mut() {
                        v.actor = actor;
                        v.action = action;
                    }
                }
                audit(app, g, true);
            }
        }
        "prune-preview" => {
            if let Some(g) = guild {
                let days = read("prune-days").parse().unwrap_or(7);
                let roles = checked_ids("prune-role-", guild_roles(&app.borrow()).into_iter());
                prune_preview(app, g, days, roles);
            }
        }
        "rule-delete" => {
            if let (Some(g), Some(r)) = (guild, id(rest)) {
                moderation(
                    &mut app.borrow_mut(),
                    ModerationCommand::DeleteAutoModRule {
                        guild_id: g,
                        rule_id: r,
                    },
                );
            }
        }
        "rule-toggle" => {
            let rule = {
                let a = app.borrow();
                a.moderation
                    .rules
                    .as_ref()
                    .and_then(|(_, rules)| id(rest).and_then(|r| rules.iter().find(|x| x.id == r)))
                    .cloned()
            };
            if let Some(mut rule) = rule {
                rule.enabled = !rule.enabled;
                moderation(
                    &mut app.borrow_mut(),
                    ModerationCommand::UpdateAutoModRule(rule),
                );
            }
        }
        // Permission editor: add a role or member, remove one, or change one bit.
        "ow-add" => {
            let (kind, target) = rest.split_once(':').unwrap_or(("", ""));
            let target = id(target).or_else(|| id(&read(&format!("ow-add-{kind}"))));
            let mut a = app.borrow_mut();
            sync_radios(&mut a);
            if let (Some(t), Some((_, list))) = (target, a.moderation.overwrites.as_mut()) {
                let kind = if kind == "member" {
                    OverwriteKind::Member
                } else {
                    OverwriteKind::Role
                };
                if !list.iter().any(|o| o.kind == kind && o.target_id == t) {
                    list.push(PermissionOverwrite {
                        kind,
                        target_id: t,
                        allow: Permissions::empty(),
                        deny: Permissions::empty(),
                    });
                }
            }
            a.touch();
        }
        "ow-remove" => {
            let (kind, target) = rest.split_once(':').unwrap_or(("", ""));
            let mut a = app.borrow_mut();
            sync_radios(&mut a);
            if let (Some(t), Some((_, list))) = (id(target), a.moderation.overwrites.as_mut()) {
                list.retain(|o| {
                    !(o.target_id == t && (kind == "member") == (o.kind == OverwriteKind::Member))
                });
            }
            a.touch();
        }
        "ow-save" | "ow-sync" | "ow-private" => {
            let mut a = app.borrow_mut();
            sync_radios(&mut a);
            let Some((c, mut list)) = a.moderation.overwrites.clone() else {
                return;
            };
            match what {
                // Nested channels follow everything they are inside; dropping this
                // channel's own overrides makes it match its parent exactly.
                "ow-sync" => list.clear(),
                "ow-private" => {
                    let private = rest == "on";
                    let everyone = a.guild.unwrap_or_default();
                    let o = match list
                        .iter_mut()
                        .find(|o| o.kind == OverwriteKind::Role && o.target_id == everyone)
                    {
                        Some(o) => o,
                        None => {
                            list.push(PermissionOverwrite {
                                kind: OverwriteKind::Role,
                                target_id: everyone,
                                allow: Permissions::empty(),
                                deny: Permissions::empty(),
                            });
                            list.last_mut().expect("just pushed")
                        }
                    };
                    o.allow.remove(Permissions::VIEW_CHANNEL);
                    if private {
                        o.deny.insert(Permissions::VIEW_CHANNEL);
                    } else {
                        o.deny.remove(Permissions::VIEW_CHANNEL);
                    }
                }
                _ => {}
            }
            // Empty overrides are noise; drop them.
            list.retain(|o| !(o.allow | o.deny).is_empty());
            a.moderation.overwrites = Some((c, list.clone()));
            a.command(Command::Launch(LaunchCommand::SetOverwrites {
                channel_id: c,
                overwrites: list,
            }));
            media_action("form-sent:mod-perms");
        }
        _ => {}
    }
}

/// The permissions the channel editor offers, grouped as in role settings.
pub(super) const CHANNEL_PERMISSIONS: &[(&str, &[(&str, Permissions)])] = &[
    (
        "General",
        &[
            ("View Channel", Permissions::VIEW_CHANNEL),
            ("Manage Channel", Permissions::MANAGE_CHANNELS),
            ("Manage Permissions", Permissions::MANAGE_ROLES),
            ("Create Invites", Permissions::CREATE_INVITES),
        ],
    ),
    (
        "Text",
        &[
            ("Send Messages", Permissions::SEND_MESSAGES),
            ("Read Message History", Permissions::READ_HISTORY),
            ("Attach Files", Permissions::ATTACH_FILES),
            ("Embed Links", Permissions::EMBED_LINKS),
            ("Add Reactions", Permissions::ADD_REACTIONS),
            ("Use External Emoji", Permissions::USE_EXTERNAL_EMOJI),
            ("Use External Stickers", Permissions::USE_EXTERNAL_STICKERS),
            ("Mention @everyone and Roles", Permissions::MENTION_EVERYONE),
            ("Manage Messages", Permissions::MANAGE_MESSAGES),
            ("Publish Messages", Permissions::PUBLISH_MESSAGES),
        ],
    ),
    (
        "Threads",
        &[
            ("Create Public Threads", Permissions::CREATE_PUBLIC_THREADS),
            (
                "Create Private Threads",
                Permissions::CREATE_PRIVATE_THREADS,
            ),
            (
                "Send Messages in Threads",
                Permissions::SEND_MESSAGES_IN_THREADS,
            ),
            ("Manage Threads", Permissions::MANAGE_THREADS),
        ],
    ),
    (
        "Voice",
        &[
            ("Connect", Permissions::CONNECT),
            ("Speak", Permissions::SPEAK),
            ("Video", Permissions::VIDEO),
            ("Share Screen", Permissions::STREAM),
            ("Voice Activity", Permissions::USE_VOICE_ACTIVITY),
            ("Use Soundboard", Permissions::USE_SOUNDBOARD),
            ("Priority Speaker", Permissions::PRIORITY_SPEAKER),
            ("Mute Members", Permissions::MUTE_MEMBERS),
            ("Deafen Members", Permissions::DEAFEN_MEMBERS),
            ("Move Members", Permissions::MOVE_MEMBERS),
        ],
    ),
];
/// Whether a channel permission means anything for a channel of this kind.
/// Server-wide rights (Manage Soundboard, View Audit Log, Time Out Members,
/// Manage Server...) are only in role settings, like Discord.
pub(super) fn applies(bit: Permissions, kind: ChannelKind) -> bool {
    use ChannelKind::*;
    let voice = Permissions::CONNECT
        | Permissions::SPEAK
        | Permissions::VIDEO
        | Permissions::STREAM
        | Permissions::USE_VOICE_ACTIVITY
        | Permissions::USE_SOUNDBOARD
        | Permissions::PRIORITY_SPEAKER
        | Permissions::MUTE_MEMBERS
        | Permissions::DEAFEN_MEMBERS
        | Permissions::MOVE_MEMBERS;
    // Threads are offered in text and announcement channels, posts in forums;
    // not in voice-channel chat.
    let threads = Permissions::CREATE_PUBLIC_THREADS
        | Permissions::CREATE_PRIVATE_THREADS
        | Permissions::SEND_MESSAGES_IN_THREADS
        | Permissions::MANAGE_THREADS;
    match kind {
        Category => true,
        _ if bit == Permissions::PUBLISH_MESSAGES => kind == Announcement,
        _ if threads.contains(bit) => matches!(kind, Text | Announcement | Forum),
        _ if voice.contains(bit) => kind == Voice,
        _ => true,
    }
}
pub(super) fn radio_name(o: &PermissionOverwrite, bit: Permissions) -> String {
    format!(
        "ow-{}-{}-{}",
        if o.kind == OverwriteKind::Member {
            "member"
        } else {
            "role"
        },
        o.target_id.0,
        bit.bits()
    )
}
/// The permission radios are native controls (no render per click); copy their
/// choices into the edited list before it changes shape or is saved.
fn sync_radios(a: &mut App) {
    let Some((_, list)) = a.moderation.overwrites.as_mut() else {
        return;
    };
    for o in list.iter_mut() {
        for (_, bits) in CHANNEL_PERMISSIONS {
            for (_, bit) in bits.iter() {
                let selector = format!("input[name=\"{}\"]:checked", radio_name(o, *bit));
                let Ok(Some(e)) = document().query_selector(&selector) else {
                    continue;
                };
                let Some(e) = e.dyn_ref::<web_sys::HtmlInputElement>() else {
                    continue;
                };
                o.allow.remove(*bit);
                o.deny.remove(*bit);
                match e.value().as_str() {
                    "allow" => o.allow.insert(*bit),
                    "deny" => o.deny.insert(*bit),
                    _ => {}
                }
            }
        }
    }
}

/// `data-live-form="mod-..."` forms. Fields are read now, synchronously.
pub(super) fn submit(app: Shared, form: &str) {
    let mut a = app.borrow_mut();
    let Some(g) = a.guild else { return };
    let cmd = match form {
        "mod-timeout" => id(&read("mod-user")).map(|u| {
            let seconds = match radio("mod-duration-choice").as_str() {
                "custom" => {
                    let amount: u64 = read("mod-custom").parse().unwrap_or(0);
                    let unit: u64 = read("mod-unit").parse().unwrap_or(60);
                    amount
                        .saturating_mul(unit)
                        .min(u64::from(MAX_TIMEOUT_SECONDS)) as u32
                }
                v => v.parse().unwrap_or(0),
            };
            ModerationCommand::Timeout {
                guild_id: g,
                user_id: u,
                seconds,
                reason: reason("mod-reason"),
            }
        }),
        "mod-kick" => id(&read("mod-user")).map(|u| ModerationCommand::Kick {
            guild_id: g,
            user_id: u,
            reason: reason("mod-reason"),
        }),
        "mod-ban" => {
            if let Some(u) = id(&read("mod-user")) {
                a.command(Command::BanMember {
                    guild_id: g,
                    user_id: u,
                    reason: reason("mod-reason"),
                });
                media_action("form-sent:mod-ban");
            }
            return;
        }
        "mod-prune" => Some(ModerationCommand::Prune {
            guild_id: g,
            days: read("prune-days").parse().unwrap_or(7),
            role_ids: checked_ids("prune-role-", guild_roles(&a).into_iter()),
            reason: reason("mod-reason"),
        }),
        "mod-verification" => {
            let level = match radio("mod-level-choice").as_str() {
                "account_age" => VerificationLevel::AccountAge,
                "member_age" => VerificationLevel::MemberAge,
                _ => VerificationLevel::None,
            };
            Some(ModerationCommand::SetVerificationLevel { guild_id: g, level })
        }
        "mod-pause-invites" | "mod-pause-dms" => {
            let seconds = read(&format!("{form}-for")).parse().unwrap_or(3600);
            Some(if form == "mod-pause-invites" {
                ModerationCommand::PauseInvites {
                    guild_id: g,
                    seconds,
                }
            } else {
                ModerationCommand::PauseDms {
                    guild_id: g,
                    seconds,
                }
            })
        }
        "mod-rule" => rule_from_form(&a, g).map(|(rule, new)| {
            if new {
                ModerationCommand::CreateAutoModRule(rule)
            } else {
                ModerationCommand::UpdateAutoModRule(rule)
            }
        }),
        "mod-channel" => {
            // Channel settings: slowmode and age restriction save together.
            let Some(c) = id(&read("mod-channel")) else {
                return;
            };
            // Only text channels have slowmode.
            if let Ok(seconds) = read("mod-slowmode").parse() {
                a.command(Command::Launch(LaunchCommand::SetSlowmode {
                    channel_id: c,
                    seconds,
                }));
            }
            moderation(
                &mut a,
                ModerationCommand::SetAgeRestricted {
                    channel_id: c,
                    age_restricted: checked("mod-age"),
                },
            );
            media_action("form-sent:mod-channel");
            return;
        }
        _ => None,
    };
    match cmd {
        Some(cmd) => {
            moderation(&mut a, cmd);
            media_action(&format!("form-sent:{form}"));
        }
        None => {
            a.error = "Check the form and try again.".into();
            media_action(&format!("form-failed:{form}"));
            a.touch();
        }
    }
}
fn lines(id: &str) -> Vec<String> {
    read(id)
        .split([',', '\n'])
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}
fn rule_from_form(a: &App, g: Id) -> Option<(AutoModRule, bool)> {
    let existing = id(&read("rule-id"));
    let trigger = match read("rule-kind").as_str() {
        "mention_spam" => AutoModTrigger::MentionSpam {
            limit: read("rule-limit").parse().ok()?,
        },
        "spam" => AutoModTrigger::Spam {
            duplicates: read("rule-duplicates").parse().ok()?,
            links: read("rule-links").parse().ok()?,
        },
        _ => AutoModTrigger::Keyword {
            keywords: lines("rule-keywords"),
            allow: lines("rule-allow"),
        },
    };
    let enabled = existing
        .and_then(|r| {
            a.moderation
                .rules
                .as_ref()
                .and_then(|(_, rules)| rules.iter().find(|x| x.id == r))
        })
        .is_none_or(|r| r.enabled);
    Some((
        AutoModRule {
            id: existing.unwrap_or_default(),
            guild_id: g,
            name: read("rule-name"),
            enabled,
            trigger,
            actions: AutoModActions {
                block: checked("rule-block"),
                message: Some(read("rule-message")).filter(|m| !m.trim().is_empty()),
                alert_channel_id: id(&read("rule-alert")),
                timeout_seconds: read("rule-timeout").parse().unwrap_or(0),
            },
            exempt_roles: checked_ids("rule-role-", guild_roles(a).into_iter()),
            exempt_channels: checked_ids(
                "rule-channel-",
                a.model
                    .channels
                    .values()
                    .filter(|c| c.guild_id == Some(g))
                    .map(|c| c.id),
            ),
        },
        existing.is_none(),
    ))
}

/// Whether `answered` handles the answer to `op`: moderation commands, and the
/// launch commands the moderation pages send.
pub(super) fn owns(op: &Command, dialog: &str) -> bool {
    match op {
        Command::Launch(LaunchCommand::Moderation(_)) => true,
        Command::Launch(LaunchCommand::SetOverwrites { .. }) => {
            dialog.starts_with("channel-perms:")
        }
        Command::Launch(LaunchCommand::SetSlowmode { .. }) => dialog.starts_with("channel-edit:"),
        Command::BanMember { .. } => dialog.starts_with("mod-ban:"),
        _ => false,
    }
}
/// A refused command from a moderation page: its form may be submitted again.
pub(super) fn failed(op: Option<&Command>) {
    let form = match op {
        Some(Command::Launch(LaunchCommand::Moderation(cmd))) => match cmd {
            ModerationCommand::Timeout { .. } => "mod-timeout",
            ModerationCommand::Kick { .. } => "mod-kick",
            ModerationCommand::Prune { .. } => "mod-prune",
            ModerationCommand::SetVerificationLevel { .. } => "mod-verification",
            ModerationCommand::PauseInvites { .. } => "mod-pause-invites",
            ModerationCommand::PauseDms { .. } => "mod-pause-dms",
            ModerationCommand::CreateAutoModRule(_) | ModerationCommand::UpdateAutoModRule(_) => {
                "mod-rule"
            }
            ModerationCommand::SetAgeRestricted { .. } => "mod-channel",
            _ => return,
        },
        Some(Command::Launch(LaunchCommand::SetOverwrites { .. })) => "mod-perms",
        Some(Command::Launch(LaunchCommand::SetSlowmode { .. })) => "mod-channel",
        Some(Command::BanMember { .. }) => "mod-ban",
        _ => return,
    };
    media_action(&format!("form-failed:{form}"));
}
/// The answer to a command `owns` claimed.
pub(super) fn answered(app: Shared, op: Command, result: CommandResult) {
    let cmd = match op {
        Command::Launch(LaunchCommand::Moderation(cmd)) => cmd,
        Command::Launch(LaunchCommand::SetOverwrites { .. }) => {
            overwrites_saved(&app);
            app.borrow().touch();
            return;
        }
        Command::Launch(LaunchCommand::SetSlowmode { .. }) => {
            media_action("form-saved:mod-channel");
            let mut a = app.borrow_mut();
            a.saved = Some((a.dialog_generation, "mod-channel"));
            a.touch();
            return;
        }
        _ => {
            // A ban from the member menu.
            media_action("form-saved:mod-ban");
            let mut a = app.borrow_mut();
            // Only the dialog that asked; another may have opened meanwhile.
            if a.dialog.starts_with("mod-") || a.dialog == "profile" {
                a.dialog.clear();
                a.profile = None;
            }
            a.notice = "Member banned.".into();
            a.error = a.notice.clone();
            a.touch();
            return;
        }
    };
    let mut a = app.borrow_mut();
    a.error.clear();
    a.notice.clear();
    let mut notice = "";
    let guild = a.guild;
    match (&cmd, result) {
        (ModerationCommand::Subscribe, _) => return,
        (ModerationCommand::ConfirmAge { channel_id }, _) => {
            let c = *channel_id;
            let guild = a.model.channels.get(&c).and_then(|ch| ch.guild_id);
            if let Some(s) = guild.and_then(|g| a.moderation.states.get_mut(&g)) {
                s.age_confirmed.push(c);
            }
            drop(a);
            operations::load_channel(app, c, None);
            return;
        }
        (ModerationCommand::Timeout { seconds, .. }, _) => {
            media_action("form-saved:mod-timeout");
            notice = if *seconds == 0 {
                "Timeout removed."
            } else {
                "Member timed out."
            };
            // Only the dialog that asked; another may have opened meanwhile.
            if a.dialog.starts_with("mod-") || a.dialog == "profile" {
                a.dialog.clear();
                a.profile = None;
            }
        }
        (ModerationCommand::Kick { .. }, _) => {
            media_action("form-saved:mod-kick");
            notice = "Member kicked.";
            // Only the dialog that asked; another may have opened meanwhile.
            if a.dialog.starts_with("mod-") || a.dialog == "profile" {
                a.dialog.clear();
                a.profile = None;
            }
        }
        (
            ModerationCommand::Prune { .. },
            CommandResult::Moderation(ModerationResult::Pruned { count }),
        ) => {
            a.error = format!(
                "Removed {count} inactive member{}.",
                if count == 1 { "" } else { "s" }
            );
            a.notice = a.error.clone();
            media_action("form-saved:mod-prune");
            a.moderation.prune = None;
            a.touch();
            drop(a);
            if let Some(g) = guild {
                let days = read("prune-days").parse().unwrap_or(7);
                let roles = checked_ids("prune-role-", guild_roles(&app.borrow()).into_iter());
                prune_preview(app, g, days, roles);
            }
            return;
        }
        (
            ModerationCommand::SetVerificationLevel { .. }
            | ModerationCommand::PauseInvites { .. }
            | ModerationCommand::PauseDms { .. }
            | ModerationCommand::DismissRaidAlert { .. },
            CommandResult::Moderation(ModerationResult::Safety(s)),
        ) => {
            if let Some(state) = a.moderation.states.get_mut(&s.guild_id) {
                let raid = state.safety.raid.clone();
                state.safety = s;
                if !matches!(cmd, ModerationCommand::DismissRaidAlert { .. }) {
                    state.safety.raid = state.safety.raid.clone().or(raid);
                }
            }
            let form = match cmd {
                ModerationCommand::SetVerificationLevel { .. } => "mod-verification",
                ModerationCommand::PauseInvites { .. } => "mod-pause-invites",
                ModerationCommand::PauseDms { .. } => "mod-pause-dms",
                _ => "",
            };
            if !form.is_empty() {
                media_action(&format!("form-saved:{form}"));
                a.saved = Some((a.dialog_generation, form));
            }
        }
        (
            ModerationCommand::CreateAutoModRule(_) | ModerationCommand::UpdateAutoModRule(_),
            CommandResult::Moderation(ModerationResult::AutoModRule(rule)),
        ) => {
            if let Some((g, rules)) = a.moderation.rules.as_mut() {
                if *g == rule.guild_id {
                    rules.retain(|r| r.id != rule.id);
                    rules.push(rule);
                    rules.sort_by_key(|r| r.id);
                }
            }
            if a.dialog.starts_with("automod-rule:") {
                media_action("form-saved:mod-rule");
                a.dialog = "automod".into();
                a.dialog_generation = a.dialog_generation.wrapping_add(1);
            }
        }
        (ModerationCommand::DeleteAutoModRule { rule_id, .. }, _) => {
            if let Some((_, rules)) = a.moderation.rules.as_mut() {
                rules.retain(|r| r.id != *rule_id);
            }
            if a.dialog.starts_with("automod-rule:") {
                a.dialog = "automod".into();
                a.dialog_generation = a.dialog_generation.wrapping_add(1);
            }
        }
        (ModerationCommand::SetAgeRestricted { .. }, _) => {
            media_action("form-saved:mod-channel");
            a.saved = Some((a.dialog_generation, "mod-channel"));
        }
        _ => {}
    }
    if !notice.is_empty() {
        a.notice = notice.into();
        a.error = notice.into();
    }
    a.touch();
}
/// Saving channel permissions succeeded: refresh who can see the channel.
fn overwrites_saved(app: &Shared) {
    media_action("form-saved:mod-perms");
    let dialog = app.borrow().dialog.clone();
    if let Some(c) = id(dialog.trim_start_matches("channel-perms:")) {
        let mut a = app.borrow_mut();
        a.notice = "Channel permissions saved.".into();
        a.error = a.notice.clone();
        drop(a);
        load_perms(app.clone(), c);
    }
}

/// Keeps the composer's slowmode countdown current, once a second, only while
/// there is a cooldown on screen; then renders once and stops.
pub(super) fn tick(a: &App) {
    if a.moderation.ticking.replace(true) {
        return;
    }
    let owner = a.owner.clone();
    spawn_local(async move {
        loop {
            Browser.sleep(1000).await;
            let Some(app) = owner.upgrade() else { return };
            let a = app.borrow();
            let left = a.channel.and_then(|c| cooldown(&a, c)).unwrap_or(0);
            if left == 0 {
                a.moderation.ticking.set(false);
                a.touch();
                return;
            }
            if let Some(e) = document().get_element_by_id("slowmode-countdown") {
                e.set_text_content(Some(&clock(left)));
            } else {
                a.moderation.ticking.set(false);
                return;
            }
        }
    });
}
/// Milliseconds until the viewer may send in channel `c` again under slowmode.
pub(super) fn cooldown(a: &App, c: Id) -> Option<u64> {
    let (s, fetched) = a.moderation.slowmode.as_ref()?;
    if s.channel_id != c || s.seconds == 0 || s.bypassed {
        return None;
    }
    // Server time differs from this clock; shift by what it said when asked.
    let skew = s.server_time as i64 - *fetched as i64;
    let server_now = (now() as i64 + skew).max(0) as u64;
    let from_status = (s.server_time + u64::from(s.retry_after_ms)).saturating_sub(server_now);
    let me = a.model.me.as_ref().map(|m| m.user.id);
    let last_own = a
        .model
        .messages
        .get(&c)
        .and_then(|list| list.values().rev().find(|m| Some(m.author_id) == me))
        .map(|m| m.created_at);
    let from_send = last_own
        .map(|t| (t + u64::from(s.seconds) * 1000).saturating_sub(server_now))
        .unwrap_or(0)
        .min(u64::from(s.seconds) * 1000);
    let left = from_status.max(from_send);
    (left > 0).then_some(left)
}
pub(super) fn clock(ms: u64) -> String {
    let s = ms.div_ceil(1000);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
