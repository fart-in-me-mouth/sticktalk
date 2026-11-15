//! Remembered window size, position and maximized state. The file and its
//! format are the previous client's (`window.json`), so upgrades keep the spot.
use serde::{Deserialize, Serialize};
use std::path::Path;

/// `[left, top, right, bottom]` in physical pixels.
pub type Rect = [i32; 4];

pub const DEFAULT_SIZE: (f64, f64) = (1280.0, 800.0);
pub const MIN_SIZE: (f64, f64) = (940.0, 600.0);
const FILE: &str = "window.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    /// The restored (not maximized, not minimized) rectangle.
    pub rect: Rect,
    pub maximized: bool,
}

/// Moves and shrinks `rect` so all of it is on the monitor work area `work`:
/// a saved spot on a monitor that is gone, or bigger than today's screen,
/// still opens fully visible. Never smaller than the minimum unless the work
/// area itself is.
pub fn fit(rect: Rect, work: Rect, min: (i32, i32)) -> Rect {
    let span = |from: i32, to: i32| (i64::from(to) - i64::from(from)).max(1);
    let (work_w, work_h) = (span(work[0], work[2]), span(work[1], work[3]));
    let width = span(rect[0], rect[2]).clamp(i64::from(min.0).min(work_w), work_w);
    let height = span(rect[1], rect[3]).clamp(i64::from(min.1).min(work_h), work_h);
    let place = |at: i32, from: i32, to: i32, size: i64| {
        let from = i64::from(from);
        i64::from(at).clamp(from, (i64::from(to) - size).max(from))
    };
    let left = place(rect[0], work[0], work[2], width);
    let top = place(rect[1], work[1], work[3], height);
    // Each value lies inside `work`, so it fits an i32.
    [
        left as i32,
        top as i32,
        (left + width) as i32,
        (top + height) as i32,
    ]
}

/// First run: the default size at this monitor's scale, centered on it.
pub fn initial(work: Rect, scale: f64) -> Rect {
    let scale = if scale.is_finite() {
        scale.clamp(1.0, 8.0)
    } else {
        1.0
    };
    let px = |logical: f64| (logical * scale).round() as i32;
    let (width, height) = (px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1));
    let center = |from: i32, to: i32, size: i32| {
        ((i64::from(from) + i64::from(to) - i64::from(size)) / 2)
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    let left = center(work[0], work[2], width);
    let top = center(work[1], work[3], height);
    fit(
        [
            left,
            top,
            left.saturating_add(width),
            top.saturating_add(height),
        ],
        work,
        (px(MIN_SIZE.0), px(MIN_SIZE.1)),
    )
}

pub fn load(root: &Path) -> Option<Placement> {
    std::fs::read(root.join(FILE))
        .ok()
        .filter(|bytes| bytes.len() < 1024)
        .and_then(|bytes| serde_json::from_slice::<Placement>(&bytes).ok())
        .filter(|p| p.rect[2] > p.rect[0] && p.rect[3] > p.rect[1])
}

pub fn store(root: &Path, placement: &Placement) {
    let Ok(bytes) = serde_json::to_vec(placement) else {
        return;
    };
    let _ = std::fs::create_dir_all(root);
    let temporary = root.join("window.tmp");
    if std::fs::write(&temporary, bytes).is_ok() {
        let _ = std::fs::rename(temporary, root.join(FILE));
    }
}

#[cfg(windows)]
pub use native::{focus, restore, save, Show};

#[cfg(windows)]
mod native {
    use super::*;
    use windows::Win32::{
        Foundation::{HWND, RECT},
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromRect, MonitorFromWindow, HMONITOR, MONITORINFO,
            MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
        },
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    };

    pub enum Show {
        /// An ordinary launch: visible and active.
        Foreground,
        /// Started with Windows: on the taskbar, without taking focus.
        Minimized,
        /// A smoke run: placed like a real window, never shown.
        Hidden,
    }

    fn work_area(monitor: HMONITOR) -> Option<Rect> {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetMonitorInfoW(monitor, &mut info) }
            .as_bool()
            .then(|| {
                let r = info.rcWork;
                [r.left, r.top, r.right, r.bottom]
            })
    }

    fn placement(hwnd: HWND) -> Option<WINDOWPLACEMENT> {
        let mut placement = WINDOWPLACEMENT {
            length: size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        unsafe { GetWindowPlacement(hwnd, &mut placement) }
            .is_ok()
            .then_some(placement)
    }

    /// Call on move/resize/close events, never on a timer of its own.
    pub fn save(hwnd: HWND, root: &Path) {
        let Some(placement) = placement(hwnd) else {
            return;
        };
        let r = placement.rcNormalPosition;
        if r.right <= r.left || r.bottom <= r.top {
            return;
        }
        store(
            root,
            &Placement {
                rect: [r.left, r.top, r.right, r.bottom],
                maximized: placement.showCmd == SW_SHOWMAXIMIZED.0 as u32
                    || (placement.showCmd == SW_SHOWMINIMIZED.0 as u32
                        && placement.flags.contains(WPF_RESTORETOMAXIMIZED)),
            },
        );
    }

    /// Places the still-hidden window where it was last time and shows it.
    pub fn restore(hwnd: HWND, root: &Path, show: Show) {
        let scale = f64::from(unsafe { GetDpiForWindow(hwnd) }.max(96)) / 96.0;
        let min = (
            (MIN_SIZE.0 * scale).round() as i32,
            (MIN_SIZE.1 * scale).round() as i32,
        );
        let saved = load(root);
        let rect = match saved {
            Some(saved) => {
                let r = saved.rect;
                let monitor = unsafe {
                    MonitorFromRect(
                        &RECT {
                            left: r[0],
                            top: r[1],
                            right: r[2],
                            bottom: r[3],
                        },
                        MONITOR_DEFAULTTONEAREST,
                    )
                };
                work_area(monitor).map(|work| fit(r, work, min))
            }
            None => work_area(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY) })
                .map(|work| initial(work, scale)),
        };
        let maximized = saved.is_some_and(|p| p.maximized);
        let command = match show {
            Show::Foreground if maximized => SW_SHOWMAXIMIZED,
            Show::Foreground => SW_SHOWNORMAL,
            Show::Minimized => SW_SHOWMINNOACTIVE,
            Show::Hidden => SW_HIDE,
        };
        let placed = rect.zip(placement(hwnd)).is_some_and(|(r, mut placement)| {
            placement.showCmd = command.0 as u32;
            placement.flags = if maximized && matches!(show, Show::Minimized) {
                WPF_RESTORETOMAXIMIZED
            } else {
                WINDOWPLACEMENT_FLAGS(0)
            };
            placement.rcNormalPosition = RECT {
                left: r[0],
                top: r[1],
                right: r[2],
                bottom: r[3],
            };
            unsafe { SetWindowPlacement(hwnd, &placement) }.is_ok()
        });
        if !placed {
            let _ = unsafe { ShowWindow(hwnd, command) };
        }
    }

    /// Only after an explicit user action (a second launch or a link).
    pub fn focus(hwnd: HWND) {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                // Back to its previous state, maximized included.
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: (i32, i32) = (940, 600);
    const FULL_HD: Rect = [0, 0, 1920, 1040];

    fn inside(rect: Rect, work: Rect) -> bool {
        rect[0] >= work[0]
            && rect[1] >= work[1]
            && rect[2] <= work[2]
            && rect[3] <= work[3]
            && rect[2] > rect[0]
            && rect[3] > rect[1]
    }

    #[test]
    fn a_visible_window_is_left_where_it_was() {
        assert_eq!(
            fit([100, 80, 1380, 880], FULL_HD, MIN),
            [100, 80, 1380, 880]
        );
        // Second monitor to the left of the primary one.
        let left = [-1920, 0, 0, 1040];
        assert_eq!(
            fit([-1800, 50, -520, 850], left, MIN),
            [-1800, 50, -520, 850]
        );
    }

    #[test]
    fn a_window_from_a_removed_monitor_moves_fully_onto_the_screen() {
        let moved = fit([-5000, -2000, -3720, -1200], FULL_HD, MIN);
        assert_eq!(moved, [0, 0, 1280, 800]);
        let moved = fit([4000, 3000, 5280, 3800], FULL_HD, MIN);
        assert_eq!(moved, [640, 240, 1920, 1040]);
        // Half off the right edge: same size, pulled back in.
        assert_eq!(
            fit([1500, 100, 2780, 900], FULL_HD, MIN),
            [640, 100, 1920, 900]
        );
    }

    #[test]
    fn size_is_bounded_by_the_minimum_and_the_work_area() {
        // Bigger than the screen: shrinks to it.
        assert_eq!(fit([-100, -100, 4000, 3000], FULL_HD, MIN), FULL_HD);
        // Smaller than the minimum: grows to the minimum.
        assert_eq!(fit([10, 10, 110, 60], FULL_HD, MIN), [10, 10, 950, 610]);
        // A work area smaller than the minimum wins over the minimum.
        let small = [0, 0, 800, 500];
        assert_eq!(fit([-5000, -2000, -3000, -1000], small, MIN), small);
        // Degenerate input still yields a usable, visible window.
        for rect in [
            [0, 0, 0, 0],
            [500, 500, 100, 100],
            [i32::MIN, i32::MIN, i32::MAX, i32::MAX],
        ] {
            assert!(inside(fit(rect, FULL_HD, MIN), FULL_HD), "{rect:?}");
            assert!(
                inside(fit(rect, [0, 0, 0, 0], MIN), [0, 0, 1, 1]),
                "{rect:?}"
            );
        }
    }

    #[test]
    fn first_run_is_centered_and_scaled() {
        assert_eq!(initial(FULL_HD, 1.0), [320, 120, 1600, 920]);
        // 150% on a 4K work area: 1920x1200 physical, centered.
        assert_eq!(initial([0, 0, 3840, 2100], 1.5), [960, 450, 2880, 1650]);
        // A small laptop screen: as large as fits.
        assert_eq!(initial([0, 0, 1366, 728], 1.0), [43, 0, 1323, 728]);
        assert_eq!(initial([0, 0, 1024, 728], 1.0), [0, 0, 1024, 728]);
        for scale in [f64::NAN, f64::INFINITY, -3.0, 0.0, 100.0] {
            assert!(inside(initial(FULL_HD, scale), FULL_HD), "{scale}");
        }
    }

    #[test]
    fn reads_the_previous_clients_file_and_ignores_damage() {
        let root =
            std::env::temp_dir().join(format!("sticktalk-desktop-window-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(load(&root), None);
        std::fs::create_dir_all(&root).unwrap();
        // Byte-for-byte what the previous client wrote.
        std::fs::write(
            root.join(FILE),
            br#"{"rect":[100,80,1380,880],"maximized":true}"#,
        )
        .unwrap();
        let placement = Placement {
            rect: [100, 80, 1380, 880],
            maximized: true,
        };
        assert_eq!(load(&root), Some(placement));
        for damaged in [
            &b"not json"[..],
            br#"{"rect":[0,0,0,0],"maximized":false}"#,
            br#"{"rect":[10,10,5,5],"maximized":false}"#,
            br#"{"rect":[1,2,3],"maximized":false}"#,
            &[b' '; 2000],
        ] {
            std::fs::write(root.join(FILE), damaged).unwrap();
            assert_eq!(load(&root), None);
        }
        store(&root, &placement);
        assert_eq!(load(&root), Some(placement));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
