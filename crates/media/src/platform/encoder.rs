use super::{
    gpu::Gpu,
    runtime::{required, Runtime},
};
use crate::{
    annexb, Codec, EncoderAvailability, EncoderKind, Error, RateControl, Result, ShareOptions, Size,
};
use std::{
    mem::ManuallyDrop,
    time::{Duration, Instant},
};
use windows::{
    core::{Interface, GUID, PWSTR},
    Win32::{
        Graphics::Direct3D11::*,
        Media::MediaFoundation::*,
        System::{Com::CoTaskMemFree, Variant::VARIANT},
    },
};

pub fn hardware_encoders() -> Result<Vec<EncoderAvailability>> {
    let _runtime = Runtime::new()?;
    let mut found = Vec::new();
    for (codec, subtype) in [
        (Codec::H264, MFVideoFormat_H264),
        (Codec::Hevc, MFVideoFormat_HEVC),
        (Codec::Av1, MFVideoFormat_AV1),
    ] {
        for activate in enumerate(subtype)? {
            found.push(EncoderAvailability {
                codec,
                name: name(&activate)?,
            });
        }
    }
    Ok(found)
}
fn enumerate(subtype: GUID) -> Result<Vec<IMFActivate>> {
    unsafe {
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: subtype,
        };
        let mut ptr = std::ptr::null_mut();
        let mut count = 0;
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
            None,
            Some(&output),
            &mut ptr,
            &mut count,
        )?;
        let result = if count == 0 {
            Vec::new()
        } else {
            // Transfer COM references before freeing the CoTaskMem array.
            std::slice::from_raw_parts_mut(ptr, count as usize)
                .iter_mut()
                .filter_map(Option::take)
                .collect()
        };
        CoTaskMemFree(Some(ptr as *const _));
        Ok(result)
    }
}
fn name(activate: &IMFActivate) -> Result<String> {
    unsafe {
        let mut ptr = PWSTR::null();
        let mut len = 0;
        activate.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut ptr, &mut len)?;
        let name = String::from_utf16_lossy(std::slice::from_raw_parts(ptr.0, len as usize));
        CoTaskMemFree(Some(ptr.0 as *const _));
        Ok(name)
    }
}
pub(super) fn manager(gpu: &Gpu) -> Result<IMFDXGIDeviceManager> {
    unsafe {
        let mut token = 0;
        let mut manager = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = required(manager, "MFCreateDXGIDeviceManager")?;
        manager.ResetDevice(&gpu.device, token)?;
        Ok(manager)
    }
}
pub(super) fn media_type(subtype: GUID, size: Size, fps: u32) -> Result<IMFMediaType> {
    unsafe {
        let ty = MFCreateMediaType()?;
        ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        ty.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
        ty.SetUINT64(
            &MF_MT_FRAME_SIZE,
            (u64::from(size.width) << 32) | u64::from(size.height),
        )?;
        ty.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(fps) << 32) | 1)?;
        ty.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        ty.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        ty.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        ty.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
        ty.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
        ty.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
        Ok(ty)
    }
}
/// Own both embedded COM pointers even on ProcessOutput errors.
pub(super) fn output(transform: &IMFTransform) -> windows::core::Result<IMFSample> {
    unsafe {
        let info = transform.GetOutputStreamInfo(0)?;
        let sample = if info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 == 0 {
            let sample = MFCreateSample()?;
            sample.AddBuffer(&MFCreateMemoryBuffer(info.cbSize.max(4096))?)?;
            Some(sample)
        } else {
            None
        };
        let mut buffer = MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: ManuallyDrop::new(sample),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        };
        let mut status = 0;
        let result = transform.ProcessOutput(0, std::slice::from_mut(&mut buffer), &mut status);
        let sample = ManuallyDrop::take(&mut buffer.pSample);
        ManuallyDrop::drop(&mut buffer.pEvents);
        result?;
        sample.ok_or_else(|| windows::core::Error::from(MF_E_TRANSFORM_NEED_MORE_INPUT))
    }
}
fn sample_bytes(sample: &IMFSample) -> Result<Vec<u8>> {
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer()?;
        let mut ptr = std::ptr::null_mut();
        let mut len = 0;
        buffer.Lock(&mut ptr, None, Some(&mut len))?;
        let data = std::slice::from_raw_parts(ptr, len as usize).to_vec();
        buffer.Unlock()?;
        Ok(data)
    }
}
fn property(api: &ICodecAPI, key: &GUID, value: VARIANT, label: &str) -> Result<()> {
    unsafe {
        api.SetValue(key, &value)
            .map_err(|e| Error::Platform(format!("{label}: {e}")))?;
    }
    Ok(())
}

pub(super) struct Hardware {
    transform: IMFTransform,
    activate: IMFActivate,
    _manager: IMFDXGIDeviceManager,
    events: IMFMediaEventGenerator,
    codec_api: ICodecAPI,
    pub name: String,
    headers: Vec<u8>,
    need_input: u32,
}
impl Hardware {
    pub fn new(gpu: &Gpu, size: Size, options: &ShareOptions) -> Result<Self> {
        let mut failures = Vec::new();
        let activations = enumerate(MFVideoFormat_H264)?;
        let candidates: Vec<_> = activations
            .iter()
            .map(|a| {
                Ok(EncoderAvailability {
                    codec: Codec::H264,
                    name: name(a)?,
                })
            })
            .collect::<Result<_>>()?;
        for index in crate::encoder_order(&candidates, &gpu.name) {
            let activate = &activations[index];
            let encoder_name = candidates[index].name.clone();
            match Self::activate(gpu, size, options, activate.clone(), encoder_name.clone()) {
                Ok(encoder) => return Ok(encoder),
                Err(e) => {
                    unsafe {
                        let _ = activate.ShutdownObject();
                    }
                    failures.push(format!("{encoder_name}: {e}"));
                }
            }
        }
        Err(Error::NoHardware(if failures.is_empty() {
            "MFTEnumEx returned no hardware H.264 encoders".into()
        } else {
            failures.join("; ")
        }))
    }
    fn activate(
        gpu: &Gpu,
        size: Size,
        options: &ShareOptions,
        activate: IMFActivate,
        name: String,
    ) -> Result<Self> {
        unsafe {
            let transform: IMFTransform = activate.ActivateObject()?;
            let attrs = transform.GetAttributes()?;
            attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
            attrs.SetUINT32(&MF_LOW_LATENCY, 1)?;
            if attrs.GetUINT32(&MF_SA_D3D11_AWARE).unwrap_or(0) == 0 {
                return Err(Error::NoHardware(
                    "MFT does not accept D3D11 surfaces".into(),
                ));
            }
            let manager = manager(gpu)?;
            transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)?;
            let codec_api: ICodecAPI = transform.cast()?;
            property(
                &codec_api,
                &CODECAPI_AVLowLatencyMode,
                VARIANT::from(true),
                "low latency",
            )?;
            property(
                &codec_api,
                &CODECAPI_AVEncCommonRateControlMode,
                VARIANT::from(match options.rate_control {
                    RateControl::Cbr => eAVEncCommonRateControlMode_CBR.0 as u32,
                    RateControl::Vbr => eAVEncCommonRateControlMode_PeakConstrainedVBR.0 as u32,
                }),
                "rate control",
            )?;
            property(
                &codec_api,
                &CODECAPI_AVEncCommonMeanBitRate,
                VARIANT::from(options.bitrate),
                "bitrate",
            )?;
            if options.rate_control == RateControl::Vbr {
                property(
                    &codec_api,
                    &CODECAPI_AVEncCommonMaxBitRate,
                    VARIANT::from(options.bitrate.saturating_mul(3) / 2),
                    "peak bitrate",
                )?;
            }
            property(
                &codec_api,
                &CODECAPI_AVEncMPVGOPSize,
                VARIANT::from(options.gop_frames),
                "GOP",
            )?;
            // NVIDIA rejects this optional property. The REQUIRED low-latency
            // mode above guarantees one input / one output without reordering.
            let _ =
                codec_api.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &VARIANT::from(0u32));
            let out = media_type(MFVideoFormat_H264, size, options.quality.fps.get())?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, options.bitrate)?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
            transform
                .SetOutputType(0, &out, 0)
                .map_err(|e| Error::Platform(format!("encoder output type: {e}")))?;
            transform
                .SetInputType(
                    0,
                    &media_type(MFVideoFormat_NV12, size, options.quality.fps.get())?,
                    0,
                )
                .map_err(|e| Error::Platform(format!("encoder NV12 input type: {e}")))?;
            let events = transform.cast()?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            let headers = if let Ok(current) = transform.GetOutputCurrentType(0) {
                if let Ok(len) = current.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER) {
                    let mut bytes = vec![0; len as usize];
                    current.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut bytes, None)?;
                    annexb::normalize(&bytes).unwrap_or_default()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            Ok(Self {
                transform,
                activate,
                _manager: manager,
                events,
                codec_api,
                name,
                headers,
                need_input: 0,
            })
        }
    }
    pub fn keyframe(&self) -> Result<()> {
        property(
            &self.codec_api,
            &CODECAPI_AVEncVideoForceKeyFrame,
            VARIANT::from(1u32),
            "force IDR",
        )
    }
    fn wait_event(&mut self, wanted: u32) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            unsafe {
                match self.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
                    Ok(event) => {
                        event.GetStatus()?.ok()?;
                        let kind = event.GetType()?;
                        if kind == METransformNeedInput.0 as u32 {
                            self.need_input += 1;
                        }
                        if kind == wanted {
                            return Ok(());
                        }
                        if kind == METransformHaveOutput.0 as u32 {
                            return Err(Error::Platform(
                                "unexpected buffered output before input".into(),
                            ));
                        }
                    }
                    Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => {
                        if Instant::now() >= deadline {
                            return Err(Error::Timeout("hardware MFT event (low-latency output)"));
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    pub fn encode(
        &mut self,
        texture: &ID3D11Texture2D,
        timestamp: i64,
        duration: i64,
    ) -> Result<Vec<u8>> {
        if self.need_input == 0 {
            self.wait_event(METransformNeedInput.0 as u32)?;
        }
        unsafe {
            let sample = MFCreateSample()?;
            sample.AddBuffer(&MFCreateDXGISurfaceBuffer(
                &ID3D11Texture2D::IID,
                texture,
                0,
                false,
            )?)?;
            sample.SetSampleTime(timestamp)?;
            sample.SetSampleDuration(duration)?;
            self.transform.ProcessInput(0, &sample, 0)?;
            self.need_input -= 1;
        }
        self.wait_event(METransformHaveOutput.0 as u32)?;
        let sample = output(&self.transform)?;
        if unsafe { sample.GetSampleTime()? } != timestamp {
            return Err(Error::Platform(
                "low-latency encoder changed/reordered sample timestamps".into(),
            ));
        }
        let mut bytes = annexb::normalize(&sample_bytes(&sample)?)?;
        if annexb::contains(&bytes, 7) && annexb::contains(&bytes, 8) {
            self.headers = parameter_sets(&bytes);
        } else if annexb::contains(&bytes, 5) && !self.headers.is_empty() {
            let mut joined = self.headers.clone();
            joined.append(&mut bytes);
            bytes = joined;
        }
        Ok(bytes)
    }
}
impl Drop for Hardware {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            let _ = self.activate.ShutdownObject();
        }
    }
}
fn parameter_sets(data: &[u8]) -> Vec<u8> {
    let mut starts = Vec::new();
    for i in 0..data.len().saturating_sub(3) {
        if data[i..].starts_with(&[0, 0, 1]) {
            starts.push(i);
        }
    }
    starts.push(data.len());
    let mut result = Vec::new();
    for pair in starts.windows(2) {
        if matches!(data[pair[0] + 3] & 31, 7 | 8) {
            result.extend_from_slice(&[0, 0, 0, 1]);
            let mut end = pair[1];
            while end > pair[0] + 3 && data[end - 1] == 0 {
                end -= 1;
            }
            result.extend_from_slice(&data[pair[0] + 3..end]);
        }
    }
    result
}

/// Deliberately separate CPU readback: ONLY explicit software fallback uses it.
struct Software {
    encoder: openh264::encoder::Encoder,
    staging: ID3D11Texture2D,
    context: ID3D11DeviceContext,
    i420: I420,
    gop: u64,
    frames: u64,
}
struct I420 {
    size: Size,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}
impl openh264::formats::YUVSource for I420 {
    fn dimensions(&self) -> (usize, usize) {
        (self.size.width as usize, self.size.height as usize)
    }
    fn strides(&self) -> (usize, usize, usize) {
        let w = self.size.width as usize;
        (w, w / 2, w / 2)
    }
    fn y(&self) -> &[u8] {
        &self.y
    }
    fn u(&self) -> &[u8] {
        &self.u
    }
    fn v(&self) -> &[u8] {
        &self.v
    }
}
impl Software {
    fn new(gpu: &Gpu, size: Size, options: &ShareOptions) -> Result<Self> {
        use openh264::encoder::*;
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(options.bitrate))
            .max_frame_rate(openh264::encoder::FrameRate::from_hz(
                options.quality.fps.get() as f32,
            ))
            .profile(Profile::High)
            .vui(VuiConfig::bt709())
            .usage_type(UsageType::ScreenContentRealTime)
            .complexity(Complexity::Low)
            .num_threads(1)
            .rate_control_mode(RateControlMode::Bitrate)
            .skip_frames(true);
        let encoder = openh264::encoder::Encoder::with_api_config(
            openh264::OpenH264API::from_source(),
            config,
        )
        .map_err(|e| Error::Platform(format!("OpenH264: {e}")))?;
        let mut texture = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.width,
            Height: size.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_NV12,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        unsafe {
            gpu.device
                .CreateTexture2D(&desc, None, Some(&mut texture))?;
        }
        let pixels = (size.width * size.height) as usize;
        Ok(Self {
            encoder,
            staging: required(texture, "software staging")?,
            context: gpu.context.clone(),
            i420: I420 {
                size,
                y: vec![0; pixels],
                u: vec![0; pixels / 4],
                v: vec![0; pixels / 4],
            },
            gop: u64::from(options.gop_frames),
            frames: 0,
        })
    }
    fn encode(&mut self, texture: &ID3D11Texture2D) -> Result<Vec<u8>> {
        unsafe {
            self.context.CopyResource(&self.staging, texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let w = self.i420.size.width as usize;
            let h = self.i420.size.height as usize;
            let pitch = mapped.RowPitch as usize;
            let data = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * h * 3 / 2);
            for row in 0..h {
                self.i420.y[row * w..(row + 1) * w]
                    .copy_from_slice(&data[row * pitch..row * pitch + w]);
            }
            for row in 0..h / 2 {
                for col in 0..w / 2 {
                    let pos = (h + row) * pitch + col * 2;
                    self.i420.u[row * w / 2 + col] = data[pos];
                    self.i420.v[row * w / 2 + col] = data[pos + 1];
                }
            }
            self.context.Unmap(&self.staging, 0);
        }
        if self.frames.is_multiple_of(self.gop) {
            self.encoder.force_intra_frame();
        }
        self.frames += 1;
        Ok(self
            .encoder
            .encode(&self.i420)
            .map_err(|e| Error::Platform(format!("OpenH264: {e}")))?
            .to_vec())
    }
}
pub(super) struct Encoder {
    hardware: Option<Hardware>,
    software: Option<Software>,
    pub kind: EncoderKind,
}
impl Encoder {
    pub fn new(
        gpu: &Gpu,
        size: Size,
        options: &ShareOptions,
        force_software: bool,
    ) -> Result<Self> {
        if force_software {
            return Ok(Self {
                hardware: None,
                software: Some(Software::new(gpu, size, options)?),
                kind: EncoderKind::Software,
            });
        }
        let hardware = Hardware::new(gpu, size, options)?;
        let kind = EncoderKind::Hardware {
            name: hardware.name.clone(),
        };
        Ok(Self {
            hardware: Some(hardware),
            software: None,
            kind,
        })
    }
    /// Update rate control without changing media types, flushing reference
    /// frames, creating a transform or forcing an IDR.
    pub fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        if let Some(hw) = &self.hardware {
            property(
                &hw.codec_api,
                &CODECAPI_AVEncCommonMeanBitRate,
                VARIANT::from(bitrate),
                "dynamic bitrate",
            )?;
        } else if let Some(sw) = &mut self.software {
            let mut info = openh264_sys2::SBitrateInfo {
                iLayer: openh264_sys2::SPATIAL_LAYER_ALL,
                iBitrate: bitrate as i32,
            };
            // SAFETY: correct C option/payload pair, borrowed synchronously on
            // the encoder's owning worker; no concurrent codec access.
            let result = unsafe {
                sw.encoder.raw_api().set_option(
                    openh264_sys2::ENCODER_OPTION_BITRATE,
                    (&mut info as *mut openh264_sys2::SBitrateInfo).cast(),
                )
            };
            if result != 0 {
                return Err(Error::Platform(format!(
                    "OpenH264 dynamic bitrate: {result}"
                )));
            }
        }
        Ok(())
    }
    pub fn keyframe(&mut self) -> Result<()> {
        if let Some(hw) = &self.hardware {
            hw.keyframe()?;
        }
        if let Some(sw) = &mut self.software {
            sw.encoder.force_intra_frame();
        }
        Ok(())
    }
    pub fn encode(
        &mut self,
        texture: &ID3D11Texture2D,
        timestamp: i64,
        duration: i64,
    ) -> Result<Vec<u8>> {
        if let Some(hw) = &mut self.hardware {
            hw.encode(texture, timestamp, duration)
        } else {
            self.software.as_mut().unwrap().encode(texture)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserve_parameter_sets_for_later_idrs() {
        let bytes = [
            0, 0, 0, 1, 0x67, 100, 0x80, 0, 0, 0, 1, 0x68, 0x81, 0, 0, 0, 1, 0x65, 0x90,
        ];
        let headers = parameter_sets(&bytes);
        assert!(annexb::contains(&headers, 7));
        assert!(annexb::contains(&headers, 8));
        assert!(!annexb::contains(&headers, 5));
        assert_eq!(
            headers,
            [0, 0, 0, 1, 0x67, 100, 0x80, 0, 0, 0, 1, 0x68, 0x81]
        );
    }
}
