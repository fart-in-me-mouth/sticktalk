use super::*;
use crate::{
    audit_tests::fixture,
    commands, http,
    state::{hash, App},
};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    response::Response,
};
use base64::Engine;
use tower::ServiceExt;

async fn sql(app: &App, q: &'static str) {
    sqlx::raw_sql(q).execute(&app.0.pool).await.unwrap();
}
/// Guild 10 (#11): 1 (owner), 2 and 4. Guild 20 (#21): 1 (owner), 3 and 4.
/// Direct message 30 between 2 and 3. Emoji 500 `party` belongs to guild 20.
async fn world() -> (tempfile::TempDir, App) {
    let (dir, app) = fixture().await;
    sql(
        &app,
        "INSERT INTO guild_members VALUES(10,4,'member',0);
         INSERT INTO guilds VALUES(20,1,'other',NULL,0);
         INSERT INTO guild_members VALUES(20,1,'owner',0),(20,3,'member',0),(20,4,'member',0);
         INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(21,'text',20,'elsewhere',0,0);
         INSERT INTO channels(id,kind,position,created_at) VALUES(30,'direct',0,0);
         INSERT INTO dm_pairs VALUES(2,3,30);
         INSERT INTO dm_members VALUES(30,2,0,1),(30,3,0,1);
         INSERT INTO read_states(user_id,channel_id) VALUES(4,11),(1,21),(3,21),(4,21),(2,30),(3,30);",
    )
    .await;
    sqlx::query("INSERT INTO guild_emoji(id,guild_id,name,animated,content_type,data,preview,creator_id,created_at) VALUES(500,20,'party',0,'image/png',?,x'',3,0)")
        .bind(png(32, 32))
        .execute(&app.0.pool)
        .await
        .unwrap();
    (dir, app)
}
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut v = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        w,
        h,
        image::Rgba([200, 40, 90, 255]),
    ))
    .write_to(&mut v, image::ImageFormat::Png)
    .unwrap();
    v.into_inner()
}
fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let mut v = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([9, 99, 199])))
        .write_to(&mut v, image::ImageFormat::Jpeg)
        .unwrap();
    v.into_inner()
}
fn gif(size: u32, frames: u8) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        for i in 0..frames {
            enc.encode_frame(image::Frame::from_parts(
                image::RgbaImage::from_pixel(size, size, image::Rgba([i * 60, 0, 0, 255])),
                0,
                0,
                image::Delay::from_numer_denom_ms(80, 1),
            ))
            .unwrap();
        }
    }
    out
}
async fn token(app: &App, u: u8) -> String {
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([u + 100; 32]);
    sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
        .bind(hash(token.as_bytes()))
        .bind(i64::from(u))
        .bind(now() as i64)
        .bind((now() + 600000) as i64)
        .bind(now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    token
}
async fn request(app: &App, token: &str, path: &str, bytes: Option<Vec<u8>>) -> Response {
    app.0.core.lock().await.limits.clear();
    let method = if bytes.is_some() { "POST" } else { "GET" };
    http::router(app.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(bytes.unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap()
}
async fn body<T: serde::de::DeserializeOwned>(r: Response) -> T {
    assert_eq!(r.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(r.into_body(), 4 << 20).await.unwrap()).unwrap()
}
async fn run(app: &App, u: u64, nonce: u64, command: Command) -> CommandOutcome {
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
fn ok(outcome: CommandOutcome) -> CommandResult {
    match outcome {
        CommandOutcome::Ok(r) => r,
        CommandOutcome::Err(e) => panic!("command failed: {e:?}"),
    }
}
fn code(outcome: &CommandOutcome) -> Option<ErrorCode> {
    match outcome {
        CommandOutcome::Err(e) => Some(e.code),
        CommandOutcome::Ok(_) => None,
    }
}
fn say(ch: u64, content: &str) -> Command {
    Command::SendMessage(MessageDraft {
        channel_id: Id(ch),
        content: content.into(),
        reply_to: None,
        attachment_ids: vec![],
        mention_ids: vec![],
    })
}
async fn sent(app: &App, u: u64, nonce: u64, ch: u64, content: &str) -> Message {
    match ok(run(app, u, nonce, say(ch, content)).await) {
        CommandResult::Message(m) => m,
        other => panic!("{other:?}"),
    }
}
fn react(ch: u64, m: Id, emoji: &str) -> Command {
    Command::AddReaction {
        channel_id: Id(ch),
        message_id: m,
        emoji: emoji.into(),
    }
}
fn expr(cmd: ExpressionCommand) -> Command {
    Command::Expression(cmd)
}
fn events(core: &Core, u: u8) -> Vec<Event> {
    core.streams[&vec![u; 32]]
        .ring
        .iter()
        .filter_map(
            |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).unwrap() {
                ServerMsg::Event(e) => Some(e.event),
                _ => None,
            },
        )
        .collect()
}
const DENY_EXTERNAL: &str =
    "INSERT INTO permission_overwrites VALUES(11,'role',10,0,52776558133248)";

#[tokio::test]
async fn everyone_has_the_external_bits_by_default() {
    let (_dir, app) = world().await;
    for g in [10_i64, 20] {
        let p: i64 = sqlx::query_scalar("SELECT permissions FROM roles WHERE id=? AND guild_id=?")
            .bind(g)
            .bind(g)
            .fetch_one(&app.0.pool)
            .await
            .unwrap();
        let p = Permissions::from_bits_retain(p as u64);
        assert!(p.contains(Permissions::USE_EXTERNAL_EMOJI | Permissions::USE_EXTERNAL_STICKERS));
    }
    let mut c = app.0.pool.acquire().await.unwrap();
    let p = permissions::evaluate(&mut c, Id(2), Id(10), Some(Id(11)))
        .await
        .unwrap();
    assert!(p.contains(Permissions::USE_EXTERNAL_EMOJI));
}

#[tokio::test]
async fn custom_emoji_cross_server_dm_grants_resolve_and_deletion() {
    let (_dir, app) = world().await;
    let (t2, t4) = (token(&app, 2).await, token(&app, 4).await);
    // Not visible yet to someone outside guild 20.
    assert_eq!(
        request(&app, &t2, "/api/v1/emoji/500", None).await.status(),
        StatusCode::NOT_FOUND
    );
    // A member of 20 uses it in guild 10: kept, and #11's readers may load it.
    let m = sent(&app, 4, 1, 11, "hi <:party:500>!").await;
    assert_eq!(m.content, "hi <:party:500>!");
    let r = request(&app, &t2, "/api/v1/emoji/500", None).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    // Someone outside guild 20 cannot use it: it becomes :party:.
    assert_eq!(
        sent(&app, 2, 2, 11, "<:party:500>").await.content,
        ":party:"
    );
    // A fake or unknown token degrades the same way.
    assert_eq!(sent(&app, 4, 3, 11, "<:nope:999>").await.content, ":nope:");
    // In a direct message, membership is all that is needed.
    assert_eq!(
        sent(&app, 3, 4, 30, "<:party:500>").await.content,
        "<:party:500>"
    );
    // Resolve: the server is named only for its members.
    let info: Vec<EmojiInfo> =
        body(request(&app, &t2, "/api/v1/emoji/resolve?ids=500", None).await).await;
    assert_eq!(info.len(), 1);
    assert!(info[0].available && info[0].guild_name.is_none() && info[0].name == "party");
    let info: Vec<EmojiInfo> =
        body(request(&app, &t4, "/api/v1/emoji/resolve?ids=500", None).await).await;
    assert_eq!(info[0].guild_name.as_deref(), Some("other"));
    assert_eq!(
        request(&app, &t4, "/api/v1/emoji/resolve?ids=", None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    // Reactions follow the same rule.
    assert_eq!(
        code(&run(&app, 2, 5, react(11, m.id, "<:party:500>")).await),
        Some(ErrorCode::InvalidRequest)
    );
    ok(run(&app, 4, 6, react(11, m.id, "<:party:500>")).await);
    // A reaction to a direct message by a member of the emoji's server.
    let dm = sent(&app, 2, 7, 30, "hey").await;
    ok(run(&app, 3, 8, react(30, dm.id, "<:party:500>")).await);
    // Use External Emoji denied in #11: a member of 20 can no longer use it there.
    sql(&app, DENY_EXTERNAL).await;
    assert_eq!(
        sent(&app, 4, 9, 11, "<:party:500>").await.content,
        ":party:"
    );
    assert!(code(&run(&app, 4, 10, react(11, m.id, "<:party:500>")).await).is_some());
    // ...but can still use guild 10's own emoji, and edits are checked too.
    let own = sent(&app, 4, 11, 11, "plain").await;
    let edit = Command::EditMessage(MessageEdit {
        channel_id: Id(11),
        message_id: own.id,
        content: "edited <:party:500>".into(),
        mention_ids: vec![],
    });
    match ok(run(&app, 4, 12, edit).await) {
        CommandResult::Message(m) => assert_eq!(m.content, "edited :party:"),
        other => panic!("{other:?}"),
    }
    // Deleted: the image is gone, and resolve keeps its name for :party:.
    sql(&app, "DELETE FROM guild_emoji WHERE id=500").await;
    assert_eq!(
        request(&app, &t2, "/api/v1/emoji/500", None).await.status(),
        StatusCode::NOT_FOUND
    );
    let info: Vec<EmojiInfo> =
        body(request(&app, &t2, "/api/v1/emoji/resolve?ids=500", None).await).await;
    assert!(!info[0].available && info[0].name == "party");
    let grants: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM emoji_grants WHERE emoji_id=500")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(grants, 0);
}

#[tokio::test]
async fn emoji_slots_are_separate_jpeg_is_accepted_and_admin_needs_manage() {
    let (_dir, app) = world().await;
    let (t1, t2) = (token(&app, 1).await, token(&app, 2).await);
    let r = request(
        &app,
        &t1,
        "/api/v1/guilds/10/emoji?name=photo",
        Some(jpeg(300, 200)),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    for bad in [
        b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
        b"GIF89a".to_vec(),
    ] {
        assert!(
            !request(&app, &t1, "/api/v1/guilds/10/emoji?name=bad", Some(bad))
                .await
                .status()
                .is_success()
        );
    }
    // Fill the still slots; an animated emoji still fits.
    let mut tx = app.0.pool.begin().await.unwrap();
    for i in 0..MAX_GUILD_STATIC_EMOJI as i64 - 1 {
        sqlx::query("INSERT INTO guild_emoji(id,guild_id,name,animated,content_type,data,preview) VALUES(?,10,?,0,'image/png',x'00',x'')")
            .bind(10_000 + i)
            .bind(format!("fill{i}"))
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    let full = request(
        &app,
        &t1,
        "/api/v1/guilds/10/emoji?name=onemore",
        Some(png(64, 64)),
    )
    .await;
    assert!(!full.status().is_success());
    let r = request(
        &app,
        &t1,
        "/api/v1/guilds/10/emoji?name=moving",
        Some(gif(64, 3)),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    // The settings page: Manage Expressions only, with who added each one.
    assert_eq!(
        request(&app, &t2, "/api/v1/guilds/10/expression", None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let admin: GuildExpressionAdmin =
        body(request(&app, &t1, "/api/v1/guilds/10/expression", None).await).await;
    assert_eq!(admin.emoji.len(), MAX_GUILD_STATIC_EMOJI + 1);
    let photo = admin
        .emoji
        .iter()
        .find(|e| e.emoji.name == "photo")
        .unwrap();
    assert_eq!(photo.creator_id, Some(Id(1)));
    assert!(admin.users.iter().any(|u| u.id == Id(1)));
    assert_eq!(admin.max_animated_emoji, 250);
}

#[tokio::test]
async fn stickers_upload_validate_send_across_servers_audit_and_delete() {
    let (_dir, app) = world().await;
    let (t1, t2, t3) = (
        token(&app, 1).await,
        token(&app, 2).await,
        token(&app, 3).await,
    );
    let path = "/api/v1/guilds/20/stickers?name=Big%20Wave&emoji=%F0%9F%91%8B&description=A%20wave";
    // Members without Manage Expressions may not upload.
    assert_eq!(
        request(&app, &t3, path, Some(png(64, 64))).await.status(),
        StatusCode::FORBIDDEN
    );
    // Validation: name, related emoji, format, size and animated dimensions.
    for (p, bytes) in [
        (
            "/api/v1/guilds/20/stickers?name=x&emoji=%F0%9F%91%8B",
            png(64, 64),
        ),
        ("/api/v1/guilds/20/stickers?name=Okay&emoji=ab", png(64, 64)),
        (
            path,
            b"<svg xmlns='http://www.w3.org/2000/svg'></svg>".to_vec(),
        ),
        (path, jpeg(64, 64)),
        (path, vec![0; MAX_STICKER_BYTES + 1]),
        (path, gif(400, 2)),
    ] {
        assert!(
            !request(&app, &t1, p, Some(bytes))
                .await
                .status()
                .is_success(),
            "{p}"
        );
    }
    let big: Sticker = body(request(&app, &t1, path, Some(png(640, 480))).await).await;
    assert!(!big.animated && big.format == StickerFormat::Png);
    let img = image::load_from_memory(
        &to_bytes(
            request(&app, &t1, &format!("/api/v1/stickers/{}", big.id.0), None)
                .await
                .into_body(),
            1 << 20,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!((img.width(), img.height()), (320, 240));
    assert_eq!(
        request(&app, &t1, path, Some(png(64, 64))).await.status(),
        StatusCode::CONFLICT
    );
    let moving: Sticker = body(
        request(
            &app,
            &t1,
            "/api/v1/guilds/20/stickers?name=Spin&emoji=%F0%9F%8E%89",
            Some(gif(120, 3)),
        )
        .await,
    )
    .await;
    assert!(moving.animated && moving.format == StickerFormat::Gif);
    let still = request(
        &app,
        &t1,
        &format!("/api/v1/stickers/{}/still", moving.id.0),
        None,
    )
    .await;
    assert_eq!(still.headers()["content-type"], "image/png");
    // Not yet visible outside guild 20.
    let asset = format!("/api/v1/stickers/{}", big.id.0);
    assert_eq!(
        request(&app, &t2, &asset, None).await.status(),
        StatusCode::NOT_FOUND
    );
    // Sending: one to three, no repeats, stickers-only messages have no text.
    let send = |ids: Vec<Id>, ch: u64, content: &str| {
        expr(ExpressionCommand::SendStickers {
            channel_id: Id(ch),
            sticker_ids: ids,
            content: content.into(),
            reply_to: None,
            mention_ids: vec![],
        })
    };
    assert!(code(&run(&app, 3, 1, send(vec![], 30, "")).await).is_some());
    assert!(code(&run(&app, 3, 2, send(vec![Id(1), Id(2), Id(3), Id(4)], 30, "")).await).is_some());
    assert!(code(&run(&app, 3, 3, send(vec![Id(1), Id(1)], 30, "")).await).is_some());
    assert!(code(&run(&app, 3, 4, send(vec![Id(999)], 30, "")).await).is_some());
    let CommandResult::Message(m) = ok(run(&app, 3, 5, send(vec![big.id, Id(1)], 30, "")).await)
    else {
        panic!()
    };
    assert_eq!(m.content, "");
    assert_eq!(m.extra.stickers().len(), 2);
    assert_eq!(m.extra.stickers()[1].guild_id, None);
    // The other person in the conversation can now load it.
    assert_eq!(
        request(&app, &t2, &asset, None).await.status(),
        StatusCode::OK
    );
    // Someone outside guild 20 cannot send it, with or without text.
    assert_eq!(
        code(&run(&app, 2, 6, send(vec![big.id], 30, "look")).await),
        Some(ErrorCode::Forbidden)
    );
    // Built-in stickers work anywhere, with text.
    let CommandResult::Message(m2) = ok(run(&app, 2, 7, send(vec![Id(4)], 11, "party!")).await)
    else {
        panic!()
    };
    assert_eq!(m2.content, "party!");
    // External stickers in guild 10 follow Use External Stickers.
    ok(run(&app, 4, 8, send(vec![big.id], 11, "")).await);
    sql(&app, DENY_EXTERNAL).await;
    assert_eq!(
        code(&run(&app, 4, 9, send(vec![big.id], 11, "")).await),
        Some(ErrorCode::Forbidden)
    );
    // Rename and delete: Manage Expressions only, recorded in the audit log.
    let edit = |name: &str| {
        expr(ExpressionCommand::UpdateSticker {
            guild_id: Id(20),
            sticker_id: big.id,
            name: name.into(),
            emoji: "🌊".into(),
            description: "A big wave".into(),
        })
    };
    assert_eq!(
        code(&run(&app, 3, 10, edit("Mine")).await),
        Some(ErrorCode::Forbidden)
    );
    assert!(code(&run(&app, 1, 11, edit("Spin")).await).is_some());
    ok(run(&app, 1, 12, edit("Huge Wave")).await);
    ok(run(
        &app,
        1,
        13,
        expr(ExpressionCommand::DeleteSticker {
            guild_id: Id(20),
            sticker_id: big.id,
        }),
    )
    .await);
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_log WHERE guild_id=20 ORDER BY id")
            .fetch_all(&app.0.pool)
            .await
            .unwrap();
    for a in ["sticker_create", "sticker_update", "sticker_delete"] {
        assert!(actions.iter().any(|v| v == a), "{a} in {actions:?}");
    }
    // The message keeps its copy, marked deleted; the image is gone.
    let mut c = app.0.pool.acquire().await.unwrap();
    let again = message(&mut c, Id(2), Id(30), m.id, false).await.unwrap();
    assert!(again.extra.stickers()[0].deleted && again.extra.stickers()[0].name == "Big Wave");
    assert!(!again.extra.stickers()[1].deleted);
    assert_eq!(
        request(&app, &t2, &asset, None).await.status(),
        StatusCode::NOT_FOUND
    );
    // Favorites: only stickers you can use.
    assert!(code(
        &run(
            &app,
            2,
            14,
            expr(ExpressionCommand::SetStickerFavorites {
                sticker_ids: vec![moving.id]
            })
        )
        .await
    )
    .is_some());
    ok(run(
        &app,
        3,
        15,
        expr(ExpressionCommand::SetStickerFavorites {
            sticker_ids: vec![moving.id, Id(1)],
        }),
    )
    .await);
    let lib: ExpressionLibrary =
        body(request(&app, &t3, "/api/v1/users/@me/expression", None).await).await;
    assert_eq!(lib.settings.sticker_favorites, vec![moving.id, Id(1)]);
    assert!(
        lib.stickers[0].guild_id.is_none()
            && lib.stickers[0].stickers.len() == BUILTIN_STICKERS.len()
    );
    assert!(lib.stickers.iter().any(|s| s.guild_id == Some(Id(20))));
    assert!(lib.emoji.iter().any(|e| e.guild_id == Id(20)));
    // The sticker limit.
    let mut tx = app.0.pool.begin().await.unwrap();
    for i in 0..MAX_GUILD_STICKERS as i64 {
        sqlx::query("INSERT INTO guild_stickers(id,guild_id,name,emoji,format,animated,data,still,created_at) VALUES(?,10,?,'👍','png',0,x'00',x'',0)")
            .bind(20_000 + i)
            .bind(format!("fill {i}"))
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    let full = request(
        &app,
        &t1,
        "/api/v1/guilds/10/stickers?name=Extra&emoji=%F0%9F%91%8D",
        Some(png(64, 64)),
    )
    .await;
    assert!(!full.status().is_success());
}

#[tokio::test]
async fn super_reactions_count_apart_and_reach_only_subscribed_streams() {
    let (_dir, app) = world().await;
    let m = sent(&app, 2, 1, 11, "hello").await;
    // User 1 has an old stream (key [1;32]) and a new one that subscribes.
    {
        let mut core = app.0.core.lock().await;
        core.cursor(&[1; 32], Id(1));
        core.cursor(&[7; 32], Id(1));
        core.streams
            .get_mut(&vec![7; 32])
            .unwrap()
            .expression_features = true;
    }
    let add = |emoji: &str| {
        expr(ExpressionCommand::AddSuperReaction {
            channel_id: Id(11),
            message_id: m.id,
            emoji: emoji.into(),
        })
    };
    let CommandResult::Expression(ExpressionResult::SuperReaction(r)) =
        ok(run(&app, 2, 2, add("👍")).await)
    else {
        panic!()
    };
    assert!(r.added && r.reaction.count == 1 && r.reaction.me);
    ok(run(&app, 4, 3, add("👍")).await);
    ok(run(&app, 4, 4, react(11, m.id, "👍")).await);
    assert!(code(&run(&app, 4, 5, add("not an emoji")).await).is_some());
    // A custom emoji from a server the reactor is not in.
    assert!(code(&run(&app, 2, 6, add("<:party:500>")).await).is_some());
    ok(run(&app, 4, 7, add("<:party:500>")).await);
    let mut c = app.0.pool.acquire().await.unwrap();
    let seen = message(&mut c, Id(2), Id(11), m.id, false).await.unwrap();
    assert_eq!(seen.reactions.len(), 1);
    assert_eq!(seen.reactions[0].count, 1);
    let thumbs = seen
        .extra
        .super_reactions()
        .iter()
        .find(|r| r.emoji == "👍")
        .unwrap();
    assert!(thumbs.count == 2 && thumbs.me);
    assert!(seen
        .extra
        .super_reactions()
        .iter()
        .any(|r| r.emoji == "<:emoji:500>"));
    {
        let core = app.0.core.lock().await;
        let old = events(&core, 1);
        assert!(!old.iter().any(|e| matches!(e, Event::Expression(_))));
        assert!(old.iter().any(|e| matches!(e, Event::ReactionUpdated(_))));
        let new: Vec<_> = core.streams[&vec![7; 32]]
            .ring
            .iter()
            .filter_map(
                |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).unwrap() {
                    ServerMsg::Event(e) => Some(e.event),
                    _ => None,
                },
            )
            .collect();
        assert!(new.iter().any(|e| matches!(e, Event::Expression(ExpressionEvent::SuperReactionUpdated(u)) if u.added && u.reaction.emoji == "👍")));
    }
    // Removing one, then deleting the message, clears them.
    ok(run(
        &app,
        2,
        8,
        expr(ExpressionCommand::RemoveSuperReaction {
            channel_id: Id(11),
            message_id: m.id,
            emoji: "👍".into(),
        }),
    )
    .await);
    let seen = message(&mut c, Id(2), Id(11), m.id, false).await.unwrap();
    let thumbs = seen
        .extra
        .super_reactions()
        .iter()
        .find(|r| r.emoji == "👍")
        .unwrap();
    assert!(thumbs.count == 1 && !thumbs.me);
    // Read-only members may not react.
    sql(
        &app,
        "INSERT INTO permission_overwrites VALUES(11,'member',2,0,16)",
    )
    .await;
    assert_eq!(
        code(&run(&app, 2, 9, add("🎉")).await),
        Some(ErrorCode::Forbidden)
    );
    ok(run(
        &app,
        2,
        10,
        Command::DeleteMessage {
            channel_id: Id(11),
            message_id: m.id,
        },
    )
    .await);
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM super_reactions")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn typing_styles_are_validated_synced_and_sent_before_typing() {
    let (_dir, app) = world().await;
    let set = |style: TypingStyle| expr(ExpressionCommand::SetTypingStyle { style: Some(style) });
    let style = |emoji: &[&str], verb: TypingVerb| TypingStyle {
        emoji: emoji.iter().map(|e| (*e).to_owned()).collect(),
        motion: TypingMotion::Wave,
        verb,
    };
    for (n, bad) in [
        style(&["✍️"], TypingVerb::Custom("typing!".into())),
        style(&["✍️"], TypingVerb::Custom("a".repeat(21))),
        style(&["✍️", "🎉", "🔥", "💬"], TypingVerb::Typing),
        style(&["ab"], TypingVerb::Typing),
        // A custom emoji from a server user 2 is not in.
        style(&["<:party:500>"], TypingVerb::Typing),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            code(&run(&app, 2, n as u64 + 1, set(bad)).await),
            Some(ErrorCode::InvalidRequest)
        );
    }
    let chosen = style(
        &["✍️", "<:party:500>"],
        TypingVerb::Custom("scribbling".into()),
    );
    // User 2 keeps a subscribed stream open on another device.
    {
        let mut core = app.0.core.lock().await;
        core.cursor(&[8; 32], Id(4));
        core.streams
            .get_mut(&vec![8; 32])
            .unwrap()
            .expression_features = true;
        core.cursor(&[9; 32], Id(1));
        core.streams
            .get_mut(&vec![9; 32])
            .unwrap()
            .expression_features = true;
    }
    let CommandResult::Expression(ExpressionResult::Settings(s)) =
        ok(run(&app, 4, 10, set(chosen.clone())).await)
    else {
        panic!()
    };
    assert_eq!(s.typing.as_ref(), Some(&chosen));
    ok(run(&app, 4, 11, Command::Typing { channel_id: Id(11) }).await);
    let core = app.0.core.lock().await;
    let mine: Vec<_> = core.streams[&vec![8; 32]]
        .ring
        .iter()
        .filter_map(
            |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).unwrap() {
                ServerMsg::Event(e) => Some(e.event),
                _ => None,
            },
        )
        .collect();
    assert!(mine.iter().any(|e| matches!(e, Event::Expression(ExpressionEvent::SettingsUpdated(s)) if s.typing.is_some())));
    let theirs: Vec<_> = core.streams[&vec![9; 32]]
        .ring
        .iter()
        .filter_map(
            |(_, _, b)| match rmp_serde::from_slice::<ServerMsg>(b).unwrap() {
                ServerMsg::Event(e) => Some(e.event),
                _ => None,
            },
        )
        .collect();
    let style_at = theirs
        .iter()
        .position(|e| matches!(e, Event::Expression(ExpressionEvent::TypingStyle { user_id, style }) if *user_id == Id(4) && style.emoji.len() == 2))
        .expect("style event");
    let typing_at = theirs
        .iter()
        .position(|e| matches!(e, Event::Typing { user_id, .. } if *user_id == Id(4)))
        .expect("typing event");
    assert!(style_at < typing_at);
}
