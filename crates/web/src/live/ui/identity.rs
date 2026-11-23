//! Server identity views (`docs/features/identity.md`).
//!
//! The renderers lists use (a name's role style, the role icon and the server
//! tag after it, the sidebar banner, the rail's animated icon, the invite
//! card's placeholder in messages) are small and draw only what the snapshot
//! already carries: they are in the core. The settings pages, the role
//! editor's style and icon controls and the tag card are this feature's view
//! module (`load_identity_dialog`), the invite card renderer another
//! (`load_invite_view`). Looks are `styles/identity.css`; badge glyphs are CSS
//! masks on `/public/tags/<glyph>.svg`, fetched only when a tag is on screen.
use super::*;
use crate::live::identity::admin_page;

fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xFF_FFFF)
}
/// A glyph name that is safe in a URL (the server only stores known ones;
/// the list itself, `TAG_GLYPHS`, is only needed by the tag editor).
fn glyph(id: &str) -> &str {
    if !id.is_empty() && id.len() <= 16 && id.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
        id
    } else {
        "star"
    }
}

// ------------------------------------------------- the snapshot's fields
//
// A user's tag and a server's or role's look arrive undecoded in the web
// build (`sticktalk_proto::Deferred`): their derived decoders would be in the
// first load, with every user, server and role. These read the few fields the
// renderers draw straight from the JSON.

fn num(v: &serde_json::Value) -> Option<u64> {
    v.as_u64()
}
fn color_of(v: &serde_json::Value) -> Option<u32> {
    num(v).and_then(|c| u32::try_from(c).ok())
}
/// The tag a user summary carries.
fn worn_tag(u: &UserSummary) -> Option<WornTag> {
    let v = u.style.tag.as_ref()?.raw();
    Some(WornTag {
        guild_id: Id(num(&v["guild_id"])?),
        text: v["text"].as_str()?.to_owned(),
        glyph: v["glyph"].as_str().unwrap_or_default().to_owned(),
        color: color_of(&v["color"]).unwrap_or(0),
        color2: color_of(&v["color2"]).unwrap_or(0),
    })
}
fn image_of(v: &serde_json::Value) -> Option<ImageMeta> {
    let crop = &v["crop"];
    Some(ImageMeta {
        id: Id(num(&v["id"])?),
        animated: v["animated"].as_bool().unwrap_or(false),
        crop: (!crop.is_null()).then(|| Crop {
            zoom: num(&crop["zoom"]).unwrap_or(100) as u16,
            x: num(&crop["x"]).unwrap_or(50).min(100) as u8,
            y: num(&crop["y"]).unwrap_or(50).min(100) as u8,
        }),
    })
}
/// A server's banner or animated icon (`slot` is `banner` or `icon`).
fn guild_image(g: &Guild, slot: &str) -> Option<ImageMeta> {
    image_of(&g.look.as_ref()?.raw()[slot])
}
fn hides_tags(g: &Guild) -> bool {
    g.look
        .as_ref()
        .is_some_and(|l| l.raw()["hide_tags"].as_bool() == Some(true))
}
/// A role's gradient or holographic style.
fn role_style(r: &Role) -> Option<RoleStyle> {
    let v = &r.look.as_ref()?.raw()["style"];
    let paint = match v["paint"].as_str()? {
        "gradient" => RolePaint::Gradient,
        "holographic" => RolePaint::Holographic,
        _ => return None,
    };
    Some(RoleStyle {
        paint,
        color2: color_of(&v["color2"]),
        color3: color_of(&v["color3"]),
    })
}
fn role_icon_of(r: &Role) -> Option<RoleIcon> {
    let v = &r.look.as_ref()?.raw()["icon"];
    if v.is_null() {
        return None;
    }
    Some(RoleIcon {
        rev: num(&v["rev"]).and_then(|n| u32::try_from(n).ok()),
        emoji_id: num(&v["emoji_id"]).map(Id),
        emoji: v["emoji"].as_str().unwrap_or_default().to_owned(),
    })
}

// ------------------------------------------------------------- list views

/// The badge: the glyph in the second color on a tile of the first.
fn badge(t: &WornTag, class: &'static str) -> AnyView {
    view!{<span class=class aria-hidden="true" style=format!("--tag-c1:{};--tag-c2:{}",hex(t.color),hex(t.color2))><span class="tag-glyph" style=format!("--tag-glyph:url(/public/tags/{}.svg)",glyph(&t.glyph))></span></span>}.into_any()
}
/// A server tag chip; a click opens its card.
fn chip(t: &WornTag) -> AnyView {
    view!{<button type="button" class="server-tag" data-live=format!("id:tag,{}",t.guild_id.0) aria-label=format!("Server tag {}",t.text) title=format!("Server tag: {}",t.text)>{badge(t,"tag-badge")}<span class="tag-text">{t.text.clone()}</span></button>}.into_any()
}
/// The open server hides other servers' tags.
fn hidden_here(a: &App, t: &WornTag) -> bool {
    a.guild != Some(t.guild_id)
        && a.guild
            .and_then(|g| a.model.guilds.get(&g))
            .is_some_and(|g| hides_tags(&g.guild))
}
/// The tag `u` wears, from the freshest copy of them the app has.
fn worn(a: &App, u: &UserSummary) -> Option<WornTag> {
    worn_tag(a.model.users.get(&u.id).unwrap_or(u)).filter(|t| !hidden_here(a, t))
}
pub(super) fn tag(a: &App, u: &UserSummary) -> AnyView {
    worn(a, u).map_or_else(|| ().into_any(), |t| chip(&t))
}
/// The chip inside a row that is itself a button (member list, direct
/// messages, friends): not a second button; the row opens the profile, whose
/// card has the clickable tag.
pub(super) fn tag_static(a: &App, u: &UserSummary) -> AnyView {
    match worn(a, u) {
        Some(t) => view!{<span class="server-tag" title=format!("Server tag: {}",t.text)>{badge(&t,"tag-badge")}<span class="tag-text">{t.text.clone()}</span></span>}.into_any(),
        None => ().into_any(),
    }
}
/// A direct message row: the other person's tag.
pub(super) fn dm_tag(a: &App, c: &ChannelSummary) -> AnyView {
    if c.kind != ChannelKind::Direct {
        return ().into_any();
    }
    let me = a.model.me.as_ref().map(|m| m.user.id);
    match c
        .recipient_ids
        .iter()
        .find(|id| Some(**id) != me)
        .and_then(|id| a.model.users.get(id))
    {
        Some(u) => tag_static(a, u),
        None => ().into_any(),
    }
}
/// For the call tile list (`#live-call-people`): the tag text only.
pub(super) fn tag_text(a: &App, user: Id) -> String {
    a.model
        .users
        .get(&user)
        .and_then(|u| worn(a, u))
        .map(|t| t.text)
        .unwrap_or_default()
}

/// The highest of the member's roles in the open server that `pick` accepts.
fn top_role(a: &App, user: Id, pick: impl Fn(&Role) -> bool) -> Option<&Role> {
    let m = a
        .model
        .members
        .get(&user)
        .filter(|m| Some(m.guild_id) == a.guild)?;
    m.role_ids
        .iter()
        .filter_map(|id| a.roles.get(id))
        .filter(|r| pick(r))
        .max_by_key(|r| r.position)
}
/// Class and CSS variables that paint a name in a role's style. Colors pass
/// through the same legibility floor as solid role colors; the stylesheet
/// then mixes in the theme's text color (`--role-ink`).
fn paint(r: &Role, s: &RoleStyle) -> (&'static str, String) {
    let c = |v: u32| hex(crate::wire::legible_role_color(v));
    match s.paint {
        RolePaint::Gradient => {
            let c2 = s.color2.unwrap_or(r.color);
            (
                "role-paint role-gradient",
                format!(
                    "--role-c1:{};--role-c2:{};--role-c3:{}",
                    c(r.color),
                    c(c2),
                    c(s.color3.unwrap_or(c2))
                ),
            )
        }
        RolePaint::Holographic => (
            "role-paint role-holo",
            format!(
                "--role-c1:{}",
                c(if r.color == 0 { 0xC9B6FF } else { r.color })
            ),
        ),
    }
}
/// The role whose color a member's name takes: the highest with a color or
/// a style.
fn name_role(a: &App, user: Id) -> Option<&Role> {
    top_role(a, user, |r| r.color != 0 || role_style(r).is_some())
}
/// A name in its role's style. The user's own name style wins (the inner
/// `.styled-name` is left alone) unless "Show plain names" is on; "Role
/// colors" off or as a dot turn the style off. See identity.css.
pub(super) fn role_name(a: &App, user: Id, shown: &str, inner: AnyView) -> AnyView {
    match name_role(a, user).and_then(|r| role_style(r).map(|s| (r, s))) {
        Some((r, s)) => {
            let (class, vars) = paint(r, &s);
            view! {<span class=class style=vars data-name=shown.to_owned()>{inner}</span>}
                .into_any()
        }
        None => inner,
    }
}
fn role_icon_view(g: Id, r: &Role, icon: &RoleIcon, class: &'static str) -> AnyView {
    let label = r.name.clone();
    if let Some(rev) = icon.rev {
        return view!{<img class=class src=format!("/api/v1/guilds/{}/roles/{}/icon?v={rev}",g.0,r.id.0) alt=label.clone() title=label width="16" height="16" loading="lazy" decoding="async"/>}.into_any();
    }
    match icon.emoji_id {
        // The still preview: nothing moves in a list.
        Some(e) => view!{<img class=class src=format!("/api/v1/emoji/{}/preview",e.0) alt=label.clone() title=label width="16" height="16" loading="lazy" decoding="async"/>}.into_any(),
        None => view!{<span class=format!("{class} role-icon-emoji") role="img" aria-label=label.clone() title=label>{icon.emoji.clone()}</span>}.into_any(),
    }
}
/// The icon of the member's highest role that has one, after their name.
pub(super) fn role_icon(a: &App, user: Id) -> AnyView {
    let Some(g) = a.guild else {
        return ().into_any();
    };
    match top_role(a, user, |r| role_icon_of(r).is_some())
        .and_then(|r| role_icon_of(r).map(|i| (r, i)))
    {
        Some((r, i)) => role_icon_view(g, r, &i, "role-icon"),
        None => ().into_any(),
    }
}
/// What follows a name in messages: the APP tag, role icon, server tag.
pub(super) fn after_name(a: &App, u: &UserSummary) -> AnyView {
    view! {{super::apps::tag(u)}{role_icon(a,u.id)}{tag(a,u)}}.into_any()
}
/// What follows a name in a member row (itself a button): role icon, tag.
pub(super) fn row_after(a: &App, u: &UserSummary) -> AnyView {
    view! {{role_icon(a,u.id)}{tag_static(a,u)}}.into_any()
}

fn crop_vars(c: Option<Crop>) -> (&'static str, String) {
    match c {
        Some(c) => (
            " id-cropped",
            format!(
                "--crop-x:{}%;--crop-y:{}%;--crop-z:{}",
                c.x,
                c.y,
                f32::from(c.zoom) / 100.0
            ),
        ),
        None => ("", String::new()),
    }
}
/// A server image: the still (or still rendition of an animation, which then
/// plays while `.id-play` is hovered or focused, never with reduced motion).
fn picture(m: &ImageMeta, still: String, moving: String, class: &str) -> AnyView {
    let (crop_class, crop) = crop_vars(m.crop);
    let anim = m.animated.then(|| view!{<span class="id-anim" style=format!("--anim:url({moving});{crop}") aria-hidden="true"></span>});
    view! {<img class=format!("{class}{crop_class}") style=crop.clone() src=still alt="" decoding="async"/>{anim}}.into_any()
}
fn file_picture(m: &ImageMeta, class: &str) -> AnyView {
    let still = if m.animated {
        format!("/api/v1/files/{}/thumbnail", m.id.0)
    } else {
        format!("/api/v1/files/{}", m.id.0)
    };
    picture(m, still, format!("/api/v1/files/{}", m.id.0), class)
}
fn open_banner(a: &App) -> Option<ImageMeta> {
    if a.nav.favorites {
        return None;
    }
    a.guild
        .and_then(|g| a.model.guilds.get(&g))
        .and_then(|g| guild_image(&g.guild, "banner"))
}
/// The channel sidebar over a banner: its header floats over the picture.
pub(super) fn sidebar_class(a: &App) -> &'static str {
    if open_banner(a).is_some() {
        " has-banner"
    } else {
        ""
    }
}
/// The banner at the top of the channel list. It scrolls away with the
/// channels (no script, no layout change); the header above it fades to the
/// sidebar color as it goes (a scroll-driven opacity, where supported).
pub(super) fn sidebar_banner(a: &App) -> AnyView {
    match open_banner(a) {
        Some(m) => view!{<div class="guild-banner id-play" aria-hidden="true">{file_picture(&m,"guild-banner-img")}<span class="guild-banner-scrim"></span></div>}.into_any(),
        None => ().into_any(),
    }
}
/// The rail's server icon when it moves or is cropped: the still first frame,
/// animating only while the button is hovered or focused.
pub(super) fn rail_face(g: &GuildSummary, class: &str) -> Option<AnyView> {
    let m = guild_image(&g.guild, "icon").filter(|m| Some(m.id) == g.guild.icon_id)?;
    // Clipped to the button: a zoomed crop must not spill out of the rail.
    Some(
        view! {<span class="absolute inset-0 overflow-hidden rounded-[inherit]">{picture(
            &m,
            format!("/api/v1/files/{}/thumbnail", m.id.0),
            format!("/api/v1/files/{}", m.id.0),
            class,
        )}</span>}
        .into_any(),
    )
}

/// Codes of invites to this server in a message: `<origin>/?invite=CODE` or
/// `<origin>/invite/CODE`, at most three.
fn invite_codes(origin: &str, text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    if origin.is_empty() {
        return out;
    }
    let base = origin.trim_end_matches('/');
    let needles = [format!("{base}/?invite="), format!("{base}/invite/")];
    // In the order the message has them: always the nearer of the two forms.
    let mut at = 0;
    while out.len() < 3 {
        let Some((i, n)) = needles
            .iter()
            .filter_map(|n| text[at..].find(n.as_str()).map(|i| (i, n.len())))
            .min()
        else {
            break;
        };
        at += i + n;
        let code: String = text[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if (3..=32).contains(&code.len()) && !out.contains(&code) {
            out.push(code);
        }
    }
    out
}
/// An invite code as text. Every place here that shows one uses this, so
/// Streamer Mode hides them all (`.streamer-invite`, themes.css).
fn invite_code(code: &str) -> AnyView {
    super::invite_text(code, "invite-code")
}
fn counts(c: &InviteCard) -> String {
    format!(
        "{} Online · {} Member{}",
        c.online_count,
        c.member_count,
        if c.member_count == 1 { "" } else { "s" }
    )
}
fn art(code: &str, slot: &str, m: &ImageMeta, class: &str) -> AnyView {
    let base = format!("/api/v1/invites/{code}/art/{slot}");
    let still = if m.animated {
        format!("{base}?still=1")
    } else {
        base.clone()
    };
    picture(m, still, base, class)
}
fn server_face(code: &str, c: &InviteCard, class: &'static str) -> AnyView {
    match &c.icon {
        Some(m) => view!{<span class=format!("{class} id-play relative overflow-hidden")>{art(code,"icon",m,"size-full object-cover")}</span>}.into_any(),
        None => view!{<span class=format!("{class} grid place-items-center bg-muted font-semibold")>{c.name.chars().take(2).collect::<String>()}</span>}.into_any(),
    }
}
fn join_button(c: &InviteCard) -> AnyView {
    if c.joined {
        view!{<button type="button" class="button secondary" data-live=format!("guild:{}",c.guild_id.0)>"Joined"</button>}.into_any()
    } else if c.apply {
        view!{<button type="button" class="button primary" data-live=format!("id:apply,{}",c.code)>"Apply"</button>}.into_any()
    } else {
        view!{<button type="button" class="button primary" data-live=format!("id:join,{}",c.code)>"Join"</button>}.into_any()
    }
}
/// One invite card in a message. Its size is fixed before it loads, so the
/// message list never moves when the card arrives.
fn invite_card(a: &App, code: &str) -> AnyView {
    let cached = a.identity.invites.borrow().get(code).cloned();
    match cached {
        None => {
            crate::live::identity::want_invite(a, code);
            invite_placeholder()
        }
        Some(None) => view!{<div class="invite-card is-invalid"><div class="invite-card-body"><span class="invite-card-icon grid place-items-center bg-muted">{icon("link")}</span><div class="min-w-0"><strong class="block">"Invite unavailable"</strong><small class="text-subtle">"This invite has expired or is not valid."</small></div></div></div>}.into_any(),
        Some(Some(c)) => {
            let banner = c.banner.as_ref().map(|m| art(code, "banner", m, "size-full object-cover"));
            view!{<div class="invite-card id-play" aria-label=format!("Invite to {}",c.name)>
                <div class="invite-card-banner">{banner}</div>
                <div class="invite-card-body">{server_face(code,&c,"invite-card-icon")}
                    <div class="min-w-0 flex-1"><div class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1"><strong class="min-w-0 wrap-anywhere">{c.name.clone()}</strong>{c.tag.as_ref().map(chip)}</div><small class="block text-subtle">{counts(&c)}</small>{(!c.description.is_empty()).then(||view!{<p class="invite-card-description">{c.description.clone()}</p>})}</div>
                    {join_button(&c)}</div>
            </div>}.into_any()
        }
    }
}
pub(super) fn invite_cards(a: &App, m: &Message) -> AnyView {
    if !m.content.contains("invite") {
        return ().into_any();
    }
    let codes = invite_codes(&a.origin, &m.content);
    if codes.is_empty() {
        return ().into_any();
    }
    // The card renderer is a split module; until it arrives each card is a
    // placeholder of the same size.
    let render = crate::live::identity::invite_view(a);
    view! {<div class="invite-cards mt-2 grid gap-2">{codes.iter().map(|c|match render{Some(r)=>r(a,c),None=>invite_placeholder()}).collect_view()}</div>}.into_any()
}
fn invite_placeholder() -> AnyView {
    view!{<div class="invite-card is-loading" role="status" aria-label="Loading invite"><div class="invite-card-banner"></div><div class="invite-card-body"><span class="invite-card-icon bg-muted"></span></div></div>}.into_any()
}
#[lazy]
pub(in crate::live) fn load_invite_view() -> fn(&App, &str) -> AnyView {
    invite_card
}

// ----------------------------------------------------- dialogs (lazy)

/// The banner over a directory preview's color band.
pub(super) fn directory_banner(a: &App, g: Id) -> AnyView {
    match a
        .identity
        .directory
        .as_ref()
        .filter(|(id, _)| *id == g)
        .and_then(|(_, art)| art.banner.as_ref())
    {
        Some(m) => view!{<span class="id-play absolute inset-0 block overflow-hidden rounded-t-lg">{file_picture(m,"size-full object-cover")}</span>}.into_any(),
        None => ().into_any(),
    }
}
/// The join screen behind an invite: its background, scrim and card.
pub(super) fn join_card(a: &App) -> AnyView {
    let code = a.invite_wanted.clone();
    if code.is_empty() || !matches!(&a.invite_preview, Some((c, Ok(_))) if *c == code) {
        return ().into_any();
    }
    let cached = a.identity.invites.borrow().get(&code).cloned();
    let Some(c) = cached else {
        crate::live::identity::want_invite(a, &code);
        return view! {<div class="invite-hero is-loading" role="status" aria-label="Loading invite"></div>}.into_any();
    };
    let Some(c) = c else { return ().into_any() };
    let background = c.background.as_ref().or(c.banner.as_ref()).map(|m| {
        let slot = if c.background.is_some() {
            "background"
        } else {
            "banner"
        };
        art(&code, slot, m, "invite-hero-img")
    });
    view!{<div class="invite-hero id-play" aria-label=format!("Invite to {}",c.name)>{background}<span class="invite-hero-scrim"></span>
        <div class="invite-hero-content">{server_face(&code,&c,"invite-hero-icon")}<div class="min-w-0"><div class="flex flex-wrap items-center gap-2"><strong class="text-lg wrap-anywhere">{c.name.clone()}</strong>{c.tag.as_ref().map(chip)}</div><small class="block">{counts(&c)}</small>{(!c.description.is_empty()).then(||view!{<p class="mt-1 text-sm wrap-anywhere">{c.description.clone()}</p>})}</div></div>
    </div>}.into_any()
}

/// A member's roles on their profile card, as pills (with styles and icons).
pub(super) fn role_pills(a: &App, guild: Option<Id>, user: Id) -> AnyView {
    let Some(g) = guild.filter(|g| a.guild == Some(*g)) else {
        return ().into_any();
    };
    let Some(m) = a.model.members.get(&user).filter(|m| m.guild_id == g) else {
        return ().into_any();
    };
    let mut roles: Vec<&Role> = m
        .role_ids
        .iter()
        .filter(|id| **id != g)
        .filter_map(|id| a.roles.get(id))
        .collect();
    if roles.is_empty() {
        return ().into_any();
    }
    roles.sort_by_key(|r| std::cmp::Reverse(r.position));
    view!{<section class="profile-section mt-3 rounded-lg p-3 text-sm"><h3 class="category mb-1">"Roles"</h3><ul class="flex flex-wrap gap-1" aria-label="Roles">{roles.into_iter().map(|r|view!{<li>{pill(g,r)}</li>}).collect_view()}</ul></section>}.into_any()
}
fn pill(g: Id, r: &Role) -> AnyView {
    let style = role_style(r);
    let (class, vars) = match &style {
        Some(s) => {
            let (c, v) = paint(r, s);
            (c, v)
        }
        None => ("", String::new()),
    };
    let dot = if r.color == 0 { 0xA8A8B0 } else { r.color };
    view!{<span class="role-pill"><span class=format!("role-pill-dot {}",if style.is_some(){format!("painted {class}")}else{String::new()}) style=format!("background-color:{};{vars}",hex(dot))></span>{role_icon_of(r).map(|i|role_icon_view(g,r,&i,"role-icon"))}<span class=class style=vars.clone() data-name=r.name.clone()><span>{r.name.clone()}</span></span></span>}.into_any()
}

/// The card behind a click on a tag.
fn tag_card(a: &App) -> AnyView {
    let Some((g, card)) = &a.identity.tag_card else {
        return view! {<p role="status">"Loading…"</p>}.into_any();
    };
    let Some(c) = card else {
        return view! {<p role="status">"Loading…"</p>}.into_any();
    };
    let icon = match c.icon_id {
        Some(i) => view!{<img class="size-16 rounded-2xl object-cover" src=format!("/api/v1/files/{}/thumbnail",i.0) alt=""/>}.into_any(),
        None => view!{<span class="grid size-16 place-items-center rounded-2xl bg-muted text-xl font-semibold">{c.name.chars().take(2).collect::<String>()}</span>}.into_any(),
    };
    let action = match &c.join {
        TagJoin::Member => {
            let wearing = a
                .model
                .me
                .as_ref()
                .and_then(|m| worn_tag(&m.user))
                .is_some_and(|t| t.guild_id == *g);
            view!{<div class="flex flex-wrap gap-2"><button type="button" class="button secondary" data-live=format!("guild:{}",g.0)>"Open Server"</button>{(!wearing).then(||view!{<button type="button" class="button primary" data-live=format!("id:wear,{}",g.0)>"Wear This Tag"</button>})}</div>}.into_any()
        }
        TagJoin::Invite { code, apply: false } => view!{<button type="button" class="button primary" data-live=format!("id:join,{code}")>"Join Server"</button>}.into_any(),
        TagJoin::Invite { code, apply: true } => view!{<button type="button" class="button primary" data-live=format!("id:apply,{code}")>"Apply to Join"</button>}.into_any(),
        TagJoin::Directory { apply } => view!{<button type="button" class="button primary" data-live=format!("id:discover,{}",g.0)>{if *apply{"Apply to Join"}else{"Join Server"}}</button>}.into_any(),
        TagJoin::None => ().into_any(),
    };
    view!{<div class="tag-card grid gap-3">
        <div class="flex items-center gap-3">{icon}<div class="min-w-0"><h2 class="m-0 wrap-anywhere">{c.name.clone()}</h2><p class="m-0 text-sm text-subtle">{format!("{} member{}",c.member_count,if c.member_count==1{""}else{"s"})}</p></div></div>
        <div class="flex items-center gap-2"><span class="server-tag large">{badge(&c.tag,"tag-badge")}<span class="tag-text">{c.tag.text.clone()}</span></span><span class="text-sm text-subtle">"Server tag"</span></div>
        {(!c.description.is_empty()).then(||view!{<p class="m-0 text-sm wrap-anywhere">{c.description.clone()}</p>})}
        {action}
    </div>}.into_any()
}

/// The person's tag in the profile editor: one per server they are in that
/// has one, or none. Saved at once (it is not part of the profile form).
pub(super) fn tag_picker(a: &App) -> AnyView {
    let Some(mine) = &a.identity.mine else {
        return ().into_any();
    };
    let row = |label: AnyView, action: String, on: bool| {
        view!{<button type="button" class=format!("tag-choice flex min-h-11 w-full items-center gap-3 rounded-lg border px-3 py-2 text-left {}",if on{"border-primary bg-secondary"}else{"hover:bg-accent"}) aria-pressed=on.to_string() data-live=action>{label}</button>}.into_any()
    };
    let name = |g: Id| {
        a.model
            .guilds
            .get(&g)
            .map_or_else(|| "Server".to_owned(), |g| g.guild.name.clone())
    };
    view!{<section class="tag-picker col-start-1 mt-8" aria-labelledby="tag-picker-title"><h3 id="tag-picker-title" class="category mb-1">"Server Tag"</h3>
        <p class="muted-copy mb-3">"Wear one of your servers’ tags next to your name everywhere."</p>
        {if mine.available.is_empty(){view!{<p class="text-sm text-subtle">"None of your servers has a tag yet."</p>}.into_any()}else{view!{<div class="grid gap-2">
            {row(view!{<span class="text-sm">"No tag"</span>}.into_any(),"id:wear,0".into(),mine.wearing.is_none())}
            {mine.available.iter().map(|t|row(view!{<span class="server-tag large">{badge(t,"tag-badge")}<span class="tag-text">{t.text.clone()}</span></span><span class="min-w-0 truncate text-sm">{name(t.guild_id)}</span>}.into_any(),format!("id:wear,{}",t.guild_id.0),mine.wearing==Some(t.guild_id))).collect_view()}
        </div>}.into_any()}}
    </section>}.into_any()
}

/// Style and icon controls under the role editor.
fn role_controls(a: &App, r: &Role) -> AnyView {
    if r.id == r.guild_id {
        return ().into_any();
    }
    let allowed = settings::can(a, Permissions::MANAGE_ROLES);
    let look = RoleLook {
        style: role_style(r),
        icon: role_icon_of(r),
    };
    let style = look.style;
    let paint_now = match style.map(|s| s.paint) {
        Some(RolePaint::Gradient) => "gradient",
        Some(RolePaint::Holographic) => "holographic",
        None => "solid",
    };
    let c2 = style.and_then(|s| s.color2).unwrap_or(0xFFC0E0);
    let c3 = style.and_then(|s| s.color3);
    let g = r.guild_id;
    let emoji = a.model.launch.emoji.get(&g).cloned().unwrap_or_default();
    let current_emoji = match &look.icon {
        Some(i) if i.rev.is_none() && i.emoji_id.is_none() => i.emoji.clone(),
        _ => String::new(),
    };
    view!{<fieldset class="role-identity mt-8" disabled=!allowed>
        <div class="settings-divider"></div>
        <h2>"Role Style"</h2>
        <p class="muted-copy">"Names with this role use its color, a gradient or a holographic style. A member’s own name style is shown instead of the role’s."</p>
        <div class="my-4 flex flex-wrap items-center gap-3 rounded-lg bg-background p-3" aria-label="Preview"><span class="text-[15px] font-semibold" style=format!("color:{}",hex(crate::wire::legible_role_color(if r.color==0{0xE8EAED}else{r.color})))>{name_preview(r)}</span>{pill(g,r)}</div>
        <div class="flex flex-wrap items-end gap-4">
            <label class="field mb-0"><span>"Style"</span><select id="id-role-paint"><option value="solid" selected={paint_now=="solid"}>"Solid color"</option><option value="gradient" selected={paint_now=="gradient"}>"Gradient"</option><option value="holographic" selected={paint_now=="holographic"}>"Holographic"</option></select></label>
            <label class="field mb-0"><span>"Second color"</span><input id="id-role-c2" type="color" value=hex(c2)/></label>
            <label class="live-checkbox mb-0"><input id="id-role-c3-on" type="checkbox" checked=c3.is_some()/>"Third color"</label>
            <label class="field mb-0"><span>"Third color"</span><input id="id-role-c3" type="color" value=hex(c3.unwrap_or(0xF2B84B))/></label>
        </div>
        <p class="mt-2 text-xs text-subtle">"A gradient starts at the role color above. Colors too dark to read on the chat background are refused."</p>
        <div class="mt-3">{button("Save Style",&format!("id:role-style,{}",r.id.0),"secondary")}</div>
        <div class="settings-divider"></div>
        <h2>"Role Icon"</h2>
        <p class="muted-copy">"Shown after the names of members whose highest role with an icon is this one."</p>
        <div class="my-3 flex items-center gap-3"><span class="grid size-10 place-items-center rounded-lg bg-background">{look.icon.as_ref().map_or_else(||view!{<span class="text-xs text-subtle">"None"</span>}.into_any(),|i|role_icon_view(g,r,i,"role-icon large"))}</span>
            {look.icon.is_some().then(||button("Remove Icon",&format!("id:role-icon,{},clear",r.id.0),"danger-text"))}</div>
        <div class="flex flex-wrap items-end gap-3">
            <label class="field mb-0"><span>"Emoji"</span><input id="id-role-emoji" type="text" maxlength="16" value=current_emoji placeholder="⭐" class="w-24"/></label>
            {button("Use Emoji",&format!("id:role-icon,{},emoji",r.id.0),"secondary")}
            <button type="button" class="button secondary" data-pick="id-role-icon-file">"Upload Picture"</button>
            <input id="id-role-icon-file" type="file" accept="image/png,image/webp" hidden data-identity-role=r.id.0.to_string()/>
        </div>
        <p class="mt-2 text-xs text-subtle">"PNG or WebP, at most 256 KB and 1024 × 1024. It is shown at 16 px."</p>
        {(!emoji.is_empty()).then(||view!{<div class="mt-3"><span class="text-[13px] font-medium text-subtle">"Server emoji"</span><div class="mt-1 flex max-h-40 flex-wrap gap-1 overflow-y-auto">{emoji.iter().take(250).map(|e|view!{<button type="button" class="grid size-9 place-items-center rounded-md hover:bg-accent" aria-label=format!("Use :{}:",e.name) title=format!(":{}:",e.name) data-live=format!("id:role-icon,{},custom,{}",r.id.0,e.id.0)><img class="size-6" src=format!("/api/v1/emoji/{}/preview",e.id.0) alt="" loading="lazy"/></button>}).collect_view()}</div></div>})}
    </fieldset>}.into_any()
}
fn name_preview(r: &Role) -> AnyView {
    match role_style(r) {
        Some(s) => {
            let (class, vars) = paint(r, &s);
            view!{<span class=class style=vars data-name="Member name"><span>"Member name"</span></span>}.into_any()
        }
        None => view! {<span>"Member name"</span>}.into_any(),
    }
}

// ----------------------------------------------------------- settings pages

pub(super) fn is_server_page(page: &str) -> bool {
    admin_page(page)
}
pub(super) fn nav(a: &App) -> Vec<(&'static str, &'static str, &'static str, bool)> {
    let manage = settings::can(a, Permissions::MANAGE_GUILD);
    vec![
        ("tag", "Server Tag", "id-tags", manage),
        ("image", "Appearance", "id-appearance", manage),
        ("link", "Vanity URL", "id-vanity", manage),
    ]
}
pub(super) fn nav_group(a: &App) -> AnyView {
    let rows = nav(a);
    if !rows.iter().any(|r| r.3) {
        return ().into_any();
    }
    view!{<h3 class="category mx-2 mt-6 mb-2">"Identity"</h3>{rows.into_iter().filter(|r|r.3).map(|(glyph,label,page,_)|settings_nav_row(glyph,label,&format!("dialog:{page}"),a.dialog==page)).collect_view()}}.into_any()
}
fn loading(title: &str) -> AnyView {
    view! {<h2>{title.to_owned()}</h2><p class="muted-copy" role="status">"Loading…"</p>}.into_any()
}
fn admin(a: &App) -> Option<&IdentityAdmin> {
    a.identity
        .admin
        .as_ref()
        .filter(|s| Some(s.guild_id) == a.guild)
}

fn tags_page(a: &App) -> AnyView {
    let Some(s) = admin(a) else {
        return loading("Server Tag");
    };
    let t = s.tag.clone();
    let text = t.as_ref().map_or(String::new(), |t| t.text.clone());
    let chosen = t.as_ref().map_or("leaf", |t| glyph(&t.glyph)).to_owned();
    let c1 = t.as_ref().map_or(0x1C3A2E, |t| t.color);
    let c2 = t.as_ref().map_or(0x3DDC97, |t| t.color2);
    let invite = t
        .as_ref()
        .and_then(|t| t.invite_code.clone())
        .unwrap_or_default();
    let hide = a
        .guild
        .and_then(|g| a.model.guilds.get(&g))
        .is_some_and(|g| hides_tags(&g.guild));
    let me = a
        .model
        .me
        .as_ref()
        .map(|m| m.user.clone())
        .unwrap_or_default();
    let sample = WornTag {
        guild_id: s.guild_id,
        text: if text.is_empty() {
            "TAG".into()
        } else {
            text.clone()
        },
        glyph: chosen.clone(),
        color: c1,
        color2: c2,
    };
    view!{<h2>"Server Tag"</h2>
        <p class="muted-copy mb-4">"Members can wear your server’s tag next to their name everywhere on StickTalk, in other servers and in direct messages too."</p>
        <div class="tag-preview mb-6 flex items-center gap-3 rounded-lg bg-background p-3" aria-label="Preview">{avatar(&me,"small")}<strong class="text-[15px]">{name(&me)}</strong><span class="server-tag" data-tag-preview>{badge(&sample,"tag-badge")}<span class="tag-text">{sample.text.clone()}</span></span></div>
        <form data-live-form="id-tag" data-identity-tag>
            <div class="field"><label for="id-tag-text">"Tag"</label><input id="id-tag-text" type="text" maxlength="4" value=text.clone() autocomplete="off" spellcheck="false" aria-describedby="id-tag-help"/><small id="id-tag-help">"1 to 4 letters, numbers or ! # $ % & * + - = ? @ ^ _ ~. Each tag is unique on this StickTalk server."</small></div>
            <fieldset class="my-4"><legend class="text-[13px] font-medium text-subtle">"Badge"</legend><input id="id-tag-glyph" type="hidden" value=chosen.clone()/>
                <div class="glyph-grid mt-2 grid grid-cols-[repeat(auto-fill,minmax(44px,1fr))] gap-2">{TAG_GLYPHS.iter().map(|(id,label)|view!{<button type="button" class=format!("glyph-choice grid size-11 place-items-center rounded-lg border {}",if *id==chosen.as_str(){"border-primary bg-secondary"}else{"hover:bg-accent"}) aria-label=*label title=*label aria-pressed=(*id==chosen.as_str()).to_string() data-identity-glyph=*id>{badge(&WornTag{glyph:(*id).into(),..sample.clone()},"tag-badge large")}</button>}).collect_view()}</div></fieldset>
            <div class="flex flex-wrap gap-4"><label class="field mb-0"><span>"Badge color"</span><input id="id-tag-c1" type="color" value=hex(c1)/></label><label class="field mb-0"><span>"Glyph color"</span><input id="id-tag-c2" type="color" value=hex(c2)/></label></div>
            <p class="form-error mt-2 text-sm text-destructive" data-identity-contrast hidden>"Those two colors are too close: the badge would not show."</p>
            <label class="field mt-4"><span>"Joining from the tag"</span><select id="id-tag-invite">
                <option value="" selected={invite.is_empty()}>"Only when the server is in Discover"</option>
                {s.vanity.as_ref().map(|v|view!{<option value=v.code.clone() selected={invite==v.code}>{format!("Vanity link /invite/{}",v.code)}</option>})}
                {s.invites.iter().map(|c|view!{<option value=c.clone() selected={invite==*c}>{format!("Invite ending …{}",c.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>())}</option>}).collect_view()}
            </select><small>"Anyone who clicks the tag sees your server’s name, icon, member count and description, and a Join button when you choose an invite."</small></label>
            <div class="mt-4 flex flex-wrap gap-2"><button type="submit" class="button primary">{if t.is_some(){"Save Tag"}else{"Create Tag"}}</button>{t.is_some().then(||button("Remove Tag","submit:id-tag-remove","danger-text"))}</div>
            {saved_note(a,"id-tag")}
        </form>
        {t.is_some().then(||view!{<p class="mt-4 text-sm text-subtle">{format!("{} member{} wear{} this tag.",s.wearers,if s.wearers==1{""}else{"s"},if s.wearers==1{"s"}else{""})}</p>})}
        <div class="settings-divider"></div>
        <label class="live-checkbox" data-live-change="id:hide"><span class="flex flex-col"><span>"Hide other servers’ tags"</span><small class="text-xs text-subtle">"Tags from other servers are not shown next to names in this server."</small></span><input id="id-hide-tags" type="checkbox" checked=hide/></label>
    }.into_any()
}

fn image_row(
    a: &App,
    title: &str,
    help: &str,
    slot: &str,
    current: Option<&ImageMeta>,
    preview: AnyView,
) -> AnyView {
    let input = format!("id-pick-{slot}");
    view!{<section class="identity-image mb-8"><h3 class="mb-1 text-base font-semibold">{title.to_owned()}</h3><p class="muted-copy mb-3">{help.to_owned()}</p>
        {preview}
        <div class="mt-3 flex flex-wrap gap-2"><button type="button" class="button secondary" data-pick=input.clone()>{if current.is_some(){"Change"}else{"Upload"}}</button>
            <input id=input type="file" accept="image/png,image/apng,image/jpeg,image/webp,image/gif" hidden data-identity-slot=slot.to_owned()/>
            {current.is_some().then(||button("Remove",&format!("id:remove,{slot}"),"danger-text"))}</div>
        {if a.uploading{view!{<p class="mt-2 text-sm text-subtle" role="status">{a.upload_progress.clone()}</p>}.into_any()}else{().into_any()}}
    </section>}.into_any()
}
fn appearance_page(a: &App) -> AnyView {
    let Some(s) = admin(a) else {
        return loading("Appearance");
    };
    let Some(g) = a.guild.and_then(|g| a.model.guilds.get(&g)) else {
        return ().into_any();
    };
    let icon = s
        .look
        .icon
        .clone()
        .filter(|m| Some(m.id) == g.guild.icon_id)
        .or(g.guild.icon_id.map(|id| ImageMeta {
            id,
            animated: false,
            crop: None,
        }));
    let banner_preview = view!{<div class="guild-banner preview id-play" aria-label="Banner preview">{s.look.banner.as_ref().map(|m|file_picture(m,"guild-banner-img"))}<span class="guild-banner-scrim"></span><strong class="guild-banner-name">{g.guild.name.clone()}</strong></div>}.into_any();
    let icon_preview = view!{<div class="flex items-center gap-4"><span class="id-play relative grid size-20 place-items-center overflow-hidden rounded-2xl bg-muted text-xl font-semibold" aria-label="Icon preview">{icon.as_ref().map_or_else(||view!{<span>{g.guild.name.chars().take(2).collect::<String>()}</span>}.into_any(),|m|picture(m,format!("/api/v1/files/{}/thumbnail",m.id.0),format!("/api/v1/files/{}",m.id.0),"size-full object-cover"))}</span><small class="text-subtle">{if icon.as_ref().is_some_and(|m|m.animated){"Animated: it plays when the icon is hovered or focused."}else{"Still or animated (GIF, animated WebP or APNG)."}}</small></div>}.into_any();
    let background_preview = view!{<div class="invite-hero preview id-play" aria-label="Invite background preview">{s.invite_background.as_ref().map(|m|file_picture(m,"invite-hero-img"))}<span class="invite-hero-scrim"></span><div class="invite-hero-content"><span class="invite-hero-icon grid place-items-center bg-muted font-semibold">{g.guild.name.chars().take(2).collect::<String>()}</span><div><strong class="text-lg">{g.guild.name.clone()}</strong><small class="block">"You’ve been invited"</small></div></div></div>}.into_any();
    view!{<h2>"Appearance"</h2>
        <p class="muted-copy mb-6">"Pictures can be still or animated. You can move and zoom each one before it is saved; animations stay animated and only play when hovered."</p>
        {image_row(a,"Banner","Shown at the top of the channel list, under the server name, and on invites and Discover. 5:2.","banner",s.look.banner.as_ref(),banner_preview)}
        {image_row(a,"Server Icon","Shown in the server list. Animated icons play when hovered or focused.","icon",icon.as_ref(),icon_preview)}
        {image_row(a,"Invite Background","Shown behind the invite when someone opens your invite link. 16:9.","background",s.invite_background.as_ref(),background_preview)}
    }.into_any()
}
fn vanity_page(a: &App) -> AnyView {
    let Some(s) = admin(a) else {
        return loading("Vanity URL");
    };
    let origin = a.origin.trim_end_matches('/').to_owned();
    let wait = (s.vanity_wait_ms > 0).then(|| {
        let minutes = s.vanity_wait_ms.div_ceil(60_000);
        format!(
            "You can change the link again in {minutes} minute{}.",
            if minutes == 1 { "" } else { "s" }
        )
    });
    view!{<h2>"Vanity URL"</h2>
        <p class="muted-copy mb-4">"A short, permanent invite link for your server. Joining with it follows your server’s rules screening, applications and safety settings, like any invite."</p>
        {s.vanity.as_ref().map(|v|{let link=format!("{origin}/invite/{}",v.code);view!{<section class="mb-6 rounded-lg border bg-card p-4"><h3 class="category mb-2">"Current link"</h3><div class="flex flex-wrap items-center gap-3"><span class="min-w-0 flex-1 wrap-anywhere">{format!("{origin}/invite/")}{invite_code(&v.code)}</span><button type="button" class="button secondary" data-copy=link>"Copy Link"</button></div><p class="mt-2 mb-0 text-sm text-subtle">{format!("{} use{} · claimed {}",v.uses,if v.uses==1{""}else{"s"},date(v.claimed_at))}</p></section>}})}
        <form data-live-form="id-vanity">
            <div class="field"><label for="id-vanity-code">{if s.vanity.is_some(){"Change link"}else{"Claim a link"}}</label><div class="flex items-center gap-2"><span class="text-sm text-subtle whitespace-nowrap phone:hidden">{format!("{origin}/invite/")}</span><input id="id-vanity-code" type="text" minlength="3" maxlength="32" value=s.vanity.as_ref().map_or(String::new(),|v|v.code.clone()) autocomplete="off" spellcheck="false" aria-describedby="id-vanity-help"/></div><small id="id-vanity-help">"3 to 32 letters, numbers or hyphens. Links are unique on this StickTalk server; a released link is held for 30 days before anyone else can claim it."</small></div>
            {wait.map(|w|view!{<p class="text-sm text-warning" role="status">{w}</p>})}
            <div class="mt-2 flex flex-wrap gap-2"><button type="submit" class="button primary" disabled={s.vanity_wait_ms>0}>"Save"</button>{s.vanity.is_some().then(||button("Release Link","id:vanity-release","danger-text"))}</div>
            {saved_note(a,"id-vanity")}
        </form>
    }.into_any()
}
fn date(ms: u64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ms as f64));
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{} {}, {}",
        MONTHS[d.get_month() as usize % 12],
        d.get_date(),
        d.get_full_year()
    )
}

/// Every dialog this module renders, with its frame.
fn dialog_view(a: &App) -> Option<AnyView> {
    let d = a.dialog.as_str();
    Some(match d {
        "id-tags" => settings::server_layout(a, tags_page(a)),
        "id-appearance" => settings::server_layout(a, appearance_page(a)),
        "id-vanity" => settings::server_layout(a, vanity_page(a)),
        _ if d.starts_with("id-tag:") => modal(a, "tag-card-dialog", "Server tag", tag_card(a)),
        _ => return None,
    })
}

/// This feature's view module: the settings pages, the tag card and the
/// role editor's style and icon controls (not in the shared dialog module).
#[derive(Clone, Copy)]
pub(in crate::live) struct Views {
    dialog: fn(&App) -> Option<AnyView>,
    role_controls: fn(&App, &Role) -> AnyView,
}
/// The table `ui::load_identity_dialog` (the split point) returns.
pub(super) fn table() -> Views {
    Views {
        dialog: dialog_view,
        role_controls,
    }
}
fn views(a: &App) -> Option<Views> {
    lazy_view(
        a,
        |a| &a.views.identity,
        || Box::pin(super::load_identity_dialog()),
    )
}
/// A dialog of this feature, or its frame with "Loading…" while its view
/// module downloads.
pub(super) fn dialog(a: &App) -> Option<AnyView> {
    let d = a.dialog.as_str();
    if !crate::live::identity::has_view(d) {
        return None;
    }
    Some(match views(a).and_then(|v| (v.dialog)(a)) {
        Some(view) => view,
        None if admin_page(d) => settings::server_layout(a, loading("Server Identity")),
        None => loading_dialog(a),
    })
}
/// The role editor's style and icon controls (from the view module).
pub(super) fn role_editor(a: &App, r: &Role) -> AnyView {
    if r.id == r.guild_id {
        return ().into_any();
    }
    match views(a) {
        Some(v) => (v.role_controls)(a, r),
        None => view! {<p class="muted-copy mt-8" role="status">"Loading…"</p>}.into_any(),
    }
}

#[cfg(test)]
mod tests {
    use super::invite_codes;

    #[test]
    fn invite_links_to_this_server_only_are_found() {
        let o = "https://chat.example";
        assert_eq!(
            invite_codes(o, "join https://chat.example/?invite=abcDEF_123-x and https://chat.example/invite/sticks!"),
            vec!["abcDEF_123-x".to_owned(), "sticks".to_owned()]
        );
        // In the order the message has them, whatever the link form.
        assert_eq!(
            invite_codes(
                o,
                "https://chat.example/invite/first then https://chat.example/?invite=second"
            ),
            vec!["first".to_owned(), "second".to_owned()]
        );
        assert!(invite_codes(o, "https://evil.example/invite/sticks").is_empty());
        assert!(invite_codes(o, "https://chat.example/invite/ab").is_empty());
        let many = (0..5)
            .map(|i| format!("https://chat.example/invite/code{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(invite_codes(o, &many).len(), 3);
    }
}
