//! Moderation audit log. Entries are written inside the transaction of the
//! action they describe, so a rolled-back action leaves no entry and a
//! committed one always has its entry.
//!
//! Existing commands are audited without touching their handlers: `prepare`
//! reads the state a command may change before it runs, and `record` compares
//! it with the state after it ran (still inside the command's transaction).
//! Nothing is written when nothing changed.
use super::*;

pub(crate) struct Entry {
    guild: Id,
    action: AuditAction,
    kind: AuditTarget,
    target: Option<Id>,
    name: Option<String>,
    changes: Vec<AuditChange>,
    reason: Option<String>,
}
impl Entry {
    pub(crate) fn new(
        guild: Id,
        action: AuditAction,
        kind: AuditTarget,
        target: Option<Id>,
    ) -> Self {
        Self {
            guild,
            action,
            kind,
            target,
            name: None,
            changes: vec![],
            reason: None,
        }
    }
    pub(crate) fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
    pub(crate) fn change(
        mut self,
        field: &str,
        before: Option<String>,
        after: Option<String>,
    ) -> Self {
        if before != after {
            self.changes.push(AuditChange {
                field: field.into(),
                before,
                after,
            });
        }
        self
    }
    pub(crate) fn reason(mut self, reason: Option<String>) -> Self {
        self.reason = reason;
        self
    }
}
fn clip(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        s
    } else {
        let mut v: String = s.chars().take(max.saturating_sub(1)).collect();
        v.push('…');
        v
    }
}
/// Write one entry and drop the oldest beyond the per-server maximum.
pub(crate) async fn write(c: &mut SqliteConnection, actor: Id, mut e: Entry) -> Result<()> {
    e.changes.truncate(MAX_AUDIT_CHANGES);
    for change in &mut e.changes {
        change.field = clip(std::mem::take(&mut change.field), 64);
        change.before = change.before.take().map(|v| clip(v, MAX_AUDIT_VALUE_CHARS));
        change.after = change.after.take().map(|v| clip(v, MAX_AUDIT_VALUE_CHARS));
    }
    let changes = serde_json::to_string(&e.changes).map_err(|_| Error::new(ErrorCode::Internal))?;
    sqlx::query("INSERT INTO audit_log(guild_id,actor_id,action,target_kind,target_id,target_name,changes,reason,created_at) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(id(e.guild)?)
        .bind(id(actor)?)
        .bind(e.action.name())
        .bind(enum_str(e.kind))
        .bind(e.target.map(|v| v.0 as i64))
        .bind(e.name.map(|v| clip(v, 100)))
        .bind(changes)
        .bind(e.reason.map(|v| clip(v, MAX_REASON_CHARS)))
        .bind(now() as i64)
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM audit_log WHERE guild_id=?1 AND id<=(SELECT id FROM audit_log WHERE guild_id=?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)")
        .bind(id(e.guild)?)
        .bind(MAX_AUDIT_ENTRIES_PER_GUILD as i64)
        .execute(&mut *c)
        .await?;
    Ok(())
}

/// Names of the permission bits, for readable before/after summaries.
pub(crate) fn permission_names(p: Permissions) -> String {
    const NAMES: &[(Permissions, &str)] = &[
        (Permissions::VIEW_CHANNEL, "View Channels"),
        (Permissions::SEND_MESSAGES, "Send Messages"),
        (Permissions::READ_HISTORY, "Read History"),
        (Permissions::ATTACH_FILES, "Attach Files"),
        (Permissions::ADD_REACTIONS, "Add Reactions"),
        (Permissions::MANAGE_MESSAGES, "Manage Messages"),
        (Permissions::MANAGE_CHANNELS, "Manage Channels"),
        (Permissions::MANAGE_GUILD, "Manage Server"),
        (Permissions::CREATE_INVITES, "Create Invites"),
        (Permissions::KICK_MEMBERS, "Kick Members"),
        (Permissions::BAN_MEMBERS, "Ban Members"),
        (Permissions::MANAGE_MEMBERS, "Manage Members"),
        (Permissions::ADMINISTRATOR, "Administrator"),
        (Permissions::MANAGE_ROLES, "Manage Roles"),
        (Permissions::CHANGE_NICKNAME, "Change Nickname"),
        (Permissions::MANAGE_NICKNAMES, "Manage Nicknames"),
        (Permissions::EMBED_LINKS, "Embed Links"),
        (Permissions::MENTION_EVERYONE, "Mention Everyone"),
        (Permissions::CONNECT, "Connect"),
        (Permissions::SPEAK, "Speak"),
        (Permissions::STREAM, "Share Screen"),
        (Permissions::VIDEO, "Video"),
        (Permissions::MUTE_MEMBERS, "Mute Members"),
        (Permissions::DEAFEN_MEMBERS, "Deafen Members"),
        (Permissions::MOVE_MEMBERS, "Move Members"),
        (Permissions::USE_VOICE_ACTIVITY, "Voice Activity"),
        (Permissions::PRIORITY_SPEAKER, "Priority Speaker"),
        (Permissions::MANAGE_EMOJI, "Manage Emoji"),
        (Permissions::VIEW_AUDIT_LOG, "View Audit Log"),
        (Permissions::MODERATE_MEMBERS, "Time Out Members"),
        (Permissions::CREATE_PUBLIC_THREADS, "Create Public Threads"),
        (
            Permissions::CREATE_PRIVATE_THREADS,
            "Create Private Threads",
        ),
        (
            Permissions::SEND_MESSAGES_IN_THREADS,
            "Send Messages in Threads",
        ),
        (Permissions::MANAGE_THREADS, "Manage Threads"),
        (Permissions::PUBLISH_MESSAGES, "Publish Messages"),
        (Permissions::USE_SOUNDBOARD, "Use Soundboard"),
        (Permissions::MANAGE_SOUNDBOARD, "Manage Soundboard"),
    ];
    let names: Vec<&str> = NAMES
        .iter()
        .filter(|(bit, _)| p.contains(*bit))
        .map(|(_, n)| *n)
        .collect();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}
fn overwrite_text(o: Option<&PermissionOverwrite>) -> Option<String> {
    o.map(|o| {
        format!(
            "allow: {}; deny: {}",
            permission_names(o.allow),
            permission_names(o.deny)
        )
    })
}
fn duration(seconds: u64) -> String {
    match seconds {
        0 => "off".into(),
        s if s % 3600 == 0 => format!("{}h", s / 3600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}
pub(crate) fn slowmode_text(seconds: u32) -> String {
    duration(u64::from(seconds))
}

/// The state a command may change, read before it runs.
pub(crate) enum Pending {
    /// Threads, forums, announcements and polls (`feature_audit.rs`).
    Feature(super::feature_audit::Pending),
    Guild {
        g: Id,
        name: String,
        icon: Option<Id>,
    },
    Profile {
        g: Id,
        description: String,
        banner: u8,
    },
    Owner {
        g: Id,
        owner: Id,
    },
    LegacyRole {
        g: Id,
        user: Id,
        role: MemberRole,
    },
    RoleCreate {
        g: Id,
    },
    Role {
        g: Id,
        role: Role,
    },
    RoleDelete {
        g: Id,
        role: Role,
    },
    Roles {
        g: Id,
        roles: Vec<Role>,
    },
    MemberRoles {
        g: Id,
        user: Id,
        roles: Vec<Id>,
    },
    Overwrites {
        g: Id,
        ch: Id,
        name: String,
        list: Vec<PermissionOverwrite>,
    },
    Nickname {
        g: Id,
        user: Id,
        nick: Option<String>,
    },
    ChannelCreate {
        g: Id,
    },
    Channel {
        g: Id,
        ch: Channel,
    },
    Slowmode {
        g: Id,
        ch: Id,
        name: String,
        seconds: u32,
    },
    Placements {
        g: Id,
        list: Vec<(Id, Option<Id>, String)>,
    },
    ChannelDelete {
        g: Id,
        ch: Channel,
    },
    Removal {
        g: Id,
        user: Id,
        ban: bool,
        reason: Option<String>,
        member: bool,
        banned: bool,
    },
    Unban {
        g: Id,
        user: Id,
    },
    InviteCreate {
        g: Id,
    },
    InviteDelete {
        g: Id,
        invite: Invite,
    },
    MessageDelete {
        g: Id,
        name: String,
        message: Id,
        author: Id,
    },
    Pin {
        g: Id,
        ch: Id,
        name: String,
        message: Id,
        author: Id,
        pinned: bool,
    },
    Emoji {
        g: Id,
        emoji: Id,
        name: String,
    },
}
async fn guild_of(c: &mut SqliteConnection, ch: Id) -> Result<Option<(Id, String)>> {
    let r = sqlx::query("SELECT guild_id,name FROM channels WHERE id=?")
        .bind(id(ch)?)
        .fetch_optional(&mut *c)
        .await?;
    Ok(r.and_then(|r| {
        r.get::<Option<i64>, _>(0).map(|g| {
            (
                Id(g as u64),
                r.get::<Option<String>, _>(1).unwrap_or_default(),
            )
        })
    }))
}
async fn guild_row(c: &mut SqliteConnection, g: Id) -> Result<(String, Option<Id>, Id)> {
    let r = sqlx::query("SELECT name,icon_id,owner_id FROM guilds WHERE id=?")
        .bind(id(g)?)
        .fetch_one(&mut *c)
        .await?;
    Ok((
        r.get(0),
        r.get::<Option<i64>, _>(1).map(|v| Id(v as u64)),
        Id(r.get::<i64, _>(2) as u64),
    ))
}
async fn banned(c: &mut SqliteConnection, g: Id, u: Id) -> Result<bool> {
    Ok(
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM guild_bans WHERE guild_id=? AND user_id=?)",
        )
        .bind(id(g)?)
        .bind(id(u)?)
        .fetch_one(&mut *c)
        .await?,
    )
}
async fn is_member(c: &mut SqliteConnection, g: Id, u: Id) -> Result<bool> {
    Ok(db::role(c, u, g).await.is_ok())
}
async fn nickname(c: &mut SqliteConnection, g: Id, u: Id) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT nickname FROM nicknames WHERE guild_id=? AND user_id=?")
            .bind(id(g)?)
            .bind(id(u)?)
            .fetch_optional(&mut *c)
            .await?,
    )
}
async fn placements(c: &mut SqliteConnection, g: Id) -> Result<Vec<(Id, Option<Id>, String)>> {
    Ok(sqlx::query(
        "SELECT id,parent_id,COALESCE(name,'') FROM channels WHERE guild_id=? ORDER BY id",
    )
    .bind(id(g)?)
    .fetch_all(&mut *c)
    .await?
    .into_iter()
    .map(|r| {
        (
            Id(r.get::<i64, _>(0) as u64),
            r.get::<Option<i64>, _>(1).map(|v| Id(v as u64)),
            r.get(2),
        )
    })
    .collect())
}
async fn slowmode_seconds(c: &mut SqliteConnection, ch: Id) -> Result<u32> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT seconds FROM channel_slowmode WHERE channel_id=?")
            .bind(id(ch)?)
            .fetch_optional(&mut *c)
            .await?
            .unwrap_or(0) as u32,
    )
}
async fn emoji_name(c: &mut SqliteConnection, g: Id, e: Id) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT name FROM guild_emoji WHERE guild_id=? AND id=?")
            .bind(id(g)?)
            .bind(id(e)?)
            .fetch_optional(&mut *c)
            .await?,
    )
}

/// Read what `cmd` may change. Never fails the command: a lookup that fails
/// here fails in the command itself with its own error.
pub(crate) async fn prepare(c: &mut SqliteConnection, cmd: &Command) -> Option<Pending> {
    prepare_inner(c, cmd).await.ok().flatten()
}
async fn prepare_inner(c: &mut SqliteConnection, cmd: &Command) -> Result<Option<Pending>> {
    use Command::*;
    let pending: Pending = match cmd {
        Threads(_) | Content(_) => {
            return Ok(super::feature_audit::prepare(c, cmd)
                .await?
                .map(Pending::Feature))
        }
        UpdateGuild { guild_id: g, .. } => {
            let (name, icon, _) = guild_row(c, *g).await?;
            Pending::Guild { g: *g, name, icon }
        }
        TransferGuildOwnership { guild_id: g, .. } => Pending::Owner {
            g: *g,
            owner: guild_row(c, *g).await?.2,
        },
        SetMemberRole {
            guild_id: g,
            user_id,
            ..
        } => Pending::LegacyRole {
            g: *g,
            user: *user_id,
            role: db::role(c, *user_id, *g).await?,
        },
        KickMember {
            guild_id: g,
            user_id,
        }
        | BanMember {
            guild_id: g,
            user_id,
            ..
        } => {
            let reason = if let BanMember { reason, .. } = cmd {
                reason.clone()
            } else {
                None
            };
            Pending::Removal {
                g: *g,
                user: *user_id,
                ban: matches!(cmd, BanMember { .. }),
                reason,
                member: is_member(c, *g, *user_id).await?,
                banned: banned(c, *g, *user_id).await?,
            }
        }
        UnbanMember {
            guild_id: g,
            user_id,
        } => {
            if !banned(c, *g, *user_id).await? {
                return Ok(None);
            }
            Pending::Unban {
                g: *g,
                user: *user_id,
            }
        }
        CreateInvite { guild_id: g, .. } => Pending::InviteCreate { g: *g },
        RevokeInvite { guild_id: g, code } => {
            let found: Option<i64> =
                sqlx::query_scalar("SELECT id FROM invites WHERE guild_id=? AND code_hash=?")
                    .bind(id(*g)?)
                    .bind(crate::state::hash(code.as_bytes()))
                    .fetch_optional(&mut *c)
                    .await?;
            let Some(found) = found else { return Ok(None) };
            Pending::InviteDelete {
                g: *g,
                invite: db::invite(c, Id(found as u64)).await?,
            }
        }
        CreateChannel { guild_id: g, .. } => Pending::ChannelCreate { g: *g },
        UpdateChannel { channel_id, .. } => {
            let ch = db::channel(c, *channel_id).await?;
            let Some(g) = ch.summary.guild_id else {
                return Ok(None);
            };
            Pending::Channel { g, ch }
        }
        ReorderChannels { guild_id: g, .. } => Pending::Placements {
            g: *g,
            list: placements(c, *g).await?,
        },
        DeleteChannel { channel_id } => {
            let ch = db::channel(c, *channel_id).await?;
            let Some(g) = ch.summary.guild_id else {
                return Ok(None);
            };
            Pending::ChannelDelete { g, ch }
        }
        SetVoicePermission { channel_id, .. } => {
            let Some((g, name)) = guild_of(c, *channel_id).await? else {
                return Ok(None);
            };
            Pending::Overwrites {
                g,
                ch: *channel_id,
                name,
                list: acl::overwrites(c, *channel_id).await?,
            }
        }
        DeleteMessage {
            channel_id,
            message_id,
        } => {
            let Some((g, name)) = guild_of(c, *channel_id).await? else {
                return Ok(None);
            };
            let r = sqlx::query(
                "SELECT author_id FROM messages WHERE id=? AND channel_id=? AND deleted_at IS NULL",
            )
            .bind(id(*message_id)?)
            .bind(id(*channel_id)?)
            .fetch_optional(&mut *c)
            .await?;
            let Some(r) = r else { return Ok(None) };
            Pending::MessageDelete {
                g,
                name,
                message: *message_id,
                author: Id(r.get::<i64, _>(0) as u64),
            }
        }
        Launch(l) => match l {
            LaunchCommand::UpdateGuildProfile { guild_id: g, .. } => {
                let p = crate::launch::guild_profile(c, *g).await?;
                Pending::Profile {
                    g: *g,
                    description: p.description,
                    banner: p.banner_color,
                }
            }
            LaunchCommand::CreateRole { guild_id: g, .. } => Pending::RoleCreate { g: *g },
            LaunchCommand::UpdateRole { role } => Pending::Role {
                g: role.guild_id,
                role: acl::roles(c, role.guild_id)
                    .await?
                    .into_iter()
                    .find(|r| r.id == role.id)
                    .ok_or_else(|| Error::new(ErrorCode::NotFound))?,
            },
            LaunchCommand::DeleteRole {
                guild_id: g,
                role_id,
            } => Pending::RoleDelete {
                g: *g,
                role: acl::roles(c, *g)
                    .await?
                    .into_iter()
                    .find(|r| r.id == *role_id)
                    .ok_or_else(|| Error::new(ErrorCode::NotFound))?,
            },
            LaunchCommand::ReorderRoles { guild_id: g, .. } => Pending::Roles {
                g: *g,
                roles: acl::roles(c, *g).await?,
            },
            LaunchCommand::SetMemberRoles {
                guild_id: g,
                user_id,
                ..
            } => Pending::MemberRoles {
                g: *g,
                user: *user_id,
                roles: acl::role_ids(c, *g, *user_id).await?,
            },
            LaunchCommand::SetOverwrites { channel_id, .. } => {
                let Some((g, name)) = guild_of(c, *channel_id).await? else {
                    return Ok(None);
                };
                Pending::Overwrites {
                    g,
                    ch: *channel_id,
                    name,
                    list: acl::overwrites(c, *channel_id).await?,
                }
            }
            LaunchCommand::SetNickname {
                guild_id: g,
                user_id,
                ..
            } => Pending::Nickname {
                g: *g,
                user: *user_id,
                nick: nickname(c, *g, *user_id).await?,
            },
            LaunchCommand::SetSlowmode { channel_id, .. } => {
                let Some((g, name)) = guild_of(c, *channel_id).await? else {
                    return Ok(None);
                };
                Pending::Slowmode {
                    g,
                    ch: *channel_id,
                    name,
                    seconds: slowmode_seconds(c, *channel_id).await?,
                }
            }
            LaunchCommand::PinMessage {
                channel_id,
                message_id,
                ..
            } => {
                let Some((g, name)) = guild_of(c, *channel_id).await? else {
                    return Ok(None);
                };
                let author: Option<i64> = sqlx::query_scalar(
                    "SELECT author_id FROM messages WHERE id=? AND channel_id=?",
                )
                .bind(id(*message_id)?)
                .bind(id(*channel_id)?)
                .fetch_optional(&mut *c)
                .await?;
                let Some(author) = author else {
                    return Ok(None);
                };
                Pending::Pin {
                    g,
                    ch: *channel_id,
                    name,
                    message: *message_id,
                    author: Id(author as u64),
                    pinned: pinned(c, *channel_id, *message_id).await?,
                }
            }
            LaunchCommand::RenameEmoji {
                guild_id: g,
                emoji_id,
                ..
            }
            | LaunchCommand::DeleteEmoji {
                guild_id: g,
                emoji_id,
            } => {
                let Some(name) = emoji_name(c, *g, *emoji_id).await? else {
                    return Ok(None);
                };
                Pending::Emoji {
                    g: *g,
                    emoji: *emoji_id,
                    name,
                }
            }
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    Ok(Some(pending))
}
async fn pinned(c: &mut SqliteConnection, ch: Id, m: Id) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pins WHERE channel_id=? AND message_id=?)")
            .bind(id(ch)?)
            .bind(id(m)?)
            .fetch_one(&mut *c)
            .await?,
    )
}
fn yes(b: bool) -> Option<String> {
    Some(if b { "yes" } else { "no" }.into())
}
async fn role_name(c: &mut SqliteConnection, g: Id, r: Id) -> Result<String> {
    Ok(acl::roles(c, g)
        .await?
        .into_iter()
        .find(|v| v.id == r)
        .map_or_else(|| format!("role {}", r.0), |v| v.name))
}
async fn target_label(c: &mut SqliteConnection, o: &PermissionOverwrite, g: Id) -> Result<String> {
    Ok(match o.kind {
        OverwriteKind::Role if o.target_id == g => "@everyone".into(),
        OverwriteKind::Role => role_name(c, g, o.target_id).await?,
        OverwriteKind::Member => match db::user(c, o.target_id).await {
            Ok(u) => format!("@{}", u.username),
            Err(_) => format!("member {}", o.target_id.0),
        },
    })
}

/// Compare with the state after the command ran, in its transaction, and log it.
pub(crate) async fn record(
    c: &mut SqliteConnection,
    actor: Id,
    pending: Option<Pending>,
    result: &CommandResult,
) -> Result<()> {
    let Some(pending) = pending else {
        return Ok(());
    };
    use AuditAction as A;
    use AuditTarget as T;
    let entry = match pending {
        Pending::Feature(p) => return super::feature_audit::record(c, actor, p, result).await,
        Pending::Guild { g, name, icon } => {
            let (after, after_icon, _) = guild_row(c, g).await?;
            Entry::new(g, A::GuildUpdate, T::Guild, Some(g))
                .named(after.clone())
                .change("name", Some(name), Some(after))
                .change(
                    "icon",
                    icon.map(|_| "custom".to_string()),
                    after_icon.map(|_| "custom".to_string()),
                )
        }
        Pending::Profile {
            g,
            description,
            banner,
        } => {
            let p = crate::launch::guild_profile(c, g).await?;
            Entry::new(g, A::GuildUpdate, T::Guild, Some(g))
                .change("description", Some(description), Some(p.description))
                .change(
                    "banner",
                    Some(banner.to_string()),
                    Some(p.banner_color.to_string()),
                )
        }
        Pending::Owner { g, owner } => {
            let after = guild_row(c, g).await?.2;
            if after == owner {
                return Ok(());
            }
            let name = db::user(c, after).await?.username;
            Entry::new(g, A::OwnershipTransfer, T::User, Some(after))
                .named(name)
                .change(
                    "owner",
                    Some(owner.0.to_string()),
                    Some(after.0.to_string()),
                )
        }
        Pending::LegacyRole { g, user, role } => {
            let after = db::role(c, user, g).await?;
            let name = db::user(c, user).await?.username;
            Entry::new(g, A::MemberRolesUpdate, T::User, Some(user))
                .named(name)
                .change("role", Some(enum_str(role)), Some(enum_str(after)))
        }
        Pending::RoleCreate { g } => {
            let CommandResult::Role(r) = result else {
                return Ok(());
            };
            Entry::new(g, A::RoleCreate, T::Role, Some(r.id))
                .named(r.name.clone())
                .change("name", None, Some(r.name.clone()))
        }
        Pending::Role { g, role } => {
            let Some(after) = acl::roles(c, g)
                .await?
                .into_iter()
                .find(|r| r.id == role.id)
            else {
                return Ok(());
            };
            let added = after.permissions - role.permissions;
            let removed = role.permissions - after.permissions;
            Entry::new(g, A::RoleUpdate, T::Role, Some(role.id))
                .named(after.name.clone())
                .change("name", Some(role.name), Some(after.name))
                .change(
                    "color",
                    Some(format!("#{:06x}", role.color)),
                    Some(format!("#{:06x}", after.color)),
                )
                .change("hoist", yes(role.hoist), yes(after.hoist))
                .change("mentionable", yes(role.mentionable), yes(after.mentionable))
                .change(
                    "permissions granted",
                    None,
                    (!added.is_empty()).then(|| permission_names(added)),
                )
                .change(
                    "permissions removed",
                    None,
                    (!removed.is_empty()).then(|| permission_names(removed)),
                )
        }
        Pending::RoleDelete { g, role } => {
            if acl::roles(c, g).await?.iter().any(|r| r.id == role.id) {
                return Ok(());
            }
            Entry::new(g, A::RoleDelete, T::Role, Some(role.id))
                .named(role.name.clone())
                .change("name", Some(role.name), None)
        }
        Pending::Roles { g, roles } => {
            let after = acl::roles(c, g).await?;
            let mut e = Entry::new(g, A::RolesReorder, T::Guild, Some(g));
            for r in &after {
                if let Some(old) = roles.iter().find(|o| o.id == r.id) {
                    e = e.change(
                        &r.name,
                        Some(old.position.to_string()),
                        Some(r.position.to_string()),
                    );
                }
            }
            e
        }
        Pending::MemberRoles { g, user, roles } => {
            let after = acl::role_ids(c, g, user).await?;
            let mut added = vec![];
            let mut removed = vec![];
            for r in after.iter().filter(|r| !roles.contains(r)) {
                added.push(role_name(c, g, *r).await?);
            }
            for r in roles.iter().filter(|r| !after.contains(r)) {
                removed.push(role_name(c, g, *r).await?);
            }
            let name = db::user(c, user).await?.username;
            Entry::new(g, A::MemberRolesUpdate, T::User, Some(user))
                .named(name)
                .change(
                    "roles added",
                    None,
                    (!added.is_empty()).then(|| added.join(", ")),
                )
                .change(
                    "roles removed",
                    None,
                    (!removed.is_empty()).then(|| removed.join(", ")),
                )
        }
        Pending::Overwrites { g, ch, name, list } => {
            let after = acl::overwrites(c, ch).await?;
            let mut e = Entry::new(g, A::OverwritesUpdate, T::Channel, Some(ch)).named(name);
            let mut targets: Vec<(OverwriteKind, Id)> = list
                .iter()
                .chain(after.iter())
                .map(|o| (o.kind, o.target_id))
                .collect();
            targets.sort_by_key(|(k, t)| (enum_str(*k), *t));
            targets.dedup();
            for (kind, target) in targets {
                let old = list
                    .iter()
                    .find(|o| o.kind == kind && o.target_id == target);
                let new = after
                    .iter()
                    .find(|o| o.kind == kind && o.target_id == target);
                if old == new {
                    continue;
                }
                let label = target_label(c, old.or(new).expect("one side exists"), g).await?;
                e = e.change(&label, overwrite_text(old), overwrite_text(new));
            }
            e
        }
        Pending::Nickname { g, user, nick } => {
            if user == actor {
                return Ok(());
            }
            let after = nickname(c, g, user).await?;
            let name = db::user(c, user).await?.username;
            Entry::new(g, A::NicknameUpdate, T::User, Some(user))
                .named(name)
                .change("nickname", nick, after)
        }
        Pending::ChannelCreate { g } => {
            let CommandResult::Channel(ch) = result else {
                return Ok(());
            };
            let name = ch.summary.name.clone().unwrap_or_default();
            Entry::new(g, A::ChannelCreate, T::Channel, Some(ch.summary.id))
                .named(name.clone())
                .change("name", None, Some(name))
                .change("type", None, Some(enum_str(ch.summary.kind)))
        }
        Pending::Channel { g, ch } => {
            let Ok(after) = db::channel(c, ch.summary.id).await else {
                return Ok(());
            };
            Entry::new(g, A::ChannelUpdate, T::Channel, Some(ch.summary.id))
                .named(after.summary.name.clone().unwrap_or_default())
                .change("name", ch.summary.name, after.summary.name)
                .change("topic", ch.topic, after.topic)
        }
        Pending::Slowmode {
            g,
            ch,
            name,
            seconds,
        } => {
            let after = slowmode_seconds(c, ch).await?;
            Entry::new(g, A::ChannelUpdate, T::Channel, Some(ch))
                .named(name)
                .change(
                    "slowmode",
                    Some(slowmode_text(seconds)),
                    Some(slowmode_text(after)),
                )
        }
        Pending::Placements { g, list } => {
            let after = placements(c, g).await?;
            let names: HashMap<Id, String> =
                after.iter().map(|(i, _, n)| (*i, n.clone())).collect();
            let label = |p: Option<Id>| {
                Some(p.map_or_else(
                    || "top level".to_string(),
                    |p| names.get(&p).cloned().unwrap_or_default(),
                ))
            };
            let mut e = Entry::new(g, A::ChannelMove, T::Guild, Some(g));
            for (ch, parent, name) in &after {
                if let Some((_, old, _)) = list.iter().find(|(i, _, _)| i == ch) {
                    if old != parent {
                        e = e.change(name, label(*old), label(*parent));
                    }
                }
            }
            if e.changes.is_empty() {
                // Only the order within the same parents changed.
                e = e.change("order", None, Some("channels reordered".into()));
            }
            e
        }
        Pending::ChannelDelete { g, ch } => {
            if db::channel(c, ch.summary.id).await.is_ok() {
                return Ok(());
            }
            let name = ch.summary.name.clone().unwrap_or_default();
            Entry::new(g, A::ChannelDelete, T::Channel, Some(ch.summary.id))
                .named(name.clone())
                .change("name", Some(name), None)
        }
        Pending::Removal {
            g,
            user,
            ban,
            reason,
            member,
            banned: was_banned,
        } => {
            let name = db::user(c, user).await?.username;
            if ban {
                if was_banned || !banned(c, g, user).await? {
                    return Ok(());
                }
                Entry::new(g, A::MemberBan, T::User, Some(user))
                    .named(name)
                    .reason(reason)
            } else {
                if !member || is_member(c, g, user).await? {
                    return Ok(());
                }
                Entry::new(g, A::MemberKick, T::User, Some(user)).named(name)
            }
        }
        Pending::Unban { g, user } => {
            if banned(c, g, user).await? {
                return Ok(());
            }
            let name = db::user(c, user).await?.username;
            Entry::new(g, A::MemberUnban, T::User, Some(user)).named(name)
        }
        Pending::InviteCreate { g } => {
            let CommandResult::Invite(i) = result else {
                return Ok(());
            };
            Entry::new(g, A::InviteCreate, T::Invite, Some(i.id))
                .named(i.code.clone())
                .change("max uses", None, Some(i.max_uses.to_string()))
                .change(
                    "expires in",
                    None,
                    Some(duration(i.expires_at.saturating_sub(now()) / 1000)),
                )
        }
        Pending::InviteDelete { g, invite } => {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM invites WHERE id=?)")
                    .bind(id(invite.id)?)
                    .fetch_one(&mut *c)
                    .await?;
            if exists {
                return Ok(());
            }
            Entry::new(g, A::InviteDelete, T::Invite, Some(invite.id))
                .named(invite.code)
                .change("uses", Some(invite.uses.to_string()), None)
        }
        Pending::MessageDelete {
            g,
            name,
            message,
            author,
        } => {
            if author == actor {
                return Ok(());
            }
            let gone: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id=? AND deleted_at IS NOT NULL)",
            )
            .bind(id(message)?)
            .fetch_one(&mut *c)
            .await?;
            if !gone {
                return Ok(());
            }
            let user = db::user(c, author).await?.username;
            Entry::new(g, A::MessageDelete, T::User, Some(author))
                .named(user)
                .change("channel", None, Some(format!("#{name}")))
        }
        Pending::Pin {
            g,
            ch,
            name,
            message,
            author,
            pinned: before,
        } => {
            let after = pinned(c, ch, message).await?;
            if after == before {
                return Ok(());
            }
            let user = db::user(c, author)
                .await
                .map(|u| u.username)
                .unwrap_or_default();
            Entry::new(
                g,
                if after {
                    A::MessagePin
                } else {
                    A::MessageUnpin
                },
                T::Message,
                Some(message),
            )
            .named(format!("message by @{user}"))
            .change("channel", None, Some(format!("#{name}")))
        }
        Pending::Emoji { g, emoji, name } => match emoji_name(c, g, emoji).await? {
            None => Entry::new(g, A::EmojiDelete, T::Emoji, Some(emoji))
                .named(name.clone())
                .change("name", Some(name), None),
            Some(after) => Entry::new(g, A::EmojiUpdate, T::Emoji, Some(emoji))
                .named(after.clone())
                .change("name", Some(name), Some(after)),
        },
    };
    if entry.changes.is_empty()
        && !matches!(
            entry.action,
            A::MemberKick | A::MemberBan | A::MemberUnban | A::MessageDelete
        )
    {
        return Ok(());
    }
    write(c, actor, entry).await
}
/// Hook for the custom emoji upload route, inside its transaction.
pub(crate) async fn emoji_created(
    c: &mut SqliteConnection,
    actor: Id,
    g: Id,
    emoji: Id,
    name: &str,
) -> Result<()> {
    write(
        c,
        actor,
        Entry::new(g, AuditAction::EmojiCreate, AuditTarget::Emoji, Some(emoji))
            .named(name)
            .change("name", None, Some(name.into())),
    )
    .await
}
fn entry_row(r: &sqlx::sqlite::SqliteRow) -> Option<AuditEntry> {
    Some(AuditEntry {
        id: Id(r.get::<i64, _>("id") as u64),
        guild_id: Id(r.get::<i64, _>("guild_id") as u64),
        actor_id: Id(r.get::<i64, _>("actor_id") as u64),
        action: AuditAction::parse(&r.get::<String, _>("action"))?,
        target_kind: en(r.get("target_kind")).ok()?,
        target_id: r.get::<Option<i64>, _>("target_id").map(|v| Id(v as u64)),
        target_name: r.get("target_name"),
        changes: serde_json::from_str(&r.get::<String, _>("changes")).unwrap_or_default(),
        reason: r.get("reason"),
        created_at: r.get::<i64, _>("created_at") as u64,
    })
}
#[derive(serde::Deserialize)]
struct Query {
    before: Option<u64>,
    limit: Option<u16>,
    actor: Option<u64>,
    action: Option<String>,
}
/// `GET /api/v1/guilds/{id}/audit-log?before=&limit=&actor=&action=`, newest first.
pub(crate) async fn endpoint(
    State(app): State<App>,
    Path(g): Path<u64>,
    axum::extract::RawQuery(raw): axum::extract::RawQuery,
    h: HeaderMap,
) -> Result<http::ApiResponse> {
    let q: Query = serde_urlencoded::from_str(raw.as_deref().unwrap_or_default())
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let _core = app.0.core.lock().await;
    let mut c = app.0.pool.acquire().await?;
    let (_, u) = auth(&mut c, http::bearer(&h)?).await?;
    let g = Id(g);
    guild_perm(&mut c, u, g, Permissions::VIEW_AUDIT_LOG).await?;
    let limit = q.limit.unwrap_or(50);
    require(
        (1..=MAX_AUDIT_PAGE).contains(&limit),
        ErrorCode::InvalidRequest,
    )?;
    let action = match q.action.as_deref().filter(|v| !v.is_empty()) {
        Some(a) => {
            Some(AuditAction::parse(a).ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?)
        }
        None => None,
    };
    let before = q.before.map(|v| id(Id(v))).transpose()?;
    let actor = q.actor.map(|v| id(Id(v))).transpose()?;
    let rows = sqlx::query("SELECT * FROM audit_log WHERE guild_id=? AND (?2 IS NULL OR id<?2) AND (?3 IS NULL OR actor_id=?3) AND (?4 IS NULL OR action=?4) ORDER BY id DESC LIMIT ?5")
        .bind(id(g)?)
        .bind(before)
        .bind(actor)
        .bind(action.map(AuditAction::name))
        .bind(i64::from(limit) + 1)
        .fetch_all(&mut *c)
        .await?;
    let has_more = rows.len() > usize::from(limit);
    let items: Vec<AuditEntry> = rows
        .iter()
        .take(usize::from(limit))
        .filter_map(entry_row)
        .collect();
    let mut ids: Vec<Id> = items.iter().map(|e| e.actor_id).collect();
    ids.extend(
        items
            .iter()
            .filter(|e| e.target_kind == AuditTarget::User)
            .filter_map(|e| e.target_id),
    );
    ids.sort();
    ids.dedup();
    let mut users = vec![];
    for v in ids {
        if let Ok(user) = db::user(&mut c, v).await {
            users.push(user);
        }
    }
    // The last row read, even if it could not be shown, so paging never stalls.
    let next_before = if has_more {
        rows.get(usize::from(limit) - 1)
            .map(|r| Id(r.get::<i64, _>("id") as u64))
    } else {
        None
    };
    Ok(http::json(AuditLogPage {
        items,
        users,
        next_before,
        has_more,
    }))
}
