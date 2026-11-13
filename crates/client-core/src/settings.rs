//! Settings data and authorization shared by desktop and future hosts. No audio/OS work.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use sticktalk_proto::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query {
    Roles(Id),
    GuildProfile(Id),
    Profile(Id),
    Overwrites(Id),
    Notifications,
    Sessions,
}
impl Query {
    pub fn path(self) -> String {
        match self {
            Self::Roles(id) => format!("/guilds/{}/roles", id.0),
            Self::GuildProfile(id) => format!("/guilds/{}/preview", id.0),
            Self::Profile(id) => format!("/users/{}/profile", id.0),
            Self::Overwrites(id) => format!("/channels/{}/overwrites", id.0),
            Self::Notifications => "/me/notifications".into(),
            Self::Sessions => "/me/sessions".into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SnapshotData {
    Roles(Vec<Role>),
    GuildProfile(GuildPreview),
    Profile(ProfileDetail),
    Overwrites(Id, Vec<PermissionOverwrite>),
    Notifications(Vec<NotificationSettings>),
    Sessions(Vec<DeviceSession>),
}
#[derive(Default)]
pub struct Projection {
    pub roles: BTreeMap<Id, Role>,
    pub memberships: BTreeMap<Id, Member>,
    pub guild_profiles: BTreeMap<Id, GuildPreview>,
    pub profiles: BTreeMap<Id, UserProfile>,
    pub overwrites: BTreeMap<Id, Vec<PermissionOverwrite>>,
    pub notifications: BTreeMap<(Option<Id>, Option<Id>), NotificationSettings>,
    pub sessions: Vec<DeviceSession>,
}
impl Projection {
    pub fn ingest(&mut self, data: SnapshotData) {
        match data {
            SnapshotData::Roles(roles) => {
                if let Some(r) = roles.first() {
                    let guild = r.guild_id;
                    self.roles.retain(|_, r| r.guild_id != guild);
                }
                self.roles.extend(roles.into_iter().map(|r| (r.id, r)));
            }
            SnapshotData::GuildProfile(v) => {
                self.guild_profiles.insert(v.guild.id, v);
            }
            SnapshotData::Profile(v) => {
                self.profiles.insert(v.profile.user.id, v.profile);
            }
            SnapshotData::Overwrites(id, v) => {
                self.overwrites.insert(id, v);
            }
            SnapshotData::Notifications(v) => {
                self.notifications = v
                    .into_iter()
                    .map(|s| ((s.guild_id, s.channel_id), s))
                    .collect();
            }
            SnapshotData::Sessions(v) => self.sessions = v,
        }
    }
    pub fn event(&mut self, event: &Event, me: Option<Id>) {
        match event {
            Event::RolesUpdated { guild_id, roles } => {
                self.roles.retain(|_, r| r.guild_id != *guild_id);
                self.roles.extend(roles.iter().map(|r| (r.id, r.clone())));
            }
            Event::ProfileUpdated(v) => {
                self.profiles.insert(v.user.id, v.clone());
            }
            Event::GuildProfileUpdated(v) => {
                if let Some(p) = self.guild_profiles.get_mut(&v.guild_id) {
                    p.profile = v.clone();
                }
            }
            Event::GuildUpsert(v) => {
                if let Some(p) = self.guild_profiles.get_mut(&v.guild.id) {
                    p.guild = v.guild.clone();
                }
            }
            Event::NotificationSettingsUpdated(v) => {
                self.notifications
                    .insert((v.guild_id, v.channel_id), v.clone());
            }
            Event::MemberUpsert(v) if Some(v.user.id) == me => {
                self.memberships.insert(v.guild_id, v.clone());
            }
            Event::GuildRemoved { guild_id, .. } => {
                self.roles.retain(|_, r| r.guild_id != *guild_id);
                self.guild_profiles.remove(guild_id);
                self.memberships.remove(guild_id);
                self.notifications.retain(|(g, _), _| *g != Some(*guild_id));
            }
            Event::ChannelRemoved { channel_id, .. } => {
                self.overwrites.remove(channel_id);
                self.notifications
                    .retain(|(_, c), _| *c != Some(*channel_id));
            }
            _ => {}
        }
        // Profile reads are on-demand; bound unsolicited shared-server updates.
        while self.profiles.len() > 1024 {
            let id = self.profiles.keys().find(|id| Some(**id) != me).copied();
            if let Some(id) = id {
                self.profiles.remove(&id);
            } else {
                break;
            }
        }
    }
    pub fn notification(&self, guild: Id) -> NotificationSettings {
        self.notifications
            .get(&(Some(guild), None))
            .cloned()
            .unwrap_or(NotificationSettings {
                guild_id: Some(guild),
                channel_id: None,
                level: NotificationLevel::All,
                muted: false,
                mute_until: None,
                suppress_everyone: false,
                hide_muted_channels: false,
            })
    }
    pub fn highest(&self, guild: Id, ids: &[Id]) -> u16 {
        ids.iter()
            .filter_map(|id| self.roles.get(id))
            .filter(|r| r.guild_id == guild)
            .map(|r| r.position)
            .max()
            .unwrap_or(0)
    }
    pub fn can_edit_role(&self, guild: &Guild, me: Id, role: &Role) -> bool {
        if guild.owner_id == me {
            return true;
        }
        let Some(m) = self.memberships.get(&guild.id) else {
            return false;
        };
        self.guild_permissions(guild, me)
            .contains(Permissions::MANAGE_ROLES)
            && role.position < self.highest(guild.id, &m.role_ids)
    }
    pub fn guild_permissions(&self, guild: &Guild, me: Id) -> Permissions {
        if guild.owner_id == me {
            return Permissions::all();
        }
        let mut p = self
            .roles
            .get(&guild.id)
            .map_or(Permissions::empty(), |r| r.permissions);
        if let Some(m) = self.memberships.get(&guild.id) {
            for r in m.role_ids.iter().filter_map(|id| self.roles.get(id)) {
                p |= r.permissions;
            }
        }
        p.remove(Permissions::OWNER);
        if p.contains(Permissions::ADMINISTRATOR) {
            Permissions::all() - Permissions::OWNER
        } else {
            p & Permissions::all()
        }
    }
    /// Same category → channel precedence as the server; this is presentation only.
    /// The server remains authoritative and rechecks every submitted mutation.
    pub fn channel_permissions(
        &self,
        guild: &Guild,
        me: Id,
        parent: Option<Id>,
        channel: Id,
    ) -> Permissions {
        let mut p = self.guild_permissions(guild, me);
        if guild.owner_id == me || p.contains(Permissions::ADMINISTRATOR) {
            return p;
        }
        let ids = self
            .memberships
            .get(&guild.id)
            .map(|m| m.role_ids.as_slice())
            .unwrap_or(&[]);
        for scope in parent.into_iter().chain(std::iter::once(channel)) {
            let Some(overwrites) = self.overwrites.get(&scope) else {
                continue;
            };
            if let Some(o) = overwrites
                .iter()
                .find(|o| o.kind == OverwriteKind::Role && o.target_id == guild.id)
            {
                p = (p - o.deny) | o.allow;
            }
            let (mut deny, mut allow) = (Permissions::empty(), Permissions::empty());
            for o in overwrites.iter().filter(|o| {
                o.kind == OverwriteKind::Role
                    && o.target_id != guild.id
                    && ids.contains(&o.target_id)
            }) {
                deny |= o.deny;
                allow |= o.allow;
            }
            p = (p - deny) | allow;
            if let Some(o) = overwrites
                .iter()
                .find(|o| o.kind == OverwriteKind::Member && o.target_id == me)
            {
                p = (p - o.deny) | o.allow;
            }
        }
        p & (Permissions::all() - Permissions::OWNER - Permissions::ADMINISTRATOR)
    }
    /// Complete lowest-to-highest reorder; boundaries cannot move even for administrators.
    pub fn reorder(
        &self,
        guild: &Guild,
        me: Id,
        dragged: Id,
        before: Option<Id>,
    ) -> Option<Vec<Id>> {
        let role = self.roles.get(&dragged)?;
        if dragged == guild.id || !self.can_edit_role(guild, me, role) {
            return None;
        }
        let mut roles: Vec<_> = self
            .roles
            .values()
            .filter(|r| r.guild_id == guild.id)
            .collect();
        roles.sort_by_key(|r| r.position);
        let original: Vec<_> = roles.iter().map(|r| r.id).collect();
        let mut ids = original.clone();
        ids.retain(|id| *id != dragged);
        let index = match before {
            Some(id) => ids.iter().position(|v| *v == id)?,
            None => ids.len(),
        };
        if index == 0 {
            return None;
        }
        ids.insert(index, dragged);
        for r in roles {
            if !self.can_edit_role(guild, me, r)
                && original.iter().position(|id| *id == r.id)
                    != ids.iter().position(|id| *id == r.id)
            {
                return None;
            }
        }
        Some(ids)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriState {
    Deny,
    Neutral,
    Allow,
}
pub fn set_overwrite(v: &mut PermissionOverwrite, permission: Permissions, state: TriState) {
    v.allow.remove(permission);
    v.deny.remove(permission);
    match state {
        TriState::Allow => v.allow.insert(permission),
        TriState::Deny => v.deny.insert(permission),
        TriState::Neutral => {}
    }
}
pub fn private_channel(v: &mut Vec<PermissionOverwrite>, guild: Id, private: bool) {
    if !v
        .iter()
        .any(|o| o.kind == OverwriteKind::Role && o.target_id == guild)
    {
        v.push(PermissionOverwrite {
            kind: OverwriteKind::Role,
            target_id: guild,
            allow: Permissions::empty(),
            deny: Permissions::empty(),
        });
    }
    let everyone = v
        .iter_mut()
        .find(|o| o.kind == OverwriteKind::Role && o.target_id == guild)
        .unwrap();
    set_overwrite(
        everyone,
        Permissions::VIEW_CHANNEL,
        if private {
            TriState::Deny
        } else {
            TriState::Neutral
        },
    );
}

/// Keep already-attached images unchanged: an edit must not try to claim its
/// existing attachment again, including after a partially successful save.
pub fn guild_patch(current: &Guild, name: &str, icon: Option<Id>) -> GuildPatch {
    GuildPatch {
        name: if current.name == name {
            Patch::Keep
        } else {
            Patch::Set(name.to_owned())
        },
        icon_id: if current.icon_id == icon {
            Patch::Keep
        } else {
            icon.map_or(Patch::Clear, Patch::Set)
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoiseSuppressionMode {
    Off,
    Standard,
    High,
}
impl std::fmt::Display for NoiseSuppressionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSettings {
    /// None follows the system default; pinned IDs are opaque engine device IDs.
    pub input_device_id: Option<String>,
    pub output_device_id: Option<String>,
    pub input_volume: f32,
    pub output_volume: f32,
    pub sensitivity_auto: bool,
    pub sensitivity_manual_db: f32,
    pub noise_suppression_mode: NoiseSuppressionMode,
    pub echo_cancellation: bool,
    pub automatic_gain_control: bool,
    pub push_to_talk: bool,
    pub push_to_talk_key: String,
    pub push_to_talk_release_delay_ms: u16,
    pub studio_mode: bool,
}
impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            input_device_id: None,
            output_device_id: None,
            input_volume: 100.,
            output_volume: 100.,
            sensitivity_auto: true,
            sensitivity_manual_db: -50.,
            noise_suppression_mode: NoiseSuppressionMode::High,
            echo_cancellation: true,
            automatic_gain_control: true,
            push_to_talk: false,
            push_to_talk_key: "Ctrl+Shift+V".into(),
            push_to_talk_release_delay_ms: 200,
            studio_mode: false,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Density {
    Cozy,
    Compact,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub voice: VoiceSettings,
    pub density: Density,
    pub chat_font_scale: f32,
    pub saturation: f32,
    pub underline_links: bool,
    pub developer_mode: bool,
    pub allow_direct_messages: bool,
    pub friend_requests: bool,
    pub notification_level: NotificationLevel,
    pub notification_sounds: bool,
    pub start_with_windows: bool,
    pub keybinds: BTreeMap<String, String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            voice: VoiceSettings::default(),
            density: Density::Cozy,
            chat_font_scale: 1.,
            saturation: 1.,
            underline_links: true,
            developer_mode: false,
            allow_direct_messages: true,
            friend_requests: true,
            notification_level: NotificationLevel::Mentions,
            notification_sounds: false,
            start_with_windows: false,
            keybinds: [
                ("Toggle Mute".into(), "Ctrl+Shift+M".into()),
                ("Toggle Deafen".into(), "Ctrl+Shift+D".into()),
                ("Quick Switcher".into(), "Ctrl+K".into()),
            ]
            .into(),
        }
    }
}
impl Preferences {
    pub fn validate(&mut self) {
        self.chat_font_scale = self.chat_font_scale.clamp(0.8, 1.5);
        self.saturation = self.saturation.clamp(0., 1.);
        self.voice.input_volume = self.voice.input_volume.clamp(0., 200.);
        self.voice.output_volume = self.voice.output_volume.clamp(0., 200.);
        self.voice.sensitivity_manual_db = self.voice.sensitivity_manual_db.clamp(-100., 0.);
        self.voice.push_to_talk_release_delay_ms =
            self.voice.push_to_talk_release_delay_ms.min(2000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_attached_icons_are_kept_instead_of_reclaimed() {
        let g = Guild {
            id: Id(10),
            owner_id: Id(1),
            name: "Test".into(),
            icon_id: Some(Id(100)),
            member_count: 1,
        };
        assert_eq!(
            guild_patch(&g, "Test", Some(Id(100))),
            GuildPatch::default()
        );
        assert_eq!(
            guild_patch(&g, "Renamed", None),
            GuildPatch {
                name: Patch::Set("Renamed".into()),
                icon_id: Patch::Clear
            }
        );
        assert_eq!(
            guild_patch(&g, "Test", Some(Id(101))).icon_id,
            Patch::Set(Id(101))
        );
    }
    #[test]
    fn private_preserves_other_bits_and_tristates_never_overlap() {
        let mut v = vec![];
        private_channel(&mut v, Id(10), true);
        v[0].allow.insert(Permissions::SEND_MESSAGES);
        set_overwrite(&mut v[0], Permissions::VIEW_CHANNEL, TriState::Allow);
        assert!(!v[0].allow.intersects(v[0].deny));
        private_channel(&mut v, Id(10), false);
        assert_eq!(v[0].allow, Permissions::SEND_MESSAGES);
    }
    #[test]
    fn voice_defaults_are_inert_and_preferences_roundtrip() {
        let p = Preferences::default();
        assert_eq!(p.voice.input_device_id, None);
        assert!(!p.voice.studio_mode);
        let mut p: Preferences = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        p.chat_font_scale = 9.;
        p.validate();
        assert_eq!(p.chat_font_scale, 1.5);
    }
    fn hierarchy() -> (Projection, Guild) {
        let mut p = Projection::default();
        for (id, position, permissions) in [
            (10, 0, Permissions::VIEW_CHANNEL),
            (20, 1, Permissions::SEND_MESSAGES),
            (30, 2, Permissions::empty()),
            (40, 3, Permissions::ADMINISTRATOR),
        ] {
            p.roles.insert(
                Id(id),
                Role {
                    id: Id(id),
                    guild_id: Id(10),
                    name: format!("role-{id}"),
                    color: 0,
                    position,
                    permissions,
                    hoist: false,
                    mentionable: false,
                },
            );
        }
        p.memberships.insert(
            Id(10),
            Member {
                guild_id: Id(10),
                user: UserSummary {
                    id: Id(2),
                    ..Default::default()
                },
                role_ids: vec![Id(10), Id(40)],
                ..Default::default()
            },
        );
        (
            p,
            Guild {
                id: Id(10),
                owner_id: Id(1),
                name: "Test".into(),
                icon_id: None,
                member_count: 2,
            },
        )
    }
    #[test]
    fn administrator_never_bypasses_role_hierarchy_or_moves_everyone() {
        let (p, g) = hierarchy();
        assert!(p.can_edit_role(&g, Id(2), &p.roles[&Id(30)]));
        assert!(!p.can_edit_role(&g, Id(2), &p.roles[&Id(40)]));
        assert_eq!(
            p.reorder(&g, Id(2), Id(30), Some(Id(20))),
            Some(vec![Id(10), Id(30), Id(20), Id(40)])
        );
        assert!(p.reorder(&g, Id(2), Id(20), None).is_none());
        assert!(p.reorder(&g, Id(1), Id(10), None).is_none());
        assert!(p.reorder(&g, Id(2), Id(30), Some(Id(10))).is_none());
    }
    #[test]
    fn category_then_channel_and_member_overwrites_follow_server_precedence() {
        let (mut p, g) = hierarchy();
        p.roles.get_mut(&Id(40)).unwrap().permissions = Permissions::MANAGE_ROLES;
        p.overwrites.insert(
            Id(11),
            vec![PermissionOverwrite {
                kind: OverwriteKind::Role,
                target_id: g.id,
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
            }],
        );
        assert!(!p
            .channel_permissions(&g, Id(2), Some(Id(11)), Id(12))
            .contains(Permissions::VIEW_CHANNEL));
        p.overwrites.insert(
            Id(12),
            vec![PermissionOverwrite {
                kind: OverwriteKind::Role,
                target_id: Id(40),
                allow: Permissions::VIEW_CHANNEL,
                deny: Permissions::empty(),
            }],
        );
        assert!(p
            .channel_permissions(&g, Id(2), Some(Id(11)), Id(12))
            .contains(Permissions::VIEW_CHANNEL));
        p.overwrites
            .get_mut(&Id(12))
            .unwrap()
            .push(PermissionOverwrite {
                kind: OverwriteKind::Member,
                target_id: Id(2),
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
            });
        assert!(!p
            .channel_permissions(&g, Id(2), Some(Id(11)), Id(12))
            .contains(Permissions::VIEW_CHANNEL));
        assert!(p
            .channel_permissions(&g, Id(1), Some(Id(11)), Id(12))
            .contains(Permissions::all()));
    }
    #[test]
    fn racing_settings_snapshots_replay_metadata_and_refetch_session_invalidations() {
        let mut model = crate::mock::Mock::new().model;
        let cursor = model.cursor.clone().unwrap();
        let old = model.settings.roles[&Id(10)].clone();
        let mut newer = old.clone();
        newer.permissions.insert(Permissions::MANAGE_ROLES);
        model.apply(EventEnvelope {
            seq: cursor.seq + 1,
            event: Event::RolesUpdated {
                guild_id: Id(10),
                roles: vec![newer.clone()],
            },
        });
        assert!(model.merge_settings(Snapshot {
            cursor: cursor.clone(),
            data: SnapshotData::Roles(vec![old])
        }));
        assert_eq!(model.settings.roles[&Id(10)], newer);
        model.apply(EventEnvelope {
            seq: cursor.seq + 2,
            event: Event::SessionsUpdated,
        });
        assert!(!model.merge_settings(Snapshot {
            cursor,
            data: SnapshotData::Sessions(vec![])
        }));
    }
}

#[cfg(test)]
mod desktop_preference_tests {
    use super::*;
    #[test]
    fn startup_is_explicitly_opt_in_and_legacy_preferences_are_compatible() {
        let p: Preferences = serde_json::from_str("{}").unwrap();
        assert!(!p.start_with_windows);
        assert!(!p.notification_sounds);
    }
}
