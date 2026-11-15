//! One window, one WebView, one event loop that sleeps until something happens.
use crate::{
    address, health,
    links::{self, Launch},
    policy::{Decision, Policy},
    shell_page::{self, lock, Form, Shared, SharedState},
    webview, window_state,
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use sticktalk_desktop_platform::{
    activation::Activation,
    windows::instance::{Instance, Launch as InstanceLaunch},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::{
        run_return::EventLoopExtRunReturn,
        windows::{IconExtWindows, WindowBuilderExtWindows, WindowExtWindows},
    },
    window::{Icon, WindowBuilder},
};
use windows::{
    core::{w, HSTRING, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, ShellExecuteW},
            WindowsAndMessaging::{
                AppendMenuW, GetSystemMenu, MessageBoxW, MB_ICONERROR, MB_OK, MF_SEPARATOR,
                MF_STRING, SW_SHOWNORMAL, WM_NCDESTROY, WM_SYSCOMMAND,
            },
        },
    },
};
use wry::WebContext;

const TITLE: &str = "StickTalk";
/// How long a smoke run waits for the page before giving up.
const SMOKE_TIMEOUT: Duration = Duration::from_secs(20);
/// With a script or a screenshot, a smoke run reports once the page has been
/// quiet this long (`--smoke-wait` overrides it).
const SMOKE_SETTLE: Duration = Duration::from_millis(1500);
/// Window placement is written once the moving or resizing has stopped.
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// A page cannot open browser tabs faster than a person clicks links.
const EXTERNAL_INTERVAL: Duration = Duration::from_millis(400);

pub enum UserEvent {
    /// A second launch or a `sticktalk://` link.
    Activation(Activation),
    /// Ctrl+Shift+S or the window menu's "Change server…".
    ChangeServer,
    OpenExternal(String),
    Title(String),
    Loaded(Load),
    Captured(bool),
}

/// A finished (or failed) top-level page load.
pub struct Load {
    pub ok: bool,
    /// Stopped before it loaded anything: an external link, a download.
    pub cancelled: bool,
    pub url: String,
    pub title: String,
    pub failure: Option<health::Failure>,
    /// WebView2's own error and HTTP status numbers, for smoke-run diagnostics.
    pub detail: (i32, i32),
}

/// `%LOCALAPPDATA%\StickTalk`: the previous client's folder, so the remembered
/// server and window position carry over. Never next to the executable.
fn data_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("StickTalk")
}

fn random_token() -> String {
    let mut bytes = [0u8; 16];
    // Without randomness there is no token, and `connect` refuses everything.
    if getrandom::fill(&mut bytes).is_err() {
        return String::new();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The one line a smoke run prints.
fn smoke_report(ok: bool, url: &str, title: &str, error: Option<&str>) {
    let mut line = serde_json::json!({ "ok": ok, "url": url, "title": title });
    if let Some(error) = error {
        line["error"] = error.into();
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn open_external(url: &str) {
    // `url` was parsed and re-serialized by the policy: http(s) or mailto only.
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

const CHANGE_SERVER_COMMAND: usize = 0x0A10;
const MENU_SUBCLASS: usize = 0x5354;

/// Adds "Change server…" to the title bar menu (right-click or Alt+Space).
unsafe fn add_window_menu(hwnd: HWND, proxy: EventLoopProxy<UserEvent>) {
    let menu = GetSystemMenu(hwnd, false);
    if menu.is_invalid() {
        return;
    }
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        CHANGE_SERVER_COMMAND,
        w!("Change server…\tCtrl+Shift+S"),
    );
    let proxy = Box::into_raw(Box::new(proxy));
    if !SetWindowSubclass(hwnd, Some(window_menu_proc), MENU_SUBCLASS, proxy as usize).as_bool() {
        drop(Box::from_raw(proxy));
    }
}

unsafe extern "system" fn window_menu_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let proxy = data as *mut EventLoopProxy<UserEvent>;
    match message {
        WM_SYSCOMMAND if wparam.0 & 0xFFF0 == CHANGE_SERVER_COMMAND => {
            let _ = (*proxy).send_event(UserEvent::ChangeServer);
            return LRESULT(0);
        }
        WM_NCDESTROY => {
            let _ = RemoveWindowSubclass(hwnd, Some(window_menu_proc), MENU_SUBCLASS);
            drop(Box::from_raw(proxy));
        }
        _ => {}
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

/// Becomes the only instance, or hands this launch to the running one.
fn single_instance(
    root: &Path,
    activation: Activation,
) -> Result<Option<(tokio::runtime::Runtime, Instance)>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    match runtime
        .block_on(Instance::launch(root, activation))
        .map_err(|e| e.to_string())?
    {
        InstanceLaunch::Forwarded => Ok(None),
        InstanceLaunch::Primary(instance) => Ok(Some((runtime, instance))),
    }
}

/// Waits (asleep, on the pipe) for later launches and passes them to the
/// window. Notifying `stop` ends it and releases the instance lock.
fn forward_activations(
    runtime: tokio::runtime::Runtime,
    mut instance: Instance,
    proxy: EventLoopProxy<UserEvent>,
    stop: Arc<tokio::sync::Notify>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        runtime.block_on(async move {
            loop {
                let received = tokio::select! {
                    received = instance.receive() => received,
                    _ = stop.notified() => break,
                };
                match received {
                    Ok(activation) => {
                        if proxy.send_event(UserEvent::Activation(activation)).is_err() {
                            break;
                        }
                    }
                    // A malformed or abandoned launch. Don't spin on a broken pipe.
                    Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
                }
            }
        })
    })
}

/// `--smoke`: the same app, never shown, with a throwaway data folder. It
/// reports the last page load on one line and exits.
struct Smoke {
    deadline: Instant,
    /// Script to run in the first page, to drive the built-in page in tests.
    eval: Option<String>,
    shot: Option<PathBuf>,
    /// `(ok, url, title)` of the most recent load.
    last: Option<(bool, String, String)>,
    settle: Duration,
    /// When the page has been quiet long enough to report.
    finish_at: Option<Instant>,
}

impl Smoke {
    fn report(&self) -> i32 {
        match &self.last {
            Some((ok, url, title)) => {
                smoke_report(*ok, url, title, (!ok).then_some("the page did not load"));
                i32::from(!ok)
            }
            None => 2,
        }
    }
}

pub fn run(launch: Launch) -> i32 {
    let Launch {
        smoke,
        smoke_eval,
        smoke_wait,
        smoke_shot,
        smoke_root,
        background,
        activation,
    } = launch;
    let is_smoke = smoke.is_some();
    // A smoke run is invisible to the installed app: its own data folder, and
    // so its own WebView profile, saved address and instance lock.
    let own_root = is_smoke && smoke_root.is_none();
    let root = match smoke_root {
        Some(root) if is_smoke => root,
        _ if is_smoke => {
            std::env::temp_dir().join(format!("StickTalk-smoke-{}", std::process::id()))
        }
        _ => data_root(),
    };
    let page = url::Url::parse(shell_page::PAGE).expect("built-in page address");

    // Where to start, and what the window may show.
    let (server, start, form) = match &smoke {
        Some(target) => {
            let server = url::Url::parse(target)
                .ok()
                .and_then(|url| address::parse(&url.origin().ascii_serialization()).ok());
            (server, target.clone(), Form::default())
        }
        None => {
            let saved = address::saved(&root);
            match address::parse(&saved) {
                Ok(server) => {
                    let start =
                        links::target(&server, &activation).unwrap_or_else(|| server.home().into());
                    let form = Form {
                        address: server.origin(),
                        error: None,
                    };
                    (Some(server), start, form)
                }
                Err(error) => {
                    let form = Form {
                        error: (!saved.is_empty()).then(|| error.to_string()),
                        address: saved,
                    };
                    (None, shell_page::PAGE.to_string(), form)
                }
            }
        }
    };
    let mut policy = Policy::new(&page, server.as_ref());
    if is_smoke && policy.navigate(&start) != Decision::Allow {
        smoke_report(
            false,
            &start,
            "",
            Some("not an address this app would load"),
        );
        return 2;
    }

    let (runtime, instance) = match single_instance(&root, activation) {
        Ok(Some(instance)) => instance,
        // The running instance has the link and comes to the front.
        Ok(None) => {
            if is_smoke {
                println!(r#"{{"ok":true,"forwarded":true}}"#);
            }
            return 0;
        }
        Err(error) => {
            if is_smoke {
                smoke_report(false, &start, "", Some(&format!("instance: {error}")));
            }
            return 1;
        }
    };
    // Groups the window with the installer's shortcuts on the taskbar.
    let _ = sticktalk_desktop_platform::windows::set_app_id();

    let shared: SharedState = Arc::new(Mutex::new(Shared {
        server,
        policy,
        form,
        navigating: String::new(),
    }));
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let mut builder = WindowBuilder::new()
        .with_title(TITLE)
        // Hidden until it is placed where it was last time (or forever, in a
        // smoke run); activation comes from showing it, not from creating it.
        .with_visible(false)
        .with_focused(false)
        .with_inner_size(LogicalSize::new(
            window_state::DEFAULT_SIZE.0,
            window_state::DEFAULT_SIZE.1,
        ))
        .with_min_inner_size(LogicalSize::new(
            window_state::MIN_SIZE.0,
            window_state::MIN_SIZE.1,
        ))
        .with_background_color(webview::BACKGROUND)
        .with_window_icon(Icon::from_resource(1, None).ok())
        // File drops belong to the page, not to the window frame.
        .with_drag_and_drop(false);
    if is_smoke {
        builder = builder.with_skip_taskbar(true).with_focusable(false);
    }
    let window = match builder.build(&event_loop) {
        Ok(window) => window,
        Err(error) => {
            if is_smoke {
                smoke_report(false, &start, "", Some(&format!("window: {error}")));
            }
            return 3;
        }
    };
    let hwnd = HWND(window.hwnd() as *mut _);
    unsafe { add_window_menu(hwnd, proxy.clone()) };
    // Show the (dark, empty) window first: the WebView takes a moment.
    let show = if is_smoke {
        window_state::Show::Hidden
    } else if background {
        window_state::Show::Minimized
    } else {
        window_state::Show::Foreground
    };
    window_state::restore(hwnd, &root, show);

    // The WebView2 user-data folder: under the data root, never beside the exe.
    let mut context = WebContext::new(Some(root.join("WebView")));
    let webview = webview::build(
        &window,
        &mut context,
        webview::Config {
            start: start.clone(),
            shared: shared.clone(),
            proxy: proxy.clone(),
            root: root.clone(),
            token: random_token(),
            focused: !is_smoke && !background,
        },
    );
    let webview = match webview {
        Ok(webview) => webview,
        Err(error) => {
            if is_smoke {
                smoke_report(false, &start, "", Some(&format!("webview: {error}")));
                let _ = std::fs::remove_dir_all(&root);
            } else {
                unsafe {
                    MessageBoxW(
                        Some(hwnd),
                        w!("StickTalk needs the Microsoft Edge WebView2 Runtime, which is missing or damaged on this PC.\n\nInstall it from microsoft.com/edge/webview2 (\"Evergreen Bootstrapper\"), then open StickTalk again."),
                        w!("StickTalk"),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
            return 3;
        }
    };
    let stop = Arc::new(tokio::sync::Notify::new());
    let forwarder = forward_activations(runtime, instance, proxy.clone(), stop.clone());

    let mut smoke = is_smoke.then(|| Smoke {
        deadline: Instant::now() + SMOKE_TIMEOUT,
        eval: smoke_eval,
        shot: smoke_shot,
        settle: smoke_wait.map_or(SMOKE_SETTLE, Duration::from_millis),
        last: None,
        finish_at: None,
    });
    let mut save_at: Option<Instant> = None;
    let mut last_external: Option<Instant> = None;
    let code = event_loop.run_return(|event, _, flow| {
        let now = Instant::now();
        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if save_at.is_some_and(|at| now >= at) {
                    save_at = None;
                    window_state::save(hwnd, &root);
                }
                if let Some(smoke) = &mut smoke {
                    if smoke.finish_at.is_some_and(|at| now >= at) {
                        smoke.finish_at = None;
                        let code = smoke.report();
                        let capturing = code == 0
                            && smoke.shot.as_deref().is_some_and(|path| {
                                webview::capture(&webview, path, proxy.clone()).is_ok()
                            });
                        if !capturing {
                            let failed_shot = code == 0 && smoke.shot.is_some();
                            *flow = ControlFlow::ExitWithCode(if failed_shot { 4 } else { code });
                            return;
                        }
                    } else if now >= smoke.deadline {
                        smoke_report(false, &start, "", Some("timed out"));
                        *flow = ControlFlow::ExitWithCode(2);
                        return;
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Moved(_) | WindowEvent::Resized(_),
                ..
            } => save_at = Some(now + SAVE_DELAY),
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                window_state::save(hwnd, &root);
                *flow = ControlFlow::Exit;
                return;
            }
            Event::UserEvent(UserEvent::Captured(saved)) => {
                *flow = ControlFlow::ExitWithCode(if saved { 0 } else { 4 });
                return;
            }
            Event::UserEvent(UserEvent::Loaded(load)) => {
                let mut first = false;
                if let Some(smoke) = &mut smoke {
                    eprintln!(
                        "load: ok={} cancelled={} web_error={} http={} {}",
                        load.ok, load.cancelled, load.detail.0, load.detail.1, load.url
                    );
                    // A navigation the policy refused is not a page load.
                    if load.cancelled || load.url == "about:blank" {
                        return;
                    }
                    first = smoke.last.is_none();
                    smoke.last = Some((load.ok, load.url.clone(), load.title.clone()));
                    if first && (!load.ok || (smoke.eval.is_none() && smoke.shot.is_none())) {
                        *flow = ControlFlow::ExitWithCode(smoke.report());
                        return;
                    }
                    if let Some(script) = smoke.eval.take() {
                        let _ = webview.evaluate_script(&script);
                    }
                    smoke.finish_at = Some(now + smoke.settle);
                }
                let mut state = lock(&shared);
                if let (Some(failure), false, false) =
                    (load.failure, first, state.policy.is_shell(&load.url))
                {
                    // Say which host failed (the server, or Google's sign-in).
                    let host = url::Url::parse(&state.navigating)
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_string))
                        .or_else(|| state.server.as_ref().map(address::Server::host))
                        .unwrap_or_default();
                    state.form.error = Some(health::explain(failure, &host));
                    if let Some(server) = &state.server {
                        state.form.address = server.origin();
                    }
                    drop(state);
                    let _ = webview.load_url(shell_page::PAGE);
                }
            }
            Event::UserEvent(UserEvent::ChangeServer) => {
                let mut state = lock(&shared);
                let on_page = webview.url().is_ok_and(|url| state.policy.is_shell(&url));
                if !on_page {
                    state.form.error = None;
                    if let Some(server) = &state.server {
                        state.form.address = server.origin();
                    }
                    drop(state);
                    let _ = webview.load_url(shell_page::PAGE);
                }
            }
            Event::UserEvent(UserEvent::Activation(activation)) => {
                if !is_smoke {
                    window_state::focus(hwnd);
                    let _ = webview.focus();
                }
                let target = lock(&shared)
                    .server
                    .as_ref()
                    .and_then(|server| links::target(server, &activation));
                if let Some(target) = target {
                    let _ = webview.load_url(&target);
                }
            }
            Event::UserEvent(UserEvent::OpenExternal(url)) => {
                if is_smoke {
                    // Tests never open the browser; say what would have opened.
                    eprintln!("external: {url}");
                } else if last_external.is_none_or(|at| now.duration_since(at) >= EXTERNAL_INTERVAL)
                {
                    last_external = Some(now);
                    open_external(&url);
                }
            }
            Event::UserEvent(UserEvent::Title(title)) => {
                // Only the server's own pages may rename the window.
                let ours = webview
                    .url()
                    .is_ok_and(|url| lock(&shared).policy.is_server(&url));
                let title = title.trim();
                if let Some((_, _, last)) = smoke.as_mut().and_then(|smoke| smoke.last.as_mut()) {
                    // What the page is called now, e.g. after a test script ran.
                    *last = title.to_string();
                }
                if ours && !title.is_empty() {
                    window.set_title(&title.chars().take(120).collect::<String>());
                } else {
                    window.set_title(TITLE);
                }
            }
            _ => {}
        }
        // Asleep until the next event or the nearest one-shot deadline.
        let smoke_at = smoke.as_ref().map(|smoke| {
            smoke
                .finish_at
                .unwrap_or(smoke.deadline)
                .min(smoke.deadline)
        });
        *flow = [save_at, smoke_at]
            .into_iter()
            .flatten()
            .min()
            .map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
    });

    drop(webview);
    drop(context);
    stop.notify_one();
    let _ = forwarder.join();
    if own_root {
        // The WebView's processes let go of the profile shortly after it closes.
        for _ in 0..20 {
            if std::fs::remove_dir_all(&root).is_ok() || !root.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }
    code
}
