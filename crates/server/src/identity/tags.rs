//! Server tags: a server defines one, its members choose to wear it, and it
//! shows next to the wearer's name everywhere. Text is checked for shape,
//! the reserved list, the administrator's blocked list, uniqueness across
//! the instance (ignoring case) and the server's own AutoMod keyword rules.
use super::*;
use std::collections::HashSet;

/// Wearers told about a tag change per batch (`identity::fanout`), each batch
/// under the command gate for a moment, with no transaction held open.
pub(super) const ANNOUNCE_BATCH: i64 = 64;

pub(super) async fn load(c: &mut SqliteConnection, g: Id) -> Result<Option<TagEdit>> {
    let s: Option<String> = sqlx::query_scalar("SELECT tag FROM server_tags WHERE guild_id=?")
        .bind(id(g)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(parse(s))
}
pub(super) async fn has_tag(c: &mut SqliteConnection, g: Id) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM server_tags WHERE guild_id=?)")
            .bind(id(g)?)
            .fetch_one(&mut *c)
            .await?,
    )
}
/// An invite of `g` that can still be used, by code (vanity codes too).
pub(super) async fn usable_invite(c: &mut SqliteConnection, g: Id, code: &str) -> Result<bool> {
    let Some(i) = find_invite(c, code).await? else {
        return Ok(false);
    };
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM invites WHERE id=? AND guild_id=? AND expires_at>? AND uses<max_uses)",
    )
    .bind(i)
    .bind(id(g)?)
    .bind(now() as i64)
    .fetch_one(&mut *c)
    .await?)
}

async fn validate(c: &mut SqliteConnection, core: &mut Core, g: Id, e: TagEdit) -> Result<TagEdit> {
    let text = e.text.trim().to_owned();
    if let Some(problem) = tag_problem(&text) {
        return Err(invalid("tag", problem));
    }
    let key = tag_key(&text);
    let blocked: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM blocked_tags WHERE tag_key=?)")
            .bind(&key)
            .fetch_one(&mut *c)
            .await?;
    if reserved_tag(&text) || blocked {
        return Err(invalid("tag", "That tag is reserved. Choose another one."));
    }
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM server_tags WHERE tag_key=? AND guild_id<>?)",
    )
    .bind(&key)
    .bind(id(g)?)
    .fetch_one(&mut *c)
    .await?;
    if taken {
        let mut err = Error::new(ErrorCode::AlreadyExists);
        err.0.field = Some("tag".into());
        err.0.message = "Another server already uses that tag.".into();
        return Err(err);
    }
    if !known_glyph(&e.glyph) {
        return Err(invalid("glyph", "Choose a badge from the list."));
    }
    if e.color > 0xFF_FFFF || e.color2 > 0xFF_FFFF {
        return Err(invalid("color", "Colors must be RGB."));
    }
    if contrast(e.color, e.color2) < MIN_GLYPH_CONTRAST {
        return Err(invalid(
            "color",
            "The badge’s two colors are too close. Make one lighter or darker.",
        ));
    }
    let invite_code = match e
        .invite_code
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
    {
        Some(code) => {
            if !usable_invite(c, g, &code).await? {
                return Err(invalid("invite", "Choose one of this server’s invites."));
            }
            Some(
                vanity_code(&code)
                    .filter(|_| code.len() != 22)
                    .unwrap_or(code),
            )
        }
        None => None,
    };
    // The server's own keyword rules apply to its tag, with no exemptions:
    // only people with Manage Server can set it, and they are the ones the
    // rules would otherwise exempt.
    let rules = crate::moderation::automod::rules(&mut core.moderation, c, g).await?;
    if let Some(hit) = crate::moderation::automod::check(
        &rules,
        &[],
        &[],
        &crate::moderation::automod::Draft {
            content: &text,
            mentions: &[],
            repeats: 0,
        },
    )
    .into_iter()
    .find(|h| matches!(h.rule.trigger, AutoModTrigger::Keyword { .. }) && h.rule.actions.block)
    {
        let mut err = Error::new(ErrorCode::Forbidden);
        err.0.field = Some("automod".into());
        err.0.message = format!("This server’s AutoMod blocks that tag because {}.", hit.why);
        return Err(err);
    }
    Ok(TagEdit {
        text,
        glyph: e.glyph,
        color: e.color,
        color2: e.color2,
        invite_code,
    })
}

pub(super) async fn set(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    g: Id,
    tag: Option<TagEdit>,
    _out: &mut Deliveries,
) -> Result<CommandResult> {
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    core.rate(format!("tag:{}", g.0), 6., 3.)?;
    let old = load(c, g).await?;
    let new = match tag {
        Some(e) => Some(validate(c, core, g, e).await?),
        None => None,
    };
    if old == new {
        return Ok(CommandResult::Ack);
    }
    match &new {
        Some(t) => {
            sqlx::query("INSERT INTO server_tags(guild_id,tag_key,tag,updated_at) VALUES(?,?,?,?) ON CONFLICT(guild_id) DO UPDATE SET tag_key=excluded.tag_key,tag=excluded.tag,updated_at=excluded.updated_at")
                .bind(id(g)?)
                .bind(tag_key(&t.text))
                .bind(json(t)?)
                .bind(now() as i64)
                .execute(&mut *c)
                .await?;
        }
        None => {
            sqlx::query("DELETE FROM server_tags WHERE guild_id=?")
                .bind(id(g)?)
                .execute(&mut *c)
                .await?;
        }
    }
    let worn = |t: &Option<TagEdit>| t.as_ref().map(|t| t.worn(g));
    if worn(&old) != worn(&new) {
        // The copy every wearer carries changes with it, and every wearer's
        // viewers are told, in batches once this commits (`fanout`).
        sqlx::query(
            "INSERT OR IGNORE INTO tag_announce(user_id) SELECT id FROM users WHERE tag_guild=?",
        )
        .bind(id(g)?)
        .execute(&mut *c)
        .await?;
        core.identity.wake.notify_one();
        match worn(&new) {
            Some(w) => {
                sqlx::query("UPDATE users SET tag=? WHERE tag_guild=?")
                    .bind(json(&w)?)
                    .bind(id(g)?)
                    .execute(&mut *c)
                    .await?
            }
            None => {
                sqlx::query("UPDATE users SET tag=NULL,tag_guild=NULL WHERE tag_guild=?")
                    .bind(id(g)?)
                    .execute(&mut *c)
                    .await?
            }
        };
    }
    let describe = |t: &Option<TagEdit>, f: fn(&TagEdit) -> String| t.as_ref().map(f);
    let entry = audit::Entry::new(g, AuditAction::ServerTagUpdate, AuditTarget::Guild, Some(g))
        .change(
            "tag",
            describe(&old, |t| t.text.clone()),
            describe(&new, |t| t.text.clone()),
        )
        .change(
            "badge",
            describe(&old, |t| t.glyph.clone()),
            describe(&new, |t| t.glyph.clone()),
        )
        .change(
            "colors",
            describe(&old, |t| format!("#{:06x} #{:06x}", t.color, t.color2)),
            describe(&new, |t| format!("#{:06x} #{:06x}", t.color, t.color2)),
        )
        .change(
            "join_invite",
            old.as_ref()
                .and_then(|t| t.invite_code.as_ref().map(|_| "set".to_owned())),
            new.as_ref()
                .and_then(|t| t.invite_code.as_ref().map(|_| "set".to_owned())),
        );
    audit::write(c, u, entry).await?;
    Ok(CommandResult::Ack)
}

pub(super) async fn wear(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
    g: Option<Id>,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    let current: Option<i64> = sqlx::query_scalar("SELECT tag_guild FROM users WHERE id=?")
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    match g {
        Some(g) => {
            role(c, u, g).await?;
            let tag = load(c, g)
                .await?
                .ok_or_else(|| invalid("tag", "That server has no tag."))?;
            let worn = json(&tag.worn(g))?;
            let stored: Option<String> = sqlx::query_scalar("SELECT tag FROM users WHERE id=?")
                .bind(id(u)?)
                .fetch_one(&mut *c)
                .await?;
            if current == Some(id(g)?) && stored.as_deref() == Some(worn.as_str()) {
                return Ok(CommandResult::Ack);
            }
            sqlx::query("UPDATE users SET tag=?,tag_guild=? WHERE id=?")
                .bind(worn)
                .bind(id(g)?)
                .bind(id(u)?)
                .execute(&mut *c)
                .await?;
        }
        None => {
            if current.is_none() {
                return Ok(CommandResult::Ack);
            }
            sqlx::query("UPDATE users SET tag=NULL,tag_guild=NULL WHERE id=?")
                .bind(id(u)?)
                .execute(&mut *c)
                .await?;
        }
    }
    crate::profiles::user_changed(c, core, u, out).await?;
    Ok(CommandResult::Ack)
}

/// `u`'s summary changed (their tag): tell their devices and everyone online
/// who can see them. One query for the audience, not one per viewer.
pub(super) async fn announce(
    c: &mut SqliteConnection,
    core: &Core,
    u: Id,
    out: &mut Deliveries,
) -> Result<()> {
    let me = me(c, u).await?;
    out.push((vec![u], Event::SelfUpdated(me.clone())));
    let online: HashSet<Id> = core.streams.values().map(|s| s.user).collect();
    let viewers: Vec<i64> = sqlx::query_scalar("SELECT other_id FROM relationships WHERE user_id=?1 UNION SELECT b.user_id FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.user_id=?1 UNION SELECT b.user_id FROM dm_members a JOIN dm_members b ON a.channel_id=b.channel_id WHERE a.user_id=?1")
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?;
    let mut to: Vec<Id> = viewers
        .into_iter()
        .map(|v| Id(v as u64))
        .filter(|v| *v != u && online.contains(v))
        .collect();
    to.sort();
    to.dedup();
    if !to.is_empty() {
        out.push((to, Event::UserUpdated(me.user)));
    }
    Ok(())
}

pub(super) async fn mine(c: &mut SqliteConnection, u: Id) -> Result<MyTags> {
    let wearing: Option<i64> = sqlx::query_scalar("SELECT tag_guild FROM users WHERE id=?")
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?;
    let mut available = vec![];
    for r in sqlx::query("SELECT t.guild_id,t.tag FROM server_tags t JOIN guild_members m ON m.guild_id=t.guild_id AND m.user_id=? ORDER BY t.guild_id")
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?
    {
        if let Some(t) = parse::<TagEdit>(Some(r.get("tag"))) {
            available.push(t.worn(Id(r.get::<i64, _>("guild_id") as u64)));
        }
    }
    Ok(MyTags {
        wearing: wearing.map(|g| Id(g as u64)),
        available,
    })
}

/// The card behind a tag: a server with a tag shows this much to anyone.
pub(super) async fn card(c: &mut SqliteConnection, u: Id, g: Id) -> Result<TagCard> {
    let tag = load(c, g)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let r = sqlx::query("SELECT name,icon_id,(SELECT COUNT(*) FROM guild_members WHERE guild_id=guilds.id) AS n,(SELECT description FROM guild_profiles WHERE guild_id=guilds.id) AS d FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    let banned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM guild_bans WHERE guild_id=? AND user_id=?)",
    )
    .bind(id(g)?)
    .bind(id(u)?)
    .fetch_one(&mut *c)
    .await?;
    let join = if role(c, u, g).await.is_ok() {
        TagJoin::Member
    } else if banned {
        TagJoin::None
    } else {
        let apply = crate::community::admission_settings(c, g)
            .await?
            .applications;
        match &tag.invite_code {
            Some(code) if usable_invite(c, g, code).await? => TagJoin::Invite {
                code: code.clone(),
                apply,
            },
            _ if crate::community::listed_icon(c, g).await? => TagJoin::Directory { apply },
            _ => TagJoin::None,
        }
    };
    Ok(TagCard {
        tag: tag.worn(g),
        name: r.get("name"),
        icon_id: r.get::<Option<i64>, _>("icon_id").map(|v| Id(v as u64)),
        member_count: r.get::<i64, _>("n") as u32,
        description: r.get::<Option<String>, _>("d").unwrap_or_default(),
        join,
    })
}
