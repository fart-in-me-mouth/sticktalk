//! Deterministic launch fixtures; never connect to an external service.
use crate::{
    launch::{Read, SnapshotData},
    model::Model,
};
use sticktalk_proto::*;
pub fn seed(model: &mut Model, now: u64) {
    let guild = Id(10);
    let member = Permissions::VIEW_CHANNEL
        | Permissions::SEND_MESSAGES
        | Permissions::READ_HISTORY
        | Permissions::ATTACH_FILES
        | Permissions::ADD_REACTIONS
        | Permissions::CREATE_INVITES
        | Permissions::CHANGE_NICKNAME
        | Permissions::EMBED_LINKS
        | Permissions::CONNECT
        | Permissions::SPEAK
        | Permissions::STREAM
        | Permissions::VIDEO
        | Permissions::USE_VOICE_ACTIVITY;
    for g in model.guilds.values_mut() {
        g.my_permissions = if Some(g.guild.owner_id) == model.me.as_ref().map(|m| m.user.id) {
            Permissions::all()
        } else {
            member
        };
    }

    model.launch.roles.insert(
        guild,
        vec![
            Role {
                id: guild,
                guild_id: guild,
                name: "@everyone".into(),
                color: 0,
                position: 0,
                hoist: false,
                mentionable: false,
                permissions: member,
            },
            Role {
                id: Id(90),
                guild_id: guild,
                name: "Moderators".into(),
                color: 0x79b8ff,
                position: 2,
                hoist: true,
                mentionable: true,
                permissions: Permissions::MANAGE_MESSAGES | Permissions::MANAGE_ROLES,
            },
            Role {
                id: Id(91),
                guild_id: guild,
                name: "Makers".into(),
                color: 0xe9ad72,
                position: 1,
                hoist: false,
                mentionable: true,
                permissions: Permissions::empty(),
            },
        ],
    );
    for m in model.members.values_mut() {
        m.role_ids = vec![guild];
        if m.user.id.0 <= 2 {
            m.role_ids.push(Id(90));
        } else if m.user.id == Id(3) {
            m.role_ids.push(Id(91));
        }
        if m.user.id == Id(2) {
            m.nickname = Some("Mika".into());
        }
        model.launch.member(m);
    }
    for (id, kind, name, parent, pos) in [
        (18, ChannelKind::Category, "Voice Channels", None, 2),
        (19, ChannelKind::Voice, "Lounge", Some(Id(18)), 0),
        (22, ChannelKind::Text, "moderators", Some(Id(11)), 3),
    ] {
        model.channels.insert(
            Id(id),
            ChannelSummary {
                id: Id(id),
                kind,
                guild_id: Some(guild),
                parent_id: parent,
                name: Some(name.into()),
                position: pos,
                recipient_ids: vec![],
                owner_id: None,
            },
        );
    }
    if let Some(c) = model.channels.get_mut(&Id(11)) {
        c.name = Some("Text Channels".into());
    }
    if let Some(c) = model.channels.get_mut(&Id(15)) {
        c.name = Some("Projects".into());
    }
    model.launch.overwrites.insert(
        Id(22),
        vec![PermissionOverwrite {
            kind: OverwriteKind::Role,
            target_id: guild,
            allow: Permissions::empty(),
            deny: Permissions::VIEW_CHANNEL,
        }],
    );
    model.launch.notification(NotificationSettings {
        guild_id: Some(guild),
        channel_id: Some(Id(17)),
        level: NotificationLevel::Nothing,
        muted: true,
        mute_until: None,
        suppress_everyone: false,
        hide_muted_channels: false,
    });
    for id in [1, 2, 3, 4] {
        let session = VoiceSession {
            user_id: Id(id),
            connection_id: Id(500 + id),
        };
        model.launch.voice.insert(
            session,
            GuildVoiceState {
                guild_id: guild,
                channel_id: Id(19),
                participant: VoiceParticipant {
                    session,
                    muted: id == 3,
                    deafened: id == 4,
                    tracks: if id == 2 {
                        vec![
                            PublishedTrack {
                                mid: "screen".into(),
                                kind: TrackKind::Screen,
                                codec: Some(VideoCodec::H264),
                                layers: vec![],
                                surface: Some(ShareSurface::Monitor),
                            },
                            PublishedTrack {
                                mid: "camera".into(),
                                kind: TrackKind::Camera,
                                codec: Some(VideoCodec::H264),
                                layers: vec![],
                                surface: None,
                            },
                        ]
                    } else {
                        vec![]
                    },
                },
            },
        );
    }
    model.launch.speaking.insert(VoiceSession {
        user_id: Id(2),
        connection_id: Id(502),
    });
    model.launch.rtt_ms = Some(24);
    if let Some(m) = model
        .messages
        .get(&Id(12))
        .and_then(|ms| ms.values().rev().nth(2))
        .cloned()
    {
        model.launch.pins.insert(Id(12), vec![m]);
    }
    for u in model.users.values() {
        model.launch.profiles.insert(
            u.id,
            ProfileDetail {
                profile: UserProfile {
                    user: u.clone(),
                    banner_color: if u.id == Id(2) { 0x315c64 } else { 0x4a426b },
                    pronouns: if u.id == Id(2) {
                        "she/her".into()
                    } else {
                        String::new()
                    },
                    about_me: "Building the next release. Ask me about the design system.".into(),
                    created_at: now.saturating_sub(400 * 86_400_000),
                },
                mutual_servers: model
                    .guilds
                    .values()
                    .take(2)
                    .map(|g| MutualServer {
                        guild: g.guild.clone(),
                        member_since: now.saturating_sub(120 * 86_400_000),
                    })
                    .collect(),
                mutual_friends: model
                    .users
                    .values()
                    .filter(|v| v.id != u.id && v.id != Id(1))
                    .take(2)
                    .cloned()
                    .collect(),
            },
        );
    }
}
pub fn read(model: &Model, read: Read) -> Option<SnapshotData> {
    let cursor = model.cursor.clone()?;
    Some(match read {
        Read::Folders => SnapshotData::Folders(Snapshot {
            data: model.launch.server_folders.clone(),
            cursor,
        }),
        Read::Slowmode(id) => SnapshotData::Slowmode(Snapshot {
            data: model
                .launch
                .slowmode
                .get(&id)
                .cloned()
                .unwrap_or(SlowmodeStatus {
                    channel_id: id,
                    seconds: 0,
                    bypassed: false,
                    retry_after_ms: 0,
                    server_time: 0,
                }),
            cursor,
        }),
        Read::Emoji(id) => SnapshotData::Emoji(
            id,
            Snapshot {
                data: model.launch.emoji.get(&id).cloned().unwrap_or_default(),
                cursor: cursor.clone(),
            },
        ),
        Read::Profile(id) => SnapshotData::Profile(Snapshot {
            cursor,
            data: model.launch.profiles.get(&id)?.clone(),
        }),
        Read::Pins(id) => SnapshotData::Pins(
            id,
            Snapshot {
                cursor,
                data: model.launch.pins.get(&id).cloned().unwrap_or_default(),
            },
        ),
        Read::Overwrites(id) => SnapshotData::Overwrites(
            id,
            Snapshot {
                cursor,
                data: model
                    .launch
                    .overwrites
                    .get(&id)
                    .cloned()
                    .unwrap_or_default(),
            },
        ),
        Read::Permissions(id) => SnapshotData::Permissions(
            id,
            Snapshot {
                cursor,
                data: crate::launch::permissions(model, id),
            },
        ),
        Read::Extent(id) => {
            let ms = model.messages.get(&id);
            let n = ms.map_or(0, |m| m.len());
            let ids: Vec<_> = ms.into_iter().flat_map(|m| m.keys()).copied().collect();
            let anchors = (0..n.min(65))
                .map(|i| {
                    let rank = if n <= 1 {
                        0
                    } else {
                        (n - 1) * i / (n.min(65) - 1)
                    };
                    HistoryAnchor {
                        rank: rank as u64,
                        message_id: ids[rank],
                    }
                })
                .collect();
            SnapshotData::Extent(Snapshot {
                cursor,
                data: HistoryExtent {
                    channel_id: id,
                    message_count: n as u64,
                    anchors,
                },
            })
        }
    })
}
