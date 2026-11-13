//! Receiver-side grant/sequence/rate validation; tests only use DryRun.
use std::collections::BTreeSet;
use sticktalk_client_core::voice::control::Consent;
use sticktalk_proto::{ControlData, ControlInput, VoiceSession};

#[derive(Clone, Copy, Debug)]
pub struct Surface {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Surface {
    /// Both rectangles are physical desktop pixels, NOT iced logical pixels.
    /// Bounds queries must be made under PER_MONITOR_AWARE_V2 on Windows.
    pub fn absolute(self, desktop: Surface, x: u16, y: u16) -> Result<(i32, i32), String> {
        let (px, py) = self.point(x, y)?;
        let (right, bottom) = desktop.point(u16::MAX, u16::MAX)?;
        if desktop.width < 2
            || desktop.height < 2
            || px < desktop.x
            || py < desktop.y
            || px > right
            || py > bottom
        {
            return Err("Shared point is outside the virtual desktop".into());
        }
        let normalize = |p: i32, origin: i32, extent: u32| {
            // SendInput bins the 16-bit coordinate into desktop pixels. Aim at
            // the pixel center, not its edge: edge rounding can escape a shared
            // window by one pixel on a negative-origin or mixed-DPI desktop.
            (((i64::from(p) - i64::from(origin)) * 65536 + 32768) / i64::from(extent))
                .clamp(0, 65535) as i32
        };
        Ok((
            normalize(px, desktop.x, desktop.width),
            normalize(py, desktop.y, desktop.height),
        ))
    }
    pub fn contains(self, x: i32, y: i32) -> bool {
        i64::from(x) >= i64::from(self.x)
            && i64::from(y) >= i64::from(self.y)
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
    pub fn point(self, x: u16, y: u16) -> Result<(i32, i32), String> {
        if self.width == 0 || self.height == 0 || self.width > 16384 || self.height > 16384 {
            return Err("Invalid shared surface".into());
        }
        let px = i64::from(self.x) + i64::from(x) * i64::from(self.width - 1) / 65535;
        let py = i64::from(self.y) + i64::from(y) * i64::from(self.height - 1) / 65535;
        Ok((
            px.try_into().map_err(|_| "Surface overflow")?,
            py.try_into().map_err(|_| "Surface overflow")?,
        ))
    }
}
pub trait Injector {
    fn input(&mut self, input: &ControlInput, surface: Surface) -> Result<(), String>;
}
/// The same atomic gate is owned by the banner and passive emergency hooks.
/// Releasing held input remains possible AFTER the gate closes.
pub(super) struct Gated<I> {
    pub inner: I,
    pub active: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl<I: Injector> Injector for Gated<I> {
    fn input(&mut self, input: &ControlInput, surface: Surface) -> Result<(), String> {
        let release = matches!(
            input,
            ControlInput::Key { down: false, .. } | ControlInput::Button { down: false, .. }
        );
        if !release && !self.active.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Control was revoked".into());
        }
        self.inner.input(input, surface)
    }
}
#[derive(Default)]
pub struct DryRun {
    pub events: Vec<ControlInput>,
}
impl Injector for DryRun {
    fn input(&mut self, input: &ControlInput, _: Surface) -> Result<(), String> {
        if self.events.len() >= 4096 {
            return Err("Dry run log full".into());
        }
        self.events.push(input.clone());
        Ok(())
    }
}
pub struct Receiver<I: Injector> {
    pub injector: I,
    pub surface: Surface,
    grant: Option<u64>,
    revoked_grant: Option<u64>,
    last: Option<u64>,
    second: u64,
    count: u32,
    keys: BTreeSet<u16>,
    buttons: BTreeSet<u8>,
}
impl<I: Injector> Receiver<I> {
    pub fn new(injector: I, surface: Surface) -> Self {
        Self {
            injector,
            surface,
            grant: None,
            revoked_grant: None,
            last: None,
            second: 0,
            count: 0,
            keys: BTreeSet::new(),
            buttons: BTreeSet::new(),
        }
    }
    pub fn receive(
        &mut self,
        consent: &Consent,
        own: VoiceSession,
        data: ControlData,
        now_ms: u64,
    ) -> Result<(), String> {
        let ControlData::Input {
            sharer,
            grant,
            sequence,
            input,
        } = data
        else {
            if let ControlData::ReleaseAll { grant } = data {
                if self.grant == Some(grant) {
                    self.release()?;
                }
            }
            return Ok(());
        };
        if sharer != own || self.revoked_grant == Some(grant) || !consent.may_inject(own, grant) {
            self.release()?;
            return Err("No current local consent".into());
        }
        if self.grant != Some(grant) {
            self.release()?;
            self.grant = Some(grant);
            self.last = None;
        }
        if self.last.is_some_and(|s| sequence <= s) {
            self.release()?;
            return Err("Replayed control input".into());
        }
        let second = now_ms / 1000;
        if second != self.second {
            self.second = second;
            self.count = 0;
        }
        self.count += 1;
        if self.count > 1000 {
            self.release()?;
            return Err("Control rate exceeded".into());
        }
        match input {
            ControlInput::Button { button, .. } if button > 4 => {
                self.release()?;
                return Err("Invalid button".into());
            }
            ControlInput::Key { usage, .. } if !(4..=0xe7).contains(&usage) => {
                self.release()?;
                return Err("Invalid key".into());
            }
            _ => {}
        }
        if let Err(error) = self.injector.input(&input, self.surface) {
            let _ = self.release();
            return Err(error);
        }
        match input {
            ControlInput::Key { usage, down } => {
                if down {
                    self.keys.insert(usage);
                } else {
                    self.keys.remove(&usage);
                }
            }
            ControlInput::Button { button, down } => {
                if down {
                    self.buttons.insert(button);
                } else {
                    self.buttons.remove(&button);
                }
            }
            _ => {}
        }
        self.last = Some(sequence);
        Ok(())
    }
    /// On BOTH WS revoke and SCTP ReleaseAll, physical movement, stream stop,
    /// hotkey, transport loss. Attempt every release even if one injection fails.
    pub fn release(&mut self) -> Result<(), String> {
        if let Some(grant) = self.grant {
            self.revoked_grant = Some(grant);
        }
        let mut error = None;
        for usage in std::mem::take(&mut self.keys) {
            if let Err(e) = self
                .injector
                .input(&ControlInput::Key { usage, down: false }, self.surface)
            {
                error = Some(e);
            }
        }
        for button in std::mem::take(&mut self.buttons) {
            if let Err(e) = self.injector.input(
                &ControlInput::Button {
                    button,
                    down: false,
                },
                self.surface,
            ) {
                error = Some(e);
            }
        }
        self.grant = None;
        self.last = None;
        error.map_or(Ok(()), Err)
    }
}
impl<I: Injector> Drop for Receiver<I> {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sticktalk_proto::Id;
    #[test]
    fn banner_hook_gate_closes_before_queue_processing_but_still_releases_held_input() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let active = Arc::new(AtomicBool::new(false));
        let mut gate = Gated {
            inner: DryRun::default(),
            active: active.clone(),
        };
        let surface = Surface {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let down = ControlInput::Key {
            usage: 4,
            down: true,
        };
        assert!(gate.input(&down, surface).is_err());
        assert!(gate.inner.events.is_empty());
        active.store(true, Ordering::Release); // simulated banner-ready acknowledgement
        gate.input(&down, surface).unwrap();
        active.store(false, Ordering::Release); // physical mouse/hotkey/banner Stop
        assert!(gate
            .input(&ControlInput::Move { x: 0, y: 0 }, surface)
            .is_err());
        gate.input(
            &ControlInput::Key {
                usage: 4,
                down: false,
            },
            surface,
        )
        .unwrap();
        assert_eq!(gate.inner.events.len(), 2);
    }
    #[test]
    fn physical_pixel_bounds_matrix_including_mixed_dpi_and_negative_origins() {
        let desktop = Surface {
            x: -3840,
            y: -2160,
            width: 7680,
            height: 4320,
        };
        // Input values are already physical pixels, as returned by PMv2 bounds APIs.
        for surface in [
            Surface {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            Surface {
                x: -3840,
                y: 0,
                width: 3840,
                height: 2160,
            },
            Surface {
                x: -1920,
                y: -2160,
                width: 2560,
                height: 1440,
            },
            Surface {
                x: 200,
                y: 150,
                width: 1200,
                height: 900,
            }, // 800x600 logical at 150%
            Surface {
                x: -1800,
                y: -1200,
                width: 1600,
                height: 1200,
            }, // 200%
            Surface {
                x: 3839,
                y: 2159,
                width: 1,
                height: 1,
            },
        ] {
            for x in [0, 1, 32767, 32768, 65534, 65535] {
                for y in [0, 32768, 65535] {
                    let (px, py) = surface.point(x, y).unwrap();
                    assert!(surface.contains(px, py));
                    let (ax, ay) = surface.absolute(desktop, x, y).unwrap();
                    assert!((0..=65535).contains(&ax) && (0..=65535).contains(&ay));
                    let actual_x =
                        desktop.x + (i64::from(ax) * i64::from(desktop.width) / 65536) as i32;
                    let actual_y =
                        desktop.y + (i64::from(ay) * i64::from(desktop.height) / 65536) as i32;
                    assert_eq!((actual_x, actual_y), (px, py));
                }
            }
        }
        assert!(Surface {
            x: i32::MAX,
            y: 0,
            width: 2,
            height: 2
        }
        .point(65535, 0)
        .is_err());
        assert!(Surface {
            x: 3840,
            y: 0,
            width: 2,
            height: 2
        }
        .absolute(desktop, 0, 0)
        .is_err());
        assert!(Surface {
            x: 0,
            y: 0,
            width: 0,
            height: 2
        }
        .point(0, 0)
        .is_err());
    }
    #[test]
    fn grant_revoke_dry_run_releases_keys_and_replay_fails_closed() {
        let a = VoiceSession {
            user_id: Id(1),
            connection_id: Id(2),
        };
        let b = VoiceSession {
            user_id: Id(3),
            connection_id: Id(4),
        };
        let mut c = Consent::default();
        c.request(b, 0, 30_000);
        c.decide(1, true);
        c.granted(a, b, 5);
        let surface = Surface {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(surface.point(65535, 65535).unwrap(), (-1, 1079));
        let mut r = Receiver::new(DryRun::default(), surface);
        let d = ControlData::Input {
            sharer: a,
            grant: 5,
            sequence: 1,
            input: ControlInput::Key {
                usage: 4,
                down: true,
            },
        };
        r.receive(&c, a, d.clone(), 2).unwrap();
        assert!(r.receive(&c, a, d, 3).is_err());
        assert_eq!(
            r.injector.events.last(),
            Some(&ControlInput::Key {
                usage: 4,
                down: false
            })
        );
        c.clear();
        assert!(r
            .receive(
                &c,
                a,
                ControlData::Input {
                    sharer: a,
                    grant: 5,
                    sequence: 2,
                    input: ControlInput::Move { x: 0, y: 0 }
                },
                4
            )
            .is_err());
    }
}

#[cfg(windows)]
#[path = "control_native.rs"]
pub(super) mod native;
