//! Vendor-neutral MFT selection policy; hardware activation still proves support.
use crate::{Codec, EncoderAvailability, EncoderSelection, Result};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncoderVendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}
impl EncoderVendor {
    pub fn from_name(name: &str) -> Self {
        let name = name.to_ascii_lowercase();
        if name.contains("nvidia") || name.contains("nvenc") {
            Self::Nvidia
        } else if name.contains("amd") || name.contains("amf") || name.contains("radeon") {
            Self::Amd
        } else if name.contains("intel") || name.contains("qsv") || name.contains("quick sync") {
            Self::Intel
        } else {
            Self::Other
        }
    }
}
/// Try same-vendor H.264 MFTs first (AMF/QSV/NVENC), then all other H.264 MFTs.
/// Never filters unfamiliar hardware names out. The DXGI manager/SetInputType
/// activation checks, not a friendly name, decide whether a candidate works.
pub fn encoder_order(available: &[EncoderAvailability], adapter: &str) -> Vec<usize> {
    let vendor = EncoderVendor::from_name(adapter);
    let mut order: Vec<_> = available
        .iter()
        .enumerate()
        .filter(|(_, e)| e.codec == Codec::H264)
        .map(|(i, _)| i)
        .collect();
    order.sort_by_key(|&i| {
        u8::from(
            vendor == EncoderVendor::Other
                || EncoderVendor::from_name(&available[i].name) != vendor,
        )
    });
    order
}
pub(crate) fn select_with_fallback<T>(
    selection: EncoderSelection,
    hardware: impl FnOnce() -> Result<T>,
    software: impl FnOnce(Option<crate::Error>) -> Result<T>,
) -> Result<T> {
    match selection {
        EncoderSelection::SoftwareOnly => software(None),
        EncoderSelection::HardwareOnly => hardware(),
        EncoderSelection::AllowSoftwareFallback => match hardware() {
            Ok(v) => Ok(v),
            Err(e) => software(Some(e)),
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    fn fake(name: &str, codec: Codec) -> EncoderAvailability {
        EncoderAvailability {
            name: name.into(),
            codec,
        }
    }
    #[test]
    fn amf_qsv_nvenc_unknown_and_codec_filter() {
        let entries = vec![
            fake("NVIDIA H.264", Codec::H264),
            fake("AMD AMF H.264", Codec::H264),
            fake("Intel Quick Sync H.264", Codec::H264),
            fake("Other hardware", Codec::H264),
            fake("Intel HEVC", Codec::Hevc),
        ];
        assert_eq!(encoder_order(&entries, "AMD Radeon"), [1, 0, 2, 3]);
        assert_eq!(encoder_order(&entries, "Intel UHD"), [2, 0, 1, 3]);
        assert_eq!(encoder_order(&entries, "NVIDIA RTX"), [0, 1, 2, 3]);
        assert_eq!(encoder_order(&entries, "Unknown"), [0, 1, 2, 3]);
        assert_eq!(EncoderVendor::from_name("QSV"), EncoderVendor::Intel);
    }
    #[test]
    fn fallback_is_explicit_and_only_after_hardware_failure() {
        let unavailable = || Err(Error::NoHardware("fake enumeration exhausted".into()));
        assert!(
            select_with_fallback(EncoderSelection::HardwareOnly, unavailable, |_| Ok(9)).is_err()
        );
        assert_eq!(
            select_with_fallback(EncoderSelection::AllowSoftwareFallback, unavailable, |e| {
                assert!(e.is_some());
                Ok(9)
            })
            .unwrap(),
            9
        );
        assert_eq!(
            select_with_fallback(
                EncoderSelection::AllowSoftwareFallback,
                || Ok(1),
                |_| panic!("must not start software")
            )
            .unwrap(),
            1
        );
        assert_eq!(
            select_with_fallback(
                EncoderSelection::SoftwareOnly,
                || panic!("must not enumerate hardware"),
                |e| {
                    assert!(e.is_none());
                    Ok(3)
                }
            )
            .unwrap(),
            3
        );
    }
    #[test]
    fn fake_enumerations_try_other_hardware_after_a_vendor_rejects_format() {
        let entries = vec![
            fake("AMF", Codec::H264),
            fake("QSV", Codec::H264),
            fake("NVENC", Codec::H264),
        ];
        let mut attempts = Vec::new();
        let picked = encoder_order(&entries, "AMD").into_iter().find(|&i| {
            attempts.push(i);
            i == 1
        });
        assert_eq!(picked, Some(1));
        assert_eq!(attempts, [0, 1]);
        assert!(encoder_order(&[], "Intel").is_empty());
    }
}
