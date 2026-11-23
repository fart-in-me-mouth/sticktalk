//! Onboarding: questions whose answers grant roles and opt members in to
//! channels, default and opt-in channels, the welcome screen and the server
//! guide. Roles are checked when the flow is configured and again when they
//! are granted: below the configuring person's top role, and never with a
//! dangerous right (`ONBOARDING_FORBIDDEN`).
use super::*;
use crate::moderation::audit;
use std::collections::BTreeSet;

/// Every channel of `g` that a channel list can show.
async fn channel_ids(c: &mut SqliteConnection, g: Id) -> Result<HashSet<Id>> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT id FROM channels WHERE guild_id=? AND kind<>'thread'")
            .bind(id(g)?)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .map(|v| Id(v as u64))
            .collect(),
    )
}
/// A role onboarding may grant: it exists, is not @everyone, has no
/// dangerous right, and (with a ceiling) is below it.
fn grantable(roles: &HashMap<Id, Role>, g: Id, r: Id, ceiling: Option<u16>) -> bool {
    roles.get(&r).is_some_and(|role| {
        r != g
            && !role.permissions.intersects(ONBOARDING_FORBIDDEN)
            && ceiling.is_none_or(|top| role.position < top)
    })
}
fn ids<T: Copy + Eq + std::hash::Hash>(list: &[T], max: usize) -> Result<()> {
    let unique: HashSet<&T> = list.iter().collect();
    require(
        list.len() <= max && unique.len() == list.len(),
        ErrorCode::InvalidRequest,
    )
}
/// Give every item a stable, unique, non-zero ID: keep valid ones, number the rest.
fn number<T>(items: &mut [T], get: impl Fn(&T) -> u32, set: impl Fn(&mut T, u32)) {
    let mut used = HashSet::new();
    let mut next = items.iter().map(&get).max().unwrap_or(0);
    for item in items.iter_mut() {
        let v = get(item);
        if v == 0 || !used.insert(v) {
            next += 1;
            set(item, next);
            used.insert(next);
        }
    }
}

/// Validate an onboarding document for server `g`. `ceiling` is the
/// configuring person's top role position (None for the owner).
pub(super) async fn validate(
    c: &mut SqliteConnection,
    g: Id,
    mut o: Onboarding,
    ceiling: Option<u16>,
) -> Result<Onboarding> {
    require(o.guild_id == g, ErrorCode::InvalidRequest)?;
    let roles: HashMap<Id, Role> = acl::roles(c, g)
        .await?
        .into_iter()
        .map(|r| (r.id, r))
        .collect();
    let channels = channel_ids(c, g).await?;
    let channel =
        |ch: &Id| -> Result<()> { require(channels.contains(ch), ErrorCode::InvalidRequest) };
    require(
        o.questions.len() <= MAX_ONBOARDING_QUESTIONS
            && o.questions.iter().filter(|q| q.pre_join).count() <= MAX_PRE_JOIN_QUESTIONS,
        ErrorCode::LimitExceeded,
    )?;
    number(&mut o.questions, |q| q.id, |q, v| q.id = v);
    for q in &mut o.questions {
        q.title = label(&q.title, MAX_ONBOARDING_TITLE_CHARS)?;
        require(
            !q.answers.is_empty() && q.answers.len() <= MAX_ONBOARDING_ANSWERS,
            ErrorCode::InvalidRequest,
        )?;
        number(&mut q.answers, |a| a.id, |a, v| a.id = v);
        for a in &mut q.answers {
            a.label = label(&a.label, MAX_ANSWER_LABEL_CHARS)?;
            a.emoji = emoji(a.emoji.take())?;
            ids(&a.role_ids, MAX_ANSWER_ROLES)?;
            ids(&a.channel_ids, MAX_ANSWER_CHANNELS)?;
            for r in &a.role_ids {
                if !grantable(&roles, g, *r, ceiling) {
                    let name = roles.get(r).map_or("That role".into(), |r| r.name.clone());
                    return Err(invalid(
                        "roles",
                        format!("{name} can’t be given by onboarding. Choose roles below your highest role, without moderator or admin rights."),
                    ));
                }
            }
            a.channel_ids.iter().try_for_each(channel)?;
        }
    }
    ids(&o.opt_in_channel_ids, MAX_CHANNELS_PER_GUILD)?;
    o.opt_in_channel_ids.iter().try_for_each(channel)?;
    o.welcome.description = text(&o.welcome.description, MAX_WELCOME_DESCRIPTION_CHARS, true)?;
    require(
        o.welcome.highlights.len() <= MAX_HIGHLIGHTS,
        ErrorCode::LimitExceeded,
    )?;
    for h in &mut o.welcome.highlights {
        channel(&h.channel_id)?;
        h.text = label(&h.text, MAX_HIGHLIGHT_CHARS)?;
        h.emoji = emoji(h.emoji.take())?;
    }
    require(
        o.tasks.len() <= MAX_GUIDE_TASKS && o.resources.len() <= MAX_GUIDE_RESOURCES,
        ErrorCode::LimitExceeded,
    )?;
    number(&mut o.tasks, |t| t.id, |t, v| t.id = v);
    for t in &mut o.tasks {
        channel(&t.channel_id)?;
        t.title = label(&t.title, MAX_GUIDE_TITLE_CHARS)?;
    }
    for r in &mut o.resources {
        channel(&r.channel_id)?;
        r.title = label(&r.title, MAX_GUIDE_TITLE_CHARS)?;
        r.description = text(&r.description, 120, false)?;
    }
    Ok(o)
}

pub(super) async fn set(
    c: &mut SqliteConnection,
    u: Id,
    o: Onboarding,
    out: &mut Deliveries,
) -> Result<CommunityResult> {
    let g = o.guild_id;
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    let rights = acl::evaluate(c, u, g, None).await?;
    let grants = o
        .questions
        .iter()
        .any(|q| q.answers.iter().any(|a| !a.role_ids.is_empty()));
    require(
        !grants || rights.contains(Permissions::MANAGE_ROLES),
        ErrorCode::Forbidden,
    )?;
    let ceiling = if rights.contains(Permissions::OWNER) {
        None
    } else {
        Some(acl::highest(c, g, u).await?)
    };
    let old = onboarding_settings(c, g).await?;
    let o = validate(c, g, o, ceiling).await?;
    save_onboarding(c, &o, u).await?;
    let on = |b: bool| Some(if b { "On" } else { "Off" }.to_owned());
    let n = |v: usize| Some(v.to_string());
    let entry = audit::Entry::new(
        g,
        AuditAction::OnboardingUpdate,
        AuditTarget::Guild,
        Some(g),
    )
    .named(guild_name(c, g).await?)
    .change("onboarding", on(old.enabled), on(o.enabled))
    .change("questions", n(old.questions.len()), n(o.questions.len()))
    .change(
        "opt-in channels",
        n(old.opt_in_channel_ids.len()),
        n(o.opt_in_channel_ids.len()),
    )
    .change("guide tasks", n(old.tasks.len()), n(o.tasks.len()))
    .change(
        "welcome highlights",
        n(old.welcome.highlights.len()),
        n(o.welcome.highlights.len()),
    );
    audit::write(c, u, entry).await?;
    emit(
        out,
        guild_users(c, g).await?,
        CommunityEvent::Updated { guild_id: g },
    );
    Ok(CommunityResult::Onboarding(Box::new(o)))
}

fn mapped<'a>(
    o: &'a Onboarding,
    responses: &'a [OnboardingResponse],
) -> impl Iterator<Item = &'a OnboardingAnswer> {
    responses.iter().flat_map(move |r| {
        o.questions
            .iter()
            .filter(move |q| q.id == r.question_id)
            .flat_map(move |q| {
                q.answers
                    .iter()
                    .filter(move |a| r.answer_ids.contains(&a.id))
            })
    })
}
fn answered(responses: &[OnboardingResponse], q: u32) -> bool {
    responses
        .iter()
        .any(|r| r.question_id == q && !r.answer_ids.is_empty())
}

/// Record that a member reached an onboarding step, once per member.
pub(super) async fn reach(c: &mut SqliteConnection, g: Id, u: Id, step: &str) -> Result<()> {
    let raw: Option<String> =
        sqlx::query_scalar("SELECT steps FROM community_members WHERE guild_id=? AND user_id=?")
            .bind(id(g)?)
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?;
    let Some(raw) = raw else { return Ok(()) };
    let mut steps: Vec<String> = parse(Some(raw));
    if steps.iter().any(|s| s == step) || steps.len() >= 32 {
        return Ok(());
    }
    steps.push(step.to_owned());
    sqlx::query("UPDATE community_members SET steps=? WHERE guild_id=? AND user_id=?")
        .bind(json(&steps)?)
        .bind(id(g)?)
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    insights::step(c, g, step).await
}

pub(super) async fn respond(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    mut responses: Vec<OnboardingResponse>,
    finish: bool,
    out: &mut Deliveries,
) -> Result<CommunityResult> {
    role(c, u, g).await?;
    let o = onboarding_settings(c, g).await?;
    require(o.enabled || responses.is_empty(), ErrorCode::InvalidRequest)?;
    ids(
        &responses.iter().map(|r| r.question_id).collect::<Vec<_>>(),
        MAX_ONBOARDING_QUESTIONS,
    )?;
    for r in &mut responses {
        let q = o
            .questions
            .iter()
            .find(|q| q.id == r.question_id)
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
        ids(&r.answer_ids, MAX_ONBOARDING_ANSWERS)?;
        require(
            (q.multi || r.answer_ids.len() <= 1)
                && r.answer_ids
                    .iter()
                    .all(|a| q.answers.iter().any(|x| x.id == *a)),
            ErrorCode::InvalidRequest,
        )?;
    }
    responses.retain(|r| !r.answer_ids.is_empty());
    if finish {
        if let Some(q) = o
            .questions
            .iter()
            .find(|q| q.required && !answered(&responses, q.id))
        {
            return Err(invalid(
                "responses",
                format!("Answer “{}” to continue.", q.title),
            ));
        }
    }
    let mut m = member_state(c, g, u).await?;
    let was_pending = m.rules_pending || m.questions_pending;
    // Roles: what the old answers gave and the new ones give.
    let old_roles: BTreeSet<Id> = mapped(&o, &m.responses)
        .flat_map(|a| a.role_ids.iter().copied())
        .collect();
    let new_roles: BTreeSet<Id> = mapped(&o, &responses)
        .flat_map(|a| a.role_ids.iter().copied())
        .collect();
    let roles: HashMap<Id, Role> = acl::roles(c, g)
        .await?
        .into_iter()
        .map(|r| (r.id, r))
        .collect();
    // Checked again now: the person who configured it may have lost rank.
    let configured_by: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT configured_by FROM community_settings WHERE guild_id=?",
    )
    .bind(id(g)?)
    .fetch_optional(&mut *c)
    .await?
    .flatten();
    let owner: i64 = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    let ceiling = match configured_by {
        Some(v) if v == owner => None,
        Some(v) if role(c, Id(v as u64), g).await.is_ok() => {
            Some(acl::highest(c, g, Id(v as u64)).await?)
        }
        _ => Some(0),
    };
    let current: BTreeSet<Id> = acl::role_ids(c, g, u).await?.into_iter().collect();
    let add: Vec<Id> = new_roles
        .difference(&old_roles)
        .filter(|r| !current.contains(r) && grantable(&roles, g, **r, ceiling))
        .copied()
        .collect();
    let remove: Vec<Id> = old_roles
        .difference(&new_roles)
        .filter(|r| current.contains(r) && grantable(&roles, g, **r, None))
        .copied()
        .collect();
    let before = channels(c, u, Some(g)).await?;
    for r in &add {
        sqlx::query("INSERT OR IGNORE INTO member_roles VALUES(?,?,?)")
            .bind(id(g)?)
            .bind(id(u)?)
            .bind(id(*r)?)
            .execute(&mut *c)
            .await?;
    }
    for r in &remove {
        sqlx::query("DELETE FROM member_roles WHERE guild_id=? AND user_id=? AND role_id=?")
            .bind(id(g)?)
            .bind(id(u)?)
            .bind(id(*r)?)
            .execute(&mut *c)
            .await?;
    }
    // Channels: drop what old answers opted in to, add what new ones do.
    let old_channels: BTreeSet<Id> = mapped(&o, &m.responses)
        .flat_map(|a| a.channel_ids.iter().copied())
        .collect();
    let new_channels: BTreeSet<Id> = mapped(&o, &responses)
        .flat_map(|a| a.channel_ids.iter().copied())
        .collect();
    m.opted_in
        .retain(|ch| !old_channels.contains(ch) || new_channels.contains(ch));
    for ch in &new_channels {
        if !m.opted_in.contains(ch) && m.opted_in.len() < MAX_CHANNELS_PER_GUILD {
            m.opted_in.push(*ch);
        }
    }
    if m.questions_pending
        && o.questions
            .iter()
            .filter(|q| q.pre_join && q.required)
            .all(|q| answered(&responses, q.id))
    {
        m.questions_pending = false;
    }
    let newly: Vec<u32> = responses
        .iter()
        .map(|r| r.question_id)
        .filter(|q| !answered(&m.responses, *q))
        .collect();
    m.responses = responses;
    m.onboarded |= finish;
    store(c, u, &m).await?;
    for q in newly {
        reach(c, g, u, &format!("q:{q}")).await?;
    }
    if finish {
        reach(c, g, u, "done").await?;
    }
    let released = was_pending && !m.rules_pending && !m.questions_pending;
    if !add.is_empty() || !remove.is_empty() || released {
        rights_changed(c, out, g, u).await?;
        let after = channels(c, u, Some(g)).await?;
        for ch in before
            .iter()
            .filter(|ch| !after.iter().any(|a| a.id == ch.id))
        {
            out.push((
                vec![u],
                Event::ChannelRemoved {
                    channel_id: ch.id,
                    reason: RemovalReason::Left,
                },
            ));
        }
        for ch in after
            .iter()
            .filter(|ch| !before.iter().any(|b| b.id == ch.id))
        {
            let mut full = channel(c, ch.id).await?;
            full.summary = ch.clone();
            out.push((vec![u], Event::ChannelUpsert(full)));
            if matches!(
                ch.kind,
                ChannelKind::Text | ChannelKind::Voice | ChannelKind::Announcement
            ) {
                init_read(c, u, ch.id, true).await?;
                out.push((vec![u], Event::ReadStateUpdated(read(c, u, ch.id).await?)));
            }
        }
    }
    if !add.is_empty() || !remove.is_empty() {
        let names = |list: &[Id]| {
            Some(
                list.iter()
                    .filter_map(|r| roles.get(r).map(|r| r.name.clone()))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
            .filter(|s| !s.is_empty())
        };
        let entry = audit::Entry::new(
            g,
            AuditAction::MemberRolesUpdate,
            AuditTarget::User,
            Some(u),
        )
        .named(user(c, u).await?.username)
        .change("roles added", None, names(&add))
        .change("roles removed", None, names(&remove))
        .reason(Some("Chosen in onboarding".into()));
        audit::write(c, u, entry).await?;
    }
    me_event(out, u, &m);
    Ok(CommunityResult::Me(Box::new(m)))
}

/// Browse Channels: which opt-in channels to show, and Show All Channels.
pub(super) async fn channels_choice(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    opted_in: Vec<Id>,
    show_all: bool,
    out: &mut Deliveries,
) -> Result<CommunityResult> {
    role(c, u, g).await?;
    ids(&opted_in, MAX_CHANNELS_PER_GUILD)?;
    let o = onboarding_settings(c, g).await?;
    require(
        opted_in.iter().all(|ch| o.opt_in_channel_ids.contains(ch)),
        ErrorCode::InvalidRequest,
    )?;
    let mut m = member_state(c, g, u).await?;
    m.opted_in = opted_in;
    m.show_all = show_all;
    store(c, u, &m).await?;
    me_event(out, u, &m);
    Ok(CommunityResult::Me(Box::new(m)))
}

/// A Read task of the server guide is done (the member opened its channel).
pub(super) async fn complete(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    task: u32,
    out: &mut Deliveries,
) -> Result<CommunityResult> {
    role(c, u, g).await?;
    let o = onboarding_settings(c, g).await?;
    let t = o
        .tasks
        .iter()
        .find(|t| t.id == task && t.kind == GuideTaskKind::Read)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    access(c, u, t.channel_id, Permissions::VIEW_CHANNEL).await?;
    let mut m = member_state(c, g, u).await?;
    if !m.guide_done.contains(&task) {
        m.guide_done.push(task);
        m.guide_done.truncate(MAX_GUIDE_TASKS);
        store(c, u, &m).await?;
        me_event(out, u, &m);
    }
    Ok(CommunityResult::Me(Box::new(m)))
}

/// After a message: tick the guide's "say hello" tasks for that channel.
pub(super) async fn talked(
    c: &mut SqliteConnection,
    g: Id,
    u: Id,
    ch: Id,
) -> Result<Option<MemberCommunity>> {
    let raw: Option<Option<String>> =
        sqlx::query_scalar("SELECT onboarding FROM community_settings WHERE guild_id=?")
            .bind(id(g)?)
            .fetch_optional(&mut *c)
            .await?;
    let Some(raw) = raw.flatten() else {
        return Ok(None);
    };
    let o: Onboarding = parse(Some(raw));
    let tasks: Vec<u32> = o
        .tasks
        .iter()
        .filter(|t| t.kind == GuideTaskKind::Talk && t.channel_id == ch)
        .map(|t| t.id)
        .collect();
    if tasks.is_empty() || role(c, u, g).await.is_err() {
        return Ok(None);
    }
    let mut m = member_state(c, g, u).await?;
    let before = m.guide_done.len();
    for t in tasks {
        if !m.guide_done.contains(&t) {
            m.guide_done.push(t);
        }
    }
    if m.guide_done.len() == before {
        return Ok(None);
    }
    m.guide_done.truncate(MAX_GUIDE_TASKS);
    store(c, u, &m).await?;
    Ok(Some(m))
}
