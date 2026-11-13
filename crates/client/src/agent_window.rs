//! Agent-only native integration mode. Hidden at creation; never activates or falls back to primary.
pub fn enabled() -> bool {
    std::env::var("STICKTALK_AGENT_WINDOW").is_ok_and(|v| v == "secondary")
}
/// Explicit debug-only live QA session. Does not read/write a real account's
/// credentials, and cannot point the test hooks off loopback. The native QA
/// harness also supplies an isolated LOCALAPPDATA settings home.
pub fn qa_session() -> Option<crate::backend::Session> {
    if !cfg!(debug_assertions) || !enabled() {
        return None;
    }
    let server = std::env::var("STICKTALK_QA_SERVER").ok()?;
    let token = std::env::var("STICKTALK_QA_TOKEN").ok()?;
    let server = sticktalk_client_core::http::validate_server(&server).ok()?;
    let local = reqwest::Url::parse(&server).ok()?;
    if local.host_str() != Some("127.0.0.1") || token.is_empty() {
        return None;
    }
    Some(crate::backend::Session {
        config: crate::backend::Config {
            server,
            token,
            mock: false,
            generation: crate::backend::nonce(),
            scenario: String::new(),
        },
    })
}
pub fn show_secondary(window: &dyn iced_runtime::window::Window) -> bool {
    #[cfg(windows)]
    {
        use iced_runtime::window::raw_window_handle::RawWindowHandle;
        use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};
        unsafe extern "system" fn monitor(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..unsafe { std::mem::zeroed() }
            };
            if unsafe { GetMonitorInfoW(h, &mut info) } != 0
                && info.dwFlags & MONITORINFOF_PRIMARY == 0
            {
                unsafe { *(data as *mut Option<RECT>) = Some(info.rcWork) };
                return 0;
            }
            1
        }
        let mut secondary = None::<RECT>;
        // SAFETY: synchronous enumeration; callback receives our live stack pointer and copies RECT.
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(monitor),
                (&mut secondary as *mut Option<RECT>) as LPARAM,
            );
        }
        let Some(rect) = secondary else {
            return false;
        };
        let Ok(handle) = window.window_handle() else {
            return false;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return false;
        };
        let hwnd = handle.hwnd.get() as HWND;
        // SAFETY: iced supplies a live HWND on its UI thread. Do not call SetForegroundWindow.
        let (shown, before, after) = unsafe {
            let before = GetForegroundWindow();
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                style | WS_EX_NOACTIVATE as isize | WS_EX_TOOLWINDOW as isize,
            );
            let shown = SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                rect.left + 32,
                rect.top + 32,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW | SWP_NOSIZE,
            ) != 0;
            (shown, before, GetForegroundWindow())
        };
        let mut client: RECT = unsafe { std::mem::zeroed() };
        // SAFETY: reading dimensions of the same live HWND.
        unsafe {
            GetClientRect(hwnd, &mut client);
        }
        let args: Vec<_> = std::env::args().collect();
        if let Some(path) = args
            .iter()
            .position(|v| v == "--agent-window-report")
            .and_then(|i| args.get(i + 1))
        {
            let report = serde_json::json!({"mode":"secondary-noactivate","shown":shown,"non_primary_work_area":[rect.left,rect.top,rect.right,rect.bottom],"window_position":[rect.left+32,rect.top+32],"foreground_unchanged":before==after,"client_foreground":after==hwnd,"client_size":[client.right-client.left,client.bottom-client.top]});
            let _ = std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap_or_default());
        }
        shown
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        false
    }
}
