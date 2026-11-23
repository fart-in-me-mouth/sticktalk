//! Profile badges. Built-in badges are computed when a card is read (no
//! timers): server owner of a big server, early member, and the account-age
//! badge that evolves at 1, 2, 3 and 5 years. The instance administrator
//! awards the rest, and creates custom badges, with the offline admin CLI:
//! `sticktalk-server admin badge ...`.
use super::*;
use axum::response::{IntoResponse, Response};
use clap::Subcommand;

const YEAR_MS: u64 = 31_557_600_000;

fn builtin(id: &str, icon: &str, name: &str, description: &str) -> Badge {
    Badge {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        icon: Some(icon.into()),
        custom: None,
    }
}
async fn early_cutoff(c: &mut SqliteConnection) -> Result<u64> {
    let v: Option<String> =
        sqlx::query_scalar("SELECT value FROM profile_settings WHERE key='early_member_cutoff'")
            .fetch_optional(&mut *c)
            .await?;
    Ok(v.and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_EARLY_MEMBER_CUTOFF))
}
/// Every badge `u` has earned, in the default order.
pub(crate) async fn earned(c: &mut SqliteConnection, u: Id) -> Result<Vec<Badge>> {
    let created: i64 = sqlx::query_scalar("SELECT created_at FROM users WHERE id=?")
        .bind(id(u)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    let created = created.max(0) as u64;
    let awards: Vec<String> = sqlx::query_scalar(
        "SELECT badge FROM badge_awards WHERE user_id=? ORDER BY awarded_at,badge",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?;
    let has = |b: &str| awards.iter().any(|a| a == b);
    let mut out = vec![];
    if has("instance_admin") {
        out.push(builtin(
            "instance_admin",
            "instance_admin",
            "Instance Administrator",
            "Runs this StickTalk server",
        ));
    }
    let big: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM guilds g WHERE g.owner_id=? AND (SELECT COUNT(*) FROM guild_members m WHERE m.guild_id=g.id)>=?)")
        .bind(id(u)?)
        .bind(i64::from(SERVER_OWNER_BADGE_MEMBERS))
        .fetch_one(&mut *c)
        .await?;
    if big {
        out.push(builtin(
            "server_owner",
            "server_owner",
            "Server Owner",
            &format!("Owns a server with {SERVER_OWNER_BADGE_MEMBERS} or more members"),
        ));
    }
    if created < early_cutoff(c).await? {
        out.push(builtin(
            "early_member",
            "early_member",
            "Early Member",
            "Joined StickTalk early",
        ));
    }
    let years = now().saturating_sub(created) / YEAR_MS;
    if let Some(tier) = MEMBER_YEAR_TIERS
        .iter()
        .rev()
        .find(|t| years >= u64::from(**t))
    {
        out.push(builtin(
            "member_years",
            &format!("years_{tier}"),
            &format!(
                "Member for {tier} Year{}",
                if *tier == 1 { "" } else { "s" }
            ),
            &format!(
                "Has been on StickTalk for {tier} year{} or more",
                if *tier == 1 { "" } else { "s" }
            ),
        ));
    }
    if has("bug_reporter") {
        out.push(builtin(
            "bug_reporter",
            "bug_reporter",
            "Bug Reporter",
            "Reported a bug that got fixed",
        ));
    }
    if has("contributor") {
        out.push(builtin(
            "contributor",
            "contributor",
            "Contributor",
            "Helped build StickTalk",
        ));
    }
    for r in sqlx::query("SELECT b.id,b.name,b.description FROM badges b JOIN badge_awards a ON a.badge='c'||b.id WHERE a.user_id=? ORDER BY a.awarded_at,b.id")
        .bind(id(u)?)
        .fetch_all(&mut *c)
        .await?
    {
        let b = r.get::<i64, _>("id");
        out.push(Badge {
            id: format!("c{b}"),
            name: r.get("name"),
            description: r.get("description"),
            icon: None,
            custom: Some(Id(b as u64)),
        });
    }
    Ok(out)
}
/// The badges a card shows: the chosen ones still earned, in their order, or
/// the first six earned when nothing was chosen.
pub(crate) fn shown(earned: &[Badge], chosen: Option<&[String]>) -> Vec<Badge> {
    match chosen {
        Some(ids) => ids
            .iter()
            .filter_map(|i| earned.iter().find(|b| b.id == *i).cloned())
            .take(MAX_SHOWN_BADGES)
            .collect(),
        None => earned.iter().take(MAX_SHOWN_BADGES).cloned().collect(),
    }
}
pub(crate) async fn icon_endpoint(
    State(app): State<App>,
    Path(b): Path<String>,
    h: HeaderMap,
) -> Result<Response> {
    let (_core, mut c, _) = rest(&app, &h).await?;
    let b = http::url_id(&b)?;
    let icon: Vec<u8> = sqlx::query_scalar("SELECT icon FROM badges WHERE id=?")
        .bind(id(b)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok((
        [
            ("content-type", "image/png"),
            ("cache-control", "private, max-age=86400, immutable"),
            ("x-content-type-options", "nosniff"),
        ],
        icon,
    )
        .into_response())
}

/// `sticktalk-server admin badge ...`: the instance administrator's badges.
#[derive(Subcommand, Clone, Debug)]
pub enum BadgeCommand {
    /// List custom badges and who has the awarded ones.
    List,
    /// Create a custom badge with an icon (PNG, JPEG, WebP or GIF; fitted to 64x64).
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        icon: std::path::PathBuf,
    },
    /// Delete a custom badge; it is taken from everyone who had it.
    Delete { id: u64 },
    /// Award a badge: instance_admin, bug_reporter, contributor, or c<ID> for a custom one.
    Award { badge: String, username: String },
    /// Take an awarded badge back.
    Revoke { badge: String, username: String },
    /// Accounts created before this UTC date (YYYY-MM-DD) earn Early Member.
    EarlyCutoff { date: String },
}
/// Awardable IDs: the administrator's built-in badges and existing custom ones.
async fn awardable(c: &mut SqliteConnection, badge: &str) -> Result<()> {
    if BUILTIN_BADGES
        .iter()
        .any(|(b, _, awarded)| *awarded && *b == badge)
    {
        return Ok(());
    }
    let n = badge
        .strip_prefix('c')
        .and_then(|n| n.parse::<i64>().ok())
        .ok_or_else(invalid)?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM badges WHERE id=?)")
        .bind(n)
        .fetch_one(&mut *c)
        .await?;
    require(exists, ErrorCode::NotFound)
}
/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
pub(crate) fn parse_date(s: &str) -> Result<u64> {
    let parts: Vec<i64> = s
        .split('-')
        .map(|p| p.parse().map_err(|_| invalid()))
        .collect::<Result<_>>()?;
    require(
        parts.len() == 3
            && (1970..=9999).contains(&parts[0])
            && (1..=12).contains(&parts[1])
            && (1..=31).contains(&parts[2]),
        ErrorCode::InvalidRequest,
    )?;
    Ok(days(parts[0], parts[1], parts[2]) as u64 * 86_400_000)
}
async fn user_id(c: &mut SqliteConnection, name: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE username=?")
        .bind(username(name)?)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}
pub(crate) async fn run(c: &mut SqliteConnection, command: &BadgeCommand) -> Result<()> {
    match command {
        BadgeCommand::List => {
            for r in sqlx::query("SELECT id,name,description FROM badges ORDER BY id")
                .fetch_all(&mut *c)
                .await?
            {
                println!(
                    "c{}\t{}\t{}",
                    r.get::<i64, _>("id"),
                    r.get::<String, _>("name"),
                    r.get::<String, _>("description")
                );
            }
            for r in sqlx::query("SELECT a.badge,u.username FROM badge_awards a JOIN users u ON u.id=a.user_id ORDER BY a.badge,u.username")
                .fetch_all(&mut *c)
                .await?
            {
                println!(
                    "awarded\t{}\t{}",
                    r.get::<String, _>("badge"),
                    r.get::<String, _>("username")
                );
            }
        }
        BadgeCommand::Create {
            name,
            description,
            icon,
        } => {
            let name = crate::db::name(name, MAX_BADGE_NAME_CHARS * 4)?;
            let description = text(description, MAX_BADGE_DESCRIPTION_CHARS)?;
            require(
                name.chars().count() <= MAX_BADGE_NAME_CHARS,
                ErrorCode::InvalidRequest,
            )?;
            let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM badges")
                .fetch_one(&mut *c)
                .await?;
            require(n < MAX_CUSTOM_BADGES as i64, ErrorCode::LimitExceeded)?;
            let bytes = std::fs::read(icon)?;
            let png = media::badge_icon(&bytes)?;
            let r =
                sqlx::query("INSERT INTO badges(name,description,icon,created_at) VALUES(?,?,?,?)")
                    .bind(name)
                    .bind(description)
                    .bind(png)
                    .bind(now() as i64)
                    .execute(&mut *c)
                    .await?;
            println!("Created badge c{}", r.last_insert_rowid());
        }
        BadgeCommand::Delete { id: b } => {
            let n = sqlx::query("DELETE FROM badges WHERE id=?")
                .bind(*b as i64)
                .execute(&mut *c)
                .await?
                .rows_affected();
            require(n == 1, ErrorCode::NotFound)?;
            println!("Deleted badge c{b}");
        }
        BadgeCommand::Award { badge, username } => {
            awardable(c, badge).await?;
            let u = user_id(c, username).await?;
            sqlx::query("INSERT OR IGNORE INTO badge_awards VALUES(?,?,?)")
                .bind(badge)
                .bind(u)
                .bind(now() as i64)
                .execute(&mut *c)
                .await?;
            println!("Awarded {badge} to {username}");
        }
        BadgeCommand::Revoke { badge, username } => {
            let u = user_id(c, username).await?;
            let n = sqlx::query("DELETE FROM badge_awards WHERE badge=? AND user_id=?")
                .bind(badge)
                .bind(u)
                .execute(&mut *c)
                .await?
                .rows_affected();
            require(n == 1, ErrorCode::NotFound)?;
            println!("Revoked {badge} from {username}");
        }
        BadgeCommand::EarlyCutoff { date } => {
            let t = parse_date(date)?;
            sqlx::query("INSERT INTO profile_settings VALUES('early_member_cutoff',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
                .bind(t.to_string())
                .execute(&mut *c)
                .await?;
            println!("Early Member: accounts created before {date}");
        }
    }
    Ok(())
}
