//! Iced adapter for explicit desktop effects. Mock/headless/agent runs never
//! register a tray, touch HKCU, activate a window, or construct notifications.
use crate::app::{App, Msg};
use iced::{Subscription, Task};
use sticktalk_desktop_platform::tray::TrayAction;
#[derive(Clone, Debug)]
pub enum Event {
    Installed(bool),
    Tray(TrayAction),
    Close,
}
fn message(event: Event) -> Msg {
    Msg::feature(crate::features::Event::Desktop(event))
}
type Channel = (
    tokio::sync::mpsc::Sender<TrayAction>,
    tokio::sync::Mutex<tokio::sync::mpsc::Receiver<TrayAction>>,
);
fn channel() -> &'static Channel {
    static CHANNEL: std::sync::OnceLock<Channel> = std::sync::OnceLock::new();
    CHANNEL.get_or_init(|| {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        (tx, tokio::sync::Mutex::new(rx))
    })
}
fn events(_: &()) -> impl iced::futures::Stream<Item = Msg> {
    use iced::futures::SinkExt;
    iced::stream::channel(
        8,
        |mut output: iced::futures::channel::mpsc::Sender<Msg>| async move {
            let mut receiver = channel().1.lock().await;
            while let Some(event) = receiver.recv().await {
                if output.send(message(Event::Tray(event))).await.is_err() {
                    break;
                }
            }
        },
    )
}
pub fn subscription(app: &App) -> Subscription<Msg> {
    if app.mock_mode || crate::agent_window::enabled() || cfg!(test) {
        Subscription::none()
    } else {
        Subscription::run_with((), events)
    }
}
pub fn initialize(dry: bool) -> Task<Msg> {
    if dry || cfg!(test) {
        return Task::none();
    }
    iced::window::oldest()
        .and_then(|id| iced::window::run(id, install))
        .map(|ok| message(Event::Installed(ok)))
}
fn install(window: &dyn iced_runtime::Window) -> bool {
    #[cfg(windows)]
    {
        use iced_runtime::window::raw_window_handle::RawWindowHandle;
        let Ok(handle) = window.window_handle() else {
            return false;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return false;
        };
        let hwnd = windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _);
        sticktalk_desktop_platform::windows::host::install(hwnd, channel().0.clone()).is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        false
    }
}
fn window_action(action: TrayAction, activate: bool) -> Task<Msg> {
    iced::window::oldest()
        .and_then(move |id| {
            iced::window::run(id, move |window| {
                #[cfg(windows)]
                {
                    use iced_runtime::window::raw_window_handle::RawWindowHandle;
                    if let Ok(handle) = window.window_handle() {
                        if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                            let hwnd =
                                windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _);
                            match action {
                                TrayAction::Open => {
                                    if activate {
                                        sticktalk_desktop_platform::windows::focus_window(hwnd)
                                    } else {
                                        sticktalk_desktop_platform::windows::show_window_inactive(
                                            hwnd,
                                        )
                                    }
                                }
                                TrayAction::Hide => {
                                    sticktalk_desktop_platform::windows::hide_window(hwnd)
                                }
                                _ => {}
                            }
                        }
                    }
                }
                #[cfg(not(windows))]
                let _ = (window, action, activate);
            })
        })
        .map(|_| Msg::Activity)
}
pub fn update(app: &mut App, event: Event) -> Task<Msg> {
    if app.mock_mode || crate::agent_window::enabled() || cfg!(test) {
        return Task::none();
    }
    match event {
        Event::Installed(ok) => {
            if !ok {
                app.error = "Tray unavailable; closing StickTalk will quit.".into();
                if std::env::args().any(|a| a == "--background") {
                    return window_action(TrayAction::Open, false);
                }
            }
        }
        Event::Tray(TrayAction::Quit) => return iced::exit(),
        Event::Tray(TrayAction::Mute) => return app.main_update(crate::main_window::Message::Mute),
        Event::Tray(TrayAction::Deafen) => {
            return app.main_update(crate::main_window::Message::Deafen)
        }
        Event::Tray(action) => return window_action(action, true),
        Event::Close => {
            #[cfg(windows)]
            if sticktalk_desktop_platform::windows::host::available() {
                return window_action(TrayAction::Hide, false);
            }
            return iced::exit();
        }
    }
    Task::none()
}
pub fn startup(enabled: bool, dry: bool) -> Result<(), String> {
    if dry || cfg!(test) {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        sticktalk_desktop_platform::windows::set_start_with_windows(&exe, enabled)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err("Start with Windows requires Windows".into())
    }
}
pub fn sync(app: &App) {
    #[cfg(windows)]
    if !app.mock_mode && !crate::agent_window::enabled() && !cfg!(test) {
        sticktalk_desktop_platform::windows::host::state(
            app.calls.call.muted,
            app.calls.call.deafened,
        );
    }
    #[cfg(not(windows))]
    let _ = app;
}
