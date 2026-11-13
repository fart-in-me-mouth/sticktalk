use super::{capture::Capture, gpu::Gpu, pattern::Pattern};
use crate::{Result, Size, Source};
use windows::{
    Graphics::Capture::Direct3D11CaptureFrame, Win32::Graphics::Direct3D11::ID3D11Texture2D,
};

/// Only acquisition differs. Conversion, pacing, controls, MFT, queues and
/// transport are identical for real WGC and the deterministic GPU workload.
pub(super) enum Input {
    Desktop(Box<Capture>),
    Pattern(Box<Pattern>),
}
pub(super) type Frame = (Option<Direct3D11CaptureFrame>, ID3D11Texture2D, u64);
impl Input {
    pub fn new(gpu: &Gpu, source: &Source, cursor: bool, synthetic: bool) -> Result<Self> {
        if synthetic {
            Ok(Self::Pattern(Box::new(Pattern::new(gpu, source.size)?)))
        } else {
            Ok(Self::Desktop(Box::new(Capture::new(
                &gpu.device,
                source,
                cursor,
            )?)))
        }
    }
    pub fn size(&self) -> Size {
        match self {
            Self::Desktop(c) => c.size,
            Self::Pattern(p) => p.size,
        }
    }
    pub fn next(&mut self) -> Result<Option<Frame>> {
        match self {
            Self::Desktop(c) => {
                let _ = c.signals.try_recv();
                Ok(c.next()?
                    .map(|(frame, texture, count)| (Some(frame), texture, count)))
            }
            Self::Pattern(p) => Ok(Some((None, p.next(), 1))),
        }
    }
    pub fn set_cursor(&self, enabled: bool) -> Result<()> {
        match self {
            Self::Desktop(c) => c.set_cursor(enabled),
            Self::Pattern(_) => Ok(()),
        }
    }
}
