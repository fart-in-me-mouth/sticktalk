//! Tray attached to the application's existing HWND. No hidden helper window,
//! polling thread or global hook; callbacks run on the owning UI thread.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use tokio::sync::mpsc::Sender;
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
};
const CALLBACK: u32 = WM_APP + 19;
const SUBCLASS: usize = 0x5354;
static AVAILABLE: AtomicBool = AtomicBool::new(false);
static STATE: AtomicU8 = AtomicU8::new(0);
struct Host {
    tray: TrayIcon,
    tx: Sender<TrayAction>,
    explorer: u32,
}
pub fn available() -> bool {
    AVAILABLE.load(Ordering::Acquire)
}
pub fn state(muted: bool, deafened: bool) {
    STATE.store(
        u8::from(muted) | (u8::from(deafened) << 1),
        Ordering::Release,
    );
}
/// The HWND must belong to the calling thread. WM_NCDESTROY releases ownership.
pub fn install(hwnd: HWND, tx: Sender<TrayAction>) -> Result<()> {
    if available() {
        return Ok(());
    }
    let tray = TrayIcon::add(hwnd, CALLBACK, Icon::brand()?)?;
    let host = Box::new(Host {
        tray,
        tx,
        explorer: unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) },
    });
    let ptr = Box::into_raw(host);
    unsafe {
        if !SetWindowSubclass(hwnd, Some(callback), SUBCLASS, ptr as usize).as_bool() {
            drop(Box::from_raw(ptr));
            return Err(Error::Invalid("could not attach tray callback"));
        }
    }
    AVAILABLE.store(true, Ordering::Release);
    Ok(())
}
unsafe extern "system" fn callback(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    let host = &*(data as *const Host);
    if message == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(callback), SUBCLASS);
        AVAILABLE.store(false, Ordering::Release);
        drop(Box::from_raw(data as *mut Host));
        return DefSubclassProc(hwnd, message, w, l);
    }
    if message == host.explorer {
        let _ = host.tray.restore_after_explorer_restart();
    }
    if message == CALLBACK {
        let flags = STATE.load(Ordering::Acquire);
        if let Ok(Some(action)) = host
            .tray
            .on_callback(l.0 as u32, flags & 1 != 0, flags & 2 != 0)
        {
            let _ = host.tx.try_send(action);
        }
        return LRESULT(0);
    }
    DefSubclassProc(hwnd, message, w, l)
}
