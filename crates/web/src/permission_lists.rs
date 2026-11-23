//! The rights offered by the role editor and the channel permission editor
//! (task `qa2`: kept here, outside the wasm-only `live` module, so a native
//! test checks them against the protocol).
use sticktalk_proto::{ChannelKind, Permissions};

/// Every right a role can hold, by group (the role editor). Tested against
/// the protocol: every assignable bit is here (`role_editor_lists_every_bit`).
pub(crate) const ROLE_PERMISSIONS: &[(&str, &[(&str, Permissions)])] = &[
    (
        "General",
        &[
            ("View Channels", Permissions::VIEW_CHANNEL),
            ("Read History", Permissions::READ_HISTORY),
            ("Manage Server", Permissions::MANAGE_GUILD),
            ("Manage Channels", Permissions::MANAGE_CHANNELS),
            ("Manage Roles", Permissions::MANAGE_ROLES),
            ("Create Invites", Permissions::CREATE_INVITES),
            ("Manage Webhooks", Permissions::MANAGE_WEBHOOKS),
            ("Manage Expressions", Permissions::MANAGE_EMOJI),
            ("Change Nickname", Permissions::CHANGE_NICKNAME),
            ("Manage Nicknames", Permissions::MANAGE_NICKNAMES),
        ],
    ),
    (
        "Text",
        &[
            ("Send Messages", Permissions::SEND_MESSAGES),
            ("Attach Files", Permissions::ATTACH_FILES),
            ("Embed Links", Permissions::EMBED_LINKS),
            ("Add Reactions", Permissions::ADD_REACTIONS),
            (
                "Use Application Commands",
                Permissions::USE_APPLICATION_COMMANDS,
            ),
            (
                "Mention @everyone, @here and All Roles",
                Permissions::MENTION_EVERYONE,
            ),
            ("Use External Emoji", Permissions::USE_EXTERNAL_EMOJI),
            ("Use External Stickers", Permissions::USE_EXTERNAL_STICKERS),
            (
                "Send Text-to-Speech Messages",
                Permissions::SEND_TTS_MESSAGES,
            ),
            ("Manage Messages", Permissions::MANAGE_MESSAGES),
            ("Publish Messages", Permissions::PUBLISH_MESSAGES),
        ],
    ),
    (
        "Threads",
        &[
            ("Create Public Threads", Permissions::CREATE_PUBLIC_THREADS),
            (
                "Create Private Threads",
                Permissions::CREATE_PRIVATE_THREADS,
            ),
            (
                "Send Messages in Threads",
                Permissions::SEND_MESSAGES_IN_THREADS,
            ),
            ("Manage Threads", Permissions::MANAGE_THREADS),
        ],
    ),
    (
        "Voice & Video",
        &[
            ("Connect", Permissions::CONNECT),
            ("Speak", Permissions::SPEAK),
            ("Share Screen", Permissions::STREAM),
            ("Video", Permissions::VIDEO),
            ("Voice Activity", Permissions::USE_VOICE_ACTIVITY),
            ("Use Soundboard", Permissions::USE_SOUNDBOARD),
            ("Manage Soundboard", Permissions::MANAGE_SOUNDBOARD),
            ("Use Activities", Permissions::USE_ACTIVITIES),
            ("Request to Speak", Permissions::REQUEST_TO_SPEAK),
            ("Priority Speaker", Permissions::PRIORITY_SPEAKER),
            ("Mute Members", Permissions::MUTE_MEMBERS),
            ("Deafen Members", Permissions::DEAFEN_MEMBERS),
            ("Move Members", Permissions::MOVE_MEMBERS),
        ],
    ),
    (
        "Events",
        &[
            ("Create Events", Permissions::CREATE_EVENTS),
            ("Manage Events", Permissions::MANAGE_EVENTS),
        ],
    ),
    (
        "Moderation",
        &[
            ("Kick Members", Permissions::KICK_MEMBERS),
            ("Ban Members", Permissions::BAN_MEMBERS),
            ("Time Out Members", Permissions::MODERATE_MEMBERS),
            ("View Audit Log", Permissions::VIEW_AUDIT_LOG),
            ("Administrator", Permissions::ADMINISTRATOR),
        ],
    ),
];

pub(crate) const CHANNEL_PERMISSIONS: &[(&str, &[(&str, Permissions)])] = &[
    (
        "General",
        &[
            ("View Channel", Permissions::VIEW_CHANNEL),
            ("Manage Channel", Permissions::MANAGE_CHANNELS),
            ("Manage Permissions", Permissions::MANAGE_ROLES),
            ("Create Invites", Permissions::CREATE_INVITES),
            ("Manage Webhooks", Permissions::MANAGE_WEBHOOKS),
        ],
    ),
    (
        "Text",
        &[
            ("Send Messages", Permissions::SEND_MESSAGES),
            ("Read Message History", Permissions::READ_HISTORY),
            ("Attach Files", Permissions::ATTACH_FILES),
            ("Embed Links", Permissions::EMBED_LINKS),
            ("Add Reactions", Permissions::ADD_REACTIONS),
            (
                "Use Application Commands",
                Permissions::USE_APPLICATION_COMMANDS,
            ),
            ("Use External Emoji", Permissions::USE_EXTERNAL_EMOJI),
            ("Use External Stickers", Permissions::USE_EXTERNAL_STICKERS),
            (
                "Send Text-to-Speech Messages",
                Permissions::SEND_TTS_MESSAGES,
            ),
            ("Mention @everyone and Roles", Permissions::MENTION_EVERYONE),
            ("Manage Messages", Permissions::MANAGE_MESSAGES),
            ("Publish Messages", Permissions::PUBLISH_MESSAGES),
        ],
    ),
    (
        "Threads",
        &[
            ("Create Public Threads", Permissions::CREATE_PUBLIC_THREADS),
            (
                "Create Private Threads",
                Permissions::CREATE_PRIVATE_THREADS,
            ),
            (
                "Send Messages in Threads",
                Permissions::SEND_MESSAGES_IN_THREADS,
            ),
            ("Manage Threads", Permissions::MANAGE_THREADS),
        ],
    ),
    (
        "Voice",
        &[
            ("Connect", Permissions::CONNECT),
            ("Speak", Permissions::SPEAK),
            ("Video", Permissions::VIDEO),
            ("Share Screen", Permissions::STREAM),
            ("Voice Activity", Permissions::USE_VOICE_ACTIVITY),
            ("Use Soundboard", Permissions::USE_SOUNDBOARD),
            ("Use Activities", Permissions::USE_ACTIVITIES),
            ("Priority Speaker", Permissions::PRIORITY_SPEAKER),
            ("Mute Members", Permissions::MUTE_MEMBERS),
            ("Deafen Members", Permissions::DEAFEN_MEMBERS),
            ("Move Members", Permissions::MOVE_MEMBERS),
            ("Request to Speak", Permissions::REQUEST_TO_SPEAK),
        ],
    ),
];
/// Whether a channel permission means anything for a channel of this kind.
/// Server-wide rights (Manage Soundboard, View Audit Log, Time Out Members,
/// Manage Server...) are only in role settings, like Discord.
pub(crate) fn applies(bit: Permissions, kind: ChannelKind) -> bool {
    use ChannelKind::*;
    let voice = Permissions::CONNECT
        | Permissions::SPEAK
        | Permissions::VIDEO
        | Permissions::STREAM
        | Permissions::USE_VOICE_ACTIVITY
        | Permissions::USE_SOUNDBOARD
        | Permissions::PRIORITY_SPEAKER
        | Permissions::MUTE_MEMBERS
        | Permissions::DEAFEN_MEMBERS
        | Permissions::MOVE_MEMBERS;
    // Threads are offered in text and announcement channels, posts in forums;
    // not in voice-channel chat.
    let threads = Permissions::CREATE_PUBLIC_THREADS
        | Permissions::CREATE_PRIVATE_THREADS
        | Permissions::SEND_MESSAGES_IN_THREADS
        | Permissions::MANAGE_THREADS;
    match kind {
        Category => true,
        _ if bit == Permissions::PUBLISH_MESSAGES => kind == Announcement,
        // Raising a hand is a Stage's (the server checks it per channel).
        _ if bit == Permissions::REQUEST_TO_SPEAK => kind == Stage,
        // Activities run in voice channels, not on a Stage.
        _ if bit == Permissions::USE_ACTIVITIES => kind == Voice,
        // Webhooks live in text, announcement and forum channels (and their
        // threads), as the server allows (`apps/webhooks.rs` KINDS).
        _ if bit == Permissions::MANAGE_WEBHOOKS => matches!(kind, Text | Announcement | Forum),
        _ if threads.contains(bit) => matches!(kind, Text | Announcement | Forum),
        // A Stage is a voice channel with roles: every voice right applies.
        _ if voice.contains(bit) => matches!(kind, Voice | Stage),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rights only a role can carry (they are about the server, not a channel).
    const SERVER_WIDE: Permissions = Permissions::MANAGE_GUILD
        .union(Permissions::MANAGE_EMOJI)
        .union(Permissions::CHANGE_NICKNAME)
        .union(Permissions::MANAGE_NICKNAMES)
        .union(Permissions::MANAGE_SOUNDBOARD)
        .union(Permissions::CREATE_EVENTS)
        .union(Permissions::MANAGE_EVENTS)
        .union(Permissions::KICK_MEMBERS)
        .union(Permissions::BAN_MEMBERS)
        .union(Permissions::MODERATE_MEMBERS)
        .union(Permissions::VIEW_AUDIT_LOG)
        .union(Permissions::ADMINISTRATOR);
    /// Never assignable: the owner flag and the retired member-management bit.
    const NOT_ASSIGNABLE: Permissions = Permissions::OWNER.union(Permissions::MANAGE_MEMBERS);

    fn all(list: &[(&str, &[(&str, Permissions)])]) -> Permissions {
        list.iter()
            .flat_map(|(_, bits)| bits.iter())
            .fold(Permissions::empty(), |a, (_, b)| a | *b)
    }

    #[test]
    fn role_editor_lists_every_bit() {
        let missing = Permissions::all() - NOT_ASSIGNABLE - all(ROLE_PERMISSIONS);
        assert!(missing.is_empty(), "not in the role editor: {missing:?}");
        let n: usize = ROLE_PERMISSIONS.iter().map(|(_, b)| b.len()).sum();
        assert_eq!(
            n as u32,
            all(ROLE_PERMISSIONS).bits().count_ones(),
            "listed twice"
        );
    }

    #[test]
    fn channel_editor_lists_every_channel_right_where_it_applies() {
        let channel = all(ROLE_PERMISSIONS) - SERVER_WIDE;
        let missing = channel - all(CHANNEL_PERMISSIONS);
        assert!(missing.is_empty(), "not in the channel editor: {missing:?}");
        assert!((all(CHANNEL_PERMISSIONS) & SERVER_WIDE).is_empty());
        use ChannelKind::*;
        for bit in [
            Permissions::CONNECT,
            Permissions::MUTE_MEMBERS,
            Permissions::USE_SOUNDBOARD,
        ] {
            assert!(applies(bit, Voice) && applies(bit, Stage) && !applies(bit, Text));
        }
        assert!(applies(Permissions::REQUEST_TO_SPEAK, Stage));
        assert!(!applies(Permissions::REQUEST_TO_SPEAK, Voice));
        assert!(applies(Permissions::CREATE_PUBLIC_THREADS, Forum));
        assert!(!applies(Permissions::CREATE_PUBLIC_THREADS, Voice));
        // Webhooks where one can live; commands wherever there is a composer.
        for kind in [Text, Announcement, Forum, Category] {
            assert!(applies(Permissions::MANAGE_WEBHOOKS, kind), "{kind:?}");
        }
        for kind in [Voice, Stage] {
            assert!(!applies(Permissions::MANAGE_WEBHOOKS, kind), "{kind:?}");
        }
        for kind in [Text, Announcement, Forum, Voice, Stage, Category] {
            assert!(
                applies(Permissions::USE_APPLICATION_COMMANDS, kind),
                "{kind:?}"
            );
        }
    }
}
