//! In-process settings peer for interactive --mock. Live tests use the real server.
use super::*;
pub(super) fn seed(model: &mut Model, now: UnixMillis) {
    let guilds: Vec<_> = model.guilds.values().map(|g| g.guild.clone()).collect();
    for guild in guilds {
        model.settings.roles.insert(
            guild.id,
            Role {
                id: guild.id,
                guild_id: guild.id,
                name: "@everyone".into(),
                color: 0,
                position: 0,
                hoist: false,
                mentionable: false,
                permissions: Permissions::VIEW_CHANNEL
                    | Permissions::SEND_MESSAGES
                    | Permissions::READ_HISTORY
                    | Permissions::CREATE_INVITES
                    | Permissions::ADD_REACTIONS
                    | Permissions::CONNECT
                    | Permissions::SPEAK,
            },
        );
        model.settings.guild_profiles.insert(
            guild.id,
            GuildPreview {
                guild: guild.clone(),
                profile: GuildProfile {
                    guild_id: guild.id,
                    description: String::new(),
                    banner_color: 0,
                    created_at: now - 86_400_000,
                },
                online_count: 5,
            },
        );
    }
    for m in model.members.values_mut() {
        m.role_ids = vec![m.guild_id];
        if Some(m.user.id) == model.me.as_ref().map(|m| m.user.id) {
            model.settings.memberships.insert(m.guild_id, m.clone());
        }
    }
    for u in model.users.values() {
        model.settings.profiles.insert(
            u.id,
            UserProfile {
                user: u.clone(),
                banner_color: 0x3ddc97,
                pronouns: String::new(),
                about_me: String::new(),
                created_at: now - 86_400_000,
            },
        );
    }
    model.settings.sessions = vec![
        DeviceSession {
            id: "mock-current".into(),
            device_name: "Desktop PC".into(),
            platform: "Windows".into(),
            last_active: now,
            place: "this device".into(),
            current: true,
        },
        DeviceSession {
            id: "mock-laptop".into(),
            device_name: "Laptop".into(),
            platform: "Windows".into(),
            last_active: now - 3_600_000,
            place: "this device".into(),
            current: false,
        },
    ];
}
pub(super) fn query(
    model: &Model,
    q: crate::settings::Query,
) -> Option<crate::settings::SnapshotData> {
    use crate::settings::{Query as Q, SnapshotData as S};
    Some(match q {
        Q::Roles(g) => S::Roles(
            model
                .settings
                .roles
                .values()
                .filter(|r| r.guild_id == g)
                .cloned()
                .collect(),
        ),
        Q::GuildProfile(g) => S::GuildProfile(model.settings.guild_profiles.get(&g)?.clone()),
        Q::Profile(u) => S::Profile(ProfileDetail {
            profile: model.settings.profiles.get(&u)?.clone(),
            mutual_servers: vec![],
            mutual_friends: vec![],
        }),
        Q::Overwrites(c) => S::Overwrites(
            c,
            model
                .settings
                .overwrites
                .get(&c)
                .cloned()
                .unwrap_or_default(),
        ),
        Q::Notifications => {
            S::Notifications(model.settings.notifications.values().cloned().collect())
        }
        Q::Sessions => S::Sessions(model.settings.sessions.clone()),
    })
}
pub(super) fn command(
    mock: &mut Mock,
    cmd: LaunchCommand,
    events: &mut Vec<Event>,
) -> Result<CommandResult, ApiError> {
    let me = mock.model.me.as_ref().unwrap().user.id;
    match cmd {
        LaunchCommand::RenameEmoji {
            guild_id,
            emoji_id,
            name,
        } => {
            if !valid_emoji_name(&name) {
                return Err(error("Invalid emoji name"));
            }
            let list = mock.model.launch.emoji.entry(guild_id).or_default();
            if let Some(e) = list.iter_mut().find(|e| e.id == emoji_id) {
                e.name = name;
            }
            events.push(Event::GuildEmojiUpdated {
                guild_id,
                emoji: list.clone(),
            });
            Ok(CommandResult::Ack)
        }
        LaunchCommand::DeleteEmoji { guild_id, emoji_id } => {
            let list = mock.model.launch.emoji.entry(guild_id).or_default();
            list.retain(|e| e.id != emoji_id);
            events.push(Event::GuildEmojiUpdated {
                guild_id,
                emoji: list.clone(),
            });
            Ok(CommandResult::Ack)
        }

        LaunchCommand::CreateRole { guild_id, name } => {
            let id = mock.id();
            for r in mock
                .model
                .settings
                .roles
                .values_mut()
                .filter(|r| r.guild_id == guild_id && r.id != guild_id)
            {
                r.position += 1;
            }
            let r = Role {
                id,
                guild_id,
                name,
                color: 0,
                position: 1,
                hoist: false,
                mentionable: false,
                permissions: Permissions::empty(),
            };
            mock.model.settings.roles.insert(id, r.clone());
            events.push(role_event(mock, guild_id));
            Ok(CommandResult::Role(r))
        }
        LaunchCommand::UpdateRole { role } => {
            let guild = role.guild_id;
            mock.model.settings.roles.insert(role.id, role.clone());
            events.push(role_event(mock, guild));
            Ok(CommandResult::Role(role))
        }
        LaunchCommand::DeleteRole { guild_id, role_id } => {
            if role_id == guild_id {
                return Err(error("The default role cannot be deleted."));
            }
            mock.model.settings.roles.remove(&role_id);
            for m in mock.model.members.values_mut() {
                m.role_ids.retain(|id| *id != role_id);
            }
            events.push(role_event(mock, guild_id));
            Ok(CommandResult::Ack)
        }
        LaunchCommand::ReorderRoles { guild_id, role_ids } => {
            for (pos, id) in role_ids.into_iter().enumerate() {
                if let Some(r) = mock.model.settings.roles.get_mut(&id) {
                    r.position = pos as u16;
                }
            }
            events.push(role_event(mock, guild_id));
            Ok(CommandResult::Ack)
        }
        LaunchCommand::SetMemberRoles {
            guild_id,
            user_id,
            role_ids,
        } => {
            let m = mock
                .model
                .members
                .get_mut(&user_id)
                .ok_or_else(|| error("Member not found"))?;
            m.role_ids = role_ids;
            if !m.role_ids.contains(&guild_id) {
                m.role_ids.push(guild_id);
            }
            events.push(Event::MemberUpsert(m.clone()));
            Ok(CommandResult::Member(m.clone()))
        }
        LaunchCommand::SetOverwrites {
            channel_id,
            overwrites,
        } => {
            mock.model
                .settings
                .overwrites
                .insert(channel_id, overwrites);
            Ok(CommandResult::Ack)
        }
        LaunchCommand::UpdateGuildProfile {
            guild_id,
            description,
            banner_color,
        } => {
            let p = &mut mock
                .model
                .settings
                .guild_profiles
                .get_mut(&guild_id)
                .ok_or_else(|| error("Server not found"))?
                .profile;
            p.description = description;
            p.banner_color = banner_color;
            events.push(Event::GuildProfileUpdated(p.clone()));
            Ok(CommandResult::Ack)
        }
        LaunchCommand::UpdateUserProfile {
            banner_color,
            pronouns,
            about_me,
        } => {
            let p = mock
                .model
                .settings
                .profiles
                .get_mut(&me)
                .ok_or_else(|| error("Profile not found"))?;
            p.banner_color = banner_color;
            p.pronouns = pronouns;
            p.about_me = about_me;
            events.push(Event::ProfileUpdated(p.clone()));
            Ok(CommandResult::Ack)
        }
        LaunchCommand::SetNotifications(s) => {
            events.push(Event::NotificationSettingsUpdated(s.clone()));
            Ok(CommandResult::NotificationSettings(s))
        }
        LaunchCommand::LogoutSession { session_id } => {
            mock.model.settings.sessions.retain(|s| s.id != session_id);
            events.push(Event::SessionsUpdated);
            Ok(CommandResult::Ack)
        }
        LaunchCommand::LogoutAllOthers => {
            mock.model.settings.sessions.retain(|s| s.current);
            events.push(Event::SessionsUpdated);
            Ok(CommandResult::Ack)
        }
        _ => Err(error("This action is not available in mock mode.")),
    }
}
fn role_event(mock: &Mock, guild_id: Id) -> Event {
    Event::RolesUpdated {
        guild_id,
        roles: mock
            .model
            .settings
            .roles
            .values()
            .filter(|r| r.guild_id == guild_id)
            .cloned()
            .collect(),
    }
}
