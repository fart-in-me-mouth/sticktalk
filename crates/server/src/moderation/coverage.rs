//! Moderation on the paths other features added (`docs/features/integration.md`):
//! thread names, forum posts, thread changes and poll votes by timed-out
//! members, soundboard sound names, and announcement crossposts arriving from
//! another server. The same rules as messages: a timeout, the verification
//! level and AutoMod.
use super::*;

/// The refusal a timed-out member gets, in any channel of server `g`.
pub(crate) async fn guild_timed_out(c: &mut SqliteConnection, g: Id, u: Id) -> Result<()> {
    match timeout(c, g, u).await? {
        Some(t) => Err(denied(
            "timeout",
            format!(
                "You are timed out in this server for {} more. You can read, but not send messages, react or join voice.",
                minutes(t.until.saturating_sub(now()))
            ),
        )),
        None => Ok(()),
    }
}
/// Refuse a timed-out member in a server channel (a no-op elsewhere, and for
/// channels the person cannot see: the command refuses those).
pub(crate) async fn timed_out(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<()> {
    let Ok(channel) = access(c, u, ch, Permissions::VIEW_CHANNEL).await else {
        return Ok(());
    };
    match channel.summary.guild_id {
        Some(g) => guild_timed_out(c, g, u).await,
        None => Ok(()),
    }
}

/// Thread commands that create visible text, or change a thread, get the
/// checks a message gets. Joining, leaving and notification settings are the
/// person's own and stay allowed.
pub(super) async fn thread_check(
    app: &App,
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    cmd: &ThreadCommand,
) -> Result<()> {
    use ThreadCommand::*;
    match cmd {
        Create(t) => {
            screen(
                app,
                core,
                c,
                u,
                t.parent_id,
                &t.name,
                &[],
                true,
                Permissions::CREATE_PUBLIC_THREADS | Permissions::CREATE_PRIVATE_THREADS,
            )
            .await
        }
        // The post's first message is sent by the forum code directly, so
        // the title and the message are screened here, as one send.
        CreatePost(p) => {
            let text = format!("{}\n{}", p.title, p.content);
            message_check(app, core, c, u, p.forum_id, &text, &p.mention_ids, true).await
        }
        Update { thread_id, patch } => match &patch.name {
            Some(name) => {
                screen(
                    app,
                    core,
                    c,
                    u,
                    *thread_id,
                    name,
                    &[],
                    false,
                    Permissions::empty(),
                )
                .await
            }
            None => timed_out(c, u, *thread_id).await,
        },
        UpdateForum { forum_id, patch } => {
            let mut text = patch.guidelines.clone().unwrap_or_default();
            for t in patch.tags.iter().flatten() {
                text.push('\n');
                text.push_str(&t.name);
            }
            screen(
                app,
                core,
                c,
                u,
                *forum_id,
                &text,
                &[],
                false,
                Permissions::MANAGE_CHANNELS,
            )
            .await
        }
        AddMember { thread_id, .. } | RemoveMember { thread_id, .. } | Delete { thread_id } => {
            timed_out(c, u, *thread_id).await
        }
        Publish { channel_id, .. } => timed_out(c, u, *channel_id).await,
        _ => Ok(()),
    }
}

/// The keyword rule that blocks `text` for `u` in server `g` outside any
/// channel (names of server things such as soundboard sounds). Members with
/// Manage Server are exempt, as for messages.
async fn blocking_keyword(
    core: &mut Core,
    c: &mut SqliteConnection,
    g: Id,
    roles: &[Id],
    lineage: &[Id],
    text: &str,
) -> Result<Option<automod::Hit>> {
    let rules = automod::rules(&mut core.moderation, c, g).await?;
    if !rules.iter().any(|r| r.rule.enabled) {
        return Ok(None);
    }
    Ok(automod::check(
        &rules,
        roles,
        lineage,
        &automod::Draft {
            content: text,
            mentions: &[],
            repeats: 0,
        },
    )
    .into_iter()
    .find(|h| matches!(h.rule.trigger, AutoModTrigger::Keyword { .. }) && h.rule.actions.block))
}

/// A name `u` gives to something in server `g` (a soundboard sound): refused
/// when a blocking keyword rule matches it, and logged like a blocked message.
pub(crate) async fn name_check(
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    what: &str,
    text: &str,
) -> Result<()> {
    guild_timed_out(c, g, u).await?;
    if acl::evaluate(c, u, g, None)
        .await?
        .contains(Permissions::MANAGE_GUILD)
    {
        return Ok(());
    }
    let roles = acl::role_ids(c, g, u).await?;
    let Some(h) = blocking_keyword(core, c, g, &roles, &[], text).await? else {
        return Ok(());
    };
    let mut tx = c.begin().await?;
    ensure_automod_user(&mut tx).await?;
    let author = user(&mut tx, u).await?;
    audit::write(
        &mut tx,
        AUTOMOD_USER,
        audit::Entry::new(g, AuditAction::AutomodBlock, AuditTarget::User, Some(u))
            .named(author.username)
            .change("rule", None, Some(h.rule.name.clone()))
            .change(what, None, Some(text.to_string()))
            .reason(Some(format!("The name was flagged because {}.", h.why))),
    )
    .await?;
    tx.commit().await?;
    Err(denied(
        "automod",
        h.rule.actions.message.clone().unwrap_or_else(|| {
            format!("This server's AutoMod blocked this name because {}.", h.why)
        }),
    ))
}

/// An announcement copy arriving from another server is screened by the
/// receiving server's keyword rules (its channel exemptions apply; nobody in
/// the receiving server wrote it, so no member is exempt). A blocked copy is
/// not delivered and the receiving server's audit log notes it. Runs inside
/// the delivery's transaction.
pub(crate) async fn crosspost_blocked(
    core: &mut Core,
    c: &mut SqliteConnection,
    f: &ChannelFollow,
    text: &str,
) -> Result<bool> {
    let g = f.target_guild_id;
    let lineage = acl::lineage(c, g, f.target_channel_id).await?;
    let Some(h) = blocking_keyword(core, c, g, &[g], &lineage, text).await? else {
        return Ok(false);
    };
    ensure_automod_user(c).await?;
    audit::write(
        c,
        AUTOMOD_USER,
        audit::Entry::new(
            g,
            AuditAction::AutomodBlock,
            AuditTarget::Channel,
            Some(f.target_channel_id),
        )
        .named(format!("#{}", f.target_channel_name))
        .change("rule", None, Some(h.rule.name.clone()))
        .change(
            "followed channel",
            None,
            Some(format!(
                "{} › #{}",
                f.source_guild_name, f.source_channel_name
            )),
        )
        .reason(Some(format!(
            "A followed announcement was not delivered because {}.",
            h.why
        ))),
    )
    .await?;
    Ok(true)
}

/// A 1:1 call with someone who is not a friend is direct contact through a
/// shared server, like a DM: that server's DM pause and verification level
/// apply, and so does a timeout there (a timed-out member cannot start or
/// answer such a call). Friends are never affected. `calls::admit` runs it on
/// every join.
pub(crate) async fn call_check(c: &mut SqliteConnection, u: Id, channel: &Channel) -> Result<()> {
    dm_check(c, u, channel).await?;
    let Some(peer) = channel
        .summary
        .recipient_ids
        .iter()
        .copied()
        .find(|v| *v != u)
    else {
        return Ok(());
    };
    if friends(c, u, peer).await.is_ok() {
        return Ok(());
    }
    let shared: Vec<i64> = sqlx::query_scalar("SELECT a.guild_id FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.user_id=? AND b.user_id=? ORDER BY a.guild_id")
        .bind(id(u)?)
        .bind(id(peer)?)
        .fetch_all(&mut *c)
        .await?;
    for g in shared {
        guild_timed_out(c, Id(g as u64), u).await?;
    }
    Ok(())
}
