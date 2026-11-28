//! Profile views: the one avatar, name and nameplate renderers every list
//! uses, the profile card (popout and editor preview), and the profile
//! editor (user profile, server profiles, badges). Styles are
//! `styles/profiles.css`; cropping and the live preview are
//! `public/profiles.js`. State and commands are `live/profiles.rs`.
use super::*;
use crate::live::profiles::{hex, Tab};
use std::cell::Cell;

thread_local! {
    /// The signed-in user, so their own cosmetics can be told apart: "Show
    /// plain names" and "Reduce cosmetics" never hide your own.
    static ME: Cell<u64> = const { Cell::new(0) };
}
/// Called once per render, before any avatar or name is drawn.
pub(super) fn begin(a: &App) {
    ME.set(a.model.me.as_ref().map_or(0, |m| m.user.id.0));
}
fn own(user: Id) -> bool {
    user.0 != 0 && ME.get() == user.0
}
fn self_class(user: Id) -> &'static str {
    if own(user) {
        " is-self"
    } else {
        ""
    }
}
/// CSS for a crop: object-fit/background cover, focal point and zoom.
fn crop_style(c: Option<Crop>) -> (&'static str, String) {
    match c {
        Some(c) => (
            " cropped",
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
fn known_or_none(catalog: &[Cosmetic], v: Option<&str>) -> Option<&'static str> {
    v.and_then(|v| catalog.iter().find(|(id, _)| *id == v).map(|(id, _)| *id))
}

/// The user as the open server shows them: server nickname, avatar, name
/// style, decoration and nameplate replace the global ones where set.
pub(super) fn effective(a: &App, user: Id) -> UserSummary {
    let mut u = a
        .model
        .users
        .get(&user)
        .cloned()
        .unwrap_or_else(|| UserSummary {
            id: user,
            username: a.model.name(user),
            ..Default::default()
        });
    if let Some(m) = a
        .guild
        .and_then(|g| a.model.members.get(&user).filter(|m| m.guild_id == g))
    {
        apply(&mut u, m.nickname.as_deref(), &m.server_style);
    }
    u
}
/// A member as their server shows them.
pub(super) fn member_user(m: &Member) -> UserSummary {
    let mut u = m.user.clone();
    apply(&mut u, m.nickname.as_deref(), &m.server_style);
    u
}
fn apply(u: &mut UserSummary, nickname: Option<&str>, s: &UserStyle) {
    if let Some(n) = nickname {
        u.display_name = Some(n.to_owned());
    }
    if let Some(av) = &s.avatar {
        u.avatar_id = Some(av.id);
        u.style.avatar = Some(av.clone());
    }
    if s.name.is_some() {
        u.style.name = s.name;
    }
    if s.decoration.is_some() {
        u.style.decoration = s.decoration.clone();
    }
    if s.nameplate.is_some() {
        u.style.nameplate = s.nameplate.clone();
    }
}

/// UI SDK Avatar: the initials stay until the image has loaded, and return if
/// it fails. An animated avatar shows its still first frame and plays only
/// while its row is hovered or focused (or always on a profile card). A
/// decoration is drawn around it.
pub(super) fn avatar(u: &UserSummary, size: &str) -> AnyView {
    let initials = name(u)
        .split_whitespace()
        .filter_map(|s| s.chars().next())
        .take(2)
        .collect::<String>();
    let meta = u
        .style
        .avatar
        .as_ref()
        .filter(|m| Some(m.id) == u.avatar_id);
    let (crop_class, crop) = crop_style(meta.and_then(|m| m.crop));
    let moving = meta.filter(|m| m.animated).map(|m| {
        view! {<span class="avatar-anim" style=format!("--anim:url(/api/v1/files/{});{crop}",m.id.0) aria-hidden="true"></span>}
    });
    let face = view! {<ui-avatar class=format!("avatar {size}")>{u.avatar_id.map(|id|view!{<img class=format!("avatar-face{crop_class}") style=crop.clone() src=format!("/api/v1/files/{}/thumbnail",id.0) alt="" loading="lazy"/>})}{moving}<ui-avatar-fallback class="avatar-face">{initials}</ui-avatar-fallback></ui-avatar>};
    match known_or_none(DECORATIONS, u.style.decoration.as_deref()) {
        Some(d) => view! {<span class="avatar-frame">{face}<span class=format!("avatar-deco cosmetic deco-{d}{}",self_class(u.id)) aria-hidden="true"></span></span>}.into_any(),
        None => face.into_any(),
    }
}
fn name_class(s: &NameStyle) -> (String, String) {
    (
        format!(
            "styled-name name-font-{} name-effect-{}",
            s.font.name(),
            s.effect.name()
        ),
        format!(
            "--name-c1:{};{}",
            hex(s.color),
            s.color2
                .map(|c| format!("--name-c2:{}", hex(c)))
                .unwrap_or_default()
        ),
    )
}
fn styled(name: &str, style: Option<&NameStyle>, own: bool) -> AnyView {
    match style {
        Some(s) => {
            let (class, vars) = name_class(s);
            view! {<span class=format!("{class}{}",if own{" is-self"}else{""}) style=vars>{name.to_owned()}</span>}.into_any()
        }
        None => view! {<span>{name.to_owned()}</span>}.into_any(),
    }
}
/// A display name in its style. Without a style it inherits the row's color
/// (the role color). "Show plain names" turns other people's styles off.
pub(super) fn styled_name(name: &str, style: Option<&NameStyle>) -> AnyView {
    styled(name, style, false)
}
/// `user`'s name, `shown` as given, in the style they have in the open server.
pub(super) fn name_for(a: &App, user: Id, shown: &str) -> AnyView {
    let u = effective(a, user);
    styled(shown, u.style.name.as_ref(), own(user))
}
/// The background behind a name in the member list and direct message list.
pub(super) fn nameplate(u: &UserSummary) -> AnyView {
    match known_or_none(NAMEPLATES, u.style.nameplate.as_deref()) {
        Some(p) => view! {<span class=format!("nameplate cosmetic plate-{p}{}",self_class(u.id)) aria-hidden="true"></span>}.into_any(),
        None => ().into_any(),
    }
}
/// A direct message row shows the other person's nameplate.
pub(super) fn dm_nameplate(a: &App, c: &ChannelSummary) -> AnyView {
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
        Some(u) => nameplate(u),
        None => ().into_any(),
    }
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
fn badge_icon(b: &Badge) -> String {
    match (&b.custom, &b.icon) {
        (Some(id), _) => format!("/api/v1/badges/{}/icon", id.0),
        (None, Some(icon)) if icon.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') => {
            format!("/public/badges/{icon}.svg")
        }
        _ => "/public/badges/contributor.svg".into(),
    }
}
fn badges_row(badges: &[Badge]) -> AnyView {
    if badges.is_empty() {
        return ().into_any();
    }
    view!{<div class="profile-badges flex flex-wrap justify-end gap-1 rounded-lg bg-black/35 p-1" role="list" aria-label="Badges">{badges.iter().map(|b|view!{<img role="listitem" src=badge_icon(b) alt=b.name.clone() title=format!("{}: {}",b.name,b.description) width="22" height="22"/>}).collect_view()}</div>}.into_any()
}
/// Profile text through the shared message renderer (Markdown subset, links,
/// emoji, and whatever else messages learn to draw, custom emoji included).
fn rich_text(a: &App, user: Id, text: &str) -> AnyView {
    // The renderer reads a message only for its mentions; a profile has none.
    let message: Option<Message> = serde_json::from_value(serde_json::json!({
        "id": 0, "channel_id": 0, "author_id": user.0, "content": "", "created_at": 0,
        "edited_at": null, "reply_to": null, "attachments": [], "mention_ids": [], "reactions": []
    }))
    .ok();
    match message {
        Some(m) => markdown(a, &m, text),
        None => view! {<p class="whitespace-pre-wrap">{text.to_owned()}</p>}.into_any(),
    }
}

/// Everything one profile card shows, already resolved for the server it is
/// shown in.
struct Card<'a> {
    user: UserSummary,
    pronouns: String,
    about: String,
    status: Option<String>,
    banner_color: u32,
    banner: Option<&'a ImageMeta>,
    primary: Option<u32>,
    accent: Option<u32>,
    effect: Option<&'a str>,
    frame: Option<&'a str>,
    badges: &'a [Badge],
    created_at: u64,
    member_since: Option<(String, u64)>,
}
fn banner_view(color: u32, banner: Option<&ImageMeta>) -> AnyView {
    let bg = format!(
        "background:{}",
        hex(if color == 0 { 0x496c63 } else { color })
    );
    view!{<div class="profile-banner-box profile-banner" data-pv="banner" style=bg>{banner.map(|b|{
        let (crop_class, crop) = crop_style(b.crop);
        if b.animated {
            view!{<img class=format!("banner-face{crop_class}") style=crop.clone() src=format!("/api/v1/files/{}/thumbnail",b.id.0) alt=""/><span class="avatar-anim" style=format!("--anim:url(/api/v1/files/{});{crop}",b.id.0) aria-hidden="true"></span>}.into_any()
        } else {
            view!{<img class=format!("banner-face{crop_class}") style=crop src=format!("/api/v1/files/{}",b.id.0) alt=""/>}.into_any()
        }
    })}</div>}.into_any()
}
/// Tint: the primary and accent colors as a gradient, with text and a scrim
/// chosen so that everything on the card stays readable.
fn tint(primary: Option<u32>, accent: Option<u32>) -> (String, String) {
    match (primary, accent) {
        (None, None) => ("profile-card".into(), String::new()),
        (p, a) => {
            let (p, a) = (p.or(a).unwrap_or(0), a.or(p).unwrap_or(0));
            let t = card_theme(p, a);
            (
                format!(
                    "profile-card tinted{}",
                    if t.text == CARD_DARK_TEXT {
                        " dark-text"
                    } else {
                        ""
                    }
                ),
                format!(
                    "--card-p:{};--card-a:{};--card-text:{};--card-scrim:{}",
                    hex(p),
                    hex(a),
                    hex(t.text),
                    f32::from(t.scrim) / 100.0
                ),
            )
        }
    }
}
/// An avatar whose decoration the editor's preview can change: the
/// decoration is a sibling it owns rather than part of `avatar()`.
fn decorated(u: &UserSummary, size: &str) -> AnyView {
    let mut face = u.clone();
    let deco = known_or_none(DECORATIONS, face.style.decoration.take().as_deref());
    view!{<span class="avatar-frame">{avatar(&face,size)}<span class=format!("avatar-deco cosmetic deco-{}{}",deco.unwrap_or("none"),self_class(u.id)) data-pv="deco" hidden=deco.is_none() aria-hidden="true"></span></span>}.into_any()
}
/// The name style as a card tinted with these colors can show it: one that
/// does not reach AA contrast on the card (a pale name on a light card) is
/// drawn in the card's own text color, in its font (task `qa2`).
fn on_card(
    style: Option<&NameStyle>,
    primary: Option<u32>,
    accent: Option<u32>,
) -> Option<NameStyle> {
    let style = style?;
    if primary.is_none() && accent.is_none() {
        return Some(*style);
    }
    let (p, a) = (
        primary.or(accent).unwrap_or(0),
        accent.or(primary).unwrap_or(0),
    );
    if sticktalk_proto::name_reads_on_card(style, p, a) {
        return Some(*style);
    }
    let text = sticktalk_proto::card_theme(p, a).text;
    Some(NameStyle {
        color: text,
        color2: None,
        effect: NameEffect::Solid,
        ..*style
    })
}
fn card_view(a: &App, c: Card, below: AnyView) -> AnyView {
    let (class, vars) = tint(c.primary, c.accent);
    let name_style = on_card(c.user.style.name.as_ref(), c.primary, c.accent);
    let effect = known_or_none(EFFECTS, c.effect);
    let frame = known_or_none(FRAMES, c.frame);
    let u = &c.user;
    let section = "profile-section mt-3 rounded-lg p-3 text-sm";
    view!{<article class=format!("{class} rounded-xl") style=vars data-pv="card" aria-label=format!("{}'s profile",name(u))>
        {banner_view(c.banner_color, c.banner)}
        <div class="relative z-[1] flex items-end justify-between gap-2 px-4 -mt-10">{decorated(u,"large")}<div data-pv="badges">{badges_row(c.badges)}</div></div>
        <div class="profile-preview-copy relative z-[1] px-4 pt-3 pb-4 wrap-anywhere">
            <h2 class="m-0 text-xl leading-tight font-semibold wrap-anywhere" data-pv="name" data-preview-name>{styled(&name(u),name_style.as_ref(),own(u.id))}</h2>{super::identity::tag(a,u)}
            <p class="profile-username mt-1 mb-0 text-sm text-subtle">{format!("@{}",u.username)}<span data-pv="pronouns">{if c.pronouns.is_empty(){String::new()}else{format!(" · {}",c.pronouns)}}</span></p>
            <p class="mt-2 mb-0 text-sm whitespace-pre-wrap empty:hidden" data-preview-status>{c.status.clone().unwrap_or_default()}</p>
            <section class=section data-pv="about-box" hidden=c.about.is_empty()><h3 class="category mb-1">"About Me"</h3><div data-pv="about">{rich_text(a,u.id,&c.about)}</div><p class="m-0 whitespace-pre-wrap" data-pv="about-draft" hidden></p></section>
            <section class=section><h3 class="category mb-1">"Member Since"</h3><p class="m-0">{format!("StickTalk · {}",date(c.created_at))}</p>{c.member_since.map(|(server,t)|view!{<p class="m-0 mt-1">{format!("{server} · {}",date(t))}</p>})}</section>
            {below}
        </div>
        <div class=format!("profile-effect cosmetic effect-{}{}",effect.unwrap_or("none"),self_class(u.id)) data-pv="effect" hidden=effect.is_none()></div>
        <div class=format!("profile-frame cosmetic frame-{}{}",frame.unwrap_or("none"),self_class(u.id)) data-pv="frame" hidden=frame.is_none()></div>
    </article>}.into_any()
}

/// The profile popout: the card as the server it was opened in shows it.
pub(super) fn card(a: &App) -> AnyView {
    let Some(detail) = &a.profile else {
        return ().into_any();
    };
    let base = &detail.profile;
    let loaded = a
        .profiles
        .card
        .as_ref()
        .filter(|(c, _)| c.user.id == base.user.id);
    let server = loaded.and_then(|(c, _)| c.server.as_ref());
    let look = loaded.map(|(c, _)| &c.look);
    let mut user = loaded.map_or_else(|| base.user.clone(), |(c, _)| c.user.clone());
    if let Some(s) = server {
        apply(&mut user, s.nickname.as_deref(), &s.style());
    } else if let Some(m) = a
        .guild
        .and_then(|g| a.model.members.get(&user.id).filter(|m| m.guild_id == g))
    {
        apply(&mut user, m.nickname.as_deref(), &m.server_style);
    }
    let guild = loaded.and_then(|(_, g)| *g);
    let guild_name = guild
        .and_then(|g| a.model.guilds.get(&g))
        .map(|g| g.guild.name.clone());
    let pick = |s: Option<&String>, g: &String| s.cloned().unwrap_or_else(|| g.clone());
    let mutual = format!(
        "{} mutual server{} · {} mutual friend{}",
        detail.mutual_servers.len(),
        if detail.mutual_servers.len() == 1 {
            ""
        } else {
            "s"
        },
        detail.mutual_friends.len(),
        if detail.mutual_friends.len() == 1 {
            ""
        } else {
            "s"
        }
    );
    let below = view! {{super::identity::role_pills(a,guild,base.user.id)}<p class="mt-3 mb-0 text-[13px] text-subtle">{mutual}</p>{profile_actions(a,&base.user)}{reset_actions(a,guild,&user,server)}}.into_any();
    card_view(
        a,
        Card {
            pronouns: pick(server.and_then(|s| s.pronouns.as_ref()), &base.pronouns),
            about: pick(server.and_then(|s| s.about_me.as_ref()), &base.about_me),
            // Someone you ignore: no status or activity (privacy.md).
            status: user
                .status_text
                .clone()
                .filter(|_| !crate::live::privacy::ignored(user.id)),
            banner_color: base.banner_color,
            banner: server
                .and_then(|s| s.banner.as_ref())
                .or(look.and_then(|l| l.banner.as_ref())),
            primary: server
                .and_then(|s| s.primary)
                .or(look.and_then(|l| l.primary)),
            accent: server
                .and_then(|s| s.accent)
                .or(look.and_then(|l| l.accent)),
            effect: server
                .and_then(|s| s.effect.as_deref())
                .or(look.and_then(|l| l.effect.as_deref())),
            frame: server
                .and_then(|s| s.frame.as_deref())
                .or(look.and_then(|l| l.frame.as_deref())),
            badges: loaded.map_or(&[][..], |(c, _)| &c.badges),
            created_at: base.created_at,
            member_since: guild_name.zip(loaded.and_then(|(c, _)| c.member_since)),
            user,
        },
        below,
    )
}
/// Moderators: reset a member's server nickname (Manage Nicknames) and remove
/// their server avatar or banner (Time Out Members). The server checks both
/// and the role hierarchy.
fn reset_actions(
    a: &App,
    guild: Option<Id>,
    u: &UserSummary,
    server: Option<&ServerProfile>,
) -> AnyView {
    let Some(g) = guild.filter(|g| a.guild == Some(*g)) else {
        return ().into_any();
    };
    if own(u.id) {
        return ().into_any();
    }
    let nick = server.is_some_and(|s| s.nickname.is_some())
        && settings::can(a, Permissions::MANAGE_NICKNAMES);
    let moderate = settings::can(a, Permissions::MODERATE_MEMBERS);
    let avatar = moderate && server.is_some_and(|s| s.avatar.is_some());
    let banner = moderate && server.is_some_and(|s| s.banner.is_some());
    if !(nick || avatar || banner) {
        return ().into_any();
    }
    let id = u.id.0;
    view!{<section class="member-profile-moderation mt-4 border-t pt-4" aria-label="Server profile moderation"><h3 class="category mb-2">"Server Profile"</h3><div class="flex flex-wrap gap-2">
        {nick.then(||button("Reset Nickname",&format!("profiles:nick-reset,{},{id}",g.0),"secondary"))}
        {avatar.then(||button("Remove Server Avatar",&format!("profiles:reset,{},{id},avatar",g.0),"danger-text"))}
        {banner.then(||button("Remove Server Banner",&format!("profiles:reset,{},{id},banner",g.0),"danger-text"))}
    </div></section>}.into_any()
}

// ---- Editor ----------------------------------------------------------------

/// One picker: a hidden field holding the choice and a grid of options.
fn picker(
    field: &str,
    label: &str,
    catalog: &[Cosmetic],
    chosen: Option<&str>,
    inherit: Option<&str>,
    swatch: fn(&str) -> AnyView,
) -> AnyView {
    let chosen = chosen.unwrap_or("").to_owned();
    let none_label = if inherit.is_some() {
        "Same as profile"
    } else {
        "None"
    };
    let option = |value: &str, text: &str, art: AnyView| {
        view! {<button type="button" class="cosmetic-option" aria-pressed=(value==chosen).to_string() data-profiles=format!("pick,{field},{value}")>{art}<span class="max-w-full truncate">{text.to_owned()}</span></button>}
    };
    view!{<fieldset class="mt-6"><legend class="mb-2 text-xs font-semibold tracking-[0.5px] text-muted-foreground uppercase">{label.to_owned()}</legend>
        <input type="hidden" id=field.to_owned() value=chosen.clone() data-saved=chosen.clone() data-fallback=inherit.unwrap_or("").to_owned()/>
        <div class="cosmetic-grid" role="group" aria-label=label.to_owned()>
            {option("",none_label,view!{<span class="grid size-10 place-items-center text-muted-foreground">{icon(if inherit.is_some(){"user-round"}else{"x"})}</span>}.into_any())}
            {catalog.iter().map(|(id,label)|option(id,label,swatch(id))).collect_view()}
        </div></fieldset>}.into_any()
}
fn deco_swatch(id: &str) -> AnyView {
    view!{<span class="deco-swatch avatar-frame"><span class="avatar">"Aa"</span><span class=format!("avatar-deco deco-{id}") aria-hidden="true"></span></span>}.into_any()
}
fn plate_swatch(id: &str) -> AnyView {
    view!{<span class="plate-swatch grid place-items-center text-[13px] font-medium"><span class=format!("nameplate plate-{id}") aria-hidden="true"></span>"Name"</span>}.into_any()
}
fn effect_swatch(id: &str) -> AnyView {
    view!{<span class="effect-swatch"><span class=format!("profile-effect effect-{id}") aria-hidden="true"></span></span>}.into_any()
}
fn frame_swatch(id: &str) -> AnyView {
    view!{<span class="frame-swatch"><span class=format!("profile-frame frame-{id}") aria-hidden="true"></span></span>}.into_any()
}
fn color_field(id: &str, label: &str, value: u32) -> AnyView {
    view!{<label class="field mb-0 min-w-32 flex-1"><span>{label.to_owned()}</span><input id=id.to_owned() type="color" value=hex(value) data-saved=hex(value)/></label>}.into_any()
}
fn check(id: &str, label: &str, on: bool, help: &str) -> AnyView {
    view!{<label class="live-checkbox"><span class="flex flex-col"><span>{label.to_owned()}</span>{(!help.is_empty()).then(||view!{<small class="text-xs text-subtle">{help.to_owned()}</small>})}</span><input id=id.to_owned() type="checkbox" checked=on data-saved=on.to_string()/></label>}.into_any()
}
/// Name style, profile colors and the four cosmetic pickers, shared by the
/// user and server forms. `inherit` is the global profile for a server form.
fn look_fields(
    style: Option<&NameStyle>,
    primary: Option<u32>,
    accent: Option<u32>,
    chosen: [Option<&str>; 4],
    inherit: Option<[Option<&str>; 4]>,
) -> AnyView {
    let s = style.copied().unwrap_or(NameStyle {
        font: NameFont::Default,
        color: 0xE8EAED,
        color2: Some(0x3DDC97),
        effect: NameEffect::Solid,
    });
    let server = inherit.is_some();
    let fallback = |i: usize| inherit.map(|v| v[i].unwrap_or(""));
    view!{
        <h3 class="mt-8">"Display Name Style"</h3>
        {check("pf-name-on",if server{"Use a different name style in this server"}else{"Style my name"},style.is_some(),"Shown in chat, the member list and your profile.")}
        <div class="flex flex-wrap gap-4" data-pv="name-fields">
            <label class="field mb-0 min-w-40 flex-1"><span>"Font"</span><select id="pf-name-font" data-saved=s.font.name()>{NameFont::ALL.iter().map(|f|view!{<option value=f.name() selected=*f==s.font>{f.label()}</option>}).collect_view()}</select></label>
            <label class="field mb-0 min-w-40 flex-1"><span>"Effect"</span><select id="pf-name-effect" data-saved=s.effect.name()>{NameEffect::ALL.iter().map(|f|view!{<option value=f.name() selected=*f==s.effect>{f.label()}</option>}).collect_view()}</select></label>
        </div>
        <div class="mt-4 flex flex-wrap gap-4">{color_field("pf-name-c1","Name Color",s.color)}{color_field("pf-name-c2","Second Color",s.color2.unwrap_or(0x3DDC97))}</div>
        <h3 class="mt-8">"Profile Colors"</h3>
        {check("pf-colors-on",if server{"Use different profile colors in this server"}else{"Use profile colors"},primary.is_some()||accent.is_some(),"Tints your profile card. Text color adjusts so it stays readable.")}
        <div class="flex flex-wrap gap-4">{color_field("pf-primary","Primary",primary.unwrap_or(0x2B5B4E))}{color_field("pf-accent","Accent",accent.unwrap_or(0x17171A))}</div>
        {picker("pf-deco","Avatar Decoration",DECORATIONS,chosen[0],fallback(0),deco_swatch)}
        {picker("pf-plate","Nameplate",NAMEPLATES,chosen[1],fallback(1),plate_swatch)}
        {picker("pf-effect","Profile Effect",EFFECTS,chosen[2],fallback(2),effect_swatch)}
        {picker("pf-frame","Profile Frame",FRAMES,chosen[3],fallback(3),frame_swatch)}
    }.into_any()
}
/// Avatar and banner buttons. Choosing a picture opens the crop dialog.
fn image_buttons(a: &App, guild: Option<Id>, avatar: bool, banner: bool) -> AnyView {
    let g = guild.map_or(0, |g| g.0);
    let remove_avatar = if guild.is_some() {
        format!("profiles:remove,avatar,{g}")
    } else {
        "avatar-remove".into()
    };
    view!{<h3 class="mt-8">"Avatar & Banner"</h3><p class="muted-copy mb-3">"PNG, JPEG, WebP or GIF, up to 10 MB. GIFs, animated WebP and APNG move when someone hovers over you."</p>
        <div class="flex flex-wrap gap-2">
            <button type="button" class="button secondary" data-pick="live-avatar">"Change Avatar"</button>
            {avatar.then(||view!{<button type="button" class="button link-button" data-live=remove_avatar.clone()>"Remove Avatar"</button>})}
            <button type="button" class="button secondary" data-pick="live-banner">"Change Banner"</button>
            {banner.then(||view!{<button type="button" class="button link-button" data-live=format!("profiles:remove,banner,{g}")>"Remove Banner"</button>})}
        </div>
        <input type="file" id="live-avatar" accept="image/png,image/jpeg,image/webp,image/gif" hidden data-profiles-guild=g.to_string()/>
        <input type="file" id="live-banner" accept="image/png,image/jpeg,image/webp,image/gif" hidden data-profiles-guild=g.to_string()/>
        {if a.uploading{view!{<p class="mt-2 text-sm text-subtle" role="status">{a.upload_progress.clone()}</p>}.into_any()}else{().into_any()}}
    }.into_any()
}
fn text_fields(pronouns: &str, about: &str, server: bool) -> AnyView {
    view!{
        <h3 class="mt-8">"About"</h3>
        <div class="field"><label for="pf-pronouns">"Pronouns"</label><input id="pf-pronouns" type="text" maxlength=MAX_PRONOUNS_CHARS.to_string() value=pronouns.to_owned() data-saved=pronouns.to_owned() placeholder=if server{"Same as profile"}else{""}/></div>
        <div class="field"><label for="pf-about">"About Me"</label><textarea id="pf-about" rows="4" maxlength=MAX_ABOUT_ME_CHARS.to_string() data-saved=about.to_owned() aria-describedby="pf-about-help" placeholder=if server{"Same as profile"}else{""}>{about.to_owned()}</textarea><small id="pf-about-help">{format!("Markdown, links and emoji work. Up to {MAX_ABOUT_ME_CHARS} characters.")}<span class="float-right" data-pv="about-count"></span></small></div>
    }.into_any()
}
fn save_bar(form: &'static str, a: &App) -> AnyView {
    view!{<div class="profile-save-bar fixed right-[max(16px,calc((100vw-280px-700px)/2+16px))] bottom-[calc(16px+var(--safe-bottom))] left-[max(296px,calc((100vw-280px-700px)/2+296px))] z-[5] flex items-center gap-2 rounded-lg border bg-rail px-4 py-3 shadow-lg phone:right-2 phone:bottom-[calc(8px+var(--safe-bottom))] phone:left-2 phone:flex-wrap phone:p-3" hidden><span class="flex-1 text-[13px] phone:basis-full" role="status">"You have unsaved changes."</span><button type="button" class="button link-button phone:flex-1" data-profile-reset>"Reset"</button><button type="submit" class="button primary phone:flex-1">"Save changes"</button></div>{saved_note(a,form)}}.into_any()
}
/// Sample rows under the preview card: how the name reads in chat and in
/// the member list (with the nameplate).
fn samples(u: &UserSummary) -> AnyView {
    view!{<div class="mt-4 overflow-hidden rounded-xl border bg-background" aria-label="How your name looks">
        <div class="message name-preview flex gap-3 px-3 py-2">{decorated(u,"small")}<div class="min-w-0"><strong class="block truncate text-[15px] font-semibold" data-pv="msg-name">{styled(&name(u),u.style.name.as_ref(),true)}</strong><span class="text-sm text-subtle">"Hey, how’s it going?"</span></div></div>
        <div class="member-row name-preview flex min-h-[42px] items-center gap-3 bg-sidebar px-3 py-1"><span class=format!("nameplate cosmetic plate-{} is-self",u.style.nameplate.as_deref().unwrap_or("none")) data-pv="plate" hidden=u.style.nameplate.is_none() aria-hidden="true"></span>{decorated(u,"small")}<span class="min-w-0 truncate text-[15px] font-medium" data-pv="row-name">{styled(&name(u),u.style.name.as_ref(),true)}</span></div>
    </div>}.into_any()
}
fn tabs(a: &App) -> AnyView {
    let tab = |t: Tab, label: &str, value: &str| {
        let on = a.profiles.tab == t;
        view! {<button type="button" role="tab" aria-selected=on.to_string() class=if on{"button secondary"}else{"button link-button"} data-live=format!("profiles:tab,{value}")>{label.to_owned()}</button>}
    };
    view!{<div class="profiles-tabs mb-6 flex flex-wrap gap-2" role="tablist" aria-label="Profiles">{tab(Tab::User,"User Profile","user")}{tab(Tab::Server,"Server Profiles","server")}{tab(Tab::Badges,"Badges","badges")}</div>}.into_any()
}
const EDITOR: &str = "profile-editor grid grid-cols-[minmax(0,1fr)_minmax(260px,320px)] items-start gap-6 narrow:grid-cols-[minmax(0,1fr)]";
const PREVIEW: &str = "profile-preview col-start-2 row-start-1 w-full self-start narrow:col-start-1 narrow:row-start-auto min-[1101px]:sticky min-[1101px]:top-16";

pub(super) fn editor(a: &App) -> AnyView {
    let body = match a.profiles.tab {
        Tab::User => user_tab(a),
        Tab::Server => server_tab(a),
        Tab::Badges => badges_tab(a),
    };
    view! {<h2>"Profiles"</h2>{tabs(a)}<div data-profiles-root>{body}</div>}.into_any()
}
fn loading() -> AnyView {
    view! {<div class="skeleton-lines grid gap-4 py-6 [&>span]:h-4 [&>span]:w-4/5 [&>span]:rounded-sm [&>span]:bg-muted" role="status" aria-label="Loading"><span></span><span></span><span></span></div>}.into_any()
}
fn user_tab(a: &App) -> AnyView {
    let Some(me) = a.model.me.as_ref().map(|m| &m.user) else {
        return ().into_any();
    };
    let Some(mine) = &a.profiles.mine else {
        return loading();
    };
    let s = &me.style;
    let l = &mine.look;
    let preview = card_view(
        a,
        Card {
            user: me.clone(),
            pronouns: mine.pronouns.clone(),
            about: mine.about_me.clone(),
            status: me.status_text.clone(),
            banner_color: mine.banner_color,
            banner: l.banner.as_ref(),
            primary: l.primary,
            accent: l.accent,
            effect: l.effect.as_deref(),
            frame: l.frame.as_deref(),
            badges: &mine.earned[..mine.shown.len().min(mine.earned.len())],
            created_at: a
                .profile
                .as_ref()
                .filter(|p| p.profile.user.id == me.id)
                .map_or(0, |p| p.profile.created_at),
            member_since: None,
        },
        ().into_any(),
    );
    let banner_color = if mine.banner_color == 0 {
        0x496c63
    } else {
        mine.banner_color
    };
    view!{<div class=EDITOR>
        <aside class=PREVIEW aria-label="Profile preview">{preview}{samples(me)}</aside>
        <form class="min-w-0" data-live-form="profiles-user" data-profile-editor data-profiles-editor data-fallback-name=me.username.clone()>
            <div class="field"><label for="f-name">"Display Name"</label><input id="f-name" type="text" maxlength="64" value=me.display_name.clone().unwrap_or_default() data-saved=me.display_name.clone().unwrap_or_default() aria-describedby="display-name-help"/><small id="display-name-help">"How you appear in conversations. Leave blank to use your username."</small></div>
            <div class="field"><label for="f-value">"Status"</label><input id="f-value" type="text" maxlength="128" value=me.status_text.clone().unwrap_or_default() data-saved=me.status_text.clone().unwrap_or_default() aria-describedby="status-help"/><small id="status-help">"Let friends know what you’re up to."</small></div>
            {text_fields(&mine.pronouns,&mine.about_me,false)}
            {image_buttons(a,None,me.avatar_id.is_some(),l.banner.is_some())}
            <div class="mt-4 flex flex-wrap gap-4">{color_field("pf-banner-color","Banner Color",banner_color)}</div>
            {look_fields(s.name.as_ref(),l.primary,l.accent,[s.decoration.as_deref(),s.nameplate.as_deref(),l.effect.as_deref(),l.frame.as_deref()],None)}
            {save_bar("profiles-user",a)}
        </form>
        {super::identity::tag_picker(a)}
    </div>}.into_any()
}
fn server_tab(a: &App) -> AnyView {
    let Some(me) = a.model.me.as_ref().map(|m| &m.user) else {
        return ().into_any();
    };
    let Some(mine) = &a.profiles.mine else {
        return loading();
    };
    let mut guilds: Vec<_> = a.model.guilds.values().collect();
    guilds.sort_by_key(|g| g.guild.name.to_lowercase());
    if guilds.is_empty() {
        return view!{<div class="settings-empty flex flex-col items-start gap-4 pt-6 pb-2">{icon("users")}<p>"Join a server to give yourself a different name, avatar and look there."</p></div>}.into_any();
    }
    let g = a
        .profiles
        .guild
        .filter(|g| a.model.guilds.contains_key(g))
        .unwrap_or(guilds[0].guild.id);
    let empty = ServerProfile::default();
    let p = mine
        .servers
        .iter()
        .find(|s| s.guild_id == g)
        .map_or(&empty, |s| &s.profile);
    let l = &mine.look;
    let s = &me.style;
    // The preview: the global profile with this server's overrides.
    let mut u = me.clone();
    apply(&mut u, p.nickname.as_deref(), &p.style());
    let preview = card_view(
        a,
        Card {
            user: u.clone(),
            pronouns: p.pronouns.clone().unwrap_or_else(|| mine.pronouns.clone()),
            about: p.about_me.clone().unwrap_or_else(|| mine.about_me.clone()),
            status: me.status_text.clone(),
            banner_color: mine.banner_color,
            banner: p.banner.as_ref().or(l.banner.as_ref()),
            primary: p.primary.or(l.primary),
            accent: p.accent.or(l.accent),
            effect: p.effect.as_deref().or(l.effect.as_deref()),
            frame: p.frame.as_deref().or(l.frame.as_deref()),
            badges: &mine.earned[..mine.shown.len().min(mine.earned.len())],
            created_at: a
                .profile
                .as_ref()
                .filter(|v| v.profile.user.id == me.id)
                .map_or(0, |v| v.profile.created_at),
            member_since: None,
        },
        ().into_any(),
    );
    let nick = p.nickname.clone().unwrap_or_default();
    let can_nick = a.model.guilds.get(&g).is_some_and(|g| {
        g.my_permissions.contains(Permissions::CHANGE_NICKNAME) || g.my_role == MemberRole::Owner
    });
    let global = [
        s.decoration.as_deref(),
        s.nameplate.as_deref(),
        l.effect.as_deref(),
        l.frame.as_deref(),
    ];
    view!{<div class=EDITOR>
        <aside class=PREVIEW aria-label="Server profile preview">{preview}{samples(&u)}</aside>
        <form class="min-w-0" data-live-form="profiles-server" data-profiles-editor data-fallback-name=name(me) data-fallback-pronouns=mine.pronouns.clone() data-fallback-about=mine.about_me.clone() data-fallback-style=s.name.map(|n|{let (c,v)=name_class(&n);format!("{c}|{v}")}).unwrap_or_default() data-fallback-primary=l.primary.map(hex).unwrap_or_default() data-fallback-accent=l.accent.map(hex).unwrap_or_default()>
            <label class="field"><span>"Server"</span><select id="pf-guild" data-profiles-guild-select>{guilds.iter().map(|v|view!{<option value=v.guild.id.0.to_string() selected=v.guild.id==g>{v.guild.name.clone()}</option>}).collect_view()}</select></label>
            <p class="muted-copy">"Everything here is optional and only shows in this server. Anything left empty uses your user profile."</p>
            <div class="field"><label for="pf-nickname">"Server Nickname"</label><input id="pf-nickname" type="text" maxlength="32" value=nick.clone() data-saved=nick placeholder=name(me) disabled=!can_nick/>{(!can_nick).then(||view!{<small>"This server doesn’t let members change their nickname."</small>})}</div>
            {text_fields(p.pronouns.as_deref().unwrap_or(""),p.about_me.as_deref().unwrap_or(""),true)}
            {image_buttons(a,Some(g),p.avatar.is_some(),p.banner.is_some())}
            {look_fields(p.name.as_ref(),p.primary,p.accent,[p.decoration.as_deref(),p.nameplate.as_deref(),p.effect.as_deref(),p.frame.as_deref()],Some(global))}
            {save_bar("profiles-server",a)}
        </form>
    </div>}.into_any()
}
fn badges_tab(a: &App) -> AnyView {
    let Some(mine) = &a.profiles.mine else {
        return loading();
    };
    if mine.earned.is_empty() {
        return view!{<div class="settings-empty flex flex-col items-start gap-4 pt-6 pb-2">{icon("shield")}<p>"You haven’t earned any badges yet. They come with time on StickTalk, with running a big server, and from the people who run this one."</p></div>}.into_any();
    }
    let shown = mine.shown.join(",");
    view!{<form data-live-form="profiles-badges" data-profiles-editor>
        <p class="muted-copy mb-4">{format!("Choose up to {MAX_SHOWN_BADGES} badges for your profile and put them in order.")}</p>
        <input type="hidden" id="pf-badges" value=shown.clone() data-saved=shown/>
        <ul class="badge-list m-0 flex list-none flex-col gap-2 p-0">{mine.earned.iter().enumerate().map(|(i,b)|{let on=mine.shown.contains(&b.id);view!{<li class="badge-row flex items-center gap-3 rounded-lg border bg-card p-3" data-badge=b.id.clone()>
            <img src=badge_icon(b) alt="" width="32" height="32"/>
            <span class="flex min-w-0 flex-1 flex-col"><strong class="font-semibold">{b.name.clone()}</strong><small class="text-subtle">{b.description.clone()}</small></span>
            <button type="button" class="icon-button" aria-label=format!("Move {} up",b.name) data-profiles="badge-up" disabled=i==0>{icon("chevron-up")}</button>
            <button type="button" class="icon-button" aria-label=format!("Move {} down",b.name) data-profiles="badge-down" disabled=i+1==mine.earned.len()>{icon("chevron-down")}</button>
            <label class="live-checkbox min-h-0 py-0"><span class="sr-only">{format!("Show {}",b.name)}</span><input type="checkbox" data-profiles-badge=b.id.clone() checked=on/></label>
        </li>}}).collect_view()}</ul>
        <div class="mt-6"><button type="submit" class="button primary">"Save Badges"</button>{saved_note(a,"profiles-badges")}</div>
    </form>}.into_any()
}
