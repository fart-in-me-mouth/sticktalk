use super::{
    encoder::{manager, media_type, output},
    gpu::Gpu,
    runtime::Runtime,
};
use crate::{EncodedFrame, Error, Result, Size};
use windows::{
    core::{Interface, GUID},
    Win32::{
        Graphics::{Direct3D11::*, Dxgi::Common::*},
        Media::MediaFoundation::*,
        System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
    },
};

/// Holds the decoder sample lease. Keep it alive while the renderer is using the
/// texture; releasing only the texture is insufficient for MF's surface pool.
/// Do not accumulate frames: the decoder pool is finite.
pub struct DecodedFrame {
    pub texture: ID3D11Texture2D,
    pub subresource: u32,
    pub size: Size,
    pub timestamp: i64,
    _sample: IMFSample,
}

pub struct Decoder {
    transform: Option<IMFTransform>,
    manager: Option<IMFDXGIDeviceManager>,
    generation: Option<u64>,
    size: Option<Size>,
    gpu: Gpu,
    // Must outlive all Media Foundation and D3D objects above it.
    _runtime: Runtime,
}
impl Decoder {
    /// Use and drop on the creating MTA thread. Does not silently use a software
    /// output path: every decoded buffer must expose IMFDXGIBuffer.
    pub fn new() -> Result<Self> {
        let runtime = Runtime::new()?;
        Ok(Self {
            transform: None,
            manager: None,
            generation: None,
            size: None,
            gpu: Gpu::new()?,
            _runtime: runtime,
        })
    }
    pub fn device(&self) -> &ID3D11Device {
        &self.gpu.device
    }
    pub fn gpu_name(&self) -> &str {
        &self.gpu.name
    }
    fn configure(&mut self, frame: &EncodedFrame) -> Result<()> {
        unsafe {
            if let Some(old) = self.transform.take() {
                let _ = old.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
                let _ = old.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            }
            // Microsoft H264 Video Decoder MFT, wmcodecdsp.h. It is a synchronous
            // MFT with hardware DXVA internally, NOT an async hardware encoder.
            const H264_DECODER: GUID = GUID::from_u128(0x62ce7e72_4c71_4d20_b15d_452831a87d9d);
            let transform: IMFTransform =
                CoCreateInstance(&H264_DECODER, None, CLSCTX_INPROC_SERVER)?;
            let attrs = transform.GetAttributes()?;
            if attrs.GetUINT32(&MF_SA_D3D11_AWARE).unwrap_or(0) == 0 {
                return Err(Error::Platform("H.264 decoder is not D3D11-aware".into()));
            }
            attrs.SetUINT32(&MF_LOW_LATENCY, 1)?;
            attrs.SetUINT32(&CODECAPI_AVDecVideoAcceleration_H264, 1)?;
            let video: ID3D11VideoDevice = self.gpu.device.cast()?;
            let desc = D3D11_VIDEO_DECODER_DESC {
                Guid: D3D11_DECODER_PROFILE_H264_VLD_NOFGT,
                SampleWidth: frame.size.width,
                SampleHeight: frame.size.height,
                OutputFormat: DXGI_FORMAT_NV12,
            };
            if video.GetVideoDecoderConfigCount(&desc)? == 0 {
                return Err(Error::Platform(
                    "no D3D11VA H.264/NV12 hardware decoder configuration for this resolution"
                        .into(),
                ));
            }
            let manager = manager(&self.gpu)?;
            transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)?;
            let fps = (10_000_000 / frame.duration.max(1)) as u32;
            transform.SetInputType(0, &media_type(MFVideoFormat_H264, frame.size, fps)?, 0)?;
            // Ask for NV12 explicitly; stream-change will refine padded size.
            transform.SetOutputType(0, &media_type(MFVideoFormat_NV12, frame.size, fps)?, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            self.transform = Some(transform);
            self.manager = Some(manager);
            self.generation = Some(frame.generation);
            self.size = Some(frame.size);
        }
        Ok(())
    }
    pub fn decode(&mut self, frame: EncodedFrame) -> Result<Vec<DecodedFrame>> {
        if frame.data.len() > 32 * 1024 * 1024 || frame.data.is_empty() || frame.duration <= 0 {
            return Err(Error::Invalid("invalid encoded access unit".into()));
        }
        frame.size.validate()?;
        if self.generation != Some(frame.generation) || self.size != Some(frame.size) {
            if !frame.keyframe {
                return Err(Error::Invalid(
                    "decoder configuration must begin with a keyframe".into(),
                ));
            }
            self.configure(&frame)?;
        }
        unsafe {
            let transform = self.transform.as_ref().unwrap();
            let buffer = MFCreateMemoryBuffer(frame.data.len() as u32)?;
            let mut ptr = std::ptr::null_mut();
            buffer.Lock(&mut ptr, None, None)?;
            std::ptr::copy_nonoverlapping(frame.data.as_ptr(), ptr, frame.data.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(frame.data.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(frame.timestamp)?;
            sample.SetSampleDuration(frame.duration)?;
            let mut frames = Vec::new();
            match transform.ProcessInput(0, &sample, 0) {
                Ok(()) => (),
                Err(e) if e.code() == MF_E_NOTACCEPTING => {
                    self.receive(&mut frames)?;
                    self.transform
                        .as_ref()
                        .unwrap()
                        .ProcessInput(0, &sample, 0)?;
                }
                Err(e) => return Err(e.into()),
            }
            self.receive(&mut frames)?;
            Ok(frames)
        }
    }
    fn receive(&self, frames: &mut Vec<DecodedFrame>) -> Result<()> {
        let transform = self.transform.as_ref().unwrap();
        // Bound retries: bad/hostile transforms cannot spin on stream-change.
        for _ in 0..32 {
            unsafe {
                match output(transform) {
                    Ok(sample) => {
                        let buffer: IMFDXGIBuffer =
                            sample.GetBufferByIndex(0)?.cast().map_err(|_| {
                                Error::Platform(
                                    "decoder returned CPU memory, refusing software decode path"
                                        .into(),
                                )
                            })?;
                        let mut raw = std::ptr::null_mut();
                        buffer.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
                        let texture = ID3D11Texture2D::from_raw(raw);
                        let mut desc = D3D11_TEXTURE2D_DESC::default();
                        texture.GetDesc(&mut desc);
                        if desc.BindFlags & D3D11_BIND_DECODER.0 as u32 == 0 {
                            return Err(Error::Platform("decoder output is not a D3D11 decoder-bound surface; refusing a CPU-upload path".into()));
                        }
                        frames.push(DecodedFrame {
                            texture,
                            subresource: buffer.GetSubresourceIndex()?,
                            size: self.size.unwrap(),
                            timestamp: sample.GetSampleTime()?,
                            _sample: sample,
                        });
                    }
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                    Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                        let mut selected = false;
                        for i in 0..32 {
                            let ty = match transform.GetOutputAvailableType(0, i) {
                                Ok(ty) => ty,
                                Err(_) => break,
                            };
                            if ty.GetGUID(&MF_MT_SUBTYPE)? == MFVideoFormat_NV12 {
                                transform.SetOutputType(0, &ty, 0)?;
                                selected = true;
                                break;
                            }
                        }
                        if !selected {
                            return Err(Error::Platform(
                                "decoder offers no NV12 output type".into(),
                            ));
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Err(Error::Platform(
            "excessive decoder outputs/stream changes".into(),
        ))
    }

    /// DIAGNOSTIC ONLY: synchronous GPU readback + CPU NV12→RGBA. Never call in
    /// production playback. Render `DecodedFrame::texture` / `subresource`
    /// directly, using this decoder's device (or explicit shared-handle interop).
    pub fn readback_rgba_for_test(&self, frame: &DecodedFrame) -> Result<Vec<u8>> {
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            frame.texture.GetDesc(&mut desc);
            frame.size.validate()?;
            if frame.size.width > desc.Width || frame.size.height > desc.Height {
                return Err(Error::Invalid(
                    "readback visible size exceeds texture bounds".into(),
                ));
            }
            if desc.Format != DXGI_FORMAT_NV12 {
                return Err(Error::Platform("test readback expects NV12".into()));
            }
            desc.ArraySize = 1;
            desc.MipLevels = 1;
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.MiscFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            self.gpu
                .device
                .CreateTexture2D(&desc, None, Some(&mut staging))?;
            let staging = super::runtime::required(staging, "test readback staging")?;
            self.gpu.context.CopySubresourceRegion(
                &staging,
                0,
                0,
                0,
                0,
                &frame.texture,
                frame.subresource,
                None,
            );
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.gpu
                .context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let stride = mapped.RowPitch as usize;
            let bytes = std::slice::from_raw_parts(
                mapped.pData as *const u8,
                stride * desc.Height as usize * 3 / 2,
            );
            let w = frame.size.width as usize;
            let h = frame.size.height as usize;
            let mut rgba = vec![0u8; w * h * 4];
            for y in 0..h {
                for x in 0..w {
                    let luma = f32::from(bytes[y * stride + x]) - 16.0;
                    let pos = desc.Height as usize * stride + (y / 2) * stride + (x & !1);
                    let u = f32::from(bytes[pos]) - 128.0;
                    let v = f32::from(bytes[pos + 1]) - 128.0;
                    let dst = (y * w + x) * 4;
                    rgba[dst] = (1.164 * luma + 1.793 * v).clamp(0.0, 255.0) as u8;
                    rgba[dst + 1] = (1.164 * luma - 0.213 * u - 0.533 * v).clamp(0.0, 255.0) as u8;
                    rgba[dst + 2] = (1.164 * luma + 2.112 * u).clamp(0.0, 255.0) as u8;
                    rgba[dst + 3] = 255;
                }
            }
            self.gpu.context.Unmap(&staging, 0);
            Ok(rgba)
        }
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        if let Some(transform) = &self.transform {
            unsafe {
                let _ = transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
                let _ = transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            }
        }
    }
}
