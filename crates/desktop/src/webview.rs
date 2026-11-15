//! The WebView: wry for the portable parts, raw WebView2 events where the
//! decision needs more than wry passes on (who is asking, did the load fail).
use crate::{
    address,
    app::{Load, UserEvent},
    health::{self, Failure},
    menu,
    policy::{Decision, Grant, Permission},
    shell_page::{self, lock, Form, Route, SharedState},
};
use std::{borrow::Cow, path::PathBuf};
use tao::{event_loop::EventLoopProxy, window::Window};
use webview2_com::{
    take_pwstr, AcceleratorKeyPressedEventHandler, CapturePreviewCompletedHandler,
    ContextMenuRequestedEventHandler, Microsoft::Web::WebView2::Win32::*,
    NavigationCompletedEventHandler, PermissionRequestedEventHandler,
    ScreenCaptureStartingEventHandler,
};
use windows::{
    core::{Interface, BOOL, HSTRING, PWSTR},
    Win32::{
        System::Com::{STGM_CREATE, STGM_SHARE_EXCLUSIVE, STGM_WRITE},
        UI::{
            Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT},
            Shell::SHCreateStreamOnFileEx,
        },
    },
};
use wry::{
    http::{Request, Response},
    NewWindowResponse, RequestAsyncResponder, WebContext, WebView, WebViewBuilder,
    WebViewExtWindows,
};

/// bg.rail from docs/UI_SPEC.md: what shows before a page has painted.
pub const BACKGROUND: (u8, u8, u8, u8) = (0x11, 0x11, 0x13, 0xff);

pub struct Config {
    pub start: String,
    pub shared: SharedState,
    pub proxy: EventLoopProxy<UserEvent>,
    /// Where the address is remembered.
    pub root: PathBuf,
    /// This run's secret for the built-in page's `connect` call.
    pub token: String,
    pub focused: bool,
}

/// Fails when the WebView2 Runtime is missing or cannot start. The text is for
/// a smoke run's report, not for people.
pub fn build(window: &Window, context: &mut WebContext, config: Config) -> Result<WebView, String> {
    let Config {
        start,
        shared,
        proxy,
        root,
        token,
        focused,
    } = config;
    let builder = WebViewBuilder::new_with_web_context(context)
        .with_url(start)
        .with_background_color(BACKGROUND)
        // Voice and video must play without a click first.
        .with_autoplay(true)
        .with_general_autofill_enabled(false)
        .with_focused(focused)
        .with_asynchronous_custom_protocol(shell_page::SCHEME.into(), {
            let shared = shared.clone();
            move |_id, request, responder| serve(&shared, &root, &token, request, responder)
        })
        .with_navigation_handler({
            let (shared, proxy) = (shared.clone(), proxy.clone());
            move |url| {
                let mut state = lock(&shared);
                match state.policy.navigate(&url) {
                    Decision::Allow => {
                        state.navigating = url;
                        true
                    }
                    Decision::External(url) => {
                        let _ = proxy.send_event(UserEvent::OpenExternal(url));
                        false
                    }
                    Decision::Block => false,
                }
            }
        })
        .with_new_window_req_handler({
            let (shared, proxy) = (shared.clone(), proxy.clone());
            move |url, _features| {
                if let Decision::External(url) = lock(&shared).policy.new_window(&url) {
                    let _ = proxy.send_event(UserEvent::OpenExternal(url));
                }
                NewWindowResponse::Deny
            }
        })
        .with_document_title_changed_handler({
            let proxy = proxy.clone();
            move |title| {
                let _ = proxy.send_event(UserEvent::Title(title));
            }
        });
    // Release builds have no developer tools, so no "Inspect" either.
    #[cfg(debug_assertions)]
    let builder = builder.with_devtools(true);
    let webview = builder.build(window).map_err(|e| e.to_string())?;
    unsafe { hook(&webview, shared, proxy) }.map_err(|e| e.to_string())?;
    Ok(webview)
}

fn reply(status: u16, mime: &str, body: Cow<'static, [u8]>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header("content-type", mime)
        .header(
            "content-security-policy",
            shell_page::CONTENT_SECURITY_POLICY,
        )
        .header("x-content-type-options", "nosniff")
        .header("cache-control", "no-store")
        .body(body)
        .expect("static response headers")
}

const JSON: &str = "application/json";

/// Answers the built-in page. Runs on the UI thread; only `connect` leaves it.
fn serve(
    shared: &SharedState,
    root: &std::path::Path,
    token: &str,
    request: Request<Vec<u8>>,
    responder: RequestAsyncResponder,
) {
    // Any page in the window can address this protocol. Other origins get
    // nothing, and cannot read the token that `connect` requires anyway.
    let foreign = request.headers().get("origin").is_some_and(|origin| {
        origin
            .to_str()
            .map_or(true, |origin| !lock(shared).policy.is_shell(origin))
    });
    if foreign {
        return responder.respond(reply(403, "text/plain", Cow::Borrowed(b"")));
    }
    match shell_page::route(request.method().as_str(), request.uri().path()) {
        Route::Asset { mime, body } => responder.respond(reply(200, mime, Cow::Borrowed(body))),
        Route::State => {
            let body = shell_page::state_json(token, &lock(shared).form);
            responder.respond(reply(200, JSON, body.into()))
        }
        Route::Connect => {
            let server = match shell_page::connect_request(request.body(), token) {
                Ok(server) => server,
                Err(error) => {
                    let body = shell_page::connect_json(Err(error));
                    return responder.respond(reply(200, JSON, body.into()));
                }
            };
            let (shared, root) = (shared.clone(), root.to_path_buf());
            // One short-lived thread per press of Connect; nothing runs otherwise.
            std::thread::spawn(move || {
                let checked = health::check(&server)
                    .map_err(|failure| health::explain(failure, &server.host()))
                    .and_then(|()| {
                        address::save(&root, &server).map_err(|_| {
                            "StickTalk couldn't remember this address. Check that your disk isn't full, then try again."
                                .to_string()
                        })
                    });
                let body = match checked {
                    Ok(()) => {
                        let mut state = lock(&shared);
                        state.policy.set_server(&server);
                        state.form = Form {
                            address: server.origin(),
                            error: None,
                        };
                        state.server = Some(server.clone());
                        shell_page::connect_json(Ok(&server))
                    }
                    Err(error) => shell_page::connect_json(Err(&error)),
                };
                responder.respond(reply(200, JSON, body.into()));
            });
        }
        Route::NotFound => responder.respond(reply(404, "text/plain", Cow::Borrowed(b""))),
    }
}

fn load_failure(status: COREWEBVIEW2_WEB_ERROR_STATUS, http: i32) -> Option<Failure> {
    match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID => Some(Failure::Untrusted),
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => Some(Failure::NotFound),
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => Some(Failure::TimedOut),
        COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE
        | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET
        | COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT
        | COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE => {
            Some(Failure::Refused)
        }
        // A cancelled load, or one that turned into a download.
        COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED => None,
        // A gateway in front of a server that is down.
        _ if matches!(http, 502..=504) => Some(Failure::Unwell(http as u16)),
        // No answer at all. WebView2 reports a refused connection this way.
        _ if http == 0 => Some(Failure::Refused),
        // An ordinary error page from the server (404 and the like) stays.
        _ => None,
    }
}

/// WebView2 events wry does not expose with enough detail.
unsafe fn hook(
    webview: &WebView,
    shared: SharedState,
    proxy: EventLoopProxy<UserEvent>,
) -> windows::core::Result<()> {
    let core = webview.webview();
    let mut token = 0i64;

    let settings = core.Settings()?;
    // Load failures show the built-in page instead of a browser error page.
    settings.SetIsBuiltInErrorPageEnabled(false)?;
    if let Ok(settings) = settings.cast::<ICoreWebView2Settings8>() {
        // SmartScreen sends visited addresses to Microsoft; this window only
        // ever shows the person's own server.
        let _ = settings.SetIsReputationCheckingRequired(false);
    }

    // Microphone and camera: granted to the server's pages without a prompt,
    // refused to everything else, and never remembered by the profile, so a
    // later change of server starts clean.
    core.add_PermissionRequested(
        &PermissionRequestedEventHandler::create(Box::new({
            let shared = shared.clone();
            move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut requester = PWSTR::null();
                args.Uri(&mut requester)?;
                let requester = take_pwstr(requester);
                let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                args.PermissionKind(&mut kind)?;
                let permission = match kind {
                    COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => Permission::Microphone,
                    COREWEBVIEW2_PERMISSION_KIND_CAMERA => Permission::Camera,
                    _ => Permission::Other,
                };
                let state = match lock(&shared).policy.permission(&requester, permission) {
                    Grant::Allow => COREWEBVIEW2_PERMISSION_STATE_ALLOW,
                    Grant::Deny => COREWEBVIEW2_PERMISSION_STATE_DENY,
                    Grant::Ask => return Ok(()),
                };
                args.SetState(state)?;
                if let Ok(args) = args.cast::<ICoreWebView2PermissionRequestedEventArgs3>() {
                    args.SetSavesInProfile(false)?;
                }
                Ok(())
            }
        })),
        &mut token,
    )?;

    // Screen sharing: WebView2 shows its own "choose what to share" picker for
    // the server's pages. Any other page is refused before the picker.
    if let Ok(core) = core.cast::<ICoreWebView2_27>() {
        core.add_ScreenCaptureStarting(
            &ScreenCaptureStartingEventHandler::create(Box::new({
                let shared = shared.clone();
                move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut source = PWSTR::null();
                    args.OriginalSourceFrameInfo()?.Source(&mut source)?;
                    if !lock(&shared).policy.is_server(&take_pwstr(source)) {
                        args.SetCancel(true)?;
                    }
                    Ok(())
                }
            })),
            &mut token,
        )?;
    }

    core.add_NavigationCompleted(
        &NavigationCompletedEventHandler::create(Box::new({
            let proxy = proxy.clone();
            move |sender, args| {
                let (Some(sender), Some(args)) = (sender, args) else {
                    return Ok(());
                };
                let mut ok = BOOL::default();
                args.IsSuccess(&mut ok)?;
                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                args.WebErrorStatus(&mut status)?;
                let mut http = 0;
                if let Ok(args) = args.cast::<ICoreWebView2NavigationCompletedEventArgs2>() {
                    let _ = args.HttpStatusCode(&mut http);
                }
                let mut url = PWSTR::null();
                sender.Source(&mut url)?;
                let mut title = PWSTR::null();
                sender.DocumentTitle(&mut title)?;
                let _ = proxy.send_event(UserEvent::Loaded(Load {
                    ok: ok.as_bool(),
                    cancelled: matches!(
                        status,
                        COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED
                            | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED
                    ),
                    url: take_pwstr(url),
                    title: take_pwstr(title),
                    detail: (status.0, http),
                    failure: if ok.as_bool() {
                        None
                    } else {
                        load_failure(status, http)
                    },
                }));
                Ok(())
            }
        })),
        &mut token,
    )?;

    // Ctrl+Shift+S: back to the built-in page to change servers. Handled here
    // because the keyboard belongs to the WebView, and so that no page script
    // can trigger or swallow it.
    webview.controller().add_AcceleratorKeyPressed(
        &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
            args.KeyEventKind(&mut kind)?;
            let mut key = 0;
            args.VirtualKey(&mut key)?;
            let down = |key: u16| GetKeyState(i32::from(key)) < 0;
            if kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                && key == u32::from(b'S')
                && down(VK_CONTROL.0)
                && down(VK_SHIFT.0)
                && !down(VK_MENU.0)
            {
                args.SetHandled(true)?;
                let _ = proxy.send_event(UserEvent::ChangeServer);
            }
            Ok(())
        })),
        &mut token,
    )?;

    if let Ok(core) = core.cast::<ICoreWebView2_11>() {
        core.add_ContextMenuRequested(
            &ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                let items = args.MenuItems()?;
                let mut count = 0;
                items.Count(&mut count)?;
                let mut entries = Vec::with_capacity(count as usize);
                for index in 0..count {
                    let item = items.GetValueAtIndex(index)?;
                    let mut name = PWSTR::null();
                    item.Name(&mut name)?;
                    let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
                    item.Kind(&mut kind)?;
                    entries.push((
                        take_pwstr(name),
                        kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR,
                    ));
                }
                let keep = menu::visible(&entries, cfg!(debug_assertions));
                for index in (0..count).rev() {
                    if !keep[index as usize] {
                        items.RemoveValueAtIndex(index)?;
                    }
                }
                if !keep.contains(&true) {
                    // Nothing left: no empty menu.
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    Ok(())
}

/// Smoke runs only: saves what the page rendered, without ever showing it.
pub fn capture(
    webview: &WebView,
    path: &std::path::Path,
    proxy: EventLoopProxy<UserEvent>,
) -> windows::core::Result<()> {
    unsafe {
        let stream = SHCreateStreamOnFileEx(
            &HSTRING::from(path.as_os_str()),
            (STGM_CREATE | STGM_WRITE | STGM_SHARE_EXCLUSIVE).0,
            0,
            true,
            None,
        )?;
        webview.webview().CapturePreview(
            COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
            &stream,
            &CapturePreviewCompletedHandler::create(Box::new(move |result| {
                let _ = proxy.send_event(UserEvent::Captured(result.is_ok()));
                Ok(())
            })),
        )
    }
}
