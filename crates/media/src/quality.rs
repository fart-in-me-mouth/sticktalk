use crate::{Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub fn validate(self) -> Result<Self> {
        if self.width < 2 || self.height < 2 || self.width > 16_384 || self.height > 16_384 {
            return Err(Error::Invalid(format!("unsupported dimensions: {self:?}")));
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FrameRate {
    Fps15,
    Fps30,
    #[default]
    Fps60,
}

impl FrameRate {
    pub const fn get(self) -> u32 {
        match self {
            Self::Fps15 => 15,
            Self::Fps30 => 30,
            Self::Fps60 => 60,
        }
    }
}

impl TryFrom<u32> for FrameRate {
    type Error = Error;
    fn try_from(value: u32) -> Result<Self> {
        match value {
            15 => Ok(Self::Fps15),
            30 => Ok(Self::Fps30),
            60 => Ok(Self::Fps60),
            _ => Err(Error::Invalid("FPS must be 15, 30 or 60".into())),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Resolution {
    P480,
    P720,
    #[default]
    P1080,
    P1440,
    Source,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Quality {
    pub fps: FrameRate,
    pub resolution: Resolution,
}

impl Quality {
    /// Preserves source aspect ratio, rounding to chroma-compatible even sizes.
    /// Source is never resized: odd native sizes are rejected rather than
    /// silently scaled/cropped, because NV12 requires even dimensions.
    pub fn output_size(self, source: Size) -> Result<Size> {
        source.validate()?;
        if self.resolution == Resolution::Source {
            if !source.width.is_multiple_of(2) || !source.height.is_multiple_of(2) {
                return Err(Error::Invalid("Source NV12 requires even native dimensions; choose a scaled quality for odd-sized windows".into()));
            }
            return Ok(source);
        }
        let height = match self.resolution {
            Resolution::P480 => 480,
            Resolution::P720 => 720,
            Resolution::P1080 => 1080,
            Resolution::P1440 => 1440,
            Resolution::Source => unreachable!(),
        };
        let width = ((u64::from(source.width) * u64::from(height) / u64::from(source.height))
            as u32)
            .max(2)
            & !1;
        Size { width, height }.validate()
    }

    pub fn recommended_bitrate(self, source: Size) -> Result<u32> {
        let size = self.output_size(source)?;
        let base =
            u64::from(size.width) * u64::from(size.height) * u64::from(self.fps.get()) * 65 / 1000;
        Ok(base.clamp(500_000, 40_000_000) as u32)
    }

    pub(crate) fn software_output_size(self, source: Size) -> Result<Size> {
        let mut size = self.software_cap().output_size(source)?;
        if size.width > 1280 {
            size.height =
                ((u64::from(size.height) * 1280 / u64::from(size.width)) as u32).max(2) & !1;
            size.width = 1280;
        }
        Ok(size)
    }

    pub(crate) fn software_cap(self) -> Self {
        Self {
            fps: if self.fps == FrameRate::Fps60 {
                FrameRate::Fps30
            } else {
                self.fps
            },
            resolution: match self.resolution {
                Resolution::P480 => Resolution::P480,
                _ => Resolution::P720,
            },
        }
    }
}

pub(crate) fn validate_bitrate(bitrate: u32) -> Result<()> {
    if !(100_000..=100_000_000).contains(&bitrate) {
        return Err(Error::Invalid(
            "bitrate must be 100 kbit/s .. 100 Mbit/s".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fifteen_quality_combinations() {
        let native = Size {
            width: 3840,
            height: 2160,
        };
        for fps in [FrameRate::Fps15, FrameRate::Fps30, FrameRate::Fps60] {
            for resolution in [
                Resolution::P480,
                Resolution::P720,
                Resolution::P1080,
                Resolution::P1440,
                Resolution::Source,
            ] {
                let q = Quality { fps, resolution };
                let out = q.output_size(native).unwrap();
                assert_eq!(out.width % 2, 0);
                assert_eq!(out.height % 2, 0);
                assert!((out.width as f64 / out.height as f64 - 16.0 / 9.0).abs() < 0.005);
                assert!(q.recommended_bitrate(native).unwrap() > 0);
            }
        }
    }

    #[test]
    fn native_and_portrait_and_invalid() {
        let native = Size {
            width: 1080,
            height: 1920,
        };
        assert_eq!(
            Quality {
                fps: FrameRate::Fps60,
                resolution: Resolution::Source
            }
            .output_size(native)
            .unwrap(),
            native
        );
        assert_eq!(Quality::default().output_size(native).unwrap().width, 606);
        assert!(Quality {
            fps: FrameRate::Fps60,
            resolution: Resolution::Source
        }
        .output_size(Size {
            width: 801,
            height: 601
        })
        .is_err());
        assert_eq!(
            Quality::default()
                .software_output_size(Size {
                    width: 3840,
                    height: 1080
                })
                .unwrap(),
            Size {
                width: 1280,
                height: 360
            }
        );
        assert!(FrameRate::try_from(24).is_err());
        assert!(validate_bitrate(0).is_err());
        assert_eq!(
            Quality::default().software_cap(),
            Quality {
                fps: FrameRate::Fps30,
                resolution: Resolution::P720
            }
        );
    }
}
