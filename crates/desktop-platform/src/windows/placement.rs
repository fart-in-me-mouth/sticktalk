//! Saved native placement. Capture only on user/OS events, never on a timer.
use super::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Placement {
    rect: [i32; 4],
    maximized: bool,
}
fn path(root: &Path) -> PathBuf {
    root.join("window.json")
}
pub fn save(hwnd: HWND, root: &Path) {
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_err() {
        return;
    }
    let r = placement.rcNormalPosition;
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let state = Placement {
        rect: [r.left, r.top, r.right, r.bottom],
        maximized: placement.showCmd == SW_SHOWMAXIMIZED.0 as u32
            || placement.flags.contains(WPF_RESTORETOMAXIMIZED),
    };
    let _ = std::fs::create_dir_all(root);
    if let Ok(bytes) = serde_json::to_vec(&state) {
        let temporary = root.join("window.tmp");
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(temporary, path(root));
        }
    }
}
pub fn restore(hwnd: HWND, root: &Path, background: bool) {
    if !restore_inner(hwnd, root, background) && !background {
        let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    }
}
fn restore_inner(hwnd: HWND, root: &Path, background: bool) -> bool {
    let Some(state) = std::fs::read(path(root))
        .ok()
        .filter(|v| v.len() < 1024)
        .and_then(|v| serde_json::from_slice::<Placement>(&v).ok())
    else {
        return false;
    };
    let [left, top, right, bottom] = state.rect;
    if right.checked_sub(left).is_none() || bottom.checked_sub(top).is_none() {
        return false;
    }
    let mut rect = windows::Win32::Foundation::RECT {
        left,
        top,
        right,
        bottom,
    };
    let monitor = unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return false;
    }
    state.clamp_to(&info.rcWork, &mut rect);
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        showCmd: if background {
            SW_HIDE.0 as u32
        } else if state.maximized {
            SW_SHOWMAXIMIZED.0 as u32
        } else {
            SW_SHOWNOACTIVATE.0 as u32
        },
        rcNormalPosition: rect,
        ..Default::default()
    };
    if background && state.maximized {
        placement.flags = WPF_RESTORETOMAXIMIZED;
    }
    unsafe { SetWindowPlacement(hwnd, &placement) }.is_ok()
}
/// Restoring an already-visible maximized window must not unmaximize it. Hidden
/// windows retain their last explicit maximized preference in the saved file.
pub fn show_saved(hwnd: HWND) {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("StickTalk");
    let maximized = std::fs::read(path(&root))
        .ok()
        .filter(|v| v.len() < 1024)
        .and_then(|v| serde_json::from_slice::<Placement>(&v).ok())
        .is_some_and(|p| p.maximized);
    let _ = unsafe {
        ShowWindow(
            hwnd,
            if maximized {
                SW_SHOWMAXIMIZED
            } else {
                SW_RESTORE
            },
        )
    };
}

impl Placement {
    fn clamp_to(
        &self,
        work: &windows::Win32::Foundation::RECT,
        rect: &mut windows::Win32::Foundation::RECT,
    ) {
        let width = (i64::from(rect.right) - i64::from(rect.left))
            .clamp(940, i64::from((work.right - work.left).max(940)))
            .min(i64::from(work.right - work.left));
        let height = (i64::from(rect.bottom) - i64::from(rect.top))
            .clamp(600, i64::from((work.bottom - work.top).max(600)))
            .min(i64::from(work.bottom - work.top));
        rect.left = rect.left.clamp(work.left, work.right - width as i32);
        rect.top = rect.top.clamp(work.top, work.bottom - height as i32);
        rect.right = rect.left + width as i32;
        rect.bottom = rect.top + height as i32;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removed_monitor_and_small_work_area_are_safe() {
        let p = Placement {
            rect: [-5000, -2000, -3000, -1000],
            maximized: true,
        };
        let mut r = windows::Win32::Foundation::RECT {
            left: -5000,
            top: -2000,
            right: -3000,
            bottom: -1000,
        };
        let work = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: 800,
            bottom: 500,
        };
        p.clamp_to(&work, &mut r);
        assert_eq!([r.left, r.top, r.right, r.bottom], [0, 0, 800, 500]);
    }
}
