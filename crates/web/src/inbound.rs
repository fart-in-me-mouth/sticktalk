//! Server frames, decoded only as far as the core needs (docs/features/perf.md).
//!
//! The socket used to decode every frame into `ServerMsg`. serde's derived
//! decoder for that enum contains every event and command result of every
//! feature, twice (adjacently tagged enums also get a buffered decoder for a
//! "data before type" order the server never sends). It was most of the core.
//!
//! Here the frame is walked with a small MessagePack reader that only finds
//! the `type` tags and the byte ranges of the `data` values. The core then
//! decodes the payloads of its own events with their own types. A feature's
//! event or command result is passed on as the bytes of its `data` value,
//! undecoded; the feature's lazily loaded module decodes it (`features.rs`).
//! The wire format is unchanged.
// Natively this module is built for its tests only; the app is live, wasm-only.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
use serde::{de::DeserializeOwned, Deserialize};
use sticktalk_proto::*;

/// The features that live in their own WASM module (`live/features.rs`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feature {
    Content,
    Threads,
    Expression,
    Moderation,
    Privacy,
    Nav,
    Profiles,
    Authsec,
    /// Rules screening, applications, onboarding, Discover, templates, insights.
    Community,
    /// Stage channels and scheduled events.
    Stage,
    /// Themes, appearance, text-to-speech.
    Themes,
}

/// What one server frame says, as far as the core reads it.
pub enum Inbound {
    Hello(Hello),
    Ready(Box<Ready>),
    Resumed,
    ResyncRequired,
    HeartbeatAck,
    /// Sequence number, the event, and its encoded size (bounds the journal).
    Event(EventSeq, Routed<Event>, usize),
    Response(Response),
    Error(ApiError),
    Voice(VoicePayload),
}

/// A core value, or a feature's payload for its module to decode.
pub enum Routed<T> {
    Core(T),
    Feature(Feature, Raw),
}

/// The undecoded `data` of a feature's event or command result, with its tag
/// (for example `"content"`, or `"saved_updated"` for a navigation event).
#[derive(Clone)]
pub struct Raw {
    pub tag: &'static str,
    pub data: Vec<u8>,
}

pub struct Response {
    pub nonce: Nonce,
    pub outcome: Result<Routed<CommandResult>, ApiError>,
}

type Fail = ();

/// Decode exactly `bytes` (one MessagePack value) as a `T`.
///
/// The value is rewritten as JSON and read with `serde_json::from_slice`, the
/// decoder the REST client already uses. A type then has one decoder in the
/// whole app, shared by the socket and REST, and serde_json's derived
/// decoders are several times smaller than rmp_serde's. Features decode their
/// payloads with this too.
#[inline(never)]
pub fn de<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Fail> {
    let json = to_json(bytes)?;
    serde_json::from_slice(&json).map_err(drop)
}

/// One MessagePack value as JSON text: maps keep their order, integer map
/// keys become decimal strings (as serde_json writes them), byte strings
/// become arrays of numbers (as serde reads `Vec<u8>`). Nesting is limited
/// like the socket's decoder was (32 levels).
#[inline(never)]
pub fn to_json(bytes: &[u8]) -> Result<Vec<u8>, Fail> {
    let mut r = Reader { b: bytes, p: 0 };
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 2 + 16);
    r.json(&mut out, 32)?;
    if r.p != bytes.len() {
        return Err(());
    }
    Ok(out)
}

/// Read a whole server frame.
#[inline(never)]
pub fn frame(bytes: &[u8]) -> Result<Inbound, Fail> {
    if bytes.len() > MAX_SERVER_FRAME_BYTES as usize {
        return Err(());
    }
    let mut r = Reader { b: bytes, p: 0 };
    let (tag, data) = r.tagged("type", "data")?;
    if r.p != bytes.len() {
        return Err(());
    }
    let data = data.unwrap_or_default();
    Ok(match tag {
        "hello" => Inbound::Hello(de(data)?),
        "ready" => Inbound::Ready(de(data)?),
        "resumed" => {
            de::<Resumed>(data)?;
            Inbound::Resumed
        }
        "resync_required" => {
            de::<ResyncRequired>(data)?;
            Inbound::ResyncRequired
        }
        "heartbeat_ack" => {
            de::<HeartbeatAck>(data)?;
            Inbound::HeartbeatAck
        }
        "error" => Inbound::Error(de(data)?),
        "voice" => Inbound::Voice(de(data)?),
        "event" => {
            let mut r = Reader { b: data, p: 0 };
            let mut seq = None;
            let mut event = None;
            for _ in 0..r.map()? {
                match r.str()? {
                    "seq" if seq.is_none() => seq = Some(r.uint()?),
                    "event" if event.is_none() => event = Some(r.value()?),
                    _ => r.skip()?,
                }
            }
            if r.p != data.len() {
                return Err(());
            }
            let (seq, event) = (seq.ok_or(())?, event.ok_or(())?);
            Inbound::Event(seq, route_event(event)?, event.len())
        }
        "response" => {
            let mut r = Reader { b: data, p: 0 };
            let (mut nonce, mut cursor, mut replayed, mut outcome) = (None, None, None, None);
            for _ in 0..r.map()? {
                match r.str()? {
                    "nonce" if nonce.is_none() => nonce = Some(r.uint()?),
                    "cursor" if cursor.is_none() => cursor = Some(r.value()?),
                    "replayed" if replayed.is_none() => replayed = Some(r.bool()?),
                    "outcome" if outcome.is_none() => outcome = Some(r.value()?),
                    _ => r.skip()?,
                }
            }
            if r.p != data.len() || replayed.is_none() {
                return Err(());
            }
            de::<ResumeCursor>(cursor.ok_or(())?)?;
            let mut r = Reader {
                b: outcome.ok_or(())?,
                p: 0,
            };
            let (status, value) = r.tagged("status", "data")?;
            let value = value.ok_or(())?;
            Inbound::Response(Response {
                nonce: nonce.ok_or(())?,
                outcome: match status {
                    "ok" => Ok(route_result(value)?),
                    "err" => Err(de(value)?),
                    _ => return Err(()),
                },
            })
        }
        _ => return Err(()),
    })
}

/// Generates the decoding of one adjacently tagged enum's core variants, and
/// the tags of the variants that belong to lazily loaded features.
macro_rules! routes {
    (
        $name:ident -> $ty:ident, tags $tags:ident;
        newtype { $($ntag:literal => $nvar:ident($nty:ty),)* }
        fields { $($stag:literal => $svar:ident { $($f:ident: $fty:ty),* },)* }
        unit { $($utag:literal => $uvar:ident,)* }
        features { $($ftag:literal => $feature:ident,)* }
    ) => {
        #[cfg(test)]
        #[allow(dead_code)]
        const $tags: &[&str] = &[$($ntag,)* $($stag,)* $($utag,)* $($ftag,)*];
        #[inline(never)]
        fn $name(bytes: &[u8]) -> Result<Routed<$ty>, Fail> {
            let mut r = Reader { b: bytes, p: 0 };
            let (tag, data) = r.tagged("type", "data")?;
            if r.p != bytes.len() {
                return Err(());
            }
            let data = || data.ok_or(());
            Ok(Routed::Core(match tag {
                $($ntag => $ty::$nvar(de::<$nty>(data()?)?),)*
                $($stag => {
                    #[derive(Deserialize)]
                    struct Fields { $($f: $fty),* }
                    let Fields { $($f),* } = de(data()?)?;
                    $ty::$svar { $($f),* }
                })*
                $($utag => $ty::$uvar,)*
                $($ftag => return Ok(Routed::Feature(Feature::$feature, Raw {
                    tag: $ftag,
                    data: data()?.to_vec(),
                })),)*
                _ => return Err(()),
            }))
        }
    };
}

routes! {
    route_event -> Event, tags EVENT_TAGS;
    newtype {
        "server_folders_updated" => ServerFoldersUpdated(Vec<ServerFolder>),
        "profile_updated" => ProfileUpdated(UserProfile),
        "guild_profile_updated" => GuildProfileUpdated(GuildProfile),
        "notification_settings_updated" => NotificationSettingsUpdated(NotificationSettings),
        "self_updated" => SelfUpdated(SelfUser),
        "user_updated" => UserUpdated(UserSummary),
        "relationship_upsert" => RelationshipUpsert(Relationship),
        "presence_updated" => PresenceUpdated(Presence),
        "guild_upsert" => GuildUpsert(GuildSummary),
        "member_upsert" => MemberUpsert(Member),
        "invite_upsert" => InviteUpsert(Invite),
        "ban_upsert" => BanUpsert(Ban),
        "channel_upsert" => ChannelUpsert(Channel),
        "message_updated" => MessageUpdated(Message),
        "reaction_updated" => ReactionUpdated(ReactionUpdate),
        "read_state_updated" => ReadStateUpdated(ReadState),
    }
    fields {
        "slowmode_updated" => SlowmodeUpdated { channel_id: ChannelId, seconds: u32 },
        "guild_emoji_updated" => GuildEmojiUpdated { guild_id: GuildId, emoji: Vec<GuildEmoji> },
        "roles_updated" => RolesUpdated { guild_id: GuildId, roles: Vec<Role> },
        "pins_updated" => PinsUpdated { channel_id: ChannelId, message_id: MessageId, pinned: bool },
        "relationship_removed" => RelationshipRemoved { user_id: UserId },
        "guild_joined" => GuildJoined { detail: GuildDetail, read_states: Vec<ReadState> },
        "guild_removed" => GuildRemoved { guild_id: GuildId, reason: RemovalReason },
        "member_removed" => MemberRemoved { guild_id: GuildId, user_id: UserId, reason: RemovalReason },
        "invite_removed" => InviteRemoved { guild_id: GuildId, invite_id: Id },
        "ban_removed" => BanRemoved { guild_id: GuildId, user_id: UserId },
        "channel_removed" => ChannelRemoved { channel_id: ChannelId, reason: RemovalReason },
        "channels_reordered" => ChannelsReordered { guild_id: GuildId, placements: Vec<ChannelPlacement> },
        "message_created" => MessageCreated { message: Message, author: UserSummary },
        "message_deleted" => MessageDeleted { channel_id: ChannelId, message_id: MessageId },
        "typing" => Typing { channel_id: ChannelId, user_id: UserId, expires_at: UnixMillis },
    }
    unit {
        "sessions_updated" => SessionsUpdated,
    }
    features {
        "nav_updated" => Nav,
        "saved_updated" => Nav,
        "reminder_due" => Nav,
        "content" => Content,
        "moderation" => Moderation,
        "data_export_updated" => Authsec,
        "threads" => Threads,
        "expression" => Expression,
        "privacy" => Privacy,
        "community" => Community,
        "schedule" => Stage,
        "appearance" => Themes,
    }
}

routes! {
    route_result -> CommandResult, tags RESULT_TAGS;
    newtype {
        "role" => Role(Role),
        "notification_settings" => NotificationSettings(NotificationSettings),
        "profile" => Profile(SelfUser),
        "presence" => Presence(Presence),
        "relationship" => Relationship(Relationship),
        "channel" => Channel(Channel),
        "guild" => Guild(GuildDetail),
        "member" => Member(Member),
        "invite" => Invite(Invite),
        "message" => Message(Message),
        "reaction" => Reaction(ReactionUpdate),
        "read_state" => ReadState(ReadState),
    }
    fields {
        "subscription" => Subscription { presences: Vec<Presence> },
    }
    unit {
        "ack" => Ack,
    }
    features {
        "content" => Content,
        "moderation" => Moderation,
        "threads" => Threads,
        "expression" => Expression,
        "privacy" => Privacy,
        "community" => Community,
        "schedule" => Stage,
        "appearance" => Themes,
    }
}

/// The tag and content of an adjacently tagged enum (`{"type", "data"}`).
pub fn tagged(bytes: &[u8]) -> Result<(&str, Option<&[u8]>), Fail> {
    let mut r = Reader { b: bytes, p: 0 };
    let tagged = r.tagged("type", "data")?;
    if r.p != bytes.len() {
        return Err(());
    }
    Ok(tagged)
}

/// Decodes an adjacently tagged enum variant by variant: the tag is read
/// first and only that variant's payload is decoded. serde's derived decoder
/// for such an enum also contains a second, buffered decoder of every variant
/// (for content that arrives before its tag, which the server never sends);
/// features use this for their own events and results to leave it out.
/// List every variant: an unknown tag is a decoding error, as with serde.
#[macro_export]
macro_rules! decode_tagged {
    (
        $vis:vis fn $name:ident -> $ty:ident, tags $tags:ident;
        newtype { $($ntag:literal => $nvar:ident($nty:ty),)* }
        fields { $($stag:literal => $svar:ident { $($(#[$m:meta])* $f:ident: $fty:ty),* },)* }
        unit { $($utag:literal => $uvar:ident,)* }
    ) => {
        /// The tags this decoder knows (a test compares them with serde's).
        #[allow(dead_code)]
        $vis const $tags: &[&str] = &[$($ntag,)* $($stag,)* $($utag,)*];
        $vis fn $name(bytes: &[u8]) -> Result<$ty, ()> {
            let (tag, data) = $crate::inbound::tagged(bytes)?;
            let data = || data.ok_or(());
            Ok(match tag {
                $($ntag => $ty::$nvar($crate::inbound::de::<$nty>(data()?)?),)*
                $($stag => {
                    #[derive(serde::Deserialize)]
                    struct Fields { $($(#[$m])* $f: $fty),* }
                    let Fields { $($f),* } = $crate::inbound::de(data()?)?;
                    $ty::$svar { $($f),* }
                })*
                $($utag => $ty::$uvar,)*
                _ => return Err(()),
            })
        }
    };
}

/// Feature enums decoded variant by variant (their state modules call these).
pub mod feature {
    use sticktalk_proto::*;
    crate::decode_tagged! {
        pub fn privacy_event -> PrivacyEvent, tags PRIVACY_EVENT;
        newtype {
            "settings" => Settings(Box<PrivacySettings>),
            "alert" => Alert(Alert),
            "requests" => Requests(RequestCounts),
            "report_updated" => ReportUpdated(MyReport),
            "note" => Note(UserNote),
        }
        fields { "guild_reports" => GuildReports { guild_id: GuildId, open: u32 }, }
        unit {}
    }
    crate::decode_tagged! {
        pub fn privacy_result -> PrivacyResult, tags PRIVACY_RESULT;
        newtype {
            "settings" => Settings(Box<PrivacySettings>),
            "note" => Note(UserNote),
            "filed" => Filed(MyReport),
        }
        fields {}
        unit {}
    }
    crate::decode_tagged! {
        pub fn thread_event -> ThreadEvent, tags THREAD_EVENT;
        newtype {
            "thread_upsert" => ThreadUpsert(ThreadInfo),
            "forum_updated" => ForumUpdated(ForumSettings),
        }
        fields {
            "thread_removed" => ThreadRemoved { thread_id: ChannelId, parent_id: ChannelId },
            "membership" => Membership { thread_id: ChannelId, membership: Option<ThreadMembership> },
            "published" => Published { channel_id: ChannelId, message_id: MessageId },
            "follows_changed" => FollowsChanged { channel_id: ChannelId },
        }
        unit {}
    }
    crate::decode_tagged! {
        pub fn thread_result -> ThreadResult, tags THREAD_RESULT;
        newtype {
            "snapshot" => Snapshot(ThreadSnapshot),
            "thread" => Thread(Box<ThreadInfo>),
            "forum" => Forum(ForumSettings),
            "follow" => Follow(ChannelFollow),
        }
        fields { "post" => Post { thread: Box<ThreadInfo>, message: Box<Message> }, }
        unit { "ack" => Ack, }
    }
    crate::decode_tagged! {
        pub fn expression_event -> ExpressionEvent, tags EXPRESSION_EVENT;
        newtype {
            "super_reaction_updated" => SuperReactionUpdated(ReactionUpdate),
            "settings_updated" => SettingsUpdated(ExpressionSettings),
        }
        fields {
            "stickers_updated" => StickersUpdated { guild_id: GuildId, stickers: Vec<Sticker> },
            "typing_style" => TypingStyle { user_id: UserId, style: TypingStyle },
        }
        unit {}
    }
    crate::decode_tagged! {
        pub fn expression_result -> ExpressionResult, tags EXPRESSION_RESULT;
        newtype {
            "super_reaction" => SuperReaction(Box<ReactionUpdate>),
            "settings" => Settings(Box<ExpressionSettings>),
            "sticker" => Sticker(Box<Sticker>),
        }
        fields {}
        unit { "ack" => Ack, }
    }
    crate::decode_tagged! {
        pub fn content_event -> ContentEvent, tags CONTENT_EVENT;
        newtype { "scheduled_upserted" => ScheduledUpserted(ScheduledMessage), }
        fields {
            "poll_updated" => PollUpdated { channel_id: ChannelId, message_id: MessageId, poll: Poll, #[serde(default)] voter: Option<PollVoter> },
            "embeds_updated" => EmbedsUpdated { channel_id: ChannelId, message_id: MessageId, embeds: Vec<LinkEmbed> },
            "scheduled_removed" => ScheduledRemoved { id: Id, channel_id: ChannelId, #[serde(default)] sent: Option<MessageId> },
        }
        unit {}
    }
    crate::decode_tagged! {
        pub fn content_result -> ContentResult, tags CONTENT_RESULT;
        newtype {
            "forwarded" => Forwarded(Vec<Message>),
            "scheduled" => Scheduled(ScheduledMessage),
        }
        fields { "poll" => Poll { channel_id: ChannelId, message_id: MessageId, poll: Poll }, }
        unit { "ack" => Ack, }
    }
    crate::decode_tagged! {
        pub fn community_event -> CommunityEvent, tags COMMUNITY_EVENT;
        newtype {
            "me" => Me(Box<MemberCommunity>),
            "application" => Application(Box<MyApplication>),
        }
        fields {
            "updated" => Updated { guild_id: GuildId },
            "queue" => Queue { guild_id: GuildId, pending: u32 },
        }
        unit {}
    }
    crate::decode_tagged! {
        pub fn community_result -> CommunityResult, tags COMMUNITY_RESULT;
        newtype {
            "admission" => Admission(Box<Admission>),
            "onboarding" => Onboarding(Box<Onboarding>),
            "me" => Me(Box<MemberCommunity>),
            "application" => Application(Box<MyApplication>),
            "listing" => Listing(Option<Box<Listing>>),
            "template" => Template(Box<GuildTemplate>),
        }
        fields {}
        unit { "ack" => Ack, }
    }
    crate::decode_tagged! {
        pub fn schedule_update -> ScheduleUpdate, tags SCHEDULE_UPDATE;
        newtype {
            "upsert" => Upsert(ScheduledEvent),
            "summary" => Summary(GuildEvents),
            "starting" => Starting(ScheduledEvent),
        }
        fields {
            "removed" => Removed { guild_id: GuildId, event_id: Id },
            "interest" => Interest { event_id: Id, interested: bool },
        }
        unit {}
    }
    crate::decode_tagged! {
        pub fn schedule_result -> ScheduleResult, tags SCHEDULE_RESULT;
        newtype {
            "event" => Event(ScheduledEvent),
            "message" => Message(Message),
            "invite" => Invite(Invite),
        }
        fields { "summary" => Summary { guilds: Vec<GuildEvents> }, }
        unit { "ack" => Ack, }
    }
    crate::decode_tagged! {
        pub fn stage_event -> StageEvent, tags STAGE_EVENT;
        newtype {
            "live" => Live(StageInstance),
            "roster" => Roster(StageRoster),
        }
        fields {
            "ended" => Ended { guild_id: GuildId, channel_id: ChannelId },
            "snapshot" => Snapshot { stages: Vec<StageInstance> },
            "members" => Members { channel_id: ChannelId, upserts: Vec<StageMember>, removed: Vec<UserId> },
        }
        unit {}
    }
    crate::decode_tagged! {
        pub fn appearance_event -> AppearanceEvent, tags APPEARANCE_EVENT;
        newtype { "library" => Library(Box<ThemeLibrary>), }
        fields {
            "server_theme" => ServerTheme { guild_id: GuildId, #[serde(default)] theme: Option<ThemeChoice> },
            "tts" => Tts { channel_id: ChannelId, message_id: MessageId },
        }
        unit {}
    }
    crate::decode_tagged! {
        pub fn appearance_result -> AppearanceResult, tags APPEARANCE_RESULT;
        newtype { "library" => Library(Box<ThemeLibrary>), }
        fields {}
        unit {}
    }
}

/// Walks MessagePack without decoding it: tags, lengths and value ranges.
/// Iterative, so hostile nesting cannot exhaust the stack.
struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, Fail> {
        let v = *self.b.get(self.p).ok_or(())?;
        self.p += 1;
        Ok(v)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], Fail> {
        let end = self
            .p
            .checked_add(n)
            .filter(|e| *e <= self.b.len())
            .ok_or(())?;
        let v = &self.b[self.p..end];
        self.p = end;
        Ok(v)
    }
    fn be(&mut self, n: usize) -> Result<u64, Fail> {
        Ok(self.take(n)?.iter().fold(0, |v, b| v << 8 | u64::from(*b)))
    }
    fn map(&mut self) -> Result<u64, Fail> {
        match self.byte()? {
            m @ 0x80..=0x8f => Ok(u64::from(m & 0x0f)),
            0xde => self.be(2),
            0xdf => self.be(4),
            _ => Err(()),
        }
    }
    fn str(&mut self) -> Result<&'a str, Fail> {
        let n = match self.byte()? {
            m @ 0xa0..=0xbf => u64::from(m & 0x1f),
            0xd9 => self.be(1)?,
            0xda => self.be(2)?,
            0xdb => self.be(4)?,
            _ => return Err(()),
        };
        std::str::from_utf8(self.take(n as usize)?).map_err(drop)
    }
    fn uint(&mut self) -> Result<u64, Fail> {
        match self.byte()? {
            m @ 0x00..=0x7f => Ok(u64::from(m)),
            0xcc => self.be(1),
            0xcd => self.be(2),
            0xce => self.be(4),
            0xcf => self.be(8),
            _ => Err(()),
        }
    }
    fn bool(&mut self) -> Result<bool, Fail> {
        match self.byte()? {
            0xc2 => Ok(false),
            0xc3 => Ok(true),
            _ => Err(()),
        }
    }
    /// The bytes of the next value.
    fn value(&mut self) -> Result<&'a [u8], Fail> {
        let start = self.p;
        self.skip()?;
        Ok(&self.b[start..self.p])
    }
    fn skip(&mut self) -> Result<(), Fail> {
        let mut pending: u64 = 1;
        while pending > 0 {
            pending -= 1;
            let m = self.byte()?;
            let (skip, items) = match m {
                0x00..=0x7f | 0xc0 | 0xc2 | 0xc3 | 0xe0..=0xff => (0, 0),
                0x80..=0x8f => (0, 2 * u64::from(m & 0x0f)),
                0x90..=0x9f => (0, u64::from(m & 0x0f)),
                0xa0..=0xbf => (u64::from(m & 0x1f), 0),
                0xc4 | 0xd9 => (self.be(1)?, 0),
                0xc5 | 0xda => (self.be(2)?, 0),
                0xc6 | 0xdb => (self.be(4)?, 0),
                0xc7 => (self.be(1)? + 1, 0),
                0xc8 => (self.be(2)? + 1, 0),
                0xc9 => (self.be(4)? + 1, 0),
                0xca => (4, 0),
                0xcb => (8, 0),
                0xcc | 0xd0 => (1, 0),
                0xcd | 0xd1 => (2, 0),
                0xce | 0xd2 => (4, 0),
                0xcf | 0xd3 => (8, 0),
                0xd4 => (2, 0),
                0xd5 => (3, 0),
                0xd6 => (5, 0),
                0xd7 => (9, 0),
                0xd8 => (17, 0),
                0xdc => (0, self.be(2)?),
                0xdd => (0, self.be(4)?),
                0xde => (0, 2 * self.be(2)?),
                0xdf => (0, 2 * self.be(4)?),
                0xc1 => return Err(()),
            };
            self.take(usize::try_from(skip).map_err(drop)?)?;
            pending = pending.checked_add(items).ok_or(())?;
            // Every value takes at least one byte.
            if pending > (self.b.len() - self.p) as u64 {
                return Err(());
            }
        }
        Ok(())
    }
    fn json(&mut self, out: &mut Vec<u8>, depth: u32) -> Result<(), Fail> {
        let depth = depth.checked_sub(1).ok_or(())?;
        let m = self.byte()?;
        match m {
            0x00..=0x7f => number(out, u64::from(m)),
            0xe0..=0xff => signed(out, i64::from(m as i8)),
            0xc0 => out.extend_from_slice(b"null"),
            0xc2 => out.extend_from_slice(b"false"),
            0xc3 => out.extend_from_slice(b"true"),
            0xcc => number(out, self.be(1)?),
            0xcd => number(out, self.be(2)?),
            0xce => number(out, self.be(4)?),
            0xcf => number(out, self.be(8)?),
            0xd0 => signed(out, i64::from(self.be(1)? as u8 as i8)),
            0xd1 => signed(out, i64::from(self.be(2)? as u16 as i16)),
            0xd2 => signed(out, i64::from(self.be(4)? as u32 as i32)),
            0xd3 => signed(out, self.be(8)? as i64),
            0xca => float(out, f64::from(f32::from_bits(self.be(4)? as u32)))?,
            0xcb => float(out, f64::from_bits(self.be(8)?))?,
            0xa0..=0xbf | 0xd9..=0xdb => {
                self.p -= 1;
                string(out, self.str_bytes()?);
            }
            0xc4..=0xc6 => {
                let n = self.be(1 << (m - 0xc4))? as usize;
                out.push(b'[');
                for (i, b) in self.take(n)?.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    number(out, u64::from(*b));
                }
                out.push(b']');
            }
            0x90..=0x9f | 0xdc | 0xdd => {
                let n = match m {
                    0xdc => self.be(2)?,
                    0xdd => self.be(4)?,
                    _ => u64::from(m & 0x0f),
                };
                if n > (self.b.len() - self.p) as u64 {
                    return Err(());
                }
                out.push(b'[');
                for i in 0..n {
                    if i > 0 {
                        out.push(b',');
                    }
                    self.json(out, depth)?;
                }
                out.push(b']');
            }
            0x80..=0x8f | 0xde | 0xdf => {
                self.p -= 1;
                let n = self.map()?;
                if n > (self.b.len() - self.p) as u64 {
                    return Err(());
                }
                out.push(b'{');
                for i in 0..n {
                    if i > 0 {
                        out.push(b',');
                    }
                    match self.b.get(self.p).copied().ok_or(())? {
                        0xa0..=0xbf | 0xd9..=0xdb => string(out, self.str_bytes()?),
                        // An integer key, quoted.
                        0x00..=0x7f | 0xcc..=0xcf | 0xd0..=0xd3 | 0xe0..=0xff => {
                            out.push(b'"');
                            self.json(out, 1)?;
                            out.push(b'"');
                        }
                        _ => return Err(()),
                    }
                    out.push(b':');
                    self.json(out, depth)?;
                }
                out.push(b'}');
            }
            _ => return Err(()),
        }
        Ok(())
    }
    fn str_bytes(&mut self) -> Result<&'a [u8], Fail> {
        let n = match self.byte()? {
            m @ 0xa0..=0xbf => u64::from(m & 0x1f),
            0xd9 => self.be(1)?,
            0xda => self.be(2)?,
            0xdb => self.be(4)?,
            _ => return Err(()),
        };
        self.take(n as usize)
    }
    /// An adjacently tagged enum: its tag, and its content if it has any.
    fn tagged(&mut self, tag: &str, content: &str) -> Result<(&'a str, Option<&'a [u8]>), Fail> {
        let (mut t, mut c) = (None, None);
        for _ in 0..self.map()? {
            let key = self.str()?;
            if key == tag && t.is_none() {
                t = Some(self.str()?);
            } else if key == content && c.is_none() {
                c = Some(self.value()?);
            } else {
                return Err(());
            }
        }
        Ok((t.ok_or(())?, c))
    }
}

fn number(out: &mut Vec<u8>, v: u64) {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    let mut v = v;
    loop {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out.extend_from_slice(&digits[i..]);
}
fn signed(out: &mut Vec<u8>, v: i64) {
    if v < 0 {
        out.push(b'-');
    }
    number(out, v.unsigned_abs());
}
fn float(out: &mut Vec<u8>, v: f64) -> Result<(), Fail> {
    if !v.is_finite() {
        return Err(());
    }
    use std::io::Write;
    write!(out, "{v:?}").map_err(drop)
}
/// A JSON string. Bytes are copied as they are; serde_json checks UTF-8.
fn string(out: &mut Vec<u8>, s: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(b'"');
    for &b in s {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x00..=0x1f => out.extend_from_slice(&[
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX[usize::from(b >> 4)],
                HEX[usize::from(b & 15)],
            ]),
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

/// Which module handles each event: every `Event` variant must be listed,
/// so a new variant does not compile until it is classified (and the round
/// trip test below covers it).
#[cfg(test)]
pub fn owner(e: &Event) -> Option<Feature> {
    use Event::*;
    match e {
        ServerFoldersUpdated(_)
        | SlowmodeUpdated { .. }
        | GuildEmojiUpdated { .. }
        | RolesUpdated { .. }
        | ProfileUpdated(_)
        | GuildProfileUpdated(_)
        | PinsUpdated { .. }
        | NotificationSettingsUpdated(_)
        | SessionsUpdated
        | SelfUpdated(_)
        | UserUpdated(_)
        | RelationshipUpsert(_)
        | RelationshipRemoved { .. }
        | PresenceUpdated(_)
        | GuildUpsert(_)
        | GuildJoined { .. }
        | GuildRemoved { .. }
        | MemberUpsert(_)
        | MemberRemoved { .. }
        | InviteUpsert(_)
        | InviteRemoved { .. }
        | BanUpsert(_)
        | BanRemoved { .. }
        | ChannelUpsert(_)
        | ChannelRemoved { .. }
        | ChannelsReordered { .. }
        | MessageCreated { .. }
        | MessageUpdated(_)
        | MessageDeleted { .. }
        | ReactionUpdated(_)
        | Typing { .. }
        | ReadStateUpdated(_) => None,
        NavUpdated(_) | SavedUpdated(_) | ReminderDue(_) => Some(Feature::Nav),
        Content(_) => Some(Feature::Content),
        Moderation(_) => Some(Feature::Moderation),
        DataExportUpdated(_) => Some(Feature::Authsec),
        Threads(_) => Some(Feature::Threads),
        Expression(_) => Some(Feature::Expression),
        Privacy(_) => Some(Feature::Privacy),
        Community(_) => Some(Feature::Community),
        Schedule(_) => Some(Feature::Stage),
        Appearance(_) => Some(Feature::Themes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encode(msg: &ServerMsg) -> Vec<u8> {
        rmp_serde::to_vec_named(msg).unwrap()
    }
    fn event(seq: u64, event: Event) -> Vec<u8> {
        encode(&ServerMsg::Event(Box::new(EventEnvelope { seq, event })))
    }
    fn core(bytes: &[u8]) -> Event {
        match frame(bytes) {
            Ok(Inbound::Event(_, Routed::Core(e), _)) => e,
            _ => panic!("not a core event"),
        }
    }
    fn user() -> UserSummary {
        UserSummary {
            id: Id(7),
            username: "alex".into(),
            display_name: Some("Alex".into()),
            avatar_id: None,
            status_text: None,
            style: Default::default(),
            status_emoji: Some("🙂".into()),
        }
    }

    /// serde's own variant names of an adjacently tagged enum: the error for
    /// an unknown tag lists every one of them.
    fn serde_tags<T: DeserializeOwned>() -> std::collections::BTreeSet<String> {
        let error = serde_json::from_str::<T>(r#"{"type":"\u0001"}"#)
            .err()
            .expect("an unknown tag is refused")
            .to_string();
        let expected = error
            .split_once("expected")
            .expect("serde lists the variants")
            .1;
        expected
            .split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }
    fn listed(tags: &[&str]) -> std::collections::BTreeSet<String> {
        let set: std::collections::BTreeSet<String> = tags.iter().map(|t| t.to_string()).collect();
        assert_eq!(set.len(), tags.len(), "a tag is listed twice");
        set
    }

    /// The routing tables and the feature decoders list exactly serde's
    /// variants: a variant added to the protocol fails here until it is routed
    /// (to the core or to a feature) and decoded (docs/features/perf.md).
    #[test]
    fn every_variant_is_listed_with_its_serde_tag() {
        use feature::*;
        assert_eq!(
            listed(EVENT_TAGS),
            serde_tags::<Event>(),
            "Event: route_event"
        );
        assert_eq!(
            listed(RESULT_TAGS),
            serde_tags::<CommandResult>(),
            "CommandResult: route_result"
        );
        let decoders: [(&str, &[&str], std::collections::BTreeSet<String>); 15] = [
            ("PrivacyEvent", PRIVACY_EVENT, serde_tags::<PrivacyEvent>()),
            (
                "PrivacyResult",
                PRIVACY_RESULT,
                serde_tags::<PrivacyResult>(),
            ),
            ("ThreadEvent", THREAD_EVENT, serde_tags::<ThreadEvent>()),
            ("ThreadResult", THREAD_RESULT, serde_tags::<ThreadResult>()),
            (
                "ExpressionEvent",
                EXPRESSION_EVENT,
                serde_tags::<ExpressionEvent>(),
            ),
            (
                "ExpressionResult",
                EXPRESSION_RESULT,
                serde_tags::<ExpressionResult>(),
            ),
            ("ContentEvent", CONTENT_EVENT, serde_tags::<ContentEvent>()),
            (
                "ContentResult",
                CONTENT_RESULT,
                serde_tags::<ContentResult>(),
            ),
            (
                "CommunityEvent",
                COMMUNITY_EVENT,
                serde_tags::<CommunityEvent>(),
            ),
            (
                "CommunityResult",
                COMMUNITY_RESULT,
                serde_tags::<CommunityResult>(),
            ),
            (
                "ScheduleUpdate",
                SCHEDULE_UPDATE,
                serde_tags::<ScheduleUpdate>(),
            ),
            (
                "ScheduleResult",
                SCHEDULE_RESULT,
                serde_tags::<ScheduleResult>(),
            ),
            ("StageEvent", STAGE_EVENT, serde_tags::<StageEvent>()),
            (
                "AppearanceEvent",
                APPEARANCE_EVENT,
                serde_tags::<AppearanceEvent>(),
            ),
            (
                "AppearanceResult",
                APPEARANCE_RESULT,
                serde_tags::<AppearanceResult>(),
            ),
        ];
        for (name, tags, serde) in decoders {
            assert_eq!(listed(tags), serde, "{name}: its decode_tagged! table");
        }
    }

    /// Each feature decoder reads exactly what the server writes.
    #[test]
    fn feature_decoders_read_what_the_server_writes() {
        use feature::*;
        fn check<T: serde::Serialize + PartialEq + std::fmt::Debug>(
            value: T,
            decode: fn(&[u8]) -> Result<T, ()>,
        ) {
            let bytes = rmp_serde::to_vec_named(&value).unwrap();
            assert_eq!(decode(&bytes).unwrap(), value);
        }
        check(
            CommunityEvent::Queue {
                guild_id: Id(3),
                pending: 2,
            },
            community_event,
        );
        check(CommunityEvent::Updated { guild_id: Id(3) }, community_event);
        check(CommunityResult::Ack, community_result);
        check(CommunityResult::Listing(None), community_result);
        check(
            ScheduleUpdate::Interest {
                event_id: Id(4),
                interested: true,
            },
            schedule_update,
        );
        check(ScheduleResult::Summary { guilds: vec![] }, schedule_result);
        check(ScheduleResult::Ack, schedule_result);
        check(StageEvent::Snapshot { stages: vec![] }, stage_event);
        check(
            StageEvent::Members {
                channel_id: Id(1),
                upserts: vec![],
                removed: vec![Id(2)],
            },
            stage_event,
        );
        check(
            AppearanceEvent::ServerTheme {
                guild_id: Id(5),
                theme: None,
            },
            appearance_event,
        );
        check(
            AppearanceEvent::Tts {
                channel_id: Id(1),
                message_id: Id(2),
            },
            appearance_event,
        );
        check(AppearanceResult::Library(Box::default()), appearance_result);
    }

    #[test]
    fn core_events_decode_exactly_as_the_derived_decoder_does() {
        let samples = vec![
            Event::SessionsUpdated,
            Event::RelationshipRemoved { user_id: Id(5) },
            Event::MessageDeleted {
                channel_id: Id(1),
                message_id: Id(2),
            },
            Event::Typing {
                channel_id: Id(1),
                user_id: Id(9),
                expires_at: 77,
            },
            Event::PinsUpdated {
                channel_id: Id(1),
                message_id: Id(3),
                pinned: true,
            },
            Event::SlowmodeUpdated {
                channel_id: Id(4),
                seconds: 30,
            },
            Event::ChannelRemoved {
                channel_id: Id(4),
                reason: RemovalReason::Left,
            },
            Event::GuildRemoved {
                guild_id: Id(8),
                reason: RemovalReason::Left,
            },
            Event::InviteRemoved {
                guild_id: Id(8),
                invite_id: Id(12),
            },
            Event::BanRemoved {
                guild_id: Id(8),
                user_id: Id(13),
            },
            Event::RolesUpdated {
                guild_id: Id(8),
                roles: vec![],
            },
            Event::ChannelsReordered {
                guild_id: Id(8),
                placements: vec![],
            },
            Event::ServerFoldersUpdated(vec![]),
            Event::UserUpdated(user()),
            Event::PresenceUpdated(Presence {
                user_id: Id(9),
                status: PresenceStatus::Idle,
            }),
        ];
        for (i, e) in samples.into_iter().enumerate() {
            assert_eq!(owner(&e), None);
            let bytes = event(i as u64 + 1, e.clone());
            match frame(&bytes) {
                Ok(Inbound::Event(seq, Routed::Core(got), _)) => {
                    assert_eq!(seq, i as u64 + 1);
                    assert_eq!(got, e);
                }
                _ => panic!("event {i} did not round trip"),
            }
        }
    }

    #[test]
    fn feature_events_are_passed_on_as_their_undecoded_data() {
        let e = Event::Content(ContentEvent::ScheduledRemoved {
            id: Id(3),
            channel_id: Id(4),
            sent: Some(Id(6)),
        });
        assert_eq!(owner(&e), Some(Feature::Content));
        let Ok(Inbound::Event(5, Routed::Feature(Feature::Content, raw), _)) = frame(&event(5, e))
        else {
            panic!("not routed to content");
        };
        assert_eq!(raw.tag, "content");
        let decoded: ContentEvent = de(&raw.data).unwrap();
        assert_eq!(
            decoded,
            ContentEvent::ScheduledRemoved {
                id: Id(3),
                channel_id: Id(4),
                sent: Some(Id(6))
            }
        );
    }

    #[test]
    fn responses_route_feature_results_and_keep_errors() {
        let response = |outcome| {
            encode(&ServerMsg::Response(Box::new(CommandResponse {
                nonce: 42,
                cursor: ResumeCursor {
                    stream_id: "s".into(),
                    seq: 9,
                },
                replayed: false,
                outcome,
            })))
        };
        let Ok(Inbound::Response(r)) = frame(&response(CommandOutcome::Ok(CommandResult::Ack)))
        else {
            panic!()
        };
        assert_eq!(r.nonce, 42);
        assert!(matches!(r.outcome, Ok(Routed::Core(CommandResult::Ack))));
        let bytes = response(CommandOutcome::Ok(CommandResult::Content(
            ContentResult::Ack,
        )));
        let Ok(Inbound::Response(r)) = frame(&bytes) else {
            panic!()
        };
        let Ok(Routed::Feature(Feature::Content, raw)) = r.outcome else {
            panic!()
        };
        assert_eq!(de::<ContentResult>(&raw.data).unwrap(), ContentResult::Ack);
        let err = ApiError {
            code: ErrorCode::NotFound,
            message: "gone".into(),
            field: None,
            retry_after_ms: None,
        };
        let Ok(Inbound::Response(r)) = frame(&response(CommandOutcome::Err(err.clone()))) else {
            panic!()
        };
        assert_eq!(r.outcome.err(), Some(err));
    }

    #[test]
    fn values_read_as_json_exactly_as_rmp_serde_reads_them() {
        use std::collections::BTreeMap;
        #[derive(serde::Serialize, Deserialize, PartialEq, Debug)]
        struct Sample {
            text: String,
            small: i8,
            big: u64,
            negative: i64,
            none: Option<u32>,
            flags: Vec<bool>,
            bytes: Vec<u8>,
            #[serde(with = "serde_bytes_like")]
            blob: Vec<u8>,
            by_id: BTreeMap<u64, String>,
            nested: Vec<Vec<Option<i16>>>,
            ratio: f32,
        }
        mod serde_bytes_like {
            pub fn serialize<S: serde::Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
                s.serialize_bytes(v)
            }
            pub fn deserialize<'de, D: serde::Deserializer<'de>>(
                d: D,
            ) -> Result<Vec<u8>, D::Error> {
                serde::Deserialize::deserialize(d)
            }
        }
        let sample = Sample {
            text: "quote \" back \\ tab \t nul \u{0} line\n é 🙂".into(),
            small: -5,
            big: u64::MAX,
            negative: i64::MIN,
            none: None,
            flags: vec![true, false],
            bytes: vec![0, 255, 7],
            blob: vec![1, 2, 3],
            by_id: [(1, "a".to_string()), (u64::MAX, "b".to_string())].into(),
            nested: vec![vec![Some(-300), None], vec![]],
            ratio: 0.25,
        };
        let bytes = rmp_serde::to_vec_named(&sample).unwrap();
        assert_eq!(de::<Sample>(&bytes).unwrap(), sample);
        assert!(de::<Sample>(&bytes[..bytes.len() - 1]).is_err());
        // Deeper than the socket's limit.
        let mut deep = vec![0x91; 40];
        deep.push(0xc0);
        assert!(to_json(&deep).is_err());
        // NaN has no JSON form.
        assert!(to_json(&[0xcb, 0x7f, 0xf8, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn control_frames_and_rejections() {
        let hello = Hello {
            version: PROTOCOL_VERSION,
            heartbeat_interval_ms: 1,
            heartbeat_timeout_ms: 2,
            max_frame_bytes: 3,
            server_time: 4,
        };
        assert!(
            matches!(frame(&encode(&ServerMsg::Hello(hello))), Ok(Inbound::Hello(h)) if h.max_frame_bytes == 3)
        );
        let ack = ServerMsg::HeartbeatAck(HeartbeatAck {
            nonce: 1,
            server_time: 2,
        });
        assert!(matches!(frame(&encode(&ack)), Ok(Inbound::HeartbeatAck)));
        let mut bytes = encode(&ack);
        bytes.push(0xc0);
        assert!(frame(&bytes).is_err(), "trailing bytes");
        assert!(frame(&bytes[..bytes.len() - 3]).is_err(), "truncated");
        // Unknown frame and event types are protocol errors, as before.
        let unknown =
            rmp_serde::to_vec_named(&serde_json::json!({"type": "nope", "data": 1})).unwrap();
        assert!(frame(&unknown).is_err());
        // A huge declared length cannot make the reader allocate or loop.
        assert!(
            frame(&[0x82, 0xa4, b't', b'y', b'p', b'e', 0xdb, 0xff, 0xff, 0xff, 0xff]).is_err()
        );
        assert!(frame(&[0xdf, 0xff, 0xff, 0xff, 0xff]).is_err());
        // Deep nesting is walked without recursion.
        let mut deep = vec![
            0x82, 0xa4, b't', b'y', b'p', b'e', 0xa5, b'e', b'v', b'e', b'n', b't', 0xa4, b'd',
            b'a', b't', b'a',
        ];
        deep.extend(std::iter::repeat_n(0x91, 100_000));
        deep.push(0xc0);
        assert!(frame(&deep).is_err());
        let _ = core;
    }
}
