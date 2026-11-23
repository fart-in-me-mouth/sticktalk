//! A stream popped out of a call into a window of its own, so it can sit on
//! another monitor or fill one while the chat stays in the main window.
//!
//! The server's page asks for the window, the app makes it (hidden, with a
//! blank WebView from the same WebView2 environment), and the page's next
//! `window.open("")` is given that WebView. The page then puts its video in
//! it. Nothing else is ever a second in-app window:
//!
//! - only the server's own page is listened to, and only one window exists;
//! - the window stays on its blank page: it cannot navigate, open windows of
//!   its own, or be granted the microphone, camera or anything else;
//! - full screen is the window's own: the page asks for it the usual web way.
use crate::{app::UserEvent, webview};
use serde::Deserialize;
use std::cell::{Cell, RefCell};
use tao::{
    dpi::{PhysicalPosition, PhysicalSize},
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    platform::windows::{IconExtWindows, WindowBuilderExtWindows},
    window::{Fullscreen, Icon, Window, WindowBuilder, WindowId},
};
use webview2_com::{
    ContainsFullScreenElementChangedEventHandler, Microsoft::Web::WebView2::Win32::*,
    PermissionRequestedEventHandler,
};
use windows::core::BOOL;
use wry::{
    NewWindowResponse, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows,
};

const TITLE: &str = "StickTalk";
/// The only page the window ever shows; the opener writes into it.
pub const BLANK: &str = "about:blank";
/// 16:9 at 1280 wide, or as much of that as fits the screen.
const WIDTH: f64 = 1280.0;
const MIN_SIZE: (u32, u32) = (320, 180);
/// A page cannot queue more requests than this before the app has acted.
const MAX_QUEUED: u8 = 4;

/// What the page may say to the app about the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "window", rename_all = "snake_case")]
pub enum Request {
    /// Make the window; the page opens into it once told it is ready.
    Stream,
    Close,
}

pub fn parse(raw: &str) -> Option<Request> {
    if raw.len() > 64 {
        return None;
    }
    serde_json::from_str(raw).ok()
}

/// What the page named the window, made safe for a title bar.
pub fn title(page: &str) -> String {
    let name: String = page.chars().filter(|c| !c.is_control()).take(80).collect();
    let name = name.trim();
    if name.is_empty() || name == BLANK {
        TITLE.into()
    } else {
        format!("{name} - {TITLE}")
    }
}

/// Size and position on a monitor: 16:9, at most 80% of it, centered.
pub fn placement(
    monitor_at: (i32, i32),
    monitor: (u32, u32),
    scale: f64,
) -> ((u32, u32), (i32, i32)) {
    let room = (f64::from(monitor.0) * 0.8, f64::from(monitor.1) * 0.8);
    let width = (WIDTH * scale).min(room.0).min(room.1 * 16.0 / 9.0);
    let width = (width as u32).max(MIN_SIZE.0);
    let height = (width * 9 / 16).max(MIN_SIZE.1);
    let x = monitor_at.0 + (monitor.0.saturating_sub(width) / 2) as i32;
    let y = monitor_at.1 + (monitor.1.saturating_sub(height) / 2) as i32;
    ((width, height), (x, y))
}

/// Shared by the event loop and the main WebView's callbacks, which all run
/// on the one UI thread.
#[derive(Default)]
pub struct Slot {
    /// A window made and not yet opened into.
    offered: RefCell<Option<ICoreWebView2>>,
    queued: Cell<u8>,
}

impl Slot {
    /// A request from the page is on its way to the event loop, unless the
    /// page has already queued too many.
    pub fn admit(&self) -> bool {
        if self.queued.get() >= MAX_QUEUED {
            return false;
        }
        self.queued.set(self.queued.get() + 1);
        true
    }

    /// The event loop took one request.
    pub fn taken(&self) {
        self.queued.set(self.queued.get().saturating_sub(1));
    }

    /// The page's `window.open`: the window made for it, once.
    pub fn claim(&self, url: &str, from_server: bool) -> NewWindowResponse {
        if !from_server || !(url.is_empty() || url == BLANK) {
            return NewWindowResponse::Deny;
        }
        let offered = self.offered.borrow_mut().take();
        offered.map_or(NewWindowResponse::Deny, |webview| {
            NewWindowResponse::Create { webview }
        })
    }

    fn withdraw(&self) {
        self.offered.borrow_mut().take();
    }
}

pub struct StreamWindow {
    // Declared first, so dropped first: the WebView goes before its window.
    webview: WebView,
    window: Window,
}

impl StreamWindow {
    /// Makes the window, hidden, and offers its WebView to the page's next
    /// `window.open`. `hidden` keeps it hidden for good (smoke runs).
    pub fn offer(
        target: &EventLoopWindowTarget<UserEvent>,
        main: &Window,
        opener: &WebView,
        slot: &Slot,
        proxy: EventLoopProxy<UserEvent>,
        hidden: bool,
    ) -> Result<Self, String> {
        slot.withdraw();
        let mut builder = WindowBuilder::new()
            .with_title(TITLE)
            // Shown once the page has opened into it.
            .with_visible(false)
            .with_focused(false)
            .with_min_inner_size(PhysicalSize::new(MIN_SIZE.0, MIN_SIZE.1))
            .with_background_color((0, 0, 0, 0xff))
            .with_window_icon(Icon::from_resource(1, None).ok())
            .with_drag_and_drop(false);
        if hidden {
            builder = builder.with_skip_taskbar(true).with_focusable(false);
        }
        // On the screen the main window is on.
        if let Some(monitor) = main.current_monitor() {
            let (size, at) = placement(
                monitor.position().into(),
                monitor.size().into(),
                monitor.scale_factor(),
            );
            builder = builder
                .with_inner_size(PhysicalSize::new(size.0, size.1))
                .with_position(PhysicalPosition::new(at.0, at.1));
        }
        let window = builder.build(target).map_err(|e| e.to_string())?;
        let webview = WebViewBuilder::new()
            .with_environment(opener.environment())
            .with_background_color((0, 0, 0, 0xff))
            .with_focused(false)
            .with_navigation_handler(|url| url == BLANK)
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_document_title_changed_handler({
                let proxy = proxy.clone();
                move |title| {
                    let _ = proxy.send_event(UserEvent::StreamTitle(title));
                }
            })
            .build(&window)
            .map_err(|e| e.to_string())?;
        unsafe { hook(&webview, proxy) }.map_err(|e| e.to_string())?;
        *slot.offered.borrow_mut() = Some(webview.webview());
        Ok(Self { webview, window })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// The page has opened into the window.
    pub fn show(&self) {
        self.window.set_visible(true);
        self.window.set_focus();
        let _ = self.webview.focus();
    }

    pub fn set_title(&self, page: &str) {
        self.window.set_title(&title(page));
    }

    pub fn set_fullscreen(&self, on: bool) {
        self.window
            .set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
    }
}

unsafe fn hook(webview: &WebView, proxy: EventLoopProxy<UserEvent>) -> windows::core::Result<()> {
    let core = webview.webview();
    let mut token = 0i64;
    // The page's own "full screen" makes the window full screen, and back.
    core.add_ContainsFullScreenElementChanged(
        &ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
            let Some(sender) = sender else { return Ok(()) };
            let mut on = BOOL::default();
            sender.ContainsFullScreenElement(&mut on)?;
            let _ = proxy.send_event(UserEvent::StreamFullscreen(on.as_bool()));
            Ok(())
        })),
        &mut token,
    )?;
    // A window that only shows a video is granted nothing.
    core.add_PermissionRequested(
        &PermissionRequestedEventHandler::create(Box::new(|_, args| {
            if let Some(args) = args {
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
            }
            Ok(())
        })),
        &mut token,
    )?;
    webview::tidy_context_menu(&core)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_known_requests_parse() {
        assert_eq!(parse(r#"{"window":"stream"}"#), Some(Request::Stream));
        assert_eq!(parse(r#"{"window":"close"}"#), Some(Request::Close));
        assert_eq!(parse(r#"{"window":"other"}"#), None);
        assert_eq!(parse(r#"{"control":"stop"}"#), None);
        assert_eq!(
            parse(&format!(r#"{{"window":"stream"{}}}"#, " ".repeat(64))),
            None
        );
    }

    #[test]
    fn titles_are_short_plain_and_never_empty() {
        assert_eq!(title("Moggity's screen"), "Moggity's screen - StickTalk");
        assert_eq!(title("  "), "StickTalk");
        assert_eq!(title("about:blank"), "StickTalk");
        assert_eq!(title("a\u{0}\n\tb"), "ab - StickTalk");
        assert_eq!(title(&"x".repeat(500)).chars().count(), 80 + 12);
    }

    #[test]
    fn the_window_is_sixteen_by_nine_centered_and_fits_the_screen() {
        // A 1080p screen: the full 1280x720.
        assert_eq!(
            placement((0, 0), (1920, 1080), 1.0),
            ((1280, 720), (320, 180))
        );
        // A second monitor to the left, at 150%: 80% of its width.
        let (size, at) = placement((-2560, 0), (2560, 1440), 1.5);
        assert_eq!(size, (1920, 1080));
        assert_eq!(at, (-2560 + 320, 180));
        // A small screen: 80% of it, still 16:9.
        assert_eq!(placement((0, 0), (1024, 768), 1.0).0, (819, 460));
        // A tall one is limited by its width, a wide one by its height.
        assert_eq!(placement((0, 0), (800, 2000), 1.0).0, (640, 360));
        assert_eq!(placement((0, 0), (4000, 500), 1.0).0, (711, 399));
        // Never smaller than the minimum.
        assert_eq!(placement((0, 0), (100, 100), 1.0).0, (320, 180));
    }

    #[test]
    fn the_page_cannot_queue_requests_without_end() {
        let slot = Slot::default();
        for _ in 0..MAX_QUEUED {
            assert!(slot.admit());
        }
        assert!(!slot.admit());
        slot.taken();
        assert!(slot.admit());
    }

    #[test]
    fn with_no_window_made_every_window_open_is_refused() {
        let slot = Slot::default();
        for (url, from_server) in [
            ("about:blank", true),
            ("", true),
            ("https://example.com/", true),
        ] {
            assert!(matches!(
                slot.claim(url, from_server),
                NewWindowResponse::Deny
            ));
        }
    }
}
