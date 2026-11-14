//! Erasure retains only an anonymous identity/tombstone needed by foreign keys.
use crate::{
    db::{auth, id},
    error::{require, Result},
    http,
    state::{self, App},
};
use axum::{body::Body, http::Request};
use sqlx::{Connection, Row, SqliteConnection};
use sticktalk_proto::*;
pub async fn api(app: &App, r: Request<Body>) -> Result<http::ApiResponse> {
    require(r.method() == "POST", ErrorCode::NotFound)?;
    let headers = r.headers().clone();
    let token = http::bearer(&headers)?.to_owned();
    let _io = app.0.upload_io.lock().await;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    // Reserve the writer before reading credentials/proofs; a concurrent local
    // admin/maintenance write must not strand a deferred read snapshot.
    let mut tx = c.begin_with("BEGIN IMMEDIATE").await?;
    let (key, user) = auth(&mut tx, &token).await?;
    crate::reauth::consume(&mut tx, &headers, &key, AccountOperation::DeleteAccount).await?;
    let keys: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions WHERE user_id=?")
        .bind(id(user)?)
        .fetch_all(&mut *tx)
        .await?;
    let paths = erase(&mut tx, user).await?;
    tx.commit().await?;
    // Invalidate queued/replayed pre-erasure profiles/content on every client.
    crate::reauth::disconnect(app, &mut core, &mut c, &keys).await?;
    // Other users' tokens remain valid. Their queued snapshots must be purged,
    // but use resync (not 4001/session-expired, which would log everyone out).
    for (_, stream) in core.streams.drain() {
        if let Some(connection) = stream.connection {
            let _ = connection.close.send(Some(4010));
        }
    }
    drop(c);
    drop(core);
    remove_files(app, &paths).await;
    Ok(http::ApiResponse::empty())
}
pub async fn remove_files(app: &App, paths: &[String]) {
    for key in paths {
        let _ = tokio::fs::remove_file(app.0.config.data_dir.join("files").join(key)).await;
    }
    // Replica GC is durable: blob_replicas survives until remote deletion succeeds.
    app.0.tier.wake.notify_one();
}
pub async fn erase(c: &mut SqliteConnection, user: Id) -> Result<Vec<String>> {
    let u = id(user)?;
    let name: String = sqlx::query_scalar("SELECT username FROM users WHERE id=?")
        .bind(u)
        .fetch_one(&mut *c)
        .await?;
    let deleted: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deleted_accounts WHERE user_id=?)")
            .bind(u)
            .fetch_one(&mut *c)
            .await?;
    if deleted {
        return Ok(vec![]);
    }
    let now = state::now() as i64;
    let release = now + 30 * 86_400_000;
    sqlx::query("INSERT INTO reserved_usernames VALUES(?,?)")
        .bind(name)
        .bind(release)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT INTO deleted_accounts VALUES(?,?,?)")
        .bind(u)
        .bind(now)
        .bind(release)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO disabled_users VALUES(?)")
        .bind(u)
        .execute(&mut *c)
        .await?;
    let mut paths = vec![];
    for row in sqlx::query("SELECT storage_key,thumbnail_key FROM uploads WHERE owner_id=? UNION ALL SELECT storage_key,thumbnail_key FROM partial_uploads WHERE owner_id=?")
        .bind(u).bind(u).fetch_all(&mut *c).await? {
        paths.push(row.get::<String,_>("storage_key"));
        if let Some(key)=row.get::<Option<String>,_>("thumbnail_key"){paths.push(key);}
    }
    for key in
        sqlx::query_scalar::<_, String>("SELECT key FROM upload_reservations WHERE user_id=?")
            .bind(u)
            .fetch_all(&mut *c)
            .await?
    {
        paths.extend([
            format!("{key}.tmp"),
            format!("{key}.bin"),
            format!("{key}.jpg"),
        ]);
    }
    sqlx::query("UPDATE users SET username=?,display_name='Deleted User',password_hash='',avatar_id=NULL,status_text=NULL,desired_presence='invisible' WHERE id=?")
        .bind(format!("!deleted:{u}")).bind(u).execute(&mut *c).await?;
    // Remove references before file metadata; no dangling attachment can be served.
    for sql in [
        "UPDATE users SET avatar_id=NULL WHERE avatar_id IN(SELECT id FROM uploads WHERE owner_id=?)",
        "UPDATE guilds SET icon_id=NULL WHERE icon_id IN(SELECT id FROM uploads WHERE owner_id=?)",
        "DELETE FROM message_attachments WHERE attachment_id IN(SELECT id FROM uploads WHERE owner_id=?)",
        "DELETE FROM message_attachments WHERE message_id IN(SELECT id FROM messages WHERE author_id=?)",
        "DELETE FROM pins WHERE message_id IN(SELECT id FROM messages WHERE author_id=?)",
        "DELETE FROM pins WHERE pinned_by=?",
        "DELETE FROM message_mentions WHERE message_id IN(SELECT id FROM messages WHERE author_id=?)",
        "DELETE FROM message_mentions WHERE user_id=?",
        "DELETE FROM message_broadcast_mentions WHERE message_id IN(SELECT id FROM messages WHERE author_id=?)",
        "DELETE FROM reactions WHERE message_id IN(SELECT id FROM messages WHERE author_id=?)",
        "DELETE FROM reactions WHERE user_id=?",
        "DELETE FROM sessions WHERE user_id=?",
        "DELETE FROM oauth_signup_tickets WHERE subject IN(SELECT subject FROM oauth_identities WHERE user_id=?)",
        "DELETE FROM oauth_identities WHERE user_id=?",
        "DELETE FROM password_resets WHERE user_id=?",
        "DELETE FROM partial_uploads WHERE owner_id=?",
        "DELETE FROM upload_reservations WHERE user_id=?",
        "DELETE FROM uploads WHERE owner_id=?",
        "DELETE FROM user_profiles WHERE user_id=?",
        "DELETE FROM gif_favorites WHERE user_id=?",
        "DELETE FROM notification_settings WHERE user_id=?",
        "DELETE FROM relationships WHERE user_id=? OR other_id=?1",
        "DELETE FROM read_states WHERE user_id=?",
        "DELETE FROM permission_overwrites WHERE kind='member' AND target_id=?",
        "DELETE FROM invites WHERE creator_id=?",
    ] {sqlx::query(sql).bind(u).execute(&mut *c).await?;}
    sqlx::query("UPDATE messages SET content='',reply_to=NULL,edited_at=NULL,deleted_at=COALESCE(deleted_at,?) WHERE author_id=?").bind(now).bind(u).execute(&mut *c).await?;
    // Transfer a surviving guild to its earliest member; erase an empty guild.
    let guilds: Vec<i64> = sqlx::query_scalar("SELECT id FROM guilds WHERE owner_id=?")
        .bind(u)
        .fetch_all(&mut *c)
        .await?;
    for guild in guilds {
        let next:Option<i64>=sqlx::query_scalar("SELECT user_id FROM guild_members WHERE guild_id=? AND user_id<>? AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=guild_members.user_id) ORDER BY joined_at,user_id LIMIT 1").bind(guild).bind(u).fetch_optional(&mut *c).await?;
        if let Some(next) = next {
            sqlx::query("UPDATE guilds SET owner_id=? WHERE id=?")
                .bind(next)
                .bind(guild)
                .execute(&mut *c)
                .await?;
            sqlx::query("UPDATE guild_members SET role='owner' WHERE guild_id=? AND user_id=?")
                .bind(guild)
                .bind(next)
                .execute(&mut *c)
                .await?;
        } else {
            sqlx::query("DELETE FROM guilds WHERE id=?")
                .bind(guild)
                .execute(&mut *c)
                .await?;
        }
    }
    sqlx::query("DELETE FROM guild_members WHERE user_id=?")
        .bind(u)
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM dm_members WHERE user_id=? AND channel_id IN(SELECT id FROM channels WHERE kind='group_dm')").bind(u).execute(&mut *c).await?;
    sqlx::query("UPDATE channels SET owner_id=(SELECT MIN(user_id) FROM dm_members WHERE channel_id=channels.id) WHERE owner_id=?").bind(u).execute(&mut *c).await?;
    // Receipts on OTHER users' sessions can contain serialized old messages too.
    sqlx::query("DELETE FROM command_receipts")
        .execute(&mut *c)
        .await?;
    sqlx::query("UPDATE read_states SET unread_count=(SELECT COUNT(*) FROM messages m WHERE m.channel_id=read_states.channel_id AND m.deleted_at IS NULL AND m.author_id<>read_states.user_id AND m.id>COALESCE(read_states.last_read_id,0)),mention_count=(SELECT COUNT(*) FROM messages m JOIN message_mentions mm ON mm.message_id=m.id WHERE m.channel_id=read_states.channel_id AND m.deleted_at IS NULL AND mm.user_id=read_states.user_id AND m.id>COALESCE(read_states.last_read_id,0))").execute(&mut *c).await?;
    Ok(paths)
}
pub async fn reserved(c: &mut SqliteConnection, name: &str) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM reserved_usernames WHERE username=? AND release_at>?)",
    )
    .bind(name)
    .bind(state::now() as i64)
    .fetch_one(c)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn admin_cli_erasure_uses_the_same_tombstone_and_username_hold() {
        let (_dir, app) = crate::audit_tests::fixture().await;
        let config = app.0.config.clone();
        drop(app);
        crate::admin::run(
            &config,
            &crate::admin::AdminCommand::EraseAccount {
                username: "user1".into(),
            },
        )
        .await
        .unwrap();
        let mut c = sqlx::SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new()
                .filename(config.data_dir.join("sticktalk.sqlite3")),
        )
        .await
        .unwrap();
        assert!(reserved(&mut c, "user1").await.unwrap());
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT display_name FROM users WHERE id=1")
                .fetch_one(&mut c)
                .await
                .unwrap(),
            "Deleted User"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE user_id=1")
                .fetch_one(&mut c)
                .await
                .unwrap(),
            0
        );
    }
}
