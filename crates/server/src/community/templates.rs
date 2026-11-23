//! Server templates: a server's structure only (channels with nesting, kinds,
//! topics, slowmode and age restriction; roles; role permission overwrites;
//! admission, onboarding and AutoMod settings; verification level). Never
//! members, messages, invites, emoji or uploaded files.
//!
//! A snapshot is untrusted input when a server is built from it (a shared link
//! or a stored row): `build` validates every field and bound again, maps every
//! role and channel reference, and runs inside the command transaction, so a
//! server is created whole or not at all.
use super::*;
use crate::moderation::{audit, automod};

fn ch_kind(kind: ChannelKind) -> bool {
    matches!(
        kind,
        ChannelKind::Category
            | ChannelKind::Text
            | ChannelKind::Voice
            | ChannelKind::Announcement
            | ChannelKind::Forum
    )
}
fn local(map: &HashMap<Id, u32>, v: &Id) -> Option<Id> {
    map.get(v).map(|l| Id(u64::from(*l)))
}

/// The structure of server `g`, with local references.
pub(super) async fn capture(c: &mut SqliteConnection, g: Id) -> Result<TemplateSnapshot> {
    // @everyone is local role 0; the others follow lowest first. The reserved
    // legacy Admin role is left out: every new server gets its own.
    let mut list = acl::roles(c, g).await?;
    list.retain(|r| r.id != Id(g.0 + 1));
    list.sort_by_key(|r| (r.id != g, r.position, r.id));
    let mut roles = vec![];
    let mut rmap: HashMap<Id, u32> = HashMap::new();
    for r in list {
        let local = roles.len() as u32;
        rmap.insert(r.id, local);
        roles.push(TemplateRole {
            id: local,
            name: r.name,
            color: r.color,
            hoist: r.hoist,
            mentionable: r.mentionable,
            permissions: r.permissions - Permissions::OWNER,
        });
    }
    // Channels, parents before children, in sidebar order.
    let rows = sqlx::query("SELECT id,kind,parent_id,name,topic,position FROM channels WHERE guild_id=? AND kind<>'thread' ORDER BY position,id")
        .bind(id(g)?)
        .fetch_all(&mut *c)
        .await?;
    let mut kids: HashMap<Option<Id>, Vec<Id>> = HashMap::new();
    let mut info: HashMap<Id, (ChannelKind, String, Option<String>)> = HashMap::new();
    let all: HashSet<Id> = rows.iter().map(|r| Id(r.get::<i64, _>(0) as u64)).collect();
    for r in &rows {
        let ch = Id(r.get::<i64, _>(0) as u64);
        let parent = r
            .get::<Option<i64>, _>(2)
            .map(|v| Id(v as u64))
            .filter(|p| all.contains(p));
        kids.entry(parent).or_default().push(ch);
        info.insert(
            ch,
            (
                en(r.get(1))?,
                r.get::<Option<String>, _>(3).unwrap_or_default(),
                r.get(4),
            ),
        );
    }
    let mut order = vec![];
    let mut stack: Vec<Id> = kids.get(&None).cloned().unwrap_or_default();
    stack.reverse();
    while let Some(ch) = stack.pop() {
        order.push(ch);
        if let Some(list) = kids.get(&Some(ch)) {
            stack.extend(list.iter().rev().copied());
        }
    }
    let cmap: HashMap<Id, u32> = order
        .iter()
        .enumerate()
        .map(|(i, ch)| (*ch, i as u32 + 1))
        .collect();
    let mut channels = vec![];
    for ch in &order {
        let (kind, name, topic) = info[ch].clone();
        let parent: Option<i64> = sqlx::query_scalar("SELECT parent_id FROM channels WHERE id=?")
            .bind(id(*ch)?)
            .fetch_one(&mut *c)
            .await?;
        let slowmode: Option<i64> =
            sqlx::query_scalar("SELECT seconds FROM channel_slowmode WHERE channel_id=?")
                .bind(id(*ch)?)
                .fetch_optional(&mut *c)
                .await?;
        let age: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM channel_age_restricted WHERE channel_id=?")
                .bind(id(*ch)?)
                .fetch_one(&mut *c)
                .await?;
        let overwrites = acl::overwrites(c, *ch)
            .await?
            .into_iter()
            .filter(|o| o.kind == OverwriteKind::Role)
            .filter_map(|o| {
                rmap.get(&o.target_id).map(|r| TemplateOverwrite {
                    role: *r,
                    allow: o.allow,
                    deny: o.deny,
                })
            })
            .collect();
        channels.push(TemplateChannel {
            id: cmap[ch],
            parent: parent.and_then(|p| cmap.get(&Id(p as u64)).copied()),
            kind,
            name,
            topic,
            slowmode: slowmode.unwrap_or(0) as u32,
            age_restricted: age > 0,
            overwrites,
        });
    }
    let a = admission_settings(c, g).await?;
    let admission = (a.screening || a.applications || !a.rules.is_empty()).then_some(Admission {
        guild_id: Id(0),
        ..a
    });
    let o = onboarding_settings(c, g).await?;
    let has_onboarding = o.enabled
        || !o.questions.is_empty()
        || !o.opt_in_channel_ids.is_empty()
        || !o.tasks.is_empty()
        || !o.welcome.highlights.is_empty();
    let onboarding = has_onboarding.then(|| remap_onboarding(o, &rmap, &cmap, Id(0)));
    let automod = automod::load(c, g)
        .await?
        .into_iter()
        .map(|mut r| {
            r.id = Id(0);
            r.guild_id = Id(0);
            r.exempt_roles = r
                .exempt_roles
                .iter()
                .filter_map(|v| local(&rmap, v))
                .collect();
            r.exempt_channels = r
                .exempt_channels
                .iter()
                .filter_map(|v| local(&cmap, v))
                .collect();
            r.actions.alert_channel_id = r.actions.alert_channel_id.and_then(|v| local(&cmap, &v));
            r
        })
        .collect();
    Ok(TemplateSnapshot {
        roles,
        channels,
        verification_level: crate::moderation::safety(c, g).await?.verification_level,
        admission,
        onboarding,
        automod,
    })
}
/// Rewrite every role and channel reference of `o` through the maps,
/// dropping references that do not map.
fn remap_onboarding(
    mut o: Onboarding,
    roles: &HashMap<Id, u32>,
    channels: &HashMap<Id, u32>,
    g: Id,
) -> Onboarding {
    o.guild_id = g;
    let ch = |v: &Id| local(channels, v);
    for q in &mut o.questions {
        for a in &mut q.answers {
            a.role_ids = a.role_ids.iter().filter_map(|v| local(roles, v)).collect();
            a.channel_ids = a.channel_ids.iter().filter_map(ch).collect();
        }
    }
    o.opt_in_channel_ids = o.opt_in_channel_ids.iter().filter_map(ch).collect();
    o.welcome
        .highlights
        .retain_mut(|h| match ch(&h.channel_id) {
            Some(v) => {
                h.channel_id = v;
                true
            }
            None => false,
        });
    o.tasks.retain_mut(|t| match ch(&t.channel_id) {
        Some(v) => {
            t.channel_id = v;
            true
        }
        None => false,
    });
    o.resources.retain_mut(|r| match ch(&r.channel_id) {
        Some(v) => {
            r.channel_id = v;
            true
        }
        None => false,
    });
    o
}

/// Check a snapshot against every bound before anything is written.
fn check(s: &TemplateSnapshot) -> Result<()> {
    let bad = || invalid("template", "This template is damaged or too large to use.");
    if s.roles.first().is_none_or(|r| r.id != 0) || s.roles.len() >= MAX_ROLES_PER_GUILD {
        return Err(bad());
    }
    let role_ids: HashSet<u32> = s.roles.iter().map(|r| r.id).collect();
    if role_ids.len() != s.roles.len() {
        return Err(bad());
    }
    for r in &s.roles {
        name(&r.name, 100).map_err(|_| bad())?;
        if r.color > 0xFF_FFFF {
            return Err(bad());
        }
    }
    if s.channels.len() > MAX_CHANNELS_PER_GUILD {
        return Err(bad());
    }
    let mut seen: HashMap<u32, ChannelKind> = HashMap::new();
    for ch in &s.channels {
        let parent_ok = ch.parent.is_none_or(|p| {
            seen.get(&p)
                .is_some_and(|k| !matches!(k, ChannelKind::Forum | ChannelKind::Thread))
        });
        let n = name(&ch.name, 64).map_err(|_| bad())?;
        let topic_ok = match &ch.topic {
            None => true,
            Some(t) => ch.kind != ChannelKind::Category && optional(Some(t.clone()), 512).is_ok(),
        };
        if ch.id == 0
            || seen.contains_key(&ch.id)
            || !parent_ok
            || !ch_kind(ch.kind)
            || n.contains(['/', '\\'])
            || !topic_ok
            || !SLOWMODE_SECONDS.contains(&ch.slowmode)
            || ch.overwrites.len() > MAX_CHANNEL_OVERWRITES
            || ch.overwrites.iter().any(|o| !role_ids.contains(&o.role))
            || ch
                .overwrites
                .iter()
                .map(|o| o.role)
                .collect::<HashSet<_>>()
                .len()
                != ch.overwrites.len()
        {
            return Err(bad());
        }
        seen.insert(ch.id, ch.kind);
    }
    let singles =
        |k: fn(&AutoModTrigger) -> bool| s.automod.iter().filter(|r| k(&r.trigger)).count() <= 1;
    if s.automod.len() > MAX_AUTOMOD_RULES
        || !singles(|t| matches!(t, AutoModTrigger::MentionSpam { .. }))
        || !singles(|t| matches!(t, AutoModTrigger::Spam { .. }))
    {
        return Err(bad());
    }
    Ok(())
}

/// Create a server owned by `u` from snapshot `s`.
#[allow(clippy::too_many_arguments)]
async fn create_guild(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    s: TemplateSnapshot,
    title: &str,
    keep_everyone: bool,
    source: JoinSource,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    check(&s)?;
    let title = name(title, 64)?;
    guild_quota(c, u).await?;
    let g = core.id().await?;
    sqlx::query("INSERT INTO guilds VALUES(?,?,?,NULL,?)")
        .bind(id(g)?)
        .bind(id(u)?)
        .bind(&title)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT INTO guild_members VALUES(?,?,'owner',?)")
        .bind(id(g)?)
        .bind(id(u)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    // Roles. @everyone may never be an administrator from a template.
    let known = |p: Permissions| Permissions::from_bits_truncate(p.bits()) - Permissions::OWNER;
    let mut rmap: HashMap<u32, Id> = HashMap::new();
    rmap.insert(0, g);
    if !keep_everyone {
        let everyone = &s.roles[0];
        sqlx::query(
            "UPDATE roles SET permissions=?,color=?,mentionable=? WHERE guild_id=? AND id=?",
        )
        .bind((known(everyone.permissions) - Permissions::ADMINISTRATOR).bits() as i64)
        .bind(i64::from(everyone.color))
        .bind(everyone.mentionable)
        .bind(id(g)?)
        .bind(id(g)?)
        .execute(&mut *c)
        .await?;
    }
    for (i, r) in s.roles.iter().enumerate().skip(1) {
        let mut rid = core.id().await?;
        // g+1 is the reserved legacy Admin role.
        while rid == Id(g.0 + 1) {
            rid = core.id().await?;
        }
        sqlx::query("INSERT INTO roles(guild_id,id,name,color,position,hoist,mentionable,permissions) VALUES(?,?,?,?,?,?,?,?)")
            .bind(id(g)?)
            .bind(id(rid)?)
            .bind(name(&r.name, 100)?)
            .bind(i64::from(r.color))
            .bind(i as i64)
            .bind(r.hoist)
            .bind(r.mentionable)
            .bind(known(r.permissions).bits() as i64)
            .execute(&mut *c)
            .await?;
        rmap.insert(r.id, rid);
    }
    sqlx::query("UPDATE roles SET position=? WHERE guild_id=? AND id=?")
        .bind(s.roles.len() as i64)
        .bind(id(g)?)
        .bind(id(Id(g.0 + 1))?)
        .execute(&mut *c)
        .await?;
    // Channels, parents first (checked), each placed after its siblings.
    let mut cmap: HashMap<u32, Id> = HashMap::new();
    let mut next: HashMap<Option<u32>, i64> = HashMap::new();
    for ch in &s.channels {
        let cid = core.id().await?;
        let position = next.entry(ch.parent).or_insert(0);
        sqlx::query("INSERT INTO channels(id,kind,guild_id,parent_id,name,topic,position,created_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(id(cid)?)
            .bind(enum_str(ch.kind))
            .bind(id(g)?)
            .bind(ch.parent.and_then(|p| cmap.get(&p)).map(|p| p.0 as i64))
            .bind(name(&ch.name, 64)?)
            .bind(optional(ch.topic.clone(), 512)?)
            .bind(*position)
            .bind(now() as i64)
            .execute(&mut *c)
            .await?;
        *position += 1;
        crate::threads::channel_created(c, cid, ch.kind).await?;
        if ch.slowmode > 0 && ch.kind != ChannelKind::Category {
            sqlx::query("INSERT INTO channel_slowmode(channel_id,seconds) VALUES(?,?)")
                .bind(id(cid)?)
                .bind(i64::from(ch.slowmode))
                .execute(&mut *c)
                .await?;
        }
        if ch.age_restricted && matches!(ch.kind, ChannelKind::Text | ChannelKind::Voice) {
            sqlx::query("INSERT INTO channel_age_restricted(channel_id) VALUES(?)")
                .bind(id(cid)?)
                .execute(&mut *c)
                .await?;
        }
        for o in &ch.overwrites {
            sqlx::query("INSERT INTO permission_overwrites VALUES(?,'role',?,?,?)")
                .bind(id(cid)?)
                .bind(id(rmap[&o.role])?)
                .bind(known(o.allow).bits() as i64)
                .bind(known(o.deny).bits() as i64)
                .execute(&mut *c)
                .await?;
        }
        cmap.insert(ch.id, cid);
    }
    if s.verification_level != VerificationLevel::None {
        sqlx::query("INSERT INTO guild_safety(guild_id,verification_level) VALUES(?,?)")
            .bind(id(g)?)
            .bind(enum_str(s.verification_level))
            .execute(&mut *c)
            .await?;
    }
    let role_of = |v: &Id| {
        rmap.get(&(v.0 as u32))
            .copied()
            .filter(|_| v.0 <= u64::from(u32::MAX))
    };
    let channel_of = |v: &Id| {
        cmap.get(&(v.0 as u32))
            .copied()
            .filter(|_| v.0 <= u64::from(u32::MAX))
    };
    if let Some(a) = s.admission {
        let a = admission::validate(Admission { guild_id: g, ..a }, &Admission::default())?;
        save_admission(c, &a).await?;
    }
    if let Some(mut o) = s.onboarding {
        o.guild_id = g;
        for q in &mut o.questions {
            for a in &mut q.answers {
                a.role_ids = a
                    .role_ids
                    .iter()
                    .filter_map(role_of)
                    .filter(|r| *r != g)
                    .collect();
                a.channel_ids = a.channel_ids.iter().filter_map(channel_of).collect();
            }
        }
        o.opt_in_channel_ids = o.opt_in_channel_ids.iter().filter_map(channel_of).collect();
        o.welcome.highlights.retain_mut(|h| {
            channel_of(&h.channel_id)
                .map(|v| h.channel_id = v)
                .is_some()
        });
        o.tasks.retain_mut(|t| {
            channel_of(&t.channel_id)
                .map(|v| t.channel_id = v)
                .is_some()
        });
        o.resources.retain_mut(|r| {
            channel_of(&r.channel_id)
                .map(|v| r.channel_id = v)
                .is_some()
        });
        let o = onboarding::validate(c, g, o, None).await?;
        save_onboarding(c, &o, u).await?;
    }
    for mut r in s.automod {
        r.guild_id = g;
        r.exempt_roles = r.exempt_roles.iter().filter_map(role_of).collect();
        r.exempt_channels = r.exempt_channels.iter().filter_map(channel_of).collect();
        r.actions.alert_channel_id = r.actions.alert_channel_id.and_then(|v| channel_of(&v));
        let mut r = automod::validate(c, g, r).await.map_err(|_| {
            invalid(
                "template",
                "This template has an AutoMod rule that can’t be used.",
            )
        })?;
        r.id = core.id().await?;
        sqlx::query("INSERT INTO automod_rules(id,guild_id,rule) VALUES(?,?,?)")
            .bind(id(r.id)?)
            .bind(id(g)?)
            .bind(json(&r)?)
            .execute(&mut *c)
            .await?;
    }
    admit(c, g, u, source, None, false).await?;
    crate::commands::joined(c, out, g, u).await?;
    Ok(CommandResult::Guild(detail(c, u, g).await?))
}

fn row(r: &sqlx::sqlite::SqliteRow) -> Result<GuildTemplate> {
    Ok(GuildTemplate {
        code: r.get("code"),
        name: r.get("name"),
        description: r.get("description"),
        source_guild_id: Some(Id(r.get::<i64, _>("source_guild_id") as u64)),
        creator_id: Some(Id(r.get::<i64, _>("creator_id") as u64)),
        created_at: r.get::<i64, _>("created_at") as u64,
        updated_at: r.get::<i64, _>("updated_at") as u64,
        uses: r.get::<i64, _>("uses") as u32,
        out_of_sync: false,
        snapshot: serde_json::from_str(&r.get::<String, _>("snapshot"))
            .map_err(|_| Error::new(ErrorCode::Internal))?,
    })
}
fn code_ok(code: &str) -> bool {
    code.len() == 16
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
pub(super) async fn by_code(c: &mut SqliteConnection, code: &str) -> Result<GuildTemplate> {
    if let Some(name) = code.strip_prefix("builtin:") {
        return builtin(name).ok_or_else(|| Error::new(ErrorCode::NotFound));
    }
    require(code_ok(code), ErrorCode::NotFound)?;
    let r = sqlx::query("SELECT * FROM guild_templates WHERE code=?")
        .bind(code)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    row(&r)
}
/// A server's templates, each marked when the server changed since its sync.
pub(super) async fn list(c: &mut SqliteConnection, u: Id, g: Id) -> Result<Vec<GuildTemplate>> {
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    let now_snapshot = capture(c, g).await?;
    let mut out = vec![];
    for r in
        sqlx::query("SELECT * FROM guild_templates WHERE source_guild_id=? ORDER BY created_at")
            .bind(id(g)?)
            .fetch_all(&mut *c)
            .await?
    {
        let mut t = row(&r)?;
        t.out_of_sync = t.snapshot != now_snapshot;
        out.push(t);
    }
    Ok(out)
}
fn template_entry(g: Id, action: AuditAction, t: &GuildTemplate) -> audit::Entry {
    audit::Entry::new(g, action, AuditTarget::Guild, Some(g))
        .named(t.name.clone())
        .change("template", None, Some(t.code.clone()))
}
pub(super) async fn create(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    title: String,
    description: String,
) -> Result<CommunityResult> {
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    let title = label(&title, 100)?;
    let description = text(&description, MAX_TEMPLATE_DESCRIPTION_CHARS, false)?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guild_templates WHERE source_guild_id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    require(
        (n as usize) < MAX_TEMPLATES_PER_GUILD,
        ErrorCode::LimitExceeded,
    )?;
    let snapshot = capture(c, g).await?;
    let code: String = crate::state::token().chars().take(16).collect();
    sqlx::query("INSERT INTO guild_templates(code,source_guild_id,creator_id,name,description,snapshot,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
        .bind(&code)
        .bind(id(g)?)
        .bind(id(u)?)
        .bind(&title)
        .bind(&description)
        .bind(json(&snapshot)?)
        .bind(now() as i64)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    let t = by_code(c, &code).await?;
    audit::write(c, u, template_entry(g, AuditAction::TemplateCreate, &t)).await?;
    Ok(CommunityResult::Template(Box::new(t)))
}
pub(super) async fn sync(c: &mut SqliteConnection, u: Id, code: &str) -> Result<CommunityResult> {
    let t = by_code(c, code).await?;
    let g = t
        .source_guild_id
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    role(c, u, g)
        .await
        .map_err(|_| Error::new(ErrorCode::NotFound))?;
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    let snapshot = capture(c, g).await?;
    sqlx::query("UPDATE guild_templates SET snapshot=?,updated_at=? WHERE code=?")
        .bind(json(&snapshot)?)
        .bind(now() as i64)
        .bind(code)
        .execute(&mut *c)
        .await?;
    let t = by_code(c, code).await?;
    audit::write(c, u, template_entry(g, AuditAction::TemplateUpdate, &t)).await?;
    Ok(CommunityResult::Template(Box::new(t)))
}
pub(super) async fn delete(c: &mut SqliteConnection, u: Id, code: &str) -> Result<CommunityResult> {
    let t = by_code(c, code).await?;
    let g = t
        .source_guild_id
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let manager = acl::evaluate(c, u, g, None)
        .await
        .is_ok_and(|p| p.contains(Permissions::MANAGE_GUILD));
    require(manager, ErrorCode::NotFound)?;
    sqlx::query("DELETE FROM guild_templates WHERE code=?")
        .bind(code)
        .execute(&mut *c)
        .await?;
    audit::write(c, u, template_entry(g, AuditAction::TemplateDelete, &t)).await?;
    Ok(CommunityResult::Ack)
}
pub(super) async fn build(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    template: TemplateRef,
    title: String,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    core.rate(format!("template-build:{}", u.0), 6., 3.)?;
    match template {
        TemplateRef::Builtin(name) => {
            let t = builtin(&name).ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            create_guild(
                c,
                core,
                u,
                t.snapshot,
                &title,
                true,
                JoinSource::Created,
                out,
            )
            .await
        }
        TemplateRef::Code(code) => {
            require(code_ok(&code), ErrorCode::NotFound)?;
            let t = by_code(c, &code).await?;
            sqlx::query("UPDATE guild_templates SET uses=uses+1 WHERE code=?")
                .bind(&code)
                .execute(&mut *c)
                .await?;
            create_guild(
                c,
                core,
                u,
                t.snapshot,
                &title,
                false,
                JoinSource::Template,
                out,
            )
            .await
        }
    }
}

// The five starter templates offered when creating a server.
pub(super) const BUILTIN: &[(&str, &str, &str)] = &[
    (
        "friends",
        "Friends",
        "A place to hang out with friends: chat, plans and voice.",
    ),
    (
        "gaming",
        "Gaming",
        "Squads, clips and looking-for-group, with rules for newcomers.",
    ),
    (
        "study",
        "Study Group",
        "Study rooms, shared resources and a homework help forum.",
    ),
    (
        "club",
        "Club or Community",
        "Welcome, announcements, events and a member role.",
    ),
    (
        "project",
        "Project Team",
        "Stand-ups, decisions and work channels for a team.",
    ),
];
fn r(id: u32, name: &str, color: u32, hoist: bool, permissions: Permissions) -> TemplateRole {
    TemplateRole {
        id,
        name: name.into(),
        color,
        hoist,
        mentionable: true,
        permissions,
    }
}
fn ch(id: u32, parent: Option<u32>, kind: ChannelKind, name: &str, topic: &str) -> TemplateChannel {
    TemplateChannel {
        id,
        parent,
        kind,
        name: name.into(),
        topic: (!topic.is_empty()).then(|| topic.into()),
        slowmode: 0,
        age_restricted: false,
        overwrites: vec![],
    }
}
/// Only moderators and the owner post here.
fn read_only(mut c: TemplateChannel) -> TemplateChannel {
    c.overwrites.push(TemplateOverwrite {
        role: 0,
        allow: Permissions::empty(),
        deny: Permissions::SEND_MESSAGES
            | Permissions::CREATE_PUBLIC_THREADS
            | Permissions::CREATE_PRIVATE_THREADS
            | Permissions::SEND_MESSAGES_IN_THREADS,
    });
    c
}
fn rules(list: &[&str]) -> Option<Admission> {
    Some(Admission {
        screening: true,
        description: "Please read these before you start talking.".into(),
        rules: list.iter().map(|r| (*r).into()).collect(),
        ..Default::default()
    })
}
const MODERATOR: Permissions = Permissions::from_bits_retain(
    Permissions::MANAGE_MESSAGES.bits()
        | Permissions::KICK_MEMBERS.bits()
        | Permissions::MODERATE_MEMBERS.bits()
        | Permissions::VIEW_AUDIT_LOG.bits()
        | Permissions::MANAGE_THREADS.bits()
        | Permissions::MUTE_MEMBERS.bits()
        | Permissions::MOVE_MEMBERS.bits(),
);
pub(super) fn builtin(key: &str) -> Option<GuildTemplate> {
    use ChannelKind::*;
    let (_, title, about) = BUILTIN.iter().find(|(k, _, _)| *k == key)?;
    let everyone = r(0, "@everyone", 0, false, Permissions::empty());
    let s = match key {
        "friends" => TemplateSnapshot {
            roles: vec![everyone],
            channels: vec![
                ch(1, None, Category, "Text Channels", ""),
                ch(2, Some(1), Text, "general", "Say hi!"),
                ch(3, Some(1), Text, "memes", ""),
                ch(4, Some(1), Text, "plans", "What are we doing this weekend?"),
                ch(5, None, Category, "Voice Channels", ""),
                ch(6, Some(5), Voice, "Lounge", ""),
                ch(7, Some(5), Voice, "Gaming", ""),
            ],
            ..Default::default()
        },
        "gaming" => TemplateSnapshot {
            roles: vec![
                everyone,
                r(1, "Squad", 0x3ba55d, true, Permissions::empty()),
                r(2, "Moderator", 0x5865f2, true, MODERATOR),
            ],
            channels: vec![
                ch(1, None, Category, "Info", ""),
                read_only(ch(2, Some(1), Text, "rules", "Read these first.")),
                ch(
                    3,
                    Some(1),
                    Announcement,
                    "announcements",
                    "Patch notes and events.",
                ),
                ch(4, None, Category, "Chat", ""),
                ch(5, Some(4), Text, "general", ""),
                ch(
                    6,
                    Some(4),
                    Text,
                    "clips-and-highlights",
                    "Show off your best plays.",
                ),
                ch(
                    7,
                    Some(4),
                    Text,
                    "looking-for-group",
                    "Find people to play with.",
                ),
                ch(8, None, Category, "Voice", ""),
                ch(9, Some(8), Voice, "Squad 1", ""),
                ch(10, Some(8), Voice, "Squad 2", ""),
            ],
            admission: rules(&[
                "Be respectful. No harassment, hate speech or slurs.",
                "No cheating, exploits or account selling.",
                "Keep clips and spoilers in their channels.",
            ]),
            ..Default::default()
        },
        "study" => TemplateSnapshot {
            roles: vec![
                everyone,
                r(1, "Tutor", 0xfaa61a, true, Permissions::empty()),
            ],
            channels: vec![
                ch(1, None, Category, "Study", ""),
                ch(2, Some(1), Text, "general", ""),
                ch(
                    3,
                    Some(1),
                    Text,
                    "resources",
                    "Notes, links and past papers.",
                ),
                ch(4, Some(1), Forum, "homework-help", "One post per question."),
                ch(5, None, Category, "Voice", ""),
                ch(6, Some(5), Voice, "Study Room", ""),
                ch(7, Some(5), Voice, "Quiet Focus", "Cameras on, mics off."),
            ],
            ..Default::default()
        },
        "club" => {
            let mut onboarding = Onboarding {
                enabled: true,
                questions: vec![OnboardingQuestion {
                    id: 1,
                    title: "What brings you here?".into(),
                    multi: true,
                    required: false,
                    pre_join: false,
                    answers: vec![
                        OnboardingAnswer {
                            id: 1,
                            label: "Events".into(),
                            emoji: Some("📅".into()),
                            role_ids: vec![Id(1)],
                            channel_ids: vec![Id(7)],
                        },
                        OnboardingAnswer {
                            id: 2,
                            label: "Meeting people".into(),
                            emoji: Some("👋".into()),
                            role_ids: vec![Id(1)],
                            channel_ids: vec![Id(6)],
                        },
                    ],
                }],
                opt_in_channel_ids: vec![Id(7)],
                ..Default::default()
            };
            onboarding.welcome = WelcomeScreen {
                description: "Welcome! Start here to find your way around.".into(),
                highlights: vec![
                    Highlight {
                        channel_id: Id(2),
                        text: "Read the rules and say hello".into(),
                        emoji: Some("👋".into()),
                    },
                    Highlight {
                        channel_id: Id(3),
                        text: "News from the organizers".into(),
                        emoji: Some("📣".into()),
                    },
                ],
            };
            onboarding.tasks = vec![
                GuideTask {
                    id: 1,
                    kind: GuideTaskKind::Read,
                    channel_id: Id(2),
                    title: "Read the welcome channel".into(),
                },
                GuideTask {
                    id: 2,
                    kind: GuideTaskKind::Talk,
                    channel_id: Id(6),
                    title: "Introduce yourself".into(),
                },
            ];
            TemplateSnapshot {
                roles: vec![
                    everyone,
                    r(1, "Member", 0x1abc9c, true, Permissions::empty()),
                    r(2, "Organizer", 0xe67e22, true, MODERATOR),
                ],
                channels: vec![
                    ch(1, None, Category, "Welcome", ""),
                    read_only(ch(2, Some(1), Text, "welcome", "Start here.")),
                    ch(3, Some(1), Announcement, "announcements", ""),
                    ch(4, None, Category, "Community", ""),
                    ch(5, Some(4), Text, "general", ""),
                    ch(6, Some(4), Text, "introductions", "Tell us about yourself."),
                    ch(7, Some(4), Text, "events", "What’s coming up."),
                    ch(8, None, Category, "Voice", ""),
                    ch(9, Some(8), Voice, "Hangout", ""),
                ],
                admission: rules(&[
                    "Be kind and welcoming.",
                    "Keep it safe for work.",
                    "Follow the organizers’ directions at events.",
                ]),
                onboarding: Some(onboarding),
                ..Default::default()
            }
        }
        "project" => TemplateSnapshot {
            roles: vec![
                everyone,
                r(1, "Contributor", 0x3498db, false, Permissions::empty()),
                r(2, "Lead", 0x9b59b6, true, MODERATOR),
            ],
            channels: vec![
                ch(1, None, Category, "Project", ""),
                ch(2, Some(1), Text, "general", ""),
                ch(3, Some(1), Text, "standup", "Yesterday, today, blockers."),
                read_only(ch(4, Some(1), Text, "decisions", "Record decisions here.")),
                ch(5, None, Category, "Work", ""),
                ch(6, Some(5), Text, "design", ""),
                ch(
                    7,
                    Some(5),
                    Forum,
                    "engineering",
                    "One post per task or bug.",
                ),
                ch(8, None, Category, "Voice", ""),
                ch(9, Some(8), Voice, "Meeting Room", ""),
            ],
            ..Default::default()
        },
        _ => return None,
    };
    Some(GuildTemplate {
        code: format!("builtin:{key}"),
        name: (*title).into(),
        description: (*about).into(),
        source_guild_id: None,
        creator_id: None,
        created_at: 0,
        updated_at: 0,
        uses: 0,
        out_of_sync: false,
        snapshot: s,
    })
}
