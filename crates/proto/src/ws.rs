use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ClientMsg {
    Identify(Identify),
    Heartbeat(Heartbeat),
    Command(CommandRequest),
    Voice(VoicePayload),
    /// Ephemeral notification suppression hint, scoped to this live connection.
    Focus {
        focused: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Identify {
    pub version: u16,
    pub token: String,
    pub resume: Option<ResumeCursor>,
    /// Opt in to launch-only event variants; old peers omit this field.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub launch_features: bool,
    /// Opt in to custom emoji, slowmode and folder event variants; independent of legacy launch support.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub text_features: bool,
}

impl Default for Identify {
    fn default() -> Self {
        Self {
            version: PROTOCOL_VERSION,
            token: String::new(),
            resume: None,
            launch_features: false,
            text_features: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Heartbeat {
    pub nonce: Nonce,
    pub last_seq: EventSeq,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Hello {
    pub version: u16,
    pub heartbeat_interval_ms: u32,
    pub heartbeat_timeout_ms: u32,
    pub max_frame_bytes: u32,
    pub server_time: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct Ready {
    pub cursor: ResumeCursor,
    pub me: SelfUser,
    pub guilds: Vec<GuildSummary>,
    pub channels: Vec<ChannelSummary>,
    pub relationships: Vec<Relationship>,
    /// Deduplicated users needed for direct-message sidebar labels (not every guild member).
    pub users: Vec<UserSummary>,
    pub read_states: Vec<ReadState>,
    pub presences: Vec<Presence>,
    #[serde(default)]
    pub voice_states: Vec<GuildVoiceState>,
    #[serde(default)]
    pub roles: Vec<Role>,
    /// Current user's memberships; full member lists remain paged.
    #[serde(default)]
    pub memberships: Vec<Member>,
    #[serde(default)]
    pub notification_settings: Vec<NotificationSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Resumed {
    pub cursor: ResumeCursor,
    pub replayed: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(rename_all = "snake_case")]
pub enum ResyncReason {
    StreamExpired,
    CursorTooOld,
    CursorAhead,
    SlowConsumer,
}
crate::wire_names::wire_names!(ResyncReason {
    StreamExpired = "stream_expired",
    CursorTooOld = "cursor_too_old",
    CursorAhead = "cursor_ahead",
    SlowConsumer = "slow_consumer",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResyncRequired {
    pub reason: ResyncReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HeartbeatAck {
    pub nonce: Nonce,
    pub server_time: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ServerMsg {
    Hello(Hello),
    Ready(Box<Ready>),
    Resumed(Resumed),
    ResyncRequired(ResyncRequired),
    HeartbeatAck(HeartbeatAck),
    Response(Box<CommandResponse>),
    Event(Box<EventEnvelope>),
    Error(ApiError),
    Voice(VoicePayload),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandRequest {
    pub nonce: Nonce,
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Command {
    Launch(LaunchCommand),
    UpdateProfile(ProfilePatch),
    SetPresence {
        status: PresenceStatus,
    },
    SendFriendRequest {
        username: String,
    },
    AcceptFriendRequest {
        user_id: UserId,
    },
    DeclineFriendRequest {
        user_id: UserId,
    },
    CancelFriendRequest {
        user_id: UserId,
    },
    RemoveFriend {
        user_id: UserId,
    },
    BlockUser {
        user_id: UserId,
    },
    UnblockUser {
        user_id: UserId,
    },
    OpenDm {
        user_id: UserId,
    },
    CreateGroupDm {
        name: String,
        user_ids: Vec<UserId>,
    },
    AddGroupRecipient {
        channel_id: ChannelId,
        user_id: UserId,
    },
    RemoveGroupRecipient {
        channel_id: ChannelId,
        user_id: UserId,
    },
    LeaveGroupDm {
        channel_id: ChannelId,
    },
    TransferGroupOwnership {
        channel_id: ChannelId,
        user_id: UserId,
    },
    CloseDm {
        channel_id: ChannelId,
    },
    CreateGuild {
        name: String,
        icon_id: Option<AttachmentId>,
    },
    UpdateGuild {
        guild_id: GuildId,
        patch: GuildPatch,
    },
    DeleteGuild {
        guild_id: GuildId,
    },
    JoinGuild {
        invite_code: String,
    },
    LeaveGuild {
        guild_id: GuildId,
    },
    TransferGuildOwnership {
        guild_id: GuildId,
        user_id: UserId,
    },
    SetMemberRole {
        guild_id: GuildId,
        user_id: UserId,
        role: MemberRole,
    },
    KickMember {
        guild_id: GuildId,
        user_id: UserId,
    },
    BanMember {
        guild_id: GuildId,
        user_id: UserId,
        reason: Option<String>,
    },
    UnbanMember {
        guild_id: GuildId,
        user_id: UserId,
    },
    CreateInvite {
        guild_id: GuildId,
        ttl_seconds: u32,
        max_uses: u32,
    },
    RevokeInvite {
        guild_id: GuildId,
        code: String,
    },
    CreateChannel {
        guild_id: GuildId,
        kind: ChannelKind,
        name: String,
        parent_id: Option<ChannelId>,
        topic: Option<String>,
    },
    UpdateChannel {
        channel_id: ChannelId,
        patch: ChannelPatch,
    },
    ReorderChannels {
        guild_id: GuildId,
        placements: Vec<ChannelPlacement>,
    },
    DeleteChannel {
        channel_id: ChannelId,
    },
    /// Per-member Connect denial; true clears the denial. Requires Manage Channels.
    SetVoicePermission {
        channel_id: ChannelId,
        user_id: UserId,
        connect: bool,
    },
    SendMessage(MessageDraft),
    EditMessage(MessageEdit),
    DeleteMessage {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    AddReaction {
        channel_id: ChannelId,
        message_id: MessageId,
        emoji: String,
    },
    RemoveReaction {
        channel_id: ChannelId,
        message_id: MessageId,
        emoji: String,
    },
    Typing {
        channel_id: ChannelId,
    },
    MarkRead {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    /// Replace this connection's optional member/presence interest set, not persistent state.
    SubscribeGuild {
        guild_id: Option<GuildId>,
        presence_user_ids: Vec<UserId>,
    },
    /// Polls, forwarding, scheduled messages and rich attachments (`content.rs`).
    Content(ContentCommand),
    /// Threads, forums and announcements (`threads.rs`).
    Threads(ThreadCommand),
    /// Stickers, Super Reactions and typing indicators (`expression.rs`).
    Expression(ExpressionCommand),
    /// Status, nicknames, notes, ignore, requests, notifications, reports (`privacy.rs`).
    Privacy(PrivacyCommand),
    /// Admission, onboarding, discovery, templates (`community.rs`).
    Community(CommunityCommand),
    /// Scheduled server events (`schedule.rs`).
    Schedule(crate::ScheduleCommand),
    /// Custom and server themes, `/tts` messages (`themes.rs`).
    Appearance(AppearanceCommand),
    /// Apps, bots, application commands and webhooks (`apps.rs`).
    Apps(crate::AppCommand),
    /// Server tags, role styles and icons, server images, vanity links (`identity.rs`).
    Identity(IdentityCommand),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandResponse {
    pub nonce: Nonce,
    pub cursor: ResumeCursor,
    /// A receipt's original result is historical; use only to resolve operation identity.
    pub replayed: bool,
    pub outcome: CommandOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
// One short-lived value per command; boxing the result would touch every match on it.
#[allow(clippy::large_enum_variant)]
pub enum CommandOutcome {
    Ok(CommandResult),
    Err(ApiError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CommandResult {
    Role(Role),
    NotificationSettings(NotificationSettings),
    Ack,
    Profile(SelfUser),
    Presence(Presence),
    Relationship(Relationship),
    Channel(Channel),
    Guild(GuildDetail),
    Member(Member),
    Invite(Invite),
    Message(Message),
    Reaction(ReactionUpdate),
    ReadState(ReadState),
    Subscription { presences: Vec<Presence> },
    Content(ContentResult),
    Moderation(ModerationResult),
    Threads(ThreadResult),
    Expression(ExpressionResult),
    Privacy(PrivacyResult),
    Community(CommunityResult),
    Schedule(crate::ScheduleResult),
    Appearance(AppearanceResult),
    Apps(crate::AppResultExtra),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReactionUpdate {
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub user_id: UserId,
    pub added: bool,
    pub reaction: ReactionSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EventEnvelope {
    pub seq: EventSeq,
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Event {
    ServerFoldersUpdated(Vec<ServerFolder>),
    SlowmodeUpdated {
        channel_id: ChannelId,
        seconds: u32,
    },
    GuildEmojiUpdated {
        guild_id: GuildId,
        emoji: Vec<GuildEmoji>,
    },
    RolesUpdated {
        guild_id: GuildId,
        roles: Vec<Role>,
    },
    ProfileUpdated(UserProfile),
    GuildProfileUpdated(GuildProfile),
    PinsUpdated {
        channel_id: ChannelId,
        message_id: MessageId,
        pinned: bool,
    },
    NotificationSettingsUpdated(NotificationSettings),
    SessionsUpdated,
    SelfUpdated(SelfUser),
    UserUpdated(UserSummary),
    RelationshipUpsert(Relationship),
    RelationshipRemoved {
        user_id: UserId,
    },
    PresenceUpdated(Presence),
    GuildUpsert(GuildSummary),
    GuildJoined {
        detail: GuildDetail,
        read_states: Vec<ReadState>,
    },
    GuildRemoved {
        guild_id: GuildId,
        reason: RemovalReason,
    },
    MemberUpsert(Member),
    MemberRemoved {
        guild_id: GuildId,
        user_id: UserId,
        reason: RemovalReason,
    },
    InviteUpsert(Invite),
    InviteRemoved {
        guild_id: GuildId,
        invite_id: Id,
    },
    BanUpsert(Ban),
    BanRemoved {
        guild_id: GuildId,
        user_id: UserId,
    },
    ChannelUpsert(Channel),
    ChannelRemoved {
        channel_id: ChannelId,
        reason: RemovalReason,
    },
    ChannelsReordered {
        guild_id: GuildId,
        placements: Vec<ChannelPlacement>,
    },
    MessageCreated {
        message: Message,
        author: UserSummary,
    },
    MessageUpdated(Message),
    MessageDeleted {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    ReactionUpdated(ReactionUpdate),
    Typing {
        channel_id: ChannelId,
        user_id: UserId,
        expires_at: UnixMillis,
    },
    ReadStateUpdated(ReadState),
    /// Sent only to streams that fetched GET /api/v1/users/@me/nav (see docs/features/nav.md).
    NavUpdated(NavSettings),
    /// Saved messages changed (another device, or a reminder came due).
    SavedUpdated(SavedSummary),
    /// A reminder's time has come. `message` is None when it may no longer be read.
    ReminderDue(SavedMessage),
    /// Sent only to connections that sent `ContentCommand::Subscribe`.
    Content(ContentEvent),
    /// Only sent to streams that sent `ModerationCommand::Subscribe`.
    Moderation(ModerationEvent),
    /// Sent only to streams that fetched GET /api/v1/users/@me/data-export (docs/features/authsec.md).
    DataExportUpdated(DataExport),
    /// Sent only to connections that sent `ThreadCommand::Subscribe`.
    Threads(ThreadEvent),
    /// Only sent to streams that sent `ExpressionCommand::Subscribe`.
    Expression(ExpressionEvent),
    /// Only sent to streams that sent `PrivacyCommand::Subscribe`.
    Privacy(PrivacyEvent),
    /// Only sent to streams that sent `CommunityCommand::Subscribe`.
    Community(CommunityEvent),
    /// Sent only to connections that sent a `ScheduleCommand`.
    Schedule(crate::ScheduleUpdate),
    /// Only sent to streams that sent `AppearanceCommand::Subscribe`.
    Appearance(AppearanceEvent),
    /// Only sent to streams that sent `AppCommand::Subscribe`, and to bots.
    Apps(crate::AppEventExtra),
}

/// Stable outer voice envelope. Version 1 bodies are named MessagePack maps:
/// `kind = "signal"` contains VoiceSignal; `kind = "event"` contains VoiceEvent.
/// The original four fields and legacy envelopes remain wire-compatible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoicePayload {
    pub nonce: Nonce,
    pub version: u16,
    pub kind: String,
    pub payload: Vec<u8>,
}

pub const VOICE_VERSION: u16 = 1;
pub const CONTROL_CHANNEL_LABEL: &str = "sticktalk-control-v1";
/// Legacy native packetization hint; the configured SFU ceiling is independent.
pub const MAX_RTP_DATAGRAM_BYTES: usize = 1200;
/// Safe UDP payload ceiling on a 1280-byte IPv6 Tailscale path.
pub const DEFAULT_VOICE_UDP_BYTES: u16 = 1232;

/// Supplied by the authenticated host, never taken from a client's signal body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct VoiceSession {
    pub user_id: UserId,
    pub connection_id: Id,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
}
crate::wire_names::wire_names!(VideoCodec {
    H264 = "h264",
    Hevc = "hevc",
    Av1 = "av1",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Audio,
    Camera,
    Screen,
    SystemAudio,
}
crate::wire_names::wire_names!(TrackKind {
    Audio = "audio",
    Camera = "camera",
    Screen = "screen",
    SystemAudio = "system_audio",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(rename_all = "snake_case")]
pub enum ShareSurface {
    Monitor,
    Window,
}
crate::wire_names::wire_names!(ShareSurface {
    Monitor = "monitor",
    Window = "window",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceCapabilities {
    /// H.264 is mandatory. Advanced codecs must ALSO appear in hardware_decode.
    pub video: Vec<VideoCodec>,
    pub hardware_decode: Vec<VideoCodec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedTrack {
    pub mid: String,
    pub kind: TrackKind,
    /// None for Opus; mandatory for camera/screen.
    pub codec: Option<VideoCodec>,
    /// Ordered low to high; empty means a single non-simulcast stream.
    pub layers: Vec<String>,
    /// Required for screen, forbidden otherwise. Re-publish invalidates control.
    pub surface: Option<ShareSurface>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum VoiceSignal {
    Join {
        channel_id: ChannelId,
        capabilities: VoiceCapabilities,
    },
    Leave,
    /// Client is always offerer. Include the complete publication set each time.
    Offer {
        sdp: String,
        tracks: Vec<PublishedTrack>,
    },
    Ice {
        candidate: String,
    },
    State {
        muted: bool,
        deafened: bool,
    },
    /// Receiver creates a recvonly m-line first; one source per receiver MID.
    Subscribe {
        publisher: VoiceSession,
        source_mid: String,
        receiver_mid: String,
        rid: Option<String>,
    },
    Unsubscribe {
        receiver_mid: String,
    },
    RequestKeyframe {
        receiver_mid: String,
        fir: bool,
    },
    RequestControl {
        sharer: VoiceSession,
    },
    DecideControl {
        requester: VoiceSession,
        allow: bool,
    },
    RevokeControl,
    /// Explicit sending mode. Legacy callers default to push-to-talk, not VAD/priority.
    Activity {
        voice_activity: bool,
        priority: bool,
    },
    /// Host-authorized moderation; never forwarded as untrusted SFU input.
    Moderate {
        session: VoiceSession,
        action: VoiceModeration,
    },
    Stats,
    /// Ring members of a direct or group call again. Host-handled; caller must be in the call.
    Ring {
        channel_id: ChannelId,
        user_ids: Vec<UserId>,
    },
    /// Stop ringing this user's devices for that conversation's call.
    DeclineCall {
        channel_id: ChannelId,
    },
    /// Play a soundboard sound to everyone in this call. Host-handled.
    PlaySound {
        sound_id: Id,
    },
    /// This user keeps a rolling clip buffer of the call's streams. Host-handled.
    SetClipping {
        enabled: bool,
    },
    /// This user just saved a clip. Host-handled.
    ClipTaken,
    /// The most this video track needs (its chosen resolution and frame rate).
    /// The SFU gives unused share to the tracks that need more. SFU-handled.
    TrackQuality {
        mid: String,
        max_bps: u64,
    },
    /// Stage roles and lifecycle (`stage.rs`). Host-handled.
    Stage(crate::StageSignal),
    /// End-to-end encrypted calls (`e2ee.rs`). Host-handled.
    E2ee(crate::E2eeSignal),
    /// Shared activities in this call (`activities.rs`). Host-handled.
    Activities(crate::ActivitySignal),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum VoiceModeration {
    Mute { muted: bool },
    Deafen { deafened: bool },
    Move { channel_id: ChannelId },
    Disconnect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceParticipant {
    pub session: VoiceSession,
    pub muted: bool,
    pub deafened: bool,
    pub tracks: Vec<PublishedTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildVoiceState {
    pub guild_id: GuildId,
    pub channel_id: ChannelId,
    pub participant: VoiceParticipant,
}

/// Tailscale DERP is invisible to ICE; only the trusted host can report that path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaPath {
    Unknown,
    Direct,
    Relayed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceStats {
    pub tx_bps: u64,
    pub rx_bps: u64,
    /// RTP marker-frame rate, not decoded/displayed frames. Aggregate video.
    pub tx_fps_milli: u32,
    pub rx_fps_milli: u32,
    pub loss_per_million: Option<u32>,
    pub rtt_ms: Option<u32>,
    pub path: MediaPath,
    pub cap_dropped_packets: u64,
    pub queue_dropped_packets: u64,
    pub mtu_dropped_packets: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum VoiceEvent {
    /// Ephemeral full replacement, broadcast to all current guild members.
    GuildState(GuildVoiceState),
    GuildLeft {
        guild_id: GuildId,
        session: VoiceSession,
    },
    /// Sent after Resumed; voice state is never replayed.
    GuildSnapshot {
        states: Vec<GuildVoiceState>,
    },
    Joined {
        channel_id: ChannelId,
        participants: Vec<VoiceParticipant>,
        max_rtp_bytes: u16,
    },
    Participant(VoiceParticipant),
    Left {
        session: VoiceSession,
    },
    Answer {
        sdp: String,
    },
    Speaking {
        session: VoiceSession,
        speaking: bool,
        level_dbov: i8,
    },
    /// Apply this upstream bitrate to the named encoder (in addition to RTCP).
    Bitrate {
        mid: String,
        bps: u64,
    },
    CodecRequired {
        mid: String,
        codec: VideoCodec,
    },
    ControlRequested {
        requester: VoiceSession,
        timeout_ms: u32,
    },
    ControlDenied {
        sharer: VoiceSession,
    },
    ControlGranted {
        sharer: VoiceSession,
        controller: VoiceSession,
        grant: u64,
    },
    ControlRevoked {
        sharer: VoiceSession,
        grant: u64,
    },
    Stats(VoiceStats),
    Ack,
    Error {
        code: ErrorCode,
        message: String,
    },
    /// Ephemeral full replacement, sent to every member of the conversation.
    CallState(DmCall),
    CallEnded {
        channel_id: ChannelId,
    },
    /// Sent after Ready and Resumed: the calls in this user's conversations.
    CallSnapshot {
        calls: Vec<DmCall>,
    },
    /// Someone in this call played a soundboard sound (or joined with one).
    SoundPlayed {
        channel_id: ChannelId,
        sound: SoundPlay,
        by: VoiceSession,
        entrance: bool,
    },
    /// Someone in this call turned clipping on or off.
    Clipping {
        session: VoiceSession,
        enabled: bool,
    },
    /// Someone in this call saved a clip.
    ClipTaken {
        session: VoiceSession,
    },
    /// The Opus bitrate this call's microphones use, enforced by the SFU.
    AudioQuality {
        channel_id: ChannelId,
        bitrate: u32,
    },
    /// Stage state (`stage.rs`).
    Stage(crate::StageEvent),
    /// End-to-end encrypted calls (`e2ee.rs`): to sessions in the call only.
    E2ee(crate::E2eeEvent),
    /// Shared activities in this call (`activities.rs`).
    Activities(crate::ActivityEvent),
}

/// ONLY sent as binary named MessagePack on CONTROL_CHANNEL_LABEL, never WS.
/// u16 coordinates divided by 65535 normalize to [0,1] of the shared surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ControlInput {
    Move {
        x: u16,
        y: u16,
    },
    Button {
        button: u8,
        down: bool,
    },
    Wheel {
        x: i16,
        y: i16,
    },
    /// USB HID keyboard usage, not text or arbitrary OS commands.
    Key {
        usage: u16,
        down: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ControlData {
    Input {
        sharer: VoiceSession,
        grant: u64,
        sequence: u64,
        input: ControlInput,
    },
    /// Server -> sharer only. Release ALL held keys/buttons, idempotently.
    ReleaseAll { grant: u64 },
}
