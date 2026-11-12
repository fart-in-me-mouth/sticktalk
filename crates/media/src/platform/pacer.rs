//! Session-scoped high-resolution frame pacing; no global timeBeginPeriod.
//! The timer exists only during an active share, never as an idle ticker.
use crate::{Error, Result};
use std::time::Instant;
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    System::Threading::*,
};
pub(super) struct Pacer(HANDLE);
impl Pacer {
    pub fn new() -> Result<Self> {
        // High-resolution waitable timers are available before our Win10 2004
        // minimum. Own unnamed timer: no admin rights or global timer changes.
        Ok(Self(unsafe {
            CreateWaitableTimerExW(
                None,
                None,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )?
        }))
    }
    pub fn until(&self, deadline: Instant) -> Result<()> {
        if let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let due = -((left.as_nanos() / 100).max(1).min(i64::MAX as u128) as i64);
            unsafe {
                SetWaitableTimerEx(self.0, &due, 0, None, None, None, 0)?;
                if WaitForSingleObject(self.0, INFINITE) != WAIT_OBJECT_0 {
                    return Err(Error::Platform("frame pacing wait failed".into()));
                }
            }
        }
        Ok(())
    }
}
impl Drop for Pacer {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
