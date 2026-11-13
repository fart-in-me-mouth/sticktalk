#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod agent_window;
mod app;
mod audio_settings;
mod desktop_host;
mod features;
mod video;
use sticktalk::backend;
use sticktalk::{animated, gifs, platform_glue, uploads};
mod datetime;
mod focus_button;
mod headless;
mod icons;
use sticktalk::markdown;
use sticktalk::media;
mod history_extent;
mod main_window;
mod message_layout;
use sticktalk::mock;
use sticktalk::model;
use sticktalk::oauth;
mod scroll;
mod scroll_bench;
#[cfg(feature = "wgpu")]
mod scroll_surface_bench;
use sticktalk::storage;
mod settings_controller;
mod theme;
mod ui;
mod voice_ui;
#[cfg(feature = "compact-allocator")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;
fn main() -> iced::Result {
    std::env::set_var("TOKIO_WORKER_THREADS", "2");
    // Keep DX12: Vulkan reduced commit but regressed the text-scroll benchmark. iced's compositor
    // still falls back to tiny-skia if initialization fails. Respect overrides.
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
    let _host = match platform_glue::bootstrap() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if !_host.should_run {
        return Ok(());
    }
    let dry = std::env::args().any(|a| a == "--mock") || agent_window::enabled();
    let _diagnostics =
        (!dry).then(|| sticktalk_desktop_platform::diagnostics::initialize(&storage::root()));
    #[cfg(windows)]
    if !dry
        && std::env::var_os("STICKTALK_UPDATE_HEALTH_FILE").is_none()
        && storage::root()
            .join("Updates/rollback/version.txt")
            .is_file()
    {
        use std::os::windows::process::CommandExt;
        if let Ok(exe) = std::env::current_exe() {
            let runner = storage::root().join("Updates/recover.exe");
            if std::fs::copy(exe.with_file_name("sticktalk-maintenance.exe"), &runner).is_ok()
                && std::process::Command::new(runner)
                    .arg("recover")
                    .arg(&exe)
                    .arg(storage::root().join("Updates"))
                    .arg(std::process::id().to_string())
                    .creation_flags(0x08000000)
                    .spawn()
                    .is_ok()
            {
                return Ok(());
            }
        }
    }
    if !dry && platform_glue::apply_pending_at_startup() {
        return Ok(());
    }
    let result = iced::daemon(
        move || {
            let (mut app, init) = app::App::new();
            let (id, open) = iced::window::open(iced::window::Settings {
                size: window_size(),
                icon: image::load_from_memory(include_bytes!("../assets/sticktalk.png"))
                    .ok()
                    .and_then(|i| {
                        let rgba = i.into_rgba8();
                        let (w, h) = rgba.dimensions();
                        iced::window::icon::from_rgba(rgba.into_raw(), w, h).ok()
                    }),
                visible: (!cfg!(windows) || dry)
                    && !agent_window::enabled()
                    && !std::env::args().any(|a| a == "--background"),
                min_size: Some(iced::Size::new(theme::MIN_W, theme::MIN_H)),
                exit_on_close_request: false,
                ..Default::default()
            });
            app.calls.main_window = Some(id);
            (
                app,
                open.then({
                    let mut init = Some(init);
                    move |_| init.take().unwrap_or_else(iced::Task::none)
                }),
            )
        },
        app::App::update,
        ui::view_window,
    )
    .executor::<platform_glue::Executor>()
    .title("StickTalk")
    .theme(|app: &app::App, _| theme::theme(app))
    .subscription(app::App::subscription)
    .default_font(theme::regular())
    .font(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice())
    .font(include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice())
    .font(include_bytes!("../assets/fonts/Inter-Bold.ttf").as_slice())
    .font(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice())
    .font(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice())
    .font(include_bytes!("../assets/fonts/Inter-Italic.ttf").as_slice())
    .font(include_bytes!("../assets/fonts/Inter-SemiBoldItalic.ttf").as_slice())
    .antialiasing(false)
    .run();
    if result.is_err() {
        sticktalk_desktop_platform::diagnostics::record_failure(
            &storage::root(),
            "The desktop window could not initialize.",
        );
    }
    result
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
