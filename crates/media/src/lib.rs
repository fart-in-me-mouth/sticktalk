//! Windows-first GPU capture → H.264 → GPU decode, independent of UI/networking.
//!
//! [`ShareSession`] owns a capture worker. Encoded output is bounded; consumers
//! must drain it promptly. Its COM/D3D work stays off the calling thread. Create
//! and use the decoder on a separate MTA media thread, not an STA UI thread.
//! Hardware is required by default; software fallback requires explicit opt-in.

mod annexb;
mod quality;
mod selection;
pub mod voice;
pub use quality::{FrameRate, Quality, Resolution, Size};
pub use selection::{encoder_order, EncoderVendor};

use std::time::Duration;

#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid configuration: {0}")]
    Invalid(String),
    #[error("media pipeline: {0}")]
    Platform(String),
    #[error("hardware encoder unavailable: {0}")]
    NoHardware(String),
    #[error("media operation timed out: {0}")]
    Timeout(&'static str),
    #[error("session has stopped")]
    Stopped,
    #[error("this pipeline requires Windows 10 2004 or newer")]
    Unsupported,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    Monitor,
    Window,
}

/// A snapshot of an OS source. Handles can become invalid after enumeration.
#[derive(Clone, Debug)]
pub struct Source {
    pub kind: SourceKind,
    pub id: isize,
    pub name: String,
    pub size: Size,
    pub primary: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncoderKind {
    Hardware {
        name: String,
    },
    /// Explicit opt-in only. Effective quality is capped to 720p30.
    Software,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

#[derive(Clone, Debug)]
pub struct EncoderAvailability {
    pub codec: Codec,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RateControl {
    #[default]
    Cbr,
    Vbr,
}

/// No implicit CPU encoder: fallback must be explicitly permitted by the caller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EncoderSelection {
    #[default]
    HardwareOnly,
    AllowSoftwareFallback,
    /// Diagnostics or an explicit user choice. Still capped to 720p30.
    SoftwareOnly,
}

#[derive(Clone, Debug)]
pub struct ShareOptions {
    pub quality: Quality,
    pub bitrate: u32,
    pub cursor: bool,
    pub encoder_selection: EncoderSelection,
    pub rate_control: RateControl,
    pub gop_frames: u32,
}

impl Default for ShareOptions {
    fn default() -> Self {
        Self {
            quality: Quality::default(),
            bitrate: 8_000_000,
            cursor: true,
            encoder_selection: EncoderSelection::HardwareOnly,
            rate_control: RateControl::Cbr,
            gop_frames: 120,
        }
    }
}

/// One Annex-B access unit, with SPS/PPS included on IDRs. All media times are
/// in 100 ns units (Media Foundation time), relative to this session's start.
#[derive(Clone, Debug)]
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub timestamp: i64,
    pub duration: i64,
    pub keyframe: bool,
    pub size: Size,
    /// Incremented when encoder configuration changes. Decoder renegotiates on
    /// the first keyframe of a new generation, without restarting capture.
    pub generation: u64,
    pub encode_latency: Duration,
}

#[derive(Clone, Debug, Default)]
pub struct SessionStats {
    pub captured: u64,
    pub encoded: u64,
    pub dropped: u64,
    pub repeated: u64,
    pub bytes: u64,
    pub reconfigurations: u64,
}

#[cfg(windows)]
mod platform;
#[cfg(windows)]
pub use platform::{
    audio_devices, camera_devices, hardware_encoders, list_sources, process_usage,
    source_gpu_thumbnail, source_process_id, source_thumbnail, AudioDevice, AudioDeviceEvent,
    AudioDeviceWatcher, AudioDirection, AudioSessionEvent, CameraDevice, CameraFrameEncoder,
    CameraOptions, CameraSession, DecodedFrame, Decoder, DisplayBridge, DisplayFrame,
    GpuThumbnailer, LoopbackSession, LoopbackSource, MicLevel, MicTestSession, ProcessUsage,
    ShareSession, SourceThumbnail, VoiceDevices, VoiceSession,
};
