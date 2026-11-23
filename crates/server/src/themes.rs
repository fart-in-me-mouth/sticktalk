//! Themes on the server (docs/features/themes.md): custom themes saved on the
//! account (synced to every device), a theme per server (Manage Server),
//! whether each person sees server themes, and `/tts` messages (Send TTS
//! Messages). The theme engine itself is `sticktalk_proto::themes`; every
//! theme that arrives here is untrusted and is validated with it.
//!
//! Hooks into shared code are one line each: `commands::mutate` dispatches
//! `Command::Appearance` here, `commands::execute` marks the stream as
//! subscribed, and `Core::emit_filtered` sends `Event::Appearance` only to
//! subscribed streams.
#[cfg(test)]
mod tests;
use crate::{
    db::*,
    error::{require, Error, Result},
    state::{now, Core},
};
use sqlx::{Row, SqliteConnection};
use sticktalk_proto::*;
pub(crate) type Deliveries = Vec<(Vec<Id>, Event)>;

fn invalid(message: &str, field: &str) -> Error {
    let mut e = Error::new(ErrorCode::InvalidRequest);
    e.0.message = message.into();
    e.0.field = Some(field.into());
    e
}
fn json<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|_| Error::new(ErrorCode::Internal))
}

/// A server theme as the audit log names it.
fn theme_summary(t: &ThemeChoice) -> String {
    match t {
        ThemeChoice::Preset(id) => format!("preset {id}"),
        ThemeChoice::Custom(spec) => format!(
            "custom {:?}, accent #{:06x}{}",
            spec.base,
            spec.accent,
            if spec.colors.is_empty() {
                String::new()
            } else {
                format!(", gradient of {}", spec.colors.len())
            }
        ),
    }
}
/// Everything about themes that belongs to `u`.
pub(crate) async fn library(c: &mut SqliteConnection, u: Id) -> Result<ThemeLibrary> {
    let themes =
        sqlx::query("SELECT id,name,spec FROM user_themes WHERE user_id=? ORDER BY id LIMIT ?")
            .bind(id(u)?)
            .bind(MAX_CUSTOM_THEMES as i64)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .filter_map(|r| {
                Some(CustomTheme {
                    id: Id(r.get::<i64, _>("id") as u64),
                    name: r.get("name"),
                    spec: serde_json::from_str(&r.get::<String, _>("spec")).ok()?,
                })
            })
            .collect();
    let settings =
        sqlx::query_scalar::<_, String>("SELECT settings FROM theme_settings WHERE user_id=?")
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
    let servers = sqlx::query(
        "SELECT t.guild_id,t.theme FROM guild_themes t JOIN guild_members m ON m.guild_id=t.guild_id WHERE m.user_id=? ORDER BY t.guild_id LIMIT ?",
    )
    .bind(id(u)?)
    .bind(MAX_GUILDS_PER_USER as i64)
    .fetch_all(&mut *c)
    .await?
    .into_iter()
    .filter_map(|r| {
        Some(ServerTheme {
            guild_id: Id(r.get::<i64, _>("guild_id") as u64),
            theme: serde_json::from_str(&r.get::<String, _>("theme")).ok()?,
        })
    })
    .collect();
    Ok(ThemeLibrary {
        themes,
        settings,
        servers,
    })
}

fn answer(lib: ThemeLibrary) -> CommandResult {
    CommandResult::Appearance(AppearanceResult::Library(Box::new(lib)))
}
/// The library, also queued for the account's other devices.
async fn changed(c: &mut SqliteConnection, u: Id, out: &mut Deliveries) -> Result<CommandResult> {
    let lib = library(c, u).await?;
    out.push((
        vec![u],
        Event::Appearance(AppearanceEvent::Library(Box::new(lib.clone()))),
    ));
    Ok(answer(lib))
}

/// `mutate`, built on the heap by a function of its own (see `commands::mutate`).
#[inline(never)]
pub(crate) fn boxed<'a>(
    c: &'a mut SqliteConnection,
    core: &'a mut Core,
    key: &'a [u8],
    u: Id,
    cmd: AppearanceCommand,
    out: &'a mut Deliveries,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<CommandResult>> + Send + 'a>> {
    Box::pin(mutate(c, core, key, u, cmd, out))
}

/// `Command::Appearance`, inside the command's transaction.
pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: AppearanceCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    use AppearanceCommand::*;
    match cmd {
        Subscribe => {
            if let Some(s) = core.streams.get_mut(key) {
                s.appearance_features = true;
            }
            Ok(answer(library(c, u).await?))
        }
        SaveTheme {
            id: theme,
            name,
            spec,
        } => {
            core.rate(format!("themes:{}", u.0), 30., 10.)?;
            let name = validate_name(&name).map_err(|m| invalid(m, "name"))?;
            spec.validate().map_err(|m| invalid(m, "spec"))?;
            let body = json(&spec)?;
            match theme {
                Some(t) => {
                    let done = sqlx::query(
                        "UPDATE user_themes SET name=?,spec=? WHERE id=? AND user_id=?",
                    )
                    .bind(&name)
                    .bind(&body)
                    .bind(id(t)?)
                    .bind(id(u)?)
                    .execute(&mut *c)
                    .await?
                    .rows_affected();
                    require(done == 1, ErrorCode::NotFound)?;
                }
                None => {
                    let count: i64 =
                        sqlx::query_scalar("SELECT COUNT(*) FROM user_themes WHERE user_id=?")
                            .bind(id(u)?)
                            .fetch_one(&mut *c)
                            .await?;
                    if count as usize >= MAX_CUSTOM_THEMES {
                        let mut e = Error::new(ErrorCode::LimitExceeded);
                        e.0.message = "You can save up to 20 custom themes.".into();
                        return Err(e);
                    }
                    let new = core.id().await?;
                    sqlx::query("INSERT INTO user_themes(id,user_id,name,spec,created_at) VALUES(?,?,?,?,?)")
                        .bind(id(new)?)
                        .bind(id(u)?)
                        .bind(&name)
                        .bind(&body)
                        .bind(now() as i64)
                        .execute(&mut *c)
                        .await?;
                }
            }
            changed(c, u, out).await
        }
        DeleteTheme { id: t } => {
            core.rate(format!("themes:{}", u.0), 30., 10.)?;
            let done = sqlx::query("DELETE FROM user_themes WHERE id=? AND user_id=?")
                .bind(id(t)?)
                .bind(id(u)?)
                .execute(&mut *c)
                .await?
                .rows_affected();
            require(done == 1, ErrorCode::NotFound)?;
            changed(c, u, out).await
        }
        SetServerTheme { guild_id, theme } => {
            core.rate(format!("themes:{}", u.0), 30., 10.)?;
            guild_perm(c, u, guild_id, Permissions::MANAGE_GUILD).await?;
            let before: Option<String> =
                sqlx::query_scalar("SELECT theme FROM guild_themes WHERE guild_id=?")
                    .bind(id(guild_id)?)
                    .fetch_optional(&mut *c)
                    .await?;
            let before = before
                .and_then(|t| serde_json::from_str::<ThemeChoice>(&t).ok())
                .map(|t| theme_summary(&t));
            match &theme {
                Some(t) => {
                    t.validate().map_err(|m| invalid(m, "theme"))?;
                    sqlx::query("INSERT INTO guild_themes(guild_id,theme,updated_by,updated_at) VALUES(?,?,?,?) ON CONFLICT(guild_id) DO UPDATE SET theme=excluded.theme,updated_by=excluded.updated_by,updated_at=excluded.updated_at")
                        .bind(id(guild_id)?)
                        .bind(json(t)?)
                        .bind(id(u)?)
                        .bind(now() as i64)
                        .execute(&mut *c)
                        .await?;
                }
                None => {
                    sqlx::query("DELETE FROM guild_themes WHERE guild_id=?")
                        .bind(id(guild_id)?)
                        .execute(&mut *c)
                        .await?;
                }
            }
            // In the audit log like the other server settings (task `qa2`).
            crate::moderation::audit::write(
                c,
                u,
                crate::moderation::audit::Entry::new(
                    guild_id,
                    AuditAction::GuildUpdate,
                    AuditTarget::Guild,
                    Some(guild_id),
                )
                .change("theme", before, theme.as_ref().map(theme_summary)),
            )
            .await?;
            out.push((
                guild_users(c, guild_id).await?,
                Event::Appearance(AppearanceEvent::ServerTheme { guild_id, theme }),
            ));
            Ok(answer(library(c, u).await?))
        }
        SetSettings(mut s) => {
            core.rate(format!("themes:{}", u.0), 30., 10.)?;
            require(
                s.server_overrides.len() <= MAX_SERVER_THEME_OVERRIDES,
                ErrorCode::InvalidRequest,
            )?;
            let mut seen = std::collections::BTreeSet::new();
            require(
                s.server_overrides.iter().all(|o| seen.insert(o.guild_id)),
                ErrorCode::InvalidRequest,
            )?;
            // Only servers you are in; others are dropped, not stored.
            let mut kept = Vec::with_capacity(s.server_overrides.len());
            for o in s.server_overrides {
                let member: Option<i64> = sqlx::query_scalar(
                    "SELECT 1 FROM guild_members WHERE guild_id=? AND user_id=?",
                )
                .bind(id(o.guild_id)?)
                .bind(id(u)?)
                .fetch_optional(&mut *c)
                .await?;
                if member.is_some() {
                    kept.push(o);
                }
            }
            s.server_overrides = kept;
            sqlx::query("INSERT INTO theme_settings(user_id,settings) VALUES(?,?) ON CONFLICT(user_id) DO UPDATE SET settings=excluded.settings")
                .bind(id(u)?)
                .bind(json(&s)?)
                .execute(&mut *c)
                .await?;
            changed(c, u, out).await
        }
        SendTts(draft) => {
            core.rate(format!("tts:{}", u.0), 10., 3.)?;
            text(
                c,
                u,
                draft.channel_id,
                Permissions::SEND_MESSAGES | Permissions::SEND_TTS_MESSAGES,
            )
            .await?;
            require(!draft.content.trim().is_empty(), ErrorCode::InvalidRequest)?;
            let CommandResult::Message(message) =
                crate::content::send(c, core, key, u, draft, out).await?
            else {
                return Err(Error::new(ErrorCode::Internal));
            };
            // Everyone who gets the message (after every delivery gate) is
            // told to read it, in the same order.
            let tts = Event::Appearance(AppearanceEvent::Tts {
                channel_id: message.channel_id,
                message_id: message.id,
            });
            let audience: Vec<Id> = out
                .iter()
                .filter(|(_, e)| matches!(e, Event::MessageCreated { message: m, .. } if m.id == message.id))
                .flat_map(|(users, _)| users.iter().copied())
                .filter(|v| *v != u)
                .collect();
            out.push((audience, tts));
            Ok(CommandResult::Message(message))
        }
    }
}
