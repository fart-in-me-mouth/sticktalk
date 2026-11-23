//! Themes, appearance and accessibility in the web app (docs/features/themes.md):
//! the chosen theme (base, preset, custom or shared), server themes while a
//! server is in view, the theme editor, share codes, text-to-speech and the
//! Rust side of Streamer Mode. Views are in `ui/themes.rs`; per-device
//! preferences, Streamer Mode and the app icon are in `public/preferences.js`.
//!
//! Tokens are derived once per change with `sticktalk_proto::derive` and handed
//! to `preferences.js`, which swaps one constructed style sheet: changing a
//! theme never re-renders the view. Nothing here polls.
use super::*;
use std::cell::Cell;

#[wasm_bindgen(raw_module = "/public/preferences.js")]
extern "C" {
    #[wasm_bindgen(js_name = preferencesJson)]
    fn prefs_json() -> String;
    #[wasm_bindgen(js_name = applyTheme)]
    fn js_apply_theme(kind: &str, id: &str, css: &str, color: &str, dark: bool, gradient: bool);
    #[wasm_bindgen(js_name = applyServerTheme)]
    fn js_server_theme(css: &str, color: &str, dark: bool, gradient: bool);
    #[wasm_bindgen(js_name = systemLight)]
    fn js_system_light() -> bool;
}
#[wasm_bindgen(raw_module = "/public/speech.js")]
extern "C" {
    #[wasm_bindgen(js_name = speak)]
    fn js_speak(text: &str, rate: f64);
    #[wasm_bindgen(js_name = deafened)]
    fn js_deafened() -> bool;
}

/// The preferences Rust needs (the rest are applied by `preferences.js`).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct Prefs {
    pub theme: String,
    pub theme_id: String,
    pub theme_code: String,
    pub saturation: f64,
    pub saturation_custom: bool,
    pub contrast_boost: bool,
    pub tts_playback: bool,
    pub tts_notify: String,
    pub tts_rate: f64,
    pub streamer: bool,
    pub streamer_popups: bool,
    pub streamer_sounds: bool,
    pub app_icon: String,
}
impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            theme_id: String::new(),
            theme_code: String::new(),
            saturation: 100.0,
            saturation_custom: false,
            contrast_boost: false,
            tts_playback: true,
            tts_notify: "off".into(),
            tts_rate: 1.0,
            streamer: false,
            streamer_popups: true,
            streamer_sounds: true,
            app_icon: "mint".into(),
        }
    }
}
thread_local! {
    static PREFS: RefCell<Prefs> = RefCell::default();
    /// What was last handed to `applyServerTheme`, so it is called only on change.
    static SERVER_APPLIED: RefCell<Option<String>> = const { RefCell::new(None) };
    static LOADED: Cell<bool> = const { Cell::new(false) };
}
pub(super) fn prefs() -> Prefs {
    if !LOADED.with(Cell::get) {
        reload_prefs();
    }
    PREFS.with(|p| p.borrow().clone())
}
fn reload_prefs() {
    let p = serde_json::from_str::<Prefs>(&prefs_json()).unwrap_or_default();
    PREFS.with(|x| *x.borrow_mut() = p);
    LOADED.with(|l| l.set(true));
}
/// The app icon's 192 px image, for the in-app logo.
pub(super) fn logo() -> String {
    let icon = PREFS.with(|p| p.borrow().app_icon.clone());
    if icon.is_empty() || icon == "mint" || !icon.bytes().all(|b| b.is_ascii_lowercase()) {
        "/public/icons/icon-192.png".into()
    } else {
        format!("/public/app-icons/{icon}-192.png")
    }
}

#[derive(Default)]
pub(super) struct State {
    pub library: Option<ThemeLibrary>,
    /// The theme editor's draft: which saved theme it edits (None: a new one).
    pub draft: Option<Draft>,
    /// A theme from a share code or link, being previewed.
    pub shared: Option<(ThemeSpec, String)>,
    pub import_error: Option<String>,
}
#[derive(Clone, Debug)]
pub(super) struct Draft {
    pub id: Option<Id>,
    pub name: String,
    pub spec: ThemeSpec,
    /// Editing the server's theme rather than one of your own.
    pub server: Option<Id>,
}

/// How saturation and contrast boost apply to a theme of this kind.
pub(super) fn adjust(p: &Prefs, custom: bool) -> Adjust {
    Adjust {
        saturation: if custom && !p.saturation_custom {
            1.0
        } else {
            (p.saturation / 100.0).clamp(0.0, 1.0)
        },
        contrast_boost: p.contrast_boost,
    }
}
/// The CSS declarations, browser color, darkness and gradient of a theme.
pub(super) fn painted(t: &Theme, adjust: Adjust) -> (String, String, bool, bool) {
    (
        t.declarations(adjust),
        t.theme_color(),
        t.dark,
        !t.stops.is_empty(),
    )
}
fn base(kind: &str) -> Option<(Palette, bool)> {
    match kind {
        "dark" => Some((DARK, true)),
        "light" => Some((LIGHT, false)),
        "darker" => Some((DARKER, true)),
        "system" => Some(if js_system_light() {
            (LIGHT, false)
        } else {
            (DARK, true)
        }),
        _ => None,
    }
}
/// The spec of a preset, custom or shared selection.
fn selected_spec(a: &App, p: &Prefs) -> Option<ThemeSpec> {
    match p.theme.as_str() {
        "preset" => preset(&p.theme_id).map(Preset::spec),
        "custom" => a.themes.library.as_ref().and_then(|l| {
            l.themes
                .iter()
                .find(|t| t.id.0.to_string() == p.theme_id)
                .map(|t| t.spec.clone())
        }),
        "shared" => parse_share_code(&p.theme_code).ok().map(|(s, _)| s),
        _ => None,
    }
}
/// Derive and apply the personal theme for `kind`/`id` (or the current one).
fn apply_personal(a: &App, kind: &str, id: &str) {
    let p = prefs();
    let mut p2 = p.clone();
    p2.theme = kind.into();
    match kind {
        "shared" => p2.theme_code = id.into(),
        _ => p2.theme_id = id.into(),
    }
    if let Some((palette, dark)) = base(kind) {
        let adj = adjust(&p, false);
        if adj == Adjust::default() {
            // Drawn by the static token sheet (UI_SPEC tables).
            js_apply_theme(kind, "", "", "", dark, false);
        } else {
            let (css, color, dark, gradient) = painted(&derive_palette(palette, dark, adj), adj);
            js_apply_theme(kind, "", &css, &color, dark, gradient);
        }
    } else if let Some(spec) = selected_spec(a, &p2) {
        let adj = adjust(&p, true);
        let (css, color, dark, gradient) = painted(&derive(&spec, adj), adj);
        js_apply_theme(kind, id, &css, &color, dark, gradient);
    } else if kind == "custom" && a.themes.library.is_none() {
        // The library has not arrived yet: keep what is painted.
        return;
    } else {
        js_apply_theme("dark", "", "", "", true, false);
    }
    reload_prefs();
    force_server_sync();
}
/// Apply the current choice again (preferences changed, library arrived).
pub(super) fn refresh(a: &App) {
    reload_prefs();
    let p = prefs();
    let id = if p.theme == "shared" {
        p.theme_code.clone()
    } else {
        p.theme_id.clone()
    };
    apply_personal(a, &p.theme, &id);
}
fn force_server_sync() {
    SERVER_APPLIED.with(|s| *s.borrow_mut() = None);
}
/// Called while rendering: shows the server's theme while that server is in
/// view (and the viewer has not turned server themes off). Cheap when
/// nothing changed; calls into JS only on a change.
pub(super) fn sync(a: &App) {
    let theme = a.guild.and_then(|g| {
        let lib = a.themes.library.as_ref()?;
        lib.settings
            .shows(g)
            .then(|| lib.servers.iter().find(|s| s.guild_id == g))
            .flatten()
            .map(|s| s.theme.clone())
    });
    let key = theme.as_ref().map(|t| format!("{t:?}")).unwrap_or_default();
    if SERVER_APPLIED.with(|s| s.borrow().as_deref() == Some(key.as_str())) {
        return;
    }
    SERVER_APPLIED.with(|s| *s.borrow_mut() = Some(key));
    match theme.and_then(|t| t.spec()) {
        Some(spec) => {
            let p = prefs();
            let adj = adjust(&p, true);
            let (css, color, dark, gradient) = painted(&derive(&spec, adj), adj);
            js_server_theme(&css, &color, dark, gradient);
        }
        None => js_server_theme("", "", true, false),
    }
}

/// After every Ready: opt in to theme events; the answer is the library.
pub(super) fn connected(app: Shared) {
    let mut a = app.borrow_mut();
    a.command(Command::Appearance(AppearanceCommand::Subscribe));
    // A theme link opened before signing in (`?theme=<code>`).
    if a.themes.shared.is_none() {
        if let Some(code) = shared_from_location() {
            if let Ok(shared) = parse_share_code(&code) {
                a.themes.shared = Some(shared);
                a.dialog = "theme-shared".into();
                a.dialog_generation = a.dialog_generation.wrapping_add(1);
                a.touch();
            }
        }
    }
}
fn shared_from_location() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let code = search
        .trim_start_matches('?')
        .split('&')
        .find_map(|p| p.strip_prefix("theme="))?
        .to_owned();
    // Take it out of the address so a reload does not ask again.
    if let Ok(history) = web_sys::window()?.history() {
        let path = web_sys::window()?.location().pathname().ok()?;
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&path));
    }
    Some(code)
}
fn library_changed(a: &mut App, lib: ThemeLibrary) {
    let first = a.themes.library.is_none();
    a.themes.library = Some(lib);
    force_server_sync();
    // A custom theme may have changed, or arrived for the first time.
    if first || prefs().theme == "custom" {
        refresh(a);
    }
}
pub(super) fn event(a: &mut App, e: &AppearanceEvent) {
    match e {
        AppearanceEvent::Library(lib) => library_changed(a, (**lib).clone()),
        AppearanceEvent::ServerTheme { guild_id, theme } => {
            if let Some(lib) = a.themes.library.as_mut() {
                lib.servers.retain(|s| s.guild_id != *guild_id);
                if let Some(t) = theme {
                    lib.servers.push(ServerTheme {
                        guild_id: *guild_id,
                        theme: t.clone(),
                    });
                }
            }
            force_server_sync();
        }
    }
}
/// A command of ours was answered.
pub(super) fn answered(a: &mut App, op: &AppearanceCommand, result: &CommandResult) {
    let CommandResult::Appearance(AppearanceResult::Library(lib)) = result else {
        return;
    };
    let before: Vec<Id> = a
        .themes
        .library
        .as_ref()
        .map(|l| l.themes.iter().map(|t| t.id).collect())
        .unwrap_or_default();
    library_changed(a, (**lib).clone());
    match op {
        AppearanceCommand::SaveTheme { id, .. } => {
            // A new theme is applied at once; an edited one if it was in use.
            let saved = id.or_else(|| lib.themes.iter().map(|t| t.id).find(|t| !before.contains(t)));
            if let Some(saved) = saved {
                apply_personal(a, "custom", &saved.0.to_string());
            }
            a.themes.draft = None;
            a.themes.shared = None;
            a.dialog = "appearance".into();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
            a.notice = "Theme saved.".into();
            a.error = a.notice.clone();
        }
        AppearanceCommand::DeleteTheme { id } => {
            let p = prefs();
            if p.theme == "custom" && p.theme_id == id.0.to_string() {
                apply_personal(a, "dark", "");
            }
        }
        AppearanceCommand::SetServerTheme { .. } => {
            a.themes.draft = None;
            a.dialog = "server-theme".into();
            a.notice = "Server theme saved.".into();
            a.error = a.notice.clone();
        }
        _ => {}
    }
}
/// The server refused one of our commands: say why in the toast.
pub(super) fn refused(_c: &AppearanceCommand, code: ErrorCode) -> Option<&'static str> {
    Some(match code {
        ErrorCode::LimitExceeded => "You can save up to 20 custom themes. Delete one first.",
        ErrorCode::Forbidden => "You need permission for that here.",
        ErrorCode::InvalidRequest => "That theme isn't valid.",
        _ => return None,
    })
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
fn checked_radio(name: &str) -> String {
    document()
        .query_selector(&format!("input[name='{name}']:checked"))
        .ok()
        .flatten()
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|e| e.value())
        .unwrap_or_default()
}
/// Read the editor's fields into the draft.
fn read_draft(d: &mut Draft) {
    let name = read("te-name");
    if !name.is_empty() || document().get_element_by_id("te-name").is_some() {
        d.name = name.chars().take(MAX_THEME_NAME_CHARS).collect();
    }
    if let Some(c) = parse_hex(&read("te-accent")) {
        d.spec.accent = c;
    }
    let colors: Vec<u32> = (0..MAX_THEME_COLORS)
        .filter_map(|i| parse_hex(&read(&format!("te-color-{i}"))))
        .collect();
    if document().get_element_by_id("te-color-0").is_some() || colors.is_empty() {
        d.spec.colors = colors;
    }
    if let Ok(angle) = read("te-angle").parse::<u16>() {
        d.spec.angle = angle.min(359);
    }
    if let Ok(i) = read("te-intensity").parse::<u8>() {
        d.spec.intensity = i.min(100);
    }
    d.spec.base = match checked_radio("te-base").as_str() {
        "light" => ThemeBase::Light,
        "dark" => ThemeBase::Dark,
        _ => d.spec.base,
    };
}
fn open(a: &mut App, page: &str) {
    a.dialog = page.into();
    a.dialog_generation = a.dialog_generation.wrapping_add(1);
    a.error.clear();
    a.touch();
}

/// `appearance:<what>,<args>` actions from the views and `preferences.js`.
pub(super) fn action(app: Shared, arg: &str) {
    let (what, rest) = arg.split_once(',').unwrap_or((arg, ""));
    match what {
        // A theme radio: "dark", "light", "darker", "system", "preset:id", "custom:id".
        "theme" => {
            let (kind, id) = rest.split_once(':').unwrap_or((rest, ""));
            let a = app.borrow();
            apply_personal(&a, kind, id);
            // No render: the radio already shows the choice.
        }
        "refresh" => {
            let a = app.borrow();
            refresh(&a);
        }
        "icon" => {
            reload_prefs();
            app.borrow().touch();
        }
        "edit" => {
            let mut a = app.borrow_mut();
            let draft = match rest {
                "new" => Some(Draft {
                    id: None,
                    name: "My Theme".into(),
                    spec: ThemeSpec {
                        base: ThemeBase::Dark,
                        accent: 0x3DDC97,
                        colors: vec![0x1B4D3E, 0x2A2F6B],
                        angle: 135,
                        intensity: 70,
                    },
                    server: None,
                }),
                "shared" => a.themes.shared.clone().map(|(spec, name)| Draft {
                    id: None,
                    name: if name.is_empty() { "Shared Theme".into() } else { name },
                    spec,
                    server: None,
                }),
                "server" => {
                    let g = a.guild;
                    let current = g.and_then(|g| {
                        a.themes
                            .library
                            .as_ref()?
                            .servers
                            .iter()
                            .find(|s| s.guild_id == g)
                            .and_then(|s| s.theme.spec())
                    });
                    g.map(|g| Draft {
                        id: None,
                        name: String::new(),
                        spec: current.unwrap_or_else(|| PRESETS[0].spec()),
                        server: Some(g),
                    })
                }
                other => {
                    if let Some(p) = other.strip_prefix("preset:").and_then(preset) {
                        Some(Draft {
                            id: None,
                            name: format!("{} Copy", p.name),
                            spec: p.spec(),
                            server: None,
                        })
                    } else {
                        let id = other.strip_prefix("custom:").unwrap_or(other);
                        a.themes.library.as_ref().and_then(|l| {
                            l.themes.iter().find(|t| t.id.0.to_string() == id).map(|t| Draft {
                                id: Some(t.id),
                                name: t.name.clone(),
                                spec: t.spec.clone(),
                                server: None,
                            })
                        })
                    }
                }
            };
            let Some(draft) = draft else { return };
            let server = draft.server.is_some();
            a.themes.draft = Some(draft);
            open(&mut a, if server { "server-theme-editor" } else { "theme-editor" });
        }
        // An editor field changed: derive again for the preview.
        "editor-input" => {
            let mut a = app.borrow_mut();
            if let Some(d) = a.themes.draft.as_mut() {
                read_draft(d);
            }
            a.touch();
        }
        "editor-color-add" => {
            let mut a = app.borrow_mut();
            if let Some(d) = a.themes.draft.as_mut() {
                read_draft(d);
                if d.spec.colors.len() < MAX_THEME_COLORS {
                    let last = d.spec.colors.last().copied().unwrap_or(0x2A2F6B);
                    d.spec.colors.push(last);
                }
            }
            a.touch();
        }
        "editor-color-remove" => {
            let mut a = app.borrow_mut();
            if let (Some(d), Ok(i)) = (a.themes.draft.as_mut(), rest.parse::<usize>()) {
                read_draft(d);
                if i < d.spec.colors.len() {
                    d.spec.colors.remove(i);
                }
            }
            a.touch();
        }
        // Use the editor's adjusted accent (the one that meets contrast).
        "editor-fix-accent" => {
            let mut a = app.borrow_mut();
            let p = prefs();
            if let Some(d) = a.themes.draft.as_mut() {
                read_draft(d);
                d.spec.accent = derive(&d.spec, adjust(&p, true)).palette.accent;
            }
            a.touch();
        }
        "editor-save" => {
            let mut a = app.borrow_mut();
            let Some(mut d) = a.themes.draft.clone() else { return };
            read_draft(&mut d);
            if let Some(g) = d.server {
                a.command(Command::Appearance(AppearanceCommand::SetServerTheme {
                    guild_id: g,
                    theme: Some(ThemeChoice::Custom(d.spec)),
                }));
                return;
            }
            match validate_name(&d.name) {
                Err(e) => {
                    a.error = e.into();
                    a.touch();
                }
                Ok(name) => {
                    a.command(Command::Appearance(AppearanceCommand::SaveTheme {
                        id: d.id,
                        name,
                        spec: d.spec,
                    }));
                }
            }
        }
        // Try the draft as your theme without saving it.
        "editor-apply" => {
            let mut a = app.borrow_mut();
            let Some(mut d) = a.themes.draft.clone() else { return };
            read_draft(&mut d);
            let code = share_code(&d.spec, &d.name);
            apply_personal(&a, "shared", &code);
            a.themes.draft = Some(d);
            a.notice = "Applied. Save it to keep it on all your devices.".into();
            a.error = a.notice.clone();
            a.touch();
        }
        "delete" => {
            if let Ok(id) = rest.parse::<u64>() {
                app.borrow_mut()
                    .command(Command::Appearance(AppearanceCommand::DeleteTheme { id: Id(id) }));
            }
        }
        // Paste a code or link.
        "import" => {
            let mut a = app.borrow_mut();
            match parse_share_code(&read("theme-code")) {
                Ok(shared) => {
                    a.themes.shared = Some(shared);
                    a.themes.import_error = None;
                    open(&mut a, "theme-shared");
                }
                Err(e) => {
                    a.themes.import_error = Some(e.into());
                    a.touch();
                }
            }
        }
        "shared-apply" => {
            let mut a = app.borrow_mut();
            if let Some((spec, name)) = a.themes.shared.clone() {
                apply_personal(&a, "shared", &share_code(&spec, &name));
                a.themes.shared = None;
                open(&mut a, "appearance");
            }
        }
        "shared-save" => {
            let mut a = app.borrow_mut();
            if let Some((spec, name)) = a.themes.shared.clone() {
                let name = validate_name(&name).unwrap_or_else(|_| "Shared Theme".into());
                a.command(Command::Appearance(AppearanceCommand::SaveTheme {
                    id: None,
                    name,
                    spec,
                }));
            }
        }
        // Server settings: a preset, or none.
        "server-theme" => {
            let mut a = app.borrow_mut();
            let Some(g) = a.guild else { return };
            let theme = match rest {
                "none" => None,
                id if preset(id).is_some() => Some(ThemeChoice::Preset(id.into())),
                custom => {
                    let id = custom.strip_prefix("custom:").unwrap_or_default();
                    let found = a
                        .themes
                        .library
                        .as_ref()
                        .and_then(|l| l.themes.iter().find(|t| t.id.0.to_string() == id))
                        .map(|t| ThemeChoice::Custom(t.spec.clone()));
                    match found {
                        Some(t) => Some(t),
                        None => return,
                    }
                }
            };
            a.command(Command::Appearance(AppearanceCommand::SetServerTheme {
                guild_id: g,
                theme,
            }));
        }
        // "Show server themes": the default, or one server's choice.
        "server-themes" => {
            let mut a = app.borrow_mut();
            let mut s = a
                .themes
                .library
                .as_ref()
                .map(|l| l.settings.clone())
                .unwrap_or_default();
            let (target, value) = rest.split_once(',').unwrap_or((rest, ""));
            if target == "default" {
                s.server_themes = value == "true";
            } else if let Ok(g) = target.parse::<u64>() {
                s.server_overrides.retain(|o| o.guild_id != Id(g));
                if value != "default" {
                    s.server_overrides.push(ServerThemeOverride {
                        guild_id: Id(g),
                        show: value == "show",
                    });
                }
            }
            a.command(Command::Appearance(AppearanceCommand::SetSettings(s)));
        }
        "speak" => {
            let a = app.borrow();
            let Ok(m) = rest.parse::<u64>() else { return };
            let found = a
                .model
                .messages
                .values()
                .find_map(|ms| ms.get(&Id(m)))
                .cloned();
            if let Some(m) = found {
                let text = format!("{} said {}", a.model.name(m.author_id), spoken(&a.model.content(&m)));
                js_speak(&text, prefs().tts_rate);
            }
        }
        "tts-preview" => {
            reload_prefs();
            js_speak(
                "This is how messages will sound when they are read aloud.",
                prefs().tts_rate,
            );
        }
        _ => {}
    }
}

/// A message as it is read aloud: custom emoji by name, no markup, bounded.
pub(super) fn spoken(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(400));
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        match tail.find('>') {
            Some(end) if tail[..end].matches(':').count() == 2 => {
                let inner = &tail[1..end];
                let name = inner.split(':').nth(1).unwrap_or_default();
                out.push_str(name);
                rest = &tail[end + 1..];
            }
            _ => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    let plain: String = out
        .replace("||", " ")
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '`' | '~' | '#' | '>'))
        .collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect()
}
/// Speaking is never done in Do Not Disturb, or while deafened in a call.
fn quiet(a: &App) -> bool {
    super::privacy::status(a) == PresenceStatus::Dnd || (a.voice_channel.is_some() && js_deafened())
}
/// A message arrived: read `/tts` messages to people viewing the channel, and
/// (with TTS notifications) every message in the channel in view.
pub(super) fn message_created(a: &App, m: &Message) {
    if a.model.me.as_ref().is_some_and(|me| me.user.id == m.author_id) || a.channel != Some(m.channel_id) {
        return;
    }
    let p = prefs();
    let wanted = (m.extra.tts && p.tts_playback) || matches!(p.tts_notify.as_str(), "channel" | "all");
    if !wanted || quiet(a) || document().hidden() {
        return;
    }
    let text = spoken(&a.model.content(m));
    if text.is_empty() {
        return;
    }
    js_speak(&format!("{} said {}", a.model.name(m.author_id), text), p.tts_rate);
}
/// An alert for another conversation (privacy.rs): read it with TTS
/// notifications for all channels.
pub(super) fn alert_spoken(a: &App, channel: Id, title: &str, body: &str) {
    let p = prefs();
    if p.tts_notify != "all" || a.channel == Some(channel) || quiet(a) {
        return;
    }
    js_speak(&format!("{title}: {}", spoken(body)), p.tts_rate);
}
/// Notification popups and sounds, with Streamer Mode's choices applied.
pub(super) fn alerts_allowed() -> (bool, bool) {
    let p = prefs();
    (
        !(p.streamer && p.streamer_popups),
        !(p.streamer && p.streamer_sounds),
    )
}
/// `/tts message` in the composer sends a message read aloud.
pub(super) fn tts_command(cmd: Command) -> Command {
    match cmd {
        Command::SendMessage(mut d) if d.content.starts_with("/tts ") && d.attachment_ids.is_empty() => {
            d.content = d.content["/tts ".len()..].trim_start().to_owned();
            Command::Appearance(AppearanceCommand::SendTts(d))
        }
        other => other,
    }
}
