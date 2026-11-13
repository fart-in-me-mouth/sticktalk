//! Launch projections and presentation rules shared by native and browser hosts.
use std::collections::{BTreeMap, BTreeSet};
use sticktalk_proto::*;

#[derive(Debug, Clone)]
pub enum Read {
    Profile(Id),
    Pins(Id),
    Overwrites(Id),
    Extent(Id),
}
#[derive(Debug, Clone)]
pub enum SnapshotData {
    Profile(Snapshot<ProfileDetail>),
    Pins(Id, Snapshot<Vec<Message>>),
    Overwrites(Id, Snapshot<Vec<PermissionOverwrite>>),
    Extent(Snapshot<HistoryExtent>),
}
impl SnapshotData {
    pub fn cursor(&self) -> &ResumeCursor {
        match self {
            Self::Profile(v) => &v.cursor,
            Self::Pins(_, v) => &v.cursor,
            Self::Overwrites(_, v) => &v.cursor,
            Self::Extent(v) => &v.cursor,
        }
    }
}
#[derive(Default)]
pub struct Projection {
    pub roles: BTreeMap<Id, Vec<Role>>,
    pub memberships: BTreeMap<(Id, Id), Member>,
    pub notifications: BTreeMap<(Option<Id>, Option<Id>), NotificationSettings>,
    pub profiles: BTreeMap<Id, ProfileDetail>,
    pub pins: BTreeMap<Id, Vec<Message>>,
    pub overwrites: BTreeMap<Id, Vec<PermissionOverwrite>>,
    pub extents: BTreeMap<Id, HistoryExtent>,
    pub voice: BTreeMap<VoiceSession, GuildVoiceState>,
    pub speaking: BTreeSet<VoiceSession>,
    pub rtt_ms: Option<u32>,
    pub voice_error: Option<String>,
}
impl Projection {
    pub fn ready(&mut self, ready: &Ready) {
        *self = Self::default();
        for r in &ready.roles {
            self.roles.entry(r.guild_id).or_default().push(r.clone());
        }
        for m in &ready.memberships {
            self.member(m);
        }
        for n in &ready.notification_settings {
            self.notification(n.clone());
        }
        for v in &ready.voice_states {
            self.voice.insert(v.participant.session, v.clone());
        }
    }
    pub fn member(&mut self, m: &Member) {
        self.memberships.insert((m.guild_id, m.user.id), m.clone());
    }
    pub fn notification(&mut self, n: NotificationSettings) {
        self.notifications.insert((n.guild_id, n.channel_id), n);
    }
    pub fn event(&mut self, e: &Event) {
        match e {
            Event::RolesUpdated { guild_id, roles } => {
                self.roles.insert(*guild_id, roles.clone());
            }
            Event::MemberUpsert(m) => self.member(m),
            Event::MemberRemoved {
                guild_id, user_id, ..
            } => {
                self.memberships.remove(&(*guild_id, *user_id));
            }
            Event::NotificationSettingsUpdated(n) => self.notification(n.clone()),
            Event::RelationshipRemoved { user_id } => {
                self.profiles.remove(user_id);
            }
            Event::ProfileUpdated(p) => {
                if let Some(v) = self.profiles.get_mut(&p.user.id) {
                    v.profile = p.clone();
                }
            }
            Event::PinsUpdated {
                channel_id,
                message_id,
                pinned,
            } => {
                // Pin bodies are fetched on demand. An unpin can always be applied immediately.
                if !pinned {
                    if let Some(p) = self.pins.get_mut(channel_id) {
                        p.retain(|m| m.id != *message_id);
                    }
                }
            }
            Event::MessageDeleted {
                channel_id,
                message_id,
            } => {
                if let Some(p) = self.pins.get_mut(channel_id) {
                    p.retain(|m| m.id != *message_id);
                }
            }
            Event::MessageUpdated(m) => {
                if let Some(p) = self.pins.get_mut(&m.channel_id) {
                    for old in p {
                        if old.id == m.id {
                            *old = m.clone();
                        }
                    }
                }
            }
            Event::GuildRemoved { guild_id, .. } => {
                self.roles.remove(guild_id);
                self.memberships.retain(|(g, _), _| g != guild_id);
                self.voice.retain(|_, v| v.guild_id != *guild_id);
                self.notifications.retain(|(g, _), _| *g != Some(*guild_id));
                self.profiles.clear();
            }
            Event::ChannelRemoved { channel_id, .. } => self.purge_channel(*channel_id),
            _ => {}
        }
    }
    pub fn purge_channel(&mut self, id: Id) {
        self.profiles.clear();
        self.pins.remove(&id);
        self.extents.remove(&id);
        self.overwrites.remove(&id);
        self.notifications.retain(|(_, c), _| *c != Some(id));
        self.voice.retain(|_, v| v.channel_id != id);
        self.speaking.retain(|s| self.voice.contains_key(s));
    }
    pub fn voice_event(&mut self, e: VoiceEvent) {
        match e {
            VoiceEvent::GuildState(v) => {
                self.voice.insert(v.participant.session, v);
            }
            VoiceEvent::GuildLeft { session, .. } | VoiceEvent::Left { session } => {
                self.voice.remove(&session);
                self.speaking.remove(&session);
            }
            VoiceEvent::GuildSnapshot { states } => {
                self.voice = states
                    .into_iter()
                    .map(|v| (v.participant.session, v))
                    .collect();
                self.speaking.retain(|s| self.voice.contains_key(s));
            }
            VoiceEvent::Participant(p) => {
                if let Some(v) = self.voice.get_mut(&p.session) {
                    v.participant = p;
                }
            }
            VoiceEvent::Speaking {
                session, speaking, ..
            } => {
                if self.voice.contains_key(&session) {
                    if speaking {
                        self.speaking.insert(session);
                    } else {
                        self.speaking.remove(&session);
                    }
                }
            }
            VoiceEvent::Stats(v) => self.rtt_ms = v.rtt_ms,
            VoiceEvent::Error { message, .. } => {
                self.voice_error = Some(message.chars().take(512).collect())
            }
            _ => {}
        }
    }
    pub fn snapshot(&mut self, data: SnapshotData) {
        match data {
            SnapshotData::Profile(v) => {
                bounded_insert(&mut self.profiles, v.data.profile.user.id, v.data, 32);
            }
            SnapshotData::Pins(id, v) => bounded_insert(&mut self.pins, id, v.data, 4),
            SnapshotData::Overwrites(id, v) => {
                bounded_insert(&mut self.overwrites, id, v.data, 256)
            }
            SnapshotData::Extent(v) => {
                bounded_insert(&mut self.extents, v.data.channel_id, v.data, 4)
            }
        }
    }
    pub fn member_roles(&self, guild: Id, user: Id) -> Vec<&Role> {
        let m = self.memberships.get(&(guild, user));
        let mut roles: Vec<_> = self
            .roles
            .get(&guild)
            .into_iter()
            .flatten()
            .filter(|r| r.id == guild || m.is_some_and(|m| m.role_ids.contains(&r.id)))
            .collect();
        roles.sort_by_key(|r| (std::cmp::Reverse(r.position), r.id));
        roles
    }
    pub fn color(&self, guild: Option<Id>, user: Id) -> Option<u32> {
        guild.and_then(|g| {
            self.member_roles(g, user)
                .into_iter()
                .find(|r| r.color != 0)
                .map(|r| r.color)
        })
    }
    pub fn name(&self, guild: Option<Id>, user: Id) -> Option<&str> {
        guild
            .and_then(|g| self.memberships.get(&(g, user)))
            .and_then(|m| m.nickname.as_deref())
    }
    pub fn hoist(&self, guild: Id, user: Id) -> Option<&Role> {
        self.member_roles(guild, user).into_iter().find(|r| r.hoist)
    }
    pub fn settings(&self, guild: Option<Id>, channel: Option<Id>) -> NotificationSettings {
        self.notifications
            .get(&(guild, channel))
            .or_else(|| self.notifications.get(&(guild, None)))
            .cloned()
            .unwrap_or(NotificationSettings {
                guild_id: guild,
                channel_id: channel,
                level: NotificationLevel::All,
                muted: false,
                mute_until: None,
                suppress_everyone: false,
                hide_muted_channels: false,
            })
    }
    pub fn muted(&self, guild: Option<Id>, channel: Id, now: u64) -> bool {
        let n = self.settings(guild, Some(channel));
        n.muted && n.mute_until.is_none_or(|t| t > now)
    }
    pub fn is_private(&self, channel: &ChannelSummary) -> bool {
        let Some(guild) = channel.guild_id else {
            return false;
        };
        let mut private = self
            .roles
            .get(&guild)
            .and_then(|rs| rs.iter().find(|r| r.id == guild))
            .is_some_and(|r| !r.permissions.contains(Permissions::VIEW_CHANNEL));
        for scope in [channel.parent_id, Some(channel.id)].into_iter().flatten() {
            for o in self
                .overwrites
                .get(&scope)
                .into_iter()
                .flatten()
                .filter(|o| o.kind == OverwriteKind::Role && o.target_id == guild)
            {
                if o.deny.contains(Permissions::VIEW_CHANNEL) {
                    private = true;
                } else if o.allow.contains(Permissions::VIEW_CHANNEL) {
                    private = false;
                }
            }
        }
        private
    }
}
fn bounded_insert<K: Ord, V>(map: &mut BTreeMap<K, V>, key: K, value: V, max: usize) {
    if max == 0 {
        map.clear();
        return;
    }
    if !map.contains_key(&key) && map.len() >= max {
        map.pop_first();
    }
    map.insert(key, value);
}
/// Predictive UI gating only. The server remains authoritative. Missing ACLs never
/// invent grants; owner/admin bypass follows the server evaluator.
pub fn permissions(model: &crate::model::Model, channel: Id) -> Permissions {
    let Some(c) = model.channels.get(&channel) else {
        return Permissions::empty();
    };
    let Some(guild) = c.guild_id else {
        return Permissions::VIEW_CHANNEL
            | Permissions::READ_HISTORY
            | Permissions::SEND_MESSAGES
            | Permissions::ATTACH_FILES
            | Permissions::ADD_REACTIONS
            | Permissions::MANAGE_MESSAGES;
    };
    let Some(me) = model.me.as_ref().map(|m| m.user.id) else {
        return Permissions::empty();
    };
    if model
        .guilds
        .get(&guild)
        .is_some_and(|g| g.guild.owner_id == me)
    {
        return Permissions::all();
    }
    let roles = model.launch.member_roles(guild, me);
    let mut p = if roles.is_empty() {
        model.permissions(Some(guild))
    } else {
        roles
            .iter()
            .fold(Permissions::empty(), |p, r| p | r.permissions)
    };
    p.remove(Permissions::OWNER);
    if p.contains(Permissions::ADMINISTRATOR) {
        return Permissions::all() & !Permissions::OWNER;
    }
    for scope in [c.parent_id, Some(channel)].into_iter().flatten() {
        let Some(ows) = model.launch.overwrites.get(&scope) else {
            continue;
        };
        let apply = |p: Permissions, deny: Permissions, allow: Permissions| (p & !deny) | allow;
        for o in ows
            .iter()
            .filter(|o| o.kind == OverwriteKind::Role && o.target_id == guild)
        {
            p = apply(p, o.deny, o.allow);
        }
        let mut deny = Permissions::empty();
        let mut allow = Permissions::empty();
        for o in ows.iter().filter(|o| {
            o.kind == OverwriteKind::Role
                && o.target_id != guild
                && roles.iter().any(|r| r.id == o.target_id)
        }) {
            deny |= o.deny;
            allow |= o.allow;
        }
        p = apply(p, deny, allow);
        for o in ows
            .iter()
            .filter(|o| o.kind == OverwriteKind::Member && o.target_id == me)
        {
            p = apply(p, o.deny, o.allow);
        }
    }
    p & Permissions::all()
}
/// Stable subsequence matching with contiguous/word-start bonuses. IDs break ties.
pub fn fuzzy_score(query: &str, label: &str) -> Option<i32> {
    let q = query.to_lowercase();
    let label = label.to_lowercase();
    let mut chars = q.chars().filter(|c| !c.is_whitespace()).peekable();
    let mut score = 0;
    let mut run = 0;
    for (i, c) in label.chars().enumerate() {
        if chars.peek() == Some(&c) {
            chars.next();
            run += 1;
            score += 10 + run * 4 + if i == 0 { 20 } else { 0 };
        } else {
            run = 0;
            score -= 1;
        }
    }
    chars.next().is_none().then_some(score)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fuzzy_is_subsequence_not_substring() {
        assert!(fuzzy_score("gnrl", "general").is_some());
        assert!(fuzzy_score("grx", "general").is_none());
        assert!(
            fuzzy_score("gen", "general").unwrap() > fuzzy_score("gen", "great evening").unwrap()
        );
    }
    #[test]
    fn nickname_color_and_hoist_use_highest_assigned_role() {
        let mut p = Projection::default();
        let mut m = crate::mock::Mock::new().model.members[&Id(2)].clone();
        m.nickname = Some("Mika".into());
        m.role_ids = vec![Id(10), Id(90), Id(91)];
        p.member(&m);
        p.roles.insert(
            Id(10),
            vec![
                Role {
                    id: Id(90),
                    guild_id: Id(10),
                    name: "Low".into(),
                    color: 0xffaa00,
                    position: 1,
                    hoist: true,
                    mentionable: false,
                    permissions: Permissions::empty(),
                },
                Role {
                    id: Id(91),
                    guild_id: Id(10),
                    name: "High".into(),
                    color: 0x3388ff,
                    position: 2,
                    hoist: true,
                    mentionable: false,
                    permissions: Permissions::empty(),
                },
            ],
        );
        assert_eq!(p.name(Some(Id(10)), Id(2)), Some("Mika"));
        assert_eq!(p.color(Some(Id(10)), Id(2)), Some(0x3388ff));
        assert_eq!(p.hoist(Id(10), Id(2)).unwrap().id, Id(91));
    }
    #[test]
    fn bounded_cache_keeps_a_new_low_id() {
        let mut map = BTreeMap::from([(10, 10), (20, 20)]);
        bounded_insert(&mut map, 1, 1, 2);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&1), Some(&1));
    }
    #[test]
    fn guild_settings_do_not_inherit_the_selected_channel_acl() {
        let mut model = crate::mock::Mock::new().model;
        model.guilds.get_mut(&Id(10)).unwrap().guild.owner_id = Id(2);
        model
            .launch
            .roles
            .get_mut(&Id(10))
            .unwrap()
            .iter_mut()
            .find(|r| r.id == Id(90))
            .unwrap()
            .permissions |= Permissions::MANAGE_GUILD;
        model.launch.overwrites.insert(
            Id(12),
            vec![PermissionOverwrite {
                kind: OverwriteKind::Member,
                target_id: Id(1),
                allow: Permissions::empty(),
                deny: Permissions::MANAGE_MESSAGES | Permissions::MANAGE_GUILD,
            }],
        );
        assert!(model
            .permissions(Some(Id(10)))
            .contains(Permissions::MANAGE_GUILD));
        assert!(!permissions(&model, Id(12))
            .intersects(Permissions::MANAGE_MESSAGES | Permissions::MANAGE_GUILD));
    }
}
