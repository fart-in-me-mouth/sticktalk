//! Offline/local administrator CLI. No TCP admin endpoint or password arguments.
use crate::{
    db::*,
    error::{require, Error, Result},
    state::{hash, now, token, App, Config},
};
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
};
use clap::{Args, Subcommand};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
    Connection, Row,
};
use sticktalk_proto::*;
#[derive(Args, Clone)]
pub struct OperationsConfig {
    #[arg(long, env="STICKTALK_MIN_FREE_DISK_GB", default_value_t=10, value_parser=clap::value_parser!(u64).range(0..=8589934591))]
    pub min_free_disk_gb: u64,
    #[arg(long, env="STICKTALK_USER_QUOTA_MB", default_value_t=0, value_parser=clap::value_parser!(u64).range(0..=8796093022207))]
    pub user_quota_mb: u64,
    /// Base64 raw 32-byte Ed25519 public key (NOT a private signing key).
    #[arg(long, env = "STICKTALK_UPDATE_PUBLIC_KEY")]
    pub update_public_key: Option<String>,
    #[arg(long, env = "STICKTALK_RELEASE_TOKEN", hide_env_values = true)]
    pub release_token: Option<String>,
    #[command(subcommand)]
    pub command: Option<Operation>,
}
impl std::fmt::Debug for OperationsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationsConfig")
            .field("min_free_disk_gb", &self.min_free_disk_gb)
            .field("user_quota_mb", &self.user_quota_mb)
            .field("update_public_key", &self.update_public_key)
            .field(
                "release_token",
                &self.release_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("command", &self.command)
            .finish()
    }
}
impl Default for OperationsConfig {
    fn default() -> Self {
        Self {
            min_free_disk_gb: 10,
            user_quota_mb: 0,
            update_public_key: None,
            release_token: None,
            command: None,
        }
    }
}
#[derive(Subcommand, Clone, Debug)]
pub enum Operation {
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}
#[derive(Subcommand, Clone, Debug)]
pub enum AdminCommand {
    ListUsers,
    ResetPassword { username: String },
    DisableUser { username: String },
    EnableUser { username: String },
}
pub async fn run(config: &Config, command: &AdminCommand) -> Result<()> {
    // Do NOT App::open: its startup orphan cleanup is unsafe alongside a live server.
    let mut c = sqlx::SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(config.data_dir.join("sticktalk.sqlite3"))
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5)),
    )
    .await?;
    crate::db::migrate(&mut c).await?;
    if matches!(command, AdminCommand::ListUsers) {
        let mut after = 0;
        loop {
            let rows=sqlx::query("SELECT id,username,EXISTS(SELECT 1 FROM disabled_users WHERE user_id=users.id) AS disabled FROM users WHERE id>? ORDER BY id LIMIT 100").bind(after).fetch_all(&mut c).await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                after = row.get::<i64, _>("id");
                println!(
                    "{}\t{}\t{}",
                    after,
                    row.get::<String, _>("username"),
                    if row.get::<bool, _>("disabled") {
                        "disabled"
                    } else {
                        "enabled"
                    }
                );
            }
        }
        return Ok(());
    }
    let name = match command {
        AdminCommand::ResetPassword { username }
        | AdminCommand::DisableUser { username }
        | AdminCommand::EnableUser { username } => username,
        _ => unreachable!(),
    };
    let mut tx = c.begin().await?;
    let user: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username=?")
        .bind(username(name)?)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let mut code = None;
    match command {
        AdminCommand::ResetPassword { .. } => {
            let secret = token();
            sqlx::query("INSERT INTO password_resets VALUES(?,?,?) ON CONFLICT(user_id) DO UPDATE SET code_hash=excluded.code_hash,expires_at=excluded.expires_at").bind(user).bind(hash(secret.as_bytes())).bind((now()+30*60*1000) as i64).execute(&mut *tx).await?;
            code = Some(secret);
        }
        AdminCommand::DisableUser { .. } => {
            sqlx::query("INSERT OR IGNORE INTO disabled_users VALUES(?)")
                .bind(user)
                .execute(&mut *tx)
                .await?;
        }
        AdminCommand::EnableUser { .. } => {
            sqlx::query("DELETE FROM disabled_users WHERE user_id=?")
                .bind(user)
                .execute(&mut *tx)
                .await?;
        }
        _ => {}
    }
    if !matches!(command, AdminCommand::EnableUser { .. }) {
        sqlx::query("DELETE FROM sessions WHERE user_id=?")
            .bind(user)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if let Some(code) = code {
        println!("{code}");
    } else {
        println!("Done.");
    }
    Ok(())
}
pub async fn reset(State(app): State<App>, r: Request<Body>) -> Result<StatusCode> {
    let bucket = crate::http::auth_bucket(&app, &r);
    app.0.core.lock().await.rate(bucket, 10., 5.)?;
    let request: ResetPasswordRequest = crate::http::parse(r).await?;
    let name = username(&request.username)?;
    require(
        request.code.len() == 43 && (12..=128).contains(&request.new_password.len()),
        ErrorCode::InvalidRequest,
    )?;
    let expected = hash(request.code.as_bytes());
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM password_resets p JOIN users u ON u.id=p.user_id WHERE u.username=? AND p.code_hash=? AND p.expires_at>? AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=u.id))").bind(&name).bind(&expected).bind(now() as i64).fetch_one(&app.0.pool).await?;
    require(valid, ErrorCode::InvalidCredentials)?;
    let permit = app
        .0
        .passwords
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let password = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::http::password_hash(request.new_password)
    })
    .await
    .map_err(|_| Error::new(ErrorCode::Internal))??;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let mut tx = c.begin().await?;
    let user:Option<i64>=sqlx::query_scalar("DELETE FROM password_resets WHERE code_hash=? AND expires_at>? AND user_id=(SELECT id FROM users WHERE username=?) AND NOT EXISTS(SELECT 1 FROM disabled_users WHERE user_id=password_resets.user_id) RETURNING user_id").bind(expected).bind(now() as i64).bind(name).fetch_optional(&mut *tx).await?;
    let user = user.ok_or_else(|| Error::new(ErrorCode::InvalidCredentials))?;
    sqlx::query("UPDATE users SET password_hash=? WHERE id=?")
        .bind(password)
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=?")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    core.streams.retain(|_, s| {
        if s.user.0 == user as u64 {
            if let Some(c) = &s.connection {
                let _ = c.close.send(Some(4001));
            }
            false
        } else {
            true
        }
    });
    Ok(StatusCode::NO_CONTENT)
}
