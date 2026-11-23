use serde::{Deserialize, Deserializer, Serialize, Serializer};

bitflags::bitflags! {
    /// Stable numeric u64 wire mask (including JSON); unused bits are reserved.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Permissions: u64 {
        const VIEW_CHANNEL = 1 << 0;
        const SEND_MESSAGES = 1 << 1;
        const READ_HISTORY = 1 << 2;
        const ATTACH_FILES = 1 << 3;
        const ADD_REACTIONS = 1 << 4;
        const MANAGE_MESSAGES = 1 << 5;
        const MANAGE_CHANNELS = 1 << 6;
        const MANAGE_GUILD = 1 << 7;
        const CREATE_INVITES = 1 << 8;
        const KICK_MEMBERS = 1 << 9;
        const BAN_MEMBERS = 1 << 10;
        const MANAGE_MEMBERS = 1 << 11;
        const ADMINISTRATOR = 1 << 12;
        const OWNER = 1 << 13;
        const MANAGE_ROLES = 1 << 14;
        const CHANGE_NICKNAME = 1 << 15;
        const MANAGE_NICKNAMES = 1 << 16;
        const EMBED_LINKS = 1 << 17;
        const MENTION_EVERYONE = 1 << 18;
        const CONNECT = 1 << 19;
        const SPEAK = 1 << 20;
        const STREAM = 1 << 21;
        const VIDEO = 1 << 22;
        const MUTE_MEMBERS = 1 << 23;
        const DEAFEN_MEMBERS = 1 << 24;
        const MOVE_MEMBERS = 1 << 25;
        const USE_VOICE_ACTIVITY = 1 << 26;
        const PRIORITY_SPEAKER = 1 << 27;
        const MANAGE_EMOJI = 1 << 28;
        const VIEW_AUDIT_LOG = 1 << 29;
        /// Time out members (Discord's "Moderate Members").
        const MODERATE_MEMBERS = 1 << 30;
        /// Play soundboard sounds in a guild voice channel (Discord's "Use Soundboard").
        /// High bits keep clear of other additions to this list.
        const USE_SOUNDBOARD = 1 << 40;
        /// Upload, rename and delete a server's soundboard sounds.
        const MANAGE_SOUNDBOARD = 1 << 41;
        // Threads, forums and announcements (`threads.rs`).
        const CREATE_PUBLIC_THREADS = 1 << 32;
        const CREATE_PRIVATE_THREADS = 1 << 33;
        const SEND_MESSAGES_IN_THREADS = 1 << 34;
        const MANAGE_THREADS = 1 << 35;
        /// Publish your own messages in an announcement channel (Manage
        /// Messages publishes anyone's).
        const PUBLISH_MESSAGES = 1 << 36;
        /// Use custom emoji from other servers here (granted to @everyone by default).
        const USE_EXTERNAL_EMOJI = 1 << 44;
        /// Send stickers from other servers here (granted to @everyone by default).
        const USE_EXTERNAL_STICKERS = 1 << 45;
        /// Send messages read aloud to the people viewing the channel (`/tts`,
        /// `themes.rs`); on for @everyone by default.
        const SEND_TTS_MESSAGES = 1 << 56;
    }
}

impl Permissions {
    /// Legacy pre-role voice wire preset, never an assignable role mask.
    /// Bit 14 here is the old wire Connect position; canonical Connect is bit 19.
    pub const MEMBER: Self = Self::from_bits_retain(
        Self::VIEW_CHANNEL.bits()
            | Self::SEND_MESSAGES.bits()
            | Self::READ_HISTORY.bits()
            | Self::ATTACH_FILES.bits()
            | Self::ADD_REACTIONS.bits()
            | Self::CREATE_INVITES.bits()
            | (1 << 14),
    );

    pub const ADMIN: Self = Self::from_bits_retain(
        Self::MEMBER.bits()
            | Self::MANAGE_MESSAGES.bits()
            | Self::MANAGE_CHANNELS.bits()
            | Self::MANAGE_GUILD.bits()
            | Self::KICK_MEMBERS.bits()
            | Self::BAN_MEMBERS.bits()
            | Self::MANAGE_MEMBERS.bits()
            | Self::ADMINISTRATOR.bits(),
    );

    pub const GUILD_OWNER: Self = Self::from_bits_retain(Self::ADMIN.bits() | Self::OWNER.bits());
}

impl Serialize for Permissions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.bits())
    }
}

impl<'de> Deserialize<'de> for Permissions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u64::deserialize(deserializer).map(Self::from_bits_retain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_include_only_their_defined_rights() {
        assert!(Permissions::GUILD_OWNER.contains(Permissions::ADMIN));
        assert!(Permissions::ADMIN.contains(Permissions::MEMBER));
        assert!(!Permissions::ADMIN.contains(Permissions::OWNER));
        assert!(!Permissions::MEMBER.contains(Permissions::MANAGE_MESSAGES));
    }

    #[test]
    fn unknown_bits_survive_messagepack_without_granting_known_rights() {
        let unknown = Permissions::from_bits_retain(1 << 60);
        let bytes = rmp_serde::to_vec_named(&unknown).unwrap();
        let decoded: Permissions = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded, unknown);
        assert!(!decoded.intersects(Permissions::GUILD_OWNER));
    }
}
