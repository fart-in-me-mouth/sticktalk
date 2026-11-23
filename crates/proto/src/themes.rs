//! Themes, appearance and accessibility (task `themes`, see
//! `docs/features/themes.md`).
//!
//! Two halves:
//! - **Wire contracts**: custom themes saved on the account, server themes,
//!   the server-theme opt-in, and messages read aloud (`/tts`). Commands
//!   travel as `Command::Appearance`; live changes as `Event::Appearance`,
//!   which the server sends only to streams that sent
//!   `AppearanceCommand::Subscribe`, so older clients never see it.
//! - **The theme engine**: the base palettes (the same values as the tables in
//!   `docs/UI_SPEC.md`; a test keeps them equal), the preset themes, and the
//!   one derivation that turns a few key colors into every token with WCAG AA
//!   contrast enforced. The server uses it to validate, the web app to paint,
//!   and the contrast report (`cargo test -p sticktalk-proto themes -- --nocapture`)
//!   to prove every theme meets AA.
use crate::*;
use serde::{Deserialize, Serialize};

pub const MAX_CUSTOM_THEMES: usize = 20;
pub const MAX_THEME_COLORS: usize = 5;
pub const MAX_THEME_NAME_CHARS: usize = 32;
/// Per-server overrides of "show server themes" kept for one account.
pub const MAX_SERVER_THEME_OVERRIDES: usize = MAX_GUILDS_PER_USER;
/// The longest share code accepted (a theme with five colors and a 32
/// character name in four-byte characters is shorter than this).
pub const MAX_THEME_CODE_CHARS: usize = 256;

// ---------------------------------------------------------------------------
// Wire contracts
// ---------------------------------------------------------------------------

/// Which palette a custom theme is built on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeBase {
    #[default]
    Dark,
    Light,
}

/// A theme as the person chose it: a base, an accent, up to five background
/// gradient colors, the gradient's direction and how strongly it tints.
/// Everything else is derived (`derive`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThemeSpec {
    #[serde(default)]
    pub base: ThemeBase,
    /// 0xRRGGBB.
    pub accent: u32,
    /// 0xRRGGBB each, 0 to 5 of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<u32>,
    /// Gradient direction in degrees, 0 to 359 (CSS `linear-gradient`).
    #[serde(default)]
    pub angle: u16,
    /// How strongly the gradient tints the surfaces, 0 to 100.
    #[serde(default)]
    pub intensity: u8,
}

/// A named custom theme saved on the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomTheme {
    pub id: Id,
    pub name: String,
    pub spec: ThemeSpec,
}

/// What a server admin chose for the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ThemeChoice {
    /// One of `PRESETS`, by id.
    Preset(String),
    Custom(ThemeSpec),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerTheme {
    pub guild_id: GuildId,
    pub theme: ThemeChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerThemeOverride {
    pub guild_id: GuildId,
    pub show: bool,
}

/// Whether this person sees server themes: a default, and per-server choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeSettings {
    #[serde(default = "yes")]
    pub server_themes: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_overrides: Vec<ServerThemeOverride>,
}
fn yes() -> bool {
    true
}
impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            server_themes: true,
            server_overrides: vec![],
        }
    }
}
impl ThemeSettings {
    /// Whether server themes show in this server.
    pub fn shows(&self, guild: GuildId) -> bool {
        self.server_overrides
            .iter()
            .find(|o| o.guild_id == guild)
            .map_or(self.server_themes, |o| o.show)
    }
}

/// Everything about themes that belongs to one account, as of now.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThemeLibrary {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<CustomTheme>,
    #[serde(default)]
    pub settings: ThemeSettings,
    /// The theme of each server this person is in that has one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<ServerTheme>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AppearanceCommand {
    /// Opt this stream in to `Event::Appearance`; answers `Library`.
    Subscribe,
    /// Create (`id` None) or replace one of your custom themes.
    SaveTheme {
        #[serde(default)]
        id: Option<Id>,
        name: String,
        spec: ThemeSpec,
    },
    DeleteTheme {
        id: Id,
    },
    /// Manage Server. `None` removes the server's theme.
    SetServerTheme {
        guild_id: GuildId,
        #[serde(default)]
        theme: Option<ThemeChoice>,
    },
    SetSettings(ThemeSettings),
    /// Send a message that is read aloud to the people viewing the channel
    /// (Send TTS Messages). Answers `CommandResult::Message`.
    SendTts(MessageDraft),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AppearanceResult {
    Library(Box<ThemeLibrary>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AppearanceEvent {
    /// Your themes or settings changed on another device.
    Library(Box<ThemeLibrary>),
    /// A server you are in changed its theme (`None`: removed).
    ServerTheme {
        guild_id: GuildId,
        #[serde(default)]
        theme: Option<ThemeChoice>,
    },
    /// A `/tts` message: read it aloud if you are looking at its conversation.
    /// Sent with (right after) its `MessageCreated`, to the same people.
    Tts {
        channel_id: ChannelId,
        message_id: MessageId,
    },
}

/// Why a theme was refused.
pub fn validate_name(name: &str) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A theme needs a name.");
    }
    if name.chars().count() > MAX_THEME_NAME_CHARS {
        return Err("Theme names are at most 32 characters.");
    }
    if name.chars().any(char::is_control) {
        return Err("Theme names cannot contain control characters.");
    }
    Ok(name.to_owned())
}
impl ThemeSpec {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.colors.len() > MAX_THEME_COLORS {
            return Err("A theme has at most five background colors.");
        }
        if self.accent > 0xFF_FFFF || self.colors.iter().any(|c| *c > 0xFF_FFFF) {
            return Err("Colors are six-digit hex values.");
        }
        if self.angle >= 360 {
            return Err("The angle is 0 to 359 degrees.");
        }
        if self.intensity > 100 {
            return Err("Intensity is 0 to 100.");
        }
        Ok(())
    }
}
impl ThemeChoice {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            ThemeChoice::Preset(id) => preset(id).map(|_| ()).ok_or("That theme does not exist."),
            ThemeChoice::Custom(spec) => spec.validate(),
        }
    }
    pub fn spec(&self) -> Option<ThemeSpec> {
        match self {
            ThemeChoice::Preset(id) => preset(id).map(|p| p.spec()),
            ThemeChoice::Custom(spec) => Some(spec.clone()),
        }
    }
}

// ---------------------------------------------------------------------------
// Color math: sRGB, WCAG 2 contrast, OKLab / OKLCH
// ---------------------------------------------------------------------------

fn channel(c: u32, shift: u32) -> f64 {
    f64::from((c >> shift) & 0xFF) / 255.0
}
fn to_linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn from_linear(v: f64) -> f64 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}
/// WCAG 2 relative luminance of 0xRRGGBB.
pub fn wcag_luminance(c: u32) -> f64 {
    0.2126 * to_linear(channel(c, 16))
        + 0.7152 * to_linear(channel(c, 8))
        + 0.0722 * to_linear(channel(c, 0))
}
/// WCAG 2 contrast ratio, 1 to 21.
pub fn wcag_contrast(a: u32, b: u32) -> f64 {
    let (x, y) = (wcag_luminance(a), wcag_luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}
fn pack(r: f64, g: f64, b: f64) -> u32 {
    let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (q(r) << 16) | (q(g) << 8) | q(b)
}
/// `a` over `b` with `a` at `alpha` (0 to 1), in sRGB as browsers blend.
pub fn mix(a: u32, b: u32, alpha: f64) -> u32 {
    let m = |s| channel(a, s) * alpha + channel(b, s) * (1.0 - alpha);
    pack(m(16), m(8), m(0))
}
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lch {
    l: f64,
    c: f64,
    h: f64,
}
fn oklab(c: u32) -> (f64, f64, f64) {
    let (r, g, b) = (
        to_linear(channel(c, 16)),
        to_linear(channel(c, 8)),
        to_linear(channel(c, 0)),
    );
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    (
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    )
}
/// Linear sRGB of an OKLab color (may be out of gamut).
fn oklab_linear(l: f64, a: f64, b: f64) -> (f64, f64, f64) {
    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    (
        4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_,
        -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701 * s_,
    )
}
fn lch(c: u32) -> Lch {
    let (l, a, b) = oklab(c);
    Lch {
        l,
        c: (a * a + b * b).sqrt(),
        h: b.atan2(a),
    }
}
/// The sRGB color nearest an OKLCH one: chroma is reduced until it fits.
fn from_lch(x: Lch) -> u32 {
    let l = x.l.clamp(0.0, 1.0);
    let fits = |c: f64| {
        let (r, g, b) = oklab_linear(l, c * x.h.cos(), c * x.h.sin());
        [r, g, b].iter().all(|v| (-1e-6..=1.0 + 1e-6).contains(v))
    };
    let mut c = x.c.max(0.0);
    if !fits(c) {
        let (mut lo, mut hi) = (0.0, c);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if fits(mid) {
                lo = mid
            } else {
                hi = mid
            }
        }
        c = lo;
    }
    let (r, g, b) = oklab_linear(l, c * x.h.cos(), c * x.h.sin());
    pack(from_linear(r), from_linear(g), from_linear(b))
}
/// Scale a color's chroma (saturation 0 to 1).
pub fn saturate(c: u32, amount: f64) -> u32 {
    if amount >= 0.999 {
        return c;
    }
    let mut x = lch(c);
    x.c *= amount.clamp(0.0, 1.0);
    from_lch(x)
}
/// Move `fg`'s lightness away from `bgs` until it reaches `min` contrast
/// with every one of them (or the end of the range). Returns the color and
/// whether it reached `min`.
fn enforce(fg: u32, bgs: &[u32], min: f64, lighter: bool) -> (u32, bool) {
    let worst = |c: u32| {
        bgs.iter()
            .map(|b| wcag_contrast(c, *b))
            .fold(f64::MAX, f64::min)
    };
    if worst(fg) >= min {
        return (fg, true);
    }
    let mut x = lch(fg);
    for _ in 0..120 {
        x.l += if lighter { 0.01 } else { -0.01 };
        let c = from_lch(x);
        if worst(c) >= min {
            return (c, true);
        }
        if !(0.0..=1.0).contains(&x.l) {
            break;
        }
    }
    let end = if lighter { 0xFF_FFFF } else { 0 };
    (end, worst(end) >= min)
}

// ---------------------------------------------------------------------------
// Palettes and tokens
// ---------------------------------------------------------------------------

/// Every color token. CSS custom property names are the field names with
/// `_` as `-` and the prefixes shown in `TOKEN_NAMES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub rail: u32,
    pub sidebar: u32,
    pub chat: u32,
    pub elevated: u32,
    pub hover: u32,
    pub selected: u32,
    pub divider: u32,
    pub panel: u32,
    pub field: u32,
    pub control: u32,
    pub control_hover: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_muted: u32,
    pub accent: u32,
    pub accent_hover: u32,
    pub accent_text: u32,
    pub danger: u32,
    pub warning: u32,
    /// `mention.bg` is `warning` at this percent alpha.
    pub mention_alpha: u8,
    pub online: u32,
    pub idle: u32,
    pub dnd: u32,
    pub offline: u32,
    pub brand_avatar: u32,
}

/// `(UI_SPEC key, CSS custom property)` in the order the tables list them.
pub const TOKEN_NAMES: [(&str, &str); 24] = [
    ("bg.rail", "--bg-rail"),
    ("bg.sidebar", "--bg-sidebar"),
    ("bg.chat", "--bg-chat"),
    ("bg.elevated", "--bg-elevated"),
    ("bg.hover", "--bg-hover"),
    ("bg.selected", "--bg-selected"),
    ("divider", "--divider"),
    ("bg.panel", "--bg-panel"),
    ("bg.field", "--bg-field"),
    ("bg.control", "--bg-control"),
    ("bg.control-hover", "--bg-control-hover"),
    ("text.primary", "--text-primary"),
    ("text.secondary", "--text-secondary"),
    ("text.muted", "--text-muted"),
    ("accent", "--accent"),
    ("accent.hover", "--accent-hover"),
    ("accent.text", "--accent-text"),
    ("danger", "--danger"),
    ("warning", "--warning"),
    ("online", "--online"),
    ("idle", "--idle"),
    ("dnd", "--dnd"),
    ("offline", "--offline"),
    ("brand.avatar", "--brand-avatar"),
];
impl Palette {
    /// The token values in `TOKEN_NAMES` order.
    pub fn values(&self) -> [u32; 24] {
        [
            self.rail,
            self.sidebar,
            self.chat,
            self.elevated,
            self.hover,
            self.selected,
            self.divider,
            self.panel,
            self.field,
            self.control,
            self.control_hover,
            self.text_primary,
            self.text_secondary,
            self.text_muted,
            self.accent,
            self.accent_hover,
            self.accent_text,
            self.danger,
            self.warning,
            self.online,
            self.idle,
            self.dnd,
            self.offline,
            self.brand_avatar,
        ]
    }
    /// The mention highlight as it is painted on the chat surface.
    pub fn mention_on_chat(&self) -> u32 {
        mix(
            self.warning,
            self.chat,
            f64::from(self.mention_alpha) / 100.0,
        )
    }
    /// A mention chip (accent at 20 %) on the chat surface.
    pub fn chip_on_chat(&self) -> u32 {
        mix(self.accent, self.chat, 0.2)
    }
}

/// The default dark palette (`docs/UI_SPEC.md` "Color (dark, default)").
/// `text_muted` is the lead-approved AA override (#A0A0AA) that the web token
/// sheet applies over the table's #74747C.
pub const DARK: Palette = Palette {
    rail: 0x111113,
    sidebar: 0x17171A,
    chat: 0x1C1C20,
    elevated: 0x26262B,
    hover: 0x232327,
    selected: 0x2E2E34,
    divider: 0x2A2A2F,
    panel: 0x141417,
    field: 0x121215,
    control: 0x3A3A41,
    control_hover: 0x45454D,
    text_primary: 0xE8EAED,
    text_secondary: 0xA8A8B0,
    text_muted: 0xA0A0AA,
    accent: 0x3DDC97,
    accent_hover: 0x5BE5AA,
    accent_text: 0x08170F,
    danger: 0xF25F5C,
    warning: 0xF2B84B,
    mention_alpha: 8,
    online: 0x3DDC97,
    idle: 0xF2B84B,
    dnd: 0xF25F5C,
    offline: 0x74747C,
    brand_avatar: 0x496C63,
};
/// `docs/UI_SPEC.md` "Color (light)".
pub const LIGHT: Palette = Palette {
    rail: 0xE3E5E8,
    sidebar: 0xF2F3F5,
    chat: 0xFFFFFF,
    elevated: 0xEBEDEF,
    hover: 0xF0F1F3,
    selected: 0xE1E3E7,
    divider: 0xD9DBDF,
    panel: 0xE8EAED,
    field: 0xFFFFFF,
    control: 0xC9CDD3,
    control_hover: 0xB9BEC6,
    text_primary: 0x1F2023,
    text_secondary: 0x45474D,
    text_muted: 0x5A5D66,
    accent: 0x06603F,
    accent_hover: 0x054E33,
    accent_text: 0xFFFFFF,
    danger: 0xB83232,
    warning: 0x8A5A00,
    mention_alpha: 12,
    online: 0x1F8A50,
    idle: 0xB07A0A,
    dnd: 0xD23F3F,
    offline: 0x80848E,
    brand_avatar: 0x9CC9BC,
};
/// `docs/UI_SPEC.md` "Color (darker)": near-black surfaces for OLED screens.
pub const DARKER: Palette = Palette {
    rail: 0x000000,
    sidebar: 0x09090A,
    chat: 0x000000,
    elevated: 0x161618,
    hover: 0x121214,
    selected: 0x1E1E22,
    divider: 0x1C1C20,
    panel: 0x060607,
    field: 0x0B0B0D,
    control: 0x2E2E34,
    control_hover: 0x3A3A41,
    text_primary: 0xE8EAED,
    text_secondary: 0xA8A8B0,
    text_muted: 0xA0A0AA,
    accent: 0x3DDC97,
    accent_hover: 0x5BE5AA,
    accent_text: 0x08170F,
    danger: 0xF25F5C,
    warning: 0xF2B84B,
    mention_alpha: 10,
    online: 0x3DDC97,
    idle: 0xF2B84B,
    dnd: 0xF25F5C,
    offline: 0x74747C,
    brand_avatar: 0x496C63,
};

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// A named preset: two or three key colors and an accent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub base: ThemeBase,
    pub accent: u32,
    pub colors: &'static [u32],
    pub angle: u16,
    pub intensity: u8,
}
impl Preset {
    pub fn spec(&self) -> ThemeSpec {
        ThemeSpec {
            base: self.base,
            accent: self.accent,
            colors: self.colors.to_vec(),
            angle: self.angle,
            intensity: self.intensity,
        }
    }
}
const fn dark(
    id: &'static str,
    name: &'static str,
    accent: u32,
    colors: &'static [u32],
    angle: u16,
) -> Preset {
    Preset {
        id,
        name,
        base: ThemeBase::Dark,
        accent,
        colors,
        angle,
        intensity: 80,
    }
}
const fn light(
    id: &'static str,
    name: &'static str,
    accent: u32,
    colors: &'static [u32],
    angle: u16,
) -> Preset {
    Preset {
        id,
        name,
        base: ThemeBase::Light,
        accent,
        colors,
        angle,
        intensity: 90,
    }
}
/// The preset themes, dark-based first. Ids are stable (saved by servers).
pub const PRESETS: [Preset; 22] = [
    dark(
        "aurora",
        "Aurora",
        0x6EE7B7,
        &[0x1B4D3E, 0x2A2F6B, 0x5B2A6B],
        135,
    ),
    dark(
        "midnight",
        "Midnight Blue",
        0x60A5FA,
        &[0x0F1C3D, 0x1E3A6B],
        160,
    ),
    dark("ember", "Ember", 0xFB923C, &[0x3D1308, 0x6B2A12], 135),
    dark("grape", "Grape", 0xC084FC, &[0x2A1440, 0x4A2370], 145),
    dark(
        "forest",
        "Forest",
        0x4ADE80,
        &[0x0F2A1A, 0x1F4D2E, 0x2E5A3A],
        180,
    ),
    dark(
        "crimson",
        "Crimson Dusk",
        0xFB7185,
        &[0x3A0D1A, 0x5E1A2E],
        135,
    ),
    dark("ocean", "Ocean Deep", 0x22D3EE, &[0x062A3A, 0x0E4A5E], 200),
    dark(
        "sunset",
        "Sunset",
        0xFDBA74,
        &[0x3D1A0F, 0x5E1F3A, 0x2A1440],
        120,
    ),
    dark("steel", "Steel", 0x93C5FD, &[0x1A2028, 0x2A3340], 180),
    dark("neon", "Neon Night", 0xF0ABFC, &[0x12002A, 0x002A2A], 135),
    dark("coffee", "Coffee", 0xE0B084, &[0x2A1E14, 0x3D2A1C], 160),
    dark(
        "galaxy",
        "Galaxy",
        0xA5B4FC,
        &[0x0B0B2A, 0x2A0B3D, 0x0B2A3D],
        135,
    ),
    light(
        "mint",
        "Daylight Mint",
        0x08704A,
        &[0xBFEBD6, 0xE8FAF1],
        135,
    ),
    light("sky", "Sky", 0x1D4ED8, &[0xC9DEFF, 0xEAF2FF], 160),
    light("peach", "Peach", 0xC2410C, &[0xFFD3BA, 0xFFEFE4], 135),
    light("lavender", "Lavender", 0x6D28D9, &[0xDDCCFF, 0xF3EDFF], 145),
    light("rose", "Rose Garden", 0xBE123C, &[0xFFCCD9, 0xFFEEF2], 135),
    light("lemonade", "Lemonade", 0x8A5A00, &[0xFFF0A8, 0xD9F0FF], 120),
    light("sage", "Sage", 0x3F6212, &[0xD3E4CF, 0xF0F6EE], 180),
    light(
        "candy",
        "Cotton Candy",
        0x9D174D,
        &[0xFFCCE6, 0xCCE4FF],
        135,
    ),
    light("sand", "Sand", 0x92400E, &[0xEBD9BF, 0xFAF3E8], 160),
    light(
        "glacier",
        "Glacier",
        0x0E7490,
        &[0xC4ECF2, 0xE6F7FA, 0xDDE3FF],
        135,
    ),
];
pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

// ---------------------------------------------------------------------------
// Derivation
// ---------------------------------------------------------------------------

/// Viewer adjustments applied on top of any theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjust {
    /// 0 to 1: chroma of accents, status colors and tints.
    pub saturation: f64,
    /// Raises text targets to AAA (7:1) and strengthens dividers.
    pub contrast_boost: bool,
}
impl Default for Adjust {
    fn default() -> Self {
        Self {
            saturation: 1.0,
            contrast_boost: false,
        }
    }
}

/// A derived theme: every token, and the gradient stops of the three large
/// surfaces when the theme has background colors.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub palette: Palette,
    pub dark: bool,
    /// Per gradient stop: (rail, sidebar, chat). Empty without a gradient.
    pub stops: Vec<(u32, u32, u32)>,
    pub angle: u16,
    /// The accent the person chose had to change to meet contrast.
    pub accent_adjusted: bool,
    /// Some requirement could not be met even at the end of the range.
    pub unmet: bool,
}

/// Contrast targets: body text, large/secondary text, non-text UI.
pub const AA_TEXT: f64 = 4.5;
pub const AA_UI: f64 = 3.0;
pub const AAA_TEXT: f64 = 7.0;

/// The base palette a theme starts from.
pub fn base_palette(base: ThemeBase) -> Palette {
    match base {
        ThemeBase::Dark => DARK,
        ThemeBase::Light => LIGHT,
    }
}

/// A surface tinted toward `tint`: its lightness stays (so the hierarchy and
/// contrast of the base palette are kept), hue and some chroma come from the tint.
fn tint(surface: u32, tint: u32, strength: f64, dark: bool) -> u32 {
    if strength <= 0.0 {
        return surface;
    }
    let s = lch(surface);
    let t = lch(tint);
    let cap = if dark { 0.09 } else { 0.06 };
    let c = s.c + ((t.c * 0.8).min(cap) - s.c) * strength;
    from_lch(Lch {
        l: s.l,
        c: c.max(0.0),
        h: if t.c < 0.004 { s.h } else { t.h },
    })
}

/// Derive every token from a base palette (built-in themes) or a spec.
pub fn derive_palette(p: Palette, dark: bool, adjust: Adjust) -> Theme {
    finish(p, dark, vec![], 0, adjust, false)
}

/// Derive a custom or preset theme.
pub fn derive(spec: &ThemeSpec, adjust: Adjust) -> Theme {
    let dark = spec.base == ThemeBase::Dark;
    let mut p = base_palette(spec.base);
    let strength = f64::from(spec.intensity.min(100)) / 100.0;
    let colors: Vec<u32> = spec
        .colors
        .iter()
        .take(MAX_THEME_COLORS)
        .map(|c| saturate(*c & 0xFF_FFFF, adjust.saturation))
        .collect();
    // The solid surfaces take the average tint; the large ones also get a
    // gradient, one stop per color.
    let average = if colors.is_empty() {
        None
    } else {
        let n = colors.len() as f64;
        let (mut l, mut a, mut b) = (0.0, 0.0, 0.0);
        for c in &colors {
            let (x, y, z) = oklab(*c);
            l += x / n;
            a += y / n;
            b += z / n;
        }
        Some(from_lch(Lch {
            l,
            c: (a * a + b * b).sqrt(),
            h: b.atan2(a),
        }))
    };
    if let Some(avg) = average {
        for s in [
            &mut p.rail,
            &mut p.sidebar,
            &mut p.chat,
            &mut p.elevated,
            &mut p.hover,
            &mut p.selected,
            &mut p.divider,
            &mut p.panel,
            &mut p.field,
            &mut p.control,
            &mut p.control_hover,
        ] {
            *s = tint(*s, avg, strength, dark);
        }
        // Text takes a whisper of the hue so it sits in the theme.
        for t in [
            &mut p.text_primary,
            &mut p.text_secondary,
            &mut p.text_muted,
        ] {
            *t = tint(*t, avg, strength * 0.25, dark);
        }
    }
    let base = base_palette(spec.base);
    let stops = if colors.len() > 1 {
        colors
            .iter()
            .map(|c| {
                (
                    tint(base.rail, *c, strength, dark),
                    tint(base.sidebar, *c, strength, dark),
                    tint(base.chat, *c, strength, dark),
                )
            })
            .collect()
    } else {
        vec![]
    };
    p.accent = spec.accent & 0xFF_FFFF;
    p.accent_hover = p.accent;
    let chosen = saturate(p.accent, adjust.saturation);
    let mut theme = finish(p, dark, stops, spec.angle % 360, adjust, true);
    theme.accent_adjusted = theme.palette.accent != chosen;
    theme
}

/// Saturation, then every contrast requirement, in an order where later
/// steps never undo earlier ones.
fn finish(
    mut p: Palette,
    dark: bool,
    stops: Vec<(u32, u32, u32)>,
    angle: u16,
    adjust: Adjust,
    derive_hover: bool,
) -> Theme {
    let s = adjust.saturation;
    for c in [
        &mut p.accent,
        &mut p.accent_hover,
        &mut p.danger,
        &mut p.warning,
        &mut p.online,
        &mut p.idle,
        &mut p.dnd,
        &mut p.brand_avatar,
    ] {
        *c = saturate(*c, s);
    }
    if s < 0.999 {
        for c in [
            &mut p.rail,
            &mut p.sidebar,
            &mut p.chat,
            &mut p.elevated,
            &mut p.hover,
            &mut p.selected,
            &mut p.divider,
            &mut p.panel,
            &mut p.field,
            &mut p.control,
            &mut p.control_hover,
        ] {
            *c = saturate(*c, s);
        }
    }
    let lighter = dark;
    let mut unmet = false;
    let check = |c: &mut u32, bgs: &[u32], min: f64| -> bool {
        let (v, ok) = enforce(*c, bgs, min, lighter);
        *c = v;
        !ok
    };
    // Every place text is drawn, including each gradient stop.
    let mut text_bgs = vec![
        p.rail, p.sidebar, p.chat, p.elevated, p.hover, p.selected, p.panel,
    ];
    for (r, sb, ch) in &stops {
        text_bgs.extend([*r, *sb, *ch]);
    }
    let mut primary_bgs = text_bgs.clone();
    primary_bgs.extend([p.field, p.control, p.control_hover]);
    let boost = adjust.contrast_boost;
    let body = if boost { AAA_TEXT } else { AA_TEXT };
    if boost {
        p.text_primary = if dark { 0xFF_FFFF } else { 0x000000 };
        p.text_secondary = mix(p.text_primary, p.text_secondary, 0.35);
        p.divider = mix(p.text_primary, p.divider, 0.18);
        p.control = mix(p.text_primary, p.control, 0.15);
    }
    unmet |= check(
        &mut p.text_primary,
        &primary_bgs,
        if boost { 12.0 } else { AA_TEXT },
    );
    unmet |= check(&mut p.text_secondary, &text_bgs, body);
    unmet |= check(&mut p.text_muted, &text_bgs, body);
    // Accent: links and mention text on surfaces; the focus ring and pressed
    // icons on the selected row are non-text.
    let link_bgs: Vec<u32> = text_bgs
        .iter()
        .copied()
        .filter(|b| *b != p.selected)
        .collect();
    unmet |= check(&mut p.accent, &link_bgs, body);
    unmet |= check(&mut p.accent, &[p.selected], AA_UI);
    if derive_hover {
        let mut h = lch(p.accent);
        h.l += if dark { 0.06 } else { -0.06 };
        p.accent_hover = from_lch(h);
    }
    unmet |= check(&mut p.accent_hover, &link_bgs, body);
    // Mention chip text: accent on accent 20 % over chat. The chip moves
    // with the accent, so step until both settle.
    let mut x = lch(p.accent);
    for _ in 0..100 {
        if wcag_contrast(p.accent, p.chip_on_chat()) >= AA_TEXT {
            break;
        }
        x.l += if lighter { 0.01 } else { -0.01 };
        p.accent = from_lch(x);
    }
    unmet |= wcag_contrast(p.accent, p.chip_on_chat()) < AA_TEXT;
    // Text on accent, danger and warning buttons and badges: black or white
    // tinted with the accent's hue, whichever reads better.
    let fills = [p.accent, p.accent_hover];
    let dark_ink = if dark { p.accent_text } else { 0x08170F };
    let light_ink = 0xFF_FFFF;
    let score = |ink: u32| {
        fills
            .iter()
            .map(|f| wcag_contrast(ink, *f))
            .fold(f64::MAX, f64::min)
    };
    p.accent_text = if !dark && p.accent_text == LIGHT.accent_text && derive_hover {
        if score(light_ink) >= score(dark_ink) {
            light_ink
        } else {
            dark_ink
        }
    } else if score(p.accent_text) >= AA_TEXT {
        p.accent_text
    } else if score(light_ink) >= score(dark_ink) {
        light_ink
    } else {
        dark_ink
    };
    if score(p.accent_text) < AA_TEXT {
        // Push the fills away from the ink.
        let away = wcag_luminance(p.accent_text) > 0.5;
        let (a, ok1) = enforce(p.accent, &[p.accent_text], AA_TEXT, !away);
        let (b, ok2) = enforce(p.accent_hover, &[p.accent_text], AA_TEXT, !away);
        p.accent = a;
        p.accent_hover = b;
        unmet |= !(ok1 && ok2);
    }
    let danger_bgs = [p.chat, p.sidebar, p.elevated, p.hover];
    unmet |= check(&mut p.danger, &danger_bgs, body);
    unmet |= check(&mut p.warning, &[p.chat, p.sidebar, p.elevated], body);
    // Danger and warning are also fills under accent.text.
    for c in [&mut p.danger, &mut p.warning] {
        if wcag_contrast(*c, p.accent_text) < AA_TEXT {
            let away = wcag_luminance(p.accent_text) > 0.5;
            let (v, ok) = enforce(*c, &[p.accent_text], AA_TEXT, !away);
            *c = v;
            unmet |= !ok;
        }
    }
    let ui_bgs = [p.sidebar, p.chat, p.elevated, p.panel];
    for c in [&mut p.online, &mut p.idle, &mut p.dnd, &mut p.offline] {
        let (v, ok) = enforce(*c, &ui_bgs, AA_UI, lighter);
        *c = v;
        unmet |= !ok;
    }
    // Mention highlight: body text on it.
    let mention = p.mention_on_chat();
    if wcag_contrast(p.text_primary, mention) < body {
        p.mention_alpha = p.mention_alpha.min(4);
    }
    Theme {
        palette: p,
        dark,
        stops,
        angle,
        accent_adjusted: false,
        unmet,
    }
}

/// One contrast requirement, for the report and the tests.
#[derive(Debug, Clone)]
pub struct Check {
    pub what: &'static str,
    pub ratio: f64,
    pub min: f64,
}
impl Theme {
    /// Every requirement this theme must meet, with its worst ratio.
    pub fn checks(&self, adjust: Adjust) -> Vec<Check> {
        let p = &self.palette;
        let body = if adjust.contrast_boost {
            AAA_TEXT
        } else {
            AA_TEXT
        };
        let worst = |fg: u32, bgs: &[u32]| {
            bgs.iter()
                .map(|b| wcag_contrast(fg, *b))
                .fold(f64::MAX, f64::min)
        };
        let mut text = vec![
            p.rail, p.sidebar, p.chat, p.elevated, p.hover, p.selected, p.panel,
        ];
        for (r, s, c) in &self.stops {
            text.extend([*r, *s, *c]);
        }
        let mut primary = text.clone();
        primary.extend([p.field, p.control, p.control_hover]);
        let links: Vec<u32> = text.iter().copied().filter(|b| *b != p.selected).collect();
        vec![
            Check {
                what: "text.primary on every surface",
                ratio: worst(p.text_primary, &primary),
                min: body,
            },
            Check {
                what: "text.secondary on every surface",
                ratio: worst(p.text_secondary, &text),
                min: body,
            },
            Check {
                what: "text.muted on every surface",
                ratio: worst(p.text_muted, &text),
                min: body,
            },
            Check {
                what: "accent (links) on surfaces",
                ratio: worst(p.accent, &links),
                min: body,
            },
            Check {
                what: "accent.hover (links) on surfaces",
                ratio: worst(p.accent_hover, &links),
                min: body,
            },
            Check {
                what: "accent (ring, pressed) on selected",
                ratio: wcag_contrast(p.accent, p.selected),
                min: AA_UI,
            },
            Check {
                what: "mention chip text",
                ratio: wcag_contrast(p.accent, p.chip_on_chat()),
                min: AA_TEXT,
            },
            Check {
                what: "accent.text on accent",
                ratio: worst(p.accent_text, &[p.accent, p.accent_hover]),
                min: AA_TEXT,
            },
            Check {
                what: "accent.text on danger and warning",
                ratio: worst(p.accent_text, &[p.danger, p.warning]),
                min: AA_TEXT,
            },
            Check {
                what: "danger text on surfaces",
                ratio: worst(p.danger, &[p.chat, p.sidebar, p.elevated, p.hover]),
                min: body,
            },
            Check {
                what: "warning text on surfaces",
                ratio: worst(p.warning, &[p.chat, p.sidebar, p.elevated]),
                min: body,
            },
            Check {
                what: "text on mention highlight",
                ratio: wcag_contrast(p.text_primary, p.mention_on_chat()),
                min: body,
            },
            Check {
                what: "status dots on surfaces",
                ratio: [p.online, p.idle, p.dnd, p.offline]
                    .iter()
                    .map(|c| worst(*c, &[p.sidebar, p.chat, p.elevated, p.panel]))
                    .fold(f64::MAX, f64::min),
                min: AA_UI,
            },
            Check {
                what: "switch knob on track",
                ratio: wcag_contrast(p.text_primary, p.control),
                min: AA_UI,
            },
        ]
    }
}

fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xFF_FFFF)
}
impl Theme {
    /// The theme as CSS declarations (no selector): every token, the gradient
    /// surfaces when there is a gradient, the role-name lightness range and
    /// the saturation for role colors.
    pub fn declarations(&self, adjust: Adjust) -> String {
        let p = &self.palette;
        let mut css = String::with_capacity(1400);
        for ((_, name), value) in TOKEN_NAMES.iter().zip(p.values()) {
            css.push_str(name);
            css.push(':');
            css.push_str(&hex(value));
            css.push(';');
        }
        let w = p.warning;
        css.push_str(&format!(
            "--mention-bg:rgb({} {} {} / {}%);",
            (w >> 16) & 0xFF,
            (w >> 8) & 0xFF,
            w & 0xFF,
            p.mention_alpha
        ));
        css.push_str(if self.dark {
            "color-scheme:dark;--name-l-min:0.7;--name-l-max:1;"
        } else {
            "color-scheme:light;--name-l-min:0;--name-l-max:0.48;"
        });
        css.push_str(&format!(
            "--saturation:{:.2};",
            adjust.saturation.clamp(0.0, 1.0)
        ));
        if !self.stops.is_empty() {
            let grad = |pick: fn(&(u32, u32, u32)) -> u32| {
                let stops: Vec<String> = self.stops.iter().map(|s| hex(pick(s))).collect();
                format!("linear-gradient({}deg,{})", self.angle, stops.join(","))
            };
            css.push_str(&format!("--surface-rail:{};", grad(|s| s.0)));
            css.push_str(&format!("--surface-sidebar:{};", grad(|s| s.1)));
            css.push_str(&format!("--surface-chat:{};", grad(|s| s.2)));
        }
        css
    }
    /// The browser UI color (`theme-color`): the rail.
    pub fn theme_color(&self) -> String {
        self.stops
            .first()
            .map_or(hex(self.palette.rail), |s| hex(s.0))
    }
}

// ---------------------------------------------------------------------------
// Share codes
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
fn b64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 3);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}
fn unb64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for ch in s.bytes() {
        let v = B64.iter().position(|b| *b == ch)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    // Leftover bits must be zero padding.
    (bits < 6 && acc & ((1 << bits) - 1) == 0).then_some(out)
}
const CODE_VERSION: u8 = 1;
/// A short, URL-safe code for a theme (and optionally its name).
pub fn share_code(spec: &ThemeSpec, name: &str) -> String {
    let mut b = vec![CODE_VERSION];
    let colors = spec.colors.len().min(MAX_THEME_COLORS) as u8;
    b.push(u8::from(spec.base == ThemeBase::Light) | (colors << 1));
    let rgb = |b: &mut Vec<u8>, c: u32| b.extend([(c >> 16) as u8, (c >> 8) as u8, c as u8]);
    rgb(&mut b, spec.accent);
    for c in spec.colors.iter().take(MAX_THEME_COLORS) {
        rgb(&mut b, *c);
    }
    b.extend((spec.angle % 360).to_be_bytes());
    b.push(spec.intensity.min(100));
    let name: String = name.trim().chars().take(MAX_THEME_NAME_CHARS).collect();
    b.extend(name.as_bytes());
    b64(&b)
}
/// Decode a share code (or a link containing `theme=<code>`). Untrusted input:
/// every field is checked, and the name is cleaned.
pub fn parse_share_code(input: &str) -> Result<(ThemeSpec, String), &'static str> {
    let input = input.trim();
    let code = input
        .split(['?', '&', '#'])
        .find_map(|part| part.strip_prefix("theme="))
        .unwrap_or(input);
    if code.is_empty() || code.len() > MAX_THEME_CODE_CHARS {
        return Err("That is not a theme code.");
    }
    let b = unb64(code).ok_or("That is not a theme code.")?;
    let bad = "That theme code is damaged.";
    if b.first() != Some(&CODE_VERSION) || b.len() < 8 {
        return Err(bad);
    }
    let flags = b[1];
    if flags >> 4 != 0 {
        return Err(bad);
    }
    let colors = usize::from(flags >> 1);
    if colors > MAX_THEME_COLORS {
        return Err(bad);
    }
    let fixed = 2 + 3 + colors * 3 + 3;
    if b.len() < fixed {
        return Err(bad);
    }
    let rgb = |i: usize| (u32::from(b[i]) << 16) | (u32::from(b[i + 1]) << 8) | u32::from(b[i + 2]);
    let spec = ThemeSpec {
        base: if flags & 1 == 1 {
            ThemeBase::Light
        } else {
            ThemeBase::Dark
        },
        accent: rgb(2),
        colors: (0..colors).map(|i| rgb(5 + i * 3)).collect(),
        angle: u16::from_be_bytes([b[5 + colors * 3], b[6 + colors * 3]]),
        intensity: b[7 + colors * 3],
    };
    spec.validate().map_err(|_| bad)?;
    let name = std::str::from_utf8(&b[fixed..]).map_err(|_| bad)?;
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_THEME_NAME_CHARS)
        .collect();
    Ok((spec, name.trim().to_owned()))
}

/// Parse `#rgb` / `#rrggbb` (the `#` optional).
pub fn parse_hex(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches('#');
    let full = match s.len() {
        3 => s.chars().flat_map(|c| [c, c]).collect(),
        6 => s.to_owned(),
        _ => return None,
    };
    u32::from_str_radix(&full, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(name: &str, theme: &Theme, adjust: Adjust) -> f64 {
        let mut least = f64::MAX;
        for c in theme.checks(adjust) {
            assert!(
                c.ratio + 1e-9 >= c.min,
                "{name}: {} is {:.2}:1, needs {:.1}:1",
                c.what,
                c.ratio,
                c.min
            );
            least = least.min(c.ratio / c.min);
        }
        least
    }

    /// Prints the contrast report (run with `--nocapture`) and fails on any
    /// requirement below WCAG AA.
    #[test]
    fn every_theme_meets_aa() {
        let plain = Adjust::default();
        let mut rows = vec![];
        for (name, p, dark) in [
            ("Dark", DARK, true),
            ("Light", LIGHT, false),
            ("Darker", DARKER, true),
        ] {
            let t = derive_palette(p, dark, plain);
            report(name, &t, plain);
            rows.push((name.to_owned(), t, plain));
            for adjust in [
                Adjust {
                    saturation: 0.0,
                    contrast_boost: false,
                },
                Adjust {
                    saturation: 0.5,
                    contrast_boost: true,
                },
            ] {
                let t = derive_palette(p, dark, adjust);
                report(name, &t, adjust);
            }
        }
        for preset in PRESETS {
            let t = derive(&preset.spec(), plain);
            assert!(!t.unmet, "{} could not meet every target", preset.name);
            report(preset.name, &t, plain);
            rows.push((preset.name.to_owned(), t, plain));
            let boost = Adjust {
                saturation: 0.3,
                contrast_boost: true,
            };
            report(preset.name, &derive(&preset.spec(), boost), boost);
        }
        println!("| Theme | text.primary | text.secondary | text.muted | accent (links) | accent.text | danger | status dots | lowest margin |");
        println!("|---|---|---|---|---|---|---|---|---|");
        for (name, t, adjust) in rows {
            let c = t.checks(adjust);
            let r = |what: &str| c.iter().find(|x| x.what == what).map_or(0.0, |x| x.ratio);
            let margin = c.iter().map(|x| x.ratio / x.min).fold(f64::MAX, f64::min);
            println!(
                "| {name} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2}x |",
                r("text.primary on every surface"),
                r("text.secondary on every surface"),
                r("text.muted on every surface"),
                r("accent (links) on surfaces"),
                r("accent.text on accent"),
                r("danger text on surfaces"),
                r("status dots on surfaces"),
                margin
            );
        }
    }

    /// Hostile custom themes are pulled up to AA (or flagged).
    #[test]
    fn hostile_custom_themes_are_fixed() {
        let plain = Adjust::default();
        for (base, accent, colors) in [
            (
                ThemeBase::Dark,
                0x000000,
                vec![0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF],
            ),
            (ThemeBase::Light, 0xFFFFFF, vec![0xFFFFFF, 0x000000]),
            (ThemeBase::Light, 0xFFFF00, vec![0xFFFF00]),
            (ThemeBase::Dark, 0x101010, vec![0x101010]),
            (ThemeBase::Dark, 0x7F7F7F, vec![]),
        ] {
            for intensity in [0, 50, 100] {
                let spec = ThemeSpec {
                    base,
                    accent,
                    colors: colors.clone(),
                    angle: 90,
                    intensity,
                };
                let t = derive(&spec, plain);
                assert!(!t.unmet, "{spec:?}");
                report("custom", &t, plain);
            }
        }
        let t = derive(
            &ThemeSpec {
                base: ThemeBase::Dark,
                accent: 0x000000,
                ..Default::default()
            },
            plain,
        );
        assert!(t.accent_adjusted);
    }

    /// The default dark theme derives to exactly the UI_SPEC values.
    #[test]
    fn default_dark_is_unchanged() {
        let t = derive_palette(DARK, true, Adjust::default());
        assert_eq!(t.palette, DARK);
        let t = derive_palette(LIGHT, false, Adjust::default());
        assert_eq!(t.palette, LIGHT);
        let t = derive_palette(DARKER, true, Adjust::default());
        assert_eq!(t.palette, DARKER);
    }

    /// These constants are the tables in docs/UI_SPEC.md.
    #[test]
    fn palettes_match_ui_spec() {
        let spec = include_str!("../../../docs/UI_SPEC.md");
        for (heading, palette) in [
            ("### Color (dark, default)", DARK),
            ("### Color (light)", LIGHT),
            ("### Color (darker)", DARKER),
        ] {
            let section = spec.split(heading).nth(1).expect(heading);
            let section = section.split("\n#").next().unwrap();
            let mut table = std::collections::BTreeMap::new();
            for line in section.lines().filter(|l| l.starts_with("| ")) {
                let cols: Vec<&str> = line.split('|').map(str::trim).collect();
                if cols.len() < 4 {
                    continue;
                }
                let (key, value) = (cols[1], cols[2]);
                if key == "online / idle / dnd / offline" {
                    for (k, v) in key.split(" / ").zip(value.split(" / ")) {
                        table.insert(k.to_owned(), parse_hex(v).unwrap());
                    }
                } else if let Some(v) = value.strip_prefix('#') {
                    table.insert(key.to_owned(), parse_hex(&v[..6]).unwrap());
                }
            }
            for ((key, _), value) in TOKEN_NAMES.iter().zip(palette.values()) {
                let expected = if *key == "text.muted" && palette == DARK {
                    // The table keeps #74747C; the token sheet's AA override applies.
                    assert_eq!(table.get(*key), Some(&0x74747C));
                    continue;
                } else {
                    table.get(*key)
                };
                assert_eq!(expected, Some(&value), "{heading} {key}");
            }
        }
    }

    #[test]
    fn share_codes_round_trip_and_reject_garbage() {
        for preset in PRESETS {
            let code = share_code(&preset.spec(), preset.name);
            assert!(code.len() < 64, "{code}");
            assert_eq!(
                parse_share_code(&code),
                Ok((preset.spec(), preset.name.to_owned()))
            );
            let link = format!("https://chat.example/?theme={code}");
            assert_eq!(parse_share_code(&link).unwrap().0, preset.spec());
        }
        let spec = ThemeSpec {
            base: ThemeBase::Light,
            accent: 0x123456,
            colors: vec![1, 2, 3, 4, 5],
            angle: 359,
            intensity: 100,
        };
        let code = share_code(&spec, "Ünïcödé 🎨 theme with a long name that is cut");
        let (back, name) = parse_share_code(&code).unwrap();
        assert_eq!(back, spec);
        assert_eq!(name.chars().count(), MAX_THEME_NAME_CHARS);
        for bad in ["", "!!!!", "AAAA", "AQ", &"A".repeat(400), "AQAAAAAA_w"] {
            assert!(parse_share_code(bad).is_err(), "{bad}");
        }
        // Every byte string decodes or is refused; none panics.
        let mut seed = 7u64;
        for _ in 0..5000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let len = (seed >> 58) as usize;
            let bytes: Vec<u8> = (0..len).map(|i| (seed >> (i % 56)) as u8).collect();
            if let Ok((s, n)) = parse_share_code(&b64(&bytes)) {
                assert!(s.validate().is_ok());
                assert!(
                    n.chars().count() <= MAX_THEME_NAME_CHARS && !n.chars().any(char::is_control)
                );
            }
        }
        // A control character in the name is removed.
        let mut raw = vec![CODE_VERSION, 0, 1, 2, 3, 0, 0, 50];
        raw.extend(b"a\x07b");
        assert_eq!(parse_share_code(&b64(&raw)).unwrap().1, "ab");
    }

    #[test]
    fn validation() {
        assert!(ThemeSpec {
            colors: vec![0; 6],
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ThemeSpec {
            accent: 0x1000000,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ThemeSpec {
            angle: 360,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ThemeSpec {
            intensity: 101,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ThemeChoice::Preset("nope".into()).validate().is_err());
        assert!(ThemeChoice::Preset("aurora".into()).validate().is_ok());
        assert!(validate_name(" ").is_err());
        assert!(validate_name(&"x".repeat(33)).is_err());
        assert!(validate_name("a\u{0}").is_err());
        assert_eq!(validate_name("  Mine ").unwrap(), "Mine");
        let ids: std::collections::BTreeSet<_> = PRESETS.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), PRESETS.len());
        assert!(
            PRESETS
                .iter()
                .filter(|p| p.base == ThemeBase::Light)
                .count()
                >= 10
        );
        assert!(PRESETS.iter().filter(|p| p.base == ThemeBase::Dark).count() >= 10);
    }

    #[test]
    fn wire_shapes() {
        let lib = ThemeLibrary {
            themes: vec![CustomTheme {
                id: Id(1),
                name: "A".into(),
                spec: PRESETS[0].spec(),
            }],
            settings: ThemeSettings {
                server_themes: false,
                server_overrides: vec![ServerThemeOverride {
                    guild_id: Id(2),
                    show: true,
                }],
            },
            servers: vec![ServerTheme {
                guild_id: Id(2),
                theme: ThemeChoice::Preset("ocean".into()),
            }],
        };
        let bytes = rmp_serde::to_vec_named(&lib).unwrap();
        assert_eq!(rmp_serde::from_slice::<ThemeLibrary>(&bytes).unwrap(), lib);
        assert!(lib.settings.shows(Id(2)));
        assert!(!lib.settings.shows(Id(3)));
        let json = serde_json::to_string(&AppearanceCommand::SetServerTheme {
            guild_id: Id(2),
            theme: Some(ThemeChoice::Custom(ThemeSpec::default())),
        })
        .unwrap();
        assert!(json.contains("set_server_theme"));
        // The protocol's tagged shape survives MessagePack, as the server reads it.
        for command in [
            AppearanceCommand::Subscribe,
            AppearanceCommand::DeleteTheme { id: Id(5) },
            AppearanceCommand::SetServerTheme {
                guild_id: Id(2),
                theme: Some(ThemeChoice::Preset("ocean".into())),
            },
        ] {
            let msg = ClientMsg::Command(CommandRequest {
                nonce: 7,
                command: Command::Appearance(command),
            });
            let bytes = rmp_serde::to_vec_named(&msg).unwrap();
            assert_eq!(rmp_serde::from_slice::<ClientMsg>(&bytes).unwrap(), msg);
        }
        let empty: ThemeSettings = serde_json::from_str("{}").unwrap();
        assert!(empty.server_themes);
    }
}
