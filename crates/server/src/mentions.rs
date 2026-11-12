//! Server-owned expansion and bounded, in-process notification/push hooks.
//! No OS notifications, external provider, background polling or message bodies.
use crate::{
    db,
    error::{require, Error, Result},
    state::{now, Core},
    App,
};
use sqlx::SqliteConnection;
use std::collections::BTreeSet;
use sticktalk_proto::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub user_id: UserId,
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub mentioned: bool,
    /// Push-provider hook: no foreground connection and not Invisible/DND.
    pub push: bool,
}
impl App {
    /// Lagging consumers receive Lagged, never unbounded retention. Consumers
    /// must recheck access before fetching bodies/sending external push payloads.
    pub fn subscribe_notifications(&self) -> tokio::sync::broadcast::Receiver<Notification> {
        self.0.notifications.subscribe()
    }
}
pub(crate) struct Expanded {
    pub users: Vec<Id>,
    origins: Vec<(Id, &'static str)>,
}
fn tokens(content: &str) -> impl Iterator<Item = &str> {
    content
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '@' | '<' | '>' | '&' | '_' | '-')))
}
pub(crate) async fn resolve(
    c: &mut SqliteConnection,
    core: &Core,
    author: Id,
    channel: &Channel,
    content: &str,
    explicit: &[Id],
) -> Result<Expanded> {
    let mut users: BTreeSet<_> = explicit.iter().copied().collect();
    let mut origins = vec![];
    let Some(guild) = channel.summary.guild_id else {
        return Ok(Expanded {
            users: users.into_iter().collect(),
            origins,
        });
    };
    if !content.contains('@') {
        return Ok(Expanded {
            users: users.into_iter().collect(),
            origins,
        });
    }
    let rights = crate::permissions::evaluate(c, author, guild, Some(channel.summary.id)).await?;
    let mut everyone = false;
    let mut here = false;
    let mut roles = BTreeSet::new();
    let available = crate::permissions::roles(c, guild).await?;
    for token in tokens(content) {
        match token {
            "@everyone" => everyone = true,
            "@here" => here = true,
            _ => {
                if let Some(role) = token.strip_prefix("<@&").and_then(|v| v.strip_suffix('>')) {
                    let id = role
                        .parse::<u64>()
                        .ok()
                        .filter(|v| *v > 0 && *v <= i64::MAX as u64)
                        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
                    if Id(id) == guild {
                        everyone = true;
                    } else {
                        roles.insert(Id(id));
                    }
                } else if let Some(name) = token.strip_prefix('@') {
                    let matches: Vec<_> = available
                        .iter()
                        .filter(|r| r.id != guild && r.name == name)
                        .collect();
                    require(matches.len() <= 1, ErrorCode::InvalidRequest)?;
                    if let Some(role) = matches.first() {
                        roles.insert(role.id);
                    }
                }
            }
        }
    }
    require(
        !(everyone || here) || rights.contains(Permissions::MENTION_EVERYONE),
        ErrorCode::Forbidden,
    )?;
    for role in &roles {
        let role = available
            .iter()
            .find(|r| r.id == *role)
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
        require(
            role.id != guild || rights.contains(Permissions::MENTION_EVERYONE),
            ErrorCode::Forbidden,
        )?;
        require(
            role.mentionable || rights.contains(Permissions::MENTION_EVERYONE),
            ErrorCode::Forbidden,
        )?;
    }
    if everyone || here || !roles.is_empty() {
        for user in db::audience(c, channel.summary.id).await? {
            let online = core
                .streams
                .values()
                .any(|s| s.user == user && s.connection.is_some())
                && matches!(
                    db::presence(c, core, user, user).await?.status,
                    PresenceStatus::Online | PresenceStatus::Idle | PresenceStatus::Dnd
                );
            let ids = crate::permissions::role_ids(c, guild, user).await?;
            for (included, kind) in [
                (everyone, "everyone"),
                (here && online, "here"),
                (ids.iter().any(|r| roles.contains(r)), "role"),
            ] {
                if included {
                    users.insert(user);
                    if !explicit.contains(&user) {
                        origins.push((user, kind));
                    }
                }
            }
        }
    }
    require(users.len() <= MAX_GUILD_MEMBERS, ErrorCode::LimitExceeded)?;
    Ok(Expanded {
        users: users.into_iter().collect(),
        origins,
    })
}
pub(crate) async fn persist(
    c: &mut SqliteConnection,
    message: Id,
    expanded: &Expanded,
) -> Result<()> {
    sqlx::query("DELETE FROM message_broadcast_mentions WHERE message_id=?")
        .bind(db::id(message)?)
        .execute(&mut *c)
        .await?;
    for (user, kind) in &expanded.origins {
        sqlx::query("INSERT INTO message_broadcast_mentions VALUES(?,?,?)")
            .bind(db::id(message)?)
            .bind(db::id(*user)?)
            .bind(kind)
            .execute(&mut *c)
            .await?;
    }
    Ok(())
}
pub(crate) async fn notify(
    app: &App,
    core: &Core,
    c: &mut SqliteConnection,
    message: &Message,
) -> Result<()> {
    if app.0.notifications.receiver_count() == 0 {
        return Ok(());
    }
    let channel = db::channel(c, message.channel_id).await?;
    for user in db::audience(c, message.channel_id).await? {
        if user == message.author_id || db::blocked(c, user, message.author_id).await? {
            continue;
        }
        let Some(mentioned) = notification_preference(c, user, &channel, message).await? else {
            continue;
        };
        let push = !core
            .streams
            .values()
            .any(|s| s.user == user && s.connection.is_some());
        let _ = app.0.notifications.send(Notification {
            user_id: user,
            channel_id: message.channel_id,
            message_id: message.id,
            mentioned,
            push,
        });
    }
    Ok(())
}

/// Current launch preferences shared by in-process notifications and Web Push.
/// Some(mentioned) preserves the effective mention after broadcast suppression.
pub(crate) async fn notification_preference(
    c: &mut SqliteConnection,
    user: Id,
    channel: &Channel,
    message: &Message,
) -> Result<Option<bool>> {
    let mut mentioned = message.mention_ids.contains(&user);
    let mut level = if channel.summary.guild_id.is_some() {
        NotificationLevel::Mentions
    } else {
        NotificationLevel::All
    };
    let mut muted = false;
    let mut suppress = false;
    // Guild preference first, channel preference takes precedence.
    let settings = crate::launch::notifications(c, user).await?;
    for setting in settings
        .iter()
        .filter(|s| s.channel_id.is_none() && s.guild_id == channel.summary.guild_id)
        .chain(
            settings
                .iter()
                .filter(|s| s.channel_id == Some(message.channel_id)),
        )
    {
        level = setting.level;
        muted = setting.muted && setting.mute_until.is_none_or(|t| t > now());
        suppress = setting.suppress_everyone;
    }
    if suppress && mentioned {
        let broadcast:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM message_broadcast_mentions WHERE message_id=? AND user_id=? AND kind IN('everyone','here')) AND NOT EXISTS(SELECT 1 FROM message_broadcast_mentions WHERE message_id=? AND user_id=? AND kind='role')").bind(db::id(message.id)?).bind(db::id(user)?).bind(db::id(message.id)?).bind(db::id(user)?).fetch_one(&mut *c).await?;
        mentioned &= !broadcast;
    }
    if muted
        || level == NotificationLevel::Nothing
        || (level == NotificationLevel::Mentions && !mentioned)
    {
        return Ok(None);
    }
    let presence = db::me(c, user).await?.desired_presence;
    if matches!(presence, PresenceStatus::Dnd | PresenceStatus::Invisible) {
        return Ok(None);
    }
    Ok(Some(mentioned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audit_tests::fixture, commands};
    async fn send(
        app: &App,
        core: &mut Core,
        user: u64,
        nonce: u64,
        content: &str,
        mention_ids: Vec<Id>,
    ) -> CommandOutcome {
        core.limits.clear();
        commands::execute(
            app,
            core,
            &[user as u8; 32],
            Id(user),
            CommandRequest {
                nonce,
                command: Command::SendMessage(MessageDraft {
                    channel_id: Id(11),
                    content: content.into(),
                    reply_to: None,
                    attachment_ids: vec![],
                    mention_ids,
                }),
            },
        )
        .await
        .unwrap()
        .outcome
    }
    fn message(outcome: CommandOutcome) -> Message {
        match outcome {
            CommandOutcome::Ok(CommandResult::Message(m)) => m,
            _ => panic!("{outcome:?}"),
        }
    }
    fn drain(rx: &mut tokio::sync::broadcast::Receiver<Notification>) -> Vec<Notification> {
        let mut out = vec![];
        while let Ok(n) = rx.try_recv() {
            out.push(n);
        }
        out
    }
    #[tokio::test]
    async fn broadcasts_and_roles_expand_only_authorized_viewers_and_update_unread_counts() {
        let (_dir, app) = fixture().await;
        sqlx::raw_sql("INSERT INTO guild_members VALUES(10,3,'member',0);INSERT INTO read_states(user_id,channel_id) VALUES(3,11);INSERT INTO roles(guild_id,id,name,position,mentionable,permissions) VALUES(10,20,'players',2,0,0);INSERT INTO member_roles VALUES(10,2,20),(10,3,20);").execute(&app.0.pool).await.unwrap();
        let mut core = app.0.core.lock().await;
        for content in ["@everyone", "@here", "<@&20>", "@players"] {
            assert!(matches!(
                send(
                    &app,
                    &mut core,
                    2,
                    10 + content.len() as u64,
                    content,
                    vec![]
                )
                .await,
                CommandOutcome::Err(ApiError {
                    code: ErrorCode::Forbidden,
                    ..
                })
            ));
        }
        let mut rx = app.subscribe_notifications();
        let m = message(send(&app, &mut core, 1, 30, "@everyone <@&20>", vec![Id(2)]).await);
        assert_eq!(m.mention_ids, vec![Id(1), Id(2), Id(3)]);
        let notices = drain(&mut rx);
        assert!(notices
            .iter()
            .any(|n| n.user_id == Id(2) && n.mentioned && n.push));
        assert!(notices.iter().any(|n| n.user_id == Id(3)));
        let mut c = app.0.pool.acquire().await.unwrap();
        assert_eq!(
            db::read(&mut c, Id(2), Id(11)).await.unwrap().mention_count,
            1
        );
        sqlx::raw_sql("UPDATE roles SET mentionable=1 WHERE id=20;INSERT INTO permission_overwrites VALUES(11,'member',3,0,1);").execute(&mut *c).await.unwrap();
        drop(c);
        let m = message(send(&app, &mut core, 2, 31, "@players <@&20>", vec![]).await);
        assert_eq!(m.mention_ids, vec![Id(2)]);
        let m = message(send(&app, &mut core, 1, 32, "@everyone", vec![]).await);
        assert_eq!(m.mention_ids, vec![Id(1), Id(2)]);
        let mut c = app.0.pool.acquire().await.unwrap();
        assert_eq!(
            db::read(&mut c, Id(2), Id(11)).await.unwrap().mention_count,
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM message_mentions WHERE message_id=? AND user_id=4"
            )
            .bind(db::id(m.id).unwrap())
            .fetch_one(&mut *c)
            .await
            .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn here_excludes_offline_invisible_and_private_members_and_edits_reexpand() {
        let (_dir, app) = fixture().await;
        sqlx::raw_sql("INSERT INTO guild_members VALUES(10,3,'member',0);INSERT INTO read_states(user_id,channel_id) VALUES(3,11);").execute(&app.0.pool).await.unwrap();
        let mut core = app.0.core.lock().await;
        let (tx, _rx) = tokio::sync::mpsc::channel(128);
        let (close, _) = tokio::sync::watch::channel(None);
        core.cursor(&[2; 32], Id(2));
        core.streams.get_mut(&vec![2; 32]).unwrap().connection = Some(crate::state::Connection {
            focused: false,
            id: "here".into(),
            alive: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            tx,
            budget: std::sync::Arc::new(tokio::sync::Semaphore::new(2 * 1024 * 1024)),
            close,
        });
        let m = message(send(&app, &mut core, 1, 1, "@here", vec![]).await);
        assert_eq!(m.mention_ids, vec![Id(2)]);
        sqlx::query("UPDATE users SET desired_presence='invisible' WHERE id=2")
            .execute(&app.0.pool)
            .await
            .unwrap();
        let m = message(send(&app, &mut core, 1, 2, "@here", vec![]).await);
        assert!(m.mention_ids.is_empty());
        let response = commands::execute(
            &app,
            &mut core,
            &[1; 32],
            Id(1),
            CommandRequest {
                nonce: 3,
                command: Command::EditMessage(MessageEdit {
                    channel_id: Id(11),
                    message_id: m.id,
                    content: "@everyone".into(),
                    mention_ids: vec![],
                }),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            message(response.outcome).mention_ids,
            vec![Id(1), Id(2), Id(3)]
        );
        let mut c = app.0.pool.acquire().await.unwrap();
        assert_eq!(
            db::read(&mut c, Id(3), Id(11)).await.unwrap().mention_count,
            1
        );
    }
    #[tokio::test]
    async fn notifications_respect_suppression_explicit_mentions_mutes_overrides_and_dnd() {
        let (_dir, app) = fixture().await;
        let mut core = app.0.core.lock().await;
        let mut rx = app.subscribe_notifications();
        let mut setting = NotificationSettings {
            guild_id: Some(Id(10)),
            channel_id: None,
            level: NotificationLevel::Mentions,
            muted: false,
            mute_until: None,
            suppress_everyone: true,
            hide_muted_channels: false,
        };
        async fn set(app: &App, core: &mut Core, nonce: u64, s: NotificationSettings) {
            let response = commands::execute(
                app,
                core,
                &[2; 32],
                Id(2),
                CommandRequest {
                    nonce,
                    command: Command::Launch(LaunchCommand::SetNotifications(s)),
                },
            )
            .await
            .unwrap();
            assert!(matches!(response.outcome, CommandOutcome::Ok(_)));
        }
        set(&app, &mut core, 1, setting.clone()).await;
        message(send(&app, &mut core, 1, 2, "@everyone", vec![]).await);
        assert!(drain(&mut rx).is_empty());
        message(send(&app, &mut core, 1, 3, "@everyone", vec![Id(2)]).await);
        assert_eq!(drain(&mut rx).len(), 1);
        setting.level = NotificationLevel::Nothing;
        set(&app, &mut core, 4, setting.clone()).await;
        message(send(&app, &mut core, 1, 5, "explicit", vec![Id(2)]).await);
        assert!(drain(&mut rx).is_empty());
        setting.channel_id = Some(Id(11));
        setting.level = NotificationLevel::All;
        setting.suppress_everyone = false;
        set(&app, &mut core, 6, setting.clone()).await;
        message(send(&app, &mut core, 1, 7, "ordinary", vec![]).await);
        assert_eq!(drain(&mut rx).len(), 1);
        setting.muted = true;
        set(&app, &mut core, 8, setting.clone()).await;
        message(send(&app, &mut core, 1, 9, "explicit", vec![Id(2)]).await);
        assert!(drain(&mut rx).is_empty());
        setting.mute_until = Some(1);
        set(&app, &mut core, 10, setting).await;
        sqlx::query("UPDATE users SET desired_presence='dnd' WHERE id=2")
            .execute(&app.0.pool)
            .await
            .unwrap();
        message(send(&app, &mut core, 1, 11, "explicit", vec![Id(2)]).await);
        assert!(drain(&mut rx).is_empty());
        // Muting/suppressing notifications never erases durable unread mention counts.
        let mut c = app.0.pool.acquire().await.unwrap();
        assert_eq!(
            db::read(&mut c, Id(2), Id(11)).await.unwrap().mention_count,
            5
        );
    }
}
