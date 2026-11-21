//! Remote control of this PC, for a screen or window being shared in a call.
//!
//! The server's page forwards a viewer's pointer and key input here, and this
//! module performs it. A page is not trusted to decide that on its own:
//!
//! - The question "may this person control your screen?" is asked by a Windows
//!   message box from this process, never by the page. Nothing is performed
//!   until the person at this PC answers Yes.
//! - Input lands only on the shared screen or window: the pointer is mapped
//!   into its bounds, and keys go nowhere while keyboard focus is outside it.
//! - The Windows key is refused, and Esc pressed on the real keyboard ends the
//!   session at once (input performed here never counts as that).
//!
//! Building what to send is pure and tested. Tests never send input.
use serde::Deserialize;
use std::{collections::BTreeSet, time::Instant};

/// One step of a viewer's input. Same shape as the protocol's `ControlInput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Input {
    /// A point on the shared surface: 0 is its left/top edge, 65535 its right/bottom.
    Move {
        x: u16,
        y: u16,
    },
    /// 0 left, 1 right, 2 middle, 3 and 4 the side buttons.
    Button {
        button: u8,
        down: bool,
    },
    Wheel {
        x: i16,
        y: i16,
    },
    /// A USB HID keyboard usage (a physical key, not a character).
    Key {
        usage: u16,
        down: bool,
    },
}

/// What the page may say to the app.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case")]
pub enum Message {
    /// Someone asked to control the shared surface: ask the person here.
    Start {
        who: String,
        /// The captured track's label, which names the screen or window.
        label: String,
        surface: String,
        #[serde(default)]
        width: u32,
        #[serde(default)]
        height: u32,
    },
    Input {
        input: Input,
    },
    /// Let go of every held key and button; control continues.
    Release,
    Stop,
}

pub fn parse(raw: &str) -> Option<Message> {
    if raw.len() > 1024 {
        return None;
    }
    serde_json::from_str(raw).ok()
}

/// What is shared, by the number Chromium gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Screen(isize),
    Window(isize),
}

/// Chromium labels a captured track `screen:<id>:<n>` or `window:<id>:<n>`.
/// Anything else (a browser tab, an unknown label) cannot be controlled.
pub fn target(label: &str, surface: &str) -> Option<Target> {
    let mut parts = label.split(':');
    let kind = parts.next()?;
    let id = parts.next()?.parse::<i64>().ok()?;
    let id = isize::try_from(id).ok()?;
    match (kind, surface) {
        ("screen", "monitor") if id >= 0 => Some(Target::Screen(id)),
        ("window", "window") if id > 0 => Some(Target::Window(id)),
        _ => None,
    }
}

/// A rectangle in physical desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Surface {
    pub fn contains(self, x: i32, y: i32) -> bool {
        i64::from(x) >= i64::from(self.x)
            && i64::from(y) >= i64::from(self.y)
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
    /// The pixel a 16-bit point names.
    pub fn point(self, x: u16, y: u16) -> Option<(i32, i32)> {
        if self.width == 0 || self.height == 0 || self.width > 16384 || self.height > 16384 {
            return None;
        }
        let px = i64::from(self.x) + i64::from(x) * i64::from(self.width - 1) / 65535;
        let py = i64::from(self.y) + i64::from(y) * i64::from(self.height - 1) / 65535;
        Some((px.try_into().ok()?, py.try_into().ok()?))
    }
    /// The same point as Windows wants it: 0..65535 across the whole desktop.
    pub fn absolute(self, desktop: Surface, x: u16, y: u16) -> Option<(i32, i32)> {
        let (px, py) = self.point(x, y)?;
        if desktop.width < 2 || desktop.height < 2 || !desktop.contains(px, py) {
            return None;
        }
        // Aim at the middle of the pixel: rounding at its edge can land one
        // pixel outside the shared surface on a desktop that starts below zero.
        let scale = |p: i32, origin: i32, extent: u32| {
            (((i64::from(p) - i64::from(origin)) * 65536 + 32768) / i64::from(extent))
                .clamp(0, 65535) as i32
        };
        Some((
            scale(px, desktop.x, desktop.width),
            scale(py, desktop.y, desktop.height),
        ))
    }
    /// Whether a picture of `width` x `height` could be a picture of this
    /// surface. The page says which screen is shared; this catches a wrong guess.
    pub fn matches(self, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 || self.width == 0 || self.height == 0 {
            return false;
        }
        let ours = f64::from(self.width) / f64::from(self.height);
        let theirs = f64::from(width) / f64::from(height);
        (ours - theirs).abs() / ours <= 0.05
    }
}

/// USB HID keyboard usage to a set-1 scan code and whether it is an extended
/// key. Unknown keys, media keys and the Windows key have none.
pub fn scan(usage: u16) -> Option<(u16, bool)> {
    const LETTERS: [u16; 26] = [
        0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18,
        0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c,
    ];
    Some(match usage {
        4..=29 => (LETTERS[usize::from(usage - 4)], false),
        30..=38 => (usage - 28, false),
        39 => (0x0b, false),
        40 => (0x1c, false),
        41 => (0x01, false),
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
        _ => return None,
    })
}

/// What Windows would be asked to do for one input. No OS call is made here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// An absolute pointer position on the whole desktop.
    Pointer {
        x: i32,
        y: i32,
    },
    Button {
        button: u8,
        down: bool,
    },
    /// Positive `vertical` is away from the person, as Windows counts it.
    Wheel {
        horizontal: i16,
        vertical: i16,
    },
    Key {
        scan: u16,
        extended: bool,
        down: bool,
    },
}

/// `None` when the input must be dropped: an unknown key or button, or a point
/// outside the desktop.
pub fn plan(input: Input, surface: Surface, desktop: Surface) -> Option<Vec<Step>> {
    Some(match input {
        Input::Move { x, y } => {
            let (x, y) = surface.absolute(desktop, x, y)?;
            vec![Step::Pointer { x, y }]
        }
        Input::Button { button, down } if button <= 4 => vec![Step::Button { button, down }],
        Input::Button { .. } => return None,
        Input::Wheel { x, y } if x.unsigned_abs() <= 4096 && y.unsigned_abs() <= 4096 => {
            vec![Step::Wheel {
                horizontal: x,
                vertical: y,
            }]
        }
        Input::Wheel { .. } => return None,
        Input::Key { usage, down } => {
            let (scan, extended) = scan(usage)?;
            vec![Step::Key {
                scan,
                extended,
                down,
            }]
        }
    })
}

/// What is held down on the viewer's behalf, so it can all be let go.
#[derive(Default)]
pub struct Held {
    keys: BTreeSet<u16>,
    buttons: BTreeSet<u8>,
}

impl Held {
    /// False for an "up" of something this session never pressed: that must
    /// not be performed.
    pub fn holds(&self, input: Input) -> bool {
        match input {
            Input::Key { usage, down: false } => self.keys.contains(&usage),
            Input::Button {
                button,
                down: false,
            } => self.buttons.contains(&button),
            _ => true,
        }
    }
    /// Records a press or a release. A press is recorded before it is sent and
    /// a release only once Windows has taken it, so a failure either way leaves
    /// the key on the list of things still to let go.
    pub fn note(&mut self, input: Input) {
        match input {
            Input::Key { usage, down: true } => {
                self.keys.insert(usage);
            }
            Input::Key { usage, down: false } => {
                self.keys.remove(&usage);
            }
            Input::Button { button, down: true } => {
                self.buttons.insert(button);
            }
            Input::Button {
                button,
                down: false,
            } => {
                self.buttons.remove(&button);
            }
            _ => {}
        }
    }
    /// The inputs that would let go of everything held.
    pub fn releases(&self) -> Vec<Input> {
        self.keys
            .iter()
            .map(|usage| Input::Key {
                usage: *usage,
                down: false,
            })
            .chain(self.buttons.iter().map(|button| Input::Button {
                button: *button,
                down: false,
            }))
            .collect()
    }
}
/// At most this many inputs a second are performed; the rest are dropped.
const PER_SECOND: u32 = 1000;

/// Inputs handed to the window's event loop and not yet performed there.
static WAITING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// More than this many waiting means the page is sending faster than input
/// can be performed: control ends rather than fall behind.
const MOST_WAITING: usize = 256;

/// Called where the page's message arrives. False when the line is full: the
/// message must not be queued, and control should end.
pub fn admit(message: &Message) -> bool {
    use std::sync::atomic::Ordering;
    !matches!(message, Message::Input { .. })
        || WAITING
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |waiting| {
                (waiting < MOST_WAITING).then_some(waiting + 1)
            })
            .is_ok()
}

/// Called where a queued message is taken off the line (or could not join it).
pub fn taken(message: &Message) {
    use std::sync::atomic::Ordering;
    if matches!(message, Message::Input { .. }) {
        let _ = WAITING.try_update(Ordering::SeqCst, Ordering::SeqCst, |waiting| {
            waiting.checked_sub(1)
        });
    }
}

/// A name from the page, made safe to show in a message box.
pub fn display_name(who: &str) -> String {
    let name: String = who
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>()
        .trim()
        .to_string();
    if name.is_empty() {
        "Someone in your call".into()
    } else {
        name
    }
}

/// The title of the question, by which an unanswered one is found again.
const QUESTION: &str = "StickTalk remote control";

#[cfg(windows)]
pub use native::{ask, Session};

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        cell::RefCell,
        sync::atomic::{AtomicBool, Ordering},
    };
    use windows::{
        core::{w, HSTRING, PCWSTR},
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
            Graphics::{
                Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
                Gdi::{
                    EnumDisplayDevicesW, EnumDisplaySettingsExW, GetMonitorInfoW, DEVMODEW,
                    DISPLAY_DEVICEW, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS, HMONITOR,
                    MONITORINFO,
                },
            },
            System::Threading::{GetCurrentProcessId, GetCurrentThreadId},
            UI::{
                Input::KeyboardAndMouse::{
                    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
                    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
                    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
                    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
                    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
                    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP,
                    MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY, VK_ESCAPE,
                },
                WindowsAndMessaging::{
                    CallNextHookEx, FindWindowW, GetCursorPos, GetForegroundWindow,
                    GetGUIThreadInfo, GetMessageW, GetSystemMetrics, GetWindowTextW,
                    GetWindowThreadProcessId, IsChild, IsIconic, IsWindow, MessageBoxW,
                    PeekMessageW, PostMessageW, PostThreadMessageW, SetWindowsHookExW,
                    UnhookWindowsHookEx, WindowFromPoint, GUITHREADINFO, IDNO, IDYES,
                    KBDLLHOOKSTRUCT, LLKHF_INJECTED, MB_DEFBUTTON2, MB_ICONWARNING,
                    MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MSG, PM_NOREMOVE, SM_CXVIRTUALSCREEN,
                    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WH_KEYBOARD_LL,
                    WM_COMMAND, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN, WM_USER,
                },
            },
        },
    };

    fn rect(r: RECT) -> Option<Surface> {
        Some(Surface {
            x: r.left,
            y: r.top,
            width: u32::try_from(r.right - r.left).ok()?,
            height: u32::try_from(r.bottom - r.top).ok()?,
        })
    }

    /// The visible frame of a window. `GetWindowRect` would add its invisible
    /// resize borders.
    unsafe fn frame(window: HWND) -> Option<Surface> {
        let mut r = RECT::default();
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut r as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
        .ok()?;
        rect(r)
    }

    fn window(id: isize) -> HWND {
        HWND(id as *mut std::ffi::c_void)
    }

    /// Where the shared surface is right now, in physical pixels (this process
    /// is per-monitor DPI aware, see `windows.manifest`).
    unsafe fn bounds(target: Target) -> Result<Surface, String> {
        match target {
            Target::Screen(id) => {
                // Chromium names a screen either by its monitor handle or by
                // its display device number, depending on how it captures.
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if GetMonitorInfoW(HMONITOR(id as *mut std::ffi::c_void), &mut info).as_bool() {
                    return rect(info.rcMonitor)
                        .ok_or_else(|| "That screen is unavailable.".into());
                }
                let mut device = DISPLAY_DEVICEW {
                    cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                    ..Default::default()
                };
                let index = u32::try_from(id).map_err(|_| "That screen is unavailable.")?;
                if !EnumDisplayDevicesW(PCWSTR::null(), index, &mut device, 0).as_bool() {
                    return Err("That screen is no longer connected.".into());
                }
                let mut mode = DEVMODEW {
                    dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                    ..Default::default()
                };
                if !EnumDisplaySettingsExW(
                    PCWSTR(device.DeviceName.as_ptr()),
                    ENUM_CURRENT_SETTINGS,
                    &mut mode,
                    ENUM_DISPLAY_SETTINGS_FLAGS(0),
                )
                .as_bool()
                {
                    return Err("That screen is no longer connected.".into());
                }
                let at = mode.Anonymous1.Anonymous2.dmPosition;
                Ok(Surface {
                    x: at.x,
                    y: at.y,
                    width: mode.dmPelsWidth,
                    height: mode.dmPelsHeight,
                })
            }
            Target::Window(id) => {
                let window = window(id);
                if !IsWindow(Some(window)).as_bool() || IsIconic(window).as_bool() {
                    return Err("The shared window is closed or minimized.".into());
                }
                frame(window).ok_or_else(|| "The shared window is unavailable.".into())
            }
        }
    }

    fn desktop() -> Surface {
        unsafe {
            Surface {
                x: GetSystemMetrics(SM_XVIRTUALSCREEN),
                y: GetSystemMetrics(SM_YVIRTUALSCREEN),
                width: GetSystemMetrics(SM_CXVIRTUALSCREEN).max(0) as u32,
                height: GetSystemMetrics(SM_CYVIRTUALSCREEN).max(0) as u32,
            }
        }
    }

    fn mouse(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data as u32,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn native(step: Step) -> Vec<INPUT> {
        match step {
            Step::Pointer { x, y } => vec![mouse(
                x,
                y,
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            )],
            Step::Button { button, down } => {
                let (flags, data) = match (button, down) {
                    (0, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                    (0, false) => (MOUSEEVENTF_LEFTUP, 0),
                    (1, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                    (1, false) => (MOUSEEVENTF_RIGHTUP, 0),
                    (2, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                    (2, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                    (side, true) => (MOUSEEVENTF_XDOWN, i32::from(side) - 2),
                    (side, false) => (MOUSEEVENTF_XUP, i32::from(side) - 2),
                };
                vec![mouse(0, 0, data, flags)]
            }
            Step::Wheel {
                horizontal,
                vertical,
            } => {
                let mut events = vec![];
                if horizontal != 0 {
                    events.push(mouse(0, 0, i32::from(horizontal), MOUSEEVENTF_HWHEEL));
                }
                if vertical != 0 {
                    events.push(mouse(0, 0, i32::from(vertical), MOUSEEVENTF_WHEEL));
                }
                events
            }
            Step::Key {
                scan,
                extended,
                down,
            } => {
                let mut flags = KEYEVENTF_SCANCODE;
                if !down {
                    flags |= KEYEVENTF_KEYUP;
                }
                if extended {
                    flags |= KEYEVENTF_EXTENDEDKEY;
                }
                vec![INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: scan,
                            dwFlags: KEYBD_EVENT_FLAGS(flags.0),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                }]
            }
        }
    }

    /// Set the moment Esc is pressed on the real keyboard, and read before every
    /// injection: input already queued behind that key press is not performed.
    static ESCAPED: AtomicBool = AtomicBool::new(false);
    thread_local! {
        /// What the watching thread calls when that happens.
        static PRESSED: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    }

    /// Esc on the real keyboard ends the session. Input performed here is
    /// marked as injected by Windows and never counts.
    unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code >= 0 && matches!(w.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
            let key = &*(l.0 as *const KBDLLHOOKSTRUCT);
            if key.vkCode == u32::from(VK_ESCAPE.0) && (key.flags.0 & LLKHF_INJECTED.0) == 0 {
                ESCAPED.store(true, Ordering::SeqCst);
                PRESSED.with(|pressed| {
                    if let Some(pressed) = pressed.borrow().as_ref() {
                        pressed();
                    }
                });
            }
        }
        CallNextHookEx(None, code, w, l)
    }

    /// The Esc hook, on a thread of its own with nothing else to do. A busy
    /// window can then neither delay it nor have Windows drop it as too slow.
    struct Watch {
        thread: u32,
        done: Option<std::thread::JoinHandle<()>>,
    }

    impl Watch {
        fn start(pressed: impl Fn() + Send + 'static) -> Option<Self> {
            let (ready, started) = std::sync::mpsc::channel();
            let done = std::thread::spawn(move || unsafe {
                let mut message = MSG::default();
                // Gives this thread the message queue that stopping it posts to.
                let _ = PeekMessageW(&mut message, None, WM_USER, WM_USER, PM_NOREMOVE);
                let Ok(hook) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), None, 0) else {
                    let _ = ready.send(None);
                    return;
                };
                PRESSED.with(|slot| *slot.borrow_mut() = Some(Box::new(pressed)));
                let _ = ready.send(Some(GetCurrentThreadId()));
                // The hook is called from inside this wait. It ends at WM_QUIT.
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {}
                let _ = UnhookWindowsHookEx(hook);
            });
            let thread = started.recv().ok().flatten()?;
            Some(Self {
                thread,
                done: Some(done),
            })
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            unsafe {
                let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            if let Some(done) = self.done.take() {
                let _ = done.join();
            }
        }
    }

    /// A consented session. Dropping it lets go of everything held and removes
    /// the Esc hook.
    pub struct Session {
        target: Target,
        held: Held,
        second: Instant,
        count: u32,
        _watch: Watch,
    }

    impl Session {
        /// `escaped` is called, from another thread, when the person presses Esc.
        pub fn start(
            target: Target,
            width: u32,
            height: u32,
            escaped: impl Fn() + Send + 'static,
        ) -> Result<Self, String> {
            let surface = unsafe { bounds(target) }?;
            if let Target::Window(id) = target {
                let mut process = 0;
                unsafe { GetWindowThreadProcessId(window(id), Some(&mut process)) };
                if process == unsafe { GetCurrentProcessId() } {
                    return Err("StickTalk’s own window can’t be controlled.".into());
                }
            }
            if !surface.matches(width, height) {
                return Err("StickTalk couldn’t tell which screen is being shared.".into());
            }
            ESCAPED.store(false, Ordering::SeqCst);
            let watch = Watch::start(escaped)
                .ok_or_else(|| "Windows did not allow the Esc safety key.".to_string())?;
            Ok(Self {
                target,
                held: Held::default(),
                second: Instant::now(),
                count: 0,
                _watch: watch,
            })
        }

        /// Performs one input. `Err` ends the session; an input that may not
        /// be performed right now is dropped without ending it.
        pub fn input(&mut self, input: Input) -> Result<(), String> {
            if ESCAPED.load(Ordering::SeqCst) {
                return Err("You stopped remote control.".into());
            }
            let now = Instant::now();
            if now.duration_since(self.second).as_secs() >= 1 {
                self.second = now;
                self.count = 0;
            }
            self.count += 1;
            let releasing = matches!(
                input,
                Input::Key { down: false, .. } | Input::Button { down: false, .. }
            );
            // Letting go is always allowed; everything else has a budget.
            if self.count > PER_SECOND && !releasing {
                return Ok(());
            }
            let surface = unsafe { bounds(self.target) }?;
            if !releasing && !self.allowed(input, surface) {
                return Ok(());
            }
            let Some(steps) = plan(input, surface, desktop()) else {
                return Ok(());
            };
            if !self.held.holds(input) {
                return Ok(());
            }
            if !releasing {
                // Asked again at the last moment: the checks above take time.
                if ESCAPED.load(Ordering::SeqCst) {
                    return Err("You stopped remote control.".into());
                }
                self.held.note(input);
            }
            send(&steps)?;
            if releasing {
                self.held.note(input);
            }
            Ok(())
        }

        /// Pointer movement, clicks, scrolling and typing land only on what is
        /// shared. Windows delivers them by its own rules, not to a place we
        /// name, so each rule is checked: the window under the pointer, the
        /// window with keyboard focus (keys, and the wheel where Windows scrolls
        /// the focused window), and a window that has captured the mouse for a
        /// drag (it receives every pointer event, wherever the pointer is).
        fn allowed(&self, input: Input, surface: Surface) -> bool {
            unsafe {
                let mut gui = GUITHREADINFO {
                    cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                    ..Default::default()
                };
                // When Windows will not say who has the mouse, assume someone else.
                let captured = GetGUIThreadInfo(0, &mut gui)
                    .is_ok()
                    .then_some(gui.hwndCapture);
                let foreground = GetForegroundWindow();
                let (inside, shared): (Box<dyn Fn(HWND) -> bool>, bool) = match self.target {
                    // Entirely on the shared screen: nothing reaches a window
                    // on a screen the viewer cannot see.
                    Target::Screen(_) => (
                        Box::new(move |other| {
                            frame(other).is_some_and(|f| {
                                surface.contains(f.x, f.y)
                                    && surface.contains(
                                        f.x + f.width as i32 - 1,
                                        f.y + f.height as i32 - 1,
                                    )
                            })
                        }),
                        false,
                    ),
                    Target::Window(id) => {
                        let shared = window(id);
                        (
                            Box::new(move |other| {
                                other == shared || IsChild(shared, other).as_bool()
                            }),
                            true,
                        )
                    }
                };
                let free = captured.is_some_and(|holder| holder.is_invalid() || inside(holder));
                if matches!(input, Input::Move { .. }) {
                    return free;
                }
                let mut cursor = POINT::default();
                if GetCursorPos(&mut cursor).is_err() || !surface.contains(cursor.x, cursor.y) {
                    return false;
                }
                match input {
                    // A click lands on the window under the pointer. On a
                    // shared screen that is any window there; for a shared
                    // window it must be that window.
                    Input::Button { .. } => free && (!shared || inside(WindowFromPoint(cursor))),
                    Input::Wheel { .. } => {
                        free && inside(foreground) && (!shared || inside(WindowFromPoint(cursor)))
                    }
                    _ => inside(foreground),
                }
            }
        }
        /// Lets go of everything held. What Windows refuses stays on the list.
        pub fn release(&mut self) {
            let desktop = desktop();
            let anywhere = Surface {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            };
            for input in self.held.releases() {
                if plan(input, anywhere, desktop).is_some_and(|steps| send(&steps).is_ok()) {
                    self.held.note(input);
                }
            }
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            self.release();
            // Windows refuses input while an administrator window is in front.
            // A key left down would stay down, so keep trying for a while.
            let stuck = self.held.releases();
            if stuck.is_empty() {
                return;
            }
            std::thread::spawn(move || {
                let anywhere = Surface {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                };
                let mut stuck = stuck;
                for _ in 0..40 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    stuck.retain(|input| {
                        !plan(*input, anywhere, desktop()).is_some_and(|steps| send(&steps).is_ok())
                    });
                    if stuck.is_empty() {
                        break;
                    }
                }
            });
        }
    }

    /// The one place input is really sent.
    fn send(steps: &[Step]) -> Result<(), String> {
        let events: Vec<INPUT> = steps.iter().flat_map(|step| native(*step)).collect();
        if events.is_empty() {
            return Ok(());
        }
        let sent = unsafe { SendInput(&events, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != events.len() {
            return Err(
                "Windows refused the remote input. An administrator window may be in front.".into(),
            );
        }
        Ok(())
    }

    /// How long a question may stay on screen: the call forgets a request
    /// after 30 seconds.
    const QUESTION_SECONDS: u64 = 30;

    /// Asks the person at this PC, in a Windows message box that no page can
    /// draw or answer. Blocks until they answer: call it off the UI thread.
    /// `wanted` says whether the request still stands; once it does not, or
    /// after 30 seconds, the question is withdrawn as a "No".
    pub fn ask(
        who: &str,
        target: Target,
        wanted: impl Fn() -> bool + Send + Sync + 'static,
    ) -> bool {
        let wanted = std::sync::Arc::new(wanted);
        if !wanted() {
            return false;
        }
        let asked = Instant::now();
        let current = {
            let wanted = wanted.clone();
            move || wanted() && asked.elapsed().as_secs() < QUESTION_SECONDS
        };
        let answered = std::sync::Arc::new(AtomicBool::new(false));
        std::thread::spawn({
            let (answered, current) = (answered.clone(), current.clone());
            move || loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                // Its own question only: a later one has the same title.
                if answered.load(Ordering::SeqCst) {
                    break;
                }
                if !current() {
                    // Until it is answered: the box may not be up yet.
                    dismiss();
                }
            }
        });
        let allowed = question(who, target);
        answered.store(true, Ordering::SeqCst);
        // A "Yes" to a request that has run out or been withdrawn allows nothing.
        allowed && current()
    }

    fn question(who: &str, target: Target) -> bool {
        let what = match target {
            Target::Screen(_) => "your whole screen".to_string(),
            Target::Window(id) => {
                let mut title = [0u16; 128];
                let length = unsafe { GetWindowTextW(window(id), &mut title) }.max(0) as usize;
                let title = String::from_utf16_lossy(&title[..length.min(80)]);
                if title.trim().is_empty() {
                    "the window you are sharing".to_string()
                } else {
                    format!("the window “{}”", title.trim())
                }
            }
        };
        let text = format!(
            "{} wants to control {what} with their mouse and keyboard.\n\n\
             Only allow this if you are sharing in a call right now and expected \
             this request, and only for someone you trust. While it is on, they \
             can do what you can do there.\n\n\
             To stop at any time, press Esc on your own keyboard, or press Stop \
             in StickTalk.\n\nAllow remote control?",
            display_name(who)
        );
        let answer = unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(text),
                &HSTRING::from(QUESTION),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST,
            )
        };
        answer == IDYES
    }

    /// Withdraws a question nobody answered in time, as a "No".
    pub fn dismiss() {
        unsafe {
            let Ok(dialog) = FindWindowW(w!("#32770"), &HSTRING::from(QUESTION)) else {
                return;
            };
            let mut process = 0;
            GetWindowThreadProcessId(dialog, Some(&mut process));
            if process == GetCurrentProcessId() {
                let _ = PostMessageW(Some(dialog), WM_COMMAND, WPARAM(IDNO.0 as usize), LPARAM(0));
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Surface = Surface {
        x: -1920,
        y: -1080,
        width: 1920,
        height: 1080,
    };
    const DESKTOP: Surface = Surface {
        x: -1920,
        y: -1080,
        width: 4480,
        height: 2520,
    };

    #[test]
    fn page_messages_are_read_strictly() {
        assert_eq!(
            parse(
                r#"{"control":"start","who":"bob","label":"screen:0:0","surface":"monitor","width":1920,"height":1080}"#
            ),
            Some(Message::Start {
                who: "bob".into(),
                label: "screen:0:0".into(),
                surface: "monitor".into(),
                width: 1920,
                height: 1080
            })
        );
        assert_eq!(
            parse(r#"{"control":"input","input":{"type":"key","data":{"usage":4,"down":true}}}"#),
            Some(Message::Input {
                input: Input::Key {
                    usage: 4,
                    down: true
                }
            })
        );
        assert_eq!(parse(r#"{"control":"stop"}"#), Some(Message::Stop));
        assert_eq!(parse(r#"{"control":"release"}"#), Some(Message::Release));
        assert_eq!(parse(r#"{"control":"shutdown"}"#), None);
        assert_eq!(
            parse(r#"{"control":"input","input":{"type":"move","data":{"x":70000,"y":0}}}"#),
            None
        );
        assert_eq!(
            parse(&format!(
                r#"{{"control":"stop","pad":"{}"}}"#,
                "x".repeat(2000)
            )),
            None
        );
    }

    #[test]
    fn only_a_shared_screen_or_window_can_be_controlled() {
        assert_eq!(target("screen:0:0", "monitor"), Some(Target::Screen(0)));
        assert_eq!(
            target("screen:65537:0", "monitor"),
            Some(Target::Screen(65537))
        );
        assert_eq!(
            target("window:590338:0", "window"),
            Some(Target::Window(590338))
        );
        // A browser tab, a label that disagrees with the surface, or nonsense.
        assert_eq!(target("web-contents-media-stream://12:3", "browser"), None);
        assert_eq!(target("window:590338:0", "monitor"), None);
        assert_eq!(target("screen:0:0", "window"), None);
        assert_eq!(target("window:0:0", "window"), None);
        assert_eq!(target("screen:-1:0", "monitor"), None);
        assert_eq!(target("screen", "monitor"), None);
        assert_eq!(target("", ""), None);
    }

    #[test]
    fn points_stay_on_the_shared_surface() {
        assert_eq!(SCREEN.point(0, 0), Some((-1920, -1080)));
        assert_eq!(SCREEN.point(65535, 65535), Some((-1, -1)));
        let (x, y) = SCREEN.absolute(DESKTOP, 0, 0).unwrap();
        assert!((0..16).contains(&x) && (0..16).contains(&y));
        // The far corner of the left screen is still left of the middle of the desktop.
        let (x, _) = SCREEN.absolute(DESKTOP, 65535, 65535).unwrap();
        assert!(x < 65536 * 1920 / 4480);
        // A surface that is not on the desktop cannot be pointed at.
        let elsewhere = Surface { x: 9000, ..SCREEN };
        assert_eq!(elsewhere.absolute(DESKTOP, 0, 0), None);
        assert_eq!(Surface { width: 0, ..SCREEN }.point(0, 0), None);
    }

    #[test]
    fn a_picture_of_another_shape_is_not_this_surface() {
        assert!(SCREEN.matches(1920, 1080));
        assert!(SCREEN.matches(1280, 720));
        assert!(SCREEN.matches(1278, 720));
        assert!(!SCREEN.matches(1280, 1024));
        assert!(!SCREEN.matches(0, 0));
    }

    #[test]
    fn inputs_become_the_expected_steps() {
        assert_eq!(
            plan(
                Input::Key {
                    usage: 0xe4,
                    down: false
                },
                SCREEN,
                DESKTOP
            ),
            Some(vec![Step::Key {
                scan: 0x1d,
                extended: true,
                down: false
            }])
        );
        assert_eq!(
            plan(
                Input::Key {
                    usage: 4,
                    down: true
                },
                SCREEN,
                DESKTOP
            ),
            Some(vec![Step::Key {
                scan: 0x1e,
                extended: false,
                down: true
            }])
        );
        // The Windows keys, media keys and anything unknown are refused.
        for usage in [0xe3, 0xe7, 0x65, 0x80, 3, 500] {
            assert_eq!(
                plan(Input::Key { usage, down: true }, SCREEN, DESKTOP),
                None
            );
        }
        for button in 0..=4 {
            assert!(plan(Input::Button { button, down: true }, SCREEN, DESKTOP).is_some());
        }
        assert_eq!(
            plan(
                Input::Button {
                    button: 5,
                    down: true
                },
                SCREEN,
                DESKTOP
            ),
            None
        );
        assert_eq!(
            plan(Input::Wheel { x: -120, y: 120 }, SCREEN, DESKTOP),
            Some(vec![Step::Wheel {
                horizontal: -120,
                vertical: 120
            }])
        );
        assert_eq!(plan(Input::Wheel { x: 0, y: 5000 }, SCREEN, DESKTOP), None);
        assert!(matches!(
            plan(Input::Move { x: 100, y: 100 }, SCREEN, DESKTOP).as_deref(),
            Some([Step::Pointer { .. }])
        ));
    }

    #[test]
    fn everything_held_is_let_go_once() {
        let key = |usage, down| Input::Key { usage, down };
        let button = |button, down| Input::Button { button, down };
        let mut held = Held::default();
        for pressed in [key(4, true), key(0xe1, true), button(0, true)] {
            assert!(held.holds(pressed));
            held.note(pressed);
        }
        held.note(Input::Move { x: 1, y: 1 });
        assert!(held.holds(key(4, false)));
        held.note(key(4, false));
        // An "up" for something this session never pressed is not performed.
        assert!(!held.holds(key(4, false)));
        assert!(!held.holds(key(9, false)));
        assert!(!held.holds(button(1, false)));
        // A release Windows refused stays on the list for the next attempt.
        assert_eq!(held.releases(), vec![key(0xe1, false), button(0, false)]);
        held.note(key(0xe1, false));
        assert_eq!(held.releases(), vec![button(0, false)]);
        held.note(button(0, false));
        assert!(held.releases().is_empty());
    }

    #[test]
    fn names_are_safe_to_show() {
        assert_eq!(display_name("  bob\u{7}\n "), "bob");
        assert_eq!(display_name(""), "Someone in your call");
        assert_eq!(display_name(&"x".repeat(500)).chars().count(), 64);
    }
}
