//! `sticktalk-server admin apps ...`: the instance administrator's controls.
//! It works on the database file and takes effect on the running server at
//! once (credentials are checked on every use; open bot connections close
//! within a minute, at the next housekeeping pass).
use super::*;
use clap::Subcommand;

#[derive(Subcommand, Clone, Debug)]
pub enum AppsAdmin {
    /// Every app: ID, name, owner, servers, state.
    List,
    /// Turn an app off everywhere: its bot is signed out and its commands disappear.
    Disable {
        app_id: u64,
    },
    Enable {
        app_id: u64,
    },
    /// Who may create apps: everyone, allowlist or disabled (no new apps, no bots connect).
    Creation {
        policy: String,
    },
    /// Let one account create apps while the policy is `allowlist`.
    Allow {
        username: String,
    },
    Disallow {
        username: String,
    },
}

pub async fn run(c: &mut SqliteConnection, command: &AppsAdmin) -> Result<()> {
    match command {
        AppsAdmin::List => {
            println!("policy\t{}", enum_str(store::creation(c).await?));
            let rows = sqlx::query("SELECT a.id,a.name,a.disabled,u.username,(SELECT COUNT(*) FROM app_installs i WHERE i.app_id=a.id) AS n FROM apps a JOIN users u ON u.id=a.owner_id ORDER BY a.id LIMIT 10000")
                .fetch_all(&mut *c)
                .await?;
            for r in rows {
                println!(
                    "{}\t{}\towner={}\tservers={}\t{}",
                    r.get::<i64, _>("id"),
                    r.get::<String, _>("name"),
                    r.get::<String, _>("username"),
                    r.get::<i64, _>("n"),
                    if r.get::<bool, _>("disabled") {
                        "disabled"
                    } else {
                        "enabled"
                    }
                );
            }
        }
        AppsAdmin::Disable { app_id } | AppsAdmin::Enable { app_id } => {
            let disable = matches!(command, AppsAdmin::Disable { .. });
            let bot: i64 = sqlx::query_scalar("SELECT bot_id FROM apps WHERE id=?")
                .bind(*app_id as i64)
                .fetch_optional(&mut *c)
                .await?
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            sqlx::query("UPDATE apps SET disabled=? WHERE id=?")
                .bind(disable)
                .bind(*app_id as i64)
                .execute(&mut *c)
                .await?;
            if disable {
                sqlx::query("INSERT OR IGNORE INTO disabled_users VALUES(?)")
                    .bind(bot)
                    .execute(&mut *c)
                    .await?;
            } else {
                sqlx::query("DELETE FROM disabled_users WHERE user_id=?")
                    .bind(bot)
                    .execute(&mut *c)
                    .await?;
            }
            println!("Done.");
        }
        AppsAdmin::Creation { policy } => {
            let p: AppCreation = en(policy.to_ascii_lowercase())
                .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
            sqlx::query("UPDATE app_settings SET creation=? WHERE singleton=1")
                .bind(enum_str(p))
                .execute(&mut *c)
                .await?;
            println!("Done.");
        }
        AppsAdmin::Allow { username: name } | AppsAdmin::Disallow { username: name } => {
            let u: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username=?")
                .bind(username(name)?)
                .fetch_optional(&mut *c)
                .await?
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            if matches!(command, AppsAdmin::Allow { .. }) {
                sqlx::query("INSERT OR IGNORE INTO app_creators VALUES(?)")
                    .bind(u)
                    .execute(&mut *c)
                    .await?;
            } else {
                sqlx::query("DELETE FROM app_creators WHERE user_id=?")
                    .bind(u)
                    .execute(&mut *c)
                    .await?;
            }
            println!("Done.");
        }
    }
    Ok(())
}
