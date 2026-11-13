#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod agent_window;
mod app;
mod audio_settings;
use sticktalk::backend;
mod datetime;
mod focus_button;
mod headless;
mod icons;
use sticktalk::markdown;
use sticktalk::media;
mod message_layout;
use sticktalk::mock;
use sticktalk::model;
use sticktalk::oauth;
mod scroll;
mod scroll_bench;
#[cfg(feature = "wgpu")]
mod scroll_surface_bench;
use sticktalk::storage;
mod theme;
mod ui;
mod voice_ui;
fn main() -> iced::Result {
    std::env::set_var("TOKIO_WORKER_THREADS", "2");
    // Avoid loading both Vulkan and D3D driver stacks on Windows. iced's compositor
    // still falls back to tiny-skia if D3D12 initialization fails. Respect overrides.
    #[cfg(all(windows, feature = "wgpu"))]
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "dx12");
    }
    // DX12 advertises Mailbox on Windows. Combined with monitor-specific scroll
    // deadlines this avoids FIFO pacing a secondary window at the primary rate.
    #[cfg(all(windows, feature = "wgpu"))]
    if std::env::var_os("ICED_PRESENT_MODE").is_none()
        && std::env::var("WGPU_BACKEND").is_ok_and(|v| v == "dx12")
    {
        std::env::set_var("ICED_PRESENT_MODE", "mailbox");
    }
    if let Some(result) = headless::run_if_requested() {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    iced::application(app::App::new, app::App::update, ui::view)
        .title("StickTalk")
        .theme(theme::theme)
        .subscription(app::App::subscription)
        .window(iced::window::Settings {
            size: window_size(),
            visible: !agent_window::enabled(),
            min_size: Some(iced::Size::new(theme::MIN_W, theme::MIN_H)),
            ..iced::window::Settings::default()
        })
        .default_font(theme::regular())
        .font(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-Italic.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-SemiBoldItalic.ttf").as_slice())
        .antialiasing(false)
        .run()
}

fn window_size() -> iced::Size {
    let args: Vec<_> = std::env::args().collect();
    args.iter()
        .position(|v| v == "--size")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.split_once('x'))
        .and_then(|(w, h)| Some(iced::Size::new(w.parse().ok()?, h.parse().ok()?)))
        .unwrap_or(iced::Size::new(theme::DEFAULT_W, theme::DEFAULT_H))
}
