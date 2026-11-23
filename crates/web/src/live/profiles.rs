//! Profiles in the web app: the profile card behind a popout, the person's own
//! profiles for the editor, image uploads for avatars and banners, and the
//! commands that save them. Views are in `ui/profiles.rs`; cropping and the
//! live preview are in `public/profiles.js`. Nothing here polls: data is
//! fetched when a card or the editor opens.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    User,
    Server,
    Badges,
}
#[derive(Default)]
pub(super) struct State {
    /// The signed-in person's profiles, while the editor is open.
    pub mine: Option<MyProfiles>,
    /// The card of the open profile popout, and the server it was opened in.
    pub card: Option<(ProfileCard, Option<Id>)>,
    pub tab: Tab,
    /// The server whose profile the editor shows.
    pub guild: Option<Id>,
    /// Where the upload on its way goes: server (none for global), slot, crop.
    upload: Option<(Option<Id>, ImageSlot, Option<Crop>)>,
}

fn read(id: &str) -> String {
    document()
        .get_element_by_id(id)
        .and_then(|e| {
            e.dyn_ref::<web_sys::HtmlInputElement>()
                .map(|i| i.value())
                .or_else(|| e.dyn_ref::<web_sys::HtmlSelectElement>().map(|s| s.value()))
                .or_else(|| {
                    e.dyn_ref::<web_sys::HtmlTextAreaElement>()
                        .map(|t| t.value())
                })
        })
        .unwrap_or_default()
}
fn checked(id: &str) -> bool {
    document()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|i| i.checked())
}
pub(super) fn hex(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xFF_FFFF)
}
fn color(id: &str) -> Option<u32> {
    u32::from_str_radix(read(id).trim().trim_start_matches('#'), 16)
        .ok()
        .filter(|c| *c <= 0xFF_FFFF)
}
fn item(id: &str) -> Option<String> {
    Some(read(id)).filter(|v| !v.is_empty())
}
/// The name style fields `pf-name-*` describe, when "Style my name" is on.
fn name_style() -> Option<NameStyle> {
    if !checked("pf-name-on") {
        return None;
    }
    let effect = NameEffect::parse(&read("pf-name-effect")).unwrap_or_default();
    Some(NameStyle {
        font: NameFont::parse(&read("pf-name-font")).unwrap_or_default(),
        color: color("pf-name-c1").unwrap_or(0xFFFFFF),
        color2: matches!(
            effect,
            NameEffect::Gradient | NameEffect::Shimmer | NameEffect::Outline
        )
        .then(|| color("pf-name-c2"))
        .flatten(),
        effect,
    })
}
fn tint() -> (Option<u32>, Option<u32>) {
    if checked("pf-colors-on") {
        (color("pf-primary"), color("pf-accent"))
    } else {
        (None, None)
    }
}
fn launch(a: &mut App, cmd: ProfilesCommand) {
    a.command(Command::Launch(LaunchCommand::Profiles(cmd)));
}
fn fail(app: &Shared, e: String) {
    let mut a = app.borrow_mut();
    a.error = e;
    a.touch();
}

/// A dialog opened: the editor loads the person's profiles.
pub(super) fn opened(app: Shared, name: &str) {
    if name != "profile-edit" {
        return;
    }
    {
        let mut a = app.borrow_mut();
        if a.profiles.guild.is_none() {
            a.profiles.guild = a.guild.or_else(|| a.model.guilds.keys().next().copied());
        }
    }
    refresh(app);
}
fn refresh(app: Shared) {
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    spawn_local(async move {
        let result = http.json::<MyProfiles>("/users/@me/profiles").await;
        let mut a = app.borrow_mut();
        if a.epoch != epoch {
            return;
        }
        match result {
            Ok(p) => a.profiles.mine = Some(p),
            Err(e) => a.error = e,
        }
        a.touch();
    });
}
/// A profile popout is opening for `user`: fetch its card for this server.
pub(super) fn card_requested(app: Shared, user: &str) {
    let Some(u) = id(user) else { return };
    let (http, epoch, guild) = {
        let mut a = app.borrow_mut();
        a.profiles.card = None;
        (a.http(), a.epoch, a.guild)
    };
    spawn_local(async move {
        let path = match guild {
            Some(g) => format!("/users/{}/card?guild={}", u.0, g.0),
            None => format!("/users/{}/card", u.0),
        };
        let result = http.json::<ProfileCard>(&path).await;
        let mut a = app.borrow_mut();
        if a.epoch != epoch {
            return;
        }
        // A card that cannot be read leaves the basic popout in place.
        if let Ok(card) = result {
            a.profiles.card = Some((card, guild));
            a.touch();
        }
    });
}

pub(super) fn action(app: Shared, arg: &str) {
    let mut parts = arg.split(',');
    let verb = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    let guild = |i: usize| rest.get(i).and_then(|v| id(v));
    match verb {
        "tab" => {
            let mut a = app.borrow_mut();
            a.profiles.tab = match rest.first().copied() {
                Some("server") => Tab::Server,
                Some("badges") => Tab::Badges,
                _ => Tab::User,
            };
            a.touch();
        }
        "guild" => {
            let mut a = app.borrow_mut();
            a.profiles.guild = guild(0).filter(|g| a.model.guilds.contains_key(g));
            a.touch();
        }
        // From the server menu: this server's profile in the editor.
        "open-server" => {
            {
                let mut a = app.borrow_mut();
                a.profiles.tab = Tab::Server;
                a.profiles.guild = guild(0);
            }
            super::operations::action(app, "dialog:profile-edit");
        }
        // `upload,<slot>,<guild or 0>,<zoom>,<x>,<y>`: the file is in #profiles-upload.
        "upload" => {
            let slot = match rest.first().copied() {
                Some("banner") => ImageSlot::Banner,
                _ => ImageSlot::Avatar,
            };
            let n = |i: usize| rest.get(i).and_then(|v| v.parse::<u16>().ok());
            let crop = match (n(2), n(3), n(4)) {
                (Some(zoom), Some(x), Some(y)) => Some(Crop {
                    zoom,
                    x: x.min(100) as u8,
                    y: y.min(100) as u8,
                })
                .filter(|c| c.valid() && !c.is_identity()),
                _ => None,
            };
            let file = document()
                .get_element_by_id("profiles-upload")
                .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
                .and_then(|e| e.files())
                .and_then(|f| f.get(0));
            let Some(file) = file else { return };
            let epoch = {
                let mut a = app.borrow_mut();
                a.profiles.upload = Some((guild(1), slot, crop));
                a.epoch
            };
            spawn_local(async move {
                super::operations::upload(app, UploadPurpose::Avatar, file, None, None, epoch)
                    .await;
            });
        }
        "remove" => {
            let slot = match rest.first().copied() {
                Some("banner") => ImageSlot::Banner,
                _ => ImageSlot::Avatar,
            };
            launch(
                &mut app.borrow_mut(),
                ProfilesCommand::SetImage {
                    guild_id: guild(1),
                    slot,
                    image: None,
                },
            );
        }
        // A moderator, from a member's card: `reset,<guild>,<user>,<avatar|banner>`.
        "reset" => {
            if let (Some(g), Some(u)) = (guild(0), guild(1)) {
                let what = rest.get(2).copied().unwrap_or_default();
                launch(
                    &mut app.borrow_mut(),
                    ProfilesCommand::Reset {
                        guild_id: g,
                        user_id: u,
                        avatar: what == "avatar",
                        banner: what == "banner",
                        reason: None,
                    },
                );
            }
        }
        "nick-reset" => {
            if let (Some(g), Some(u)) = (guild(0), guild(1)) {
                app.borrow_mut()
                    .command(Command::Launch(LaunchCommand::SetNickname {
                        guild_id: g,
                        user_id: u,
                        nickname: None,
                    }));
            }
        }
        "error" => fail(&app, rest.join(",")),
        _ => {}
    }
}
pub(super) const FORMS: [&str; 3] = ["profiles-user", "profiles-server", "profiles-badges"];
pub(super) fn submit(app: Shared, form: &str) {
    let mut a = app.borrow_mut();
    let Some(me) = a.model.me.as_ref().map(|me| me.user.clone()) else {
        return;
    };
    let (primary, accent) = tint();
    match form {
        "profiles-user" => {
            let display = read("f-name").trim().to_owned();
            let status = read("f-value").trim().to_owned();
            let text = |s: String| {
                if s.is_empty() {
                    Patch::Clear
                } else {
                    Patch::Set(s)
                }
            };
            if display != me.display_name.clone().unwrap_or_default()
                || status != me.status_text.clone().unwrap_or_default()
            {
                a.command(Command::UpdateProfile(ProfilePatch {
                    display_name: text(display),
                    status_text: text(status),
                    ..Default::default()
                }));
            }
            launch(
                &mut a,
                ProfilesCommand::Update(ProfileEdit {
                    pronouns: read("pf-pronouns"),
                    about_me: read("pf-about"),
                    banner_color: color("pf-banner-color").unwrap_or(0),
                    primary,
                    accent,
                    name: name_style(),
                    decoration: item("pf-deco"),
                    nameplate: item("pf-plate"),
                    effect: item("pf-effect"),
                    frame: item("pf-frame"),
                }),
            );
        }
        "profiles-server" => {
            let Some(g) = a.profiles.guild else { return };
            let nickname = read("pf-nickname").trim().to_owned();
            let current = a
                .profiles
                .mine
                .as_ref()
                .and_then(|m| m.servers.iter().find(|s| s.guild_id == g))
                .and_then(|s| s.profile.nickname.clone())
                .unwrap_or_default();
            if nickname != current {
                a.command(Command::Launch(LaunchCommand::SetNickname {
                    guild_id: g,
                    user_id: me.id,
                    nickname: Some(nickname).filter(|n| !n.is_empty()),
                }));
            }
            let text = |id: &str| Some(read(id).trim().to_owned()).filter(|v| !v.is_empty());
            launch(
                &mut a,
                ProfilesCommand::UpdateServer {
                    guild_id: g,
                    edit: ServerProfileEdit {
                        pronouns: text("pf-pronouns"),
                        about_me: text("pf-about"),
                        primary,
                        accent,
                        name: name_style(),
                        decoration: item("pf-deco"),
                        nameplate: item("pf-plate"),
                        effect: item("pf-effect"),
                        frame: item("pf-frame"),
                    },
                },
            );
        }
        "profiles-badges" => {
            let shown = read("pf-badges")
                .split(',')
                .filter(|v| !v.is_empty())
                .take(MAX_SHOWN_BADGES)
                .map(str::to_owned)
                .collect();
            launch(&mut a, ProfilesCommand::SetBadges { shown });
        }
        _ => return,
    }
    media_action(&format!("form-sent:{form}"));
}
/// An upload finished. Returns whether it was for a profile image.
pub(super) fn uploaded(a: &mut App, purpose: UploadPurpose, file: &Attachment) -> bool {
    if purpose != UploadPurpose::Avatar {
        return false;
    }
    let Some((guild, slot, crop)) = a.profiles.upload.take() else {
        return false;
    };
    launch(
        a,
        ProfilesCommand::SetImage {
            guild_id: guild,
            slot,
            image: Some(ImageRef { id: file.id, crop }),
        },
    );
    true
}
fn form_of(cmd: &ProfilesCommand) -> Option<&'static str> {
    match cmd {
        ProfilesCommand::Update(_) => Some("profiles-user"),
        ProfilesCommand::UpdateServer { .. } => Some("profiles-server"),
        ProfilesCommand::SetBadges { .. } => Some("profiles-badges"),
        _ => None,
    }
}
pub(super) fn owns(cmd: &Command) -> bool {
    matches!(cmd, Command::Launch(LaunchCommand::Profiles(_)))
}
/// A profile command was refused.
pub(super) fn failed(cmd: Option<&Command>) {
    if let Some(Command::Launch(LaunchCommand::Profiles(c))) = cmd {
        if let Some(form) = form_of(c) {
            media_action(&format!("form-failed:{form}"));
        }
    }
}
/// A profile command succeeded: the editor shows it saved and reloads; a
/// moderator's reset reloads the card it was made from.
pub(super) fn answered(app: Shared, cmd: Command) {
    let Command::Launch(LaunchCommand::Profiles(c)) = cmd else {
        return;
    };
    let editing = {
        let mut a = app.borrow_mut();
        a.error.clear();
        if let Some(form) = form_of(&c) {
            a.saved = Some((a.dialog_generation, form));
            media_action(&format!("form-saved:{form}"));
        }
        if let ProfilesCommand::Reset { user_id, .. } = &c {
            a.notice = "Server profile reset.".into();
            a.error = a.notice.clone();
            let user = user_id.0.to_string();
            drop(a);
            card_requested(app, &user);
            return;
        }
        a.touch();
        a.dialog == "profile-edit"
    };
    if editing {
        refresh(app);
    }
}
