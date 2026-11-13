//! Explicit opt-in session attenuation. Never touches master/device volume.
//! Changes are restored on silence, device change, disable and Drop; user-made
//! volume changes while ducked are not overwritten by restoration.
use crate::{Error, Result};
use windows::{
    core::{Interface, HSTRING},
    Win32::{Media::Audio::*, System::Com::*},
};
#[derive(Default)]
pub struct AppAttenuation {
    saved: Vec<(ISimpleAudioVolume, f32, f32)>,
    state: Option<(u16, Option<String>)>,
}
impl AppAttenuation {
    pub fn set(&mut self, speaking: bool, percent: u16, output: Option<&str>) -> Result<()> {
        if percent > 100 {
            return Err(Error::Invalid("attenuation must be 0..100%".into()));
        }
        let next = (speaking && percent > 0).then(|| (percent, output.map(str::to_owned)));
        if self.state == next {
            return Ok(());
        }
        self.restore();
        self.state = next;
        if self.state.is_none() {
            return Ok(());
        }
        // COM interfaces below are owned by this media actor; the actor's
        // apartment must remain initialized until this guard is dropped.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = if let Some(id) = output {
                enumerator.GetDevice(&HSTRING::from(id))?
            } else {
                enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?
            };
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
            let sessions = manager.GetSessionEnumerator()?;
            for i in 0..sessions.GetCount()?.min(256) {
                let session = sessions.GetSession(i)?;
                let control: IAudioSessionControl2 = session.cast()?;
                if control.GetProcessId()? == std::process::id() {
                    continue;
                }
                let volume: ISimpleAudioVolume = session.cast()?;
                let original = volume.GetMasterVolume()?;
                let applied = original * (1. - f32::from(percent) / 100.);
                volume.SetMasterVolume(applied, std::ptr::null())?;
                self.saved.push((volume, original, applied));
            }
        }
        Ok(())
    }
    fn restore(&mut self) {
        for (volume, original, applied) in self.saved.drain(..) {
            unsafe {
                if volume
                    .GetMasterVolume()
                    .is_ok_and(|current| (current - applied).abs() < 0.001)
                {
                    let _ = volume.SetMasterVolume(original, std::ptr::null());
                }
            }
        }
    }
}
impl Drop for AppAttenuation {
    fn drop(&mut self) {
        self.restore();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_attenuation_never_enumerates_or_changes_sessions() {
        let mut guard = AppAttenuation::default();
        guard.set(true, 0, None).unwrap();
        guard.set(false, 100, None).unwrap();
        assert!(guard.saved.is_empty());
        assert!(guard.set(true, 101, None).is_err());
    }
}
