//! This instance's server directory. Owners list their servers; anyone signed
//! in can browse, search and preview them, and join following each server's
//! admission settings. The instance administrator features, hides or delists
//! servers and can turn the directory off (`sticktalk-server admin directory`).
//!
//! Counts are cached: the list is rebuilt at most once a minute, and only when
//! someone opens the directory, or when a listing changed.
use super::*;
use crate::moderation::audit;

const TTL_MS: u64 = 60_000;
/// Listings this instance accepts.
const MAX_LISTINGS: i64 = 1_000;

#[derive(Default)]
pub(crate) struct Cache {
    at: u64,
    stamp: (i64, i64, bool),
    entries: Vec<(DirectoryEntry, u32)>,
}

pub(crate) async fn enabled(c: &mut SqliteConnection) -> Result<bool> {
    let v: Option<i64> = sqlx::query_scalar("SELECT enabled FROM directory_settings WHERE id=1")
        .fetch_optional(c)
        .await?;
    Ok(v.is_none_or(|v| v != 0))
}

pub(super) async fn requirements(
    c: &mut SqliteConnection,
    g: Id,
) -> Result<Vec<ListingRequirement>> {
    let a = admission_settings(c, g).await?;
    let o = onboarding_settings(c, g).await?;
    let level = crate::moderation::safety(c, g).await?.verification_level;
    let restricted: Vec<i64> = sqlx::query_scalar("SELECT r.channel_id FROM channel_age_restricted r JOIN channels ch ON ch.id=r.channel_id WHERE ch.guild_id=?")
        .bind(id(g)?)
        .fetch_all(&mut *c)
        .await?;
    let default_restricted = restricted
        .iter()
        .any(|ch| !o.opt_in_channel_ids.contains(&Id(*ch as u64)));
    Ok(vec![
        ListingRequirement {
            key: "admission".into(),
            label: "Rules screening or applications are turned on".into(),
            met: (a.screening && !a.rules.is_empty()) || a.applications,
        },
        ListingRequirement {
            key: "verification".into(),
            label: "A verification level is set in Safety Setup".into(),
            met: level != VerificationLevel::None,
        },
        ListingRequirement {
            key: "age".into(),
            label: "No age-restricted channel is a default channel".into(),
            met: !default_restricted,
        },
    ])
}
async fn qualifies(c: &mut SqliteConnection, g: Id) -> Result<bool> {
    Ok(requirements(c, g).await?.iter().all(|r| r.met))
}
async fn listing(c: &mut SqliteConnection, g: Id) -> Result<Option<Listing>> {
    let r = sqlx::query("SELECT * FROM directory_listings WHERE guild_id=?")
        .bind(id(g)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(r.map(|r| Listing {
        guild_id: g,
        draft: ListingDraft {
            category: DirectoryCategory::parse(&r.get::<String, _>("category")).unwrap_or_default(),
            description: r.get("description"),
            tags: parse(r.get("tags")),
            language: r.get("language"),
            featured_emoji: parse(r.get("featured_emoji")),
        },
        featured: r.get::<i64, _>("featured") != 0,
        hidden: r.get::<i64, _>("hidden") != 0,
        delisted: r.get::<i64, _>("delisted") != 0,
        listed_at: r.get::<i64, _>("listed_at") as u64,
    }))
}
/// A listed, visible, qualifying server in an enabled directory.
pub(super) async fn joinable(c: &mut SqliteConnection, g: Id) -> Result<()> {
    let visible = enabled(c).await?
        && listing(c, g)
            .await?
            .is_some_and(|l| !l.hidden && !l.delisted);
    require(visible && qualifies(c, g).await?, ErrorCode::NotFound)
}
/// For the file route: a listed server's icon may be shown to anyone signed in.
pub(crate) async fn listed_icon(c: &mut SqliteConnection, g: Id) -> Result<bool> {
    Ok(joinable(c, g).await.is_ok())
}
/// For the emoji route: a listed server's featured emoji may be shown to anyone signed in.
pub(crate) async fn featured_emoji(c: &mut SqliteConnection, g: Id, e: Id) -> Result<bool> {
    let shown = listing(c, g)
        .await?
        .is_some_and(|l| l.draft.featured_emoji.contains(&e));
    Ok(shown && joinable(c, g).await.is_ok())
}

fn draft(mut d: ListingDraft) -> Result<ListingDraft> {
    d.description = label(&d.description, MAX_DIRECTORY_DESCRIPTION_CHARS)?;
    require(d.tags.len() <= MAX_DIRECTORY_TAGS, ErrorCode::LimitExceeded)?;
    let mut tags = vec![];
    for t in &d.tags {
        let t = t.trim().to_lowercase();
        require(
            !t.is_empty()
                && t.chars().count() <= MAX_DIRECTORY_TAG_CHARS
                && t.chars()
                    .all(|c| c.is_alphanumeric() || c == ' ' || c == '-'),
            ErrorCode::InvalidRequest,
        )?;
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    d.tags = tags;
    d.language = d.language.trim().to_owned();
    require(
        (2..=10).contains(&d.language.len())
            && d.language
                .chars()
                .all(|c| c.is_ascii_alphabetic() || c == '-'),
        ErrorCode::InvalidRequest,
    )?;
    let unique: HashSet<&Id> = d.featured_emoji.iter().collect();
    require(
        d.featured_emoji.len() <= MAX_FEATURED_EMOJI && unique.len() == d.featured_emoji.len(),
        ErrorCode::InvalidRequest,
    )?;
    Ok(d)
}
pub(super) async fn set(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    d: Option<ListingDraft>,
) -> Result<CommunityResult> {
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    let old = listing(c, g).await?;
    if old.as_ref().is_some_and(|l| l.delisted) {
        return Err(denied(
            "directory",
            "The administrator of this StickTalk instance removed this server from the directory.",
        ));
    }
    let entry = |after: &str| {
        audit::Entry::new(g, AuditAction::ListingUpdate, AuditTarget::Guild, Some(g)).change(
            "listed",
            Some(if old.is_some() { "Yes" } else { "No" }.into()),
            Some(after.into()),
        )
    };
    let Some(d) = d else {
        sqlx::query("DELETE FROM directory_listings WHERE guild_id=?")
            .bind(id(g)?)
            .execute(&mut *c)
            .await?;
        let title = guild_name(c, g).await?;
        audit::write(c, u, entry("No").named(title)).await?;
        return Ok(CommunityResult::Listing(None));
    };
    if !enabled(c).await? {
        return Err(denied(
            "directory",
            "The server directory is turned off on this StickTalk instance.",
        ));
    }
    let d = draft(d)?;
    for e in &d.featured_emoji {
        let found: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM guild_emoji WHERE id=? AND guild_id=?")
                .bind(id(*e)?)
                .bind(id(g)?)
                .fetch_one(&mut *c)
                .await?;
        require(found == 1, ErrorCode::InvalidRequest)?;
    }
    if let Some(missing) = requirements(c, g).await?.into_iter().find(|r| !r.met) {
        return Err(invalid(
            "requirements",
            format!("Before listing this server: {}.", missing.label),
        ));
    }
    if old.is_none() {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM directory_listings")
            .fetch_one(&mut *c)
            .await?;
        require(n < MAX_LISTINGS, ErrorCode::LimitExceeded)?;
    }
    sqlx::query("INSERT INTO directory_listings(guild_id,category,description,tags,language,featured_emoji,listed_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?7) ON CONFLICT(guild_id) DO UPDATE SET category=?2,description=?3,tags=?4,language=?5,featured_emoji=?6,listed=1,updated_at=?7")
        .bind(id(g)?)
        .bind(d.category.name())
        .bind(&d.description)
        .bind(json(&d.tags)?)
        .bind(&d.language)
        .bind(json(&d.featured_emoji)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    let e = entry("Yes")
        .named(guild_name(c, g).await?)
        .change(
            "category",
            old.as_ref().map(|l| l.draft.category.label().to_owned()),
            Some(d.category.label().into()),
        )
        .change(
            "description",
            old.as_ref().map(|l| l.draft.description.clone()),
            Some(d.description.clone()),
        );
    audit::write(c, u, e).await?;
    Ok(CommunityResult::Listing(listing(c, g).await?.map(Box::new)))
}
pub(super) async fn admin(c: &mut SqliteConnection, u: Id, g: Id) -> Result<ListingAdmin> {
    guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
    Ok(ListingAdmin {
        directory_enabled: enabled(c).await?,
        listing: listing(c, g).await?,
        requirements: requirements(c, g).await?,
    })
}

/// Rebuild the cached entries when listings changed or the counts are a minute old.
async fn refresh(core: &mut Core, c: &mut SqliteConnection) -> Result<()> {
    let r = sqlx::query("SELECT COUNT(*),COALESCE(MAX(updated_at),0) FROM directory_listings")
        .fetch_one(&mut *c)
        .await?;
    let stamp = (r.get::<i64, _>(0), r.get::<i64, _>(1), enabled(c).await?);
    let cache = &core.community.directory;
    if cache.stamp == stamp && now().saturating_sub(cache.at) < TTL_MS && cache.at > 0 {
        return Ok(());
    }
    let online: HashSet<Id> = core
        .streams
        .values()
        .filter(|s| s.connection.is_some())
        .map(|s| s.user)
        .collect();
    let rows = sqlx::query("SELECT l.guild_id,g.name,g.icon_id,COALESCE(p.banner_color,0) AS banner FROM directory_listings l JOIN guilds g ON g.id=l.guild_id LEFT JOIN guild_profiles p ON p.guild_id=g.id WHERE l.listed=1 AND l.hidden=0 AND l.delisted=0 ORDER BY l.guild_id LIMIT ?")
        .bind(MAX_LISTINGS)
        .fetch_all(&mut *c)
        .await?;
    let week = i64::from(day_of(now())) - 6;
    let mut entries = vec![];
    for r in rows {
        let g = Id(r.get::<i64, _>(0) as u64);
        if !stamp.2 || !qualifies(c, g).await? {
            continue;
        }
        let Some(l) = listing(c, g).await? else {
            continue;
        };
        let members: Vec<i64> = sqlx::query_scalar("SELECT m.user_id FROM guild_members m JOIN users u ON u.id=m.user_id WHERE m.guild_id=? AND u.desired_presence NOT IN('invisible','offline')")
            .bind(id(g)?)
            .fetch_all(&mut *c)
            .await?;
        let member_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id=?")
                .bind(id(g)?)
                .fetch_one(&mut *c)
                .await?;
        let active: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(messages),0) FROM insight_daily WHERE guild_id=? AND day>=?",
        )
        .bind(id(g)?)
        .bind(week)
        .fetch_one(&mut *c)
        .await?;
        let mut featured_emoji = vec![];
        for e in &l.draft.featured_emoji {
            if let Some(row) =
                sqlx::query("SELECT name,animated FROM guild_emoji WHERE id=? AND guild_id=?")
                    .bind(id(*e)?)
                    .bind(id(g)?)
                    .fetch_optional(&mut *c)
                    .await?
            {
                featured_emoji.push(FeaturedEmoji {
                    id: *e,
                    name: row.get("name"),
                    animated: row.get("animated"),
                });
            }
        }
        entries.push((
            DirectoryEntry {
                guild_id: g,
                name: r.get(1),
                icon_id: r.get::<Option<i64>, _>(2).map(|v| Id(v as u64)),
                banner_color: r.get::<i64, _>(3).clamp(0, 9) as u8,
                description: l.draft.description,
                category: l.draft.category,
                tags: l.draft.tags,
                language: l.draft.language,
                member_count: member_count as u32,
                online_count: members
                    .iter()
                    .filter(|v| online.contains(&Id(**v as u64)))
                    .count() as u32,
                featured_emoji,
                featured: l.featured,
                joined: false,
            },
            active as u32,
        ));
    }
    core.community.directory = Cache {
        at: now(),
        stamp,
        entries,
    };
    Ok(())
}
async fn mark_joined(
    c: &mut SqliteConnection,
    u: Id,
    list: impl Iterator<Item = DirectoryEntry>,
) -> Result<Vec<DirectoryEntry>> {
    let joined: HashSet<i64> =
        sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id=?")
            .bind(id(u)?)
            .fetch_all(&mut *c)
            .await?
            .into_iter()
            .collect();
    Ok(list
        .map(|mut e| {
            e.joined = joined.contains(&(e.guild_id.0 as i64));
            e
        })
        .collect())
}
pub(super) async fn page(
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    q: &str,
    category: Option<DirectoryCategory>,
) -> Result<DirectoryPage> {
    if !enabled(c).await? {
        return Ok(DirectoryPage::default());
    }
    refresh(core, c).await?;
    let all = &core.community.directory.entries;
    let mut featured: Vec<&(DirectoryEntry, u32)> =
        all.iter().filter(|(e, _)| e.featured).collect();
    featured.sort_by_key(|(e, _)| std::cmp::Reverse(e.member_count));
    let mut active: Vec<&(DirectoryEntry, u32)> = all.iter().collect();
    active.sort_by_key(|(e, n)| (std::cmp::Reverse(*n), std::cmp::Reverse(e.member_count)));
    let mut counts: HashMap<DirectoryCategory, u32> = HashMap::new();
    for (e, _) in all {
        *counts.entry(e.category).or_default() += 1;
    }
    let q = q.trim().to_lowercase();
    let mut results: Vec<&(DirectoryEntry, u32)> = if q.is_empty() && category.is_none() {
        vec![]
    } else {
        all.iter()
            .filter(|(e, _)| category.is_none_or(|c| e.category == c))
            .filter(|(e, _)| {
                q.split_whitespace().all(|w| {
                    e.name.to_lowercase().contains(w) || e.tags.iter().any(|t| t.contains(w))
                })
            })
            .collect()
    };
    results.sort_by_key(|(e, _)| std::cmp::Reverse(e.member_count));
    let take = |v: Vec<&(DirectoryEntry, u32)>| {
        v.into_iter()
            .take(DIRECTORY_PAGE)
            .map(|(e, _)| e.clone())
            .collect::<Vec<_>>()
    };
    let (featured, active, results) = (take(featured), take(active), take(results));
    Ok(DirectoryPage {
        enabled: true,
        featured: mark_joined(c, u, featured.into_iter()).await?,
        active: mark_joined(c, u, active.into_iter()).await?,
        results: mark_joined(c, u, results.into_iter()).await?,
        categories: DirectoryCategory::ALL
            .iter()
            .map(|c| (*c, counts.get(c).copied().unwrap_or(0)))
            .collect(),
    })
}
pub(super) async fn preview(
    core: &mut Core,
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
) -> Result<DirectoryPreview> {
    joinable(c, g).await?;
    refresh(core, c).await?;
    let entry = core
        .community
        .directory
        .entries
        .iter()
        .find(|(e, _)| e.guild_id == g)
        .map(|(e, _)| e.clone())
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let entry = mark_joined(c, u, std::iter::once(entry))
        .await?
        .pop()
        .ok_or_else(|| Error::new(ErrorCode::Internal))?;
    let o = onboarding_settings(c, g).await?;
    let welcome = (o.enabled
        && (!o.welcome.description.is_empty() || !o.welcome.highlights.is_empty()))
    .then_some(o.welcome);
    let mut channel_names = vec![];
    for h in welcome.iter().flat_map(|w| w.highlights.iter()) {
        if let Ok(ch) = channel(c, h.channel_id).await {
            channel_names.push((h.channel_id, ch.summary.name.unwrap_or_default()));
        }
    }
    Ok(DirectoryPreview {
        entry,
        gate: admission::join_gate(c, u, g).await?,
        welcome,
        channel_names,
    })
}

/// Join a listed server straight from the directory.
pub(super) async fn join(
    c: &mut SqliteConnection,
    u: Id,
    g: Id,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    joinable(c, g).await?;
    if role(c, u, g).await.is_ok() {
        return Ok(CommandResult::Guild(detail(c, u, g).await?));
    }
    let banned: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_bans WHERE guild_id=? AND user_id=?")
            .bind(id(g)?)
            .bind(id(u)?)
            .fetch_one(&mut *c)
            .await?;
    require(banned == 0, ErrorCode::Banned)?;
    gate(c, g).await?;
    guild_quota(c, u).await?;
    require(
        guild_users(c, g).await?.len() < MAX_GUILD_MEMBERS,
        ErrorCode::LimitExceeded,
    )?;
    sqlx::query("INSERT INTO guild_members VALUES(?,?,'member',?)")
        .bind(id(g)?)
        .bind(id(u)?)
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    admit(c, g, u, JoinSource::Discovery, None, false).await?;
    crate::commands::joined(c, out, g, u).await?;
    Ok(CommandResult::Guild(detail(c, u, g).await?))
}

/// `sticktalk-server admin directory ...`: run against the database directly.
pub(crate) async fn admin_cli(
    c: &mut SqliteConnection,
    command: &crate::admin::DirectoryAdmin,
) -> Result<()> {
    use crate::admin::DirectoryAdmin::*;
    let flag = |name: &'static str, on: bool| (name, on);
    let (column, value, guild) = match command {
        List => {
            println!(
                "Directory: {}",
                if enabled(c).await? { "on" } else { "off" }
            );
            for r in sqlx::query("SELECT l.guild_id,g.name,l.category,l.featured,l.hidden,l.delisted FROM directory_listings l JOIN guilds g ON g.id=l.guild_id ORDER BY l.guild_id LIMIT 1000")
                .fetch_all(&mut *c)
                .await?
            {
                let mut marks = vec![];
                for (i, mark) in [(3, "featured"), (4, "hidden"), (5, "delisted")] {
                    if r.get::<i64, _>(i) != 0 {
                        marks.push(mark);
                    }
                }
                println!(
                    "{}\t{}\t{}\t{}",
                    r.get::<i64, _>(0),
                    r.get::<String, _>(1),
                    r.get::<String, _>(2),
                    marks.join(",")
                );
            }
            return Ok(());
        }
        Enable | Disable => {
            sqlx::query("INSERT INTO directory_settings(id,enabled) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled")
                .bind(matches!(command, Enable))
                .execute(&mut *c)
                .await?;
            println!(
                "Directory turned {}.",
                if matches!(command, Enable) {
                    "on"
                } else {
                    "off"
                }
            );
            return Ok(());
        }
        Feature { guild_id } => (flag("featured", true), 1, *guild_id),
        Unfeature { guild_id } => (flag("featured", false), 0, *guild_id),
        Hide { guild_id } => (flag("hidden", true), 1, *guild_id),
        Unhide { guild_id } => (flag("hidden", false), 0, *guild_id),
        Delist { guild_id } => (flag("delisted", true), 1, *guild_id),
        Relist { guild_id } => (flag("delisted", false), 0, *guild_id),
    };
    // `column.0` is one of three fixed names above.
    let r = sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE directory_listings SET {}=?,updated_at=? WHERE guild_id=?",
        column.0
    )))
    .bind(value)
    .bind(now() as i64)
    .bind(id(Id(guild))?)
    .execute(&mut *c)
    .await?;
    require(r.rows_affected() == 1, ErrorCode::NotFound)?;
    println!(
        "Server {guild}: {} {}.",
        column.0,
        if column.1 { "set" } else { "cleared" }
    );
    Ok(())
}
