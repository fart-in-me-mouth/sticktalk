//! Permission-filtered, keyset-paginated message search. No raw FTS syntax from clients.
use crate::{
    db::*,
    error::{require, Error, Result},
    http,
    state::App,
};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use sticktalk_proto::*;
#[derive(Default, Debug)]
struct Filters {
    words: Vec<String>,
    from: Option<String>,
    channel: Option<String>,
    mentions: Option<String>,
    before: Option<u64>,
    after: Option<u64>,
    link: bool,
    file: bool,
    image: bool,
    video: bool,
}
fn parse(q: &str) -> Result<Filters> {
    require(
        !q.trim().is_empty() && q.len() <= 512,
        ErrorCode::InvalidRequest,
    )?;
    let mut f = Filters::default();
    // Quoted phrases are one FTS phrase. Filter operands are identifiers (no spaces).
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    for c in q.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            _ => token.push(c),
        }
    }
    require(!quoted, ErrorCode::InvalidRequest)?;
    if !token.is_empty() {
        tokens.push(token);
    }
    require(tokens.len() <= 32, ErrorCode::InvalidRequest)?;
    for token in tokens {
        let value = token.split_once(':');
        match value {
            Some(("from", s)) => {
                require(f.from.is_none() && !s.is_empty(), ErrorCode::InvalidRequest)?;
                f.from = Some(s.to_owned());
            }
            Some(("in", s)) => {
                require(
                    f.channel.is_none() && !s.is_empty(),
                    ErrorCode::InvalidRequest,
                )?;
                f.channel = Some(s.trim_start_matches('#').to_owned());
            }
            Some(("mentions", s)) => {
                require(
                    f.mentions.is_none() && !s.is_empty(),
                    ErrorCode::InvalidRequest,
                )?;
                f.mentions = Some(s.to_owned());
            }
            Some(("before", s)) => {
                require(f.before.is_none(), ErrorCode::InvalidRequest)?;
                f.before = Some(date(s)?);
            }
            Some(("after", s)) => {
                require(f.after.is_none(), ErrorCode::InvalidRequest)?;
                f.after = Some(date(s)?);
            }
            Some(("has", "link")) => f.link = true,
            Some(("has", "file")) => f.file = true,
            Some(("has", "image")) => f.image = true,
            Some(("has", "video")) => f.video = true,
            Some(("during", s)) => {
                require(
                    f.before.is_none() && f.after.is_none(),
                    ErrorCode::InvalidRequest,
                )?;
                let day = date(s)?;
                f.after = Some(day.saturating_sub(1));
                f.before = Some(day + 86_400_000);
            }
            Some(("has", _)) => return Err(Error::new(ErrorCode::InvalidRequest)),
            _ => f.words.push(token),
        }
    }
    require(
        f.before.zip(f.after).is_none_or(|(b, a)| a < b),
        ErrorCode::InvalidRequest,
    )?;
    Ok(f)
}
fn date(s: &str) -> Result<u64> {
    let invalid = || Error::new(ErrorCode::InvalidRequest);
    let p: Vec<_> = s.split('-').collect();
    require(
        p.len() == 3
            && p[0].len() == 4
            && p[1].len() == 2
            && p[2].len() == 2
            && p.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit())),
        ErrorCode::InvalidRequest,
    )?;
    let y: u64 = p[0].parse().map_err(|_| invalid())?;
    let m: usize = p[1].parse().map_err(|_| invalid())?;
    let d: u64 = p[2].parse().map_err(|_| invalid())?;
    require(
        (1970..=2200).contains(&y) && (1..=12).contains(&m),
        ErrorCode::InvalidRequest,
    )?;
    let leap = y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    require(d > 0 && d <= months[m - 1], ErrorCode::InvalidRequest)?;
    let leaps = |y: u64| (y - 1) / 4 - (y - 1) / 100 + (y - 1) / 400;
    Ok(
        (365 * (y - 1970) + leaps(y) - leaps(1970) + months[..m - 1].iter().sum::<u64>() + d - 1)
            * 86_400_000,
    )
}
async fn user_filter(c: &mut SqliteConnection, u: Id, value: &str) -> Result<Option<Id>> {
    if value == "me" {
        return Ok(Some(u));
    }
    let value = value
        .trim_start_matches('@')
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_start_matches('@');
    if let Ok(n) = value.parse::<u64>() {
        return Ok((n > 0 && n <= i64::MAX as u64).then_some(Id(n)));
    }
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE username=?")
            .bind(value.to_ascii_lowercase())
            .fetch_optional(c)
            .await?
            .map(|n| Id(n as u64)),
    )
}
pub(crate) async fn endpoint(
    State(app): State<App>,
    Path(g): Path<u64>,
    Query(q): Query<MessageSearchQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("search:{}", u.0), 30., 5.)?;
    let (items, users, next_before, has_more, terms) = search(&mut c, u, Id(g), &q).await?;
    Ok(http::json(Snapshot {
        data: MessageSearchPage {
            items,
            users,
            next_before,
            has_more,
            terms,
        },
        cursor: core.cursor(&key, u),
    }))
}
#[derive(serde::Deserialize)]
pub(crate) struct DirectQuery {
    q: String,
    before: Option<Id>,
    limit: Option<u16>,
    channel_id: Option<Id>,
}
/// GET /api/v1/users/@me/messages/search: direct messages and groups only.
pub(crate) async fn direct_endpoint(
    State(app): State<App>,
    Query(q): Query<DirectQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let mut core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (key, u) = auth(&mut c, http::bearer(&h)?).await?;
    core.rate(format!("search:{}", u.0), 30., 5.)?;
    let query = MessageSearchQuery {
        q: q.q,
        before: q.before,
        limit: q.limit,
    };
    let (items, users, next_before, has_more, terms) =
        direct(&mut c, u, q.channel_id, &query).await?;
    Ok(http::json(Snapshot {
        data: MessageSearchPage {
            items,
            users,
            next_before,
            has_more,
            terms,
        },
        cursor: core.cursor(&key, u),
    }))
}
pub(crate) type SearchResult = (
    Vec<Message>,
    Vec<UserSummary>,
    Option<Id>,
    bool,
    Vec<String>,
);
pub(crate) async fn search(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    q: &MessageSearchQuery,
) -> Result<SearchResult> {
    guild(c, u, g).await?;
    let f = checked(q)?;
    let mut readable = Vec::new();
    for ch in channels(c, u, Some(g)).await? {
        if ch.kind != ChannelKind::Text {
            continue;
        }
        if let Some(filter) = &f.channel {
            if ch.name.as_ref() != Some(filter) && filter.parse::<u64>().ok() != Some(ch.id.0) {
                continue;
            }
        }
        if access(
            c,
            u,
            ch.id,
            Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
        )
        .await
        .is_ok()
            && !crate::privacy::gate::age_blocked(c, u, ch.id).await?
        {
            readable.push(ch.id);
        }
    }
    run(c, u, readable, f, q).await
}
/// Parses a query and its paging bounds, with the error message the search box shows.
fn checked(q: &MessageSearchQuery) -> Result<Filters> {
    let f = parse(&q.q).map_err(|mut e| {
        e.0.message = "Check your search filters and dates (YYYY-MM-DD).".into();
        e.0.field = Some("q".into());
        e
    })?;
    let limit = q.limit.unwrap_or(25);
    require((1..=50).contains(&limit), ErrorCode::InvalidRequest)?;
    if let Some(b) = q.before {
        id(b)?;
    }
    Ok(f)
}
/// Direct messages and groups `u` is a member of, open or closed. `only`
/// narrows to one of them; anything else is not found.
pub(crate) async fn direct(
    c: &mut SqliteConnection,
    u: Id,
    only: Option<Id>,
    q: &MessageSearchQuery,
) -> Result<SearchResult> {
    let f = checked(q)?;
    let mut readable = Vec::new();
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT channel_id FROM dm_members WHERE user_id=? ORDER BY channel_id")
            .bind(id(u)?)
            .fetch_all(&mut *c)
            .await?;
    for ch in ids {
        let ch = Id(ch as u64);
        if only.is_some_and(|o| o != ch) {
            continue;
        }
        let channel = access(c, u, ch, Permissions::empty()).await?;
        if let Some(filter) = &f.channel {
            let mut names = vec![ch.0.to_string()];
            names.extend(channel.summary.name.clone().map(|n| n.to_lowercase()));
            for v in &channel.summary.recipient_ids {
                if *v != u {
                    names.push(user(c, *v).await?.username);
                }
            }
            let filter = filter.trim_start_matches('@').to_lowercase();
            if !names.contains(&filter) {
                continue;
            }
        }
        readable.push(ch);
    }
    if let Some(o) = only {
        require(
            readable.contains(&o) || f.channel.is_some(),
            ErrorCode::NotFound,
        )?;
    }
    run(c, u, readable, f, q).await
}
async fn run(
    c: &mut SqliteConnection,
    u: Id,
    readable: Vec<Id>,
    f: Filters,
    q: &MessageSearchQuery,
) -> Result<SearchResult> {
    let limit = q.limit.unwrap_or(25);
    if readable.is_empty() {
        return Ok((vec![], vec![], None, false, f.words));
    }
    let from = if let Some(s) = &f.from {
        let Some(v) = user_filter(c, u, s).await? else {
            return Ok((vec![], vec![], None, false, f.words));
        };
        Some(v)
    } else {
        None
    };
    let mentions = if let Some(s) = &f.mentions {
        let Some(v) = user_filter(c, u, s).await? else {
            return Ok((vec![], vec![], None, false, f.words));
        };
        Some(v)
    } else {
        None
    };
    let mut query = QueryBuilder::<Sqlite>::new("SELECT m.id FROM messages m ");
    if !f.words.is_empty() {
        query.push("JOIN message_fts ON message_fts.rowid=m.id ");
    }
    query.push("WHERE m.deleted_at IS NULL AND m.channel_id IN (");
    {
        let mut seq = query.separated(",");
        for ch in readable {
            seq.push_bind(id(ch)?);
        }
        seq.push_unseparated(")");
    }
    if !f.words.is_empty() {
        query.push(" AND message_fts MATCH ").push_bind(
            f.words
                .iter()
                .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" AND "),
        );
    }
    if let Some(v) = q.before {
        query.push(" AND m.id<").push_bind(id(v)?);
    }
    if let Some(v) = from {
        query.push(" AND m.author_id=").push_bind(id(v)?);
    }
    if let Some(v) = mentions {
        query.push(" AND EXISTS(SELECT 1 FROM message_mentions mm WHERE mm.message_id=m.id AND mm.user_id=").push_bind(id(v)?).push(")");
    }
    if let Some(v) = f.before {
        query.push(" AND m.created_at<").push_bind(v as i64);
    }
    if let Some(v) = f.after {
        query.push(" AND m.created_at>").push_bind(v as i64);
    }
    if f.link {
        query.push(
            " AND (instr(lower(m.content),'https://')>0 OR instr(lower(m.content),'http://')>0)",
        );
    }
    if f.file || f.image || f.video {
        query.push(" AND EXISTS(SELECT 1 FROM message_attachments ma JOIN uploads a ON a.id=ma.attachment_id WHERE ma.message_id=m.id");
        if f.image {
            query.push(" AND a.content_type LIKE 'image/%'");
        }
        if f.video {
            query.push(" AND a.content_type LIKE 'video/%'");
        }
        query.push(")");
    }
    query
        .push(" ORDER BY m.id DESC LIMIT ")
        .push_bind(i64::from(limit) + 1);
    let mut rows = query.build().fetch_all(&mut *c).await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let mut items = Vec::new();
    let mut users = std::collections::BTreeMap::new();
    for r in rows {
        let mid = Id(r.get::<i64, _>("id") as u64);
        let ch: i64 = sqlx::query_scalar("SELECT channel_id FROM messages WHERE id=?")
            .bind(id(mid)?)
            .fetch_one(&mut *c)
            .await?;
        let message = message(c, u, Id(ch as u64), mid, false).await?;
        users.insert(message.author_id, user(c, message.author_id).await?);
        items.push(message);
    }
    let before = if more {
        items.last().map(|m| m.id)
    } else {
        None
    };
    Ok((items, users.into_values().collect(), before, more, f.words))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audit_tests::fixture, permissions, state};
    use sqlx::Connection;
    fn query(q: &str) -> MessageSearchQuery {
        MessageSearchQuery {
            q: q.into(),
            before: None,
            limit: Some(2),
        }
    }
    #[test]
    fn validates_dates_filters_and_quotes_literal_fts_terms() {
        assert_eq!(date("2025-01-01").unwrap(), SNOWFLAKE_EPOCH_MS);
        for q in [
            "",
            "has:audio",
            "during:2026-01-01 after:2026-01-02",
            "from:",
            "before:2026-02-29",
            "after:2026-12-01 before:2026-01-01",
            "from:1 from:2",
            "\"unclosed",
        ] {
            assert!(parse(q).is_err(), "{q}");
        }
        let f=parse("\"launch notes\" from:me in:general has:link has:file mentions:2 before:2027-01-01 after:2026-01-01").unwrap();
        assert_eq!(f.words, ["launch notes"]);
        assert!(f.link && f.file);
        assert_eq!(f.mentions.as_deref(), Some("2"));
    }
    #[tokio::test]
    async fn migration_backfills_and_tracks_soft_delete_edit_hard_delete() {
        let mut c = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TABLE messages(id INTEGER PRIMARY KEY,content TEXT,deleted_at INTEGER);INSERT INTO messages VALUES(1,'existing launch notes',NULL),(2,'deleted launch',1);").execute(&mut c).await.unwrap();
        sqlx::raw_sql(include_str!(
            "../migrations/202611130003_message_search.sql"
        ))
        .execute(&mut c)
        .await
        .unwrap();

        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM message_fts WHERE message_fts MATCH ?"
            )
            .bind("launch")
            .fetch_one(&mut c)
            .await
            .unwrap(),
            1
        );
        sqlx::query("UPDATE messages SET content='edited sketch' WHERE id=1")
            .execute(&mut c)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM message_fts WHERE message_fts MATCH ?"
            )
            .bind("launch")
            .fetch_one(&mut c)
            .await
            .unwrap(),
            0
        );
        sqlx::query("INSERT INTO messages VALUES(3,'sketch',NULL)")
            .execute(&mut c)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM message_fts WHERE message_fts MATCH ?"
            )
            .bind("sketch")
            .fetch_one(&mut c)
            .await
            .unwrap(),
            2
        );
        sqlx::query("UPDATE messages SET deleted_at=1 WHERE id=1")
            .execute(&mut c)
            .await
            .unwrap();
        sqlx::query("DELETE FROM messages WHERE id=3")
            .execute(&mut c)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM message_fts WHERE message_fts MATCH ?"
            )
            .bind("sketch")
            .fetch_one(&mut c)
            .await
            .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn search_filters_keysets_and_permissions_are_applied_before_limit() {
        let (_dir, app) = fixture().await;
        sqlx::raw_sql("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'text',10,'hidden',1,0);INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(100,11,1,'launch notes https://example.test',1767225600001),(101,11,2,'launch sketches',1767225600002),(102,11,2,'launch images',1767225600003),(103,12,1,'launch secret',1767225600004);INSERT INTO message_mentions VALUES(100,2);").execute(&app.0.pool).await.unwrap();
        sqlx::query("INSERT INTO permission_overwrites VALUES(12,'member',2,0,?)")
            .bind(Permissions::READ_HISTORY.bits() as i64)
            .execute(&app.0.pool)
            .await
            .unwrap();
        let mut c = app.0.pool.acquire().await.unwrap();
        let (page, _, next, more, _) = search(&mut c, Id(2), Id(10), &query("launch"))
            .await
            .unwrap();
        assert!(more);
        assert_eq!(page.iter().map(|m| m.id.0).collect::<Vec<_>>(), [102, 101]);
        let mut q = query("launch");
        q.before = next;
        let (page, _, _, more, _) = search(&mut c, Id(2), Id(10), &q).await.unwrap();
        assert!(!more);
        assert_eq!(page[0].id, Id(100));
        let (page,_,_,_,_)=search(&mut c,Id(2),Id(10),&query("from:user1 mentions:me has:link in:general after:2026-01-01 before:2027-01-01 launch")).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, Id(100));
        assert!(search(&mut c, Id(3), Id(10), &query("secret"))
            .await
            .is_err());
        assert!(search(&mut c, Id(2), Id(10), &query("in:hidden launch"))
            .await
            .unwrap()
            .0
            .is_empty());
        assert!(search(&mut c, Id(2), Id(10), &query("from:unknown launch"))
            .await
            .unwrap()
            .0
            .is_empty());
        let all = search(&mut c, Id(1), Id(10), &query("secret"))
            .await
            .unwrap();
        assert_eq!(all.0[0].id, Id(103));
        // Literal FTS operators cannot expand a query or escape the SQL boundary.
        for value in [
            "launch OR secret",
            "launch' UNION SELECT",
            "NEAR(launch)",
            "*",
        ] {
            assert!(
                search(&mut c, Id(2), Id(10), &query(value)).await.is_ok(),
                "{value}"
            );
        }
        // Attachment-type predicates use the actual stored MIME and message binding.
        sqlx::query("INSERT INTO uploads(id,owner_id,purpose,channel_id,filename,content_type,byte_size,storage_key,created_at) VALUES(200,2,'message',11,'image.png','image/png',1,'search-test',0)").execute(&mut *c).await.unwrap();
        sqlx::query("INSERT INTO message_attachments VALUES(102,200,0)")
            .execute(&mut *c)
            .await
            .unwrap();
        for value in ["has:file launch", "has:image launch"] {
            let rows = search(&mut c, Id(2), Id(10), &query(value))
                .await
                .unwrap()
                .0;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, Id(102));
        }
        // A category deny also removes descendants from search, not just the sidebar.
        sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(13,'category',10,'private',2,0)").execute(&mut *c).await.unwrap();
        sqlx::query("UPDATE channels SET parent_id=13 WHERE id=11")
            .execute(&mut *c)
            .await
            .unwrap();
        sqlx::query("INSERT INTO permission_overwrites VALUES(13,'member',2,0,?)")
            .bind(Permissions::VIEW_CHANNEL.bits() as i64)
            .execute(&mut *c)
            .await
            .unwrap();
        assert!(
            permissions::evaluate(&mut c, Id(2), Id(10), Some(Id(11)))
                .await
                .unwrap()
                & Permissions::VIEW_CHANNEL
                == Permissions::empty()
        );
        assert!(search(&mut c, Id(2), Id(10), &query("launch"))
            .await
            .unwrap()
            .0
            .is_empty());
        drop(c);
        // Read endpoint authenticates, returns a bounded cursor-bearing result, and does not bind a socket.
        use base64::Engine;
        use tower::ServiceExt;
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7; 32]);
        sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
            .bind(state::hash(token.as_bytes()))
            .bind(1i64)
            .bind(state::now() as i64)
            .bind((state::now() + 600000) as i64)
            .bind(state::now() as i64)
            .execute(&app.0.pool)
            .await
            .unwrap();
        let response = http::router(app.clone())
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/guilds/10/messages/search?q=launch&limit=2")
                    .header("authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        let v: Snapshot<MessageSearchPage> = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(v.data.items.len(), 2);
        assert!(v.data.has_more);
    }
}
