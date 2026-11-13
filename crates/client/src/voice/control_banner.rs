//! Always-on-top, non-activating emergency stop surface. Created ONLY after consent.
//! No tests create this window. The injector and this lifetime are inseparable.
use super::control::{native::NativeInjector, Gated, Receiver, Surface};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::sync_channel,
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
};
use sticktalk_media::Source;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{CreateEventW, GetCurrentThreadId, SetEvent},
    },
    UI::WindowsAndMessaging::*,
};

pub(super) struct SafetyUnit {
    pub(super) receiver: Arc<Mutex<Receiver<Gated<NativeInjector>>>>,
    stop_event: Arc<StopEvent>,
    pub(super) active: Arc<AtomicBool>,
    id: u32,
    worker: Option<JoinHandle<()>>,
}
struct StopEvent(usize);
impl StopEvent {
    fn signal(&self) {
        unsafe {
            SetEvent(self.0 as HANDLE);
        }
    }
}
impl Drop for StopEvent {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0 as HANDLE);
        }
    }
}
struct BannerState {
    active: Arc<AtomicBool>,
    receiver: Arc<Mutex<Receiver<Gated<NativeInjector>>>>,
}
impl BannerState {
    fn stop(&self) {
        self.active.store(false, Ordering::Release);
        if let Ok(mut receiver) = self.receiver.lock() {
            let _ = receiver.release();
        }
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let flag = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const BannerState;
    match msg {
        WM_LBUTTONUP | WM_CLOSE => {
            if !flag.is_null() {
                (*flag).stop();
            }
            return 0;
        }
        WM_DESTROY => {
            if !flag.is_null() {
                (*flag).stop();
            }
            PostQuitMessage(0);
            return 0;
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut rect = RECT::default();
            GetClientRect(hwnd, &mut rect);
            let brush = CreateSolidBrush(0x002b2626);
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x00edeae8);
            let family: Vec<u16> = "Inter\0".encode_utf16().collect();
            let font = CreateFontW(
                -15,
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                family.as_ptr(),
            );
            let old = SelectObject(dc, font);
            let mut label = [0u16; 512];
            let length = GetWindowTextW(hwnd, label.as_mut_ptr(), 512);
            DrawTextW(
                dc,
                label.as_mut_ptr(),
                length,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            SelectObject(dc, old);
            DeleteObject(font);
            EndPaint(hwnd, &paint);
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, w, l)
}
impl SafetyUnit {
    pub fn start(source: Source, name: String, active: Arc<AtomicBool>) -> Result<Self, String> {
        // The real native unit is intentionally impossible in headless/mock invocations.
        if std::env::var_os("STICKTALK_AGENT_WINDOW").is_some()
            || std::env::args().any(|a| a == "--mock" || a == "--screenshots" || a == "--capture")
            || cfg!(test)
        {
            return Err("Native control is disabled in tests and mock mode".into());
        }
        active.store(false, Ordering::Release);
        let surface = Surface {
            x: 0,
            y: 0,
            width: source.size.width,
            height: source.size.height,
        };
        let receiver = Arc::new(Mutex::new(Receiver::new(
            Gated {
                inner: NativeInjector { source },
                active: active.clone(),
            },
            surface,
        )));
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if handle.is_null() {
            return Err("Emergency stop event unavailable".into());
        }
        let stop_event = Arc::new(StopEvent(handle as usize));
        let wake = stop_event.clone();
        let held = receiver.clone();
        let (ready_tx, ready) = sync_channel(1);
        let flag = active.clone();
        let display = banner_label(&name);
        let worker = thread::Builder::new()
            .name("sticktalk-control-stop".into())
            .spawn(move || unsafe {
                struct FontResource(HANDLE);
                impl Drop for FontResource {
                    fn drop(&mut self) {
                        unsafe {
                            if !self.0.is_null() {
                                RemoveFontMemResourceEx(self.0);
                            }
                        }
                    }
                }
                let bytes = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
                let mut fonts = 0;
                let _font = FontResource(AddFontMemResourceEx(
                    bytes.as_ptr().cast(),
                    bytes.len() as u32,
                    std::ptr::null(),
                    std::ptr::addr_of_mut!(fonts),
                ));
                let state = BannerState {
                    active: flag.clone(),
                    receiver: held,
                };
                let class: Vec<u16> = "StickTalkControlStop\0".encode_utf16().collect();
                let module = GetModuleHandleW(std::ptr::null());
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(procedure),
                    hInstance: module,
                    lpszClassName: class.as_ptr(),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_HAND),
                    ..std::mem::zeroed()
                };
                RegisterClassW(&wc);
                let label: Vec<u16> = display.encode_utf16().chain(std::iter::once(0)).collect();
                let width = 760;
                let height = 48;
                let x = (GetSystemMetrics(SM_CXSCREEN) - width) / 2;
                // No activation/focus theft. Never started by qualification tests.
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    class.as_ptr(),
                    label.as_ptr(),
                    WS_POPUP,
                    x,
                    0,
                    width,
                    height,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    module,
                    (&state as *const BannerState).cast(),
                );
                if hwnd.is_null() {
                    let _ = ready_tx.send(Err("Could not create Stop banner".to_string()));
                    return;
                }
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                UpdateWindow(hwnd);
                // Arm before publishing readiness. The caller must never arm
                // again: a physical takeback/Stop racing with readiness wins.
                flag.store(true, Ordering::Release);
                let _ = ready_tx.send(Ok(GetCurrentThreadId()));
                let mut msg = std::mem::zeroed();
                'messages: loop {
                    let result =
                        MsgWaitForMultipleObjects(1, &(wake.0 as HANDLE), 0, u32::MAX, QS_ALLINPUT);
                    if result == 0 {
                        state.stop();
                    } else if result == u32::MAX {
                        state.stop();
                        break;
                    }
                    while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        if msg.message == WM_QUIT {
                            break 'messages;
                        }
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                flag.store(false, Ordering::Release);
                DestroyWindow(hwnd);
            })
            .map_err(|e| e.to_string())?;
        let id = match ready.recv().map_err(|e| e.to_string())? {
            Ok(id) => id,
            Err(e) => {
                let _ = worker.join();
                return Err(e);
            }
        };
        eprintln!("remote control granted: local consent and safety unit ready");
        Ok(Self {
            receiver,
            active,
            stop_event,
            id,
            worker: Some(worker),
        })
    }
    pub fn emergency(&self) -> Arc<dyn Fn() + Send + Sync> {
        let event = self.stop_event.clone();
        Arc::new(move || event.signal())
    }
}
impl Drop for SafetyUnit {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        let released = self
            .receiver
            .lock()
            .is_ok_and(|mut receiver| receiver.release().is_ok());
        eprintln!("remote control revoked: release_attempt_ok={released}");
        unsafe {
            PostThreadMessageW(self.id, WM_QUIT, 0, 0);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn banner_label(name: &str) -> String {
    // Untrusted names cannot terminate/reorder the fixed emergency-stop text.
    let display: String = name
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(20)
        .collect();
    format!("{display} is controlling your screen — Stop (Ctrl+Alt+Shift+S)")
}
#[cfg(test)]
mod tests {
    #[test]
    fn untrusted_long_name_cannot_hide_emergency_stop_text() {
        let label = super::banner_label(&format!("\0\u{202e}{}", "W".repeat(80)));
        assert!(label.starts_with(&"W".repeat(20)));
        assert!(!label.contains('\0'));
        assert!(!label.contains('\u{202e}'));
        assert!(label.ends_with("— Stop (Ctrl+Alt+Shift+S)"));
    }
}
