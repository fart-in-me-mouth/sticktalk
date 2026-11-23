//! Native input construction is pure and independently testable. Tests NEVER call SendInput.
use super::{Injector, Surface};
use sticktalk_proto::ControlInput;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

/// USB HID keyboard usages -> Set-1 scan codes. Unknown/media usages fail closed.
fn scan(usage: u16) -> Option<(u16, bool)> {
    const LETTERS: [u16; 26] = [
        0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18,
        0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c,
    ];
    Some(match usage {
        4..=29 => (LETTERS[(usage - 4) as usize], false),
        30..=38 => (usage - 28, false),
        39 => (0x0b, false),
        40 => (0x1c, false),
        41 => (1, false),
        42 => (0x0e, false),
        43 => (0x0f, false),
        44 => (0x39, false),
        45..=46 => (usage - 33, false),
        47..=48 => (usage - 21, false),
        49 => (0x2b, false),
        51 => (0x27, false),
        52 => (0x28, false),
        53 => (0x29, false),
        54..=56 => (usage - 3, false),
        57 => (0x3a, false),
        58..=67 => (usage + 1, false),
        68 => (0x57, false),
        69 => (0x58, false),
        73 => (0x52, true),
        74 => (0x47, true),
        75 => (0x49, true),
        76 => (0x53, true),
        77 => (0x4f, true),
        78 => (0x51, true),
        79 => (0x4d, true),
        80 => (0x4b, true),
        81 => (0x50, true),
        82 => (0x48, true),
        0xe0 => (0x1d, false),
        0xe1 => (0x2a, false),
        0xe2 => (0x38, false),
        0xe4 => (0x1d, true),
        0xe5 => (0x36, false),
        0xe6 => (0x38, true),
        // Windows/Super is deliberately not allowed to escape to OS shell actions.
        _ => return None,
    })
}
/// Pure construction. No OS calls, focus changes, or injection.
pub fn build(
    input: &ControlInput,
    surface: Surface,
    desktop: Surface,
) -> Result<Vec<INPUT>, String> {
    let mouse = |dx, dy, data, flags| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    Ok(match *input {
        ControlInput::Move { x, y } => {
            let (x, y) = surface.absolute(desktop, x, y)?;
            vec![mouse(
                x,
                y,
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            )]
        }
        ControlInput::Button { button, down } => {
            let (flags, data) = match (button, down) {
                (0, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                (0, false) => (MOUSEEVENTF_LEFTUP, 0),
                (1, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                (1, false) => (MOUSEEVENTF_RIGHTUP, 0),
                (2, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                (2, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                (3..=4, _) => (
                    if down {
                        MOUSEEVENTF_XDOWN
                    } else {
                        MOUSEEVENTF_XUP
                    },
                    if button == 3 { 1 } else { 2 },
                ),
                _ => return Err("Unknown mouse button".into()),
            };
            vec![mouse(0, 0, data, flags)]
        }
        ControlInput::Wheel { x, y } => {
            let mut events = vec![];
            if x != 0 {
                events.push(mouse(0, 0, i32::from(x) as u32, MOUSEEVENTF_HWHEEL));
            }
            if y != 0 {
                events.push(mouse(0, 0, i32::from(y) as u32, MOUSEEVENTF_WHEEL));
            }
            events
        }
        ControlInput::Key { usage, down } => {
            let (scan, extended) = scan(usage).ok_or("Unsupported key")?;
            vec![INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: 0,
                        wScan: scan,
                        dwFlags: KEYEVENTF_SCANCODE
                            | if down { 0 } else { KEYEVENTF_KEYUP }
                            | if extended { KEYEVENTF_EXTENDEDKEY } else { 0 },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }]
        }
        // Raw mouse movement exists for AI agents, which this client never allows.
        ControlInput::MoveBy { .. } => return Err("Unsupported input".into()),
    })
}

use sticktalk_media::{Source, SourceKind};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

pub(crate) struct NativeInjector {
    pub source: Source,
}
impl Injector for NativeInjector {
    fn input(&mut self, input: &ControlInput, _: Surface) -> Result<(), String> {
        let release = matches!(
            input,
            ControlInput::Key { down: false, .. } | ControlInput::Button { down: false, .. }
        );
        unsafe {
            // All bounds/cursor APIs here operate in physical pixels, even at mixed DPI.
            let old = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            struct Dpi(DPI_AWARENESS_CONTEXT);
            impl Drop for Dpi {
                fn drop(&mut self) {
                    unsafe {
                        SetThreadDpiAwarenessContext(self.0);
                    }
                }
            }
            if old.is_null() {
                return Err("Could not enter physical desktop coordinates".into());
            }
            let _dpi = Dpi(old);
            let surface = if release {
                Surface {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                }
            } else {
                bounds(&self.source)?
            };
            let desktop = Surface {
                x: GetSystemMetrics(SM_XVIRTUALSCREEN),
                y: GetSystemMetrics(SM_YVIRTUALSCREEN),
                width: GetSystemMetrics(SM_CXVIRTUALSCREEN) as u32,
                height: GetSystemMetrics(SM_CYVIRTUALSCREEN) as u32,
            };
            if !release && !matches!(input, ControlInput::Move { .. }) {
                let mut cursor = POINT::default();
                if GetCursorPos(&mut cursor) == 0 || !surface.contains(cursor.x, cursor.y) {
                    return Err("Pointer left shared surface".into());
                }
                if self.source.kind == SourceKind::Monitor
                    && matches!(input, ControlInput::Key { .. })
                {
                    let foreground = visible_window_rect(GetForegroundWindow())?;
                    if !surface.contains(foreground.left, foreground.top)
                        || !surface.contains(foreground.right - 1, foreground.bottom - 1)
                    {
                        return Err("Keyboard focus is outside the shared monitor".into());
                    }
                }
                if self.source.kind == SourceKind::Window {
                    let window = self.source.id as HWND;
                    let foreground = GetForegroundWindow();
                    if matches!(input, ControlInput::Key { .. } | ControlInput::Wheel { .. })
                        && foreground != window
                        && IsChild(window, foreground) == 0
                    {
                        // A click below can activate the unoccluded shared
                        // window through normal Windows input routing. Requiring
                        // SetForegroundWindow BEFORE that click incorrectly
                        // rejects legitimate control under the foreground lock.
                        // Keys/wheel must still fail closed unless focus is here.
                        if SetForegroundWindow(window) == 0 || GetForegroundWindow() != window {
                            return Err("Could not focus the shared window".into());
                        }
                    }
                    if matches!(
                        input,
                        ControlInput::Button { down: true, .. } | ControlInput::Wheel { .. }
                    ) {
                        let hit = WindowFromPoint(cursor);
                        if hit != window && IsChild(window, hit) == 0 {
                            return Err("Shared window is occluded at the pointer".into());
                        }
                    }
                }
            }
            let events = build(input, surface, desktop)?;
            // The sole real injection call. Ordinary tests only validate build();
            // the explicit ignored E2E test targets its own secondary HWND only.
            if SendInput(
                events.len() as u32,
                events.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            ) != events.len() as u32
            {
                return Err("Windows refused remote input".into());
            }
        }
        Ok(())
    }
}
// GetWindowRect includes invisible resize borders (and rejects maximized
// monitor keyboard focus by several pixels). WGC uses the visible DWM frame.
// https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-getwindowrect
#[link(name = "dwmapi")]
extern "system" {
    #[link_name = "DwmGetWindowAttribute"]
    fn dwm_attribute(hwnd: HWND, attribute: u32, out: *mut std::ffi::c_void, bytes: u32) -> i32;
}
unsafe fn visible_window_rect(hwnd: HWND) -> Result<RECT, String> {
    let mut rect = RECT::default();
    const EXTENDED_FRAME_BOUNDS: u32 = 9;
    if hwnd.is_null()
        || dwm_attribute(
            hwnd,
            EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        ) < 0
    {
        return Err("Visible shared-window bounds unavailable".into());
    }
    Ok(rect)
}
pub(super) unsafe fn bounds(source: &Source) -> Result<Surface, String> {
    let r = if source.kind == SourceKind::Monitor {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(source.id as HMONITOR, &mut info) == 0 {
            return Err("Shared monitor disappeared".into());
        }
        info.rcMonitor
    } else {
        if IsWindow(source.id as HWND) == 0 || IsIconic(source.id as HWND) != 0 {
            return Err("Shared window is unavailable".into());
        }
        visible_window_rect(source.id as HWND)?
    };
    let surface = Surface {
        x: r.left,
        y: r.top,
        width: (r.right - r.left).try_into().map_err(|_| "Invalid width")?,
        height: (r.bottom - r.top)
            .try_into()
            .map_err(|_| "Invalid height")?,
    };
    surface.point(65535, 65535)?;
    Ok(surface)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_structs_only_never_send_input() {
        let s = Surface {
            x: -1920,
            y: -1080,
            width: 1920,
            height: 1080,
        };
        let d = Surface {
            x: -1920,
            y: -1080,
            width: 4480,
            height: 2520,
        };
        let move_input = build(&ControlInput::Move { x: 0, y: 0 }, s, d).unwrap();
        unsafe {
            assert!((0..16).contains(&move_input[0].Anonymous.mi.dx));
            assert!((0..16).contains(&move_input[0].Anonymous.mi.dy));
            assert_eq!(
                move_input[0].Anonymous.mi.dwFlags,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK
            );
        }
        let key = build(
            &ControlInput::Key {
                usage: 0xe4,
                down: false,
            },
            s,
            d,
        )
        .unwrap();
        unsafe {
            assert_eq!(key[0].Anonymous.ki.wScan, 0x1d);
            assert_eq!(
                key[0].Anonymous.ki.dwFlags,
                KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
            );
        }
        assert!(build(
            &ControlInput::Key {
                usage: 0xe3,
                down: true
            },
            s,
            d
        )
        .is_err());
        for b in 0..5 {
            for down in [false, true] {
                assert_eq!(
                    build(&ControlInput::Button { button: b, down }, s, d)
                        .unwrap()
                        .len(),
                    1
                );
            }
        }
        assert!(build(
            &ControlInput::Button {
                button: 5,
                down: true
            },
            s,
            d
        )
        .is_err());
        assert_eq!(
            build(&ControlInput::Wheel { x: -120, y: 120 }, s, d)
                .unwrap()
                .len(),
            2
        );
    }
}
