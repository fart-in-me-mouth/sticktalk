//! Push to talk and push to mute while another app (a game) has focus.
//!
//! While the page is in a call with a push-to-talk or push-to-mute key, it asks
//! for this. A thread of its own then installs passive low-level keyboard (and,
//! for a mouse button binding, mouse) hooks. They need no administrator rights,
//! open no window, never take focus and never swallow input: every event is
//! passed on to the app it was meant for. Only the bound keys are reported, as
//! "pressed" and "released", and only real input: anything injected (by remote
//! control, say) is ignored. When the call ends the hooks are removed.
//!
//! Windows does not let a normal process see input sent to a window that runs
//! elevated (as administrator). A game started that way does not report its key
//! presses to these hooks; the page's own key handling still works whenever
//! StickTalk itself has focus.
use crate::control;
use serde::Deserialize;

/// A key as a USB HID keyboard usage, or a mouse button: 1 middle, 3 back
/// (XBUTTON1), 4 forward (XBUTTON2). Left and right buttons are never bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    #[serde(default)]
    pub usage: Option<u16>,
    #[serde(default)]
    pub button: Option<u8>,
}

/// What the hooks look for: a scan code, or a mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Key { scan: u16, extended: bool },
    Mouse(u8),
}

impl Binding {
    pub fn target(self) -> Option<Target> {
        match (self.usage, self.button) {
            (Some(usage), None) => {
                let (scan, extended) = control::scan(usage)?;
                Some(Target::Key { scan, extended })
            }
            (None, Some(button @ (1 | 3 | 4))) => Some(Target::Mouse(button)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Talk,
    Mute,
}

impl Which {
    pub fn name(self) -> &'static str {
        match self {
            Which::Talk => "talk",
            Which::Mute => "mute",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Start {
        talk: Option<Target>,
        mute: Option<Target>,
    },
    Stop,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    ptt: String,
    #[serde(default)]
    talk: Option<Binding>,
    #[serde(default)]
    mute: Option<Binding>,
}

/// A page message about push to talk, read strictly. Anything else is not one.
pub fn parse(raw: &str) -> Option<Message> {
    if raw.len() > 256 || !raw.contains("\"ptt\"") {
        return None;
    }
    let raw: Raw = serde_json::from_str(raw).ok()?;
    match raw.ptt.as_str() {
        "start" => {
            let talk = match raw.talk {
                Some(b) => Some(b.target()?),
                None => None,
            };
            let mute = match raw.mute {
                Some(b) => Some(b.target()?),
                None => None,
            };
            (talk.is_some() || mute.is_some()).then_some(Message::Start { talk, mute })
        }
        "stop" if raw.talk.is_none() && raw.mute.is_none() => Some(Message::Stop),
        _ => None,
    }
}

/// Which binding an input event is, given what is bound. Pure, for the hooks
/// and the tests.
pub fn classify(talk: Option<Target>, mute: Option<Target>, input: Target) -> Vec<Which> {
    let mut out = vec![];
    if talk == Some(input) {
        out.push(Which::Talk);
    }
    if mute == Some(input) {
        out.push(Which::Mute);
    }
    out
}

/// The script that tells the page a bound key went down or up.
pub fn script(which: Which, down: bool) -> String {
    format!(
        "window.dispatchEvent(new CustomEvent('sticktalk-host',{{detail:{{ptt:'{}',down:{down}}}}}))",
        which.name()
    )
}

#[cfg(windows)]
pub use windows_impl::Hook;

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use std::cell::RefCell;
    use windows::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{
            CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
            UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED,
            LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL,
            WM_KEYDOWN, WM_KEYUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_QUIT, WM_SYSKEYDOWN,
            WM_SYSKEYUP, WM_USER, WM_XBUTTONDOWN, WM_XBUTTONUP,
        },
    };

    type Notify = Box<dyn Fn(Which, bool)>;
    struct Watched {
        talk: Option<Target>,
        mute: Option<Target>,
        held: [bool; 2],
        notify: Notify,
    }
    thread_local! {
        /// Only the hook thread touches this; the hooks are called on it.
        static WATCHED: RefCell<Option<Watched>> = const { RefCell::new(None) };
    }

    fn report(input: Target, down: bool) {
        WATCHED.with(|w| {
            if let Some(w) = w.borrow_mut().as_mut() {
                for which in classify(w.talk, w.mute, input) {
                    let slot = &mut w.held[which as usize];
                    // Key repeat sends "down" again and again: report changes only.
                    if *slot != down {
                        *slot = down;
                        (w.notify)(which, down);
                    }
                }
            }
        });
    }

    unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code >= 0 {
            let key = &*(l.0 as *const KBDLLHOOKSTRUCT);
            let down = match w.0 as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
                WM_KEYUP | WM_SYSKEYUP => Some(false),
                _ => None,
            };
            if let Some(down) = down.filter(|_| key.flags.0 & LLKHF_INJECTED.0 == 0) {
                report(
                    Target::Key {
                        scan: key.scanCode as u16,
                        extended: key.flags.0 & LLKHF_EXTENDED.0 != 0,
                    },
                    down,
                );
            }
        }
        // Passive: the key still reaches the app it was meant for.
        CallNextHookEx(None, code, w, l)
    }

    unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code >= 0 {
            let event = &*(l.0 as *const MSLLHOOKSTRUCT);
            let x = (event.mouseData >> 16) as u16;
            let input = match w.0 as u32 {
                WM_MBUTTONDOWN => Some((1, true)),
                WM_MBUTTONUP => Some((1, false)),
                WM_XBUTTONDOWN => Some((if x == 1 { 3 } else { 4 }, true)),
                WM_XBUTTONUP => Some((if x == 1 { 3 } else { 4 }, false)),
                _ => None,
            };
            if let Some((button, down)) = input.filter(|_| event.flags & LLMHF_INJECTED == 0) {
                report(Target::Mouse(button), down);
            }
        }
        CallNextHookEx(None, code, w, l)
    }

    /// The running hooks. Dropping it removes them and ends their thread.
    pub struct Hook {
        thread: u32,
        done: Option<std::thread::JoinHandle<()>>,
    }

    impl Hook {
        /// `notify` is called on the hook thread; keep it to a channel send.
        pub fn start(
            talk: Option<Target>,
            mute: Option<Target>,
            notify: impl Fn(Which, bool) + Send + 'static,
        ) -> Option<Self> {
            let wants_mouse = [talk, mute]
                .iter()
                .any(|t| matches!(t, Some(Target::Mouse(_))));
            let wants_keys = [talk, mute]
                .iter()
                .any(|t| matches!(t, Some(Target::Key { .. })));
            let (ready, started) = std::sync::mpsc::channel();
            let done = std::thread::spawn(move || unsafe {
                let mut message = MSG::default();
                // Gives this thread the message queue that stopping it posts to.
                let _ = PeekMessageW(&mut message, None, WM_USER, WM_USER, PM_NOREMOVE);
                WATCHED.with(|w| {
                    *w.borrow_mut() = Some(Watched {
                        talk,
                        mute,
                        held: [false; 2],
                        notify: Box::new(notify),
                    })
                });
                let install =
                    |kind, proc_: unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT| {
                        SetWindowsHookExW(kind, Some(proc_), None, 0).ok()
                    };
                let hooks: Vec<HHOOK> = [
                    wants_keys
                        .then(|| install(WH_KEYBOARD_LL, keyboard))
                        .flatten(),
                    wants_mouse.then(|| install(WH_MOUSE_LL, mouse)).flatten(),
                ]
                .into_iter()
                .flatten()
                .collect();
                if hooks.is_empty() {
                    let _ = ready.send(None);
                    return;
                }
                let _ = ready.send(Some(GetCurrentThreadId()));
                // The hooks are called from inside this wait. It ends at WM_QUIT.
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {}
                for hook in hooks {
                    let _ = UnhookWindowsHookEx(hook);
                }
                WATCHED.with(|w| *w.borrow_mut() = None);
            });
            let thread = started.recv().ok().flatten()?;
            Some(Self {
                thread,
                done: Some(done),
            })
        }
    }

    impl Drop for Hook {
        fn drop(&mut self) {
            unsafe {
                let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            if let Some(done) = self.done.take() {
                let _ = done.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_messages_are_read_strictly() {
        let f = Target::Key {
            scan: 0x21,
            extended: false,
        };
        assert_eq!(
            parse(r#"{"ptt":"start","talk":{"usage":9},"mute":null}"#),
            Some(Message::Start {
                talk: Some(f),
                mute: None
            })
        );
        assert_eq!(
            parse(r#"{"ptt":"start","talk":null,"mute":{"button":4}}"#),
            Some(Message::Start {
                talk: None,
                mute: Some(Target::Mouse(4))
            })
        );
        assert_eq!(parse(r#"{"ptt":"stop"}"#), Some(Message::Stop));
        for bad in [
            r#"{"ptt":"start"}"#,
            r#"{"ptt":"start","talk":null,"mute":null}"#,
            r#"{"ptt":"start","talk":{"button":0}}"#,
            r#"{"ptt":"start","talk":{"button":2}}"#,
            r#"{"ptt":"start","talk":{"usage":9,"button":1}}"#,
            r#"{"ptt":"start","talk":{"usage":227}}"#,
            r#"{"ptt":"start","talk":{"usage":9},"extra":1}"#,
            r#"{"ptt":"stop","talk":{"usage":9}}"#,
            r#"{"ptt":"record"}"#,
            r#"{"control":"start"}"#,
            "not json",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert_eq!(
            parse(&format!(r#"{{"ptt":"stop","x":"{}"}}"#, "a".repeat(300))),
            None
        );
    }

    #[test]
    fn keys_map_to_the_scan_codes_the_hook_sees() {
        let target = |usage| {
            Binding {
                usage: Some(usage),
                button: None,
            }
            .target()
        };
        // F, the right-hand Ctrl (extended), and the Up arrow (extended).
        assert_eq!(
            target(9),
            Some(Target::Key {
                scan: 0x21,
                extended: false
            })
        );
        assert_eq!(
            target(0xe4),
            Some(Target::Key {
                scan: 0x1d,
                extended: true
            })
        );
        assert_eq!(
            target(82),
            Some(Target::Key {
                scan: 0x48,
                extended: true
            })
        );
        // The left Ctrl is a different key from the right one.
        assert_ne!(target(0xe0), target(0xe4));
    }

    #[test]
    fn one_input_can_be_both_bindings_and_others_are_ignored() {
        let v = Target::Key {
            scan: 0x2f,
            extended: false,
        };
        let other = Target::Key {
            scan: 0x2e,
            extended: false,
        };
        assert_eq!(classify(Some(v), None, v), vec![Which::Talk]);
        assert_eq!(
            classify(Some(v), Some(v), v),
            vec![Which::Talk, Which::Mute]
        );
        assert!(classify(Some(v), Some(Target::Mouse(3)), other).is_empty());
        assert_eq!(
            classify(None, Some(Target::Mouse(3)), Target::Mouse(3)),
            vec![Which::Mute]
        );
    }

    #[test]
    fn the_page_is_told_with_a_fixed_script() {
        assert_eq!(
            script(Which::Talk, true),
            "window.dispatchEvent(new CustomEvent('sticktalk-host',{detail:{ptt:'talk',down:true}}))"
        );
        assert_eq!(
            script(Which::Mute, false),
            "window.dispatchEvent(new CustomEvent('sticktalk-host',{detail:{ptt:'mute',down:false}}))"
        );
    }
}
