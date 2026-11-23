//! Audit log entries for the administrative actions of threads, forums,
//! announcement channels and polls (`docs/features/integration.md`). Hooked
//! into `audit::prepare`/`audit::record` like the existing commands: read the
//! state before, compare after, inside the command's transaction.
//! Soundboard changes and voice bitrate are REST routes; they write their
//! entries directly (`callextras.rs`).
use super::audit::Entry;
use super::*;

pub(crate) enum Pending {
    Thread(Box<ThreadInfo>),
    ThreadDelete(Box<ThreadInfo>),
    ThreadMember {
        info: Box<ThreadInfo>,
        user: Id,
    },
    Forum {
        g: Id,
        name: String,
        before: Box<ForumSettings>,
    },
    Publish {
        g: Id,
        ch: Id,
        name: String,
        message: Id,
        author: Id,
    },
    Unfollow(Box<ChannelFollow>),
    Follow,
    Message {
        g: Id,
        name: String,
        message: Id,
        author: Id,
        action: AuditAction,
    },
}

fn yes(b: bool) -> Option<String> {
    Some(if b { "yes" } else { "no" }.into())
}
async fn guild_channel(c: &mut SqliteConnection, ch: Id) -> Result<Option<(Id, String)>> {
    let r = sqlx::query("SELECT guild_id,name FROM channels WHERE id=?")
        .bind(id(ch)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(r.and_then(|r| {
        let g: Option<i64> = r.get(0);
        g.map(|g| {
            (
                Id(g as u64),
                r.get::<Option<String>, _>(1).unwrap_or_default(),
            )
        })
    }))
}
async fn author(c: &mut SqliteConnection, ch: Id, m: Id) -> Result<Option<Id>> {
    let a: Option<i64> = sqlx::query_scalar(
        "SELECT author_id FROM messages WHERE id=? AND channel_id=? AND deleted_at IS NULL",
    )
    .bind(id(m)?)
    .bind(id(ch)?)
    .fetch_optional(&mut *c)
    .await?;
    Ok(a.map(|a| Id(a as u64)))
}
async fn username(c: &mut SqliteConnection, u: Id) -> String {
    db::user(c, u)
        .await
        .map_or_else(|_| format!("user {}", u.0), |u| format!("@{}", u.username))
}
async fn is_member(c: &mut SqliteConnection, t: Id, u: Id) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM thread_members WHERE thread_id=? AND user_id=?)",
    )
    .bind(id(t)?)
    .bind(id(u)?)
    .fetch_one(&mut *c)
    .await?)
}
fn kind(t: &ThreadInfo) -> &'static str {
    if t.forum_post {
        "forum post"
    } else if t.private {
        "private thread"
    } else {
        "thread"
    }
}
fn tag_names(f: Option<&ForumSettings>, ids: &[u32]) -> Option<String> {
    let names: Vec<String> = ids
        .iter()
        .map(|i| {
            f.and_then(|f| f.tags.iter().find(|t| t.id == *i))
                .map_or_else(|| format!("tag {i}"), |t| t.name.clone())
        })
        .collect();
    Some(if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    })
}

pub(super) async fn prepare(c: &mut SqliteConnection, cmd: &Command) -> Result<Option<Pending>> {
    Ok(Some(match cmd {
        Command::Threads(t) => match t {
            ThreadCommand::Update { thread_id, .. } => {
                match crate::threads::load(c, *thread_id).await? {
                    Some(info) => Pending::Thread(Box::new(info)),
                    None => return Ok(None),
                }
            }
            ThreadCommand::Delete { thread_id } => match crate::threads::load(c, *thread_id).await?
            {
                Some(info) => Pending::ThreadDelete(Box::new(info)),
                None => return Ok(None),
            },
            ThreadCommand::RemoveMember { thread_id, user_id } => {
                let Some(info) = crate::threads::load(c, *thread_id).await? else {
                    return Ok(None);
                };
                if !is_member(c, *thread_id, *user_id).await? {
                    return Ok(None);
                }
                Pending::ThreadMember {
                    info: Box::new(info),
                    user: *user_id,
                }
            }
            ThreadCommand::UpdateForum { forum_id, .. } => {
                let Some((g, name)) = guild_channel(c, *forum_id).await? else {
                    return Ok(None);
                };
                Pending::Forum {
                    g,
                    name,
                    before: Box::new(crate::threads::forum_settings(c, *forum_id).await?),
                }
            }
            ThreadCommand::Publish {
                channel_id,
                message_id,
            } => {
                let Some((g, name)) = guild_channel(c, *channel_id).await? else {
                    return Ok(None);
                };
                let Some(author) = author(c, *channel_id, *message_id).await? else {
                    return Ok(None);
                };
                Pending::Publish {
                    g,
                    ch: *channel_id,
                    name,
                    message: *message_id,
                    author,
                }
            }
            ThreadCommand::Follow { .. } => Pending::Follow,
            ThreadCommand::Unfollow { follow_id } => {
                Pending::Unfollow(Box::new(crate::threads::follow_info(c, *follow_id).await?))
            }
            _ => return Ok(None),
        },
        // Moderators acting on someone else's poll or link previews.
        Command::Content(
            ContentCommand::EndPoll {
                channel_id,
                message_id,
            }
            | ContentCommand::RemoveEmbeds {
                channel_id,
                message_id,
            },
        ) => {
            let Some((g, name)) = guild_channel(c, *channel_id).await? else {
                return Ok(None);
            };
            let Some(author) = author(c, *channel_id, *message_id).await? else {
                return Ok(None);
            };
            let action = if matches!(cmd, Command::Content(ContentCommand::EndPoll { .. })) {
                let open: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM polls WHERE message_id=? AND closed_at IS NULL)",
                )
                .bind(id(*message_id)?)
                .fetch_one(&mut *c)
                .await?;
                if !open {
                    return Ok(None);
                }
                AuditAction::PollEnd
            } else {
                AuditAction::EmbedsRemove
            };
            Pending::Message {
                g,
                name,
                message: *message_id,
                author,
                action,
            }
        }
        _ => return Ok(None),
    }))
}

pub(super) async fn record(
    c: &mut SqliteConnection,
    actor: Id,
    pending: Pending,
    result: &CommandResult,
) -> Result<()> {
    use AuditAction as A;
    use AuditTarget as T;
    let entries: Vec<Entry> = match pending {
        Pending::Thread(before) => {
            let Some(after) = crate::threads::load(c, before.id).await? else {
                return Ok(());
            };
            // A thread's owner managing their own thread is not a moderation
            // action; locking and pinning always are (Manage Threads only).
            if actor == before.owner_id
                && before.locked == after.locked
                && before.pinned == after.pinned
            {
                return Ok(());
            }
            let forum = if after.forum_post {
                crate::threads::forum_settings(c, after.parent_id)
                    .await
                    .ok()
            } else {
                None
            };
            let e = Entry::new(after.guild_id, A::ThreadUpdate, T::Channel, Some(after.id))
                .named(after.name.clone())
                .change("kind", None, Some(kind(&after).into()))
                .change("name", Some(before.name.clone()), Some(after.name.clone()))
                .change("archived", yes(before.archived), yes(after.archived))
                .change("locked", yes(before.locked), yes(after.locked))
                .change("pinned", yes(before.pinned), yes(after.pinned))
                .change("invitable", yes(before.invitable), yes(after.invitable))
                .change(
                    "auto-archive",
                    Some(format!("{} minutes", before.auto_archive_minutes)),
                    Some(format!("{} minutes", after.auto_archive_minutes)),
                )
                .change(
                    "tags",
                    tag_names(forum.as_ref(), &before.tag_ids),
                    tag_names(forum.as_ref(), &after.tag_ids),
                );
            // Only the "kind" line: nothing changed.
            if before.name == after.name
                && before.archived == after.archived
                && before.locked == after.locked
                && before.pinned == after.pinned
                && before.invitable == after.invitable
                && before.auto_archive_minutes == after.auto_archive_minutes
                && before.tag_ids == after.tag_ids
            {
                return Ok(());
            }
            vec![e]
        }
        Pending::ThreadDelete(t) => {
            if actor == t.owner_id {
                return Ok(());
            }
            let owner = username(c, t.owner_id).await;
            vec![
                Entry::new(t.guild_id, A::ThreadDelete, T::Channel, Some(t.id))
                    .named(t.name.clone())
                    .change("kind", None, Some(kind(&t).into()))
                    .change("started by", None, Some(owner))
                    .change("messages", None, Some(t.message_count.to_string())),
            ]
        }
        Pending::ThreadMember { info, user } => {
            if actor == user {
                return Ok(());
            }
            let who = username(c, user).await;
            vec![
                Entry::new(info.guild_id, A::ThreadMemberRemove, T::User, Some(user))
                    .named(who.trim_start_matches('@').to_string())
                    .change("thread", None, Some(info.name.clone())),
            ]
        }
        Pending::Forum { g, name, before } => {
            let CommandResult::Threads(ThreadResult::Forum(after)) = result else {
                return Ok(());
            };
            let tags = |f: &ForumSettings| {
                Some(if f.tags.is_empty() {
                    "none".to_string()
                } else {
                    f.tags
                        .iter()
                        .map(|t| {
                            format!(
                                "{}{}",
                                t.name,
                                if t.moderated {
                                    " (moderators only)"
                                } else {
                                    ""
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
            };
            let secs = |s: u32| Some(super::audit::slowmode_text(s));
            let e = Entry::new(g, A::ForumUpdate, T::Channel, Some(after.channel_id))
                .named(format!("#{name}"))
                .change(
                    "guidelines",
                    Some(before.guidelines.clone()),
                    Some(after.guidelines.clone()),
                )
                .change("tags", tags(&before), tags(after))
                .change(
                    "require tags",
                    yes(before.require_tags),
                    yes(after.require_tags),
                )
                .change(
                    "default reaction",
                    before.default_reaction.clone(),
                    after.default_reaction.clone(),
                )
                .change(
                    "sort",
                    Some(enum_str(before.sort)),
                    Some(enum_str(after.sort)),
                )
                .change(
                    "layout",
                    Some(enum_str(before.layout)),
                    Some(enum_str(after.layout)),
                )
                .change(
                    "post slowmode",
                    secs(before.post_slowmode),
                    secs(after.post_slowmode),
                )
                .change(
                    "reply slowmode",
                    secs(before.reply_slowmode),
                    secs(after.reply_slowmode),
                )
                .change(
                    "default auto-archive",
                    Some(format!("{} minutes", before.default_auto_archive_minutes)),
                    Some(format!("{} minutes", after.default_auto_archive_minutes)),
                );
            if *before == *after {
                return Ok(());
            }
            vec![e]
        }
        Pending::Publish {
            g,
            ch,
            name,
            message,
            author,
        } => {
            let followers: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM channel_follows WHERE source_channel_id=?",
            )
            .bind(id(ch)?)
            .fetch_one(&mut *c)
            .await?;
            let by = username(c, author).await;
            vec![Entry::new(g, A::MessagePublish, T::Message, Some(message))
                .named(format!("#{name}"))
                .change("author", None, Some(by))
                .change("followers", None, Some(followers.to_string()))]
        }
        Pending::Follow => {
            let CommandResult::Threads(ThreadResult::Follow(f)) = result else {
                return Ok(());
            };
            vec![Entry::new(
                f.target_guild_id,
                A::ChannelFollow,
                T::Channel,
                Some(f.target_channel_id),
            )
            .named(format!("#{}", f.target_channel_name))
            .change(
                "follows",
                None,
                Some(format!(
                    "{} › #{}",
                    f.source_guild_name, f.source_channel_name
                )),
            )]
        }
        Pending::Unfollow(f) => {
            let followed = Some(format!(
                "{} › #{}",
                f.source_guild_name, f.source_channel_name
            ));
            let mut out = vec![Entry::new(
                f.target_guild_id,
                A::ChannelUnfollow,
                T::Channel,
                Some(f.target_channel_id),
            )
            .named(format!("#{}", f.target_channel_name))
            .change("followed", followed, None)];
            // Removed from the announcement side: that server's log too.
            let source_side = acl::evaluate(c, actor, f.source_guild_id, Some(f.source_channel_id))
                .await
                .is_ok_and(|p| p.contains(Permissions::MANAGE_CHANNELS));
            if source_side && f.source_guild_id != f.target_guild_id {
                out.push(
                    Entry::new(
                        f.source_guild_id,
                        A::ChannelUnfollow,
                        T::Channel,
                        Some(f.source_channel_id),
                    )
                    .named(format!("#{}", f.source_channel_name))
                    .change(
                        "follower",
                        Some(format!(
                            "{} › #{}",
                            f.target_guild_name, f.target_channel_name
                        )),
                        None,
                    ),
                );
            }
            out
        }
        Pending::Message {
            g,
            name,
            message,
            author,
            action,
            ..
        } => {
            if actor == author {
                return Ok(());
            }
            let by = username(c, author).await;
            vec![Entry::new(g, action, T::Message, Some(message))
                .named(format!("#{name}"))
                .change("author", None, Some(by))]
        }
    };
    for e in entries {
        super::audit::write(c, actor, e).await?;
    }
    Ok(())
}
