//! Persisted channel slowmode. The serialized command transaction makes concurrent sends atomic.
use crate::{
    db::*,
    error::{require, Error, Result},
    http, permissions,
    state::{now, App},
};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
};
use sqlx::SqliteConnection;
use sticktalk_proto::*;
pub(crate) async fn status(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    at: u64,
) -> Result<SlowmodeStatus> {
    let channel = text(c, u, ch, Permissions::VIEW_CHANNEL).await?;
    let seconds =
        sqlx::query_scalar::<_, i64>("SELECT seconds FROM channel_slowmode WHERE channel_id=?")
            .bind(id(ch)?)
            .fetch_optional(&mut *c)
            .await?
            .unwrap_or(0) as u32;
    let bypassed = if let Some(g) = channel.summary.guild_id {
        permissions::evaluate(c, u, g, Some(ch))
            .await?
            .intersects(Permissions::MANAGE_MESSAGES | Permissions::MANAGE_CHANNELS)
    } else {
        true
    };
    let retry_after_ms = if seconds > 0 && !bypassed {
        let last=sqlx::query_scalar::<_,i64>("SELECT created_at FROM messages WHERE author_id=? AND channel_id=? ORDER BY id DESC LIMIT 1").bind(id(u)?).bind(id(ch)?).fetch_optional(c).await?;
        last.map_or(0, |last| {
            (last.max(0) as u64)
                .saturating_add(u64::from(seconds) * 1000)
                .saturating_sub(at)
                .min(u64::from(seconds) * 1000)
        }) as u32
    } else {
        0
    };
    Ok(SlowmodeStatus {
        channel_id: ch,
        seconds,
        bypassed,
        retry_after_ms,
        server_time: at,
    })
}
pub(crate) async fn check(c: &mut SqliteConnection, u: Id, ch: Id) -> Result<()> {
    let s = status(c, u, ch, now()).await?;
    if s.retry_after_ms > 0 {
        let mut e = Error::rate(s.retry_after_ms);
        e.0.field = Some("slowmode".into());
        e.0.message = "Slowmode is enabled in this channel.".into();
        return Err(e);
    }
    Ok(())
}
pub(crate) async fn set(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    seconds: u32,
    out: &mut Vec<(Vec<Id>, Event)>,
) -> Result<CommandResult> {
    require(
        SLOWMODE_SECONDS.contains(&seconds),
        ErrorCode::InvalidRequest,
    )?;
    let channel = text(c, u, ch, Permissions::MANAGE_CHANNELS).await?;
    require(
        channel.summary.kind == ChannelKind::Text && channel.summary.guild_id.is_some(),
        ErrorCode::InvalidRequest,
    )?;
    sqlx::query("INSERT INTO channel_slowmode(channel_id,seconds) VALUES(?,?) ON CONFLICT(channel_id) DO UPDATE SET seconds=excluded.seconds").bind(id(ch)?).bind(i64::from(seconds)).execute(&mut *c).await?;
    out.push((
        audience(c, ch).await?,
        Event::SlowmodeUpdated {
            channel_id: ch,
            seconds,
        },
    ));
    Ok(CommandResult::Ack)
}
pub(crate) async fn endpoint(
    State(app): State<App>,
    Path(ch): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    Ok(http::json(Snapshot {
        data: status(&mut c, u, Id(ch), now()).await?,
        cursor: core.cursor(&key, u),
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audit_tests::fixture, commands};
    async fn command(app: &App, u: u64, nonce: u64, command: Command) -> CommandOutcome {
        let mut core = app.0.core.lock().await;
        core.limits.clear();
        commands::execute(
            app,
            &mut core,
            &[u as u8; 32],
            Id(u),
            CommandRequest { nonce, command },
        )
        .await
        .unwrap()
        .outcome
    }
    fn set_to(s: u32) -> Command {
        Command::Launch(LaunchCommand::SetSlowmode {
            channel_id: Id(11),
            seconds: s,
        })
    }
    fn send() -> Command {
        Command::SendMessage(MessageDraft {
            channel_id: Id(11),
            content: "slowmode test".into(),
            reply_to: None,
            attachment_ids: vec![],
            mention_ids: vec![],
        })
    }
    #[tokio::test]
    async fn slowmode_validates_permissions_persists_and_serializes_sends() {
        let (_dir, app) = fixture().await;
        assert!(matches!(
            command(&app, 2, 1, set_to(5)).await,
            CommandOutcome::Err(ApiError {
                code: ErrorCode::Forbidden,
                ..
            })
        ));
        assert!(matches!(
            command(&app, 1, 1, set_to(6)).await,
            CommandOutcome::Err(ApiError {
                code: ErrorCode::InvalidRequest,
                ..
            })
        ));
        assert!(matches!(
            command(&app, 1, 2, set_to(5)).await,
            CommandOutcome::Ok(_)
        ));
        let a = command(&app, 2, 2, send()).await;
        let CommandOutcome::Ok(CommandResult::Message(m)) = a else {
            panic!("{a:?}")
        };
        let a = command(&app, 2, 3, send()).await;
        let CommandOutcome::Err(e) = a else {
            panic!("{a:?}")
        };
        assert_eq!(e.code, ErrorCode::RateLimited);
        assert_eq!(e.field.as_deref(), Some("slowmode"));
        assert!((1..=5000).contains(&e.retry_after_ms.unwrap()));
        // A retry of the first command is its original receipt, not a second message or a cooldown error.
        assert!(matches!(
            command(&app, 2, 2, send()).await,
            CommandOutcome::Ok(CommandResult::Message(_))
        ));
        assert!(matches!(
            command(
                &app,
                2,
                4,
                Command::EditMessage(MessageEdit {
                    channel_id: Id(11),
                    message_id: m.id,
                    content: "edited during slowmode".into(),
                    mention_ids: vec![]
                })
            )
            .await,
            CommandOutcome::Ok(_)
        ));
        assert!(matches!(
            command(
                &app,
                2,
                5,
                Command::DeleteMessage {
                    channel_id: Id(11),
                    message_id: m.id
                }
            )
            .await,
            CommandOutcome::Ok(_)
        ));
        assert!(matches!(
            command(&app, 2, 6, send()).await,
            CommandOutcome::Err(_)
        )); // deletion cannot evade the limit
        let mut c = app.0.pool.acquire().await.unwrap();
        assert_eq!(
            status(&mut c, Id(2), Id(11), m.created_at + 5000)
                .await
                .unwrap()
                .retry_after_ms,
            0
        );
        assert!(status(&mut c, Id(3), Id(11), now()).await.is_err());
        assert!(status(&mut c, Id(1), Id(11), now()).await.unwrap().bypassed);
        assert_eq!(
            status(&mut c, Id(2), Id(11), m.created_at)
                .await
                .unwrap()
                .seconds,
            5
        );
        sqlx::query("INSERT INTO permission_overwrites VALUES(11,'member',2,?,0)")
            .bind(Permissions::MANAGE_MESSAGES.bits() as i64)
            .execute(&mut *c)
            .await
            .unwrap();
        drop(c);
        for n in [7, 8] {
            assert!(matches!(
                command(&app, 2, n, send()).await,
                CommandOutcome::Ok(_)
            ));
        }
        sqlx::query("UPDATE permission_overwrites SET allow=? WHERE channel_id=11 AND target_id=2")
            .bind(Permissions::MANAGE_CHANNELS.bits() as i64)
            .execute(&app.0.pool)
            .await
            .unwrap();
        assert!(matches!(
            command(&app, 2, 9, send()).await,
            CommandOutcome::Ok(_)
        ));
        assert!(matches!(
            command(&app, 2, 10, set_to(21600)).await,
            CommandOutcome::Ok(_)
        ));
        assert!(matches!(
            command(&app, 1, 3, set_to(0)).await,
            CommandOutcome::Ok(_)
        ));
        sqlx::query("DELETE FROM permission_overwrites WHERE channel_id=11")
            .execute(&app.0.pool)
            .await
            .unwrap();
        assert!(matches!(
            command(&app, 2, 11, send()).await,
            CommandOutcome::Ok(_)
        ));
    }
}
