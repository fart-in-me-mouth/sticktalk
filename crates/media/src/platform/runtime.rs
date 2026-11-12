use crate::{Error, Result};
use std::{marker::PhantomData, rc::Rc, sync::OnceLock};
use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::Win32::{Media::MediaFoundation::*, System::WinRT::*};

impl From<windows::core::Error> for Error {
    fn from(value: windows::core::Error) -> Self {
        Self::Platform(format!("{} ({:#010x})", value.message(), value.code().0))
    }
}

/// Apartment-affine: COM is uninitialized on exactly the thread that entered it.
pub(super) struct Runtime(PhantomData<Rc<()>>);
impl Runtime {
    pub fn new() -> Result<Self> {
        // windows-rs caches agile WinRT factory pointers process-wide. The last
        // apartment must not disappear between sessions, invalidating those
        // factories. Match windows-core's own implicit-MTA initialization: hold
        // ONE process-lifetime MTA usage cookie, not a capture/codec/device.
        // Per-thread RoInitialize/RoUninitialize and MF references stay balanced.
        static MTA: OnceLock<std::result::Result<usize, String>> = OnceLock::new();
        if let Err(error) = MTA.get_or_init(|| unsafe {
            CoIncrementMTAUsage()
                .map(|cookie| cookie.0 as usize)
                .map_err(|e| e.to_string())
        }) {
            return Err(Error::Platform(error.clone()));
        }
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                RoUninitialize();
                return Err(e.into());
            }
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            RoUninitialize();
        }
    }
}

pub(super) fn required<T>(value: Option<T>, label: &str) -> Result<T> {
    value.ok_or_else(|| Error::Platform(format!("{label} returned null")))
}
