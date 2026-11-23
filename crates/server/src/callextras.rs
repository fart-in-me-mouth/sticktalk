//! Call extras on the server: each server's soundboard, playing a sound in a
//! call, entrance sounds, clip notices, a voice channel's Opus bitrate and
//! custom camera backgrounds. See `docs/features/callextras.md`.
//!
//! Sounds are never mixed into a microphone. `VoiceSignal::PlaySound` is checked
//! here (in the call, Use Soundboard there, a member of the sound's server, not
//! server-muted, rate limits) and becomes one `VoiceEvent::SoundPlayed` to each
//! connection in that call; every listener plays its own copy of the clip.
pub(crate) mod audio;
mod backgrounds;
#[cfg(test)]
mod tests;

use crate::{
    db::{self, id},
    error::{require, Error, Result},
    http, permissions,
    state::{now, App, Core},
    voice,
};
use axum::{
    body::{to_bytes, Body},
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post, put},
};
use serde::Serialize;
use sqlx::{Row, SqliteConnection};
use sticktalk_proto::*;

pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/soundboard", get(library))
        .route("/api/v1/guilds/{id}/soundboard", post(upload))
        .route(
            "/api/v1/soundboard/sounds/{id}",
            patch(update).delete(delete_sound),
        )
        .route("/api/v1/soundboard/sounds/{id}/audio", get(clip))
        .route(
            "/api/v1/soundboard/favorites/{id}",
            put(favorite).delete(unfavorite),
        )
        .route(
            "/api/v1/soundboard/entrance",
            get(entrance).put(set_entrance),
        )
        .route(
            "/api/v1/channels/{id}/voice-quality",
            get(quality).put(set_quality),
        )
        .merge(backgrounds::routes())
}

// ---- JSON for the browser ---------------------------------------------------
// Snowflake IDs do not fit a JavaScript number. These routes are read by the
// web app's own scripts, so IDs travel as decimal strings both ways.

fn is_id(key: &str) -> bool {
    key == "id" || key.ends_with("_id") || key == "created_by" || key == "favorites"
}
pub(crate) fn stringify(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if is_id(k) {
                    match v {
                        serde_json::Value::Number(n) if n.is_u64() => *v = n.to_string().into(),
                        serde_json::Value::Array(a) => {
                            for n in a.iter_mut().filter(|n| n.is_u64()) {
                                *n = n.to_string().into();
                            }
                        }
                        _ => {}
                    }
                } else {
                    stringify(v)
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(stringify),
        _ => {}
    }
}
fn numbers(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if is_id(k) {
                    if let Some(n) = v.as_str().and_then(|s| s.parse::<u64>().ok()) {
                        *v = n.into();
                    }
                } else {
                    numbers(v)
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(numbers),
        _ => {}
    }
}
pub(crate) fn js<T: Serialize>(value: &T) -> http::ApiResponse {
    let mut v = serde_json::to_value(value).unwrap_or_default();
    stringify(&mut v);
    http::json(v)
}
async fn body<T: serde::de::DeserializeOwned>(r: Request<Body>) -> Result<T> {
    let mut v: serde_json::Value = http::parse(r).await?;
    numbers(&mut v);
    serde_json::from_value(v).map_err(|_| Error::new(ErrorCode::InvalidRequest))
}
fn token(h: &HeaderMap) -> Result<String> {
    Ok(http::bearer(h)?.to_owned())
}

// ---- the soundboard library -------------------------------------------------

fn sound_row(r: &sqlx::sqlite::SqliteRow) -> SoundboardSound {
    SoundboardSound {
        id: Id(r.get::<i64, _>("id") as u64),
        guild_id: Id(r.get::<i64, _>("guild_id") as u64),
        name: r.get("name"),
        emoji: r.get("emoji"),
        volume: r.get::<i64, _>("volume").clamp(0, 100) as u8,
        duration_ms: r.get::<i64, _>("duration_ms") as u32,
        format: match r.get::<String, _>("format").as_str() {
            "mp3" => SoundFormat::Mp3,
            "ogg" => SoundFormat::Ogg,
            _ => SoundFormat::Wav,
        },
        created_by: Id(r.get::<i64, _>("created_by") as u64),
    }
}

async fn sound(c: &mut SqliteConnection, sound: Id) -> Result<SoundboardSound> {
    sqlx::query("SELECT id,guild_id,name,emoji,volume,duration_ms,format,created_by FROM soundboard_sounds WHERE id=?")
    .bind(id(sound)?)
    .fetch_optional(c)
    .await?
    .map(|r| sound_row(&r))
    .ok_or_else(|| Error::new(ErrorCode::NotFound))
}
/// Manage Soundboard, or Manage Server, in that server.
async fn can_manage(c: &mut SqliteConnection, u: Id, g: Id) -> Result<bool> {
    Ok(permissions::evaluate(c, u, g, None)
        .await?
        .intersects(Permissions::MANAGE_SOUNDBOARD | Permissions::MANAGE_GUILD))
}
async fn manage(c: &mut SqliteConnection, u: Id, g: Id) -> Result<()> {
    require(can_manage(c, u, g).await?, ErrorCode::Forbidden)
}
/// The server's audit log entry for a soundboard change, written in the
/// change's own transaction (`moderation::audit`).
async fn audit_sound(
    c: &mut SqliteConnection,
    u: Id,
    action: AuditAction,
    before: Option<&SoundboardSound>,
    after: Option<&SoundboardSound>,
) -> Result<()> {
    use crate::moderation::audit::{write, Entry};
    let Some(s) = after.or(before) else {
        return Ok(());
    };
    let volume = |s: Option<&SoundboardSound>| s.map(|s| format!("{}%", s.volume));
    let e = Entry::new(s.guild_id, action, AuditTarget::Sound, Some(s.id))
        .named(s.name.clone())
        .change(
            "name",
            before.map(|s| s.name.clone()),
            after.map(|s| s.name.clone()),
        )
        .change(
            "emoji",
            before.and_then(|s| s.emoji.clone()),
            after.and_then(|s| s.emoji.clone()),
        )
        .change("volume", volume(before), volume(after));
    if action == AuditAction::SoundUpdate && before == after {
        return Ok(());
    }
    write(c, u, e).await
}

#[derive(serde::Deserialize)]
struct LibraryQuery {
    channel_id: Option<String>,
}
async fn library(
    State(app): State<App>,
    Query(q): Query<LibraryQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    core.rate(format!("soundboard-list:{}", u.0), 60., 10.)?;
    let channel = match q.channel_id.as_deref() {
        None | Some("") => None,
        Some(s) => Some(Id(s
            .parse()
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?)),
    };
    let (current, can_play) = match channel {
        None => (None, false),
        Some(ch) => {
            let channel = db::access(&mut c, u, ch, Permissions::VIEW_CHANNEL).await?;
            match channel.summary.guild_id {
                None => (None, true),
                Some(g) => (
                    Some(g),
                    permissions::evaluate(&mut c, u, g, Some(ch))
                        .await?
                        .contains(Permissions::USE_SOUNDBOARD),
                ),
            }
        }
    };
    let rows = sqlx::query("SELECT g.id,g.name FROM guilds g JOIN guild_members m ON m.guild_id=g.id WHERE m.user_id=? ORDER BY g.name,g.id")
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?;
    let mut guilds = vec![];
    for r in rows {
        let g = Id(r.get::<i64, _>("id") as u64);
        let sounds = sqlx::query("SELECT id,guild_id,name,emoji,volume,duration_ms,format,created_by FROM soundboard_sounds WHERE guild_id=? ORDER BY name,id")
        .bind(id(g)?)
        .fetch_all(&mut *c)
        .await?
        .iter()
        .map(sound_row)
        .collect();
        guilds.push(SoundboardGuild {
            guild_id: g,
            name: r.get("name"),
            can_manage: can_manage(&mut c, u, g).await?,
            sounds,
        });
    }
    // The call's own server first; then the rest, as they are.
    if let Some(g) = current {
        if let Some(i) = guilds.iter().position(|x| x.guild_id == g) {
            let first = guilds.remove(i);
            guilds.insert(0, first);
        }
    }
    let favorites = sqlx::query_scalar::<_, i64>("SELECT f.sound_id FROM soundboard_favorites f JOIN soundboard_sounds s ON s.id=f.sound_id JOIN guild_members m ON m.guild_id=s.guild_id AND m.user_id=f.user_id WHERE f.user_id=? ORDER BY f.sound_id")
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|v| Id(v as u64))
        .collect();
    Ok(js(&SoundboardLibrary {
        current_guild_id: current,
        can_play,
        guilds,
        favorites,
    }))
}

#[derive(serde::Deserialize)]
struct UploadQuery {
    name: String,
    #[serde(default)]
    emoji: Option<String>,
    #[serde(default)]
    volume: Option<u8>,
}
fn emoji_value(emoji: Option<String>) -> Result<Option<String>> {
    match emoji {
        None => Ok(None),
        Some(e) if e.is_empty() => Ok(None),
        Some(e) => {
            require(valid_sound_emoji(&e), ErrorCode::InvalidRequest)?;
            Ok(Some(e))
        }
    }
}
async fn upload(
    State(app): State<App>,
    Path(g): Path<u64>,
    Query(q): Query<UploadQuery>,
    h: HeaderMap,
    body: Body,
) -> Result<http::ApiResponse> {
    let g = Id(g);
    require(valid_sound_name(&q.name), ErrorCode::InvalidRequest)?;
    let emoji = emoji_value(q.emoji)?;
    let volume = q.volume.unwrap_or(100);
    require(volume <= 100, ErrorCode::InvalidRequest)?;
    let token = token(&h)?;
    let slot = app
        .0
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    let u = {
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await?;
        let (_, u) = db::auth(&mut c, &token).await?;
        manage(&mut c, u, g).await?;
        core.rate(format!("sound-upload:{}", u.0), 20., 5.)?;
        u
    };
    let bytes = to_bytes(body, MAX_SOUND_BYTES)
        .await
        .map_err(|_| Error::new(ErrorCode::PayloadTooLarge))?;
    drop(slot);
    let clip = audio::inspect(&bytes).map_err(|e| match e {
        audio::Rejected::TooLarge => Error::new(ErrorCode::PayloadTooLarge),
        audio::Rejected::TooLong => {
            let mut e = Error::new(ErrorCode::InvalidRequest);
            e.0.message = format!(
                "Sounds can be at most {} seconds long.",
                MAX_SOUND_MS as f32 / 1000.
            );
            e
        }
        audio::Rejected::Unsupported => {
            let mut e = Error::new(ErrorCode::UnsupportedMediaType);
            e.0.message = "Use an MP3, OGG or WAV file.".into();
            e
        }
    })?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    // Auth and permissions can change while the body arrives.
    let (_, current) = db::auth(&mut c, &token).await?;
    require(current == u, ErrorCode::Unauthenticated)?;
    manage(&mut c, u, g).await?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM soundboard_sounds WHERE guild_id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    require(n < MAX_GUILD_SOUNDS as i64, ErrorCode::LimitExceeded)?;
    crate::moderation::coverage::name_check(&mut core, &mut c, u, g, "name", &q.name).await?;
    let sid = core.id().await?;
    let format = match clip.format {
        SoundFormat::Mp3 => "mp3",
        SoundFormat::Ogg => "ogg",
        SoundFormat::Wav => "wav",
    };
    let mut tx = sqlx::Connection::begin(&mut *c).await?;
    sqlx::query("INSERT INTO soundboard_sounds(id,guild_id,name,emoji,volume,duration_ms,format,data,created_by,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind(id(sid)?)
        .bind(id(g)?)
        .bind(&q.name)
        .bind(&emoji)
        .bind(i64::from(volume))
        .bind(i64::from(clip.duration_ms))
        .bind(format)
        .bind(&bytes[..])
        .bind(id(u)?)
        .bind(now() as i64)
        .execute(&mut *tx)
        .await?;
    let created = sound(&mut tx, sid).await?;
    audit_sound(&mut tx, u, AuditAction::SoundCreate, None, Some(&created)).await?;
    core.persist_id(&mut tx).await?;
    tx.commit().await?;
    Ok(js(&created))
}
async fn update(
    State(app): State<App>,
    Path(s): Path<u64>,
    r: Request<Body>,
) -> Result<http::ApiResponse> {
    let token = token(r.headers())?;
    let patch: SoundPatch = body(r).await?;
    if let Some(name) = &patch.name {
        require(valid_sound_name(name), ErrorCode::InvalidRequest)?;
    }
    require(
        patch.volume.is_none_or(|v| v <= 100),
        ErrorCode::InvalidRequest,
    )?;
    let emoji = patch
        .emoji
        .clone()
        .map(|e| emoji_value(Some(e)))
        .transpose()?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token).await?;
    core.rate(format!("sound-edit:{}", u.0), 60., 10.)?;
    let old = sound(&mut c, Id(s)).await?;
    // A non-member cannot learn that the sound exists.
    db::role(&mut c, u, old.guild_id).await?;
    manage(&mut c, u, old.guild_id).await?;
    if let Some(name) = patch.name.as_deref().filter(|n| *n != old.name) {
        crate::moderation::coverage::name_check(&mut core, &mut c, u, old.guild_id, "name", name)
            .await?;
    }
    let mut tx = sqlx::Connection::begin(&mut *c).await?;
    sqlx::query("UPDATE soundboard_sounds SET name=?,emoji=?,volume=? WHERE id=?")
        .bind(patch.name.unwrap_or_else(|| old.name.clone()))
        .bind(emoji.unwrap_or_else(|| old.emoji.clone()))
        .bind(i64::from(patch.volume.unwrap_or(old.volume)))
        .bind(id(old.id)?)
        .execute(&mut *tx)
        .await?;
    let new = sound(&mut tx, old.id).await?;
    audit_sound(&mut tx, u, AuditAction::SoundUpdate, Some(&old), Some(&new)).await?;
    tx.commit().await?;
    Ok(js(&new))
}
async fn delete_sound(
    State(app): State<App>,
    Path(s): Path<u64>,
    h: HeaderMap,
) -> Result<Response> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    core.rate(format!("sound-edit:{}", u.0), 60., 10.)?;
    let old = sound(&mut c, Id(s)).await?;
    db::role(&mut c, u, old.guild_id).await?;
    manage(&mut c, u, old.guild_id).await?;
    let mut tx = sqlx::Connection::begin(&mut *c).await?;
    sqlx::query("DELETE FROM soundboard_sounds WHERE id=?")
        .bind(id(old.id)?)
        .execute(&mut *tx)
        .await?;
    audit_sound(&mut tx, u, AuditAction::SoundDelete, Some(&old), None).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
/// The clip itself: for members of its server, and for anyone in a call with a
/// member of its server (who may have played it there).
async fn clip(State(app): State<App>, Path(s): Path<u64>, h: HeaderMap) -> Result<Response> {
    let core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    let r = sqlx::query("SELECT guild_id,format,data FROM soundboard_sounds WHERE id=?")
        .bind(id(Id(s))?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let g = Id(r.get::<i64, _>("guild_id") as u64);
    if db::role(&mut c, u, g).await.is_err() {
        let mut heard = false;
        for user in voice::companions(&core, u) {
            if db::role(&mut c, user, g).await.is_ok() {
                heard = true;
                break;
            }
        }
        require(heard, ErrorCode::NotFound)?;
    }
    let mime = match r.get::<String, _>("format").as_str() {
        "mp3" => SoundFormat::Mp3,
        "ogg" => SoundFormat::Ogg,
        _ => SoundFormat::Wav,
    }
    .mime();
    let bytes: Vec<u8> = r.get("data");
    Ok((
        StatusCode::OK,
        [
            ("content-type", mime),
            ("cache-control", "private, no-store"),
            ("x-content-type-options", "nosniff"),
            ("content-disposition", "attachment"),
        ],
        bytes,
    )
        .into_response())
}
async fn favorite(State(app): State<App>, Path(s): Path<u64>, h: HeaderMap) -> Result<Response> {
    set_favorite(app, Id(s), h, true).await
}
async fn unfavorite(State(app): State<App>, Path(s): Path<u64>, h: HeaderMap) -> Result<Response> {
    set_favorite(app, Id(s), h, false).await
}
async fn set_favorite(app: App, s: Id, h: HeaderMap, on: bool) -> Result<Response> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    core.rate(format!("sound-favorite:{}", u.0), 120., 20.)?;
    if on {
        let found = sound(&mut c, s).await?;
        db::role(&mut c, u, found.guild_id).await?;
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM soundboard_favorites WHERE user_id=?")
                .bind(id(u)?)
                .fetch_one(&mut *c)
                .await?;
        require(n < MAX_SOUND_FAVORITES as i64, ErrorCode::LimitExceeded)?;
        sqlx::query("INSERT OR IGNORE INTO soundboard_favorites VALUES(?,?)")
            .bind(id(u)?)
            .bind(id(s)?)
            .execute(&mut *c)
            .await?;
    } else {
        sqlx::query("DELETE FROM soundboard_favorites WHERE user_id=? AND sound_id=?")
            .bind(id(u)?)
            .bind(id(s)?)
            .execute(&mut *c)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---- entrance sounds --------------------------------------------------------

async fn entrance_settings(c: &mut SqliteConnection, u: Id) -> Result<EntranceSettings> {
    let enabled: Option<bool> =
        sqlx::query_scalar("SELECT enabled FROM soundboard_entrance_enabled WHERE user_id=?")
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?;
    let rows = sqlx::query(
        "SELECT guild_id,sound_id FROM soundboard_entrance WHERE user_id=? ORDER BY guild_id",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?;
    let mut settings = EntranceSettings {
        enabled: enabled.unwrap_or(true),
        ..Default::default()
    };
    for r in rows {
        let g = r.get::<i64, _>("guild_id") as u64;
        let s = r.get::<Option<i64>, _>("sound_id").map(|v| Id(v as u64));
        if g == 0 {
            settings.default_sound_id = s;
        } else {
            settings.overrides.push(EntranceOverride {
                guild_id: Id(g),
                sound_id: s,
            });
        }
    }
    Ok(settings)
}
async fn entrance(State(app): State<App>, h: HeaderMap) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    Ok(js(&entrance_settings(&mut c, u).await?))
}
async fn set_entrance(State(app): State<App>, r: Request<Body>) -> Result<http::ApiResponse> {
    let token = token(r.headers())?;
    let next: EntranceSettings = body(r).await?;
    require(
        next.overrides.len() <= 200
            && next.overrides.iter().enumerate().all(|(i, o)| {
                o.guild_id.0 != 0 && !next.overrides[..i].iter().any(|p| p.guild_id == o.guild_id)
            }),
        ErrorCode::InvalidRequest,
    )?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token).await?;
    core.rate(format!("sound-entrance:{}", u.0), 30., 10.)?;
    // Only sounds of the user's own servers, and only servers they are in.
    if let Some(s) = next.default_sound_id {
        let found = sound(&mut c, s).await?;
        db::role(&mut c, u, found.guild_id).await?;
    }
    for o in &next.overrides {
        db::role(&mut c, u, o.guild_id).await?;
        if let Some(s) = o.sound_id {
            let found = sound(&mut c, s).await?;
            db::role(&mut c, u, found.guild_id).await?;
        }
    }
    let mut tx = sqlx::Connection::begin(&mut *c).await?;
    sqlx::query("DELETE FROM soundboard_entrance WHERE user_id=?")
        .bind(id(u)?)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO soundboard_entrance_enabled VALUES(?,?) ON CONFLICT(user_id) DO UPDATE SET enabled=excluded.enabled")
        .bind(id(u)?)
        .bind(next.enabled)
        .execute(&mut *tx)
        .await?;
    let rows = next
        .default_sound_id
        .map(|s| (0i64, Some(s)))
        .into_iter()
        .chain(
            next.overrides
                .iter()
                .map(|o| (o.guild_id.0 as i64, o.sound_id)),
        );
    for (g, s) in rows {
        sqlx::query("INSERT INTO soundboard_entrance VALUES(?,?,?)")
            .bind(id(u)?)
            .bind(g)
            .bind(s.map(id).transpose()?)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(js(&entrance_settings(&mut c, u).await?))
}

// ---- playing sounds in a call -----------------------------------------------

/// Everything a session must be allowed before a sound it names is heard.
async fn playable(
    core: &Core,
    c: &mut SqliteConnection,
    session: VoiceSession,
    s: Id,
) -> Result<SoundPlay> {
    let entry = core
        .voice
        .get(&session)
        .ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
    // A moderator's mute is not bypassed by the soundboard, and on a Stage
    // only speakers play sounds.
    require(
        !entry.server_silenced() && !entry.listener,
        ErrorCode::Forbidden,
    )?;
    let (guild, channel) = (entry.state.guild_id, entry.state.channel_id);
    let found = sound(c, s).await?;
    // A sound of a server the player is not in does not exist for them.
    db::role(c, session.user_id, found.guild_id).await?;
    if guild.0 != 0 {
        require(
            permissions::evaluate(c, session.user_id, guild, Some(channel))
                .await?
                .contains(Permissions::USE_SOUNDBOARD),
            ErrorCode::Forbidden,
        )?;
    }
    Ok(SoundPlay {
        sound_id: found.id,
        guild_id: found.guild_id,
        name: found.name,
        emoji: found.emoji,
        volume: found.volume,
        duration_ms: found.duration_ms,
    })
}
/// One event to every connection in that call.
fn room(core: &mut Core, channel: Id, event: &VoiceEvent) {
    for key in voice::room_keys(core, channel) {
        let _ = voice::send(core, &key, 0, event);
    }
}
/// `PlaySound`, `SetClipping` and `ClipTaken`, from a connection in a call.
pub(crate) async fn signal(
    core: &mut Core,
    c: &mut SqliteConnection,
    session: Option<VoiceSession>,
    signal: &VoiceSignal,
) -> Result<()> {
    let session = session
        .filter(|s| core.voice.get(s).is_some_and(|e| e.joined))
        .ok_or_else(|| Error::new(ErrorCode::Forbidden))?;
    let user = session.user_id;
    let channel = core.voice[&session].state.channel_id;
    match signal {
        VoiceSignal::PlaySound { sound_id } => {
            core.rate(format!("sound-play:{}", user.0), 20., 4.)?;
            core.rate(format!("sound-room:{}", channel.0), 60., 8.)?;
            let sound = playable(core, c, session, *sound_id).await?;
            room(
                core,
                channel,
                &VoiceEvent::SoundPlayed {
                    channel_id: channel,
                    sound,
                    by: session,
                    entrance: false,
                },
            );
        }
        VoiceSignal::SetClipping { enabled } => {
            core.rate(format!("clip-state:{}", user.0), 30., 6.)?;
            let entry = core.voice.get_mut(&session).unwrap();
            if entry.clipping != *enabled {
                entry.clipping = *enabled;
                room(
                    core,
                    channel,
                    &VoiceEvent::Clipping {
                        session,
                        enabled: *enabled,
                    },
                );
            }
        }
        VoiceSignal::ClipTaken => {
            core.rate(format!("clip-taken:{}", user.0), 12., 3.)?;
            require(core.voice[&session].clipping, ErrorCode::InvalidRequest)?;
            room(core, channel, &VoiceEvent::ClipTaken { session });
        }
        _ => return Err(Error::new(ErrorCode::InvalidRequest)),
    }
    Ok(())
}
/// A session's media just connected: tell it the call's bitrate and who is
/// clipping, then play its entrance sound. Never fails the join.
pub(crate) async fn joined(core: &mut Core, c: &mut SqliteConnection, session: VoiceSession) {
    if let Err(error) = joined_inner(core, c, session).await {
        tracing::debug!(code = ?error.0.code, "call extras on join skipped");
    }
}
async fn joined_inner(
    core: &mut Core,
    c: &mut SqliteConnection,
    session: VoiceSession,
) -> Result<()> {
    let Some(entry) = core.voice.get(&session) else {
        return Ok(());
    };
    let (guild, channel, key) = (
        entry.state.guild_id,
        entry.state.channel_id,
        entry.key().to_vec(),
    );
    let bitrate = bitrate(c, guild, channel).await?;
    voice::send(
        core,
        &key,
        0,
        &VoiceEvent::AudioQuality {
            channel_id: channel,
            bitrate,
        },
    )?;
    let clipping: Vec<_> = core
        .voice
        .iter()
        .filter(|(s, e)| **s != session && e.clipping && e.state.channel_id == channel)
        .map(|(s, _)| *s)
        .collect();
    for other in clipping {
        voice::send(
            core,
            &key,
            0,
            &VoiceEvent::Clipping {
                session: other,
                enabled: true,
            },
        )?;
    }
    let settings = entrance_settings(c, session.user_id).await?;
    // No entrance sounds on a Stage: hundreds of listeners arrive at once.
    if !settings.enabled || core.voice.get(&session).is_some_and(|e| e.stage) {
        return Ok(());
    }
    let chosen = settings
        .overrides
        .iter()
        .find(|o| guild.0 != 0 && o.guild_id == guild)
        .map(|o| o.sound_id)
        .unwrap_or(settings.default_sound_id);
    let Some(s) = chosen else {
        return Ok(());
    };
    // Rejoining is not a way to spam: one entrance sound a minute.
    core.rate(
        format!("sound-entrance-play:{}", session.user_id.0),
        60_000. / ENTRANCE_SOUND_COOLDOWN_MS as f64,
        1.,
    )?;
    core.rate(format!("sound-room:{}", channel.0), 60., 8.)?;
    let sound = playable(core, c, session, s).await?;
    room(
        core,
        channel,
        &VoiceEvent::SoundPlayed {
            channel_id: channel,
            sound,
            by: session,
            entrance: true,
        },
    );
    Ok(())
}

// ---- a voice channel's bitrate ----------------------------------------------

/// The Opus bitrate a call uses: the channel's setting, or the best for a call.
pub(crate) async fn bitrate(c: &mut SqliteConnection, guild: Id, channel: Id) -> Result<u32> {
    if guild.0 == 0 {
        return Ok(CALL_VOICE_BITRATE);
    }
    let v: Option<i64> =
        sqlx::query_scalar("SELECT bitrate FROM voice_channel_quality WHERE channel_id=?")
            .bind(id(channel)?)
            .fetch_optional(c)
            .await?;
    Ok(v.map_or(DEFAULT_VOICE_BITRATE, |v| v as u32))
}
async fn voice_channel(c: &mut SqliteConnection, u: Id, ch: Id, p: Permissions) -> Result<Id> {
    let channel = db::access(c, u, ch, Permissions::VIEW_CHANNEL | p).await?;
    require(
        matches!(
            channel.summary.kind,
            ChannelKind::Voice | ChannelKind::Stage
        ),
        ErrorCode::InvalidRequest,
    )?;
    channel
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))
}
async fn quality(
    State(app): State<App>,
    Path(ch): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token(&h)?).await?;
    core.rate(format!("rest:{}", u.0), 120., 30.)?;
    let g = voice_channel(&mut c, u, Id(ch), Permissions::empty()).await?;
    Ok(js(&VoiceQuality {
        channel_id: Id(ch),
        bitrate: bitrate(&mut c, g, Id(ch)).await?,
    }))
}
async fn set_quality(
    State(app): State<App>,
    Path(ch): Path<u64>,
    r: Request<Body>,
) -> Result<http::ApiResponse> {
    #[derive(serde::Deserialize)]
    struct Body {
        bitrate: u32,
    }
    let token = token(r.headers())?;
    let next: Body = body(r).await?;
    require(
        (MIN_VOICE_BITRATE..=MAX_VOICE_BITRATE).contains(&next.bitrate),
        ErrorCode::InvalidRequest,
    )?;
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = db::auth(&mut c, &token).await?;
    core.rate(format!("voice-quality:{}", u.0), 30., 10.)?;
    let ch = Id(ch);
    let g = voice_channel(&mut c, u, ch, Permissions::MANAGE_CHANNELS).await?;
    let before = bitrate(&mut c, g, ch).await?;
    let mut tx = sqlx::Connection::begin(&mut *c).await?;
    if next.bitrate == DEFAULT_VOICE_BITRATE {
        sqlx::query("DELETE FROM voice_channel_quality WHERE channel_id=?")
            .bind(id(ch)?)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("INSERT INTO voice_channel_quality VALUES(?,?) ON CONFLICT(channel_id) DO UPDATE SET bitrate=excluded.bitrate")
            .bind(id(ch)?)
            .bind(i64::from(next.bitrate))
            .execute(&mut *tx)
            .await?;
    }
    if before != next.bitrate {
        let name: Option<String> = sqlx::query_scalar("SELECT name FROM channels WHERE id=?")
            .bind(id(ch)?)
            .fetch_one(&mut *tx)
            .await?;
        crate::moderation::audit::write(
            &mut tx,
            u,
            crate::moderation::audit::Entry::new(
                g,
                AuditAction::ChannelUpdate,
                AuditTarget::Channel,
                Some(ch),
            )
            .named(name.unwrap_or_default())
            .change(
                "bitrate",
                Some(format!("{} kbps", before / 1000)),
                Some(format!("{} kbps", next.bitrate / 1000)),
            ),
        )
        .await?;
    }
    tx.commit().await?;
    // Live calls in the channel change now: the SFU's limit and every encoder.
    voice::set_audio_bitrate(&app, ch, next.bitrate);
    room(
        &mut core,
        ch,
        &VoiceEvent::AudioQuality {
            channel_id: ch,
            bitrate: next.bitrate,
        },
    );
    Ok(js(&VoiceQuality {
        channel_id: ch,
        bitrate: next.bitrate,
    }))
}
