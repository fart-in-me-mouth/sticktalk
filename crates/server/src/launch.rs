use crate::{
    db::*,
    error::{require, Error, Result},
    permissions as acl,
    state::{now, Core},
};
use sqlx::{Row, SqliteConnection};
use sticktalk_proto::*;
type Deliveries = Vec<(Vec<Id>, Event)>;
fn bounded(s: &str, max: usize) -> Result<()> {
    require(
        s.chars().count() <= max && !s.chars().any(|c| c.is_control() && c != '\n'),
        ErrorCode::InvalidRequest,
    )
}
async fn role(c: &mut SqliteConnection, g: Id, r: Id) -> Result<Role> {
    acl::roles(c, g)
        .await?
        .into_iter()
        .find(|v| v.id == r)
        .ok_or_else(|| Error::new(ErrorCode::NotFound))
}
async fn roles_event(c: &mut SqliteConnection, out: &mut Deliveries, g: Id) -> Result<()> {
    let users = guild_users(c, g).await?;
    out.push((
        users.clone(),
        Event::RolesUpdated {
            guild_id: g,
            roles: acl::roles(c, g).await?,
        },
    ));
    // Role definitions invalidate cached computed permissions; member role IDs stay unchanged.
    for u in users {
        out.push((vec![u], Event::GuildUpsert(guild(c, u, g).await?)));
    }
    Ok(())
}
pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: LaunchCommand,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    use LaunchCommand::*;
    match cmd {
        SetServerFolders(folders) => crate::folders::set(c, u, folders, out).await,
        SetSlowmode {
            channel_id,
            seconds,
        } => crate::slowmode::set(c, u, channel_id, seconds, out).await,
        RenameEmoji {
            guild_id,
            emoji_id,
            name,
        } => crate::custom_emoji::mutate(c, u, guild_id, emoji_id, Some(name), out).await,
        DeleteEmoji { guild_id, emoji_id } => {
            crate::custom_emoji::mutate(c, u, guild_id, emoji_id, None, out).await
        }
        CreateRole {
            guild_id: g,
            name: s,
        } => {
            let p = acl::evaluate(c, u, g, None).await?;
            require(p.contains(Permissions::MANAGE_ROLES), ErrorCode::Forbidden)?;
            let list = acl::roles(c, g).await?;
            require(list.len() < MAX_ROLES_PER_GUILD, ErrorCode::LimitExceeded)?;
            let high = acl::highest(c, g, u).await?;
            // Insert below every existing non-everyone role; numeric shifts preserve relative hierarchy.
            require(
                p.contains(Permissions::OWNER) || high > 0,
                ErrorCode::HierarchyViolation,
            )?;
            let position = 1u16;
            let r = core.id().await?;
            require(!list.iter().any(|v| v.id == r), ErrorCode::Conflict)?;
            sqlx::query("UPDATE roles SET position=position+1 WHERE guild_id=? AND position>=?")
                .bind(id(g)?)
                .bind(i64::from(position))
                .execute(&mut *c)
                .await?;
            sqlx::query(
                "INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(?,?,?,?,0)",
            )
            .bind(id(g)?)
            .bind(id(r)?)
            .bind(name(&s, 100)?)
            .bind(i64::from(position))
            .execute(&mut *c)
            .await?;
            roles_event(c, out, g).await?;
            Ok(CommandResult::Role(role(c, g, r).await?))
        }
        UpdateRole { role: r } => {
            let old = role(c, r.guild_id, r.id).await?;
            acl::manage_role(c, r.guild_id, u, &old).await?;
            let p = acl::evaluate(c, u, r.guild_id, None).await?;
            require(
                r.position == old.position
                    && r.color <= 0xffffff
                    && (r.permissions - Permissions::all()).is_empty()
                    && !r.permissions.contains(Permissions::OWNER),
                ErrorCode::InvalidRequest,
            )?;
            require(
                p.contains(Permissions::OWNER) || p.contains(r.permissions),
                ErrorCode::Forbidden,
            )?;
            require(
                r.id != r.guild_id || (r.name == "@everyone" && !r.hoist && !r.mentionable),
                ErrorCode::InvalidRequest,
            )?;
            sqlx::query("UPDATE roles SET name=?,color=?,hoist=?,mentionable=?,permissions=? WHERE guild_id=? AND id=?").bind(name(&r.name,100)?).bind(i64::from(r.color)).bind(r.hoist).bind(r.mentionable).bind(r.permissions.bits() as i64).bind(id(r.guild_id)?).bind(id(r.id)?).execute(&mut *c).await?;
            roles_event(c, out, r.guild_id).await?;
            Ok(CommandResult::Role(role(c, r.guild_id, r.id).await?))
        }
        DeleteRole {
            guild_id: g,
            role_id: r,
        } => {
            require(r != g, ErrorCode::InvalidRequest)?;
            let old = role(c, g, r).await?;
            acl::manage_role(c, g, u, &old).await?;
            sqlx::query("DELETE FROM permission_overwrites WHERE kind='role' AND target_id=? AND channel_id IN(SELECT id FROM channels WHERE guild_id=?)").bind(id(r)?).bind(id(g)?).execute(&mut *c).await?;
            let affected = sqlx::query_scalar::<_, i64>(
                "SELECT user_id FROM member_roles WHERE guild_id=? AND role_id=?",
            )
            .bind(id(g)?)
            .bind(id(r)?)
            .fetch_all(&mut *c)
            .await?;
            if r.0 == g.0 + 1 {
                sqlx::query(
                    "UPDATE guild_members SET role='member' WHERE guild_id=? AND role='admin'",
                )
                .bind(id(g)?)
                .execute(&mut *c)
                .await?;
            }
            sqlx::query("DELETE FROM roles WHERE guild_id=? AND id=?")
                .bind(id(g)?)
                .bind(id(r)?)
                .execute(&mut *c)
                .await?;
            sqlx::query("UPDATE roles SET position=position-1 WHERE guild_id=? AND position>?")
                .bind(id(g)?)
                .bind(i64::from(old.position))
                .execute(&mut *c)
                .await?;
            for v in affected {
                out.push((
                    guild_users(c, g).await?,
                    Event::MemberUpsert(member(c, g, Id(v as u64)).await?),
                ));
            }
            roles_event(c, out, g).await?;
            Ok(CommandResult::Ack)
        }
        ReorderRoles {
            guild_id: g,
            role_ids: ids,
        } => {
            unique(&ids, MAX_ROLES_PER_GUILD)?;
            let list = acl::roles(c, g).await?;
            require(
                ids.len() == list.len() && ids.first() == Some(&g),
                ErrorCode::InvalidRequest,
            )?;
            for (position, r) in ids.iter().enumerate() {
                let old = list
                    .iter()
                    .find(|v| v.id == *r)
                    .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
                if usize::from(old.position) != position {
                    acl::manage_role(c, g, u, old).await?;
                    let p = acl::evaluate(c, u, g, None).await?;
                    require(
                        p.contains(Permissions::OWNER)
                            || position < usize::from(acl::highest(c, g, u).await?),
                        ErrorCode::HierarchyViolation,
                    )?;
                }
            }
            // Even no-op requests require Manage Roles.
            require(
                acl::evaluate(c, u, g, None)
                    .await?
                    .contains(Permissions::MANAGE_ROLES),
                ErrorCode::Forbidden,
            )?;
            for (position, r) in ids.iter().enumerate() {
                sqlx::query("UPDATE roles SET position=? WHERE guild_id=? AND id=?")
                    .bind(position as i64)
                    .bind(id(g)?)
                    .bind(id(*r)?)
                    .execute(&mut *c)
                    .await?;
            }
            roles_event(c, out, g).await?;
            Ok(CommandResult::Ack)
        }
        SetMemberRoles {
            guild_id: g,
            user_id: v,
            role_ids: ids,
        } => {
            unique(&ids, MAX_ROLES_PER_GUILD)?;
            crate::db::role(c, v, g).await?;
            require(
                acl::evaluate(c, u, g, None)
                    .await?
                    .contains(Permissions::MANAGE_ROLES),
                ErrorCode::Forbidden,
            )?;
            let old = acl::role_ids(c, g, v).await?;
            // Only changed assignments must be below actor's highest role.
            for r in old
                .iter()
                .chain(ids.iter())
                .filter(|r| **r != g && old.contains(r) != ids.contains(r))
            {
                let old = role(c, g, *r).await?;
                acl::manage_role(c, g, u, &old).await?;
            }
            for r in &ids {
                role(c, g, *r).await?;
            }
            sqlx::query("DELETE FROM member_roles WHERE guild_id=? AND user_id=?")
                .bind(id(g)?)
                .bind(id(v)?)
                .execute(&mut *c)
                .await?;
            let legacy_admin = ids.contains(&Id(g.0 + 1));
            for r in ids.into_iter().filter(|r| *r != g) {
                sqlx::query("INSERT INTO member_roles VALUES(?,?,?)")
                    .bind(id(g)?)
                    .bind(id(v)?)
                    .bind(id(r)?)
                    .execute(&mut *c)
                    .await?;
            }
            sqlx::query("UPDATE guild_members SET role=CASE WHEN role='owner' THEN 'owner' WHEN ? THEN 'admin' ELSE 'member' END WHERE guild_id=? AND user_id=?").bind(legacy_admin).bind(id(g)?).bind(id(v)?).execute(&mut *c).await?;
            out.push((
                guild_users(c, g).await?,
                Event::MemberUpsert(member(c, g, v).await?),
            ));
            out.push((vec![v], Event::GuildUpsert(guild(c, v, g).await?)));
            Ok(CommandResult::Member(member(c, g, v).await?))
        }
        SetOverwrites {
            channel_id: ch,
            overwrites: os,
        } => {
            let channel = access(c, u, ch, Permissions::MANAGE_ROLES).await?;
            let g = channel
                .summary
                .guild_id
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            require(os.len() <= MAX_CHANNEL_OVERWRITES, ErrorCode::LimitExceeded)?;
            let p = acl::evaluate(c, u, g, Some(ch)).await?;
            let previous = acl::overwrites(c, ch).await?;
            let mut seen = std::collections::HashSet::new();
            for o in &os {
                require(
                    seen.insert((enum_str(o.kind), o.target_id))
                        && !o.allow.intersects(o.deny)
                        && !(o.allow | o.deny)
                            .intersects(Permissions::OWNER | Permissions::ADMINISTRATOR)
                        && ((o.allow | o.deny) - Permissions::all()).is_empty(),
                    ErrorCode::InvalidRequest,
                )?;
                require(
                    p.contains(Permissions::OWNER) || {
                        let old = previous
                            .iter()
                            .find(|old| old.kind == o.kind && old.target_id == o.target_id);
                        let (allow, deny) = old
                            .map_or((Permissions::empty(), Permissions::empty()), |o| {
                                (o.allow, o.deny)
                            });
                        p.contains((o.allow ^ allow) | (o.deny ^ deny))
                    },
                    ErrorCode::Forbidden,
                )?;
                match o.kind {
                    OverwriteKind::Role => {
                        role(c, g, o.target_id).await?;
                    }
                    OverwriteKind::Member => {
                        crate::db::role(c, o.target_id, g).await?;
                    }
                }
            }
            // Authorize the complete old-to-new delta using PRE-mutation rights.
            // Reject atomically: omitted entries and cleared bits are mutations too.
            for old in previous {
                let new = os
                    .iter()
                    .find(|o| o.kind == old.kind && o.target_id == old.target_id);
                let (allow, deny) = new.map_or((Permissions::empty(), Permissions::empty()), |o| {
                    (o.allow, o.deny)
                });
                let changed = (old.allow ^ allow) | (old.deny ^ deny);
                require(
                    p.contains(Permissions::OWNER) || p.contains(changed),
                    ErrorCode::Forbidden,
                )?;
            }
            sqlx::query("DELETE FROM permission_overwrites WHERE channel_id=?")
                .bind(id(ch)?)
                .execute(&mut *c)
                .await?;
            for o in os {
                sqlx::query("INSERT INTO permission_overwrites VALUES(?,?,?,?,?)")
                    .bind(id(ch)?)
                    .bind(enum_str(o.kind))
                    .bind(id(o.target_id)?)
                    .bind(o.allow.bits() as i64)
                    .bind(o.deny.bits() as i64)
                    .execute(&mut *c)
                    .await?;
            }
            Ok(CommandResult::Ack)
        }
        UpdateGuildProfile {
            guild_id: g,
            description,
            banner_color,
        } => {
            guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
            bounded(&description, 120)?;
            require(banner_color < 10, ErrorCode::InvalidRequest)?;
            sqlx::query("INSERT INTO guild_profiles VALUES(?,?,?) ON CONFLICT(guild_id) DO UPDATE SET description=excluded.description,banner_color=excluded.banner_color").bind(id(g)?).bind(description).bind(i64::from(banner_color)).execute(&mut *c).await?;
            out.push((
                guild_users(c, g).await?,
                Event::GuildProfileUpdated(guild_profile(c, g).await?),
            ));
            Ok(CommandResult::Ack)
        }
        UpdateUserProfile {
            banner_color,
            pronouns,
            about_me,
        } => {
            require(banner_color <= 0xffffff, ErrorCode::InvalidRequest)?;
            bounded(&pronouns, 40)?;
            bounded(&about_me, 190)?;
            sqlx::query("INSERT INTO user_profiles VALUES(?,?,?,?) ON CONFLICT(user_id) DO UPDATE SET banner_color=excluded.banner_color,pronouns=excluded.pronouns,about_me=excluded.about_me").bind(id(u)?).bind(i64::from(banner_color)).bind(pronouns).bind(about_me).execute(&mut *c).await?;
            let mut viewers = vec![u];
            for v in sqlx::query_scalar::<_,i64>("SELECT id FROM users WHERE id IN(SELECT other_id FROM relationships WHERE user_id=?) OR id IN(SELECT b.user_id FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.user_id=?) OR id IN(SELECT b.user_id FROM dm_members a JOIN dm_members b ON a.channel_id=b.channel_id WHERE a.user_id=?)").bind(id(u)?).bind(id(u)?).bind(id(u)?).fetch_all(&mut *c).await? {viewers.push(Id(v as u64));}
            out.push((viewers, Event::ProfileUpdated(user_profile(c, u).await?)));
            Ok(CommandResult::Ack)
        }
        SetNickname {
            guild_id: g,
            user_id: v,
            nickname,
        } => {
            if u == v {
                guild_perm(c, u, g, Permissions::CHANGE_NICKNAME).await?;
            } else {
                guild_perm(c, u, g, Permissions::MANAGE_NICKNAMES).await?;
                acl::above_member(c, g, u, v).await?;
            }
            crate::db::role(c, v, g).await?;
            if let Some(n) = nickname {
                let n = name(&n, 128)?;
                bounded(&n, 32)?;
                sqlx::query("INSERT INTO nicknames VALUES(?,?,?) ON CONFLICT(guild_id,user_id) DO UPDATE SET nickname=excluded.nickname").bind(id(g)?).bind(id(v)?).bind(n).execute(&mut *c).await?;
            } else {
                sqlx::query("DELETE FROM nicknames WHERE guild_id=? AND user_id=?")
                    .bind(id(g)?)
                    .bind(id(v)?)
                    .execute(&mut *c)
                    .await?;
            }
            let m = member(c, g, v).await?;
            out.push((guild_users(c, g).await?, Event::MemberUpsert(m.clone())));
            Ok(CommandResult::Member(m))
        }
        PinMessage {
            channel_id: ch,
            message_id: m,
            pinned,
        } => {
            // Guilds require Manage Messages; DMs use their existing participant ACL.
            text(c, u, ch, Permissions::MANAGE_MESSAGES).await?;
            message(c, u, ch, m, false).await?;
            if pinned {
                let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pins WHERE channel_id=?")
                    .bind(id(ch)?)
                    .fetch_one(&mut *c)
                    .await?;
                let existing: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pins WHERE channel_id=? AND message_id=?)",
                )
                .bind(id(ch)?)
                .bind(id(m)?)
                .fetch_one(&mut *c)
                .await?;
                require(
                    existing || n < MAX_PINS_PER_CHANNEL as i64,
                    ErrorCode::LimitExceeded,
                )?;
                sqlx::query("INSERT OR IGNORE INTO pins VALUES(?,?,?,?)")
                    .bind(id(ch)?)
                    .bind(id(m)?)
                    .bind(id(u)?)
                    .bind(now() as i64)
                    .execute(&mut *c)
                    .await?;
            } else {
                sqlx::query("DELETE FROM pins WHERE channel_id=? AND message_id=?")
                    .bind(id(ch)?)
                    .bind(id(m)?)
                    .execute(&mut *c)
                    .await?;
            }
            out.push((
                audience(c, ch).await?,
                Event::PinsUpdated {
                    channel_id: ch,
                    message_id: m,
                    pinned,
                },
            ));
            Ok(CommandResult::Ack)
        }
        SetNotifications(s) => {
            let (scope, target) = notification_scope(c, u, &s).await?;
            require(
                s.mute_until.is_none_or(|t| t <= i64::MAX as u64),
                ErrorCode::InvalidRequest,
            )?;
            sqlx::query("INSERT INTO notification_settings VALUES(?,?,?,?) ON CONFLICT(user_id,scope,target_id) DO UPDATE SET settings=excluded.settings").bind(id(u)?).bind(scope).bind(id(target)?).bind(serde_json::to_string(&s).map_err(|_|Error::new(ErrorCode::Internal))?).execute(&mut *c).await?;
            out.push((vec![u], Event::NotificationSettingsUpdated(s.clone())));
            Ok(CommandResult::NotificationSettings(s))
        }
        UpdateDevice {
            device_name,
            platform,
        } => {
            sqlx::query("UPDATE session_devices SET device_name=?,platform=? WHERE token_hash=?")
                .bind(name(&device_name, 100)?)
                .bind(name(&platform, 40)?)
                .bind(key)
                .execute(&mut *c)
                .await?;
            out.push((vec![u], Event::SessionsUpdated));
            Ok(CommandResult::Ack)
        }
        LogoutSession { session_id } => {
            require(
                session_id.len() == 32 && session_id.bytes().all(|b| b.is_ascii_hexdigit()),
                ErrorCode::InvalidRequest,
            )?;
            let n=sqlx::query("DELETE FROM sessions WHERE user_id=? AND token_hash IN(SELECT token_hash FROM session_devices WHERE id=?)").bind(id(u)?).bind(session_id).execute(&mut *c).await?.rows_affected();
            require(n > 0, ErrorCode::NotFound)?;
            out.push((vec![u], Event::SessionsUpdated));
            Ok(CommandResult::Ack)
        }
        LogoutAllOthers => {
            sqlx::query("DELETE FROM sessions WHERE user_id=? AND token_hash<>?")
                .bind(id(u)?)
                .bind(key)
                .execute(&mut *c)
                .await?;
            out.push((vec![u], Event::SessionsUpdated));
            Ok(CommandResult::Ack)
        }
    }
}
pub(crate) async fn guild_profile(c: &mut SqliteConnection, g: Id) -> Result<GuildProfile> {
    let r=sqlx::query("SELECT g.created_at,COALESCE(p.description,'') AS description,COALESCE(p.banner_color,0) AS banner_color FROM guilds g LEFT JOIN guild_profiles p ON p.guild_id=g.id WHERE g.id=?").bind(id(g)?).fetch_one(c).await?;
    Ok(GuildProfile {
        guild_id: g,
        description: r.get("description"),
        banner_color: r.get::<i64, _>("banner_color") as u8,
        created_at: r.get::<i64, _>("created_at") as u64,
    })
}
pub(crate) async fn user_profile(c: &mut SqliteConnection, u: Id) -> Result<UserProfile> {
    let r=sqlx::query("SELECT u.created_at,COALESCE(p.pronouns,'') AS pronouns,COALESCE(p.about_me,'') AS about_me,COALESCE(p.banner_color,0) AS banner_color FROM users u LEFT JOIN user_profiles p ON p.user_id=u.id WHERE u.id=?").bind(id(u)?).fetch_optional(&mut *c).await?.ok_or_else(||Error::new(ErrorCode::NotFound))?;
    Ok(UserProfile {
        user: user(c, u).await?,
        banner_color: r.get::<i64, _>("banner_color") as u32,
        pronouns: r.get("pronouns"),
        about_me: r.get("about_me"),
        created_at: r.get::<i64, _>("created_at") as u64,
    })
}
pub(crate) async fn profile(c: &mut SqliteConnection, u: Id, v: Id) -> Result<ProfileDetail> {
    profile_access(c, u, v).await?;
    let mut mutual_servers = vec![];
    for r in sqlx::query("SELECT b.guild_id,b.joined_at FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.user_id=? AND b.user_id=? ORDER BY b.guild_id").bind(id(u)?).bind(id(v)?).fetch_all(&mut *c).await? {
        mutual_servers.push(MutualServer {guild:guild(c,u,Id(r.get::<i64,_>("guild_id") as u64)).await?.guild,member_since:r.get::<i64,_>("joined_at") as u64});
    }
    let mut mutual_friends = vec![];
    for v in sqlx::query_scalar::<_,i64>("SELECT a.other_id FROM relationships a JOIN relationships b ON a.other_id=b.other_id WHERE a.user_id=? AND b.user_id=? AND a.kind='friend' AND b.kind='friend' ORDER BY a.other_id").bind(id(u)?).bind(id(v)?).fetch_all(&mut *c).await? {mutual_friends.push(user(c,Id(v as u64)).await?);}
    Ok(ProfileDetail {
        profile: user_profile(c, v).await?,
        mutual_servers,
        mutual_friends,
    })
}
async fn notification_scope(
    c: &mut SqliteConnection,
    u: Id,
    s: &NotificationSettings,
) -> Result<(&'static str, Id)> {
    if let Some(ch) = s.channel_id {
        let ch = access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
        require(s.guild_id == ch.summary.guild_id, ErrorCode::InvalidRequest)?;
        Ok(("channel", ch.summary.id))
    } else if let Some(g) = s.guild_id {
        crate::db::role(c, u, g).await?;
        Ok(("guild", g))
    } else {
        Err(Error::new(ErrorCode::InvalidRequest))
    }
}
pub(crate) async fn notifications(
    c: &mut SqliteConnection,
    u: Id,
) -> Result<Vec<NotificationSettings>> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT settings FROM notification_settings WHERE user_id=? ORDER BY scope,target_id",
    )
    .bind(id(u)?)
    .fetch_all(&mut *c)
    .await?;
    let mut out = vec![];
    for r in rows {
        let mut s: NotificationSettings =
            serde_json::from_str(&r).map_err(|_| Error::new(ErrorCode::Internal))?;
        if notification_scope(c, u, &s).await.is_ok() {
            if s.mute_until.is_some_and(|t| t <= now()) {
                s.muted = false;
            }
            out.push(s);
        }
    }
    Ok(out)
}
pub(crate) async fn sessions(
    c: &mut SqliteConnection,
    u: Id,
    key: &[u8],
) -> Result<Vec<DeviceSession>> {
    Ok(sqlx::query("SELECT d.*,s.last_used_at FROM session_devices d JOIN sessions s ON d.token_hash=s.token_hash WHERE s.user_id=? AND s.expires_at>? ORDER BY s.created_at,d.id").bind(id(u)?).bind(now() as i64).fetch_all(c).await?.into_iter().map(|r|DeviceSession {id:r.get("id"),device_name:r.get("device_name"),platform:r.get("platform"),last_active:r.get::<i64,_>("last_used_at") as u64,place:"this device".into(),current:r.get::<Vec<u8>,_>("token_hash")==key}).collect())
}
/// Only mutations which can change channel visibility require before/after projections.
pub(crate) async fn acl_guild(c: &mut SqliteConnection, cmd: &Command) -> Result<Option<Id>> {
    Ok(match cmd {
        Command::Launch(cmd) => match cmd {
            LaunchCommand::UpdateRole { role } => Some(role.guild_id),
            LaunchCommand::CreateRole { guild_id, .. }
            | LaunchCommand::DeleteRole { guild_id, .. }
            | LaunchCommand::ReorderRoles { guild_id, .. }
            | LaunchCommand::SetMemberRoles { guild_id, .. } => Some(*guild_id),
            LaunchCommand::SetOverwrites { channel_id, .. } => {
                channel(c, *channel_id).await?.summary.guild_id
            }
            _ => None,
        },
        Command::SetMemberRole { guild_id, .. }
        | Command::TransferGuildOwnership { guild_id, .. }
        | Command::ReorderChannels { guild_id, .. } => Some(*guild_id),
        Command::DeleteChannel { channel_id } => channel(c, *channel_id).await?.summary.guild_id,
        _ => None,
    })
}
/// Final defense-in-depth gate for all channel-bearing event projections.
pub(crate) async fn filter_event(
    c: &mut SqliteConnection,
    users: Vec<Id>,
    e: Event,
) -> Result<Deliveries> {
    // Preserve grouped immutable payloads so Core can share encoded frames among
    // recipients. Only sidebar placement/parent projections need per-user copies.
    if matches!(e, Event::ChannelsReordered { .. } | Event::ChannelUpsert(_)) {
        let mut out: Deliveries = vec![];
        for u in users {
            let projected = match &e {
                Event::ChannelUpsert(ch) => {
                    match access(c, u, ch.summary.id, Permissions::VIEW_CHANNEL).await {
                        Ok(visible) => Event::ChannelUpsert(visible),
                        Err(_) => continue,
                    }
                }
                Event::ChannelsReordered {
                    guild_id,
                    placements,
                } => {
                    let visible = channels(c, u, Some(*guild_id)).await?;
                    let placements = placements
                        .iter()
                        .filter_map(|p| {
                            visible.iter().find(|ch| ch.id == p.channel_id).map(|ch| {
                                ChannelPlacement {
                                    channel_id: p.channel_id,
                                    parent_id: ch.parent_id,
                                    position: p.position,
                                }
                            })
                        })
                        .collect();
                    Event::ChannelsReordered {
                        guild_id: *guild_id,
                        placements,
                    }
                }
                _ => unreachable!(),
            };
            if let Some((users, _)) = out.iter_mut().find(|(_, e)| *e == projected) {
                users.push(u);
            } else {
                out.push((vec![u], projected));
            }
        }
        return Ok(out);
    }
    let ch = match &e {
        Event::MessageCreated { message, .. } | Event::MessageUpdated(message) => {
            Some(message.channel_id)
        }
        Event::MessageDeleted { channel_id, .. }
        | Event::Typing { channel_id, .. }
        | Event::PinsUpdated { channel_id, .. } => Some(*channel_id),
        Event::ReadStateUpdated(r) => Some(r.channel_id),
        Event::ReactionUpdated(r) => Some(r.channel_id),
        _ => None,
    };
    let mut allowed = vec![];
    for u in users {
        if let Some(ch) = ch {
            if access(c, u, ch, Permissions::VIEW_CHANNEL).await.is_err() {
                continue;
            }
        }
        allowed.push(u);
    }
    Ok(vec![(allowed, e)])
}

pub(crate) async fn snapshot(
    c: &mut SqliteConnection,
    core: &Core,
    u: Id,
    key: &[u8],
    parts: &[&str],
    cursor: &ResumeCursor,
) -> Result<Option<crate::http::ApiResponse>> {
    use crate::http::{json, url_id};
    macro_rules! snap {
        ($data:expr) => {
            Some(json(Snapshot {
                data: $data,
                cursor: cursor.clone(),
            }))
        };
    }
    Ok(match parts {
        ["users", v, "profile"] => snap!(profile(c, u, url_id(v)?).await?),
        ["guilds", g, "roles"] => {
            let g = url_id(g)?;
            crate::db::role(c, u, g).await?;
            snap!(acl::roles(c, g).await?)
        }
        ["guilds", g, "profile"] => {
            let g = url_id(g)?;
            crate::db::role(c, u, g).await?;
            snap!(guild_profile(c, g).await?)
        }
        ["guilds", g, "preview"] => {
            let g = url_id(g)?;
            let guild = guild(c, u, g).await?.guild;
            let mut online_count = 0;
            for v in guild_users(c, g).await? {
                if !matches!(
                    presence(c, core, u, v).await?.status,
                    PresenceStatus::Offline | PresenceStatus::Invisible
                ) {
                    online_count += 1;
                }
            }
            snap!(GuildPreview {
                guild,
                profile: guild_profile(c, g).await?,
                online_count
            })
        }
        ["channels", ch, "permissions"] => {
            let ch = url_id(ch)?;
            let channel = access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
            let p = if let Some(guild) = channel.summary.guild_id {
                acl::evaluate(c, u, guild, Some(ch)).await?
            } else {
                let mut p = Permissions::VIEW_CHANNEL
                    | Permissions::READ_HISTORY
                    | Permissions::SEND_MESSAGES
                    | Permissions::ATTACH_FILES
                    | Permissions::ADD_REACTIONS;
                if channel.summary.kind == ChannelKind::Direct {
                    for peer in channel
                        .summary
                        .recipient_ids
                        .iter()
                        .filter(|peer| **peer != u)
                    {
                        if blocked(c, u, *peer).await? {
                            p.remove(Permissions::SEND_MESSAGES | Permissions::ATTACH_FILES);
                        }
                    }
                }
                p
            };
            snap!(p)
        }
        ["channels", ch, "overwrites"] => {
            let ch = url_id(ch)?;
            access(c, u, ch, Permissions::MANAGE_ROLES).await?;
            snap!(acl::overwrites(c, ch).await?)
        }
        ["channels", ch, "pins"] => {
            let ch = url_id(ch)?;
            text(c, u, ch, Permissions::READ_HISTORY).await?;
            let mut messages = vec![];
            for m in sqlx::query_scalar::<_,i64>("SELECT p.message_id FROM pins p JOIN messages m ON m.id=p.message_id WHERE p.channel_id=? AND m.deleted_at IS NULL ORDER BY p.pinned_at DESC,p.message_id DESC LIMIT 50").bind(id(ch)?).fetch_all(&mut *c).await? {messages.push(message(c,u,ch,Id(m as u64),false).await?);}
            snap!(messages)
        }
        ["me", "notifications"] => snap!(notifications(c, u).await?),
        ["me", "sessions"] => snap!(sessions(c, u, key).await?),
        _ => None,
    })
}
pub(crate) async fn ensure_legacy_admin(c: &mut SqliteConnection, g: Id) -> Result<()> {
    let list = acl::roles(c, g).await?;
    require(
        list.len() < MAX_ROLES_PER_GUILD || list.iter().any(|r| r.id == Id(g.0 + 1)),
        ErrorCode::LimitExceeded,
    )?;
    sqlx::query("INSERT OR IGNORE INTO roles(guild_id,id,name,position,permissions) SELECT ?,?,'Admin',COALESCE(MAX(position),0)+1,? FROM roles WHERE guild_id=?").bind(id(g)?).bind(id(g)?+1).bind(Permissions::ADMINISTRATOR.bits() as i64).bind(id(g)?).execute(c).await?;
    Ok(())
}
