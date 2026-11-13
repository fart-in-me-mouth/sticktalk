//! Main-window interaction state. No audio devices are opened by this module.
use crate::{
    app::{App, Dialog, Msg, Screen},
    backend::Request,
    mock::now,
};
use iced::{
    keyboard::{self, key::Named},
    widget::operation,
    Point, Task,
};
use std::collections::BTreeSet;
use sticktalk_client_core::launch::Read;
use sticktalk_proto::*;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entity {
    User(Id),
    Channel(Id),
    Guild(Id),
    Rail,
    Link(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popover {
    Server,
    Profile(Id),
    Pins(Id),
    Notifications(Option<Id>, Option<Id>),
    Topic(Id),
    Quick,
    Devices(bool),
    Context(Entity),
    Mutual(Id, bool),
    Roles(Id),
    Nickname(Id),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Camera,
    Screen,
    StopScreen,
}
#[derive(Default)]
pub struct State {
    pub popover: Option<Popover>,
    pub position: Point,
    pub anchor: Option<iced::Rectangle>,
    pub popover_size: iced::Size,
    pub measurement_revision: u64,
    pub search_focused: bool,
    pub avatar_gallery: bool,
    pub pointer: Point,
    pub collapsed: BTreeSet<Id>,
    pub hovered_channel: Option<Id>,
    pub hovered_category: Option<Id>,
    pub hovered_member: Option<Id>,
    pub user_hover: bool,
    pub split_hover: Option<bool>,
    pub muted: bool,
    pub deafened: bool,
    pub developer: bool,
    pub dropping: bool,
    pub search: String,
    pub quick: String,
    pub quick_index: usize,
    pub message: String,
    pub extent_changes: u16,
    pub acl_next: std::collections::VecDeque<Id>,
    pub pending_dm: Option<(Id, String)>,
    pub media_action: Option<MediaAction>,
    pub focus_voice: Option<Id>,
}
#[derive(Debug, Clone)]
pub enum Message {
    Open(Popover),
    Close,
    Pointer(Point),
    Anchor(iced::Rectangle),
    PopoverSize(iced::Size),
    SearchFocus(bool),
    Category(Id),
    ChannelHover(Option<Id>),
    CategoryHover(Option<Id>),
    MemberHover(Option<Id>),
    UserHover(bool),
    SplitHover(Option<bool>),
    Search(String),
    Quick(String),
    QuickMove(bool),
    QuickChoose,
    Switch(i32, bool),
    Mute,
    Deafen,
    Voice(Id),
    Disconnect,
    FocusVoice(Id),
    Media(MediaAction),
    DropHover(bool),
    Notify(NotificationSettings),
    NotificationExpiry,
    Pin(Id, Id, bool),
    HideMuted,
    Draft(String),
    SendDm(Id),
    Nickname(String),
    SaveNickname(Id),
    Role(Id, Id),
    Developer(bool),
}
pub fn key(key: &keyboard::Key, mods: keyboard::Modifiers) -> Option<Message> {
    if mods.control() && mods.shift() {
        if let keyboard::Key::Character(c) = key {
            return match c.to_lowercase().as_str() {
                "m" => Some(Message::Mute),
                "d" => Some(Message::Deafen),
                _ => None,
            };
        }
    }
    if mods.control() {
        if let keyboard::Key::Character(c) = key {
            if c.eq_ignore_ascii_case("k") {
                return Some(Message::Open(Popover::Quick));
            }
        }
    }
    if mods.alt() {
        return match key {
            keyboard::Key::Named(Named::ArrowUp) => Some(Message::Switch(-1, mods.shift())),
            keyboard::Key::Named(Named::ArrowDown) => Some(Message::Switch(1, mods.shift())),
            _ => None,
        };
    }
    None
}
impl App {
    pub(crate) fn main_open(&mut self, p: Popover) -> Task<Msg> {
        self.dialog = Dialog::None;
        self.main.position = self.main.pointer;
        self.main.anchor = self.main.anchor.filter(|r| r.contains(self.main.pointer));
        self.main.popover_size = iced::Size::ZERO;
        self.main.measurement_revision = self.main.measurement_revision.wrapping_add(1);
        self.main.message.clear();
        match &p {
            Popover::Profile(id) => {
                self.request(Request::Launch {
                    read: Read::Profile(*id),
                    generation: self.state.generation,
                });
            }
            Popover::Pins(id) => {
                self.request(Request::Launch {
                    read: Read::Pins(*id),
                    generation: self.state.generation,
                });
            }
            Popover::Quick => {
                self.main.quick.clear();
                self.main.quick_index = 0;
            }
            Popover::Nickname(id) => {
                self.main.message = self
                    .state
                    .model
                    .launch
                    .name(self.state.guild, *id)
                    .unwrap_or("")
                    .into()
            }
            _ => {}
        }
        let focus = match &p {
            Popover::Quick => Some("quick-switcher"),
            Popover::Profile(_) => Some("popout-message"),
            Popover::Nickname(_) => Some("nickname"),
            _ => None,
        };
        self.main.popover = Some(p);
        focus.map_or(Task::none(), operation::focus)
    }
    pub(crate) fn main_update(&mut self, m: Message) -> Task<Msg> {
        match m {
            Message::Open(p) => return self.main_open(p),
            Message::Close => self.main.popover = None,
            Message::Pointer(p) => self.main.pointer = p,
            Message::Anchor(r) => self.main.anchor = Some(r),
            Message::PopoverSize(s) => self.main.popover_size = s,
            Message::SearchFocus(focused) => self.main.search_focused = focused,
            Message::Category(id) => {
                if !self.main.collapsed.remove(&id) {
                    self.main.collapsed.insert(id);
                }
            }
            Message::ChannelHover(id) => self.main.hovered_channel = id,
            Message::CategoryHover(id) => self.main.hovered_category = id,
            Message::MemberHover(id) => self.main.hovered_member = id,
            Message::UserHover(v) => self.main.user_hover = v,
            Message::SplitHover(v) => self.main.split_hover = v,
            Message::Search(s) => self.main.search = s,
            Message::Quick(s) => {
                self.main.quick = s;
                self.main.quick_index = 0;
            }
            Message::QuickMove(up) => {
                let n = self.quick_items().len();
                if n > 0 {
                    self.main.quick_index = if up {
                        (self.main.quick_index + n - 1) % n
                    } else {
                        (self.main.quick_index + 1) % n
                    };
                }
            }
            Message::QuickChoose => {
                if let Some((_, e, _)) = self.quick_items().get(self.main.quick_index).cloned() {
                    self.main.popover = None;
                    return match e {
                        Entity::Channel(id) => self.update(Msg::Select(id)),
                        Entity::Guild(id) => self.update(Msg::Guild(Some(id))),
                        _ => Task::none(),
                    };
                }
            }
            Message::Switch(delta, unread) => {
                let cs: Vec<_> = self
                    .sorted_channels()
                    .into_iter()
                    .filter(|c| {
                        matches!(
                            c.kind,
                            ChannelKind::Text | ChannelKind::Direct | ChannelKind::GroupDm
                        )
                    })
                    .map(|c| c.id)
                    .collect();
                if !cs.is_empty() {
                    let old = cs
                        .iter()
                        .position(|id| self.state.screen == Screen::Channel(*id))
                        .unwrap_or(0);
                    for step in 1..=cs.len() {
                        let i =
                            (old as i32 + step as i32 * delta).rem_euclid(cs.len() as i32) as usize;
                        let id = cs[i];
                        if !unread
                            || self
                                .state
                                .model
                                .reads
                                .get(&id)
                                .is_some_and(|r| r.unread_count > 0)
                        {
                            return self.update(Msg::Select(id));
                        }
                    }
                }
            }
            Message::Mute => {
                let task = self.call_update(crate::voice_ui::Action::Mute);
                self.main.muted = self.calls.call.muted;
                return task;
            }
            Message::Deafen => {
                let task = self.call_update(crate::voice_ui::Action::Deafen);
                self.main.deafened = self.calls.call.deafened;
                self.main.muted = self.calls.call.muted;
                return task;
            }
            Message::Voice(id) => {
                self.main.popover = None;
                return self.call_update(crate::voice_ui::Action::Join(id));
            }
            Message::FocusVoice(id) => {
                let g = self.state.model.channels.get(&id).and_then(|c| c.guild_id);
                let navigate = if g != self.state.guild {
                    self.update(Msg::Guild(g))
                } else {
                    Task::none()
                };
                self.main.focus_voice = Some(id);
                let index = self
                    .sorted_channels()
                    .iter()
                    .position(|c| c.id == id)
                    .unwrap_or(0);
                return Task::batch([
                    navigate,
                    operation::scroll_to(
                        "channels",
                        iced::widget::scrollable::AbsoluteOffset {
                            x: 0.,
                            y: index as f32 * crate::theme::CHANNEL_ROW,
                        },
                    ),
                ]);
            }
            Message::Media(action) => {
                self.main.media_action = Some(action);
                return self.call_update(match action {
                    MediaAction::Camera => crate::voice_ui::Action::ToggleCamera,
                    MediaAction::Screen => crate::voice_ui::Action::Picker,
                    MediaAction::StopScreen => crate::voice_ui::Action::StopShare,
                });
            }
            Message::Disconnect => {
                self.main.focus_voice = None;
                self.main.popover = None;
                return self.call_update(crate::voice_ui::Action::Leave);
            }
            Message::DropHover(v) => self.main.dropping = v,
            Message::NotificationExpiry => {
                for n in self.state.model.launch.notifications.values_mut() {
                    if n.mute_until.is_some_and(|t| t <= now()) {
                        n.muted = false;
                    }
                }
            }
            Message::Notify(n) => {
                self.send(Command::Launch(LaunchCommand::SetNotifications(n)));
            }
            Message::Pin(channel_id, message_id, pinned) => {
                if self
                    .channel_permissions(channel_id)
                    .contains(Permissions::MANAGE_MESSAGES)
                {
                    self.send(Command::Launch(LaunchCommand::PinMessage {
                        channel_id,
                        message_id,
                        pinned,
                    }));
                    self.main.popover = None;
                    self.dialog = Dialog::None;
                }
            }
            Message::HideMuted => {
                let mut n = self.state.model.launch.settings(self.state.guild, None);
                n.hide_muted_channels = !n.hide_muted_channels;
                self.send(Command::Launch(LaunchCommand::SetNotifications(n)));
            }
            Message::Draft(s) | Message::Nickname(s) => self.main.message = s,
            Message::SendDm(user) => {
                if self.main.message.trim().is_empty() {
                    return Task::none();
                }
                let dm = self
                    .state
                    .model
                    .channels
                    .values()
                    .find(|c| c.kind == ChannelKind::Direct && c.recipient_ids.contains(&user))
                    .map(|c| c.id);
                if let Some(id) = dm {
                    let content = std::mem::take(&mut self.main.message);
                    self.send(Command::SendMessage(MessageDraft {
                        channel_id: id,
                        content,
                        reply_to: None,
                        attachment_ids: vec![],
                        mention_ids: vec![],
                    }));
                    self.main.popover = None;
                    return self.update(Msg::Select(id));
                }
                self.main.pending_dm = Some((user, self.main.message.clone()));
                self.send(Command::OpenDm { user_id: user });
                self.notice = "Conversation opened. Send your message from the composer.".into();
                // Preserve typed text for the OpenDm response, rather than silently losing it.
            }
            Message::SaveNickname(user_id) => {
                if let Some(guild_id) = self.state.guild {
                    let nickname = (!self.main.message.trim().is_empty())
                        .then(|| self.main.message.trim().to_owned());
                    self.send(Command::Launch(LaunchCommand::SetNickname {
                        guild_id,
                        user_id,
                        nickname,
                    }));
                    self.main.popover = None;
                }
            }
            Message::Role(user_id, role_id) => {
                if let Some(guild_id) = self.state.guild {
                    let mut ids = self
                        .state
                        .model
                        .launch
                        .memberships
                        .get(&(guild_id, user_id))
                        .map_or(vec![], |m| m.role_ids.clone());
                    if ids.contains(&role_id) {
                        ids.retain(|id| *id != role_id)
                    } else {
                        ids.push(role_id)
                    }
                    self.send(Command::Launch(LaunchCommand::SetMemberRoles {
                        guild_id,
                        user_id,
                        role_ids: ids,
                    }));
                }
            }
            Message::Developer(v) => self.main.developer = v,
        }
        Task::none()
    }
    pub(crate) fn own_voice(&self) -> Option<&GuildVoiceState> {
        self.state
            .model
            .launch
            .voice
            .values()
            .find(|v| Some(v.participant.session.user_id) == self.me())
    }
    pub(crate) fn channel_permissions(&self, id: Id) -> Permissions {
        sticktalk_client_core::launch::permissions(&self.state.model, id)
    }
    pub(crate) fn quick_items(&self) -> Vec<(String, Entity, String)> {
        let mut items = vec![];
        for c in self
            .state
            .model
            .channels
            .values()
            .filter(|c| c.kind != ChannelKind::Category)
        {
            let name = self.state.model.channel_name(c);
            let server = c
                .guild_id
                .and_then(|g| self.state.model.guilds.get(&g))
                .map_or("Direct Messages".into(), |g| g.guild.name.clone());
            if let Some(score) = sticktalk_client_core::launch::fuzzy_score(
                &self.main.quick,
                &format!("{name} {server}"),
            ) {
                items.push((score, c.id, name, Entity::Channel(c.id), server));
            }
        }
        for g in self.state.model.guilds.values() {
            if let Some(score) =
                sticktalk_client_core::launch::fuzzy_score(&self.main.quick, &g.guild.name)
            {
                items.push((
                    score,
                    g.guild.id,
                    g.guild.name.clone(),
                    Entity::Guild(g.guild.id),
                    "Server".into(),
                ));
            }
        }
        items.sort_by_key(|(s, id, ..)| (std::cmp::Reverse(*s), *id));
        items
            .into_iter()
            .take(12)
            .map(|(_, _, n, e, s)| (n, e, s))
            .collect()
    }
    pub(crate) fn main_channel(&mut self, id: Id) {
        self.main.popover = None;
        self.main.focus_voice = None;
        self.main.search.clear();
        self.main.extent_changes = 0;
        self.state.model.active_guild = self.state.guild;
        if self
            .state
            .model
            .channels
            .get(&id)
            .is_some_and(|c| c.kind != ChannelKind::Voice)
        {
            self.request(Request::Launch {
                read: Read::Extent(id),
                generation: self.state.generation,
            });
        }
        let me = self.me();
        let guild = self.state.guild;
        self.state
            .model
            .launch
            .memberships
            .retain(|(g, u), _| Some(*g) == guild || Some(*u) == me);
        self.main.acl_next = if self.permissions().contains(Permissions::MANAGE_ROLES) {
            let mut ids = vec![id];
            if let Some(parent) = self.state.model.channels.get(&id).and_then(|c| c.parent_id) {
                ids.push(parent);
            }
            for c in self.sorted_channels() {
                if !ids.contains(&c.id) && !self.state.model.launch.overwrites.contains_key(&c.id) {
                    ids.push(c.id);
                }
            }
            ids.into_iter().take(256).collect()
        } else {
            std::collections::VecDeque::new()
        };
        self.main_next_acl();
    }
    pub(crate) fn main_next_acl(&mut self) {
        if let Some(channel) = self.main.acl_next.pop_front() {
            self.request(Request::Launch {
                read: Read::Overwrites(channel),
                generation: self.state.generation,
            });
        }
    }
    pub(crate) fn mark_channel_read(&mut self) {
        if let Screen::Channel(channel) = self.state.screen {
            self.state.manual_unread.remove(&channel);
            if let Some(message_id) = self
                .state
                .model
                .reads
                .get(&channel)
                .and_then(|r| r.latest_message_id)
                .or_else(|| {
                    self.state
                        .model
                        .messages
                        .get(&channel)
                        .and_then(|ms| ms.keys().next_back().copied())
                })
            {
                self.send(Command::MarkRead {
                    channel_id: channel,
                    message_id,
                });
            }
        }
    }
    pub(crate) fn main_voice(&mut self, v: VoicePayload) {
        self.state.model.voice_payload(&v);
        if let Some(error) = self.state.model.launch.voice_error.take() {
            self.error = error;
        }
        self.main_sync_voice();
    }
    pub(crate) fn main_sync_voice(&mut self) {
        if let Some(v) = self.own_voice() {
            let muted = v.participant.muted;
            let deafened = v.participant.deafened;
            self.main.muted = muted;
            self.main.deafened = deafened;
        }
    }
    pub(crate) fn main_live_event(&mut self, event: &Event) {
        if let Event::PinsUpdated { channel_id, .. } = event {
            if self.main.popover == Some(Popover::Pins(*channel_id)) {
                self.request(Request::Launch {
                    read: Read::Pins(*channel_id),
                    generation: self.state.generation,
                });
            }
        }
        let changed_channel = match event {
            Event::MessageCreated { message, .. } => Some(message.channel_id),
            Event::MessageDeleted { channel_id, .. } => Some(*channel_id),
            _ => None,
        };
        if matches!(self.state.screen,Screen::Channel(id) if Some(id)==changed_channel) {
            self.main.extent_changes += 1;
            if self.main.extent_changes >= 100 {
                self.main.extent_changes = 0;
                if let Screen::Channel(id) = self.state.screen {
                    self.request(Request::Launch {
                        read: Read::Extent(id),
                        generation: self.state.generation,
                    });
                }
            }
        }
    }
}
pub fn mm(m: Message) -> Msg {
    Msg::Main(m)
}
pub fn muted(app: &App, id: Id) -> bool {
    app.state
        .model
        .channels
        .get(&id)
        .is_some_and(|c| app.state.model.launch.muted(c.guild_id, id, now()))
}

/// Cursor-first menus avoid their source row when an adjacent side has room.
pub fn place_popover(
    desired: Point,
    size: iced::Size,
    window: iced::Size,
    avoid: Option<iced::Rectangle>,
) -> Point {
    use crate::theme as t;
    let clamp = |p: Point| {
        Point::new(
            p.x.clamp(t::S8, (window.width - size.width - t::S8).max(t::S8)),
            p.y.clamp(t::S8, (window.height - size.height - t::S8).max(t::S8)),
        )
    };
    let mut p = clamp(desired);
    if let Some(row) = avoid {
        let overlaps = |p: Point| iced::Rectangle::new(p, size).intersects(&row);
        if overlaps(p) {
            let candidates = [
                Point::new(row.x + row.width + t::S8, p.y),
                Point::new(row.x - size.width - t::S8, p.y),
                Point::new(p.x, row.y + row.height + t::S4),
                Point::new(p.x, row.y - size.height - t::S4),
            ];
            if let Some(next) = candidates
                .into_iter()
                .map(clamp)
                .filter(|p| !overlaps(*p))
                .min_by(|a, b| {
                    let distance = |q: Point| (q.x - desired.x).powi(2) + (q.y - desired.y).powi(2);
                    distance(*a).total_cmp(&distance(*b))
                })
            {
                p = next;
            }
        }
    }
    p
}

#[cfg(test)]
mod placement_tests {
    use super::*;
    #[test]
    fn menus_stay_inside_window_and_avoid_clicked_rows() {
        let window = iced::Size::new(1280., 800.);
        let size = iced::Size::new(220., 320.);
        for row in [
            iced::Rectangle::new(Point::new(80., 170.), iced::Size::new(224., 34.)),
            iced::Rectangle::new(Point::new(1040., 120.), iced::Size::new(224., 42.)),
            iced::Rectangle::new(Point::new(312., 700.), iced::Size::new(728., 64.)),
        ] {
            let p = place_popover(
                Point::new(row.x + 20., row.y + 20.),
                size,
                window,
                Some(row),
            );
            assert!(
                p.x >= 8.
                    && p.y >= 8.
                    && p.x + size.width <= window.width - 8.
                    && p.y + size.height <= window.height - 8.
            );
            assert!(!iced::Rectangle::new(p, size).intersects(&row));
        }
        let p = place_popover(Point::new(1279., 799.), size, window, None);
        assert_eq!(p, Point::new(1052., 472.));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_are_independent_of_input_focus() {
        let control = keyboard::Modifiers::CTRL;
        let shift = keyboard::Modifiers::SHIFT;
        let alt = keyboard::Modifiers::ALT;
        assert!(matches!(
            key(&keyboard::Key::Character("k".into()), control),
            Some(Message::Open(Popover::Quick))
        ));
        assert!(matches!(
            key(&keyboard::Key::Character("M".into()), control | shift),
            Some(Message::Mute)
        ));
        assert!(matches!(
            key(&keyboard::Key::Character("d".into()), control | shift),
            Some(Message::Deafen)
        ));
        assert!(matches!(
            key(&keyboard::Key::Named(Named::ArrowUp), alt | shift),
            Some(Message::Switch(-1, true))
        ));
    }
    #[test]
    fn quick_switcher_fuzzy_matches_and_hoisted_names_use_nicknames() {
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        app.main.quick = "gnrl".into();
        assert!(matches!(app.quick_items()[0].1, Entity::Channel(Id(12))));
        assert_eq!(app.state.model.name(Id(2)), "Mika");
        assert_eq!(
            app.state.model.launch.color(app.state.guild, Id(2)),
            Some(0x79b8ff)
        );
    }
    #[test]
    fn mute_and_deafen_do_not_start_media_or_timers() {
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        let _ = app.main_update(Message::Mute);
        assert!(app.main.muted);
        let _ = app.main_update(Message::Deafen);
        assert!(app.main.deafened);
        assert!(app.tx.is_none()); // headless fixture has no network or audio tasks
    }
}
