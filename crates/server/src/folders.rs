//! Private, per-account server organization. Relational FKs prune departed memberships.
use crate::{
    db::*,
    error::{require, Result},
    http,
    state::App,
};
use axum::{extract::State, http::HeaderMap};
use sqlx::{Row, SqliteConnection};
use sticktalk_proto::*;
pub(crate) async fn list(c: &mut SqliteConnection, u: Id) -> Result<Vec<ServerFolder>> {
    let rows = sqlx::query(
        "SELECT id,name,color,collapsed FROM server_folders WHERE user_id=? ORDER BY position,id",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?;
    let mut out = Vec::new();
    for r in rows {
        let fid = r.get::<i64, _>("id");
        let guild_ids=sqlx::query_scalar::<_,i64>("SELECT guild_id FROM server_folder_guilds WHERE user_id=? AND folder_id=? ORDER BY position").bind(id(u)?).bind(fid).fetch_all(&mut *c).await?.into_iter().map(|n|Id(n as u64)).collect::<Vec<_>>();
        if !guild_ids.is_empty() {
            out.push(ServerFolder {
                id: fid as u32,
                name: r.get("name"),
                color: r.get::<i64, _>("color") as u32,
                collapsed: r.get("collapsed"),
                guild_ids,
            });
        }
    }
    Ok(out)
}
pub(crate) async fn set(
    c: &mut SqliteConnection,
    u: Id,
    folders: Vec<ServerFolder>,
    out: &mut Vec<(Vec<Id>, Event)>,
) -> Result<CommandResult> {
    require(folders.len() <= 10, ErrorCode::InvalidRequest)?;
    let mut ids = std::collections::BTreeSet::new();
    let mut guilds = std::collections::BTreeSet::new();
    for f in &folders {
        require(
            f.id > 0
                && ids.insert(f.id)
                && f.color <= 0xffffff
                && valid_folder_name(&f.name)
                && !f.guild_ids.is_empty()
                && f.guild_ids.len() <= 10,
            ErrorCode::InvalidRequest,
        )?;
        for g in &f.guild_ids {
            require(guilds.insert(*g), ErrorCode::InvalidRequest)?;
            role(c, u, *g).await?;
        }
    }
    sqlx::query("DELETE FROM server_folders WHERE user_id=?")
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    for (position, f) in folders.iter().enumerate() {
        sqlx::query("INSERT INTO server_folders(user_id,id,name,color,collapsed,position) VALUES(?,?,?,?,?,?)").bind(id(u)?).bind(i64::from(f.id)).bind(f.name.trim()).bind(i64::from(f.color)).bind(f.collapsed).bind(position as i64).execute(&mut *c).await?;
        for (position, g) in f.guild_ids.iter().enumerate() {
            sqlx::query("INSERT INTO server_folder_guilds(user_id,folder_id,guild_id,position) VALUES(?,?,?,?)").bind(id(u)?).bind(i64::from(f.id)).bind(id(*g)?).bind(position as i64).execute(&mut *c).await?;
        }
    }
    out.push((vec![u], Event::ServerFoldersUpdated(list(c, u).await?)));
    Ok(CommandResult::Ack)
}
pub(crate) async fn endpoint(State(app): State<App>, h: HeaderMap) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    Ok(http::json(Snapshot {
        data: list(&mut c, u).await?,
        cursor: core.cursor(&key, u),
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audit_tests::fixture, commands};
    fn folder() -> ServerFolder {
        ServerFolder {
            id: 1,
            name: "Work".into(),
            color: 0x3ddc97,
            collapsed: true,
            guild_ids: vec![Id(10)],
        }
    }
    async fn command(app: &App, u: u64, n: u64, folders: Vec<ServerFolder>) -> CommandOutcome {
        let mut core = app.0.core.lock().await;
        core.limits.clear();
        core.cursor(&[u as u8; 32], Id(u));
        core.streams
            .get_mut(&vec![u as u8; 32])
            .unwrap()
            .text_features = true;
        commands::execute(
            app,
            &mut core,
            &[u as u8; 32],
            Id(u),
            CommandRequest {
                nonce: n,
                command: Command::Launch(LaunchCommand::SetServerFolders(folders)),
            },
        )
        .await
        .unwrap()
        .outcome
    }
    #[tokio::test]
    async fn folders_are_private_validated_persistent_and_membership_scoped() {
        let (_dir, app) = fixture().await;
        assert!(matches!(
            command(&app, 1, 1, vec![folder()]).await,
            CommandOutcome::Ok(_)
        ));
        let mut other = folder();
        other.name = "Friends".into();
        other.collapsed = false;
        assert!(matches!(
            command(&app, 2, 1, vec![other.clone()]).await,
            CommandOutcome::Ok(_)
        ));
        assert!(matches!(
            command(&app, 3, 1, vec![folder()]).await,
            CommandOutcome::Err(_)
        ));
        for (i, bad) in [
            ServerFolder { id: 0, ..folder() },
            ServerFolder {
                color: 0x1000000,
                ..folder()
            },
            ServerFolder {
                name: "\nBad".into(),
                ..folder()
            },
            ServerFolder {
                name: "a".repeat(33),
                ..folder()
            },
            ServerFolder {
                guild_ids: vec![Id(10), Id(10)],
                ..folder()
            },
            ServerFolder {
                guild_ids: vec![],
                ..folder()
            },
        ]
        .into_iter()
        .enumerate()
        {
            assert!(matches!(
                command(&app, 1, 10 + i as u64, vec![bad]).await,
                CommandOutcome::Err(_)
            ));
        }
        assert!(matches!(
            command(
                &app,
                1,
                30,
                vec![folder(), ServerFolder { id: 2, ..folder() }]
            )
            .await,
            CommandOutcome::Err(_)
        ));
        let mut c = app.0.pool.acquire().await.unwrap();
        assert_eq!(list(&mut c, Id(1)).await.unwrap(), vec![folder()]);
        assert_eq!(list(&mut c, Id(2)).await.unwrap(), vec![other]);
        // Only this account's streams are recipients. An unrelated member's organization never leaks.
        let core = app.0.core.lock().await;
        assert!(!core.streams[&vec![1; 32]].ring.is_empty());
        for (key, s) in &core.streams {
            for (_, _, bytes) in &s.ring {
                if let ServerMsg::Event(e) = rmp_serde::from_slice::<ServerMsg>(bytes).unwrap() {
                    if let Event::ServerFoldersUpdated(f) = e.event {
                        assert!(key == &vec![1; 32] || key == &vec![2; 32]);
                        assert_eq!(
                            f[0].name,
                            if key == &vec![1; 32] {
                                "Work"
                            } else {
                                "Friends"
                            }
                        );
                    }
                }
            }
        }
        drop(core);
        sqlx::query("DELETE FROM guild_members WHERE guild_id=10 AND user_id=2")
            .execute(&mut *c)
            .await
            .unwrap();
        assert!(list(&mut c, Id(2)).await.unwrap().is_empty());
        assert_eq!(list(&mut c, Id(1)).await.unwrap().len(), 1);
        drop(c);
        assert!(matches!(
            command(&app, 1, 31, vec![]).await,
            CommandOutcome::Ok(_)
        ));
    }
}
