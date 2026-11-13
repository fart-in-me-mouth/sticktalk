//! Offscreen rendering of the real iced view tree. This module never constructs a winit window.
use crate::{
    app::{App, Msg},
    theme as t,
};
use iced::{advanced::widget::operation, mouse, Size};
use iced_runtime::user_interface::{Cache, UserInterface};
use std::{path::Path, time::Instant};
pub const SCREENS: &[&str] = &[
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
            for screen in SCREENS {
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
    for pass in 0..12 {
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
        if pass == 11 {
            ui.draw(
                &mut renderer,
                &t::theme(&app),
                &iced::advanced::renderer::Style {
                    text_color: t::TEXT_PRIMARY,
                },
                mouse::Cursor::Unavailable,
            );
        }
        cache = ui.into_cache();
        for message in messages {
            if matches!(message, Msg::Measured { .. }) {
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
    image::save_buffer(path, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
