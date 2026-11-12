//! Local, transport-free view models. A future client-core adapter owns conversion
//! into these types; views never import protocol, network, or native client code.
use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq)]
pub struct PersonVm {
    pub name: String,
    pub username: String,
    pub initials: String,
    pub color: String,
    pub role_color: String,
    pub presence: String,
    pub status: String,
    pub owner: bool,
}

pub fn fixture_people() -> Vec<PersonVm> {
    vec![
        PersonVm {
            name: "Alex Chen".into(),
            username: "alex".into(),
            initials: "AC".into(),
            color: "#496C63".into(),
            role_color: "#3DDC97".into(),
            presence: "online".into(),
            status: "Around for a game".into(),
            owner: true,
        },
        PersonVm {
            name: "Maya".into(),
            username: "maya".into(),
            initials: "M".into(),
            color: "#7B5C85".into(),
            role_color: "#C8A2E0".into(),
            presence: "online".into(),
            status: "Working on something".into(),
            owner: false,
        },
        PersonVm {
            name: "Jordan".into(),
            username: "jordan".into(),
            initials: "J".into(),
            color: "#54738A".into(),
            role_color: "#C8A2E0".into(),
            presence: "idle".into(),
            status: "Back in a bit".into(),
            owner: false,
        },
        PersonVm {
            name: "Sam Rivera".into(),
            username: "sam".into(),
            initials: "SR".into(),
            color: "#8A6550".into(),
            role_color: "#E8EAED".into(),
            presence: "online".into(),
            status: "".into(),
            owner: false,
        },
        PersonVm {
            name: "Riley".into(),
            username: "riley".into(),
            initials: "R".into(),
            color: "#5B6889".into(),
            role_color: "#E8EAED".into(),
            presence: "dnd".into(),
            status: "Finishing a project".into(),
            owner: false,
        },
        PersonVm {
            name: "Taylor".into(),
            username: "taylor".into(),
            initials: "T".into(),
            color: "#73825B".into(),
            role_color: "#E8EAED".into(),
            presence: "offline".into(),
            status: "".into(),
            owner: false,
        },
        PersonVm {
            name: "Morgan Alexander".into(),
            username: "morgan".into(),
            initials: "MA".into(),
            color: "#806476".into(),
            role_color: "#E8EAED".into(),
            presence: "offline".into(),
            status: "".into(),
            owner: false,
        },
        PersonVm {
            name: "Casey".into(),
            username: "casey".into(),
            initials: "C".into(),
            color: "#607580".into(),
            role_color: "#E8EAED".into(),
            presence: "offline".into(),
            status: "".into(),
            owner: false,
        },
    ]
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoleVm {
    pub name: String,
    pub color: String,
    pub members: usize,
    pub hoisted: bool,
    pub mentionable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttachmentVm {
    None,
    Image {
        src: String,
        srcset: String,
        alt: String,
        width: u32,
        height: u32,
        name: String,
    },
    File {
        name: String,
        size: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageVm {
    pub id: usize,
    pub author: usize,
    pub time: String,
    pub body: String,
    pub continuation: bool,
    pub reply: bool,
    pub mention: bool,
    pub edited: bool,
    pub attachment: AttachmentVm,
    pub reactions: Vec<(String, usize, bool)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppVm {
    pub people: Vec<PersonVm>,
    pub typing: Vec<String>,
    pub screen: String,
    pub panel: String,
    pub modal: String,
    pub menu: String,
    pub profile: Option<usize>,
    pub tab: String,
    pub setting: String,
    pub role_tab: String,
    pub roles: Vec<RoleVm>,
    pub selected_role: usize,
    pub permissions: Vec<bool>,
    pub messages: Vec<MessageVm>,
    pub selected_message: usize,
    pub toast: String,
    pub channel: String,
    pub server: String,
    pub server_description: String,
    pub banner: usize,
    pub muted: bool,
    pub deafened: bool,
    pub connected: bool,
    pub offline: bool,
    pub dirty: bool,
    pub keyboard: bool,
}

impl AppVm {
    pub fn fixture() -> Self {
        let mut vm = Self {
            people: fixture_people(),
            typing: vec!["Maya".into(), "Sam".into()],
            screen: "chat".into(),
            panel: String::new(),
            modal: String::new(),
            menu: String::new(),
            profile: None,
            tab: "Online".into(),
            setting: "Account Info".into(),
            role_tab: "Display".into(),
            roles: [
                ("Owner", "#3DDC97", 1),
                ("Moderators", "#C8A2E0", 2),
                ("Friends", "#6CB5ED", 8),
                ("Game Night", "#F2B84B", 5),
                ("@everyone", "#A8A8B0", 21),
            ]
            .into_iter()
            .map(|(n, c, m)| RoleVm {
                name: n.into(),
                color: c.into(),
                members: m,
                hoisted: true,
                mentionable: false,
            })
            .collect(),
            selected_role: 1,
            permissions: vec![
                true, false, true, false, true, true, false, true, false, false, true, false,
            ],
            messages: fixture_messages(),
            selected_message: 0,
            toast: String::new(),
            channel: "general".into(),
            server: "The Den".into(),
            server_description: "Games, projects, and everything in between.".into(),
            banner: 0,
            muted: false,
            deafened: false,
            connected: true,
            offline: false,
            dirty: false,
            keyboard: false,
        };
        #[cfg(target_arch = "wasm32")]
        if let Ok(search) = web_sys::window().unwrap().location().search() {
            for pair in search.trim_start_matches('?').split('&') {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                match key {
                    "screen" => vm.screen = value.into(),
                    "panel" => vm.panel = value.into(),
                    "modal" => vm.modal = value.into(),
                    "profile" => vm.profile = Some(1),
                    "keyboard" => vm.keyboard = value == "1",
                    "offline" => vm.offline = value == "1",
                    "tab" => vm.role_tab = value.replace("%20", " ").replace('+', " "),
                    "setting" => vm.setting = value.replace("%20", " ").replace('+', " "),
                    _ => {}
                }
            }
        }
        // Keep mutable fixture initialization identical in native tests.
        if vm.screen == "server-settings" && vm.setting == "Account Info" {
            vm.setting = "Server Profile".into();
        }
        vm.toast.clear();
        vm
    }

    pub fn reduce(&mut self, action: &str) {
        let (kind, value) = action.split_once(':').unwrap_or((action, ""));
        self.toast.clear();
        match kind {
            "screen" => {
                self.screen = value.into();
                self.panel.clear();
                self.modal.clear();
                self.menu.clear();
                self.profile = None;
                self.setting = if value == "server-settings" {
                    "Server Profile"
                } else {
                    "Account Info"
                }
                .into();
            }
            "panel" => {
                self.panel = if self.panel == value {
                    String::new()
                } else {
                    value.into()
                }
            }
            "modal" => {
                self.modal = value.into();
                self.menu.clear();
                self.profile = None;
            }
            "menu" => {
                self.menu = if self.menu == value {
                    String::new()
                } else {
                    value.into()
                }
            }
            "profile" => {
                self.profile = value.parse().ok();
                self.menu.clear();
            }
            "close" => {
                self.modal.clear();
                self.menu.clear();
                self.profile = None;
                self.panel.clear();
            }
            "tab" => self.tab = value.into(),
            "setting" => {
                self.setting = value.into();
                if matches!(self.screen.as_str(), "roles" | "role-editor") {
                    self.screen = "server-settings".into();
                }
            }
            "role-tab" => self.role_tab = value.into(),
            "role" => {
                self.selected_role = value
                    .parse::<usize>()
                    .unwrap_or(1)
                    .min(self.roles.len() - 1);
                self.screen = "role-editor".into();
            }
            "create-role" => {
                self.roles.push(RoleVm {
                    name: "New Role".into(),
                    color: "#3DDC97".into(),
                    members: 0,
                    hoisted: false,
                    mentionable: false,
                });
                self.selected_role = self.roles.len() - 1;
                self.screen = "role-editor".into();
                self.dirty = true;
            }
            "role-name" => {
                self.roles[self.selected_role].name = value.chars().take(100).collect();
                self.dirty = true;
            }
            "role-color" => {
                if valid_hex(value) {
                    self.roles[self.selected_role].color = value.into();
                    self.dirty = true;
                }
            }
            "hoist" => {
                let role = &mut self.roles[self.selected_role];
                role.hoisted = !role.hoisted;
                self.dirty = true;
            }
            "mentionable" => {
                let role = &mut self.roles[self.selected_role];
                role.mentionable = !role.mentionable;
                self.dirty = true;
            }
            "permission" => {
                if let Ok(i) = value.parse::<usize>() {
                    if let Some(permission) = self.permissions.get_mut(i) {
                        *permission = !*permission;
                        self.dirty = true;
                    }
                }
            }
            "reorder" => {
                if let Some((from, to)) = value.split_once(',') {
                    if let (Ok(from), Ok(to)) = (from.parse::<usize>(), to.parse::<usize>()) {
                        if from < self.roles.len() && to < self.roles.len() {
                            let role = self.roles.remove(from);
                            self.roles.insert(to, role);
                            self.dirty = true;
                        }
                    }
                }
            }
            "save" => {
                self.dirty = false;
                self.toast = "Changes saved locally".into();
            }
            "reset" => {
                self.roles = Self::fixture().roles;
                self.dirty = false;
            }
            "dirty" => self.dirty = true,
            "server-name" => {
                self.server = value.chars().take(100).collect();
                self.dirty = true;
            }
            "server-description" => {
                self.server_description = value.chars().take(120).collect();
                self.dirty = true;
            }
            "banner" => {
                self.banner = value.parse::<usize>().unwrap_or(0).min(9);
                self.dirty = true;
            }
            "channel" => {
                self.channel = value.into();
                self.screen = if value == "Lounge" { "voice" } else { "chat" }.into();
                self.panel.clear();
            }
            "server" => {
                self.server = value.into();
                self.screen = "chat".into();
            }
            "mute" => self.muted = !self.muted,
            "deafen" => self.deafened = !self.deafened,
            "disconnect" => {
                self.connected = false;
                self.toast = "Disconnected from voice".into();
            }
            "connect" => {
                self.connected = true;
                self.screen = "voice".into();
            }
            "offline" => self.offline = value == "true",
            "send" => {
                let body = value.trim();
                if !body.is_empty() {
                    self.messages.push(MessageVm {
                        id: self.messages.len() + 1000,
                        author: 0,
                        time: "Today at 4:24 PM".into(),
                        body: body.chars().take(4000).collect(),
                        continuation: false,
                        reply: false,
                        mention: false,
                        edited: false,
                        attachment: AttachmentVm::None,
                        reactions: vec![],
                    });
                }
            }
            "message-sheet" | "message-emoji" => {
                self.selected_message = value.parse().unwrap_or(0);
                self.modal = if kind == "message-sheet" {
                    "sheet"
                } else {
                    "emoji"
                }
                .into();
            }
            "react" => {
                let id = value
                    .split(',')
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(self.selected_message);
                let requested = value.split(',').nth(1).unwrap_or("1f44d");
                let emoji = [
                    "1f44b", "1f44d", "2764", "1f602", "1f525", "1f389", "1f680", "2728", "2705",
                    "1f331", "1f440", "1f604",
                ]
                .into_iter()
                .find(|e| *e == requested)
                .unwrap_or("1f44d");
                if let Some(m) = self.messages.iter_mut().find(|m| m.id == id) {
                    if let Some(r) = m.reactions.iter_mut().find(|r| r.0 == emoji) {
                        r.2 = !r.2;
                        r.1 = if r.2 { r.1 + 1 } else { r.1.saturating_sub(1) };
                    } else {
                        m.reactions.push((emoji.into(), 1, true));
                    }
                    m.reactions.retain(|r| r.1 > 0);
                }
                self.modal.clear();
            }
            "copy" => {
                self.toast = "Copied!".into();
                self.menu.clear();
            }
            "toast" => {
                self.toast = value.into();
                self.menu.clear();
            }
            "clear-toast" => self.toast.clear(),
            _ => {}
        }
    }
}

pub fn valid_hex(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn markdown(source: &str) -> String {
    let parser = Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH).map(|event| match event {
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            let safe = dest_url.starts_with("https://") || dest_url.starts_with("http://");
            Event::Start(Tag::Link {
                link_type,
                dest_url: if safe {
                    dest_url
                } else {
                    CowStr::Borrowed("#")
                },
                title,
                id,
            })
        }
        // Remote image requests are not permitted from message markdown.
        Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => Event::Text("".into()),
        other => other,
    });
    let mut events = Vec::new();
    let mut literal = false;
    for event in parser {
        match event {
            Event::Start(Tag::Link { .. } | Tag::CodeBlock(_)) => {
                literal = true;
                events.push(event);
            }
            Event::End(TagEnd::Link | TagEnd::CodeBlock) => {
                literal = false;
                events.push(event);
            }
            Event::Text(text) if !literal => {
                // Mentions are inserted only into text events, never HTML attributes,
                // links or code. Everything else stays escaped by pulldown-cmark.
                let mut remaining = text.as_ref();
                while let Some(index) = remaining.find('@') {
                    events.push(Event::Text(remaining[..index].to_string().into()));
                    remaining = &remaining[index..];
                    let mention = [("@alex", "Alex Chen", "0"), ("@maya", "Maya", "1")]
                        .into_iter()
                        .find(|(key, _, _)| {
                            remaining.starts_with(key)
                                && remaining[key.len()..]
                                    .chars()
                                    .next()
                                    .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                        });
                    if let Some((key, name, id)) = mention {
                        events.push(Event::InlineHtml(format!("<button class=\"mention role-{id}\" data-action=\"profile:{id}\">@{name}</button>").into()));
                        remaining = &remaining[key.len()..];
                    } else {
                        events.push(Event::Text("@".into()));
                        remaining = &remaining[1..];
                    }
                }
                events.push(Event::Text(remaining.to_string().into()));
            }
            other => events.push(other),
        }
    }
    let mut output = String::new();
    html::push_html(&mut output, events.into_iter());
    output
}

pub fn fixture_messages() -> Vec<MessageVm> {
    let rows = [
        (
            1,
            "Today at 4:12 PM",
            "Anyone around for a few games tonight?",
            false,
            false,
            false,
            "",
        ),
        (
            3,
            "Today at 4:13 PM",
            "I'm in. Should be free after 7.",
            false,
            false,
            false,
            "",
        ),
        (
            0,
            "Today at 4:14 PM",
            "Same here! Let's use **Lounge** when everyone's ready.",
            false,
            false,
            false,
            "",
        ),
        (
            1,
            "Today at 4:16 PM",
            "Also, finally cleaned up my desk. A little more space makes a difference.",
            false,
            false,
            false,
            "image",
        ),
        (
            2,
            "Today at 4:18 PM",
            "That looks so good. The light is perfect.",
            false,
            true,
            false,
            "",
        ),
        (
            2,
            "Today at 4:18 PM",
            "I need to sort the cables under mine.",
            true,
            false,
            false,
            "",
        ),
        (
            3,
            "Today at 4:20 PM",
            "@alex I put the notes here for later.\n\n`cargo run --release` — that's all you need.",
            false,
            false,
            true,
            "file",
        ),
        (
            0,
            "Today at 4:22 PM",
            "Got it, thanks. **7:30 tonight?** @maya",
            false,
            false,
            false,
            "",
        ),
    ];
    rows.into_iter()
        .enumerate()
        .map(
            |(id, (author, time, body, continuation, reply, mention, attachment))| MessageVm {
                id,
                author,
                time: time.into(),
                body: body.into(),
                continuation,
                reply,
                mention,
                edited: id == 6,
                attachment: match attachment {
                    "image" => AttachmentVm::Image {
                        src: "public/mock-workspace.jpg".into(),
                        srcset: "public/mock-workspace.jpg 1x, public/mock-workspace.jpg 2x".into(),
                        alt: "A clean desk with a plant, notebook and warm light".into(),
                        width: 256,
                        height: 170,
                        name: "workspace.jpg".into(),
                    },
                    "file" => AttachmentVm::File {
                        name: "game-night-notes.md".into(),
                        size: "2.4 KB".into(),
                    },
                    _ => AttachmentVm::None,
                },
                reactions: if id == 3 {
                    vec![("1f525".into(), 4, true), ("1f44d".into(), 3, false)]
                } else if id == 7 {
                    vec![("1f44d".into(), 3, false)]
                } else {
                    vec![]
                },
            },
        )
        .collect()
}

/// Fixed-height history windows: only visible rows plus ten rows on each side.
pub fn history_window(scroll_top: f64, viewport: f64, count: usize) -> (usize, usize) {
    let start = ((scroll_top.max(0.) / 88.).floor() as usize)
        .saturating_sub(10)
        .min(count);
    let end = (((scroll_top + viewport).max(0.) / 88.).ceil() as usize + 10).min(count);
    (start, end.max(start))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_virtual_window() {
        assert_eq!(history_window(0., 900., 10000), (0, 21));
        let (start, end) = history_window(440000., 900., 10000);
        assert!(end - start <= 32);
        assert_eq!(history_window(9999999., 900., 20), (20, 20));
    }
    #[test]
    fn markdown_is_not_executable() {
        let output = markdown(
            "<script>alert(1)</script> [bad](javascript:alert) ![alt](https://evil.test/track)",
        );
        assert!(!output.contains("<script>"));
        assert!(!output.contains("href=\"javascript:"));
        assert!(!markdown("[bad](javascript:alert)").contains("href=\"javascript:"));
        assert!(!output.contains("<img"));
        assert!(markdown("**hello** `rust`").contains("<strong>hello</strong>"));
        assert!(!markdown("`@alex` [@alex](https://example.test/@alex)").contains("<button"));
        assert!(markdown("Hi @alex").contains("data-action=\"profile:0\""));
    }
    #[test]
    fn local_actions_and_input_bounds() {
        let mut vm = AppVm::fixture();
        vm.reduce("message-sheet:7");
        assert_eq!(vm.selected_message, 7);
        assert_eq!(vm.modal, "sheet");
        vm.reduce("react:0,1f525");
        assert_eq!(vm.messages[0].reactions[0], ("1f525".into(), 1, true));
        vm.reduce("send:hello");
        assert_eq!(vm.messages.last().unwrap().body, "hello");
        vm.reduce("permission:999");
        vm.reduce("role:999");
        assert_eq!(vm.selected_role, 4);
        vm.reduce("role-color:javascript:foo");
        assert_eq!(vm.roles[4].color, "#A8A8B0");
        vm.reduce("role-color:#123abc");
        assert!(vm.dirty);
        vm.reduce("save");
        assert!(!vm.dirty);
        vm.reduce("reorder:0,2");
        assert_eq!(vm.roles[2].name, "Owner");
    }
}
