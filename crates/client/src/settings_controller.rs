//! Thin iced controller. Wire DTOs and local preferences live in client-core.
use crate::{
    app::{App, Dialog, GuildTab, Msg},
    backend::Request,
};
use iced::Task;
use std::collections::BTreeSet;
use sticktalk_client_core::settings::{self, Preferences, Query, TriState};
use sticktalk_proto::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserPage {
    Account,
    Profiles,
    Appearance,
    Privacy,
    Devices,
    Notifications,
    Voice,
    Accessibility,
    Keybinds,
    Advanced,
}
impl UserPage {
    pub const ALL: [Self; 10] = [
        Self::Account,
        Self::Profiles,
        Self::Privacy,
        Self::Devices,
        Self::Notifications,
        Self::Voice,
        Self::Appearance,
        Self::Accessibility,
        Self::Keybinds,
        Self::Advanced,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Account => "My Account",
            Self::Profiles => "Profiles",
            Self::Privacy => "Privacy & Safety",
            Self::Devices => "Devices",
            Self::Notifications => "Notifications",
            Self::Voice => "Voice & Video",
            Self::Appearance => "Appearance",
            Self::Accessibility => "Accessibility",
            Self::Keybinds => "Keybinds",
            Self::Advanced => "Advanced",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Account | Self::Profiles => "user-round",
            Self::Privacy => "shield",
            Self::Devices => "monitor",
            Self::Notifications => "bell",
            Self::Voice => "mic",
            Self::Appearance => "palette",
            Self::Accessibility => "accessibility",
            Self::Keybinds => "keyboard",
            Self::Advanced => "code",
        }
    }
    pub fn index(self) -> u8 {
        match self {
            Self::Account => 0,
            Self::Profiles => 1,
            Self::Appearance => 2,
            Self::Privacy => 3,
            Self::Devices => 4,
            Self::Notifications => 5,
            Self::Voice => 6,
            Self::Accessibility => 7,
            Self::Keybinds => 8,
            Self::Advanced => 9,
        }
    }
    pub fn from_index(n: u8) -> Self {
        Self::ALL
            .into_iter()
            .find(|p| p.index() == n)
            .unwrap_or(Self::Account)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleTab {
    Display,
    Permissions,
    Members,
}
#[derive(Debug, Clone, Copy)]
pub enum SaveScope {
    AccountName,
    AccountUsername,
    Profile,
    Server,
    Role,
    Channel,
}
#[derive(Debug, Clone, Copy)]
pub enum TextField {
    NavSearch,
    Search,
    PermissionSearch,
    DisplayName,
    Username,
    Pronouns,
    About,
    ServerName,
    Description,
    RoleName,
    RoleHex,
    ChannelName,
    Topic,
    DeleteName,
    Keybind(usize),
    PushToTalkKey,
}
#[derive(Debug, Clone, Copy)]
pub enum Toggle {
    Hoist,
    Mentionable,
    Private,
    SuppressEveryone,
    Muted,
    AutoSensitivity,
    EchoCancellation,
    Agc,
    PushToTalk,
    Studio,
    DirectMessages,
    FriendRequests,
    UnderlineLinks,
    Developer,
    Startup,
    Sounds,
    DesktopNotifications,
}
#[derive(Debug, Clone, Copy)]
pub enum Number {
    InputVolume,
    OutputVolume,
    Sensitivity,
    ReleaseDelay,
    FontScale,
    Saturation,
}
#[derive(Debug, Clone)]
pub enum Action {
    ShakeTick(u64, u8),
    EditAbout(iced::widget::text_editor::Action),
    EditDescription(iced::widget::text_editor::Action),
    User(UserPage),
    Server(GuildTab),
    Text(TextField, String),
    Toggle(Toggle, bool),
    Number(Number, f32),
    RemoveIcon,
    Banner(u8),
    ProfileColor(u32),
    Density(settings::Density),
    Noise(settings::NoiseSuppressionMode),
    Device(bool, String),
    EditAccount(bool),
    Reveal,
    SaveAccount,
    Save,
    Reset,
    EditRole(Id),
    RoleTab(RoleTab),
    BackRoles,
    CreateRole,
    RoleMenu(Id),
    DeleteRole(Id),
    Permission(Permissions, bool),
    ClearPermissions(Permissions),
    MemberFilter(Option<Id>),
    AddMembers(bool),
    Assign(Id, bool),
    DragStart(Id),
    DragOver(Id),
    DragLeave(Id),
    DragEnd,
    ChannelTab(bool),
    OverwriteTarget(OverwriteKind, Id),
    AddOverwrite(OverwriteKind, Id),
    RemoveOverwrite,
    OverwritePermission(Permissions, TriState),
    Notifications(Id),
    Level(NotificationLevel),
    MuteDuration(u64),
    SaveNotifications,
    LogoutDevice(String),
    LogoutOthers,
    DeleteServer,
    CancelDelete,
    ConfirmDelete,
    Bind(usize),
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProfileDraft {
    pub display_name: String,
    pub pronouns: String,
    pub about: String,
    pub color: u32,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServerDraft {
    pub name: String,
    pub description: String,
    pub banner: u8,
    pub icon: Option<Id>,
}
#[derive(Default)]
pub struct State {
    pub preference_task: Option<iced::task::Handle>,
    pub preference_revision: u64,
    pub about_editor: iced::widget::text_editor::Content,
    pub description_editor: iced::widget::text_editor::Content,
    pub preferences: Preferences,
    pub nav_search: String,
    pub shake_serial: u64,
    pub shake_step: u8,
    pub search: String,
    pub permission_search: String,
    pub reveal: bool,
    pub edit_account: bool,
    pub edit_username: bool,
    pub profile: ProfileDraft,
    pub profile_base: ProfileDraft,
    pub server: ServerDraft,
    pub server_base: ServerDraft,
    pub role: Option<Role>,
    pub role_base: Option<Role>,
    pub role_hex: String,
    pub role_tab: Option<RoleTab>,
    pub role_menu: Option<Id>,
    pub member_filter: Option<Id>,
    pub adding_members: bool,
    pub dragging: Option<Id>,
    pub drag_over: Option<Id>,
    pub channel_permissions: bool,
    pub channel: Option<Id>,
    pub channel_name: String,
    pub channel_topic: String,
    pub overwrites: Vec<PermissionOverwrite>,
    pub overwrites_base: Vec<PermissionOverwrite>,
    pub overwrite_target: Option<(OverwriteKind, Id)>,
    pub notification: Option<NotificationSettings>,
    pub mute_duration_ms: u64,
    pub delete_open: bool,
    pub delete_name: String,
    pub binding: Option<usize>,
    pub saving: BTreeSet<Nonce>,
    pub save_failed: bool,
    pub save_scope: Option<SaveScope>,
    pub loaded_user: Option<Id>,
    pub loaded_guild: Option<Id>,
}
impl State {
    pub fn load_preferences(mock: bool) -> Preferences {
        if mock {
            return Preferences::default();
        }
        let mut p: Preferences = std::fs::read(crate::storage::root().join("settings.json"))
            .ok()
            .filter(|v| v.len() <= 32768)
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        p.validate();
        p
    }
    pub fn profile_dirty(&self) -> bool {
        self.profile != self.profile_base
    }
    pub fn server_dirty(&self) -> bool {
        self.server != self.server_base
    }
    pub fn channel_dirty(&self, model: &crate::model::Model) -> bool {
        self.overwrites != self.overwrites_base
            || self.channel.is_some_and(|id| {
                model
                    .channels
                    .get(&id)
                    .is_some_and(|c| c.name.as_deref().unwrap_or_default() != self.channel_name)
                    || model
                        .topics
                        .get(&id)
                        .and_then(|v| v.as_deref())
                        .unwrap_or_default()
                        != self.channel_topic
            })
    }
    pub fn role_dirty(&self) -> bool {
        self.role != self.role_base
    }
}
impl App {
    // Legacy settings actions forward to the authoritative, persisted call model.
    fn settings_voice_action(
        &self,
        action: &Action,
    ) -> Option<sticktalk_client_core::voice::VoiceSettings> {
        use sticktalk_client_core::voice::settings::NoiseSuppression;
        let mut s = self.calls.call.settings.clone();
        match action {
            Action::Number(Number::InputVolume, v) => s.input_volume = v.clamp(0., 200.) as u16,
            Action::Number(Number::OutputVolume, v) => s.output_volume = v.clamp(0., 200.) as u16,
            Action::Number(Number::Sensitivity, v) => s.sensitivity_dbfs = v.clamp(-96., 0.),
            Action::Number(Number::ReleaseDelay, v) => {
                s.push_to_talk_release_delay_ms = v.clamp(0., 1000.) as u16
            }
            Action::Noise(v) => {
                s.noise_suppression = match v {
                    settings::NoiseSuppressionMode::Off => NoiseSuppression::Off,
                    settings::NoiseSuppressionMode::Standard => NoiseSuppression::Standard,
                    settings::NoiseSuppressionMode::High => NoiseSuppression::High,
                }
            }
            Action::Device(input, v) => {
                let id = (v != "Default").then(|| v.clone());
                if *input {
                    s.input_device_id = id;
                } else {
                    s.output_device_id = id;
                }
            }
            Action::Toggle(Toggle::AutoSensitivity, v) => s.sensitivity_auto = *v,
            Action::Toggle(Toggle::EchoCancellation, v) => s.echo_cancellation = *v,
            Action::Toggle(Toggle::Agc, v) => s.agc = *v,
            Action::Toggle(Toggle::PushToTalk, v) => s.push_to_talk = *v,
            Action::Toggle(Toggle::Studio, v) => s.studio_mode = *v,
            _ => return None,
        }
        Some(s)
    }
    fn settings_sync_voice(&mut self) {
        use sticktalk_client_core::voice::settings::NoiseSuppression;
        let s = &self.calls.call.settings;
        let v = &mut self.settings.preferences.voice;
        v.input_device_id = s.input_device_id.clone();
        v.output_device_id = s.output_device_id.clone();
        v.input_volume = s.input_volume as f32;
        v.output_volume = s.output_volume as f32;
        v.sensitivity_auto = s.sensitivity_auto;
        v.sensitivity_manual_db = s.sensitivity_dbfs;
        v.push_to_talk_release_delay_ms = s.push_to_talk_release_delay_ms;
        v.echo_cancellation = s.echo_cancellation;
        v.automatic_gain_control = s.agc;
        v.push_to_talk = s.push_to_talk;
        v.studio_mode = s.studio_mode;
        v.noise_suppression_mode = match s.noise_suppression {
            NoiseSuppression::Off => settings::NoiseSuppressionMode::Off,
            NoiseSuppression::Standard => settings::NoiseSuppressionMode::Standard,
            NoiseSuppression::High => settings::NoiseSuppressionMode::High,
            NoiseSuppression::Auto => {
                if s.auto_high_quality == Some(true) {
                    settings::NoiseSuppressionMode::High
                } else {
                    settings::NoiseSuppressionMode::Standard
                }
            }
        };
    }
    pub(crate) fn settings_dirty(&self) -> bool {
        match self.dialog {
            Dialog::UserSettings => {
                self.settings.profile_dirty()
                    || self.settings.edit_account
                        && self.settings.edit_username
                        && self
                            .state
                            .model
                            .me
                            .as_ref()
                            .is_some_and(|me| self.form.code != me.user.username)
            }
            Dialog::GuildSettings(GuildTab::Overview) => self.settings.server_dirty(),
            Dialog::GuildSettings(GuildTab::Roles) => self.settings.role_dirty(),
            Dialog::ChannelSettings(_) => self.settings.channel_dirty(&self.state.model),
            _ => false,
        }
    }
    pub(crate) fn settings_attention(&mut self) -> Task<Msg> {
        self.settings.shake_serial = self.settings.shake_serial.wrapping_add(1);
        self.settings.shake_step = 1;
        self.settings_shake_tick(self.settings.shake_serial, 2)
    }
    fn settings_shake_tick(&self, serial: u64, step: u8) -> Task<Msg> {
        Task::perform(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(
                    crate::theme::settings::SHAKE_MS,
                ))
                .await;
            },
            move |_| Msg::Settings(Action::ShakeTick(serial, step)),
        )
    }

    pub(crate) fn settings_persist(&mut self, revision: u64) -> Task<Msg> {
        if revision != self.settings.preference_revision {
            return Task::none();
        }
        self.settings.preference_task.take();
        let prefs = self.settings.preferences.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let dir = crate::storage::root();
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    let data = serde_json::to_vec_pretty(&prefs).map_err(|e| e.to_string())?;
                    std::fs::write(dir.join("settings.json"), data).map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| e.to_string())?
            },
            Msg::SettingsStored,
        )
    }
    pub(crate) fn settings_query(&mut self, query: Query) {
        self.request(Request::Settings {
            query,
            generation: self.form_revision,
        });
    }
    pub(crate) fn settings_open(&mut self, dialog: &Dialog) {
        self.settings.role_menu = None;
        match dialog {
            Dialog::UserSettings => {
                self.settings.edit_account = false;
                self.settings.edit_username = false;
                if self.user_tab == UserPage::Voice.index() {
                    self.settings_sync_voice();
                }
                if let Some(id) = self.me() {
                    if self.settings.loaded_user != Some(id) {
                        self.settings.loaded_user = Some(id);
                        self.settings_sync_profile();
                    }
                    self.settings_query(Query::Profile(id));
                    self.settings_query(Query::Sessions);
                    self.settings_query(Query::Notifications);
                }
            }
            Dialog::GuildSettings(_) => {
                if let Some(id) = self.state.guild {
                    if self.settings.loaded_guild != Some(id) {
                        self.settings.loaded_guild = Some(id);
                        self.settings.role = None;
                        self.settings.role_base = None;
                        self.settings_sync_server();
                    }
                    self.settings_query(Query::GuildProfile(id));
                    self.settings_query(Query::Roles(id));
                }
            }
            Dialog::ChannelSettings(id) => {
                self.settings.channel = Some(*id);
                self.settings.channel_permissions = false;
                if let Some(ch) = self.state.model.channels.get(id) {
                    self.settings.channel_name = ch.name.clone().unwrap_or_default();
                }
                self.settings.channel_topic = self
                    .state
                    .model
                    .topics
                    .get(id)
                    .cloned()
                    .flatten()
                    .unwrap_or_default();
                self.settings.overwrites.clear();
                self.settings.overwrites_base.clear();
                self.settings.overwrite_target = None;
                self.settings_query(Query::Overwrites(*id));
                self.request(Request::Channel {
                    channel: *id,
                    generation: self.state.generation,
                });
                if let Some(parent) = self.state.model.channels.get(id).and_then(|c| c.parent_id) {
                    self.settings_query(Query::Overwrites(parent));
                }
            }
            _ => {}
        }
    }
    pub(crate) fn settings_sync_profile(&mut self) {
        if self.settings.profile_dirty() || !self.settings.saving.is_empty() {
            return;
        }
        if let Some(me) = &self.state.model.me {
            let p = self.state.model.settings.profiles.get(&me.user.id);
            self.settings.profile = ProfileDraft {
                display_name: me.user.display_name.clone().unwrap_or_default(),
                pronouns: p.map_or(String::new(), |p| p.pronouns.clone()),
                about: p.map_or(String::new(), |p| p.about_me.clone()),
                color: p.map_or(0x3ddc97, |p| p.banner_color),
            };
            self.settings.profile_base = self.settings.profile.clone();
            self.settings.about_editor =
                iced::widget::text_editor::Content::with_text(&self.settings.profile.about);
        }
    }
    pub(crate) fn settings_sync_server(&mut self) {
        if self.settings.server_dirty() || !self.settings.saving.is_empty() {
            return;
        }
        if let Some(guild) = self
            .state
            .guild
            .and_then(|g| self.state.model.guilds.get(&g))
        {
            let p = self
                .state
                .model
                .settings
                .guild_profiles
                .get(&guild.guild.id);
            self.settings.server = ServerDraft {
                name: guild.guild.name.clone(),
                icon: guild.guild.icon_id,
                description: p.map_or(String::new(), |p| p.profile.description.clone()),
                banner: p.map_or(0, |p| p.profile.banner_color),
            };
            self.settings.server_base = self.settings.server.clone();
            self.settings.description_editor =
                iced::widget::text_editor::Content::with_text(&self.settings.server.description);
        }
    }
    pub(crate) fn settings_snapshot(&mut self, data: &settings::SnapshotData) {
        if let settings::SnapshotData::Overwrites(id, v) = data {
            if self.settings.channel == Some(*id)
                && self.settings.overwrites == self.settings.overwrites_base
            {
                self.settings.overwrites = v.clone();
                self.settings.overwrites_base = v.clone();
                self.settings.overwrite_target = v.first().map(|o| (o.kind, o.target_id));
            }
        }
    }
    fn settings_save_command(&mut self, c: Command) {
        let old: BTreeSet<_> = self.state.pending.keys().copied().collect();
        if self.send(c) {
            self.settings.saving.extend(
                self.state
                    .pending
                    .keys()
                    .filter(|n| !old.contains(n))
                    .copied(),
            );
        } else {
            self.settings.save_failed = true;
        }
    }
    pub(crate) fn settings_response(&mut self, nonce: Nonce, success: bool) {
        if !self.settings.saving.remove(&nonce) {
            return;
        }
        self.settings.save_failed |= !success;
        if self.settings.saving.is_empty() && !self.settings.save_failed {
            match self.settings.save_scope.take() {
                Some(SaveScope::AccountName) => {
                    self.settings.profile_base.display_name =
                        self.settings.profile.display_name.clone();
                    self.settings.edit_account = false;
                }
                Some(SaveScope::AccountUsername) => self.settings.edit_account = false,
                Some(SaveScope::Profile) => {
                    self.settings.profile_base = self.settings.profile.clone()
                }
                Some(SaveScope::Server) => self.settings.server_base = self.settings.server.clone(),
                Some(SaveScope::Role) => self.settings.role_base = self.settings.role.clone(),
                Some(SaveScope::Channel) => {
                    self.settings.overwrites_base = self.settings.overwrites.clone()
                }
                None => {}
            }
            self.notice = "Changes saved.".into();
        }
    }
    pub(crate) fn settings_update(&mut self, action: Action) -> Task<Msg> {
        use Action as A;
        let mut persist = false;
        let voice_settings = self.settings_voice_action(&action);
        match action {
            A::ShakeTick(serial, step) => {
                if serial != self.settings.shake_serial {
                    return Task::none();
                }
                self.settings.shake_step = if step < 6 { step } else { 0 };
                if step < 6 {
                    return self.settings_shake_tick(serial, step + 1);
                }
            }
            A::EditAbout(a) => {
                if self.settings.saving.is_empty() {
                    let old = self.settings.profile.about.clone();
                    self.settings.about_editor.perform(a);
                    let value = self.settings.about_editor.text();
                    if value.chars().count() <= 190 {
                        self.settings.profile.about = value;
                    } else {
                        self.settings.about_editor =
                            iced::widget::text_editor::Content::with_text(&old);
                    }
                }
            }
            A::EditDescription(a) => {
                if self.settings.saving.is_empty() {
                    let old = self.settings.server.description.clone();
                    self.settings.description_editor.perform(a);
                    let value = self.settings.description_editor.text();
                    if value.chars().count() <= 120 {
                        self.settings.server.description = value;
                    } else {
                        self.settings.description_editor =
                            iced::widget::text_editor::Content::with_text(&old);
                    }
                }
            }
            A::User(page) => {
                if page.index() != self.user_tab && self.settings_dirty() {
                    return self.settings_attention();
                }
                if page.index() != self.user_tab {
                    self.settings.edit_account = false;
                    self.settings.edit_username = false;
                }
                if page == UserPage::Voice {
                    self.settings_sync_voice();
                }
                self.user_tab = page.index();
                self.settings.search.clear();
                self.error.clear();
            }
            A::Server(tab) => {
                if self.settings_dirty() {
                    return self.settings_attention();
                }
                self.settings.search.clear();
                self.settings.role = None;
                self.settings.role_base = None;
                return self.update(Msg::Open(Dialog::GuildSettings(tab)));
            }
            A::Text(field, value) => {
                if !self.settings.saving.is_empty() {
                    return Task::none();
                }
                match field {
                    TextField::NavSearch => self.settings.nav_search = value,
                    TextField::Search => self.settings.search = value,
                    TextField::PermissionSearch => self.settings.permission_search = value,
                    TextField::DisplayName => {
                        self.settings.profile.display_name = value.chars().take(32).collect()
                    }
                    TextField::Username => self.form.code = value,
                    TextField::Pronouns => {
                        self.settings.profile.pronouns = value.chars().take(40).collect()
                    }
                    TextField::About => {
                        self.settings.profile.about = value.chars().take(190).collect()
                    }
                    TextField::ServerName => self.settings.server.name = value,
                    TextField::Description => {
                        self.settings.server.description = value.chars().take(120).collect()
                    }
                    TextField::RoleName => {
                        if let Some(r) = &mut self.settings.role {
                            r.name = value;
                        }
                    }
                    TextField::RoleHex => {
                        self.settings.role_hex = value.clone();
                        if let Ok(color) = u32::from_str_radix(value.trim_start_matches('#'), 16) {
                            if color <= 0xffffff {
                                if let Some(r) = &mut self.settings.role {
                                    r.color = color;
                                }
                            }
                        }
                    }
                    TextField::ChannelName => self.settings.channel_name = value,
                    TextField::Topic => self.settings.channel_topic = value,
                    TextField::DeleteName => self.settings.delete_name = value,
                    TextField::Keybind(i) => {
                        if let Some(key) = self.settings.preferences.keybinds.keys().nth(i).cloned()
                        {
                            self.settings.preferences.keybinds.insert(key, value);
                            persist = true;
                        }
                    }
                    TextField::PushToTalkKey => {
                        self.settings.preferences.voice.push_to_talk_key = value;
                        persist = true;
                    }
                }
            }
            A::Toggle(key, value) => {
                if !self.settings.saving.is_empty() {
                    return Task::none();
                }
                let p = &mut self.settings.preferences;
                persist = true;
                match key {
                    Toggle::Hoist => {
                        if let Some(r) = &mut self.settings.role {
                            r.hoist = value;
                        }
                        persist = false;
                    }
                    Toggle::Mentionable => {
                        if let Some(r) = &mut self.settings.role {
                            r.mentionable = value;
                        }
                        persist = false;
                    }
                    Toggle::Private => {
                        if let Some(g) = self.state.guild {
                            settings::private_channel(&mut self.settings.overwrites, g, value);
                            self.settings.overwrite_target = Some((OverwriteKind::Role, g));
                        }
                        persist = false;
                    }
                    Toggle::SuppressEveryone => {
                        if let Some(s) = &mut self.settings.notification {
                            s.suppress_everyone = value;
                        }
                        persist = false;
                    }
                    Toggle::Muted => {
                        if let Some(s) = &mut self.settings.notification {
                            s.muted = value;
                        }
                        persist = false;
                    }
                    Toggle::AutoSensitivity => p.voice.sensitivity_auto = value,
                    Toggle::EchoCancellation => p.voice.echo_cancellation = value,
                    Toggle::Agc => p.voice.automatic_gain_control = value,
                    Toggle::PushToTalk => p.voice.push_to_talk = value,
                    Toggle::Studio => p.voice.studio_mode = value,
                    Toggle::DirectMessages => p.allow_direct_messages = value,
                    Toggle::FriendRequests => p.friend_requests = value,
                    Toggle::UnderlineLinks => p.underline_links = value,
                    Toggle::Developer => p.developer_mode = value,
                    Toggle::DesktopNotifications => p.desktop_notifications = value,
                    Toggle::Sounds => p.notification_sounds = value,
                    Toggle::Startup => {
                        if let Err(error) = crate::desktop_host::startup(
                            value,
                            self.mock_mode || crate::agent_window::enabled(),
                        ) {
                            self.error = error;
                            return Task::none();
                        }
                        p.start_with_windows = value;
                    }
                }
            }
            A::Number(key, value) => {
                let p = &mut self.settings.preferences;
                match key {
                    Number::InputVolume => p.voice.input_volume = value,
                    Number::OutputVolume => p.voice.output_volume = value,
                    Number::Sensitivity => p.voice.sensitivity_manual_db = value,
                    Number::ReleaseDelay => p.voice.push_to_talk_release_delay_ms = value as u16,
                    Number::FontScale => p.chat_font_scale = value / 100.,
                    Number::Saturation => p.saturation = value / 100.,
                }
                persist = true;
                self.message_layouts.clear();
                self.layout_revision = self.layout_revision.wrapping_add(1);
            }
            A::Density(v) => {
                self.settings.preferences.density = v;
                persist = true;
                self.message_layouts.clear();
                self.layout_revision = self.layout_revision.wrapping_add(1);
            }
            A::Noise(v) => {
                self.settings.preferences.voice.noise_suppression_mode = v;
                persist = true;
            }
            A::Device(input, v) => {
                let id = (v != "Default").then_some(v);
                if input {
                    self.settings.preferences.voice.input_device_id = id;
                } else {
                    self.settings.preferences.voice.output_device_id = id;
                }
                persist = true;
            }
            A::RemoveIcon => self.settings.server.icon = None,
            A::Banner(v) => self.settings.server.banner = v,
            A::ProfileColor(v) => self.settings.profile.color = v,
            A::EditAccount(username) => {
                if self.settings_dirty() {
                    return self.settings_attention();
                }
                if username {
                    self.form.code = self
                        .state
                        .model
                        .me
                        .as_ref()
                        .map_or(String::new(), |me| me.user.username.clone());
                }
                self.settings.edit_account = true;
                self.settings.edit_username = username;
            }
            A::Reveal => self.settings.reveal = !self.settings.reveal,
            A::SaveAccount => {
                if !self.settings.saving.is_empty() {
                    return Task::none();
                }
                self.settings.save_failed = false;
                self.settings.save_scope = Some(if self.settings.edit_username {
                    SaveScope::AccountUsername
                } else {
                    SaveScope::AccountName
                });
                self.settings_save_command(Command::UpdateProfile(ProfilePatch {
                    username: if self.settings.edit_username {
                        Patch::Set(self.form.code.trim().to_ascii_lowercase())
                    } else {
                        Patch::Keep
                    },
                    display_name: if self.settings.edit_username {
                        Patch::Keep
                    } else {
                        Patch::Set(self.settings.profile.display_name.clone())
                    },
                    ..Default::default()
                }));
            }
            A::EditRole(id) => {
                if self.settings.role_dirty()
                    && self.settings.role.as_ref().is_some_and(|r| r.id == id)
                {
                    return Task::none();
                }
                if self.settings.role_dirty()
                    && self.settings.role.as_ref().is_some_and(|r| r.id != id)
                {
                    return self.settings_attention();
                }
                self.settings.role = self.state.model.settings.roles.get(&id).cloned();
                self.settings.role_base = self.settings.role.clone();
                self.settings.role_hex = self
                    .settings
                    .role
                    .as_ref()
                    .map_or(String::new(), |r| format!("#{:06X}", r.color));
                self.settings.role_tab = Some(RoleTab::Display);
                self.settings.search.clear();
            }
            A::RoleTab(tab) => {
                self.settings.role_tab = Some(tab);
                self.settings.search.clear();
            }
            A::BackRoles => {
                if self.settings.role_dirty() {
                    return self.settings_attention();
                } else {
                    self.settings.role = None;
                    self.settings.role_base = None;
                }
            }
            A::CreateRole => {
                if self.settings_dirty() {
                    return self.settings_attention();
                }
                if let Some(guild_id) = self.state.guild {
                    self.send(Command::Launch(LaunchCommand::CreateRole {
                        guild_id,
                        name: "New Role".into(),
                    }));
                }
            }
            A::RoleMenu(id) => {
                self.settings.role_menu = (self.settings.role_menu != Some(id)).then_some(id)
            }
            A::DeleteRole(role_id) => {
                if let Some(guild_id) = self.state.guild {
                    if role_id != guild_id
                        && self.send(Command::Launch(LaunchCommand::DeleteRole {
                            guild_id,
                            role_id,
                        }))
                    {
                        self.settings.role_menu = None;
                        self.settings.role = None;
                        self.settings.role_base = None;
                    }
                }
            }
            A::Permission(p, on) => {
                if let Some(r) = &mut self.settings.role {
                    r.permissions.set(p, on);
                }
            }
            A::ClearPermissions(p) => {
                if let Some(r) = &mut self.settings.role {
                    r.permissions.remove(p);
                }
            }
            A::MemberFilter(v) => self.settings.member_filter = v,
            A::AddMembers(v) => self.settings.adding_members = v,
            A::Assign(user_id, add) => {
                if let (Some(r), Some(m)) =
                    (&self.settings.role, self.state.model.members.get(&user_id))
                {
                    let mut ids = m.role_ids.clone();
                    ids.retain(|id| *id != r.id);
                    if add {
                        ids.push(r.id);
                    }
                    self.send(Command::Launch(LaunchCommand::SetMemberRoles {
                        guild_id: r.guild_id,
                        user_id,
                        role_ids: ids,
                    }));
                }
            }
            A::DragStart(id) => {
                if let Some(r) = self.state.model.settings.roles.get(&id) {
                    if self
                        .state
                        .model
                        .guilds
                        .get(&r.guild_id)
                        .zip(self.me())
                        .is_some_and(|(g, me)| {
                            self.state.model.settings.can_edit_role(&g.guild, me, r)
                        })
                    {
                        self.settings.dragging = Some(id);
                        self.settings.drag_over = Some(id);
                    }
                }
            }
            A::DragOver(id) => {
                if self.settings.dragging.is_some() {
                    self.settings.drag_over = Some(id);
                }
            }
            A::DragEnd => {
                if let (Some(guild), Some(me), Some(dragged), Some(target)) = (
                    self.state
                        .guild
                        .and_then(|g| self.state.model.guilds.get(&g)),
                    self.me(),
                    self.settings.dragging.take(),
                    self.settings.drag_over.take(),
                ) {
                    if target != dragged {
                        if let Some(ids) = self.state.model.settings.reorder(
                            &guild.guild,
                            me,
                            dragged,
                            Some(target),
                        ) {
                            self.send(Command::Launch(LaunchCommand::ReorderRoles {
                                guild_id: guild.guild.id,
                                role_ids: ids,
                            }));
                        } else {
                            self.error = "You cannot move roles above your highest role.".into();
                        }
                    }
                }
            }
            A::DragLeave(id) => {
                if self.settings.drag_over == Some(id) {
                    self.settings.drag_over = None;
                }
            }
            A::ChannelTab(v) => {
                if v != self.settings.channel_permissions && self.settings_dirty() {
                    return self.settings_attention();
                }
                self.settings.channel_permissions = v;
            }
            A::OverwriteTarget(k, id) => self.settings.overwrite_target = Some((k, id)),
            A::AddOverwrite(kind, target_id) => {
                if !self
                    .settings
                    .overwrites
                    .iter()
                    .any(|o| o.kind == kind && o.target_id == target_id)
                {
                    self.settings.overwrites.push(PermissionOverwrite {
                        kind,
                        target_id,
                        allow: Permissions::empty(),
                        deny: Permissions::empty(),
                    });
                }
                self.settings.overwrite_target = Some((kind, target_id));
            }
            A::RemoveOverwrite => {
                if let Some((k, id)) = self.settings.overwrite_target.take() {
                    self.settings
                        .overwrites
                        .retain(|o| o.kind != k || o.target_id != id);
                }
            }
            A::OverwritePermission(p, state) => {
                if let Some((k, id)) = self.settings.overwrite_target {
                    if let Some(o) = self
                        .settings
                        .overwrites
                        .iter_mut()
                        .find(|o| o.kind == k && o.target_id == id)
                    {
                        settings::set_overwrite(o, p, state);
                    }
                }
            }
            A::Notifications(id) => {
                self.settings.notification = Some(self.state.model.settings.notification(id));
                self.settings.mute_duration_ms = 3_600_000;
                return self.update(Msg::Open(Dialog::Notifications(id)));
            }
            A::Level(v) => {
                if let Some(s) = &mut self.settings.notification {
                    s.level = v;
                } else {
                    self.settings.preferences.notification_level = v;
                    persist = true;
                }
            }
            A::MuteDuration(v) => self.settings.mute_duration_ms = v,
            A::SaveNotifications => {
                if let Some(mut s) = self.settings.notification.clone() {
                    s.mute_until = if s.muted && self.settings.mute_duration_ms > 0 {
                        Some(crate::mock::now() + self.settings.mute_duration_ms)
                    } else {
                        None
                    };
                    if self.send(Command::Launch(LaunchCommand::SetNotifications(s))) {
                        self.dialog = self.settings_return.take().unwrap_or(Dialog::None);
                        self.settings.notification = None;
                    }
                }
            }
            A::LogoutDevice(session_id) => {
                self.send(Command::Launch(LaunchCommand::LogoutSession { session_id }));
            }
            A::LogoutOthers => {
                self.send(Command::Launch(LaunchCommand::LogoutAllOthers));
            }
            A::DeleteServer => {
                self.settings.delete_open = true;
                self.settings.delete_name.clear();
            }
            A::CancelDelete => self.settings.delete_open = false,
            A::ConfirmDelete => {
                if let Some(g) = self
                    .state
                    .guild
                    .and_then(|g| self.state.model.guilds.get(&g))
                {
                    if Some(g.guild.owner_id) == self.me()
                        && self.settings.delete_name == g.guild.name
                        && self.send(Command::DeleteGuild {
                            guild_id: g.guild.id,
                        })
                    {
                        self.settings.delete_open = false;
                        self.dialog = Dialog::None;
                    }
                }
            }
            A::Bind(i) => self.settings.binding = Some(i),
            A::Reset => {
                if !self.settings.saving.is_empty() {
                    return Task::none();
                }
                self.settings.shake_serial = self.settings.shake_serial.wrapping_add(1);
                self.settings.shake_step = 0;
                self.settings.edit_account = false;
                self.form.code = self
                    .state
                    .model
                    .me
                    .as_ref()
                    .map_or(String::new(), |me| me.user.username.clone());
                if let Some(id) = self.settings.channel {
                    self.settings.channel_name = self
                        .state
                        .model
                        .channels
                        .get(&id)
                        .and_then(|c| c.name.clone())
                        .unwrap_or_default();
                    self.settings.channel_topic = self
                        .state
                        .model
                        .topics
                        .get(&id)
                        .cloned()
                        .flatten()
                        .unwrap_or_default();
                }
                self.settings.profile = self.settings.profile_base.clone();
                self.settings.server = self.settings.server_base.clone();
                self.settings.role = self.settings.role_base.clone();
                self.settings.role_hex = self
                    .settings
                    .role
                    .as_ref()
                    .map_or(String::new(), |r| format!("#{:06X}", r.color));
                self.settings.overwrites = self.settings.overwrites_base.clone();
                self.settings.about_editor =
                    iced::widget::text_editor::Content::with_text(&self.settings.profile.about);
                self.settings.description_editor = iced::widget::text_editor::Content::with_text(
                    &self.settings.server.description,
                );
                self.error.clear();
            }
            A::Save => {
                if !self.settings.saving.is_empty() {
                    return Task::none();
                }
                self.settings.save_failed = false;
                if let Dialog::ChannelSettings(id) = self.dialog {
                    self.settings.save_scope = Some(SaveScope::Channel);
                    if self.settings.channel_permissions {
                        self.settings_save_command(Command::Launch(LaunchCommand::SetOverwrites {
                            channel_id: id,
                            overwrites: self.settings.overwrites.clone(),
                        }));
                    } else {
                        self.settings_save_command(Command::UpdateChannel {
                            channel_id: id,
                            patch: ChannelPatch {
                                name: Patch::Set(self.settings.channel_name.clone()),
                                topic: if self.settings.channel_topic.is_empty() {
                                    Patch::Clear
                                } else {
                                    Patch::Set(self.settings.channel_topic.clone())
                                },
                            },
                        });
                    }
                } else if let Some(mut role) = self
                    .settings
                    .role
                    .clone()
                    .filter(|_| matches!(self.dialog, Dialog::GuildSettings(GuildTab::Roles)))
                {
                    if role.name.trim().is_empty() || role.name.len() > 100 {
                        self.error = "Role names must be 1–100 bytes.".into();
                        return Task::none();
                    }
                    let hex = self.settings.role_hex.trim_start_matches('#');
                    if hex.len() != 6 || u32::from_str_radix(hex, 16).is_err() {
                        self.error = "Enter a six-digit hex color.".into();
                        return Task::none();
                    }
                    if let Some(current) = self.state.model.settings.roles.get(&role.id) {
                        role.position = current.position;
                        if let Some(draft) = &mut self.settings.role {
                            draft.position = current.position;
                        }
                    }
                    self.settings.save_scope = Some(SaveScope::Role);
                    self.settings_save_command(Command::Launch(LaunchCommand::UpdateRole { role }));
                } else if matches!(self.dialog, Dialog::UserSettings) {
                    self.settings.save_scope = Some(SaveScope::Profile);
                    let p = self.settings.profile.clone();
                    self.settings_save_command(Command::UpdateProfile(ProfilePatch {
                        display_name: if p.display_name.is_empty() {
                            Patch::Clear
                        } else {
                            Patch::Set(p.display_name)
                        },
                        ..Default::default()
                    }));
                    self.settings_save_command(Command::Launch(LaunchCommand::UpdateUserProfile {
                        banner_color: p.color,
                        pronouns: p.pronouns,
                        about_me: p.about,
                    }));
                } else if let Some(guild_id) = self.state.guild {
                    self.settings.save_scope = Some(SaveScope::Server);
                    let s = self.settings.server.clone();
                    let patch = self
                        .state
                        .model
                        .guilds
                        .get(&guild_id)
                        .map(|g| settings::guild_patch(&g.guild, &s.name, s.icon))
                        .unwrap_or_default();
                    if patch != GuildPatch::default() {
                        self.settings_save_command(Command::UpdateGuild { guild_id, patch });
                    }
                    self.settings_save_command(Command::Launch(
                        LaunchCommand::UpdateGuildProfile {
                            guild_id,
                            description: s.description,
                            banner_color: s.banner,
                        },
                    ));
                }
            }
        }
        let voice_task = if let Some(settings) = voice_settings {
            let task = self.call_update(crate::voice_ui::Action::Settings(settings));
            self.settings_sync_voice();
            task
        } else {
            Task::none()
        };
        if persist {
            self.settings.preferences.validate();
            if !self.state.session.as_ref().is_some_and(|s| s.config.mock) {
                self.settings.preference_revision =
                    self.settings.preference_revision.wrapping_add(1);
                let revision = self.settings.preference_revision;
                let (task, handle) = Task::perform(
                    async move {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        revision
                    },
                    Msg::SettingsPersist,
                )
                .abortable();
                self.settings.preference_task = Some(handle.abort_on_drop());
                return Task::batch([voice_task, task]);
            }
        }
        voice_task
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_username_draft_blocks_navigation_and_modals_until_reset() {
        let mut app = App::headless("user-settings-account", iced::Size::new(1280., 800.));
        let username = app.state.model.me.as_ref().unwrap().user.username.clone();
        let _ = app.settings_update(Action::EditAccount(true));
        assert_eq!(app.form.code, username);
        assert!(!app.settings_dirty());
        let _ = app.settings_update(Action::Text(TextField::Username, "new_username".into()));
        assert!(app.settings_dirty());
        let _ = app.update(Msg::Open(Dialog::Password));
        assert!(matches!(app.dialog, Dialog::UserSettings));
        let _ = app.settings_update(Action::User(UserPage::Profiles));
        assert_eq!(app.user_tab, UserPage::Account.index());
        let _ = app.settings_update(Action::Reset);
        assert!(!app.settings_dirty());
        assert_eq!(app.form.code, username);
        assert!(!app.settings.edit_account);
        let _ = app.settings_update(Action::EditAccount(true));
        let _ = app.update(Msg::Open(Dialog::Password));
        assert!(matches!(app.dialog, Dialog::Password));
        let _ = app.update(Msg::Close);
        assert!(matches!(app.dialog, Dialog::UserSettings));
        assert!(!app.settings.edit_account);
        assert!(!app.settings_dirty());
    }
    #[test]
    fn merged_voice_controls_reach_the_existing_call_settings() {
        let mut app = App::headless("user-settings-voice", iced::Size::new(1280., 800.));
        let _ = app.settings_update(Action::Number(Number::InputVolume, 135.));
        assert_eq!(app.calls.call.settings.input_volume, 135);
        let _ = app.settings_update(Action::Toggle(Toggle::EchoCancellation, false));
        assert!(!app.calls.call.settings.echo_cancellation);
        let _ = app.settings_update(Action::Number(Number::ReleaseDelay, 1500.));
        assert_eq!(app.calls.call.settings.push_to_talk_release_delay_ms, 1000);
        assert_eq!(
            app.settings.preferences.voice.push_to_talk_release_delay_ms,
            1000
        );
        assert!(app.calls.call.settings.validate().is_ok());
    }
    #[test]
    fn dirty_profile_blocks_navigation_and_close_until_reset() {
        let mut app = App::headless("user-settings-profile", iced::Size::new(1280., 800.));
        assert!(!app.settings_dirty());
        let _ = app.settings_update(Action::Text(TextField::Pronouns, "she/her".into()));
        assert!(app.settings_dirty());
        let _ = app.settings_update(Action::User(UserPage::Account));
        assert_eq!(app.user_tab, UserPage::Profiles.index());
        assert_eq!(app.settings.shake_step, 1);
        let _ = app.update(Msg::Escape);
        assert!(matches!(app.dialog, Dialog::UserSettings));
        let _ = app.settings_update(Action::Reset);
        assert!(!app.settings_dirty());
        assert_eq!(app.settings.shake_step, 0);
        let _ = app.update(Msg::Close);
        assert!(matches!(app.dialog, Dialog::None));
    }
    #[test]
    fn dirty_server_and_channel_keep_their_drafts_on_navigation() {
        let mut app = App::headless("guild-settings-overview", iced::Size::new(1920., 1080.));
        let _ = app.settings_update(Action::Text(TextField::ServerName, "Changed".into()));
        let _ = app.settings_update(Action::Server(GuildTab::Members));
        assert!(matches!(
            app.dialog,
            Dialog::GuildSettings(GuildTab::Overview)
        ));
        assert_eq!(app.settings.server.name, "Changed");
        let mut app = App::headless("channel-settings-overview", iced::Size::new(1280., 800.));
        assert!(!app.settings_dirty());
        let _ = app.settings_update(Action::Text(TextField::ChannelName, "renamed".into()));
        assert!(app.settings_dirty());
        let _ = app.settings_update(Action::ChannelTab(true));
        assert!(!app.settings.channel_permissions);
        let _ = app.settings_update(Action::Reset);
        assert!(!app.settings_dirty());
        let _ = app.settings_update(Action::ChannelTab(true));
        assert!(app.settings.channel_permissions);
    }
    fn app(screen: &str) -> App {
        App::headless(screen, iced::Size::new(1280., 800.))
    }
    #[test]
    fn preserved_role_drafts_never_hijack_user_or_server_profile_saves() {
        let mut app = app("guild-settings-role-display");
        app.dialog = Dialog::UserSettings;
        let _ = app.settings_update(Action::Save);
        assert!(matches!(app.settings.save_scope, Some(SaveScope::Profile)));
        app.dialog = Dialog::GuildSettings(GuildTab::Overview);
        let _ = app.settings_update(Action::Save);
        assert!(matches!(app.settings.save_scope, Some(SaveScope::Server)));
    }
    #[test]
    fn failed_saves_stay_dirty_and_success_only_cleans_its_own_scope() {
        let mut app = app("user-settings-profile-dirty");
        app.settings.server.name = "Unsaved server change".into();
        app.settings.save_scope = Some(SaveScope::Profile);
        app.settings.saving.insert(1);
        app.settings_response(1, false);
        assert!(app.settings.profile_dirty());
        app.settings.save_failed = false;
        app.settings.saving.insert(2);
        app.settings_response(2, true);
        assert!(!app.settings.profile_dirty());
        assert!(app.settings.server_dirty());
    }
    #[test]
    fn appearance_and_font_changes_invalidate_real_chat_layout_immediately() {
        let mut app = app("guild-settings-members");
        let message = app.state.model.messages[&Id(12)]
            .values()
            .next()
            .unwrap()
            .clone();
        let before = app.row_revision(&message);
        let _ = app.settings_update(Action::Density(settings::Density::Compact));
        assert_ne!(before, app.row_revision(&message));
        let _ = app.settings_update(Action::Number(Number::FontScale, 125.));
        assert_eq!(app.settings.preferences.chat_font_scale, 1.25);
        let _ = app.settings_update(Action::Number(Number::Saturation, 50.));
        assert_eq!(app.settings.preferences.saturation, 0.5);
    }
    #[test]
    fn role_drafts_cannot_be_lost_by_switching_roles_or_pages() {
        let mut app = app("guild-settings-role-display");
        let _ = app.settings_update(Action::Text(TextField::RoleName, "Changed".into()));
        let _ = app.settings_update(Action::EditRole(Id(103)));
        assert_eq!(app.settings.role.as_ref().unwrap().name, "Changed");
        let _ = app.settings_update(Action::EditRole(Id(104)));
        assert_eq!(app.settings.role.as_ref().unwrap().id, Id(103));
        let _ = app.settings_update(Action::Server(GuildTab::Members));
        assert!(matches!(app.dialog, Dialog::GuildSettings(GuildTab::Roles)));
        let _ = app.settings_update(Action::Reset);
        assert!(!app.settings.role_dirty());
    }
    #[test]
    fn disabled_role_cannot_begin_drag_and_confirmation_requires_exact_name() {
        let mut app = app("guild-settings-role-disabled");
        let _ = app.settings_update(Action::DragStart(Id(103)));
        assert!(app.settings.dragging.is_none());
        let mut app = App::headless("guild-settings-delete", iced::Size::new(1280., 800.));
        app.settings.delete_name = "the common room".into();
        let _ = app.settings_update(Action::ConfirmDelete);
        assert!(app.settings.delete_open);
        assert!(app.state.pending.is_empty());
    }
    #[test]
    fn unicode_limits_and_keybind_edits_are_bounded_and_local() {
        let mut app = app("user-settings-profile");
        let _ = app.settings_update(Action::Text(TextField::About, "é".repeat(200)));
        assert_eq!(app.settings.profile.about.chars().count(), 190);
        let _ = app.settings_update(Action::Text(TextField::Description, "界".repeat(130)));
        assert_eq!(app.settings.server.description.chars().count(), 120);
        let _ = app.settings_update(Action::Text(TextField::Keybind(0), "Ctrl+Shift+P".into()));
        assert_eq!(
            app.settings.preferences.keybinds.values().next().unwrap(),
            "Ctrl+Shift+P"
        );
        assert!(app.state.pending.is_empty());
    }
}
