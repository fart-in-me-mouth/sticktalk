//! Server identity in the web app: server tags (wearing one, the tag card),
//! role styles and icons, the server banner, animated icon and invite
//! background, rich invite cards and the vanity link. Views are in
//! `ui/identity.rs`, cropping in `public/identity.js`, looks in
//! `styles/identity.css`. Nothing here polls: settings load when a page
//! opens, a tag card when it is clicked, an invite card once when a link to it
//! is on screen (then from a bounded cache).
use super::*;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, VecDeque};
use sticktalk_client_core::http::decode_json;

/// Draws one invite card (a split module, see `invite_view`).
pub(super) type InviteView = fn(&App, &str) -> AnyView;
/// Invite cards remembered (each a few hundred bytes).
const INVITE_CARDS: usize = 32;

#[derive(Default)]
pub(super) struct State {
    /// The open server's identity settings, while a settings page shows them.
    pub admin: Option<IdentityAdmin>,
    /// The tags you may wear, while the profile editor is open.
    pub mine: Option<MyTags>,
    /// The tag card being shown, by server; None until it has loaded.
    pub tag_card: Option<(Id, Option<TagCard>)>,
    /// Invite cards by code; None for a code that does not lead anywhere.
    pub invites: RefCell<BTreeMap<String, Option<InviteCard>>>,
    order: RefCell<VecDeque<String>>,
    wanted: RefCell<BTreeSet<String>>,
    fetching: Cell<bool>,
    /// The banner of the server open in the directory preview.
    pub directory: Option<(Id, DirectoryArt)>,
    /// The invite card renderer, a split module loaded with the first
    /// invite link on screen.
    pub invite_view: Cell<Option<InviteView>>,
    loading_view: Cell<bool>,
    /// Where the image on its way goes, and its crop.
    upload: Option<(Id, GuildImageSlot, Option<Crop>)>,
}

fn read(id: &str) -> String {
    document()
        .get_element_by_id(id)
        .and_then(|e| {
            e.dyn_ref::<web_sys::HtmlInputElement>()
                .map(|i| i.value())
                .or_else(|| e.dyn_ref::<web_sys::HtmlSelectElement>().map(|s| s.value()))
        })
        .unwrap_or_default()
}
fn checked(id: &str) -> bool {
    document()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|i| i.checked())
}
fn color(id: &str) -> Option<u32> {
    u32::from_str_radix(read(id).trim().trim_start_matches('#'), 16)
        .ok()
        .filter(|c| *c <= 0xFF_FFFF)
}
fn file(id: &str) -> Option<web_sys::File> {
    document()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .and_then(|e| e.files())
        .and_then(|f| f.get(0))
}
fn send(a: &mut App, cmd: IdentityCommand) {
    a.command(Command::Identity(cmd));
}
fn slot(s: &str) -> GuildImageSlot {
    match s {
        "icon" => GuildImageSlot::Icon,
        "background" => GuildImageSlot::InviteBackground,
        _ => GuildImageSlot::Banner,
    }
}
pub(super) fn admin_page(page: &str) -> bool {
    matches!(page, "id-tags" | "id-appearance" | "id-vanity")
}

/// A dialog opened: load what it shows. The loading code (and the decoders
/// of what it loads) is a split module, out of the first load.
pub(super) fn opened(app: Shared, name: &str) {
    if admin_page(name)
        || name == "profile-edit"
        || name.starts_with("discover:")
        || name.starts_with("join:")
    {
        let name = name.to_owned();
        spawn_local(async move { opened_lazy(app, name).await });
    }
}
#[lazy]
fn opened_lazy(app: Shared, name: String) {
    let name = name.as_str();
    let guild = app.borrow().guild;
    if admin_page(name) {
        if let Some(g) = guild {
            app.borrow_mut().identity.admin = None;
            load_admin(app, g);
        }
    } else if name == "profile-edit" {
        load_mine(app);
    } else if let Some(g) = name.strip_prefix("discover:").and_then(id) {
        let (http, epoch) = {
            let a = app.borrow();
            (a.http(), a.epoch)
        };
        spawn_local(async move {
            let result = http
                .json::<DirectoryArt>(&format!("/directory/{}/art", g.0))
                .await;
            let mut a = app.borrow_mut();
            if a.epoch == epoch {
                if let Ok(art) = result {
                    a.identity.directory = Some((g, art));
                    a.touch();
                }
            }
        });
    } else if let Some(code) = name.strip_prefix("join:") {
        // The join screen behind an invite: its card (and background).
        want_invite(&app.borrow(), code);
    }
}
fn load_admin(app: Shared, g: Id) {
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    spawn_local(async move {
        let result = http
            .json::<IdentityAdmin>(&format!("/guilds/{}/identity", g.0))
            .await;
        let mut a = app.borrow_mut();
        if a.epoch != epoch {
            return;
        }
        match result {
            Ok(v) => a.identity.admin = Some(v),
            Err(e) => a.error = e,
        }
        a.touch();
    });
}
fn load_mine(app: Shared) {
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    spawn_local(async move {
        let result = http.json::<MyTags>("/users/@me/tags").await;
        let mut a = app.borrow_mut();
        if a.epoch == epoch {
            if let Ok(v) = result {
                a.identity.mine = Some(v);
                a.touch();
            }
        }
    });
}
fn load_tag(app: Shared, g: Id) {
    let (http, epoch) = {
        let mut a = app.borrow_mut();
        a.identity.tag_card = Some((g, None));
        (a.http(), a.epoch)
    };
    spawn_local(async move {
        let result = http.json::<TagCard>(&format!("/tags/{}", g.0)).await;
        let mut a = app.borrow_mut();
        if a.epoch != epoch || a.identity.tag_card.as_ref().map(|t| t.0) != Some(g) {
            return;
        }
        match result {
            Ok(card) => a.identity.tag_card = Some((g, Some(card))),
            Err(_) => {
                // The tag went away: say so in the card.
                a.identity.tag_card = None;
                a.error = "That server tag is no longer available.".into();
                a.dialog.clear();
            }
        }
        a.touch();
    });
}

/// Called while rendering: an invite link is on screen. Its card is fetched
/// once, after this render, and kept in a small cache.
// Decoding an invite card is only needed once one is on screen.
#[lazy]
fn invite_decoder() -> fn(sticktalk_client_core::http::HttpResponse) -> Result<InviteCard, String> {
    decode_json::<InviteCard>
}
pub(super) fn want_invite(a: &App, code: &str) {
    let s = &a.identity;
    if s.invites.borrow().contains_key(code) {
        return;
    }
    s.wanted.borrow_mut().insert(code.to_owned());
    if s.fetching.replace(true) {
        return;
    }
    let owner = a.owner.clone();
    spawn_local(async move {
        loop {
            let Some(app) = owner.upgrade() else { return };
            let (http, epoch, code) = {
                let a = app.borrow();
                let next = a.identity.wanted.borrow_mut().pop_first();
                match next {
                    Some(c) => (a.http(), a.epoch, c),
                    None => {
                        a.identity.fetching.set(false);
                        return;
                    }
                }
            };
            let decode = invite_decoder().await;
            let result = http
                .execute(
                    "GET",
                    &format!("/api/v1/invites/{}/card", encode(&code)),
                    Body::Empty,
                    vec![],
                    30_000,
                )
                .await
                .and_then(decode);
            let a = app.borrow();
            if a.epoch != epoch {
                a.identity.fetching.set(false);
                return;
            }
            {
                let mut cards = a.identity.invites.borrow_mut();
                let mut order = a.identity.order.borrow_mut();
                if cards.len() >= INVITE_CARDS {
                    if let Some(old) = order.pop_front() {
                        cards.remove(&old);
                    }
                }
                order.push_back(code.clone());
                cards.insert(code, result.ok());
            }
            a.touch();
        }
    });
}
fn encode(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}
/// The invite card renderer, if loaded; asks for it otherwise.
pub(super) fn invite_view(a: &App) -> Option<InviteView> {
    let view = a.identity.invite_view.get();
    if view.is_none() && !a.identity.loading_view.replace(true) {
        let owner = a.owner.clone();
        spawn_local(async move {
            let render = super::ui::load_invite_view().await;
            if let Some(app) = owner.upgrade() {
                let a = app.borrow();
                a.identity.invite_view.set(Some(render));
                a.touch();
            }
        });
    }
    view
}
/// A joined server's invite card says you are in it.
pub(super) fn forget_invites(a: &App) {
    a.identity.invites.borrow_mut().clear();
    a.identity.order.borrow_mut().clear();
}

/// `id:` actions. Their code is a split module, loaded on first use.
pub(super) fn action(app: Shared, arg: &str) {
    let arg = arg.to_owned();
    spawn_local(async move { action_lazy(app, arg).await });
}
#[lazy]
fn action_lazy(app: Shared, arg: String) {
    let mut parts = arg.split(',');
    let verb = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    let num = |i: usize| rest.get(i).and_then(|v| id(v));
    let guild = app.borrow().guild;
    match verb {
        // A click on a tag: its card.
        "tag" => {
            if let Some(g) = num(0) {
                load_tag(app.clone(), g);
                super::operations::action(app, &format!("dialog:id-tag:{}", g.0));
            }
        }
        "wear" => send(
            &mut app.borrow_mut(),
            IdentityCommand::WearTag { guild_id: num(0) },
        ),
        "hide" => {
            if let Some(g) = guild {
                send(
                    &mut app.borrow_mut(),
                    IdentityCommand::SetHideTags {
                        guild_id: g,
                        hide: checked("id-hide-tags"),
                    },
                );
            }
        }
        // `upload,<slot>,<zoom>,<x>,<y>`: the picture is in #identity-upload.
        "upload" => {
            let Some(g) = guild else { return };
            let n = |i: usize| rest.get(i).and_then(|v| v.parse::<u16>().ok());
            let crop = match (n(1), n(2), n(3)) {
                (Some(zoom), Some(x), Some(y)) => Some(Crop {
                    zoom,
                    x: x.min(100) as u8,
                    y: y.min(100) as u8,
                })
                .filter(|c| c.valid() && !c.is_identity()),
                _ => None,
            };
            let Some(f) = file("identity-upload") else {
                return;
            };
            let epoch = {
                let mut a = app.borrow_mut();
                a.identity.upload = Some((g, slot(rest.first().copied().unwrap_or("")), crop));
                a.epoch
            };
            spawn_local(async move {
                super::operations::upload(app, UploadPurpose::Avatar, f, None, None, epoch).await;
            });
        }
        "remove" => {
            if let Some(g) = guild {
                send(
                    &mut app.borrow_mut(),
                    IdentityCommand::SetImage {
                        guild_id: g,
                        slot: slot(rest.first().copied().unwrap_or("")),
                        image: None,
                    },
                );
            }
        }
        "role-style" => {
            let (Some(g), Some(r)) = (guild, num(0)) else {
                return;
            };
            let style = match read("id-role-paint").as_str() {
                "gradient" => Some(RoleStyle {
                    paint: RolePaint::Gradient,
                    color2: color("id-role-c2"),
                    color3: checked("id-role-c3-on")
                        .then(|| color("id-role-c3"))
                        .flatten(),
                }),
                "holographic" => Some(RoleStyle {
                    paint: RolePaint::Holographic,
                    color2: None,
                    color3: None,
                }),
                _ => None,
            };
            send(
                &mut app.borrow_mut(),
                IdentityCommand::SetRoleStyle {
                    guild_id: g,
                    role_id: r,
                    style,
                },
            );
        }
        // `role-icon,<role>,emoji|custom|clear[,<emoji id>]`
        "role-icon" => {
            let (Some(g), Some(r)) = (guild, num(0)) else {
                return;
            };
            let icon = match rest.get(1).copied() {
                Some("emoji") => Some(RoleIconRef::Emoji {
                    emoji: read("id-role-emoji").trim().to_owned(),
                }),
                Some("custom") => num(2).map(|emoji_id| RoleIconRef::Custom { emoji_id }),
                _ => None,
            };
            send(
                &mut app.borrow_mut(),
                IdentityCommand::SetRoleIcon {
                    guild_id: g,
                    role_id: r,
                    icon,
                },
            );
        }
        "role-icon-upload" => {
            let (Some(g), Some(r)) = (guild, num(0)) else {
                return;
            };
            let Some(f) = file("id-role-icon-file") else {
                return;
            };
            if f.size() > MAX_ROLE_ICON_BYTES as f64 {
                let mut a = app.borrow_mut();
                a.error = "That picture is larger than 256 KB.".into();
                a.touch();
                return;
            }
            let (http, epoch) = {
                let a = app.borrow();
                (a.http(), a.epoch)
            };
            spawn_local(async move {
                let bytes = match wasm_bindgen_futures::JsFuture::from(f.array_buffer()).await {
                    Ok(buf) => js_sys::Uint8Array::new(&buf).to_vec(),
                    Err(_) => return,
                };
                let result = http
                    .execute(
                        "POST",
                        &format!("/api/v1/guilds/{}/roles/{}/icon", g.0, r.0),
                        Body::Bytes(bytes),
                        vec![("content-type".into(), "application/octet-stream".into())],
                        30_000,
                    )
                    .await
                    .and_then(decode_json::<RoleLook>);
                let mut a = app.borrow_mut();
                if a.epoch != epoch {
                    return;
                }
                match result {
                    Ok(_) => {
                        a.notice = "Role icon saved.".into();
                        a.error = a.notice.clone();
                    }
                    Err(_) => {
                        a.error = "That picture could not be used. Choose a PNG or WebP of at most 1024 × 1024.".into()
                    }
                }
                a.touch();
            });
        }
        // A tag card or invite card offers joining.
        "join" => {
            if let Some(code) = rest.first() {
                let mut a = app.borrow_mut();
                a.dialog.clear();
                a.dialog_generation = a.dialog_generation.wrapping_add(1);
                a.command(Command::JoinGuild {
                    invite_code: (*code).to_owned(),
                });
                a.touch();
            }
        }
        // The server takes applications: community's form, for this invite.
        "apply" => {
            if let Some(code) = rest.first().map(|c| encode(c)) {
                super::operations::action(app.clone(), &format!("dialog:cm-apply:invite:{code}"));
                app.borrow_mut().invite_wanted = code.clone();
                super::features::spawn(
                    app,
                    super::features::Feature::Community,
                    super::features::Call::Other("gate", code),
                );
            }
        }
        "discover" => {
            if let Some(g) = num(0) {
                super::operations::action(app, &format!("dialog:discover:{}", g.0));
            }
        }
        "vanity-release" => {
            if let Some(g) = guild {
                send(
                    &mut app.borrow_mut(),
                    IdentityCommand::SetVanity {
                        guild_id: g,
                        code: None,
                    },
                );
            }
        }
        "error" => {
            let mut a = app.borrow_mut();
            a.error = rest.join(",");
            a.touch();
        }
        _ => {}
    }
}

pub(super) fn submit(app: Shared, form: &str) {
    let form = form.to_owned();
    spawn_local(async move { submit_lazy(app, form).await });
}
#[lazy]
fn submit_lazy(app: Shared, form: String) {
    let form = form.as_str();
    let mut a = app.borrow_mut();
    let Some(g) = a.guild else { return };
    match form {
        "id-tag" => {
            let text = read("id-tag-text").trim().to_owned();
            let tag = (!text.is_empty()).then(|| TagEdit {
                text,
                glyph: read("id-tag-glyph"),
                color: color("id-tag-c1").unwrap_or(0x1C3A2E),
                color2: color("id-tag-c2").unwrap_or(0x3DDC97),
                invite_code: Some(read("id-tag-invite")).filter(|v| !v.is_empty()),
            });
            send(&mut a, IdentityCommand::SetTag { guild_id: g, tag });
        }
        "id-tag-remove" => send(
            &mut a,
            IdentityCommand::SetTag {
                guild_id: g,
                tag: None,
            },
        ),
        "id-vanity" => send(
            &mut a,
            IdentityCommand::SetVanity {
                guild_id: g,
                code: Some(read("id-vanity-code")),
            },
        ),
        _ => return,
    }
    media_action(&format!("form-sent:{form}"));
}

/// An upload finished. Returns whether it was for a server image.
pub(super) fn uploaded(a: &mut App, purpose: UploadPurpose, file: &Attachment) -> bool {
    if purpose != UploadPurpose::Avatar {
        return false;
    }
    let Some((g, slot, crop)) = a.identity.upload.take() else {
        return false;
    };
    send(
        a,
        IdentityCommand::SetImage {
            guild_id: g,
            slot,
            image: Some(ImageRef { id: file.id, crop }),
        },
    );
    true
}

fn form_of(cmd: &IdentityCommand) -> Option<&'static str> {
    match cmd {
        IdentityCommand::SetTag { tag: Some(_), .. } => Some("id-tag"),
        IdentityCommand::SetTag { tag: None, .. } => Some("id-tag-remove"),
        IdentityCommand::SetVanity { code: Some(_), .. } => Some("id-vanity"),
        _ => None,
    }
}
pub(super) fn owns(cmd: &Command) -> bool {
    matches!(cmd, Command::Identity(_))
}
pub(super) fn failed(cmd: Option<&Command>) {
    if let Some(Command::Identity(c)) = cmd {
        if let Some(form) = form_of(c) {
            media_action(&format!("form-failed:{form}"));
        }
    }
}
pub(super) fn answered(app: Shared, cmd: Command, result: CommandResult) {
    let Command::Identity(c) = cmd else { return };
    let (reload, mine) = {
        let mut a = app.borrow_mut();
        a.error.clear();
        a.notice.clear();
        if let Some(form) = form_of(&c) {
            a.saved = Some((a.dialog_generation, form));
            media_action(&format!("form-saved:{form}"));
        }
        let notice = match (&c, &result) {
            (IdentityCommand::WearTag { guild_id: Some(_) }, _) => "You’re wearing the tag.",
            (IdentityCommand::WearTag { guild_id: None }, _) => "Tag removed from your profile.",
            (IdentityCommand::SetVanity { code: None, .. }, _) => {
                "Vanity link released. It’s held for 30 days."
            }
            (IdentityCommand::SetImage { .. }, _) => "Saved.",
            (IdentityCommand::SetRoleStyle { .. } | IdentityCommand::SetRoleIcon { .. }, _) => {
                "Role updated."
            }
            _ => "",
        };
        if !notice.is_empty() {
            a.notice = notice.into();
            a.error = a.notice.clone();
        }
        a.touch();
        (
            admin_page(&a.dialog).then_some(a.guild).flatten(),
            matches!(c, IdentityCommand::WearTag { .. }) && a.dialog == "profile-edit",
        )
    };
    if let Some(g) = reload {
        load_admin(app.clone(), g);
    }
    if mine {
        load_mine(app);
    }
}
