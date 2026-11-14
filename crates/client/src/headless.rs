//! Offscreen rendering of the real iced view tree. This module never constructs a winit window.
use crate::{
    app::{App, Msg},
    theme as t,
};
use iced::{advanced::widget::operation, mouse, Size};
use iced_runtime::user_interface::{Cache, UserInterface};
use std::{path::Path, time::Instant};
pub const AUDIO_SCREENS: &[&str] = &[
    "audio-default",
    "audio-mic-test",
    "audio-manual",
    "audio-ptt",
    "audio-devices",
    "audio-bluetooth",
    "audio-processing",
];
pub const SCREENS: &[&str] = &[
    "focused-stream",
    "stream-stats",
    "share-picker-windows",
    "share-quality",
    "controlling-banner",
    "request-control",
    "camera-tiles",
    "camera-grid-1",
    "camera-grid-2",
    "camera-grid-3",
    "camera-grid-4",
    "camera-grid-5",
    "camera-grid-6",
    "camera-grid-7",
    "camera-grid-8",
    "camera-grid-9",
    "call-controls-hover",
    "camera-chooser",
    "voice-call",
    "call-empty",
    "call-solo",
    "call-nine",
    "call-muted",
    "call-streaming",
    "voice-quality",
    "share-picker",
    "voice-consent",
    "voice-settings",
    "emoji-picker",
    "emoji-search",
    "emoji-autocomplete",
    "reaction-picker",
    "gif-picker",
    "gif-search",
    "gif-favorites",
    "image-viewer",
    "image-viewer-loading",
    "image-viewer-zoom",
    "upload-progress",
    "upload-storage-full",
    "upload-other-channel",
    "restart-update",
    "login",
    "register",
    "home-online",
    "home-all",
    "home-pending",
    "home-blocked",
    "dm-chat",
    "guild-chat",
    "group-chat",
    "community-picker",
    "create-guild-form",
    "join-guild",
    "invite",
    "guild-settings-overview",
    "guild-settings-channels",
    "guild-settings-members",
    "guild-settings-invites",
    "guild-settings-bans",
    "user-settings-account",
    "user-settings-profile",
    "user-settings-appearance",
    "context",
    "toast",
    "empty-home",
    "empty-channel",
    "loading",
    "disconnected",
    "mention-autocomplete",
    "edit-message",
    "message-hover",
    "message-hover-own",
    "message-hover-grouped",
    "add-friend",
    "google-login",
    "pick-username",
    "sign-in-methods",
    "markdown",
    "masked-link-confirm",
    "context-own-message",
    "change-server",
    "channel-top",
    "markdown-revealed",
    "member-menu",
    "password-settings",
    "sign-in-methods-unlinked",
    "sign-in-methods-google-only",
    "scroll-5000",
    "scroll-skeleton",
    "history-jump",
    "reply-preview",
];
pub const PICKER_SCREENS: &[&str] = &[
    "emoji-picker",
    "emoji-search",
    "emoji-autocomplete",
    "reaction-picker",
    "gif-picker",
    "gif-search",
    "gif-favorites",
];
pub const MEDIA_SCREENS: &[&str] = &[
    "emoji-picker",
    "emoji-search",
    "emoji-autocomplete",
    "reaction-picker",
    "gif-picker",
    "gif-search",
    "gif-favorites",
    "image-viewer",
    "image-viewer-loading",
    "image-viewer-zoom",
    "upload-progress",
    "upload-storage-full",
    "upload-other-channel",
    "restart-update",
    "inline-video-poster",
    "inline-video-playing",
    "inline-video-fullscreen",
];
pub const MAIN_SCREENS: &[&str] = &[
    "guild-chat",
    "main-server-menu",
    "main-profile",
    "main-profile-idle",
    "main-profile-dnd",
    "main-profile-offline",
    "main-no-voice",
    "main-members-hidden",
    "main-rail-unread",
    "main-own-profile",
    "main-pins",
    "main-notifications",
    "main-topic",
    "main-quick-switcher",
    "main-user-context",
    "main-channel-context",
    "main-server-context",
    "main-rail-context",
    "main-devices",
    "main-mutual-servers",
    "main-role-picker",
    "main-nickname",
    "main-rail-hover",
    "main-channel-hover",
    "main-category-hover",
    "main-category-collapsed",
    "main-muted-controls",
    "main-user-hover",
    "main-offline-hover",
    "main-drop-overlay",
    "main-local-search",
    "main-message-menu",
    "main-tooltip",
    "main-streaming",
    "main-history-extent",
    "main-history-skeleton",
    "main-server-menu-member",
    "main-last-long",
    "main-last-long-no-typing",
    "main-last-image",
    "main-search-focused",
    "main-rail-counts",
    "main-avatar-sizes",
];
pub fn run_if_requested() -> Option<Result<(), String>> {
    let args: Vec<_> = std::env::args().collect();
    #[cfg(feature = "wgpu")]
    if let Some(index) = args.iter().position(|v| v == "--scroll-bench") {
        return Some(crate::scroll_surface_bench::run(&args, index));
    }
    #[cfg(not(feature = "wgpu"))]
    if args.iter().any(|v| v == "--scroll-bench") {
        return Some(Err("--scroll-bench requires the wgpu feature".into()));
    }
    if let Some(i) = args.iter().position(|v| v == "--idle-proof") {
        return Some(
            args.get(i + 1)
                .ok_or_else(|| "Missing proof path".into())
                .and_then(|p| idle_proof(Path::new(p))),
        );
    }
    if let Some(index) = args.iter().position(|v| v == "--bench-resources") {
        return Some(crate::scroll_bench::resources(&args, index));
    }
    if let Some(index) = args.iter().position(|v| v == "--bench-scroll") {
        return Some(crate::scroll_bench::run(&args, index));
    }
    if let Some(index) = args.iter().position(|v| v == "--capture") {
        return Some((|| {
            if !args.iter().any(|v| v == "--mock") {
                return Err("Captures require --mock".into());
            }
            let path = args.get(index + 1).ok_or("Missing capture path")?;
            let screen = args
                .iter()
                .position(|v| v == "--screen")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
                .unwrap_or("guild-chat");
            load_fonts();
            let size = crate::window_size();
            render(
                screen,
                size.width as u32,
                size.height as u32,
                Path::new(path),
            )
        })());
    }
    let index = args.iter().position(|v| v == "--screenshots")?;
    Some((|| {
        if !args.iter().any(|v| v == "--mock") {
            return Err("--screenshots requires --mock".into());
        }
        let dir = args.get(index + 1).ok_or("Missing screenshot directory")?;
        let sizes = args
            .iter()
            .position(|v| v == "--sizes")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
            .unwrap_or("1280x800,1920x1080");
        load_fonts();
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut manifest = Vec::new();
        for size in sizes.split(',') {
            let (w, h) = size.split_once('x').ok_or("Expected WIDTHxHEIGHT")?;
            let (w, h): (u32, u32) = (
                w.parse().map_err(|_| "Invalid width")?,
                h.parse().map_err(|_| "Invalid height")?,
            );
            if !(940..=3840).contains(&w) || !(600..=2160).contains(&h) {
                return Err("Screenshot size is out of range".into());
            }
            let screens = if args.iter().any(|v| v == "--audio-only") {
                AUDIO_SCREENS
            } else if args.iter().any(|v| v == "--main-only") {
                MAIN_SCREENS
            } else if args.iter().any(|v| v == "--media-only") {
                MEDIA_SCREENS
            } else if args.iter().any(|v| v == "--pickers-only") {
                PICKER_SCREENS
            } else if args.iter().any(|v| v == "--settings-only") {
                crate::ui::settings::SCREENS
            } else {
                SCREENS
            };

            let filter = args
                .iter()
                .position(|v| v == "--screens")
                .and_then(|i| args.get(i + 1));
            for screen in screens
                .iter()
                .filter(|s| filter.is_none_or(|f| f.split(',').any(|name| name == **s)))
            {
                let name = format!("{screen}-{w}x{h}.png");
                let path = Path::new(dir).join(&name);
                render(screen, w, h, &path)?;
                use sha2::Digest;
                let hash = format!(
                    "{:x}",
                    sha2::Sha256::digest(std::fs::read(&path).map_err(|e| e.to_string())?)
                );
                manifest.push(serde_json::json!({"file":name,"width":w,"height":h,"sha256":hash}));
            }
        }
        let manifest = serde_json::json!({"renderer":"iced-tiny-skia","window_backend":"none","screens":manifest});
        std::fs::write(
            Path::new(dir).join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })())
}
pub(crate) fn load_fonts() {
    let mut system = iced_tiny_skia::graphics::text::font_system()
        .write()
        .unwrap();
    for bytes in [
        include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice(),
        include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice(),
        include_bytes!("../assets/fonts/Inter-Bold.ttf").as_slice(),
        include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice(),
        include_bytes!("../assets/fonts/Inter-Italic.ttf").as_slice(),
        include_bytes!("../assets/fonts/Inter-SemiBoldItalic.ttf").as_slice(),
        include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
    ] {
        system.load_font(std::borrow::Cow::Borrowed(bytes));
    }
}
pub fn render(screen: &str, w: u32, h: u32, path: &Path) -> Result<(), String> {
    let size = Size::new(w as f32, h as f32);
    let mut app = App::headless(screen, size);
    let mut cache = Cache::new();
    #[cfg(not(feature = "wgpu"))]
    let mut renderer = iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY));
    #[cfg(feature = "wgpu")]
    let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
        t::regular(),
        iced::Pixels(t::BODY),
    ));
    // Native layout sensors settle actual virtual-row heights; no timer, browser or application Task is run.
    // Full-screen settings have no virtual chat-row measurement sensors. Two
    // deterministic layout passes suffice; retain the chat settling budget elsewhere.
    let passes = if matches!(
        app.dialog,
        crate::app::Dialog::UserSettings
            | crate::app::Dialog::GuildSettings(_)
            | crate::app::Dialog::ChannelSettings(_)
    ) {
        2
    } else {
        12
    };
    for pass in 0..passes {
        app.headless_scroll();
        let y = app.scroll_y;
        let mut ui = UserInterface::build(crate::ui::view(&app), size, cache, &mut renderer);
        let mut scroll = operation::scrollable::scroll_to::<()>(
            "messages".into(),
            operation::scrollable::AbsoluteOffset {
                x: Some(0.),
                y: Some(y),
            },
        );
        ui.operate(&renderer, &mut scroll);
        // My Account is one page. These fixtures inspect its lower sections rather
        // than recreating the removed sidebar sub-pages.
        let settings_y = match screen {
            "audio-processing" => 950.,
            "user-settings-voice-processing" => 720.,
            "user-settings-voice-input" => 320.,
            "user-settings-password" => 560.,
            "user-settings-sign-in"
            | "sign-in-methods-unlinked"
            | "sign-in-methods-google-only" => 700.,
            "user-settings-account-status" => 880.,
            _ => 0.,
        };
        if settings_y > 0. {
            ui.operate(
                &renderer,
                &mut operation::scrollable::scroll_to::<()>(
                    "settings-content".into(),
                    operation::scrollable::AbsoluteOffset {
                        x: Some(0.),
                        y: Some(settings_y),
                    },
                ),
            );
        }
        let mut messages = Vec::new();
        let cursor = if screen == "audio-devices" {
            let (left, _) = t::settings::columns(w as f32);
            mouse::Cursor::Available(iced::Point::new(
                left + t::settings::CONTENT_GAP + t::S32,
                t::settings::TOP + t::S32 * 2. + t::S8,
            ))
        } else if screen == "main-tooltip" {
            mouse::Cursor::Available(iced::Point::new(
                t::RAIL + t::SIDEBAR - t::S16,
                h as f32 - t::USER_HEIGHT / 2.,
            ))
        } else if screen == "call-controls-hover" {
            mouse::Cursor::Available(iced::Point::new(
                (w as f32 + t::RAIL + t::SIDEBAR) / 2. - 2. * (t::GUILD_ICON + t::S12),
                h as f32 - t::S16 - t::GUILD_ICON / 2.,
            ))
        } else if screen.starts_with("main-history-") {
            mouse::Cursor::Available(iced::Point::new(
                w as f32 - t::MEMBERS - t::S4,
                h as f32 / 2.,
            ))
        } else {
            mouse::Cursor::Unavailable
        };
        let now = Instant::now();
        if cursor.position().is_some() {
            let _ = ui.update(
                &[iced::Event::Mouse(mouse::Event::CursorMoved {
                    position: cursor.position().unwrap(),
                })],
                cursor,
                &mut renderer,
                &mut iced::advanced::graphics::core::clipboard::Null,
                &mut messages,
            );
        }
        if screen == "audio-devices" && pass == 0 {
            let _ = ui.update(
                &[iced::Event::Mouse(mouse::Event::ButtonPressed(
                    mouse::Button::Left,
                ))],
                cursor,
                &mut renderer,
                &mut iced::advanced::graphics::core::clipboard::Null,
                &mut messages,
            );
        }
        let _ = ui.update(
            &[iced::Event::Window(iced::window::Event::RedrawRequested(
                now + std::time::Duration::from_millis(500),
            ))],
            cursor,
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        if pass + 1 == passes {
            ui.draw(
                &mut renderer,
                &t::theme(&app),
                &iced::advanced::renderer::Style {
                    text_color: t::TEXT_PRIMARY,
                },
                cursor,
            );
        }
        cache = ui.into_cache();
        for message in messages {
            if matches!(
                message,
                Msg::Measured { .. }
                    | Msg::HistoryViewport(_)
                    | Msg::Main(crate::main_window::Message::PopoverSize(_))
            ) {
                let _ = app.update(message);
            }
        }
    }
    #[cfg(feature = "wgpu")]
    let iced::Renderer::Secondary(mut renderer) = renderer
    else {
        unreachable!()
    };
    let rgba = iced_tiny_skia::window::compositor::screenshot(
        &mut renderer,
        &iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(w, h), 1.),
        t::BG_CHAT,
    );
    use image::{
        codecs::png::{CompressionType, FilterType, PngEncoder},
        ImageEncoder,
    };
    // Lossless fast encoding keeps the full parity suite practical in debug builds.
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    PngEncoder::new_with_quality(file, CompressionType::Fast, FilterType::Sub)
        .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())
}

/// Resource proof for a loaded, rendered view with no window/event-loop or media.
/// Report this as headless residency, not as a measurement of the GPU client.
#[cfg(windows)]
fn idle_proof(path: &Path) -> Result<(), String> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
        Threading::{GetCurrentProcess, GetProcessTimes},
    };
    load_fonts();
    let app = App::headless("guild-chat", Size::new(1920., 1080.));
    #[cfg(not(feature = "wgpu"))]
    let mut renderer = iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY));
    #[cfg(feature = "wgpu")]
    let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
        t::regular(),
        iced::Pixels(t::BODY),
    ));
    let mut ui = UserInterface::build(
        crate::ui::view(&app),
        Size::new(1920., 1080.),
        Cache::new(),
        &mut renderer,
    );
    ui.draw(
        &mut renderer,
        &t::theme(&app),
        &iced::advanced::renderer::Style {
            text_color: t::TEXT_PRIMARY,
        },
        mouse::Cursor::Unavailable,
    );
    #[cfg(feature = "wgpu")]
    let rgba = if let iced::Renderer::Secondary(raster) = &mut renderer {
        iced_tiny_skia::window::compositor::screenshot(
            raster,
            &iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(1920, 1080), 1.),
            t::BG_CHAT,
        )
    } else {
        unreachable!()
    };
    #[cfg(not(feature = "wgpu"))]
    let rgba = iced_tiny_skia::window::compositor::screenshot(
        &mut renderer,
        &iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(1920, 1080), 1.),
        t::BG_CHAT,
    );
    fn cpu() -> u64 {
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // Valid initialized out-pointers and pseudo-handle of our own process.
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            );
        }
        let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        ticks(kernel) + ticks(user)
    }
    let before = cpu();
    let wall = Instant::now();
    std::thread::sleep(std::time::Duration::from_secs(10));
    let after = cpu();
    let seconds = wall.elapsed().as_secs_f64();
    let mut mem = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // The EX layout begins with PROCESS_MEMORY_COUNTERS; size is supplied explicitly.
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut mem as *mut _ as *mut _,
            std::mem::size_of_val(&mem) as u32,
        )
    };
    if ok == 0 {
        return Err("Could not measure own process memory".into());
    }
    let proof = serde_json::json!({"mode":"headless rendered iced view; no OS window; no network or audio", "sample_seconds":seconds,"cpu_percent_one_core":(after-before) as f64/10_000_000./seconds*100.,"working_set_bytes":mem.WorkingSetSize,"private_bytes":mem.PrivateUsage,"messages":app.state.model.retained_count(),"renderer":"tiny-skia","raster_bytes":rgba.len()});
    std::fs::write(path, serde_json::to_vec_pretty(&proof).unwrap()).map_err(|e| e.to_string())?;
    drop(ui);
    drop(renderer);
    drop(app);
    Ok(())
}
#[cfg(not(windows))]
fn idle_proof(_: &Path) -> Result<(), String> {
    Err("Windows process resource proof is only available on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delete_confirmation_backdrop_blocks_settings_navigation() {
        load_fonts();
        let size = Size::new(1280., 800.);
        let app = App::headless("guild-settings-delete", size);
        #[cfg(not(feature = "wgpu"))]
        let mut renderer = iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY));
        #[cfg(feature = "wgpu")]
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            t::regular(),
            iced::Pixels(t::BODY),
        ));
        let mut ui = UserInterface::build(crate::ui::view(&app), size, Cache::new(), &mut renderer);
        let mut messages = Vec::new();
        let _ = ui.update(
            &[
                iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ],
            mouse::Cursor::Available(iced::Point::new(200., 198.)),
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        assert!(!messages.iter().any(|m| matches!(
            m,
            Msg::Settings(crate::settings_controller::Action::Server(_))
        )));
    }
    #[test]
    fn request_pending_and_sharer_consent_are_distinct_from_focused_stream() {
        load_fonts();
        let dir =
            std::env::temp_dir().join(format!("sticktalk-call-states-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut rasters = Vec::new();
        for screen in ["focused-stream", "request-control", "voice-consent"] {
            let path = dir.join(format!("{screen}.png"));
            render(screen, 1280, 800, &path).unwrap();
            rasters.push(image::open(&path).unwrap().into_rgba8());
            std::fs::remove_file(path).unwrap();
        }
        assert_ne!(
            rasters[0], rasters[1],
            "viewer request must visibly acknowledge pending consent"
        );
        assert_ne!(
            rasters[0], rasters[2],
            "sharer consent must render above the call"
        );
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn offscreen_png_has_exact_size_and_neutral_rgba_tokens() {
        load_fonts();
        let path =
            std::env::temp_dir().join(format!("sticktalk-offscreen-{}.png", std::process::id()));
        render("login", 1280, 800, &path).unwrap();
        let png = image::open(&path).unwrap().into_rgba8();
        assert_eq!(png.dimensions(), (1280, 800));
        assert_eq!(png.get_pixel(0, 0).0, [0x11, 0x11, 0x13, 255]);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn tooltip_is_anchored_to_the_icon_not_the_viewport() {
        load_fonts();
        let path =
            std::env::temp_dir().join(format!("sticktalk-tooltip-{}.png", std::process::id()));
        render("main-tooltip", 1280, 800, &path).unwrap();
        let png = image::open(&path).unwrap().into_rgba8();
        assert_eq!(png.get_pixel(4, 710).0, [0x11, 0x11, 0x13, 255]);
        let mut dark = 0;
        for y in 680..750 {
            for x in 200..360 {
                if png.get_pixel(x, y).0 == [0x11, 0x12, 0x14, 255] {
                    dark += 1;
                }
            }
        }
        assert!(dark > 200, "tooltip did not appear above the settings icon");
        let _ = std::fs::remove_file(path);
    }
    use crate::scroll::test_geometry::Geometry;
    fn settle(app: &mut App, size: Size, mut cache: Cache) -> (Cache, Geometry) {
        #[cfg(feature = "wgpu")]
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            t::regular(),
            iced::Pixels(t::BODY),
        ));
        #[cfg(not(feature = "wgpu"))]
        let mut renderer = iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY));
        let mut geometry = Geometry::default();
        for _ in 0..16 {
            app.headless_scroll();
            let y = app.scroll_y;
            let mut ui = UserInterface::build(crate::ui::view(app), size, cache, &mut renderer);
            ui.operate(
                &renderer,
                &mut operation::scrollable::scroll_to::<()>(
                    "messages".into(),
                    operation::scrollable::AbsoluteOffset {
                        x: Some(0.),
                        y: Some(y),
                    },
                ),
            );
            let mut messages = Vec::new();
            let _ = ui.update(
                &[iced::Event::Window(iced::window::Event::RedrawRequested(
                    Instant::now(),
                ))],
                mouse::Cursor::Unavailable,
                &mut renderer,
                &mut iced::advanced::graphics::core::clipboard::Null,
                &mut messages,
            );
            geometry = Geometry {
                last_id: "message-2279".into(),
                ..Default::default()
            };
            ui.operate(&renderer, &mut geometry);
            cache = ui.into_cache();
            for m in messages {
                if matches!(
                    m,
                    Msg::Measured { .. }
                        | Msg::HistoryViewport(_)
                        | Msg::Main(crate::main_window::Message::PopoverSize(_))
                ) {
                    let _ = app.update(m);
                }
            }
        }
        (cache, geometry)
    }
    #[test]
    fn newest_message_clears_composer_with_long_text_images_and_typing_toggles() {
        load_fonts();
        for size in [Size::new(1280., 800.), Size::new(1920., 1080.)] {
            for screen in ["guild-chat", "main-last-long", "main-last-image"] {
                let mut app = App::headless(screen, size);
                let (cache, g) = settle(&mut app, size, Cache::new());
                assert_bottom(&app, &g, screen);
                let typing = app.state.model.typing.clone();
                app.state.model.typing.clear();
                let (cache, g) = settle(&mut app, size, cache);
                assert_bottom(&app, &g, screen);
                app.state.model.typing = typing;
                let (_, g) = settle(&mut app, size, cache);
                assert_bottom(&app, &g, screen);
            }
        }
    }
    #[test]
    fn pending_sparse_seek_does_not_replace_loaded_messages_with_skeletons() {
        load_fonts();
        let size = Size::new(1280., 800.);
        let mut app = App::headless("guild-chat", size);
        app.extent.channel = Some(sticktalk_proto::Id(12));
        app.extent.count = 10_000;
        app.extent.start_rank = 500;
        app.extent.loaded = false;
        app.extent.ends_at_present = true;
        app.state.at_present = true;
        let (_, geometry) = settle(&mut app, size, Cache::new());
        assert!(
            geometry.last.height > 0.,
            "loaded last message was replaced by a skeleton during a pending seek"
        );
    }
    #[test]
    fn profile_popouts_measure_natural_height_before_clamping_at_the_bottom() {
        load_fonts();
        for size in [Size::new(1280., 800.), Size::new(1920., 1080.)] {
            for screen in [
                "main-own-profile",
                "main-profile-idle",
                "main-profile-offline",
                "main-user-context",
                "main-message-menu",
            ] {
                let mut app = App::headless(screen, size);
                app.main.position = iced::Point::new(size.width - t::S8, size.height - t::S8);
                let (_, g) = settle(&mut app, size, Cache::new());
                assert!(g.popover.width > 0., "{screen}: popover not found");
                assert!(
                    g.popover.x >= t::S8 - 1.
                        && g.popover.y >= t::S8 - 1.
                        && g.popover.x + g.popover.width <= size.width - t::S8 + 1.
                        && g.popover.y + g.popover.height <= size.height - t::S8 + 1.,
                    "{screen}: popover outside window: {:?}",
                    g.popover
                );
                if screen == "main-own-profile" {
                    assert!(
                        g.popover.height > 400.,
                        "own profile was constrained before measuring: {}",
                        g.popover.height
                    );
                }
            }
        }
    }
    fn assert_bottom(app: &App, g: &Geometry, screen: &str) {
        assert!(
            (g.content.height - app.history_height()).abs() < 2.,
            "{screen}: content {} model {}",
            g.content.height,
            app.history_height()
        );
        assert!(
            (g.offset.y + g.viewport.height - g.content.height).abs() < 2.,
            "{screen}: not at bottom"
        );
        assert!(g.last.height > 0., "{screen}: last row not rendered");
        // Native operations report row bounds in content coordinates; account for the scroll translation.
        let bottom = g.last.y + g.last.height - g.offset.y;
        let gap = g.composer.y + t::S8 - bottom;
        assert!(
            (16. - 2. ..=24. + 2.).contains(&gap),
            "{screen}: last-message/composer gap {gap}, row {:?}, scroll {}",
            g.last,
            g.offset.y
        );
    }
}
