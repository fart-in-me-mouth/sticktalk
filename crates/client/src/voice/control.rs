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
        if sharer != own || !consent.may_inject(own, grant) {
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
