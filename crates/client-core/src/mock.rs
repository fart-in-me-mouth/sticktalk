//! In-process protocol peer. Commands and events are the real proto enums, not UI DTOs.
mod settings;
use crate::model::Model;
use std::collections::BTreeMap;
use sticktalk_proto::*;
pub const IMAGE: &[u8] = include_bytes!("../assets/mock-workspace.jpg");

pub fn user(id: u64, name: &str, display: &str) -> UserSummary {
    UserSummary {
        id: Id(id),
        username: name.into(),
        display_name: Some(display.into()),
        avatar_id: None,
        status_text: None,
    }
}

pub struct Mock {
    pub model: Model,
    next: u64,
    uploads: BTreeMap<Id, Attachment>,
    receipts: BTreeMap<Nonce, (Command, CommandOutcome)>,
    clock: std::sync::Arc<dyn crate::io::Clock>,
    closed_dms: BTreeMap<Id, (Channel, BTreeMap<Id, Message>)>,
}
impl Default for Mock {
    fn default() -> Self {
        Self::new()
    }
}
impl Mock {
    pub fn new() -> Self {
        Self::with_clock(std::sync::Arc::new(crate::io::FixedClock::default()))
    }
    pub fn with_clock(clock: std::sync::Arc<dyn crate::io::Clock>) -> Self {
        let now = || clock.unix_ms();
        let me = user(1, "river", "River Chen");
        let users = vec![
            user(2, "mika", "Mika Park"),
            user(3, "sam", "Sam Rivera"),
            user(4, "jules", "Jules Bennett-Ramirez"),
            user(5, "alex", "Alex Morgan"),
            user(6, "noor", "Noor Ali"),
        ];
        let guild = GuildSummary {
            guild: Guild {
                id: Id(10),
                owner_id: Id(1),
                name: "The Common Room".into(),
                icon_id: None,
                member_count: 6,
            },
            my_role: MemberRole::Owner,
            my_permissions: Permissions::GUILD_OWNER,
        };
        let mut channels = vec![];
        for (id, kind, name, parent, position) in [
            (11, ChannelKind::Category, "THE COMMON ROOM", None, 0),
            (12, ChannelKind::Text, "general", Some(Id(11)), 0),
            (13, ChannelKind::Text, "introductions", Some(Id(11)), 1),
            (14, ChannelKind::Text, "little-wins", Some(Id(11)), 2),
            (15, ChannelKind::Category, "MAKE SOMETHING", None, 1),
            (16, ChannelKind::Text, "design-lab", Some(Id(15)), 0),
            (17, ChannelKind::Text, "off-topic", Some(Id(15)), 1),
        ] {
            channels.push(ChannelSummary {
                id: Id(id),
                kind,
                guild_id: Some(Id(10)),
                parent_id: parent,
                name: Some(name.into()),
                position,
                recipient_ids: vec![],
                owner_id: None,
            });
        }
        channels.push(ChannelSummary {
            id: Id(20),
            kind: ChannelKind::Direct,
            guild_id: None,
            parent_id: None,
            name: None,
            position: 0,
            recipient_ids: vec![Id(1), Id(2)],
            owner_id: None,
        });
        channels.push(ChannelSummary {
            id: Id(21),
            kind: ChannelKind::GroupDm,
            guild_id: None,
            parent_id: None,
            name: Some("Weekend makers".into()),
            position: 0,
            recipient_ids: vec![Id(1), Id(2), Id(3)],
            owner_id: Some(Id(1)),
        });
        for (id, guild, name) in [(31, 30, "workbench"), (41, 40, "field-notes")] {
            channels.push(ChannelSummary {
                id: Id(id),
                kind: ChannelKind::Text,
                guild_id: Some(Id(guild)),
                parent_id: None,
                name: Some(name.into()),
                position: 0,
                recipient_ids: vec![],
                owner_id: None,
            });
        }
        let ready = Ready {
            cursor: ResumeCursor {
                stream_id: "00000000000000000000000000000001".into(),
                seq: 0,
            },
            me: SelfUser {
                user: me.clone(),
                desired_presence: PresenceStatus::Online,
            },
            guilds: vec![
                guild,
                GuildSummary {
                    guild: Guild {
                        id: Id(30),
                        owner_id: Id(2),
                        name: "Weekend Workshop".into(),
                        icon_id: None,
                        member_count: 12,
                    },
                    my_role: MemberRole::Member,
                    my_permissions: Permissions::MEMBER,
                },
                GuildSummary {
                    guild: Guild {
                        id: Id(40),
                        owner_id: Id(3),
                        name: "Field Notes".into(),
                        icon_id: None,
                        member_count: 8,
                    },
                    my_role: MemberRole::Member,
                    my_permissions: Permissions::MEMBER,
                },
            ],
            read_states: channels
                .iter()
                .filter(|c| c.kind != ChannelKind::Category)
                .map(|c| ReadState {
                    channel_id: c.id,
                    last_read_id: None,
                    latest_message_id: None,
                    unread_count: if c.id == Id(14) { 3 } else { 0 },
                    mention_count: if c.id == Id(14) { 2 } else { 0 },
                })
                .collect(),
            channels,
            relationships: users
                .iter()
                .enumerate()
                .map(|(i, u)| Relationship {
                    user: u.clone(),
                    kind: if i == 3 {
                        RelationshipKind::IncomingRequest
                    } else if i == 4 {
                        RelationshipKind::Blocked
                    } else {
                        RelationshipKind::Friend
                    },
                    since: now(),
                })
                .collect(),
            users: users.clone(),
            presences: vec![
                Presence {
                    user_id: Id(1),
                    status: PresenceStatus::Online,
                },
                Presence {
                    user_id: Id(2),
                    status: PresenceStatus::Online,
                },
                Presence {
                    user_id: Id(3),
                    status: PresenceStatus::Idle,
                },
                Presence {
                    user_id: Id(4),
                    status: PresenceStatus::Online,
                },
            ],
            ..Default::default()
        };
        let mut model = Model::default();
        model.ready(ready);
        for u in std::iter::once(me).chain(users) {
            let role = if u.id == Id(1) {
                MemberRole::Owner
            } else {
                MemberRole::Member
            };
            model.members.insert(
                u.id,
                Member {
                    guild_id: Id(10),
                    user: u,
                    role,
                    permissions: if role == MemberRole::Owner {
                        Permissions::GUILD_OWNER
                    } else {
                        Permissions::MEMBER
                    },
                    joined_at: now() - 86400000,
                    ..Default::default()
                },
            );
        }
        model
            .topics
            .insert(Id(12), Some("Project updates and discussion.".into()));
        let texts = [
            "A quiet corner of the internet, for good company.",
            "Morning, everyone! What are you making this week?",
            "Finally finished the little reading nook. Plants make everything better.",
            "That sounds lovely. I'm working on a tiny app with a big heart.",
            "Less noise, more connection. That's the idea.",
            "<@1> I put the first sketches in design-lab. Would love your thoughts.",
            "The warm green is a keeper. Feels like a place to slow down.",
            "Agreed! Let's keep the details simple and the conversations interesting.",
        ];
        for channel in [12, 13, 14, 16, 17, 20, 21] {
            for n in 0..80u64 {
                let id = Id(1000 + channel * 100 + n);
                let author = Id(if n % 4 == 0 { 1 } else { 2 + n % 3 });
                let content = texts[(n as usize) % texts.len()].to_string();
                model.insert(Message {
                    id,
                    channel_id: Id(channel),
                    author_id: author,
                    content,
                    created_at: now() - ((80 - n) * 180000),
                    edited_at: None,
                    reply_to: if n == 79 { Some(Id(id.0 - 2)) } else { None },
                    attachments: vec![],
                    mention_ids: if n % 8 == 5 { vec![Id(1)] } else { vec![] },
                    call: None,
                    reactions: if n % 8 == 6 {
                        vec![ReactionSummary {
                            emoji: "🌱".into(),
                            count: 3,
                            me: false,
                        }]
                    } else {
                        vec![]
                    },
                });
            }
        }
        // A deliberately varied, reviewable last screen, using real protocol entities.
        for channel in [12u64, 20, 21] {
            let base = 1000 + channel * 100;
            let texts=["I finally made a little room for the things I want to build.","Here's the reading corner I was talking about. A plant, a good notebook, and absolutely no notifications for an hour.","", "The setup looks great. I'm taking notes for my own desk!", "I tried the first version today. It feels thoughtful: small details, no noise, and enough space to actually finish a conversation. This is the kind of place I'd keep open while I work.", "<@1> The notes are ready whenever you have a moment.","```rust\nlet connection = client.connect();\n```", "Thanks, everyone. Let's pick this up tomorrow."];
            let authors = [2, 2, 2, 3, 1, 4, 4, 2];
            for (i, content) in texts.iter().enumerate() {
                if let Some(m) = model
                    .messages
                    .get_mut(&Id(channel))
                    .and_then(|ms| ms.get_mut(&Id(base + 72 + i as u64)))
                {
                    m.content = (*content).into();
                    m.author_id = Id(authors[i]);
                    m.created_at = now() - (8 - i as u64) * 60000;
                    m.mention_ids = if i == 5 { vec![Id(1)] } else { vec![] };
                    m.reply_to = if i == 7 { Some(Id(base + 77)) } else { None };
                    m.edited_at = if i == 4 { Some(now()) } else { None };
                    m.reactions = if i == 3 {
                        vec![
                            ReactionSummary {
                                emoji: "🌱".into(),
                                count: 4,
                                me: true,
                            },
                            ReactionSummary {
                                emoji: "❤️".into(),
                                count: 2,
                                me: false,
                            },
                        ]
                    } else {
                        vec![]
                    };
                    if i == 2 {
                        m.attachments = vec![Attachment {
                            id: Id(900001),
                            filename: "a-little-room.jpg".into(),
                            content_type: "image/jpeg".into(),
                            byte_size: IMAGE.len() as u64,
                            width: Some(256),
                            height: Some(170),
                            thumbnail: Some(Thumbnail {
                                width: 256,
                                height: 170,
                                byte_size: IMAGE.len() as u32,
                                content_type: "image/jpeg".into(),
                            }),
                        }];
                    }
                }
            }
        }
        for (channel, allowed) in [(Id(20), Id(2)), (Id(21), Id(3))] {
            if let Some(messages) = model.messages.get_mut(&channel) {
                for m in messages.values_mut() {
                    if m.author_id != Id(1) && m.author_id != Id(2) {
                        m.author_id = allowed;
                    }
                }
            }
        }
        model.users.get_mut(&Id(2)).unwrap().status_text = Some("Making a little room".into());
        model.users.get_mut(&Id(4)).unwrap().status_text = Some("Sketching the next thing".into());
        crate::launch_mock::seed(&mut model, now());
        settings::seed(&mut model, now());
        Self {
            model,
            next: 100_000,
            uploads: BTreeMap::new(),
            receipts: BTreeMap::new(),
            clock,
            closed_dms: BTreeMap::new(),
        }
    }
    pub fn configure(&mut self, scenario: &str) {
        if scenario == "stress-scroll" || matches!(scenario, "scroll-5000" | "scroll-skeleton") {
            let sample = self.model.messages[&Id(12)]
                .last_key_value()
                .unwrap()
                .1
                .clone();
            let count = if matches!(scenario, "scroll-5000" | "scroll-skeleton") {
                5_000
            } else {
                10_000
            };
            let history=(0..count).map(|n|{let mut m=sample.clone();m.id=Id(1_000_000+n);m.author_id=if n%3==0{Id(1)}else{Id(2)};m.content=format!("History note {n}: Keep the details small and the conversation thoughtful. Here's an idea worth coming back to, with enough context to exercise wrapping and the retained-data budget.");m.created_at=self.clock.unix_ms()-(10_000-n)*60_000;m.reply_to=None;m.edited_at=None;m.mention_ids.clear();m.reactions.clear();m.attachments=if scenario == "stress-scroll" && n%5==0{self.model.messages[&Id(12)][&Id(2274)].attachments.clone()}else{vec![]};(m.id,m)}).collect();
            self.model.messages.insert(Id(12), history);
            if scenario != "stress-scroll" {
                self.model.channels.get_mut(&Id(12)).unwrap().name =
                    Some("scroll-benchmark".into());
            }
        }
        if scenario == "image-heavy" || scenario == "stress-scroll" {
            let sample = self.model.messages[&Id(12)][&if scenario == "image-heavy" {
                Id(2274)
            } else {
                Id(1_000_000)
            }]
                .attachments[0]
                .clone();
            for (n, m) in self
                .model
                .messages
                .get_mut(&Id(12))
                .unwrap()
                .values_mut()
                .enumerate()
            {
                if scenario == "image-heavy" || !m.attachments.is_empty() {
                    let mut a = sample.clone();
                    a.id = Id(910_000 + (n as u64 % 100));
                    m.attachments = vec![a];
                }
            }
        }
        if matches!(scenario, "markdown" | "markdown-revealed") {
            let messages = self.model.messages.get_mut(&Id(12)).unwrap();
            messages.retain(|id, _| id.0 >= 2277);
            for (id,content) in [
                    (2277,"# Release notes\n## Message formatting\n### Examples\n**Bold** · *italic* · _italic_ · __underline__ · ~~strike~~"),
                    (2278,"> Review the release before publishing.\n- Confirm the version\n- Update the changelog\n`inline code` · ||A hidden spoiler|| · \\*literal asterisks\\*"),
                    (2279,"Links: https://example.com and [Release notes](https://example.com/release-notes)\n```rust\nlet message = client.send(\"A long code line scrolls horizontally instead of wrapping in the message pane.\");\nprintln!(\"{message:?}\");\n```"),
                ] {let m=messages.get_mut(&Id(id)).unwrap();m.content=content.into();m.reply_to=None;m.mention_ids.clear();m.attachments.clear();m.reactions.clear();m.author_id=Id(2+(id%2));}
        }
        if scenario == "channel-top" {
            self.model
                .messages
                .get_mut(&Id(12))
                .unwrap()
                .retain(|id, _| id.0 <= 2210);
        }
        if scenario == "empty-channel" {
            self.model.messages.remove(&Id(12));
        }
        if scenario == "empty-home" {
            self.model.relationships.clear();
            self.model.channels.retain(|_, c| c.guild_id.is_some());
        }
    }
    pub fn id(&mut self) -> Id {
        self.next += 1;
        Id(self.next)
    }
    pub fn startup(&mut self, config: &crate::backend::Config) -> Vec<ServerMsg> {
        self.configure(&config.scenario);
        if !config.token.is_empty() {
            self.model.me.as_mut().unwrap().user.username = config.token.clone();
        }
        let mut frames = vec![ServerMsg::Ready(Box::new(self.ready()))];
        for user_id in [Id(2), Id(3)] {
            let seq = self.model.cursor.as_ref().unwrap().seq + 1;
            self.model.cursor.as_mut().unwrap().seq = seq;
            frames.push(ServerMsg::Event(Box::new(EventEnvelope {
                seq,
                event: Event::Typing {
                    channel_id: Id(12),
                    user_id,
                    expires_at: self.clock.unix_ms() + 8000,
                },
            })));
        }
        frames
    }
    pub fn ready(&self) -> Ready {
        Ready {
            cursor: self.cursor(),
            me: self.model.me.clone().unwrap(),
            guilds: self.model.guilds.values().cloned().collect(),
            channels: self.model.channels.values().cloned().collect(),
            relationships: self.model.relationships.values().cloned().collect(),
            users: self.model.users.values().cloned().collect(),
            read_states: self.model.reads.values().cloned().collect(),
            presences: self
                .model
                .presences
                .iter()
                .map(|(&user_id, &status)| Presence { user_id, status })
                .collect(),
            roles: self
                .model
                .launch
                .roles
                .values()
                .flatten()
                .chain(self.model.settings.roles.values())
                .map(|r| (r.id, r.clone()))
                .collect::<BTreeMap<_, _>>()
                .into_values()
                .collect(),
            memberships: self
                .model
                .launch
                .memberships
                .values()
                .filter(|m| m.user.id == self.model.me.as_ref().unwrap().user.id)
                .cloned()
                .collect(),
            notification_settings: self.model.launch.notifications.values().cloned().collect(),
            voice_states: self.model.launch.voice.values().cloned().collect(),
        }
    }
    pub fn cursor(&self) -> ResumeCursor {
        self.model.cursor.clone().unwrap()
    }
    pub fn history(&self, channel: Id, before: Option<Id>) -> Page<Message> {
        let mut items: Vec<_> = self
            .model
            .messages
            .get(&channel)
            .into_iter()
            .flat_map(|m| m.values().rev())
            .filter(|m| before.is_none_or(|b| m.id < b))
            .take(51)
            .cloned()
            .collect();
        let more = items.len() > 50;
        items.truncate(50);
        Page {
            next_before: if more {
                items.last().map(|m| m.id)
            } else {
                None
            },
            next_after: None,
            has_more: more,
            items,
            users: self.model.users.values().cloned().collect(),
            cursor: self.cursor(),
        }
    }
    pub fn page<T: PageKey>(&self, mut items: Vec<T>) -> Page<T> {
        let more = items.len() > 50;
        items.truncate(50);
        let next = if more {
            items.last().map(PageKey::key)
        } else {
            None
        };
        Page {
            items,
            users: vec![],
            next_before: None,
            next_after: next,
            has_more: more,
            cursor: self.cursor(),
        }
    }
    pub fn upload(
        &mut self,
        attachment: Attachment,
        purpose: UploadPurpose,
    ) -> Result<UploadResponse, String> {
        if attachment.byte_size > MAX_UPLOAD_BYTES {
            return Err("The file exceeds 25 MB".into());
        }
        if purpose != UploadPurpose::Message && attachment.thumbnail.is_none() {
            return Err("Avatars and icons must be PNG, JPEG or WebP images".into());
        }
        self.uploads.insert(attachment.id, attachment.clone());
        Ok(UploadResponse {
            attachment,
            expires_at: self.clock.unix_ms() + 3600000,
        })
    }
    /// Pure REST fixture responses, shared by desktop and browser mock clients.
    pub fn request<R, H>(
        &self,
        request: crate::backend::Request<R>,
    ) -> Option<crate::backend::Data<H>> {
        use crate::backend::{Data, HistoryAnchor, Request};
        Some(match request {
            Request::Launch { read, generation } => Data::Launch {
                data: crate::launch_mock::read(&self.model, read)?,
                generation,
            },
            Request::Settings { query, generation } => Data::Settings {
                snapshot: Snapshot {
                    data: settings::query(&self.model, query)?,
                    cursor: self.cursor(),
                },
                generation,
            },
            Request::NewerHistory {
                seek,
                channel,
                after,
                generation,
            } => {
                let mut items: Vec<_> = self
                    .model
                    .messages
                    .get(&channel)
                    .into_iter()
                    .flat_map(|m| m.values())
                    .filter(|m| m.id > after)
                    .take(51)
                    .cloned()
                    .collect();
                let has_more = items.len() > 50;
                items.truncate(50);
                let next_after = items.last().map(|m| m.id).filter(|_| has_more);
                Data::NewerHistory {
                    seek,
                    channel,
                    generation,
                    page: Page {
                        items,
                        users: self.model.users.values().cloned().collect(),
                        next_before: None,
                        next_after,
                        has_more,
                        cursor: self.cursor(),
                    },
                }
            }
            Request::AroundHistory {
                channel,
                anchor,
                seek,
                generation,
            } => {
                let pivot = match anchor {
                    HistoryAnchor::Message(id) => id.0,
                    HistoryAnchor::Timestamp(t) => t.saturating_sub(SNOWFLAKE_EPOCH_MS) << 22,
                };
                let messages = self.model.messages.get(&channel);
                let mut left: Vec<_> = messages
                    .into_iter()
                    .flat_map(|m| m.values())
                    .filter(|m| m.id.0 <= pivot)
                    .rev()
                    .take(25)
                    .cloned()
                    .collect();
                let right: Vec<_> = messages
                    .into_iter()
                    .flat_map(|m| m.values())
                    .filter(|m| m.id.0 > pivot)
                    .take(50 - left.len())
                    .cloned()
                    .collect();
                left.extend(right);
                left.sort_by_key(|m| std::cmp::Reverse(m.id));
                let mut page = Page {
                    items: left,
                    users: self.model.users.values().cloned().collect(),
                    next_before: None,
                    next_after: None,
                    has_more: false,
                    cursor: self.cursor(),
                };
                page.next_before = page
                    .items
                    .last()
                    .map(|m| m.id)
                    .filter(|id| messages.is_some_and(|ms| ms.keys().any(|v| v < id)));
                page.next_after = page
                    .items
                    .first()
                    .map(|m| m.id)
                    .filter(|id| messages.is_some_and(|ms| ms.keys().any(|v| v > id)));
                page.has_more = page.next_before.is_some() || page.next_after.is_some();
                Data::AroundHistory {
                    channel,
                    anchor,
                    seek,
                    page,
                    generation,
                }
            }
            Request::History {
                seek,
                channel,
                before,
                generation,
            } => Data::History {
                seek,
                channel,
                page: self.history(channel, before),
                generation,
            },
            Request::Channel {
                channel,
                generation,
            } => {
                let c = self.model.channels.get(&channel)?;
                Data::Channel {
                    snapshot: Snapshot {
                        data: Channel {
                            summary: c.clone(),
                            topic: self.model.topics.get(&channel).cloned().flatten(),
                        },
                        cursor: self.cursor(),
                    },
                    generation,
                }
            }
            Request::Members {
                guild,
                after,
                generation,
            } => Data::Members {
                guild,
                after,
                page: self.page(
                    self.model
                        .members
                        .values()
                        .filter(|m| m.guild_id == guild && after.is_none_or(|v| m.user.id > v))
                        .cloned()
                        .collect(),
                ),
                generation,
            },
            Request::Invites {
                guild,
                after,
                generation,
            } => Data::Invites {
                guild,
                after,
                page: self.page(
                    self.model
                        .invites
                        .values()
                        .filter(|v| v.guild_id == guild && after.is_none_or(|id| v.id > id))
                        .cloned()
                        .collect(),
                ),
                generation,
            },
            Request::Bans {
                guild,
                after,
                generation,
            } => Data::Bans {
                guild,
                after,
                page: self.page(
                    self.model
                        .bans
                        .values()
                        .filter(|v| v.guild_id == guild && after.is_none_or(|id| v.user.id > id))
                        .cloned()
                        .collect(),
                ),
                generation,
            },
            Request::Users { ids, generation } => Data::Users {
                snapshot: Snapshot {
                    data: ids
                        .iter()
                        .filter_map(|id| self.model.users.get(id).cloned())
                        .collect(),
                    cursor: self.cursor(),
                },
                generation,
            },
            Request::Message {
                channel,
                message,
                generation,
            } => match self
                .model
                .messages
                .get(&channel)
                .and_then(|m| m.get(&message))
            {
                Some(m) => Data::Message {
                    snapshot: Snapshot {
                        data: m.clone(),
                        cursor: self.cursor(),
                    },
                    generation,
                },
                None => Data::Error("The original message was deleted or is unavailable".into()),
            },
            _ => return None,
        })
    }
    pub fn command(&mut self, request: CommandRequest) -> Vec<ServerMsg> {
        if let Some((old, outcome)) = self.receipts.get(&request.nonce) {
            return vec![ServerMsg::Response(Box::new(CommandResponse {
                nonce: request.nonce,
                cursor: self.cursor(),
                replayed: true,
                outcome: if old == &request.command {
                    outcome.clone()
                } else {
                    CommandOutcome::Err(error("Nonce already used"))
                },
            }))];
        }
        let mut events = vec![];
        let result = self.execute(request.command.clone(), &mut events);
        let mut output = vec![];
        for event in events {
            let seq = self.model.cursor.as_ref().unwrap().seq + 1;
            self.model.cursor.as_mut().unwrap().seq = seq;
            if let Event::MessageCreated { message, .. } = &event {
                self.model.insert(message.clone());
            }
            self.model.reduce(event.clone());
            output.push(ServerMsg::Event(Box::new(EventEnvelope { seq, event })));
        }
        let outcome = match result {
            Ok(v) => CommandOutcome::Ok(v),
            Err(v) => CommandOutcome::Err(v),
        };
        if matches!(outcome, CommandOutcome::Ok(_))
            && !matches!(
                request.command,
                Command::Typing { .. } | Command::SubscribeGuild { .. }
            )
        {
            if self.receipts.len() >= 256 {
                self.receipts.pop_first();
            }
            self.receipts
                .insert(request.nonce, (request.command, outcome.clone()));
        }
        output.push(ServerMsg::Response(Box::new(CommandResponse {
            nonce: request.nonce,
            cursor: self.cursor(),
            replayed: false,
            outcome,
        })));
        output
    }
    fn execute(
        &mut self,
        command: Command,
        events: &mut Vec<Event>,
    ) -> Result<CommandResult, ApiError> {
        let me = self.model.me.as_ref().unwrap().user.id;
        let adding = matches!(
            command,
            Command::AddReaction { .. } | Command::AddGroupRecipient { .. }
        );
        let accepting = matches!(command, Command::AcceptFriendRequest { .. });
        let leaving = matches!(
            command,
            Command::LeaveGuild { .. } | Command::LeaveGroupDm { .. }
        );
        let transferring = matches!(command, Command::TransferGroupOwnership { .. });
        let ack = CommandResult::Ack;
        match command {
            Command::Launch(LaunchCommand::PinMessage {
                channel_id,
                message_id,
                pinned,
            }) => {
                if pinned {
                    let message = self
                        .model
                        .messages
                        .get(&channel_id)
                        .and_then(|ms| ms.get(&message_id))
                        .ok_or_else(|| error("Message not found"))?
                        .clone();
                    let pins = self.model.launch.pins.entry(channel_id).or_default();
                    if !pins.iter().any(|m| m.id == message_id) {
                        pins.insert(0, message);
                    }
                }
                events.push(Event::PinsUpdated {
                    channel_id,
                    message_id,
                    pinned,
                });
                Ok(ack)
            }
            Command::Launch(LaunchCommand::SetNotifications(n)) => {
                events.push(Event::NotificationSettingsUpdated(n.clone()));
                Ok(CommandResult::NotificationSettings(n))
            }
            Command::Launch(LaunchCommand::SetNickname {
                guild_id,
                user_id,
                nickname,
            }) => {
                let mut m = self
                    .model
                    .members
                    .get(&user_id)
                    .ok_or_else(|| error("Member not found"))?
                    .clone();
                m.guild_id = guild_id;
                m.nickname = nickname;
                events.push(Event::MemberUpsert(m.clone()));
                Ok(CommandResult::Member(m))
            }
            Command::Launch(cmd) => settings::command(self, cmd, events),
            Command::SendMessage(v) => {
                if v.content.chars().count() > MAX_MESSAGE_CHARS
                    || (v.content.trim().is_empty() && v.attachment_ids.is_empty())
                {
                    return Err(error(
                        "Write a message or attach a file (4,000 characters maximum).",
                    ));
                }
                let id = self.id();
                let attachments = v
                    .attachment_ids
                    .iter()
                    .filter_map(|id| self.uploads.remove(id))
                    .collect();
                let m = Message {
                    id,
                    channel_id: v.channel_id,
                    author_id: me,
                    content: v.content,
                    created_at: self.clock.unix_ms(),
                    edited_at: None,
                    reply_to: v.reply_to,
                    attachments,
                    mention_ids: v.mention_ids,
                    call: None,
                    reactions: vec![],
                };
                events.push(Event::MessageCreated {
                    message: m.clone(),
                    author: self.model.me.as_ref().unwrap().user.clone(),
                });
                Ok(CommandResult::Message(m))
            }
            Command::EditMessage(v) => {
                let mut m = self.message(v.channel_id, v.message_id)?;
                if m.author_id != me {
                    return Err(error("Only the author can edit this message"));
                }
                m.content = v.content;
                m.mention_ids = v.mention_ids;
                m.edited_at = Some(self.clock.unix_ms());
                events.push(Event::MessageUpdated(m.clone()));
                Ok(CommandResult::Message(m))
            }
            Command::DeleteMessage {
                channel_id,
                message_id,
            } => {
                events.push(Event::MessageDeleted {
                    channel_id,
                    message_id,
                });
                Ok(ack)
            }
            Command::AddReaction {
                channel_id,
                message_id,
                emoji,
            }
            | Command::RemoveReaction {
                channel_id,
                message_id,
                emoji,
            } => {
                let m = self.message(channel_id, message_id)?;
                let old = m.reactions.iter().find(|v| v.emoji == emoji);
                let add = adding;
                let r = ReactionUpdate {
                    channel_id,
                    message_id,
                    user_id: me,
                    added: add,
                    reaction: ReactionSummary {
                        emoji,
                        count: if add {
                            old.map_or(1, |v| v.count + u32::from(!v.me))
                        } else {
                            old.map_or(0, |v| v.count.saturating_sub(u32::from(v.me)))
                        },
                        me: add,
                    },
                };
                events.push(Event::ReactionUpdated(r.clone()));
                Ok(CommandResult::Reaction(r))
            }
            Command::MarkRead {
                channel_id,
                message_id,
            } => {
                let previous = self
                    .model
                    .reads
                    .get(&channel_id)
                    .and_then(|r| r.last_read_id);
                let read = ReadState {
                    channel_id,
                    last_read_id: Some(previous.map_or(message_id, |v| v.max(message_id))),
                    latest_message_id: self
                        .model
                        .messages
                        .get(&channel_id)
                        .and_then(|v| v.last_key_value().map(|(k, _)| *k)),
                    unread_count: 0,
                    mention_count: 0,
                };
                events.push(Event::ReadStateUpdated(read.clone()));
                Ok(CommandResult::ReadState(read))
            }
            Command::Typing { .. } => Ok(ack),
            Command::SubscribeGuild {
                guild_id,
                presence_user_ids,
            } => {
                if let Some(g) = guild_id {
                    for m in self.model.members.values().filter(|m| m.guild_id == g) {
                        events.push(Event::MemberUpsert(m.clone()));
                    }
                }
                Ok(CommandResult::Subscription {
                    presences: presence_user_ids
                        .into_iter()
                        .map(|user_id| Presence {
                            user_id,
                            status: self
                                .model
                                .presences
                                .get(&user_id)
                                .copied()
                                .unwrap_or(PresenceStatus::Offline),
                        })
                        .collect(),
                })
            }
            Command::UpdateProfile(p) => {
                let mut u = self.model.me.clone().unwrap();
                patch(&mut u.user.display_name, p.display_name);
                patch(&mut u.user.status_text, p.status_text);
                patch(&mut u.user.avatar_id, p.avatar_id);
                if let Patch::Set(v) = p.username {
                    u.user.username = v;
                }
                events.push(Event::SelfUpdated(u.clone()));
                Ok(CommandResult::Profile(u))
            }
            Command::SetPresence { status } => {
                if status == PresenceStatus::Offline {
                    return Err(error("Choose Invisible instead of Offline"));
                }
                let mut u = self.model.me.clone().unwrap();
                u.desired_presence = status;
                events.push(Event::SelfUpdated(u));
                let p = Presence {
                    user_id: me,
                    status,
                };
                events.push(Event::PresenceUpdated(p.clone()));
                Ok(CommandResult::Presence(p))
            }
            Command::SendFriendRequest { username } => {
                let u = self
                    .model
                    .users
                    .values()
                    .find(|u| u.username == username)
                    .cloned()
                    .unwrap_or_else(|| {
                        let id = self.id();
                        user(id.0, &username, &username)
                    });
                if u.id == me {
                    return Err(error("You cannot add yourself"));
                }
                let r = Relationship {
                    user: u,
                    kind: RelationshipKind::OutgoingRequest,
                    since: self.clock.unix_ms(),
                };
                events.push(Event::RelationshipUpsert(r.clone()));
                Ok(CommandResult::Relationship(r))
            }
            Command::AcceptFriendRequest { user_id } | Command::BlockUser { user_id } => {
                let kind = if accepting {
                    RelationshipKind::Friend
                } else {
                    RelationshipKind::Blocked
                };
                let u = self
                    .model
                    .users
                    .get(&user_id)
                    .cloned()
                    .ok_or_else(|| error("User not found"))?;
                let r = Relationship {
                    user: u,
                    kind,
                    since: self.clock.unix_ms(),
                };
                events.push(Event::RelationshipUpsert(r.clone()));
                Ok(CommandResult::Relationship(r))
            }
            Command::DeclineFriendRequest { user_id }
            | Command::CancelFriendRequest { user_id }
            | Command::RemoveFriend { user_id }
            | Command::UnblockUser { user_id } => {
                events.push(Event::RelationshipRemoved { user_id });
                Ok(ack)
            }
            Command::OpenDm { user_id } => {
                if self
                    .model
                    .relationships
                    .get(&user_id)
                    .is_some_and(|r| r.kind == RelationshipKind::Blocked)
                {
                    return Err(ApiError {
                        code: ErrorCode::Blocked,
                        message: "Direct messaging is not available".into(),
                        field: None,
                        retry_after_ms: None,
                    });
                }
                let closed = self
                    .closed_dms
                    .iter()
                    .find(|(_, (c, _))| c.summary.recipient_ids.contains(&user_id))
                    .map(|(id, _)| *id);
                if let Some(id) = closed {
                    let (c, messages) = self.closed_dms.remove(&id).unwrap();
                    self.model.messages.insert(id, messages);
                    events.push(Event::ChannelUpsert(c.clone()));
                    return Ok(CommandResult::Channel(c));
                }

                if let Some(c) =
                    self.model.channels.values().find(|c| {
                        c.kind == ChannelKind::Direct && c.recipient_ids.contains(&user_id)
                    })
                {
                    return Ok(CommandResult::Channel(Channel {
                        summary: c.clone(),
                        topic: None,
                    }));
                }
                let id = self.id();
                let c = Channel {
                    summary: ChannelSummary {
                        id,
                        kind: ChannelKind::Direct,
                        guild_id: None,
                        parent_id: None,
                        name: None,
                        position: 0,
                        recipient_ids: vec![me, user_id],
                        owner_id: None,
                    },
                    topic: None,
                };
                events.push(Event::ChannelUpsert(c.clone()));
                Ok(CommandResult::Channel(c))
            }
            Command::CreateGroupDm { name, mut user_ids } => {
                user_ids.push(me);
                user_ids.sort();
                user_ids.dedup();
                if user_ids.len() < 3 {
                    return Err(error("Select at least two friends"));
                }
                let c = Channel {
                    summary: ChannelSummary {
                        id: self.id(),
                        kind: ChannelKind::GroupDm,
                        guild_id: None,
                        parent_id: None,
                        name: Some(name),
                        position: 0,
                        recipient_ids: user_ids,
                        owner_id: Some(me),
                    },
                    topic: None,
                };
                events.push(Event::ChannelUpsert(c.clone()));
                Ok(CommandResult::Channel(c))
            }
            Command::AddGroupRecipient {
                channel_id,
                user_id,
            }
            | Command::RemoveGroupRecipient {
                channel_id,
                user_id,
            }
            | Command::TransferGroupOwnership {
                channel_id,
                user_id,
            } => {
                let mut c = self.channel(channel_id)?;
                if c.summary.owner_id != Some(me) {
                    return Err(error("Group owner required"));
                }
                if transferring {
                    if !c.summary.recipient_ids.contains(&user_id) {
                        return Err(error("Recipient not found"));
                    }
                    c.summary.owner_id = Some(user_id);
                } else if adding {
                    if !c.summary.recipient_ids.contains(&user_id) {
                        c.summary.recipient_ids.push(user_id);
                        c.summary.recipient_ids.sort();
                    }
                } else {
                    c.summary.recipient_ids.retain(|id| *id != user_id);
                }
                events.push(Event::ChannelUpsert(c.clone()));
                Ok(CommandResult::Channel(c))
            }
            Command::CloseDm { channel_id } | Command::LeaveGroupDm { channel_id } => {
                if !leaving {
                    let c = self.channel(channel_id)?;
                    let messages = self
                        .model
                        .messages
                        .get(&channel_id)
                        .cloned()
                        .unwrap_or_default();
                    self.closed_dms.insert(channel_id, (c, messages));
                }

                if leaving {
                    if let Some(c) = self.model.channels.get(&channel_id) {
                        if c.owner_id == Some(me) && c.recipient_ids.len() > 1 {
                            return Err(error("Transfer group ownership before leaving"));
                        }
                    }
                }
                events.push(Event::ChannelRemoved {
                    channel_id,
                    reason: RemovalReason::Left,
                });
                Ok(ack)
            }
            Command::CreateGuild { name, icon_id } => {
                let g = GuildSummary {
                    guild: Guild {
                        id: self.id(),
                        owner_id: me,
                        name,
                        icon_id,
                        member_count: 1,
                    },
                    my_role: MemberRole::Owner,
                    my_permissions: Permissions::GUILD_OWNER,
                };
                let c = ChannelSummary {
                    id: self.id(),
                    kind: ChannelKind::Text,
                    guild_id: Some(g.guild.id),
                    parent_id: None,
                    name: Some("general".into()),
                    position: 0,
                    recipient_ids: vec![],
                    owner_id: None,
                };
                let d = GuildDetail {
                    summary: g,
                    channels: vec![c],
                };
                events.push(Event::GuildJoined {
                    detail: d.clone(),
                    read_states: vec![],
                });
                Ok(CommandResult::Guild(d))
            }
            Command::UpdateGuild { guild_id, patch: p } => {
                let mut g = self
                    .model
                    .guilds
                    .get(&guild_id)
                    .cloned()
                    .ok_or_else(|| error("Guild not found"))?;
                if let Patch::Set(v) = p.name {
                    g.guild.name = v;
                }
                patch(&mut g.guild.icon_id, p.icon_id);
                events.push(Event::GuildUpsert(g.clone()));
                Ok(CommandResult::Guild(GuildDetail {
                    summary: g,
                    channels: self
                        .model
                        .channels
                        .values()
                        .filter(|c| c.guild_id == Some(guild_id))
                        .cloned()
                        .collect(),
                }))
            }
            Command::JoinGuild { invite_code } => {
                let invite = self
                    .model
                    .invites
                    .values()
                    .find(|v| v.code == invite_code)
                    .ok_or_else(|| {
                        error("Invite not found. Create an invite in guild settings first.")
                    })?;
                let g = self.model.guilds[&invite.guild_id].clone();
                Ok(CommandResult::Guild(GuildDetail {
                    summary: g,
                    channels: vec![],
                }))
            }
            Command::LeaveGuild { guild_id } | Command::DeleteGuild { guild_id } => {
                if leaving
                    && self
                        .model
                        .guilds
                        .get(&guild_id)
                        .is_some_and(|g| g.guild.owner_id == me)
                {
                    return Err(error("Transfer ownership or delete the server"));
                }
                events.push(Event::GuildRemoved {
                    guild_id,
                    reason: RemovalReason::Left,
                });
                Ok(ack)
            }
            Command::TransferGuildOwnership { guild_id, user_id } => {
                let mut g = self
                    .model
                    .guilds
                    .get(&guild_id)
                    .cloned()
                    .ok_or_else(|| error("Guild not found"))?;
                g.guild.owner_id = user_id;
                g.my_role = MemberRole::Admin;
                g.my_permissions = Permissions::ADMIN;
                events.push(Event::GuildUpsert(g.clone()));
                Ok(CommandResult::Guild(GuildDetail {
                    summary: g,
                    channels: vec![],
                }))
            }
            Command::SetMemberRole {
                guild_id: _,
                user_id,
                role,
            } => {
                let mut m = self
                    .model
                    .members
                    .get(&user_id)
                    .cloned()
                    .ok_or_else(|| error("Member not found"))?;
                m.role = role;
                m.permissions = if role == MemberRole::Admin {
                    Permissions::ADMIN
                } else {
                    Permissions::MEMBER
                };
                events.push(Event::MemberUpsert(m.clone()));
                Ok(CommandResult::Member(m))
            }
            Command::KickMember { guild_id, user_id } => {
                events.push(Event::MemberRemoved {
                    guild_id,
                    user_id,
                    reason: RemovalReason::Kicked,
                });
                Ok(ack)
            }
            Command::BanMember {
                guild_id,
                user_id,
                reason,
            } => {
                let u = self
                    .model
                    .users
                    .get(&user_id)
                    .cloned()
                    .ok_or_else(|| error("User not found"))?;
                events.push(Event::MemberRemoved {
                    guild_id,
                    user_id,
                    reason: RemovalReason::Banned,
                });
                events.push(Event::BanUpsert(Ban {
                    guild_id,
                    user: u,
                    actor_id: me,
                    reason,
                    created_at: self.clock.unix_ms(),
                }));
                Ok(ack)
            }
            Command::UnbanMember { guild_id, user_id } => {
                events.push(Event::BanRemoved { guild_id, user_id });
                Ok(ack)
            }
            Command::CreateInvite {
                guild_id,
                ttl_seconds,
                max_uses,
            } => {
                let id = self.id();
                let v = Invite {
                    id,
                    code: format!("sticktalk-mock-{}", id.0),
                    guild_id,
                    creator_id: me,
                    expires_at: self.clock.unix_ms() + ttl_seconds as u64 * 1000,
                    max_uses,
                    uses: 0,
                };
                events.push(Event::InviteUpsert(v.clone()));
                Ok(CommandResult::Invite(v))
            }
            Command::RevokeInvite { guild_id, code } => {
                if let Some(v) = self.model.invites.values().find(|v| v.code == code) {
                    events.push(Event::InviteRemoved {
                        guild_id,
                        invite_id: v.id,
                    });
                }
                Ok(ack)
            }
            Command::CreateChannel {
                guild_id,
                kind,
                name,
                parent_id,
                topic,
            } => {
                if name.trim().is_empty() {
                    return Err(error("Give the channel a name"));
                }
                let c = Channel {
                    summary: ChannelSummary {
                        id: self.id(),
                        kind,
                        guild_id: Some(guild_id),
                        parent_id,
                        name: Some(name),
                        position: self
                            .model
                            .channels
                            .values()
                            .filter(|c| c.guild_id == Some(guild_id) && c.parent_id == parent_id)
                            .count() as u16,
                        recipient_ids: vec![],
                        owner_id: None,
                    },
                    topic,
                };
                events.push(Event::ChannelUpsert(c.clone()));
                Ok(CommandResult::Channel(c))
            }
            Command::UpdateChannel {
                channel_id,
                patch: p,
            } => {
                let mut c = self.channel(channel_id)?;
                if let Patch::Set(v) = p.name {
                    c.summary.name = Some(v);
                }
                patch(&mut c.topic, p.topic);
                events.push(Event::ChannelUpsert(c.clone()));
                Ok(CommandResult::Channel(c))
            }
            Command::ReorderChannels {
                guild_id,
                placements,
            } => {
                events.push(Event::ChannelsReordered {
                    guild_id,
                    placements,
                });
                Ok(ack)
            }
            Command::DeleteChannel { channel_id } => {
                if leaving {
                    if let Some(c) = self.model.channels.get(&channel_id) {
                        if c.owner_id == Some(me) && c.recipient_ids.len() > 1 {
                            return Err(error("Transfer group ownership before leaving"));
                        }
                    }
                }
                events.push(Event::ChannelRemoved {
                    channel_id,
                    reason: RemovalReason::Deleted,
                });
                Ok(ack)
            }
            _ => Err(error("Unsupported feature in mock mode")),
        }
    }
    fn message(&self, c: Id, m: Id) -> Result<Message, ApiError> {
        self.model
            .messages
            .get(&c)
            .and_then(|v| v.get(&m))
            .cloned()
            .ok_or_else(|| error("Message not found"))
    }
    fn channel(&self, id: Id) -> Result<Channel, ApiError> {
        Ok(Channel {
            summary: self
                .model
                .channels
                .get(&id)
                .cloned()
                .ok_or_else(|| error("Channel not found"))?,
            topic: self.model.topics.get(&id).cloned().flatten(),
        })
    }
}
fn patch<T>(value: &mut Option<T>, p: Patch<T>) {
    match p {
        Patch::Set(v) => *value = Some(v),
        Patch::Clear => *value = None,
        Patch::Keep => {}
    }
}
fn error(message: &str) -> ApiError {
    ApiError {
        code: ErrorCode::InvalidRequest,
        message: message.into(),
        field: None,
        retry_after_ms: None,
    }
}
pub trait PageKey {
    fn key(&self) -> Id;
}
impl PageKey for Member {
    fn key(&self) -> Id {
        self.user.id
    }
}
impl PageKey for Invite {
    fn key(&self) -> Id {
        self.id
    }
}
impl PageKey for Ban {
    fn key(&self) -> Id {
        self.user.id
    }
}

#[cfg(test)]
mod tests {
    fn test_nonce() -> u64 {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u64
    }
    use super::*;
    fn run(mock: &mut Mock, command: Command) -> CommandResult {
        let frames = mock.command(CommandRequest {
            nonce: test_nonce(),
            command,
        });
        let Some(ServerMsg::Response(response)) = frames.last() else {
            panic!("missing response")
        };
        match &response.outcome {
            CommandOutcome::Ok(v) => v.clone(),
            CommandOutcome::Err(e) => panic!("{}", e.message),
        }
    }
    #[test]
    fn messages_reactions_and_receipt_replay() {
        let mut mock = Mock::new();
        let command = Command::SendMessage(MessageDraft {
            channel_id: Id(12),
            content: "A new idea 🌱".into(),
            reply_to: None,
            attachment_ids: vec![],
            mention_ids: vec![],
        });
        let nonce = test_nonce();
        let req = CommandRequest { nonce, command };
        let frames = mock.command(req.clone());
        let id = match frames.last().unwrap() {
            ServerMsg::Response(r) => match &r.outcome {
                CommandOutcome::Ok(CommandResult::Message(m)) => m.id,
                _ => panic!(),
            },
            _ => panic!(),
        };
        let replay = mock.command(req);
        assert_eq!(replay.len(), 1);
        assert!(matches!(&replay[0],ServerMsg::Response(r)if r.replayed));
        run(
            &mut mock,
            Command::EditMessage(MessageEdit {
                channel_id: Id(12),
                message_id: id,
                content: "Edited".into(),
                mention_ids: vec![],
            }),
        );
        for _ in 0..2 {
            run(
                &mut mock,
                Command::AddReaction {
                    channel_id: Id(12),
                    message_id: id,
                    emoji: "✨".into(),
                },
            );
        }
        assert_eq!(mock.model.messages[&Id(12)][&id].reactions[0].count, 1);
        run(
            &mut mock,
            Command::RemoveReaction {
                channel_id: Id(12),
                message_id: id,
                emoji: "✨".into(),
            },
        );
        assert!(mock.model.messages[&Id(12)][&id].reactions.is_empty());
        run(
            &mut mock,
            Command::DeleteMessage {
                channel_id: Id(12),
                message_id: id,
            },
        );
        assert!(!mock.model.messages[&Id(12)].contains_key(&id));
    }
    #[test]
    fn relationships_and_group_lifecycle() {
        let mut mock = Mock::new();
        run(&mut mock, Command::AcceptFriendRequest { user_id: Id(5) });
        assert_eq!(
            mock.model.relationships[&Id(5)].kind,
            RelationshipKind::Friend
        );
        run(&mut mock, Command::BlockUser { user_id: Id(5) });
        assert_eq!(
            mock.model.relationships[&Id(5)].kind,
            RelationshipKind::Blocked
        );
        run(&mut mock, Command::UnblockUser { user_id: Id(5) });
        run(
            &mut mock,
            Command::SendFriendRequest {
                username: "newfriend".into(),
            },
        );
        let id = mock
            .model
            .relationships
            .values()
            .find(|r| r.user.username == "newfriend")
            .unwrap()
            .user
            .id;
        run(&mut mock, Command::CancelFriendRequest { user_id: id });
        run(&mut mock, Command::DeclineFriendRequest { user_id: Id(5) });
        run(&mut mock, Command::RemoveFriend { user_id: Id(6) });
        let c = match run(
            &mut mock,
            Command::CreateGroupDm {
                name: "Planning".into(),
                user_ids: vec![Id(2), Id(3)],
            },
        ) {
            CommandResult::Channel(c) => c.summary.id,
            _ => panic!(),
        };
        run(
            &mut mock,
            Command::AddGroupRecipient {
                channel_id: c,
                user_id: Id(4),
            },
        );
        run(
            &mut mock,
            Command::AddGroupRecipient {
                channel_id: c,
                user_id: Id(4),
            },
        );
        assert_eq!(mock.model.channels[&c].recipient_ids.len(), 4);
        run(
            &mut mock,
            Command::RemoveGroupRecipient {
                channel_id: c,
                user_id: Id(4),
            },
        );
        run(
            &mut mock,
            Command::TransferGroupOwnership {
                channel_id: c,
                user_id: Id(2),
            },
        );
        assert_eq!(mock.model.channels[&c].owner_id, Some(Id(2)));
        assert!(mock.model.channels[&c].recipient_ids.contains(&Id(2)));
        run(&mut mock, Command::LeaveGroupDm { channel_id: c });
        run(&mut mock, Command::OpenDm { user_id: Id(2) });
        run(&mut mock, Command::CloseDm { channel_id: Id(20) });
    }
    #[test]
    fn guild_channel_invite_and_moderation_flows() {
        let mut mock = Mock::new();
        let g = match run(
            &mut mock,
            Command::CreateGuild {
                name: "New home".into(),
                icon_id: None,
            },
        ) {
            CommandResult::Guild(g) => g.summary.guild.id,
            _ => panic!(),
        };
        run(
            &mut mock,
            Command::UpdateGuild {
                guild_id: g,
                patch: GuildPatch {
                    name: Patch::Set("Renamed".into()),
                    icon_id: Patch::Keep,
                },
            },
        );
        let c = match run(
            &mut mock,
            Command::CreateChannel {
                guild_id: g,
                kind: ChannelKind::Text,
                name: "notes".into(),
                parent_id: None,
                topic: None,
            },
        ) {
            CommandResult::Channel(c) => c.summary.id,
            _ => panic!(),
        };
        run(
            &mut mock,
            Command::UpdateChannel {
                channel_id: c,
                patch: ChannelPatch {
                    name: Patch::Set("ideas".into()),
                    topic: Patch::Set("Keep it simple".into()),
                },
            },
        );
        run(
            &mut mock,
            Command::SendMessage(MessageDraft {
                channel_id: c,
                content: "New channel works".into(),
                reply_to: None,
                attachment_ids: vec![],
                mention_ids: vec![],
            }),
        );
        assert_eq!(mock.history(c, None).items.len(), 1);
        run(
            &mut mock,
            Command::ReorderChannels {
                guild_id: g,
                placements: vec![ChannelPlacement {
                    channel_id: c,
                    parent_id: None,
                    position: 0,
                }],
            },
        );
        let invite = match run(
            &mut mock,
            Command::CreateInvite {
                guild_id: g,
                ttl_seconds: 3600,
                max_uses: 10,
            },
        ) {
            CommandResult::Invite(i) => i,
            _ => panic!(),
        };
        run(
            &mut mock,
            Command::JoinGuild {
                invite_code: invite.code.clone(),
            },
        );
        run(
            &mut mock,
            Command::RevokeInvite {
                guild_id: g,
                code: invite.code,
            },
        );
        run(
            &mut mock,
            Command::SetMemberRole {
                guild_id: Id(10),
                user_id: Id(2),
                role: MemberRole::Admin,
            },
        );
        run(
            &mut mock,
            Command::KickMember {
                guild_id: Id(10),
                user_id: Id(3),
            },
        );
        run(
            &mut mock,
            Command::BanMember {
                guild_id: Id(10),
                user_id: Id(4),
                reason: Some("Test".into()),
            },
        );
        assert!(mock.model.bans.contains_key(&Id(4)));
        run(
            &mut mock,
            Command::UnbanMember {
                guild_id: Id(10),
                user_id: Id(4),
            },
        );
        run(
            &mut mock,
            Command::TransferGuildOwnership {
                guild_id: g,
                user_id: Id(2),
            },
        );
        run(&mut mock, Command::LeaveGuild { guild_id: g });
        run(&mut mock, Command::DeleteChannel { channel_id: Id(17) });
        run(&mut mock, Command::DeleteGuild { guild_id: Id(10) });
        assert!(!mock.model.guilds.contains_key(&Id(10)));
    }
    #[test]
    fn profile_presence_pagination_and_read() {
        let mut mock = Mock::new();
        run(
            &mut mock,
            Command::UpdateProfile(ProfilePatch {
                display_name: Patch::Set("A new name".into()),
                status_text: Patch::Set("Making things".into()),
                ..ProfilePatch::default()
            }),
        );
        run(
            &mut mock,
            Command::SetPresence {
                status: PresenceStatus::Dnd,
            },
        );
        assert_eq!(
            mock.model.me.as_ref().unwrap().desired_presence,
            PresenceStatus::Dnd
        );
        let first = mock.history(Id(12), None);
        assert_eq!(first.items.len(), 50);
        let second = mock.history(Id(12), first.next_before);
        assert_eq!(second.items.len(), 30);
        assert!(first
            .items
            .iter()
            .all(|a| second.items.iter().all(|b| a.id != b.id)));
        run(
            &mut mock,
            Command::MarkRead {
                channel_id: Id(12),
                message_id: first.items[0].id,
            },
        );
        run(&mut mock, Command::Typing { channel_id: Id(12) });
        run(
            &mut mock,
            Command::SubscribeGuild {
                guild_id: Some(Id(10)),
                presence_user_ids: vec![Id(2)],
            },
        );
        assert_eq!(mock.model.reads[&Id(12)].unread_count, 0);
    }
}

/// Stateful, local Google/account-method fixture. No real Google tokens are accepted here.
#[derive(Clone)]
pub struct MockAuth {
    clock: std::sync::Arc<dyn crate::io::Clock>,
    pub methods: AuthMethodsResponse,
    password: Option<String>,
    ticket: Option<String>,
}
impl Default for MockAuth {
    fn default() -> Self {
        Self {
            clock: std::sync::Arc::new(crate::io::FixedClock::default()),
            methods: AuthMethodsResponse {
                has_password: true,
                google_linked: false,
            },
            ticket: None,
            password: Some("mock-password".into()),
        }
    }
}
impl MockAuth {
    pub fn with_clock(clock: std::sync::Arc<dyn crate::io::Clock>) -> Self {
        Self {
            clock,
            ..Self::default()
        }
    }
    pub fn google(&mut self, token: &str) -> Result<GoogleAuthResponse, String> {
        match token {
            "mock-existing" => Ok(GoogleAuthResponse::Session(self.session("river"))),
            "mock-new" => {
                self.ticket = Some("mock-signup-ticket".into());
                Ok(GoogleAuthResponse::SignupRequired(GoogleSignupRequired {
                    ticket: self.ticket.clone().unwrap(),
                    expires_at: self.clock.unix_ms() + 600_000,
                }))
            }
            _ => Err("Google sign-in was not completed.".into()),
        }
    }
    pub fn complete(&mut self, ticket: &str, name: &str) -> Result<AuthResponse, String> {
        if self.ticket.as_deref() != Some(ticket) {
            return Err("Signup expired. Sign in with Google again.".into());
        }
        if name == "river" {
            return Err("That username is already taken.".into());
        }
        if !(3..=32).contains(&name.len())
            || !name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        {
            return Err("Use 3–32 lowercase letters, numbers or underscores.".into());
        }
        self.ticket = None;
        self.password = None;
        self.methods = AuthMethodsResponse {
            has_password: false,
            google_linked: true,
        };
        Ok(self.session(name))
    }
    fn session(&self, name: &str) -> AuthResponse {
        AuthResponse {
            token: name.into(),
            expires_at: self.clock.unix_ms() + 86_400_000,
            user: SelfUser {
                user: user(1, name, name),
                desired_presence: PresenceStatus::Online,
            },
        }
    }
    pub fn action(
        &mut self,
        action: &crate::backend::AccountAction,
    ) -> Result<AuthMethodsResponse, String> {
        use crate::backend::AccountAction;
        match action {
            AccountAction::Read => {}
            AccountAction::Link(token) => {
                if token != "mock-new" && token != "mock-existing" {
                    return Err("Invalid Google token".into());
                }
                self.methods.google_linked = true;
            }
            AccountAction::Unlink => {
                if self.methods.google_linked && !self.methods.has_password {
                    return Err(
                        "Set a password before unlinking Google. It is your only sign-in method."
                            .into(),
                    );
                }
                self.methods.google_linked = false;
            }
            AccountAction::Password { current, new } => {
                if !(1..=crate::http::MAX_PASSWORD_BYTES).contains(&new.len()) {
                    return Err("Enter a new password (128 characters at most).".into());
                }
                if self.methods.has_password && current.as_deref() != self.password.as_deref() {
                    return Err("Current password is incorrect.".into());
                }
                self.methods.has_password = true;
                self.password = Some(new.clone());
            }
        }
        Ok(self.methods.clone())
    }
}
#[cfg(test)]
mod google_tests {
    use super::*;
    use crate::backend::AccountAction;
    #[test]
    fn google_signup_collision_replay_and_last_method() {
        let mut a = MockAuth::default();
        assert!(matches!(
            a.google("mock-existing"),
            Ok(GoogleAuthResponse::Session(_))
        ));
        let GoogleAuthResponse::SignupRequired(t) = a.google("mock-new").unwrap() else {
            panic!()
        };
        assert!(a.complete(&t.ticket, "river").is_err());
        assert!(a.complete(&t.ticket, "new_user").is_ok());
        assert!(a.complete(&t.ticket, "new_user").is_err());
        assert!(a
            .action(&AccountAction::Unlink)
            .unwrap_err()
            .contains("only sign-in"));
        assert!(
            a.action(&AccountAction::Password {
                current: None,
                new: "strong-password".into()
            })
            .unwrap()
            .has_password
        );
        assert!(a
            .action(&AccountAction::Password {
                current: Some("strong-password".into()),
                new: "new-strong-password".into()
            })
            .is_ok());
        assert!(!a.action(&AccountAction::Unlink).unwrap().google_linked);
        assert!(
            a.action(&AccountAction::Link("mock-new".into()))
                .unwrap()
                .google_linked
        );
        assert!(a
            .action(&AccountAction::Password {
                current: Some("wrong".into()),
                new: "strong-password".into()
            })
            .is_err());
    }
}
