//! Profiles and cosmetics (`docs/features/profiles.md`): animated avatars,
//! profile banners, profile colors, per-server profiles, display name styles,
//! badges, avatar decorations, profile effects, nameplates and profile frames.
//! Everything is free for every user. Every type here is additive: commands
//! travel as `LaunchCommand::Profiles`, the look of a name and avatar travels
//! with `UserSummary::style` and `Member::server_style`, and both are omitted
//! when empty, so older clients and servers never see them.
use crate::{AttachmentId, GuildId, Id, UnixMillis, UserId, UserSummary};
use serde::{Deserialize, Serialize};

pub const MAX_ABOUT_ME_CHARS: usize = 190;
pub const MAX_PRONOUNS_CHARS: usize = 40;
pub const MAX_SHOWN_BADGES: usize = 6;
/// Avatars and banners, still or animated.
pub const MAX_PROFILE_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
/// Largest still image side accepted for an avatar or banner.
pub const MAX_PROFILE_IMAGE_SIDE: u32 = 4096;
/// An animation may have at most this many pixels in one frame (1920x1080).
pub const MAX_ANIMATED_FRAME_PIXELS: u64 = 1920 * 1080;
pub const MAX_ANIMATION_FRAMES: u32 = 500;
/// All frames together, which bounds the work of validating one upload.
pub const MAX_ANIMATION_TOTAL_PIXELS: u64 = 250_000_000;
pub const MAX_CUSTOM_BADGES: usize = 100;
pub const MAX_BADGE_NAME_CHARS: usize = 40;
pub const MAX_BADGE_DESCRIPTION_CHARS: usize = 120;
/// A badge icon is a small still image.
pub const MAX_BADGE_ICON_BYTES: usize = 256 * 1024;
/// "Server Owner" is earned by owning a server with at least this many members.
pub const SERVER_OWNER_BADGE_MEMBERS: u32 = 50;
/// "Early Member" is earned by accounts created before this time unless the
/// administrator chose another cutoff: 2027-01-01T00:00:00Z.
pub const DEFAULT_EARLY_MEMBER_CUTOFF: UnixMillis = 1_798_761_600_000;
/// Account-age badge tiers, in years. The badge evolves through them.
pub const MEMBER_YEAR_TIERS: [u32; 4] = [1, 2, 3, 5];
/// Name colors must reach this contrast with the lightest surface a name is
/// drawn on (`bg.selected`, also darker than a hovered or mentioned row).
pub const MIN_NAME_CONTRAST: f32 = 4.5;
pub const NAME_SURFACE: u32 = 0x2E2E34;

/// Which part of an image is shown: zoom in percent (100 to 400) and the
/// point, in percent of the image, that stays fixed while zooming. Applied by
/// the client (object-fit: cover), so an animation never has to be re-encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crop {
    pub zoom: u16,
    pub x: u8,
    pub y: u8,
}
impl Crop {
    pub fn valid(&self) -> bool {
        (100..=400).contains(&self.zoom) && self.x <= 100 && self.y <= 100
    }
    /// The identity crop is stored as no crop.
    pub fn is_identity(&self) -> bool {
        self.zoom == 100 && self.x == 50 && self.y == 50
    }
}
/// An avatar or banner image and how to draw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageMeta {
    pub id: AttachmentId,
    /// The original moves. Lists draw its still first frame (`/thumbnail`)
    /// and play the original only while hovered or focused.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub animated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
}
/// An uploaded image chosen for an avatar or banner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    pub id: AttachmentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
}

/// Bundled display-name fonts (OFL, see `crates/web/public/fonts/names/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameFont {
    #[default]
    Default,
    Script,
    Pixel,
    Future,
    Serif,
    Hand,
    Display,
}
impl NameFont {
    pub const ALL: [NameFont; 7] = [
        Self::Default,
        Self::Script,
        Self::Pixel,
        Self::Future,
        Self::Serif,
        Self::Hand,
        Self::Display,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Script => "script",
            Self::Pixel => "pixel",
            Self::Future => "future",
            Self::Serif => "serif",
            Self::Hand => "hand",
            Self::Display => "display",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Script => "Script",
            Self::Pixel => "Pixel",
            Self::Future => "Future",
            Self::Serif => "Serif",
            Self::Hand => "Handwritten",
            Self::Display => "Display",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == s)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameEffect {
    #[default]
    Solid,
    Gradient,
    Glow,
    Outline,
    /// A light sweep across the name, only while it is hovered or focused.
    Shimmer,
}
impl NameEffect {
    pub const ALL: [NameEffect; 5] = [
        Self::Solid,
        Self::Gradient,
        Self::Glow,
        Self::Outline,
        Self::Shimmer,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Gradient => "gradient",
            Self::Glow => "glow",
            Self::Outline => "outline",
            Self::Shimmer => "shimmer",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Gradient => "Gradient",
            Self::Glow => "Glow",
            Self::Outline => "Outline",
            Self::Shimmer => "Shimmer on hover",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == s)
    }
}
/// How a display name is drawn. `color2` is the second gradient color and is
/// required for `Gradient` (and allowed for `Shimmer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameStyle {
    #[serde(default)]
    pub font: NameFont,
    pub color: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color2: Option<u32>,
    #[serde(default)]
    pub effect: NameEffect,
}
impl NameStyle {
    /// Why this style cannot be saved, if it cannot.
    pub fn problem(&self) -> Option<&'static str> {
        if self.color > 0xFF_FFFF || self.color2.is_some_and(|c| c > 0xFF_FFFF) {
            return Some("Colors must be RGB.");
        }
        if self.effect == NameEffect::Gradient && self.color2.is_none() {
            return Some("A gradient needs two colors.");
        }
        if !legible_name_color(self.color) || self.color2.is_some_and(|c| !legible_name_color(c)) {
            return Some("That color is too dark to read. Choose a lighter one.");
        }
        None
    }
}

/// The look of a name and avatar wherever it is drawn. On `UserSummary` it is
/// the global look; on `Member::server_style` it holds only what that server's
/// profile overrides.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserStyle {
    /// Describes the avatar whose ID it carries; ignored for any other avatar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<ImageMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<NameStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoration: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nameplate: Option<String>,
    /// A bot or webhook account: shown with an APP tag (`apps.rs`). Set only
    /// by the server, for the accounts it creates for apps and webhooks.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bot: bool,
}
impl UserStyle {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// The profile card: banner, colors, effect and frame.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProfileLook {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner: Option<ImageMeta>,
    /// Primary and accent colors tint the card as a gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
}

/// A member's profile in one server. Every field is optional and falls back
/// to the global profile. The nickname is the existing server nickname.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServerProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<ImageMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner: Option<ImageMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pronouns: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about_me: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<NameStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoration: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nameplate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
}
impl ServerProfile {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
    /// What `Member::server_style` carries: the overrides lists need.
    pub fn style(&self) -> UserStyle {
        UserStyle {
            avatar: self.avatar.clone(),
            name: self.name,
            decoration: self.decoration.clone(),
            nameplate: self.nameplate.clone(),
            bot: false,
        }
    }
}

/// One badge on a profile. Built-in badges have a `slug` icon drawn by the
/// web app (`/public/badges/<slug>.svg`); custom badges have an uploaded icon
/// at `/api/v1/badges/<custom>/icon`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Badge {
    /// Stable ID used to choose and order shown badges: a built-in name such
    /// as `early_member`, or `c<number>` for a custom badge.
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<Id>,
}
/// Built-in badges, by ID: name, and whether the administrator awards it.
pub const BUILTIN_BADGES: &[(&str, &str, bool)] = &[
    ("instance_admin", "Instance Administrator", true),
    ("server_owner", "Server Owner", false),
    ("early_member", "Early Member", false),
    ("member_years", "Member for Years", false),
    ("bug_reporter", "Bug Reporter", true),
    ("contributor", "Contributor", true),
];

/// Everything a profile card shows. `server` is the member's profile in the
/// server the card was opened from, when they have one there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileCard {
    pub user: UserSummary,
    pub pronouns: String,
    pub about_me: String,
    pub banner_color: u32,
    pub created_at: UnixMillis,
    #[serde(default)]
    pub look: ProfileLook,
    /// Shown badges, in the order chosen.
    #[serde(default)]
    pub badges: Vec<Badge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_since: Option<UnixMillis>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildServerProfile {
    pub guild_id: GuildId,
    pub profile: ServerProfile,
}
/// The signed-in person's own profiles, for the editor.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MyProfiles {
    pub pronouns: String,
    pub about_me: String,
    pub banner_color: u32,
    #[serde(default)]
    pub look: ProfileLook,
    /// Every badge earned, in display order (shown first).
    #[serde(default)]
    pub earned: Vec<Badge>,
    /// IDs of the shown badges, in order; empty until chosen, which shows the
    /// first six earned.
    #[serde(default)]
    pub shown: Vec<String>,
    #[serde(default)]
    pub chosen: bool,
    #[serde(default)]
    pub servers: Vec<GuildServerProfile>,
}

/// The global profile, replaced as a whole. Avatar and banner images are set
/// with `SetImage`, the display name with the existing `UpdateProfile`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProfileEdit {
    pub pronouns: String,
    pub about_me: String,
    pub banner_color: u32,
    #[serde(default)]
    pub primary: Option<u32>,
    #[serde(default)]
    pub accent: Option<u32>,
    #[serde(default)]
    pub name: Option<NameStyle>,
    #[serde(default)]
    pub decoration: Option<String>,
    #[serde(default)]
    pub nameplate: Option<String>,
    #[serde(default)]
    pub effect: Option<String>,
    #[serde(default)]
    pub frame: Option<String>,
}
/// A server profile, replaced as a whole; `None` falls back to the global
/// profile. Images are set with `SetImage`, the nickname with `SetNickname`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServerProfileEdit {
    #[serde(default)]
    pub pronouns: Option<String>,
    #[serde(default)]
    pub about_me: Option<String>,
    #[serde(default)]
    pub primary: Option<u32>,
    #[serde(default)]
    pub accent: Option<u32>,
    #[serde(default)]
    pub name: Option<NameStyle>,
    #[serde(default)]
    pub decoration: Option<String>,
    #[serde(default)]
    pub nameplate: Option<String>,
    #[serde(default)]
    pub effect: Option<String>,
    #[serde(default)]
    pub frame: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageSlot {
    Avatar,
    Banner,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ProfilesCommand {
    Update(ProfileEdit),
    UpdateServer {
        guild_id: GuildId,
        edit: ServerProfileEdit,
    },
    /// `guild_id` none sets the global image. `image` none removes it.
    SetImage {
        guild_id: Option<GuildId>,
        slot: ImageSlot,
        image: Option<ImageRef>,
    },
    /// Up to six earned badge IDs, in display order.
    SetBadges {
        shown: Vec<String>,
    },
    /// A moderator removes a member's server avatar and/or banner. Time Out
    /// Members is required, and the member must be below the moderator. The
    /// nickname is reset with the existing `SetNickname` (Manage Nicknames).
    Reset {
        guild_id: GuildId,
        user_id: UserId,
        avatar: bool,
        banner: bool,
        #[serde(default)]
        reason: Option<String>,
    },
}

/// One catalog entry: stable ID and display name.
pub type Cosmetic = (&'static str, &'static str);
/// Frames drawn around the avatar (original CSS/SVG art in the web app).
pub const DECORATIONS: &[Cosmetic] = &[
    ("halo", "Halo"),
    ("orbit", "Orbit"),
    ("sprout", "Sprout"),
    ("crown", "Crown"),
    ("cat_ears", "Cat Ears"),
    ("starfield", "Starfield"),
    ("ember", "Ember"),
    ("frost", "Frost"),
    ("pixel", "Pixel"),
    ("prism", "Prism"),
    ("bubbles", "Bubbles"),
    ("headset", "Headset"),
    ("hearts", "Hearts"),
    ("lightning", "Lightning"),
];
/// Subtle overlays on the profile card only.
pub const EFFECTS: &[Cosmetic] = &[
    ("fireflies", "Fireflies"),
    ("snowfall", "Snowfall"),
    ("aurora", "Aurora"),
    ("confetti", "Confetti"),
    ("rain", "Rain"),
    ("sparkle", "Sparkle"),
    ("bubbles", "Bubbles"),
];
/// Backgrounds behind a name in the member list and direct message list.
pub const NAMEPLATES: &[Cosmetic] = &[
    ("dusk", "Dusk"),
    ("forest", "Forest"),
    ("ocean", "Ocean"),
    ("ember", "Ember"),
    ("candy", "Candy"),
    ("circuit", "Circuit"),
    ("stars", "Stars"),
    ("waves", "Waves"),
    ("plaid", "Plaid"),
];
/// Borders around the whole profile card.
pub const FRAMES: &[Cosmetic] = &[
    ("gilded", "Gilded"),
    ("neon", "Neon"),
    ("vine", "Vine"),
    ("pixel", "Pixel"),
    ("ribbon", "Ribbon"),
    ("frost", "Frost"),
];
pub fn known(catalog: &[Cosmetic], id: &str) -> bool {
    catalog.iter().any(|(v, _)| *v == id)
}

fn channel(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.039_28 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
/// WCAG relative luminance of 0xRRGGBB.
pub fn luminance(rgb: u32) -> f32 {
    0.2126 * channel((rgb >> 16) as u8)
        + 0.7152 * channel((rgb >> 8) as u8)
        + 0.0722 * channel(rgb as u8)
}
/// WCAG contrast ratio of two 0xRRGGBB colors.
pub fn contrast(a: u32, b: u32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
/// A name color readable on every surface names are drawn on, in both the
/// cozy and compact message layouts (they share their backgrounds).
pub fn legible_name_color(rgb: u32) -> bool {
    contrast(rgb, NAME_SURFACE) >= MIN_NAME_CONTRAST
}
/// Mix `rgb` toward black by `percent`.
pub fn darken(rgb: u32, percent: u8) -> u32 {
    let k = 100 - u32::from(percent.min(100));
    let ch = |shift: u32| ((rgb >> shift) & 0xff) * k / 100;
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}
/// Text and scrim for a profile card tinted with `primary` and `accent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardTheme {
    /// 0xRRGGBB of all text on the card.
    pub text: u32,
    /// Percent of black laid over the tint so that `text` stays readable.
    pub scrim: u8,
}
pub const CARD_LIGHT_TEXT: u32 = 0xE8EAED;
pub const CARD_DARK_TEXT: u32 = 0x111113;
/// Text on a card must reach 7:1, so that its secondary (80%) text still
/// reaches 4.5:1 everywhere on the gradient.
pub const CARD_TEXT_CONTRAST: f32 = 7.0;
/// Choose light or dark text for the card's two colors, and darken the tint
/// as little as needed for that text to reach `CARD_TEXT_CONTRAST` on both.
pub fn card_theme(primary: u32, accent: u32) -> CardTheme {
    let worst = |text: u32, scrim: u8| {
        contrast(text, darken(primary, scrim)).min(contrast(text, darken(accent, scrim)))
    };
    if worst(CARD_DARK_TEXT, 0) >= CARD_TEXT_CONTRAST {
        return CardTheme {
            text: CARD_DARK_TEXT,
            scrim: 0,
        };
    }
    let scrim = (0..=90u8)
        .step_by(5)
        .find(|s| worst(CARD_LIGHT_TEXT, *s) >= CARD_TEXT_CONTRAST)
        .unwrap_or(90);
    CardTheme {
        text: CARD_LIGHT_TEXT,
        scrim,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_are_omitted_when_empty_and_old_payloads_decode() {
        let u = UserSummary {
            id: Id(1),
            username: "a".into(),
            ..Default::default()
        };
        let bytes = rmp_serde::to_vec_named(&u).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("style"));
        #[derive(Serialize)]
        struct Old {
            id: Id,
            username: String,
            display_name: Option<String>,
            avatar_id: Option<Id>,
            status_text: Option<String>,
        }
        let old = rmp_serde::to_vec_named(&Old {
            id: Id(1),
            username: "a".into(),
            display_name: None,
            avatar_id: None,
            status_text: None,
        })
        .unwrap();
        assert_eq!(rmp_serde::from_slice::<UserSummary>(&old).unwrap(), u);
        let styled = UserSummary {
            style: Box::new(UserStyle {
                decoration: Some("halo".into()),
                ..Default::default()
            }),
            ..u.clone()
        };
        let back: UserSummary =
            rmp_serde::from_slice(&rmp_serde::to_vec_named(&styled).unwrap()).unwrap();
        assert_eq!(back, styled);
    }
    #[test]
    fn name_colors_must_be_readable() {
        assert!(legible_name_color(0xFFFFFF));
        assert!(legible_name_color(0x3DDC97));
        assert!(!legible_name_color(0x000000));
        assert!(!legible_name_color(0x5865F2 & 0x3f3f3f));
        let style = NameStyle {
            font: NameFont::Pixel,
            color: 0xFFFFFF,
            color2: None,
            effect: NameEffect::Gradient,
        };
        assert!(style.problem().is_some());
        assert!(NameStyle {
            color2: Some(0x101010),
            ..style
        }
        .problem()
        .is_some());
        assert!(NameStyle {
            color2: Some(0xF2B84B),
            ..style
        }
        .problem()
        .is_none());
    }
    #[test]
    fn card_text_is_readable_on_any_two_colors() {
        for (p, a) in [
            (0x000000, 0xFFFFFF),
            (0xFFFFFF, 0xFFFFFF),
            (0xFF0000, 0x00FF00),
            (0x3DDC97, 0x1C1C20),
            (0x777777, 0x888888),
        ] {
            let t = card_theme(p, a);
            for c in [p, a] {
                assert!(
                    contrast(t.text, darken(c, t.scrim)) >= CARD_TEXT_CONTRAST,
                    "{p:06x}/{a:06x} -> {t:?}"
                );
            }
        }
        assert_eq!(card_theme(0xFFFFFF, 0xF0F0F0).text, CARD_DARK_TEXT);
        assert_eq!(card_theme(0x101010, 0x202020).scrim, 0);
    }
    #[test]
    fn catalogs_meet_the_minimums_and_ids_are_unique() {
        assert!(DECORATIONS.len() >= 12);
        assert!(EFFECTS.len() >= 6);
        assert!(NAMEPLATES.len() >= 8);
        assert!(FRAMES.len() >= 4);
        for list in [DECORATIONS, EFFECTS, NAMEPLATES, FRAMES] {
            let mut ids: Vec<_> = list.iter().map(|(id, _)| *id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), list.len());
            assert!(ids
                .iter()
                .all(|id| id.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')));
        }
        assert!(Crop {
            zoom: 100,
            x: 50,
            y: 50
        }
        .is_identity());
        assert!(!Crop {
            zoom: 401,
            x: 0,
            y: 0
        }
        .valid());
    }
}
