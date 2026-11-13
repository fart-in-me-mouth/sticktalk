//! Passive low-level hooks: no window, no focus changes, no swallowed game input.
fn physical_mouse(flags: u32) -> bool {
    flags & windows_sys::Win32::UI::WindowsAndMessaging::LLMHF_INJECTED == 0
} // LLMHF_INJECTED
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    PushToTalk(bool),
    Mute,
    Deafen,
    Revoke,
}
fn action(
    ptt: Option<u32>,
    key: u32,
    down: bool,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Option<Hotkey> {
    // The emergency stop chord is reserved, even if S is the configured PTT.
    if down && ctrl && shift && alt && key == 0x53 {
        Some(Hotkey::Revoke)
    } else if down && ctrl && shift && key == 0x4d {
        Some(Hotkey::Mute)
    } else if down && ctrl && shift && key == 0x44 {
        Some(Hotkey::Deafen)
    } else if ptt == Some(key) {
        Some(Hotkey::PushToTalk(down))
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn injected_mouse_never_revokes_and_physical_mouse_does() {
        assert!(physical_mouse(0));
        assert!(!physical_mouse(1));
        assert!(!physical_mouse(3));
    }
    #[test]
    fn stop_chord_cannot_be_shadowed_by_ptt() {
        assert_eq!(
            action(Some(0x53), 0x53, true, true, true, true),
            Some(Hotkey::Revoke)
        );
        assert_eq!(
            action(Some(0x53), 0x53, false, true, true, true),
            Some(Hotkey::PushToTalk(false))
        );
        assert_eq!(
            action(None, 0x4d, true, true, true, false),
            Some(Hotkey::Mute)
        );
        assert_eq!(
            action(Some(0x4d), 0x4d, true, true, true, false),
            Some(Hotkey::Mute)
        );
        assert_eq!(
            action(None, 0x44, true, true, true, false),
            Some(Hotkey::Deafen)
        );
        assert_eq!(
            action(Some(0x53), 0x53, true, false, false, false),
            Some(Hotkey::PushToTalk(true))
        );
    }
}
#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        collections::BTreeSet,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex, OnceLock,
        },
        thread::{self, JoinHandle},
    };
    use windows_sys::Win32::{
        Foundation::*,
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::WindowsAndMessaging::*,
    };
    struct State {
        sender: SyncSender<Hotkey>,
        ptt: Option<u32>,
        down: BTreeSet<u32>,
        control: Arc<AtomicBool>,
        notify: Option<Arc<dyn Fn() + Send + Sync>>,
        emergency: Option<Arc<dyn Fn() + Send + Sync>>,
    }
    static STATE: OnceLock<Mutex<Option<State>>> = OnceLock::new();
    unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code >= 0 {
            let data = &*(l as *const KBDLLHOOKSTRUCT);
            if data.flags & LLKHF_INJECTED == 0 {
                if let Ok(mut state) = STATE.get().unwrap().lock() {
                    if let Some(s) = state.as_mut() {
                        let down = w as u32 == WM_KEYDOWN || w as u32 == WM_SYSKEYDOWN;
                        let repeat = if down {
                            !s.down.insert(data.vkCode)
                        } else {
                            !s.down.remove(&data.vkCode)
                        };
                        if !repeat {
                            let ctrl = s.down.contains(&0xa2) || s.down.contains(&0xa3);
                            let shift = s.down.contains(&0xa0) || s.down.contains(&0xa1);
                            let alt = s.down.contains(&0xa4) || s.down.contains(&0xa5);
                            let event = action(s.ptt, data.vkCode, down, ctrl, shift, alt);
                            if let Some(event) = event {
                                if event == Hotkey::Revoke {
                                    s.control.store(false, Ordering::Release);
                                    if let Some(stop) = &s.emergency {
                                        stop();
                                    }
                                }
                                let _ = s.sender.try_send(event);
                                if let Some(notify) = &s.notify {
                                    notify();
                                }
                            }
                        }
                    }
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, w, l)
    }
    unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code >= 0
            && w as u32 == WM_MOUSEMOVE
            && physical_mouse((*(l as *const MSLLHOOKSTRUCT)).flags)
        {
            if let Ok(state) = STATE.get().unwrap().lock() {
                if let Some(s) = state.as_ref() {
                    if s.control.swap(false, Ordering::AcqRel) {
                        if let Some(stop) = &s.emergency {
                            stop();
                        }
                        let _ = s.sender.try_send(Hotkey::Revoke);
                        if let Some(notify) = &s.notify {
                            notify();
                        }
                    }
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, w, l)
    }
    pub struct Hooks {
        id: u32,
        thread: Option<JoinHandle<()>>,
        pub receiver: Receiver<Hotkey>,
        control: Arc<AtomicBool>,
    }
    impl Hooks {
        pub fn start(ptt: Option<u32>) -> Result<Self, String> {
            let (sender, receiver) = sync_channel(128);
            let (ready_tx, ready) = sync_channel(1);
            let state = STATE.get_or_init(|| Mutex::new(None));
            let control = Arc::new(AtomicBool::new(false));
            {
                let mut s = state.lock().map_err(|_| "Hotkey state poisoned")?;
                if s.is_some() {
                    return Err("Hotkeys already active".into());
                }
                *s = Some(State {
                    sender,
                    ptt,
                    down: BTreeSet::new(),
                    control: control.clone(),
                    notify: None,
                    emergency: None,
                });
            }
            let thread = thread::Builder::new()
                .name("sticktalk-hotkeys".into())
                .spawn(move || unsafe {
                    let id = GetCurrentThreadId();
                    let mut msg = std::mem::zeroed();
                    PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                    let module = GetModuleHandleW(std::ptr::null());
                    let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), module, 0);
                    let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), module, 0);
                    if keyboard.is_null() || mouse.is_null() {
                        if !keyboard.is_null() {
                            UnhookWindowsHookEx(keyboard);
                        }
                        if !mouse.is_null() {
                            UnhookWindowsHookEx(mouse);
                        }
                        let _ = ready_tx.send(Err("Global hotkey hooks unavailable".to_string()));
                    } else {
                        let _ = ready_tx.send(Ok(id));
                        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                        UnhookWindowsHookEx(keyboard);
                        UnhookWindowsHookEx(mouse);
                    }
                    if let Ok(mut s) = state.lock() {
                        *s = None;
                    }
                })
                .map_err(|e| e.to_string())?;
            match ready.recv().map_err(|e| e.to_string())? {
                Ok(id) => Ok(Self {
                    id,
                    thread: Some(thread),
                    receiver,
                    control,
                }),
                Err(e) => {
                    let _ = thread.join();
                    Err(e)
                }
            }
        }
        pub fn set_emergency(&self, stop: Arc<dyn Fn() + Send + Sync>) {
            if let Ok(mut state) = STATE.get().unwrap().lock() {
                if let Some(s) = state.as_mut() {
                    s.emergency = Some(stop);
                }
            }
        }
        pub fn control_flag(&self) -> Arc<AtomicBool> {
            self.control.clone()
        }
        pub fn set_control_active(&self, active: bool) {
            self.control.store(active, Ordering::Release);
        }
        pub fn set_notifier(&self, notify: Arc<dyn Fn() + Send + Sync>) {
            if let Ok(mut state) = STATE.get().unwrap().lock() {
                if let Some(state) = state.as_mut() {
                    state.notify = Some(notify);
                }
            }
        }
        pub fn set_ptt_key(&self, key: Option<u32>) {
            if let Ok(mut state) = STATE.get().unwrap().lock() {
                if let Some(state) = state.as_mut() {
                    state.ptt = key;
                }
            }
        }
    }
    impl Drop for Hooks {
        fn drop(&mut self) {
            unsafe {
                PostThreadMessageW(self.id, WM_QUIT, 0, 0);
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}
#[cfg(windows)]
pub use native::Hooks;
