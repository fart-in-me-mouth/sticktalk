//! Explicit opt-in input target. Never opens during ordinary cargo test.
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

#[derive(Default)]
pub struct Counts {
    pub key_down: AtomicU32,
    pub key_up: AtomicU32,
    pub button_down: AtomicU32,
    pub button_up: AtomicU32,
    pub moves: AtomicU32,
    pub first_key_down: std::sync::Mutex<Option<std::time::Instant>>,
}
pub struct Target {
    hwnd: usize,
    before: usize,
    worker: Option<std::thread::JoinHandle<()>>,
    pub counts: Arc<Counts>,
    pub source: sticktalk_media::Source,
}
impl Target {
    pub fn start() -> Self {
        assert_eq!(
            std::env::var("STICKTALK_AGENT_WINDOW").as_deref(),
            Ok("secondary")
        );
        unsafe extern "system" fn monitor(h: HMONITOR, _: HDC, _: *mut RECT, p: LPARAM) -> i32 {
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(h, &mut info) != 0 && info.dwFlags & MONITORINFOF_PRIMARY == 0 {
                *(p as *mut Option<RECT>) = Some(info.rcWork);
                return 0;
            }
            1
        }
        unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            if msg == WM_NCCREATE {
                let create = &*(l as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Counts;
            if let Some(c) = state.as_ref() {
                let counter = match msg {
                    WM_KEYDOWN => Some(&c.key_down),
                    WM_KEYUP => Some(&c.key_up),
                    WM_LBUTTONDOWN => Some(&c.button_down),
                    WM_LBUTTONUP => Some(&c.button_up),
                    WM_MOUSEMOVE => Some(&c.moves),
                    _ => None,
                };
                if let Some(counter) = counter {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                if msg == WM_KEYDOWN {
                    c.first_key_down
                        .lock()
                        .unwrap()
                        .get_or_insert_with(std::time::Instant::now);
                }
            }
            if msg == WM_DESTROY {
                PostQuitMessage(0);
                return 0;
            }
            DefWindowProcW(hwnd, msg, w, l)
        }
        let mut secondary = None::<RECT>;
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(monitor),
                (&mut secondary as *mut Option<RECT>) as isize,
            );
        }
        let rect = secondary.expect("secondary monitor required; no primary fallback");
        let counts = Arc::new(Counts::default());
        let shared = counts.clone();
        let before = unsafe { GetForegroundWindow() as usize };
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || unsafe {
            let class: Vec<u16> = "StickTalkQaInputTarget\0".encode_utf16().collect();
            let title: Vec<u16> = "StickTalk QA input target\0".encode_utf16().collect();
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&wc), 0);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                rect.left + 32,
                rect.top + 32,
                640,
                400,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                Arc::as_ptr(&shared).cast(),
            );
            assert!(!hwnd.is_null());
            assert_ne!(
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW
                ),
                0
            );
            tx.send(hwnd as usize).unwrap();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            UnregisterClassW(class.as_ptr(), instance);
        });
        let hwnd = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let source = sticktalk_media::list_sources()
            .unwrap()
            .into_iter()
            .find(|s| s.id == hwnd as isize && s.kind == sticktalk_media::SourceKind::Window)
            .unwrap();
        Self {
            hwnd,
            before,
            worker: Some(worker),
            counts,
            source,
        }
    }
    pub fn assert_received(&self) {
        assert!(
            self.counts.key_down.load(Ordering::SeqCst) > 0,
            "target did not receive key down"
        );
        assert!(
            self.counts.key_up.load(Ordering::SeqCst) > 0,
            "held key was not released"
        );
        assert!(
            self.counts.button_down.load(Ordering::SeqCst) > 0,
            "target did not receive mouse button"
        );
        assert!(
            self.counts.button_up.load(Ordering::SeqCst) > 0,
            "held button was not released"
        );
        assert!(
            self.counts.moves.load(Ordering::SeqCst) > 0,
            "target did not receive mouse move"
        );
    }
}
impl Drop for Target {
    fn drop(&mut self) {
        unsafe {
            PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        unsafe {
            if IsWindow(self.before as HWND) != 0 {
                SetForegroundWindow(self.before as HWND);
            }
        }
    }
}
