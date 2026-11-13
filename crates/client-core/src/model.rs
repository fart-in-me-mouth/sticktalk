//! Bounded, deterministic projection of protocol state. No GUI or I/O in reducers.
use std::collections::{BTreeMap, VecDeque};
use sticktalk_proto::*;

pub const MESSAGE_LIMIT: usize = 1_000;
pub const MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const JOURNAL_LIMIT: usize = 2_048;

#[derive(Default)]
pub struct Model {
    pub launch: crate::launch::Projection,
    pub active_guild: Option<Id>,
    pub cursor: Option<ResumeCursor>,
    pub layout_revision: u64,
    pub me: Option<SelfUser>,
    pub guilds: BTreeMap<Id, GuildSummary>,
    pub channels: BTreeMap<Id, ChannelSummary>,
    pub topics: BTreeMap<Id, Option<String>>,
    pub users: BTreeMap<Id, UserSummary>,
    pub relationships: BTreeMap<Id, Relationship>,
    pub presences: BTreeMap<Id, PresenceStatus>,
    pub reads: BTreeMap<Id, ReadState>,
    pub messages: BTreeMap<Id, BTreeMap<Id, Message>>,
    pub members: BTreeMap<Id, Member>,
    pub invites: BTreeMap<Id, Invite>,
    pub bans: BTreeMap<Id, Ban>,
    pub typing: BTreeMap<(Id, Id), UnixMillis>,
    lru: VecDeque<Id>,
    journal: VecDeque<EventEnvelope>,
    floor: EventSeq,
    journal_bytes: usize,
    trim_newest: std::collections::BTreeSet<Id>,
}

impl Model {
    pub fn ready(&mut self, ready: Ready) {
        *self = Self::default();
        self.launch.ready(&ready);
        self.cursor = Some(ready.cursor);
        self.users.insert(ready.me.user.id, ready.me.user.clone());
        self.me = Some(ready.me);
        self.guilds = ready.guilds.into_iter().map(|v| (v.guild.id, v)).collect();
        self.channels = ready.channels.into_iter().map(|v| (v.id, v)).collect();
        self.reads = ready
            .read_states
            .into_iter()
            .map(|v| (v.channel_id, v))
            .collect();
        for user in ready.users {
            self.users.insert(user.id, user);
        }
        for rel in ready.relationships {
            self.users.insert(rel.user.id, rel.user.clone());
            self.relationships.insert(rel.user.id, rel);
        }
        for presence in ready.presences {
            self.presences.insert(presence.user_id, presence.status);
        }
    }

    pub fn apply(&mut self, envelope: EventEnvelope) -> bool {
        let Some(cursor) = self.cursor.as_mut() else {
            return false;
        };
        if envelope.seq <= cursor.seq {
            return false;
        }
        cursor.seq = envelope.seq;
        self.reduce(envelope.event.clone());
        self.journal_bytes += event_bytes(&envelope);
        self.journal.push_back(envelope);
        while self.journal.len() > JOURNAL_LIMIT || self.journal_bytes > 2 * 1024 * 1024 {
            if let Some(old) = self.journal.pop_front() {
                self.floor = old.seq;
                self.journal_bytes = self.journal_bytes.saturating_sub(event_bytes(&old));
            }
        }
        self.enforce_bounds();
        true
    }

    pub fn reduce(&mut self, event: Event) {
        self.launch.event(&event);
        if matches!(
            &event,
            Event::MessageCreated { .. }
                | Event::MessageUpdated(_)
                | Event::MessageDeleted { .. }
                | Event::ReactionUpdated(_)
                | Event::ReadStateUpdated(_)
                | Event::UserUpdated(_)
                | Event::MemberUpsert(_)
                | Event::RolesUpdated { .. }
        ) {
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
        match event {
            Event::SelfUpdated(v) => {
                self.users.insert(v.user.id, v.user.clone());
                self.me = Some(v);
            }
            Event::UserUpdated(v) => {
                self.users.insert(v.id, v);
            }
            Event::RelationshipUpsert(v) => {
                self.users.insert(v.user.id, v.user.clone());
                self.relationships.insert(v.user.id, v);
            }
            Event::RelationshipRemoved { user_id } => {
                self.relationships.remove(&user_id);
            }
            Event::PresenceUpdated(v) => {
                self.presences.insert(v.user_id, v.status);
            }
            Event::GuildUpsert(v) => {
                self.guilds.insert(v.guild.id, v);
            }
            Event::GuildJoined {
                detail,
                read_states,
            } => {
                self.guilds.insert(detail.summary.guild.id, detail.summary);
                for v in detail.channels {
                    self.channels.insert(v.id, v);
                }
                for v in read_states {
                    self.reads.insert(v.channel_id, v);
                }
            }
            Event::GuildRemoved { guild_id, .. } => {
                self.guilds.remove(&guild_id);
                let ids: Vec<_> = self
                    .channels
                    .values()
                    .filter(|v| v.guild_id == Some(guild_id))
                    .map(|v| v.id)
                    .collect();
                for id in ids {
                    self.purge_channel(id);
                }
                self.members.retain(|_, v| v.guild_id != guild_id);
                self.invites.retain(|_, v| v.guild_id != guild_id);
                self.bans.retain(|_, v| v.guild_id != guild_id);
            }
            Event::MemberUpsert(v) => {
                self.users.insert(v.user.id, v.user.clone());
                self.members.insert(v.user.id, v);
            }
            Event::MemberRemoved { user_id, .. } => {
                self.members.remove(&user_id);
            }
            Event::InviteUpsert(v) => {
                self.invites.insert(v.id, v);
            }
            Event::InviteRemoved { invite_id, .. } => {
                self.invites.remove(&invite_id);
            }
            Event::BanUpsert(v) => {
                self.bans.insert(v.user.id, v);
            }
            Event::BanRemoved { user_id, .. } => {
                self.bans.remove(&user_id);
            }
            Event::ChannelUpsert(v) => {
                self.topics.insert(v.summary.id, v.topic);
                self.channels.insert(v.summary.id, v.summary);
            }
            Event::ChannelRemoved { channel_id, .. } => self.purge_channel(channel_id),
            Event::ChannelsReordered { placements, .. } => {
                for v in placements {
                    if let Some(c) = self.channels.get_mut(&v.channel_id) {
                        c.parent_id = v.parent_id;
                        c.position = v.position;
                    }
                }
            }
            Event::MessageCreated { message, author } => {
                self.users.insert(author.id, author);
                if self.messages.contains_key(&message.channel_id)
                    || self.lru.contains(&message.channel_id)
                {
                    self.insert(message);
                }
            }
            Event::MessageUpdated(v) => {
                if self.messages.contains_key(&v.channel_id) || self.lru.contains(&v.channel_id) {
                    self.insert(v);
                }
            }
            Event::MessageDeleted {
                channel_id,
                message_id,
            } => {
                if let Some(list) = self.messages.get_mut(&channel_id) {
                    list.remove(&message_id);
                }
            }
            Event::ReactionUpdated(v) => {
                if let Some(m) = self
                    .messages
                    .get_mut(&v.channel_id)
                    .and_then(|ms| ms.get_mut(&v.message_id))
                {
                    m.reactions.retain(|r| r.emoji != v.reaction.emoji);
                    if v.reaction.count > 0 {
                        m.reactions.push(v.reaction);
                        m.reactions.sort_by(|a, b| a.emoji.cmp(&b.emoji));
                    }
                }
            }
            Event::Typing {
                channel_id,
                user_id,
                expires_at,
            } => {
                if self.channels.contains_key(&channel_id) {
                    self.typing.insert((channel_id, user_id), expires_at);
                }
            }
            Event::ReadStateUpdated(v) => {
                self.reads.insert(v.channel_id, v);
            }
            _ => {}
        }
    }

    /// Strict bounded voice envelope decoding shared by all hosts.
    pub fn voice_payload(&mut self, v: &VoicePayload) -> bool {
        if v.kind != "event" || v.version != VOICE_VERSION || v.payload.len() > 72 * 1024 {
            return false;
        }
        let mut de = rmp_serde::Deserializer::new(std::io::Cursor::new(&v.payload));
        de.set_max_depth(32);
        let Ok(event) = serde::Deserialize::deserialize(&mut de) else {
            return false;
        };
        if de.position() != v.payload.len() as u64 {
            return false;
        }
        self.launch.voice_event(event);
        true
    }
    pub fn insert(&mut self, message: Message) {
        if self.channels.contains_key(&message.channel_id) {
            self.messages
                .entry(message.channel_id)
                .or_default()
                .insert(message.id, message);
        }
    }

    /// A snapshot never advances the WebSocket cursor. Reapply newer live mutations,
    /// including tombstones, so late pages cannot resurrect or overwrite messages.
    pub fn merge_page(&mut self, channel: Id, page: Page<Message>) -> bool {
        self.layout_revision = self.layout_revision.wrapping_add(1);
        let Some(cursor) = &self.cursor else {
            return false;
        };
        if cursor.stream_id != page.cursor.stream_id
            || page.cursor.seq < self.floor
            || !self.channels.contains_key(&channel)
        {
            return false;
        }
        let incoming_max = page.items.iter().map(|m| m.id).max();
        let existing_min = self
            .messages
            .get(&channel)
            .and_then(|m| m.first_key_value().map(|(id, _)| *id));
        if matches!((incoming_max,existing_min),(Some(a),Some(b)) if a<b) {
            self.trim_newest.insert(channel);
        } else {
            self.trim_newest.remove(&channel);
        }
        for user in page.users {
            self.users.insert(user.id, user);
        }
        for message in page.items {
            if message.channel_id == channel {
                self.insert(message);
            }
        }
        let newer: Vec<_> = self
            .journal
            .iter()
            .filter(|v| v.seq > page.cursor.seq)
            .cloned()
            .collect();
        for v in newer {
            match &v.event {
                Event::MessageCreated { message, .. } | Event::MessageUpdated(message)
                    if message.channel_id == channel =>
                {
                    self.reduce(v.event)
                }
                Event::MessageDeleted { channel_id, .. } if *channel_id == channel => {
                    self.reduce(v.event)
                }
                Event::ReactionUpdated(r) if r.channel_id == channel => self.reduce(v.event),
                _ => {}
            }
        }
        self.touch(channel);
        self.enforce_bounds();
        true
    }

    pub fn snapshot_valid(&self, cursor: &ResumeCursor) -> bool {
        self.cursor
            .as_ref()
            .is_some_and(|v| v.stream_id == cursor.stream_id)
            && cursor.seq >= self.floor
    }
    pub fn reconcile_metadata(&mut self, cursor: &ResumeCursor) {
        let events: Vec<_> = self
            .journal
            .iter()
            .filter(|e| e.seq > cursor.seq)
            .map(|e| e.event.clone())
            .filter(|e| {
                !matches!(
                    e,
                    Event::MessageCreated { .. }
                        | Event::MessageUpdated(_)
                        | Event::MessageDeleted { .. }
                        | Event::ReactionUpdated(_)
                        | Event::Typing { .. }
                        | Event::ReadStateUpdated(_)
                )
            })
            .collect();
        for e in events {
            self.reduce(e);
        }
    }
    pub fn touch(&mut self, channel: Id) {
        self.lru.retain(|id| *id != channel);
        self.lru.push_back(channel);
    }
    pub fn retained_bytes(&self) -> usize {
        self.messages
            .values()
            .flat_map(|v| v.values())
            .map(|m| {
                std::mem::size_of::<Message>()
                    + m.content.capacity()
                    + m.attachments
                        .iter()
                        .map(|a| {
                            std::mem::size_of::<Attachment>()
                                + a.filename.capacity()
                                + a.content_type.capacity()
                        })
                        .sum::<usize>()
                    + m.mention_ids.capacity() * 8
                    + m.reactions
                        .iter()
                        .map(|r| 40 + r.emoji.capacity())
                        .sum::<usize>()
            })
            .sum()
    }
    pub fn retained_count(&self) -> usize {
        self.messages.values().map(|v| v.len()).sum()
    }
    pub fn enforce_bounds(&mut self) {
        // Keep only four recently visited channels; never cache unseen live history.
        while self.lru.len() > 4 {
            if let Some(id) = self.lru.pop_front() {
                self.messages.remove(&id);
            }
        }
        while self.retained_count() > MESSAGE_LIMIT || self.retained_bytes() > MESSAGE_BYTES {
            let id = self
                .lru
                .iter()
                .find(|id| self.messages.get(id).is_some_and(|v| !v.is_empty()))
                .copied()
                .or_else(|| self.messages.keys().next().copied());
            let Some(id) = id else { break };
            if let Some(list) = self.messages.get_mut(&id) {
                if self.trim_newest.contains(&id) {
                    list.pop_last();
                } else {
                    list.pop_first();
                }
                if list.is_empty() {
                    self.messages.remove(&id);
                }
            }
        }
        self.compact_users();
    }
    /// Auxiliary identities are recoverable via the bounded bulk-user endpoint.
    /// Keep direct contacts and the active member page; discard remote history
    /// identities first instead of retaining every author ever encountered.
    pub fn compact_users(&mut self) {
        const USER_LIMIT: usize = 4096;
        if self.users.len() <= USER_LIMIT {
            return;
        }
        let mut keep = std::collections::BTreeSet::new();
        keep.extend(self.me.as_ref().map(|m| m.user.id));
        keep.extend(self.relationships.keys().copied());
        keep.extend(self.members.keys().copied());
        keep.extend(self.bans.keys().copied());
        keep.extend(
            self.channels
                .values()
                .flat_map(|c| c.recipient_ids.iter().copied()),
        );
        let excess = self.users.len().saturating_sub(USER_LIMIT);
        let remove: Vec<_> = self
            .users
            .keys()
            .filter(|id| !keep.contains(id))
            .take(excess)
            .copied()
            .collect();
        for id in remove {
            self.users.remove(&id);
            self.presences.remove(&id);
        }
    }
    pub fn purge_channel(&mut self, id: Id) {
        self.journal.retain(|e| event_channel(&e.event) != Some(id));
        self.journal_bytes = self.journal.iter().map(event_bytes).sum();
        self.channels.remove(&id);
        self.topics.remove(&id);
        self.messages.remove(&id);
        self.reads.remove(&id);
        self.typing.retain(|(c, _), _| *c != id);
        self.lru.retain(|v| *v != id);
    }
    pub fn name(&self, id: Id) -> String {
        if let Some(name) = self.launch.name(self.active_guild, id) {
            return name.to_owned();
        }
        self.users
            .get(&id)
            .map(|v| v.display_name.as_deref().unwrap_or(&v.username).to_owned())
            .unwrap_or_else(|| format!("User {}", id.0))
    }
    pub fn channel_name(&self, c: &ChannelSummary) -> String {
        c.name.clone().unwrap_or_else(|| {
            c.recipient_ids
                .iter()
                .filter(|id| Some(**id) != self.me.as_ref().map(|m| m.user.id))
                .map(|id| self.name(*id))
                .collect::<Vec<_>>()
                .join(", ")
        })
    }
    /// Guild-scoped capabilities; channel affordances must use launch::permissions.
    pub fn permissions(&self, guild: Option<Id>) -> Permissions {
        let Some(guild) = guild else {
            return Permissions::empty();
        };
        let Some(summary) = self.guilds.get(&guild) else {
            return Permissions::empty();
        };
        let Some(me) = self.me.as_ref().map(|m| m.user.id) else {
            return Permissions::empty();
        };
        if summary.guild.owner_id == me {
            return Permissions::all();
        }
        let roles = self.launch.member_roles(guild, me);
        let p = if roles.is_empty() {
            summary.my_permissions
        } else {
            roles
                .iter()
                .fold(Permissions::empty(), |p, r| p | r.permissions)
        };
        if p.contains(Permissions::ADMINISTRATOR) {
            Permissions::all() & !Permissions::OWNER
        } else {
            p & Permissions::all() & !Permissions::OWNER
        }
    }
    pub fn mention_candidates(&self, text: &str) -> Vec<Id> {
        let Some(query) = text
            .rsplit_once('@')
            .map(|(_, v)| v)
            .filter(|v| !v.contains(char::is_whitespace) && !v.contains('>'))
        else {
            return vec![];
        };
        self.users
            .values()
            .filter(|u| u.username.starts_with(query))
            .take(5)
            .map(|u| u.id)
            .collect()
    }
    pub fn mark_read_command(&self, channel_id: Id, message_id: Id) -> Option<Command> {
        let old = self.reads.get(&channel_id).and_then(|v| v.last_read_id);
        old.is_none_or(|v| v < message_id)
            .then_some(Command::MarkRead {
                channel_id,
                message_id,
            })
    }
    pub fn content(&self, m: &Message) -> String {
        let mut s = m.content.clone();
        for id in &m.mention_ids {
            s = s.replace(&format!("<@{}>", id.0), &format!("@{}", self.name(*id)));
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionState {
    #[default]
    Offline,
    Connecting,
    Ready,
    Backoff(u32),
    Stopped(String),
}

#[derive(Default)]
pub struct Recovery {
    pub state: ConnectionState,
    pub cursor: Option<ResumeCursor>,
    attempts: u32,
}
impl Recovery {
    pub fn disconnected(&mut self, close: Option<u16>) -> Option<u64> {
        if matches!(
            close,
            Some(1000 | 1002 | 1003 | 1008 | 1009 | 4001 | 4002 | 4009)
        ) {
            self.state = ConnectionState::Stopped(
                match close {
                    Some(4001) => "Session expired. Sign in again.",
                    Some(4009) => "This session opened elsewhere. Reconnect explicitly.",
                    _ => "Connection stopped. Check server and client version.",
                }
                .into(),
            );
            return None;
        }
        let delay = (1u64 << self.attempts.min(5)).min(30);
        self.attempts = self.attempts.saturating_add(1);
        self.state = ConnectionState::Backoff(delay as u32);
        Some(delay)
    }
    pub fn applied(&mut self, cursor: ResumeCursor) {
        if self
            .cursor
            .as_ref()
            .is_none_or(|v| v.stream_id != cursor.stream_id || v.seq <= cursor.seq)
        {
            self.cursor = Some(cursor);
        }
    }
    pub fn fresh(&mut self, cursor: ResumeCursor) {
        self.cursor = Some(cursor);
        self.state = ConnectionState::Ready;
    }
    pub fn healthy(&mut self) {
        self.attempts = 0;
    }
    pub fn resync(&mut self) {
        self.cursor = None;
        self.state = ConnectionState::Connecting;
    }
}

fn event_channel(e: &Event) -> Option<Id> {
    match e {
        Event::MessageCreated { message, .. } | Event::MessageUpdated(message) => {
            Some(message.channel_id)
        }
        Event::MessageDeleted { channel_id, .. } | Event::Typing { channel_id, .. } => {
            Some(*channel_id)
        }
        Event::ReactionUpdated(v) => Some(v.channel_id),
        _ => None,
    }
}
fn event_bytes(e: &EventEnvelope) -> usize {
    // Reactions, mentions, user summaries and future enum payloads count too.
    rmp_serde::to_vec_named(e).map_or(2 * 1024 * 1024 + 1, |v| v.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Model {
        let mut m = Model {
            cursor: Some(ResumeCursor {
                stream_id: "a".into(),
                seq: 0,
            }),
            ..Model::default()
        };
        m.channels.insert(
            Id(1),
            ChannelSummary {
                id: Id(1),
                kind: ChannelKind::Text,
                guild_id: None,
                parent_id: None,
                name: Some("test".into()),
                position: 0,
                recipient_ids: vec![],
                owner_id: None,
            },
        );
        m.touch(Id(1));
        m
    }
    fn msg(id: u64, content: &str) -> Message {
        Message {
            id: Id(id),
            channel_id: Id(1),
            author_id: Id(2),
            content: content.into(),
            created_at: 0,
            edited_at: None,
            reply_to: None,
            attachments: vec![],
            mention_ids: vec![],
            reactions: vec![],
        }
    }
    #[test]
    fn ordered_upserts_and_duplicate_sequences() {
        let mut m = model();
        for (seq, id) in [(1, 4), (3, 2), (7, 3)] {
            assert!(m.apply(EventEnvelope {
                seq,
                event: Event::MessageUpdated(msg(id, "old"))
            }));
        }
        assert!(!m.apply(EventEnvelope {
            seq: 3,
            event: Event::MessageUpdated(msg(2, "bad"))
        }));
        m.apply(EventEnvelope {
            seq: 9,
            event: Event::MessageUpdated(msg(3, "edited")),
        });
        assert_eq!(
            m.messages[&Id(1)].keys().copied().collect::<Vec<_>>(),
            vec![Id(2), Id(3), Id(4)]
        );
        assert_eq!(m.messages[&Id(1)][&Id(3)].content, "edited");
    }
    #[test]
    fn pagination_replays_deletion_and_edit() {
        let mut m = model();
        m.apply(EventEnvelope {
            seq: 2,
            event: Event::MessageDeleted {
                channel_id: Id(1),
                message_id: Id(2),
            },
        });
        m.apply(EventEnvelope {
            seq: 5,
            event: Event::MessageUpdated(msg(3, "new")),
        });
        let page = Page {
            items: vec![msg(2, "deleted"), msg(3, "stale"), msg(1, "older")],
            users: vec![],
            next_before: None,
            next_after: None,
            has_more: false,
            cursor: ResumeCursor {
                stream_id: "a".into(),
                seq: 0,
            },
        };
        assert!(m.merge_page(Id(1), page.clone()));
        assert!(!m.messages[&Id(1)].contains_key(&Id(2)));
        assert_eq!(m.messages[&Id(1)][&Id(3)].content, "new");
        let mut stale = page;
        stale.cursor.stream_id = "b".into();
        assert!(!m.merge_page(Id(1), stale));
        assert_eq!(m.cursor.as_ref().unwrap().seq, 5);
    }
    #[test]
    fn read_counts_are_authoritative_and_idempotent() {
        let mut m = model();
        for seq in 1..=2 {
            m.apply(EventEnvelope {
                seq,
                event: Event::ReadStateUpdated(ReadState {
                    channel_id: Id(1),
                    last_read_id: None,
                    latest_message_id: Some(Id(3)),
                    unread_count: 2,
                    mention_count: 1,
                }),
            });
        }
        m.apply(EventEnvelope {
            seq: 3,
            event: Event::MessageUpdated(msg(4, "own send does not imply read")),
        });
        assert_eq!(m.reads[&Id(1)].unread_count, 2);
        assert_eq!(m.reads[&Id(1)].mention_count, 1);
    }
    #[test]
    fn bounded_cache_and_purge() {
        let mut m = model();
        for id in 1..=10_000 {
            m.insert(msg(id, "hello"));
            m.enforce_bounds();
        }
        assert_eq!(m.retained_count(), MESSAGE_LIMIT);
        m.purge_channel(Id(1));
        assert_eq!(m.retained_bytes(), 0);
    }
    #[test]
    fn reconnect_resume_backoff_and_terminal_close() {
        let mut r = Recovery::default();
        r.fresh(ResumeCursor {
            stream_id: "a".into(),
            seq: 10,
        });
        assert_eq!(
            (0..7)
                .map(|_| r.disconnected(None).unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 4, 8, 16, 30, 30]
        );
        assert_eq!(r.cursor.as_ref().unwrap().seq, 10);
        r.applied(ResumeCursor {
            stream_id: "a".into(),
            seq: 8,
        });
        assert_eq!(r.cursor.as_ref().unwrap().seq, 10);
        r.healthy();
        assert_eq!(r.disconnected(None), Some(1));
        assert_eq!(r.disconnected(Some(4009)), None);
        r.resync();
        assert!(r.cursor.is_none());
    }
    #[test]
    fn replay_budget_includes_reactions_and_mentions() {
        let mut model = model();
        let mut message = msg(1, "with reactions");
        message.mention_ids = (1..=100).map(Id).collect();
        message.reactions = (0..20)
            .map(|n| ReactionSummary {
                emoji: format!("emoji-{n}-with-modifiers"),
                count: 4,
                me: false,
            })
            .collect();
        let event = EventEnvelope {
            seq: 1,
            event: Event::MessageUpdated(message),
        };
        assert_eq!(
            event_bytes(&event),
            rmp_serde::to_vec_named(&event).unwrap().len()
        );
        for seq in 1..=3000 {
            let mut e = event.clone();
            e.seq = seq;
            model.apply(e);
        }
        assert!(model.journal_bytes <= 2 * 1024 * 1024);
        assert_eq!(
            model.journal_bytes,
            model.journal.iter().map(event_bytes).sum::<usize>()
        );
    }
}
