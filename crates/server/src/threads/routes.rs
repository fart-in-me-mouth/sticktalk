//! REST reads for threads, forums and followings. Mutations are WebSocket
//! commands (`ThreadCommand`).
use super::*;
use crate::http;
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::get,
};

pub(crate) fn routes() -> axum::Router<App> {
    axum::Router::new()
        .route("/api/v1/threads/{thread}", get(detail))
        .route("/api/v1/threads/{thread}/members", get(members))
        .route("/api/v1/channels/{channel}/threads", get(list))
        .route("/api/v1/channels/{channel}/forum", get(forum_settings))
        .route("/api/v1/channels/{channel}/posts", get(posts))
        .route("/api/v1/channels/{channel}/follows", get(follows))
}

async fn who(app: &App, h: &HeaderMap) -> Result<(sqlx::pool::PoolConnection<sqlx::Sqlite>, Id)> {
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(h)?).await?;
    app.0
        .core
        .lock()
        .await
        .rate(format!("rest:{}", u.0), 120., 30.)?;
    Ok((c, u))
}

async fn detail(
    State(app): State<App>,
    Path(t): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    let (thread, _) = visible(&mut c, u, Id(t)).await?;
    let me = membership(&mut c, thread.id, u).await?;
    let read_state = if me.is_some() {
        Some(read(&mut c, u, thread.id).await?)
    } else {
        None
    };
    Ok(http::json(ThreadDetail {
        thread,
        me,
        read_state,
    }))
}

#[derive(serde::Deserialize)]
struct After {
    after: Option<u64>,
}

async fn members(
    State(app): State<App>,
    Path(t): Path<u64>,
    Query(q): Query<After>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    visible(&mut c, u, Id(t)).await?;
    let rows = sqlx::query("SELECT user_id,joined_at FROM thread_members WHERE thread_id=? AND user_id>? ORDER BY user_id LIMIT ?")
        .bind(id(Id(t))?)
        .bind(q.after.unwrap_or(0).min(i64::MAX as u64) as i64)
        .bind(MAX_THREAD_MEMBERS_PAGE as i64 + 1)
        .fetch_all(&mut *c)
        .await?;
    let has_more = rows.len() > MAX_THREAD_MEMBERS_PAGE;
    let mut page = ThreadMembersPage {
        has_more,
        ..Default::default()
    };
    for r in rows.into_iter().take(MAX_THREAD_MEMBERS_PAGE) {
        page.members.push(ThreadMember {
            user: user(&mut c, Id(r.get::<i64, _>("user_id") as u64)).await?,
            joined_at: r.get::<i64, _>("joined_at").max(0) as u64,
        });
    }
    Ok(http::json(page))
}

#[derive(serde::Deserialize)]
pub(super) struct ListQuery {
    #[serde(default)]
    pub(super) archived: bool,
    pub(super) before: Option<u64>,
    pub(super) limit: Option<u16>,
}

/// The Threads browser: active threads by latest activity, or archived ones by
/// when they were archived. Private threads appear only to their members and
/// to people with Manage Threads.
async fn list(
    State(app): State<App>,
    Path(ch): Path<u64>,
    Query(q): Query<ListQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    let page = threads_page(&mut c, u, Id(ch), q).await?;
    Ok(http::json(page))
}

pub(super) async fn threads_page(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    q: ListQuery,
) -> Result<ThreadPage> {
    let parent = access(
        c,
        u,
        ch,
        Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
    )
    .await?;
    let g = parent
        .summary
        .guild_id
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    let limit = q.limit.unwrap_or(MAX_THREADS_PAGE);
    require(
        (1..=MAX_THREADS_PAGE).contains(&limit),
        ErrorCode::InvalidRequest,
    )?;
    let manage = crate::permissions::evaluate(c, u, g, Some(ch))
        .await?
        .contains(Permissions::MANAGE_THREADS);
    let key = if q.archived {
        "t.archived_at"
    } else {
        "t.last_activity_at"
    };
    let r = sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE t.parent_id=?1 AND t.archived=?2 AND (t.private=0 OR ?3 OR EXISTS(SELECT 1 FROM thread_members m WHERE m.thread_id=t.channel_id AND m.user_id=?4)) AND {key}<?5 ORDER BY {key} DESC,t.channel_id DESC LIMIT ?6")))
        .bind(id(ch)?)
        .bind(q.archived)
        .bind(manage)
        .bind(id(u)?)
        .bind(q.before.unwrap_or(i64::MAX as u64).min(i64::MAX as u64) as i64)
        .bind(i64::from(limit) + 1)
        .fetch_all(&mut *c)
        .await?;
    let mut threads = rows(c, r).await?;
    let has_more = threads.len() > usize::from(limit);
    threads.truncate(usize::from(limit));
    let mut joined = vec![];
    for t in &threads {
        if is_member(c, t.id, u).await? {
            joined.push(t.id);
        }
    }
    let next = if has_more {
        threads.last().map(|t| {
            if q.archived {
                t.archived_at.unwrap_or(0)
            } else {
                t.last_activity_at
            }
        })
    } else {
        None
    };
    Ok(ThreadPage {
        threads,
        joined,
        has_more,
        next,
    })
}

async fn forum_settings(
    State(app): State<App>,
    Path(ch): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    forum::forum(&mut c, u, Id(ch), Permissions::empty()).await?;
    Ok(http::json(forum::settings(&mut c, Id(ch)).await?))
}

async fn posts(
    State(app): State<App>,
    Path(ch): Path<u64>,
    Query(q): Query<forum::PostQuery>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    Ok(http::json(forum::posts(&mut c, u, Id(ch), q).await?))
}

async fn follows(
    State(app): State<App>,
    Path(ch): Path<u64>,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let (mut c, u) = who(&app, &h).await?;
    Ok(http::json(announce::follows(&mut c, u, Id(ch)).await?))
}
