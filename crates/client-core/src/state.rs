//! UI-independent session/navigation, drafts, read state and history-window state.
use crate::{
    backend::{HistoryAnchor, Session},
    model::{ConnectionState, Model},
};
use std::collections::{BTreeMap, BTreeSet};
use sticktalk_proto::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendTab {
    Online,
    All,
    Pending,
    Blocked,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Friends(FriendTab),
    Channel(Id),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuildTab {
    Engagement,
    Roles,
    Emoji,
    Access,
    AuditLog,
    Overview,
    Channels,
    Members,
    Invites,
    Bans,
}
#[derive(Debug, Clone)]
pub enum Dialog {
    None,
    AddGuild,
    CreateGuild,
    JoinGuild,
    Invite,
    AddFriend,
    GuildMenu,
    EmojiPicker,
    UserSettings,
    Notifications(Id),
    GuildSettings(GuildTab),
    NewChannel(ChannelKind, Option<Id>),
    ChannelSettings(Id),
    NewGroup,
    GroupSettings(Id),
    MessageActions(Id),
    Profile(Id),
    Confirm(String, Command),
    ExternalLink(String),
    ReplyPreview(Id),
    HistoryJump,
    Password,
    MemberMenu(Id),
}
#[derive(Default)]
pub struct Form {
    pub name: String,
    pub topic: String,
    pub code: String,
    pub reason: String,
    pub selected: BTreeSet<Id>,
    pub icon: Option<Id>,
}

pub struct ClientState {
    pub model: Model,
    pub session: Option<Session>,
    pub connection: ConnectionState,
    pub screen: Screen,
    pub guild: Option<Id>,
    pub generation: u64,
    pub history_seek: u64,
    pub seek_loading: bool,
    pub jump_seek: Option<u64>,
    pub manual_unread: BTreeSet<Id>,
    pub reply: Option<Id>,
    pub editing: Option<Id>,
    pub attachments: Vec<Attachment>,
    pub pending: BTreeMap<Nonce, Command>,
    pub pending_files: BTreeMap<Nonce, Vec<Attachment>>,
    pub loading: bool,
    pub history_more: bool,
    pub before: Option<Id>,
    pub after: Option<Id>,
    pub member_after: Option<Id>,
    pub invite_after: Option<Id>,
    pub ban_after: Option<Id>,
    pub at_present: bool,
    pub drafts: BTreeMap<Id, String>,
    pub draft_mentions: BTreeMap<Id, BTreeSet<Id>>,
    pub selected_mentions: BTreeSet<Id>,
    pub read_serial: u64,
    pub read_id: Option<Id>,
    pub history_seen: usize,
}
impl Default for ClientState {
    fn default() -> Self {
        Self {
            model: Model::default(),
            session: None,
            connection: ConnectionState::Offline,
            screen: Screen::Friends(FriendTab::Online),
            guild: None,
            generation: 0,
            history_seek: 0,
            seek_loading: false,
            jump_seek: None,
            manual_unread: BTreeSet::new(),
            reply: None,
            editing: None,
            attachments: vec![],
            pending: BTreeMap::new(),
            pending_files: BTreeMap::new(),
            loading: false,
            history_more: false,
            before: None,
            after: None,
            member_after: None,
            invite_after: None,
            ban_after: None,
            at_present: true,
            drafts: BTreeMap::new(),
            draft_mentions: BTreeMap::new(),
            selected_mentions: BTreeSet::new(),
            read_serial: 0,
            read_id: None,
            history_seen: 0,
        }
    }
}
impl ClientState {
    pub fn member_page(
        &mut self,
        guild: Id,
        after: Option<Id>,
        page: Page<Member>,
        generation: u64,
    ) -> bool {
        if self.guild == Some(guild)
            && generation == self.generation
            && self.model.snapshot_valid(&page.cursor)
        {
            if after.is_none() {
                self.model.members.clear();
            }
            self.member_after = page.next_after;
            for m in page.items {
                self.model.users.insert(m.user.id, m.user.clone());
                self.model.members.insert(m.user.id, m);
            }
            self.model.reconcile_metadata(&page.cursor);

            return true;
        }
        false
    }
    pub fn invite_page(
        &mut self,
        guild: Id,
        after: Option<Id>,
        page: Page<Invite>,
        generation: u64,
    ) -> bool {
        if self.guild == Some(guild)
            && generation == self.generation
            && self.model.snapshot_valid(&page.cursor)
        {
            if after.is_none() {
                self.model.invites.clear();
            }
            self.invite_after = page.next_after;
            for v in page.items {
                self.model.invites.insert(v.id, v);
            }
            self.model.reconcile_metadata(&page.cursor);

            return true;
        }
        false
    }
    pub fn ban_page(
        &mut self,
        guild: Id,
        after: Option<Id>,
        page: Page<Ban>,
        generation: u64,
    ) -> bool {
        if self.guild == Some(guild)
            && generation == self.generation
            && self.model.snapshot_valid(&page.cursor)
        {
            if after.is_none() {
                self.model.bans.clear();
            }
            self.ban_after = page.next_after;
            for v in page.items {
                self.model.users.insert(v.user.id, v.user.clone());
                self.model.bans.insert(v.user.id, v);
            }
            self.model.reconcile_metadata(&page.cursor);

            return true;
        }
        false
    }
    pub fn accepts_history(&self, channel: Id, generation: u64, seek: Option<u64>) -> bool {
        generation == self.generation
            && self.screen == Screen::Channel(channel)
            && seek.is_none_or(|serial| serial == self.history_seek)
    }
    pub fn newer_history(&mut self, channel: Id, page: Page<Message>) -> bool {
        self.loading = false;
        if !self.model.snapshot_valid(&page.cursor) {
            return false;
        }
        self.after = page.next_after;
        self.model.merge_page(channel, page)
    }
    /// Merge older bodies; caller preserves measured scroll anchoring in its view.
    pub fn older_history(&mut self, channel: Id, page: Page<Message>) -> bool {
        self.loading = false;
        self.history_seen += page.items.len();
        self.history_more = page.has_more;
        self.before = page.next_before;
        self.model.merge_page(channel, page)
    }
    pub fn replace_history(
        &mut self,
        channel: Id,
        anchor: HistoryAnchor,
        page: Page<Message>,
    ) -> Id {
        let target = match anchor {
            HistoryAnchor::Message(id) => id,
            HistoryAnchor::Timestamp(time) => page
                .items
                .iter()
                .min_by_key(|m| m.created_at.abs_diff(time))
                .map_or(Id(0), |m| m.id),
        };
        self.jump_seek = None;
        self.history_more = page.next_before.is_some();
        self.before = page.next_before;
        self.after = page.next_after;
        self.loading = false;
        self.at_present = false;
        self.model.messages.remove(&channel);
        self.model.merge_page(channel, page);
        target
    }
    pub fn mark_unread(&mut self, id: Id) {
        if let Screen::Channel(channel) = self.screen {
            let last = self
                .model
                .messages
                .get(&channel)
                .and_then(|ms| ms.range(..id).next_back())
                .map(|(id, _)| *id);
            self.manual_unread.insert(channel);
            self.read_id = None;
            self.read_serial += 1;
            self.model.reads.insert(
                channel,
                ReadState {
                    channel_id: channel,
                    last_read_id: last,
                    latest_message_id: Some(id),
                    unread_count: 1,
                    mention_count: 0,
                },
            );
        }
    }
    pub fn read_command(&self) -> Option<Command> {
        if self.connection != ConnectionState::Ready {
            return None;
        }
        if let (Screen::Channel(channel_id), Some(message_id)) = (self.screen, self.read_id) {
            if !self.manual_unread.contains(&channel_id) {
                return self.model.mark_read_command(channel_id, message_id);
            }
        }
        None
    }
}
