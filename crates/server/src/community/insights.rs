//! Server insights: small daily rollups, updated as events happen, kept for
//! `INSIGHTS_DAYS`. Reading them never scans messages, and nothing runs on a
//! schedule: old rows are trimmed by the first event of a server each day.
//! Only aggregate counts leave this module.
use super::*;

const DAY_MS: u64 = 86_400_000;

fn today() -> i64 {
    i64::from(day_of(now()))
}
pub(super) async fn joined(
    c: &mut SqliteConnection,
    g: Id,
    u: Id,
    source: JoinSource,
    code: Option<&str>,
) -> Result<()> {
    sqlx::query("INSERT INTO insight_daily(guild_id,day,joins) VALUES(?,?,1) ON CONFLICT(guild_id,day) DO UPDATE SET joins=joins+1")
        .bind(id(g)?)
        .bind(today())
        .execute(&mut *c)
        .await?;
    // Invite codes are kept only for invites; they name which link worked.
    let code = if source == JoinSource::Invite {
        code.unwrap_or_default()
    } else {
        ""
    };
    sqlx::query("INSERT INTO insight_sources(guild_id,day,source,code,joins) VALUES(?,?,?,?,1) ON CONFLICT(guild_id,day,source,code) DO UPDATE SET joins=joins+1")
        .bind(id(g)?)
        .bind(today())
        .bind(enum_str(source))
        .bind(code)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO insight_joins(guild_id,user_id,joined_at) VALUES(?,?,?)")
        .bind(id(g)?)
        .bind(id(u)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    Ok(())
}
pub(super) async fn left(c: &mut SqliteConnection, g: Id, u: Id) -> Result<()> {
    sqlx::query("INSERT INTO insight_daily(guild_id,day,leaves) VALUES(?,?,1) ON CONFLICT(guild_id,day) DO UPDATE SET leaves=leaves+1")
        .bind(id(g)?)
        .bind(today())
        .execute(&mut *c)
        .await?;
    sqlx::query(
        "UPDATE insight_joins SET left_at=? WHERE guild_id=? AND user_id=? AND left_at IS NULL",
    )
    .bind(now() as i64)
    .bind(id(g)?)
    .bind(id(u)?)
    .execute(&mut *c)
    .await?;
    Ok(())
}
pub(super) async fn message(c: &mut SqliteConnection, g: Id, ch: Id, u: Id) -> Result<()> {
    sqlx::query("INSERT INTO insight_daily(guild_id,day,messages) VALUES(?,?,1) ON CONFLICT(guild_id,day) DO UPDATE SET messages=messages+1")
        .bind(id(g)?)
        .bind(today())
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT INTO insight_channels(guild_id,day,channel_id,messages) VALUES(?,?,?,1) ON CONFLICT(guild_id,day,channel_id) DO UPDATE SET messages=messages+1")
        .bind(id(g)?)
        .bind(today())
        .bind(id(ch)?)
        .execute(&mut *c)
        .await?;
    sqlx::query("UPDATE insight_joins SET first_message_at=? WHERE guild_id=? AND user_id=? AND left_at IS NULL AND first_message_at IS NULL")
        .bind(now() as i64)
        .bind(id(g)?)
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    Ok(())
}
pub(super) async fn voice(c: &mut SqliteConnection, g: Id, ch: Id, minutes: u64) -> Result<()> {
    let minutes = minutes.min(24 * 60) as i64;
    sqlx::query("INSERT INTO insight_daily(guild_id,day,voice_minutes) VALUES(?1,?2,?3) ON CONFLICT(guild_id,day) DO UPDATE SET voice_minutes=voice_minutes+?3")
        .bind(id(g)?)
        .bind(today())
        .bind(minutes)
        .execute(&mut *c)
        .await?;
    sqlx::query("INSERT INTO insight_channels(guild_id,day,channel_id,voice_minutes) VALUES(?1,?2,?3,?4) ON CONFLICT(guild_id,day,channel_id) DO UPDATE SET voice_minutes=voice_minutes+?4")
        .bind(id(g)?)
        .bind(today())
        .bind(id(ch)?)
        .bind(minutes)
        .execute(&mut *c)
        .await?;
    Ok(())
}
pub(super) async fn step(c: &mut SqliteConnection, g: Id, step: &str) -> Result<()> {
    sqlx::query("INSERT INTO insight_onboarding(guild_id,day,step,reached) VALUES(?,?,?,1) ON CONFLICT(guild_id,day,step) DO UPDATE SET reached=reached+1")
        .bind(id(g)?)
        .bind(today())
        .bind(step)
        .execute(&mut *c)
        .await?;
    Ok(())
}
/// Drop rows older than the window.
pub(super) async fn trim(c: &mut SqliteConnection, g: Id, today: u32) -> Result<()> {
    let cutoff = i64::from(today.saturating_sub(INSIGHTS_DAYS));
    for table in [
        "insight_daily",
        "insight_channels",
        "insight_sources",
        "insight_onboarding",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE guild_id=? AND day<?"
        )))
        .bind(id(g)?)
        .bind(cutoff)
        .execute(&mut *c)
        .await?;
    }
    sqlx::query("DELETE FROM insight_joins WHERE guild_id=? AND joined_at<?")
        .bind(id(g)?)
        .bind(cutoff * DAY_MS as i64)
        .execute(&mut *c)
        .await?;
    Ok(())
}

/// The insights page for the last `days` days (1 to `INSIGHTS_DAYS`).
pub(super) async fn report(c: &mut SqliteConnection, g: Id, days: u32) -> Result<Insights> {
    let days = days.clamp(1, INSIGHTS_DAYS);
    let today = day_of(now());
    let from = today.saturating_sub(days - 1);
    let mut by_day: HashMap<u32, InsightDay> = HashMap::new();
    for r in sqlx::query("SELECT day,joins,leaves,messages,voice_minutes FROM insight_daily WHERE guild_id=? AND day>=? ORDER BY day")
        .bind(id(g)?)
        .bind(i64::from(from))
        .fetch_all(&mut *c)
        .await?
    {
        let day = r.get::<i64, _>(0) as u32;
        by_day.insert(
            day,
            InsightDay {
                day,
                joins: r.get::<i64, _>(1) as u32,
                leaves: r.get::<i64, _>(2) as u32,
                messages: r.get::<i64, _>(3) as u32,
                voice_minutes: r.get::<i64, _>(4) as u32,
            },
        );
    }
    let series = (from..=today)
        .map(|day| {
            by_day.remove(&day).unwrap_or(InsightDay {
                day,
                ..Default::default()
            })
        })
        .collect();
    let sources = sqlx::query("SELECT source,code,SUM(joins) AS n FROM insight_sources WHERE guild_id=? AND day>=? GROUP BY source,code ORDER BY n DESC LIMIT 20")
        .bind(id(g)?)
        .bind(i64::from(from))
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| InsightSource {
            source: en(r.get("source")).unwrap_or(JoinSource::Other),
            code: r.get("code"),
            joins: r.get::<i64, _>("n") as u32,
        })
        .collect();
    let channels = sqlx::query("SELECT i.channel_id,ch.name,SUM(i.messages) AS m,SUM(i.voice_minutes) AS v FROM insight_channels i JOIN channels ch ON ch.id=i.channel_id WHERE i.guild_id=? AND i.day>=? GROUP BY i.channel_id ORDER BY m+v DESC LIMIT 20")
        .bind(id(g)?)
        .bind(i64::from(from))
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| InsightChannel {
            channel_id: Id(r.get::<i64, _>(0) as u64),
            name: r.get::<Option<String>, _>(1).unwrap_or_default(),
            messages: r.get::<i64, _>(2) as u32,
            voice_minutes: r.get::<i64, _>(3) as u32,
        })
        .collect();
    // A cohort counts as retained after N days when it is still a member then
    // and had talked by then; it is reported once every member is N days in.
    let t = now() as i64;
    let retention = sqlx::query("SELECT joined_at/86400000 AS d,COUNT(*) AS n,SUM(CASE WHEN (left_at IS NULL OR left_at>joined_at+86400000) AND first_message_at<=joined_at+86400000 THEN 1 ELSE 0 END) AS r1,SUM(CASE WHEN (left_at IS NULL OR left_at>joined_at+604800000) AND first_message_at<=joined_at+604800000 THEN 1 ELSE 0 END) AS r7 FROM insight_joins WHERE guild_id=? AND joined_at>=? GROUP BY d ORDER BY d")
        .bind(id(g)?)
        .bind(i64::from(from) * DAY_MS as i64)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| {
            let d = r.get::<i64, _>("d");
            let mature = |n: i64| (d + 1 + n) * DAY_MS as i64 <= t;
            RetentionCohort {
                day: d as u32,
                joined: r.get::<i64, _>("n") as u32,
                retained_1d: mature(1).then(|| r.get::<i64, _>("r1") as u32),
                retained_7d: mature(7).then(|| r.get::<i64, _>("r7") as u32),
            }
        })
        .collect();
    let reached: HashMap<String, u32> = sqlx::query("SELECT step,SUM(reached) FROM insight_onboarding WHERE guild_id=? AND day>=? GROUP BY step")
        .bind(id(g)?)
        .bind(i64::from(from))
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| (r.get::<String, _>(0), r.get::<i64, _>(1) as u32))
        .collect();
    let o = onboarding_settings(c, g).await?;
    let mut funnel = vec![];
    let mut add = |key: String, label: String| {
        if let Some(n) = reached.get(&key) {
            funnel.push(FunnelStep { label, reached: *n });
        }
    };
    add("joined".into(), "Joined".into());
    let mut questions: Vec<&OnboardingQuestion> = o.questions.iter().collect();
    questions.sort_by_key(|q| !q.pre_join);
    for q in questions {
        add(format!("q:{}", q.id), q.title.clone());
    }
    add("rules".into(), "Accepted the rules".into());
    add("done".into(), "Finished onboarding".into());
    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id=?")
            .bind(id(g)?)
            .fetch_one(&mut *c)
            .await?;
    Ok(Insights {
        guild_id: g,
        member_count: member_count as u32,
        today,
        days: series,
        sources,
        channels,
        retention,
        funnel,
    })
}
