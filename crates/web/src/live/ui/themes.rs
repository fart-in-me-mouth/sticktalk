//! Appearance, Accessibility and Streamer Mode settings, the theme editor, a
//! shared theme's preview and the server Theme page (docs/features/themes.md).
//! Behavior is in `live/themes.rs` and `public/preferences.js`: device
//! preferences are `data-preference` fields; theme choices are radios that
//! `preferences.js` turns into `appearance:theme,...` actions, so picking a
//! theme never re-renders this page.
use super::*;
use crate::live::themes as th;
use std::cell::RefCell;

pub(super) use th::logo;

thread_local! {
    /// The presets' swatch styles never change: derived once.
    static PRESET_SWATCHES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xFF_FFFF)
}
/// Inline custom properties drawing a miniature of `t` (prefix `sw` or `pv`).
fn vars(t: &Theme, prefix: &str) -> String {
    let p = &t.palette;
    let mut s = format!(
        "--{prefix}-rail:{};--{prefix}-sidebar:{};--{prefix}-chat:{};--{prefix}-elevated:{};--{prefix}-text:{};--{prefix}-muted:{};--{prefix}-accent:{};--{prefix}-accent-text:{};--{prefix}-divider:{};",
        hex(p.rail),
        hex(p.sidebar),
        hex(p.chat),
        hex(p.elevated),
        hex(p.text_primary),
        hex(p.text_muted),
        hex(p.accent),
        hex(p.accent_text),
        hex(p.divider)
    );
    if !t.stops.is_empty() {
        let grad = |pick: fn(&(u32, u32, u32)) -> u32| {
            let stops: Vec<String> = t.stops.iter().map(|x| hex(pick(x))).collect();
            format!("linear-gradient({}deg,{})", t.angle, stops.join(","))
        };
        s.push_str(&format!(
            "--{prefix}-rail-img:{};--{prefix}-sidebar-img:{};--{prefix}-chat-img:{};",
            grad(|x| x.0),
            grad(|x| x.1),
            grad(|x| x.2)
        ));
    }
    s
}
fn option(value: &str, label: &str, style: String, selected: bool) -> AnyView {
    view!{<label class="theme-option"><input type="radio" name="theme-choice" value=value.to_owned() data-theme-choice="" checked=selected/><span class="theme-swatch" style=style><i></i><i></i><span><b></b><b></b><b></b></span></span><span class="theme-check">{icon("check")}</span><span class="truncate">{label.to_owned()}</span></label>}.into_any()
}
fn selection() -> String {
    let p = th::prefs();
    match p.theme.as_str() {
        "preset" | "custom" => format!("{}:{}", p.theme, p.theme_id),
        other => other.to_owned(),
    }
}
fn preset_swatches() -> Vec<String> {
    PRESET_SWATCHES.with(|s| {
        if s.borrow().is_empty() {
            *s.borrow_mut() = PRESETS
                .iter()
                .map(|p| vars(&derive(&p.spec(), Adjust::default()), "sw"))
                .collect();
        }
        s.borrow().clone()
    })
}
fn check(label: &str, help: &str, pref: &str) -> AnyView {
    view!{<label class="live-checkbox"><span class="flex flex-col"><span>{label.to_owned()}</span>{if help.is_empty(){().into_any()}else{view!{<small class="text-xs text-subtle">{help.to_owned()}</small>}.into_any()}}</span><input type="checkbox" data-preference=pref.to_owned()/></label>}.into_any()
}
fn select(label: &str, pref: &str, options: &[(&str, &str)]) -> AnyView {
    view!{<label class="field"><span>{label.to_owned()}</span><select data-preference=pref.to_owned()>{options.iter().map(|(v,l)|view!{<option value=v.to_string()>{l.to_string()}</option>}).collect_view()}</select></label>}.into_any()
}
fn radios(legend: &str, pref: &str, options: &[(&str, &str)]) -> AnyView {
    view!{<fieldset><legend class="text-[13px] font-medium text-subtle">{legend.to_owned()}</legend><div class="choice-row">{options.iter().map(|(v,l)|view!{<label><input type="radio" name=format!("pref-{pref}") value=v.to_string() data-preference=pref.to_owned()/>{l.to_string()}</label>}).collect_view()}</div></fieldset>}.into_any()
}
fn range(label: &str, pref: &str, min: u32, max: u32, step: u32) -> AnyView {
    let out = format!("out-{pref}");
    view!{<label class="range-field"><span>{label.to_owned()}</span><input type="range" min=min.to_string() max=max.to_string() step=step.to_string() data-preference=pref.to_owned() data-output=out.clone()/><output id=out></output></label>}.into_any()
}
const AUTOPLAY: [(&str, &str); 3] = [
    ("always", "Always"),
    ("hover", "On hover or focus"),
    ("never", "Never"),
];

/// Settings > Appearance.
pub(super) fn appearance(a: &App) -> AnyView {
    let sel = selection();
    let bases = [
        (
            "dark",
            "Dark",
            vars(&derive_palette(DARK, true, Adjust::default()), "sw"),
        ),
        (
            "light",
            "Light",
            vars(&derive_palette(LIGHT, false, Adjust::default()), "sw"),
        ),
        (
            "darker",
            "Darker",
            vars(&derive_palette(DARKER, true, Adjust::default()), "sw"),
        ),
        (
            "system",
            "Sync with system",
            vars(&derive_palette(DARK, true, Adjust::default()), "sw"),
        ),
    ];
    let swatches = preset_swatches();
    let presets = |base: ThemeBase| {
        PRESETS
            .iter()
            .zip(swatches.iter())
            .filter(move |(p, _)| p.base == base)
            .map(|(p, s)| {
                let value = format!("preset:{}", p.id);
                option(&value, p.name, s.clone(), sel == value)
            })
            .collect_view()
    };
    let library = a.themes.library.as_ref();
    let custom = library.map(|l| l.themes.clone()).unwrap_or_default();
    let servers: Vec<(Id, String, bool)> = library
        .map(|l| {
            l.servers
                .iter()
                .filter_map(|s| a.model.guilds.get(&s.guild_id))
                .map(|g| {
                    let id = g.guild.id;
                    let choice = l
                        .settings
                        .server_overrides
                        .iter()
                        .find(|o| o.guild_id == id)
                        .map(|o| o.show);
                    (
                        id,
                        g.guild.name.clone(),
                        choice.unwrap_or(l.settings.server_themes),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let overrides = library
        .map(|l| l.settings.server_overrides.clone())
        .unwrap_or_default();
    let shows_default = library.is_none_or(|l| l.settings.server_themes);
    view!{<h2>"Appearance"</h2><p class="muted-copy mb-6">"Saved on this device. Custom themes are saved to your account."</p>
        <h3>"Theme"</h3><div class="theme-grid" role="radiogroup" aria-label="Theme">{bases.into_iter().map(|(v,l,s)|option(v,l,s,sel==v)).collect_view()}</div>
        <div class="settings-divider"></div><h3>"Color Themes"</h3>
        <p class="muted-copy mb-3">"Dark"</p><div class="theme-grid" role="radiogroup" aria-label="Dark color themes">{presets(ThemeBase::Dark)}</div>
        <p class="muted-copy mt-6 mb-3">"Light"</p><div class="theme-grid" role="radiogroup" aria-label="Light color themes">{presets(ThemeBase::Light)}</div>
        <div class="settings-divider"></div><h3>"Custom Themes"</h3>
        {if custom.is_empty(){view!{<p class="muted-copy mb-3">"Pick colors for your own theme. You can save up to 20."</p>}.into_any()}else{view!{<div class="theme-grid mb-4" role="radiogroup" aria-label="Your themes">{custom.iter().map(|t|{let value=format!("custom:{}",t.id.0);let code=share_code(&t.spec,&t.name);view!{<div class="flex flex-col gap-1">{option(&value,&t.name,vars(&derive(&t.spec,Adjust::default()),"sw"),sel==value)}<div class="flex gap-1">{ib_plain("pencil",&format!("Edit {}",t.name),&format!("appearance:edit,custom:{}",t.id.0))}<button class="icon-button" aria-label=format!("Copy a link to {}",t.name) title="Copy Share Link" data-copy=format!("{}/?theme={code}",a.origin)>{icon("link")}</button>{ib_plain("trash-2",&format!("Delete {}",t.name),&format!("appearance:delete,{}",t.id.0))}</div></div>}}).collect_view()}</div>}.into_any()}}
        <div class="flex flex-wrap gap-2">{button("Create Theme","appearance:edit,new","primary")}{if sel.starts_with("preset:"){button("Customize This Theme",&format!("appearance:edit,{sel}"),"secondary")}else{().into_any()}}</div>
        <div class="field mt-4"><label for="theme-code">"Theme Code or Link"</label><div class="flex gap-2"><input id="theme-code" class="flex-1" type="text" maxlength="512" autocomplete="off" spellcheck="false" placeholder="Paste a theme code someone shared" aria-describedby="theme-code-help"/>{button("Preview","appearance:import","secondary")}</div><small id="theme-code-help">{a.themes.import_error.clone().unwrap_or_else(||"Themes shared with you open here first, so you can look before you apply.".into())}</small></div>
        <div class="settings-divider"></div><h3>"Server Themes"</h3>
        <label class="live-checkbox"><span class="flex flex-col"><span>"Show server themes"</span><small class="text-xs text-subtle">"A server can choose a theme for its members. It shows only while you’re in that server."</small></span><input type="checkbox" checked=shows_default data-appearance="server-themes,default"/></label>
        {servers.into_iter().map(|(g,label,_)|{let current=overrides.iter().find(|o|o.guild_id==g).map(|o|if o.show{"show"}else{"hide"}).unwrap_or("default");view!{<label class="field"><span>{label.clone()}</span><select data-appearance=format!("server-themes,{}",g.0)><option value="default" selected=current=="default">"Use my default"</option><option value="show" selected=current=="show">"Show this server’s theme"</option><option value="hide" selected=current=="hide">"Keep my own theme"</option></select></label>}}).collect_view()}
        <div class="settings-divider"></div><h3>"Messages"</h3>
        {select("Message Display","density",&[("cozy","Cozy"),("compact","Compact")])}
        {select("Chat Font Scaling","fontScale",&[("0.8","80%"),("0.9","90%"),("1","100%"),("1.1","110%"),("1.25","125%"),("1.5","150%")])}
        {select("Chat Font","chatFont",&[("inter","Inter (default)"),("system","System"),("serif","Serif"),("mono","Monospace"),("atkinson","Atkinson Hyperlegible"),("dyslexic","OpenDyslexic (dyslexia-friendly)")])}
        {range("Space Between Messages","messageSpacing",0,16,1)}
        {range("Space Between Message Groups","groupSpacing",0,32,2)}
        {select("Zoom Level","zoom",&[("0.8","80%"),("0.9","90%"),("1","100%"),("1.1","110%"),("1.25","125%"),("1.5","150%"),("1.75","175%"),("2","200%")])}
        {check("Show send button","With a keyboard, Enter sends. Touch screens always show it.","sendButton")}
        {check("Show website previews for links","","linkPreviews")}
        <h3 class="mt-6">"Profiles"</h3>{check("Show plain names","Other people’s name fonts, colors and effects are turned off. Role colors stay.","plainNames")}
        {check("Reduce cosmetics","Hide other people’s avatar decorations, nameplates, profile effects and frames.","reduceCosmetics")}
        <div class="settings-divider"></div><h3>"App Icon"</h3><p class="muted-copy mb-3">"Changes the icon in the app and the browser tab. Installed apps follow where the device allows it."</p>
        <div class="app-icon-grid" role="radiogroup" aria-label="App icon">{APP_ICONS.iter().map(|(id,label)|view!{<label class="app-icon-option"><input type="radio" name="app-icon" value=id.to_string() data-preference="appIcon"/><img src=format!("/public/app-icons/{id}-192.png") alt="" width="56" height="56" loading="lazy"/><span>{label.to_string()}</span></label>}).collect_view()}</div>
        <div class="settings-divider"></div><h3>"Preview"</h3><p class="appearance-preview rounded-lg border bg-card p-4">"This is how your messages will look."</p>
        <p class="muted-copy mt-3">"Motion, saturation, contrast and text-to-speech are in Accessibility."</p>{button("Open Accessibility","dialog:accessibility","link-button")}
    }.into_any()
}
/// The app icons (`public/app-icons/`, drawn by `scripts/app-icons.py`).
pub(super) const APP_ICONS: [(&str, &str); 12] = [
    ("mint", "Mint"),
    ("ocean", "Ocean"),
    ("violet", "Violet"),
    ("rose", "Rose"),
    ("amber", "Amber"),
    ("graphite", "Graphite"),
    ("snow", "Snow"),
    ("sunset", "Sunset"),
    ("aurora", "Aurora"),
    ("outline", "Outline"),
    ("midnight", "Midnight"),
    ("lime", "Lime"),
];

/// Settings > Accessibility.
pub(super) fn accessibility() -> AnyView {
    view!{<h2>"Accessibility"</h2><p class="muted-copy mb-6">"Saved on this device."</p>
        <h3>"Color"</h3>
        {range("Saturation","saturation",0,100,5)}<p class="muted-copy">"Lowers the color of role colors, name styles, theme accents and status colors."</p>
        {check("Apply saturation to color and custom themes","","saturationCustom")}
        {check("Increase contrast","Makes text and dividers stand out more (AAA contrast for text).","contrastBoost")}
        {radios("Role Colors","roleColors",&[("names","Show in names"),("dot","Show as a dot next to names"),("off","Don’t show")])}
        <div class="settings-divider"></div><h3>"Links and Focus"</h3>
        {check("Always underline links","","underlineLinks")}
        {check("Always show focus","Draw the focus ring on whatever you click, not only when you use the keyboard.","focusAlways")}
        <div class="settings-divider"></div><h3>"Motion"</h3>
        {radios("Reduced Motion","motion",&[("system","Follow my device"),("on","On"),("off","Off")])}
        <p class="muted-copy">"With reduced motion on, nothing in StickTalk animates: animated emoji, stickers and avatars show a still frame."</p>
        {check("Play Super Reaction animations","","superReactions")}
        <div class="settings-divider"></div><h3>"Play Animations"</h3>
        {select("GIFs","autoplayGifs",&AUTOPLAY)}
        {select("Animated emoji","autoplayEmoji",&AUTOPLAY)}
        {select("Stickers","autoplayStickers",&AUTOPLAY)}
        {select("Animated avatars","autoplayAvatars",&AUTOPLAY)}
        {select("Profile effects","autoplayEffects",&AUTOPLAY)}
        <div class="settings-divider"></div><h3>"Text-to-Speech"</h3>
        {check("Allow playback of /tts messages","Messages sent with /tts are read aloud while you’re looking at that conversation.","ttsPlayback")}
        {select("Read Notifications Aloud","ttsNotify",&[("off","Never"),("channel","For the conversation I’m in"),("all","For all conversations")])}
        <div class="flex flex-wrap items-end gap-3">{select("Speech Rate","ttsRate",&[("0.5","0.5×"),("0.75","0.75×"),("1","1×"),("1.25","1.25×"),("1.5","1.5×"),("2","2×")])}<span class="mb-4">{button("Preview","appearance:tts-preview","secondary")}</span></div>
        <p class="muted-copy">"Nothing is read aloud while you’re in Do Not Disturb or deafened in a call. Type /tts before a message to have it read to others."</p>
    }.into_any()
}

/// Settings > Streamer Mode.
pub(super) fn streamer() -> AnyView {
    view!{<h2>"Streamer Mode"</h2><p class="muted-copy mb-6">"Hides personal information while you stream or share your screen. Press Ctrl+Alt+Shift+S to turn it on or off."</p>
        {check("Streamer Mode","","streamer")}
        {check("Turn on while I share my screen","When you share your screen in a call, Streamer Mode turns on, then back off when you stop.","streamerAuto")}
        <div class="settings-divider"></div><h3>"While Streamer Mode Is On"</h3>
        {check("Hide account details","Your email address and sign-in methods in settings.","streamerAccount")}
        {check("Hide invite links and codes","Click a hidden invite to show it once.","streamerInvites")}
        {check("Hide personal notes","","streamerNotes")}
        {check("Hide message request contents","","streamerRequests")}
        {check("Turn off notification popups","","streamerPopups")}
        {check("Turn off notification sounds","","streamerSounds")}
        {select("Message Previews in the Inbox","streamerPreviews",&[("blur","Blur"),("hide","Hide"),("show","Show")])}
    }.into_any()
}

/// The theme editor (yours, or the server's with `server`).
pub(super) fn editor(a: &App) -> AnyView {
    let Some(d) = a.themes.draft.as_ref() else {
        return view!{<h2>"Theme"</h2><p class="muted-copy">"Choose a theme to edit in Appearance."</p>}.into_any();
    };
    let p = th::prefs();
    let adj = th::adjust(&p, true);
    let t = derive(&d.spec, adj);
    let chosen = saturate(d.spec.accent, adj.saturation);
    let server = d.server.is_some();
    let code = share_code(&d.spec, &d.name);
    let title = if server {
        "Server Theme"
    } else if d.id.is_some() {
        "Edit Theme"
    } else {
        "Create Theme"
    };
    view!{<h2>{title}</h2><div class="theme-editor-grid"><div class="min-w-0" data-theme-editor="">
        {if server{().into_any()}else{view!{<div class="field"><label for="te-name">"Name"</label><input id="te-name" type="text" maxlength="32" value=d.name.clone() autocomplete="off"/></div>}.into_any()}}
        <fieldset><legend class="text-[13px] font-medium text-subtle">"Base"</legend><div class="choice-row"><label><input type="radio" name="te-base" value="dark" checked=d.spec.base==ThemeBase::Dark/>"Dark"</label><label><input type="radio" name="te-base" value="light" checked=d.spec.base==ThemeBase::Light/>"Light"</label></div></fieldset>
        <div class="field"><label for="te-accent">"Accent"</label><input id="te-accent" class="max-w-24" type="color" value=hex(d.spec.accent)/></div>
        <fieldset class="field"><legend class="text-[13px] font-medium text-subtle">"Background Colors (up to 5)"</legend><div class="color-chips">
            {d.spec.colors.iter().enumerate().map(|(i,c)|view!{<span class="inline-flex items-center gap-1"><input id=format!("te-color-{i}") type="color" value=hex(*c) aria-label=format!("Background color {}",i+1)/>{ib_plain("x",&format!("Remove color {}",i+1),&format!("appearance:editor-color-remove,{i}"))}</span>}).collect_view()}
            {if d.spec.colors.len()<MAX_THEME_COLORS{button("Add Color","appearance:editor-color-add","secondary")}else{().into_any()}}</div></fieldset>
        <label class="range-field"><span>"Gradient Direction"</span><input id="te-angle" type="range" min="0" max="359" step="5" value=d.spec.angle.to_string() data-output="te-angle-out" data-unit="°"/><output id="te-angle-out">{format!("{}°",d.spec.angle)}</output></label>
        <label class="range-field"><span>"Intensity"</span><input id="te-intensity" type="range" min="0" max="100" step="5" value=d.spec.intensity.to_string() data-output="te-intensity-out" data-unit="%"/><output id="te-intensity-out">{format!("{}%",d.spec.intensity)}</output></label>
        {if t.accent_adjusted{view!{<div class="theme-warning" role="status">{icon("eye")}<span>{format!("Your accent {} is changed to {} so text stays readable on this background.",hex(chosen),hex(t.palette.accent))}" "<button type="button" class="link-button underline" data-live="appearance:editor-fix-accent">"Use it"</button></span></div>}.into_any()}else{().into_any()}}
        {if t.unmet{view!{<div class="theme-warning" role="alert">{icon("eye-off")}<span>"These colors can’t all reach readable contrast. Lower the intensity or pick other colors."</span></div>}.into_any()}else{().into_any()}}
        <div class="mt-6 flex flex-wrap gap-2">{button(if server{"Save Server Theme"}else{"Save Theme"},"appearance:editor-save","primary")}{if server{().into_any()}else{button("Try It","appearance:editor-apply","secondary")}}{button("Cancel",if server{"dialog:server-theme"}else{"dialog:appearance"},"link-button")}</div>
        {if server{().into_any()}else{view!{<div class="field mt-6"><label for="te-code">"Share Code"</label><div class="flex gap-2"><input id="te-code" class="flex-1 font-mono" type="text" readonly value=code.clone()/><button type="button" class="button secondary" data-copy=format!("{}/?theme={code}",a.origin)>"Copy Link"</button></div><small>"Anyone with this code can preview and use your theme."</small></div>}.into_any()}}
    </div>{preview(&t)}</div>}.into_any()
}
fn preview(t: &Theme) -> AnyView {
    let style = vars(t, "pv");
    view!{<aside class="theme-preview" style=style aria-label="Preview"><div class="pv-head">"# general"</div><div class="pv-body"><div><strong>"Alex"</strong>" "<span class="pv-muted">"Today at 4:12 PM"</span><p>"Does this look readable to everyone?"</p></div><div><strong>"Sam"</strong><p>"Looks great. Here’s the "<span class="pv-link">"link"</span>" you asked for."</p></div><span class="pv-button">"Join Call"</span><div class="pv-composer">"Message #general"</div></div></aside>}.into_any()
}

/// A theme someone shared, before it is applied or saved.
pub(super) fn shared(a: &App) -> AnyView {
    let Some((spec, name)) = a.themes.shared.as_ref() else {
        return view! {<h2>"Shared Theme"</h2><p>"That theme is no longer here."</p>}.into_any();
    };
    let t = derive(spec, th::adjust(&th::prefs(), true));
    let label = if name.is_empty() {
        "Shared Theme".to_owned()
    } else {
        name.clone()
    };
    view!{<h2>{label}</h2><p class="muted-copy">"Someone shared this theme with you. Look it over, then use it or save it to your themes."</p><div class="my-4">{preview(&t)}</div>
        <div class="flex flex-wrap gap-2">{button("Use Theme","appearance:shared-apply","primary")}{button("Save to My Themes","appearance:shared-save","secondary")}{button("Customize","appearance:edit,shared","secondary")}</div>}.into_any()
}

/// Server Settings > Theme (Manage Server).
pub(super) fn server_theme(a: &App) -> AnyView {
    let allowed = settings::can(a, Permissions::MANAGE_GUILD);
    let current = a.guild.and_then(|g| {
        a.themes
            .library
            .as_ref()?
            .servers
            .iter()
            .find(|s| s.guild_id == g)
            .map(|s| s.theme.clone())
    });
    let selected = match &current {
        Some(ThemeChoice::Preset(id)) => id.clone(),
        Some(ThemeChoice::Custom(_)) => "custom".into(),
        None => "none".into(),
    };
    let swatches = preset_swatches();
    let mine = a
        .themes
        .library
        .as_ref()
        .map(|l| l.themes.clone())
        .unwrap_or_default();
    view!{<h2>"Theme"</h2><p class="muted-copy mb-6">"Members see this theme while they’re in this server, unless they turn server themes off in Appearance."</p>
        <fieldset disabled=!allowed><div class="theme-grid" role="radiogroup" aria-label="Server theme">
            <label class="theme-option"><input type="radio" name="server-theme" value="none" data-appearance="server-theme" checked=selected=="none"/><span class="theme-swatch" style=vars(&derive_palette(DARK,true,Adjust::default()),"sw")><i></i><i></i><span><b></b><b></b><b></b></span></span><span class="theme-check">{icon("check")}</span><span>"None (members’ own)"</span></label>
            {PRESETS.iter().zip(swatches).map(|(p,s)|view!{<label class="theme-option"><input type="radio" name="server-theme" value=p.id data-appearance="server-theme" checked=selected==p.id/><span class="theme-swatch" style=s><i></i><i></i><span><b></b><b></b><b></b></span></span><span class="theme-check">{icon("check")}</span><span class="truncate">{p.name}</span></label>}).collect_view()}
            {if let Some(ThemeChoice::Custom(spec))=&current{view!{<label class="theme-option"><input type="radio" name="server-theme" value="custom" checked=true disabled=true/><span class="theme-swatch" style=vars(&derive(spec,Adjust::default()),"sw")><i></i><i></i><span><b></b><b></b><b></b></span></span><span class="theme-check">{icon("check")}</span><span>"Custom"</span></label>}.into_any()}else{().into_any()}}
        </div>
        <div class="mt-6 flex flex-wrap gap-2">{button("Customize","appearance:edit,server","secondary")}</div>
        {if mine.is_empty(){().into_any()}else{view!{<label class="field mt-4"><span>"Use One of My Themes"</span><select data-appearance="server-theme"><option value="" selected=true disabled=true>"Choose a theme"</option>{mine.iter().map(|t|view!{<option value=format!("custom:{}",t.id.0)>{t.name.clone()}</option>}).collect_view()}</select></label>}.into_any()}}
        </fieldset>{if allowed{().into_any()}else{view!{<p class="muted-copy mt-4">"You need Manage Server permission to change the server’s theme."</p>}.into_any()}}
    }.into_any()
}

/// "Speak Message", shown in the message menu (not the hover bar).
pub(super) fn message_actions(m: &Message) -> AnyView {
    view!{<button class="icon-button menu-only" aria-label="Speak Message" title="Speak Message" data-live=format!("appearance:speak,{}",m.id.0)>{icon("volume-2")}</button>}.into_any()
}
/// The style of a role-colored name: its color, and `--rc` for theme adjustment.
pub(super) fn role_style(color: &str) -> String {
    if color.starts_with('#') {
        format!("color:{color};--rc:{color}")
    } else {
        format!("color:{color}")
    }
}

/// User settings pages of ours.
pub(super) fn is_user_page(page: &str) -> bool {
    matches!(page, "accessibility" | "streamer" | "theme-editor")
}
pub(super) fn title(page: &str) -> Option<&'static str> {
    Some(match page {
        "accessibility" => "Accessibility",
        "streamer" => "Streamer Mode",
        "theme-editor" => "Theme",
        "server-theme" | "server-theme-editor" => "Theme",
        "theme-shared" => "Shared Theme",
        _ => return None,
    })
}
pub(super) fn is_server_page(page: &str) -> bool {
    matches!(page, "server-theme" | "server-theme-editor")
}
/// The page body for one of our dialogs.
pub(super) fn page(a: &App, page: &str) -> Option<AnyView> {
    Some(match page {
        "appearance" => appearance(a),
        "accessibility" => accessibility(),
        "streamer" => streamer(),
        "theme-editor" | "server-theme-editor" => editor(a),
        "theme-shared" => shared(a),
        "server-theme" => server_theme(a),
        _ => return None,
    })
}
pub(super) fn settings_nav(a: &App) -> AnyView {
    view! {{settings_nav_row("eye","Accessibility","dialog:accessibility",a.dialog=="accessibility")}{settings_nav_row("video","Streamer Mode","dialog:streamer",a.dialog=="streamer")}}.into_any()
}
