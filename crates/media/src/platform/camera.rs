//! Media Foundation camera source reader → NV12 → existing H.264 encoder.
use super::{
    encoder::{manager, media_type, Encoder},
    gpu::Gpu,
    runtime::{required, Runtime},
};
use crate::{
    annexb, EncodedFrame, EncoderKind, Error, FrameRate, Quality, Resolution, Result, ShareOptions,
    Size,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    core::{implement, Interface, Ref, GUID, HRESULT, HSTRING, PWSTR},
    Win32::{
        Graphics::{Direct3D11::*, Dxgi::Common::DXGI_FORMAT_NV12},
        Media::MediaFoundation::*,
        System::Com::CoTaskMemFree,
    },
};
const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

#[derive(Clone, Debug)]
pub struct CameraDevice {
    pub id: String,
    pub name: String,
}
unsafe fn attr_string(attrs: &IMFAttributes, key: &GUID) -> Result<String> {
    let mut ptr = PWSTR::null();
    let mut len = 0;
    attrs.GetAllocatedString(key, &mut ptr, &mut len)?;
    let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr.0, len as usize));
    CoTaskMemFree(Some(ptr.0.cast()));
    Ok(s)
}
fn source_attributes() -> Result<IMFAttributes> {
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 2)?;
        let attrs = required(attrs, "camera attributes")?;
        attrs.SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )?;
        Ok(attrs)
    }
}
/// Enumerates only: never activates a camera or shows a permission prompt.
pub fn camera_devices() -> Result<Vec<CameraDevice>> {
    let _runtime = Runtime::new()?;
    unsafe {
        let attrs = source_attributes()?;
        let mut ptr = std::ptr::null_mut();
        let mut count = 0;
        MFEnumDeviceSources(&attrs, &mut ptr, &mut count)?;
        let sources: Vec<_> = if count == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts_mut(ptr, count as usize)
                .iter_mut()
                .filter_map(Option::take)
                .collect()
        };
        CoTaskMemFree(Some(ptr.cast()));
        sources
            .into_iter()
            .map(|s| {
                Ok(CameraDevice {
                    id: attr_string(&s, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK)?,
                    name: attr_string(&s, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)?,
                })
            })
            .collect()
    }
}
#[derive(Clone, Debug)]
pub struct CameraOptions {
    pub size: Size,
    pub fps: FrameRate,
    pub bitrate: u32,
}
impl Default for CameraOptions {
    fn default() -> Self {
        Self {
            size: Size {
                width: 1280,
                height: 720,
            },
            fps: FrameRate::Fps30,
            bitrate: 2_000_000,
        }
    }
}
impl CameraOptions {
    fn validate(&self) -> Result<()> {
        self.size.validate()?;
        crate::quality::validate_bitrate(self.bitrate)?;
        if !self.size.width.is_multiple_of(2) || !self.size.height.is_multiple_of(2) {
            return Err(Error::Invalid("camera NV12 size must be even".into()));
        }
        Ok(())
    }
}
enum Message {
    Sample(Result<(Option<IMFSample>, i64, u32)>),
    Stop,
}
// SAFETY: MF source-reader samples are handed from MF's MTA callback to our
// MTA worker. No sample ever reaches an STA/UI thread, and the sample lease is
// exclusively owned after the callback clones its COM reference.
unsafe impl Send for Message {}
#[implement(IMFSourceReaderCallback)]
struct Callback {
    tx: SyncSender<Message>,
}
impl IMFSourceReaderCallback_Impl for Callback_Impl {
    fn OnReadSample(
        &self,
        status: HRESULT,
        _: u32,
        flags: u32,
        timestamp: i64,
        sample: Ref<'_, IMFSample>,
    ) -> windows::core::Result<()> {
        let result = status
            .ok()
            .map_err(Error::from)
            .map(|_| (sample.as_ref().cloned(), timestamp, flags));
        // Only one asynchronous read is outstanding, so this never queues a
        // growing camera backlog. Stop has its own reserved queue slot.
        let _ = self.tx.try_send(Message::Sample(result));
        Ok(())
    }
    fn OnFlush(&self, _: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnEvent(&self, _: u32, _: Ref<'_, IMFMediaEvent>) -> windows::core::Result<()> {
        Ok(())
    }
}
struct Source(IMFMediaSource);
impl Drop for Source {
    fn drop(&mut self) {
        unsafe {
            let _ = self.0.Shutdown();
        }
    }
}
/// Created ONLY after explicit camera enable. Drop flushes the asynchronous
/// reader, shuts down the source (camera off), and joins. Hardware H.264 required.
pub struct CameraSession {
    tx: SyncSender<Message>,
    rx: Receiver<EncodedFrame>,
    worker: Option<JoinHandle<()>>,
    failure: Arc<Mutex<Option<Error>>>,
    stopped: Arc<AtomicBool>,
    keyframe: Arc<AtomicBool>,
    pub encoder_kind: EncoderKind,
}
impl CameraSession {
    pub fn start(device: CameraDevice, options: CameraOptions) -> Result<Self> {
        options.validate()?;
        if device.id.is_empty() || device.id.len() > 32768 {
            return Err(Error::Invalid("invalid camera ID".into()));
        }
        let (tx, messages) = mpsc::sync_channel(4);
        let callback_tx = tx.clone();
        let (frames, rx) = mpsc::sync_channel(3);
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let failure = Arc::new(Mutex::new(None));
        let err = failure.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let keyframe = Arc::new(AtomicBool::new(false));
        let key = keyframe.clone();
        let worker = thread::Builder::new()
            .name("sticktalk-camera".into())
            .spawn(move || {
                let result = camera_loop(
                    device,
                    options,
                    messages,
                    callback_tx,
                    frames,
                    &ready_tx,
                    &stop,
                    &key,
                );
                if let Err(e) = result {
                    *err.lock().unwrap() = Some(e.clone());
                    let _ = ready_tx.try_send(Err(e));
                }
            })
            .map_err(|e| Error::Platform(e.to_string()))?;
        let encoder_kind = match ready.recv().unwrap_or(Err(Error::Stopped)) {
            Ok(kind) => kind,
            Err(e) => {
                let _ = worker.join();
                return Err(e);
            }
        };
        Ok(Self {
            tx,
            rx,
            worker: Some(worker),
            failure,
            stopped,
            keyframe,
            encoder_kind,
        })
    }
    pub fn request_keyframe(&self) {
        self.keyframe.store(true, Ordering::Release);
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<EncodedFrame> {
        self.rx.recv_timeout(timeout).map_err(|e| {
            self.failure.lock().unwrap().clone().unwrap_or(match e {
                mpsc::RecvTimeoutError::Timeout => Error::Timeout("camera frame"),
                _ => Error::Stopped,
            })
        })
    }
    pub fn stop(&mut self) -> Result<()> {
        self.stopped.store(true, Ordering::Release);
        let _ = self.tx.try_send(Message::Stop);
        if let Some(w) = self.worker.take() {
            w.join()
                .map_err(|_| Error::Platform("camera worker panicked".into()))?;
        }
        Ok(())
    }
}
impl Drop for CameraSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
#[allow(clippy::too_many_arguments)]
fn camera_loop(
    device: CameraDevice,
    options: CameraOptions,
    messages: Receiver<Message>,
    callback_tx: SyncSender<Message>,
    frames: SyncSender<EncodedFrame>,
    ready: &SyncSender<Result<EncoderKind>>,
    stopped: &AtomicBool,
    key: &AtomicBool,
) -> Result<()> {
    let _runtime = Runtime::new()?;
    // Own the callback queue as a local declared AFTER Runtime: queued MF sample
    // leases must be released before this thread calls MFShutdown, even on an
    // early error/stop with an unread callback pending.
    let camera_messages = messages;
    let gpu = Gpu::new()?;
    let mut encoder = Encoder::new(
        &gpu,
        options.size,
        &ShareOptions {
            quality: Quality {
                fps: options.fps,
                resolution: Resolution::Source,
            },
            bitrate: options.bitrate,
            ..Default::default()
        },
        false,
    )?;
    unsafe {
        let attrs = source_attributes()?;
        attrs.SetString(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
            &HSTRING::from(device.id),
        )?;
        let source = Source(MFCreateDeviceSource(&attrs)?);
        let mut reader_attrs = None;
        MFCreateAttributes(&mut reader_attrs, 4)?;
        let reader_attrs = required(reader_attrs, "camera reader attributes")?;
        let callback: IMFSourceReaderCallback = Callback { tx: callback_tx }.into();
        let manager = manager(&gpu)?;
        reader_attrs.SetUnknown(&MF_SOURCE_READER_ASYNC_CALLBACK, &callback)?;
        reader_attrs.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &manager)?;
        reader_attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        reader_attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        let reader = MFCreateSourceReaderFromMediaSource(&source.0, &reader_attrs)?;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(VIDEO, true)?;
        // Select requested native camera timing first, then NV12 conversion.
        let mut matched = false;
        for index in 0..1024 {
            let Ok(native) = reader.GetNativeMediaType(VIDEO, index) else {
                break;
            };
            let size = native.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
            let rate = native.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
            if size == (u64::from(options.size.width) << 32) | u64::from(options.size.height)
                && rate as u32 != 0
                && (rate >> 32).abs_diff(u64::from(rate as u32) * u64::from(options.fps.get()))
                    * 1000
                    <= u64::from(rate as u32) * u64::from(options.fps.get()) * 5
            {
                reader.SetCurrentMediaType(VIDEO, None, &native)?;
                matched = true;
                break;
            }
        }
        if !matched {
            return Err(Error::Invalid(
                "camera does not expose requested native size/FPS".into(),
            ));
        }
        reader.SetCurrentMediaType(
            VIDEO,
            None,
            &media_type(MFVideoFormat_NV12, options.size, options.fps.get())?,
        )?;
        let current = reader.GetCurrentMediaType(VIDEO)?;
        if current.GetGUID(&MF_MT_SUBTYPE)? != MFVideoFormat_NV12 {
            return Err(Error::Platform("camera did not negotiate NV12".into()));
        }
        let texture = gpu.texture(
            options.size,
            DXGI_FORMAT_NV12,
            D3D11_BIND_RENDER_TARGET.0 as u32,
        )?;
        reader.ReadSample(VIDEO, 0, None, None, None, None)?;
        ready
            .send(Ok(encoder.kind.clone()))
            .map_err(|_| Error::Stopped)?;
        let mut epoch = None;
        let mut last_timestamp = -1;
        let mut force_keyframe = true;
        let result = (|| -> Result<()> {
            while !stopped.load(Ordering::Acquire) {
                match camera_messages.recv().map_err(|_| Error::Stopped)? {
                    Message::Stop => break,
                    Message::Sample(result) => {
                        let (sample, time, flags) = result?;
                        if flags
                            & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32
                                | MF_SOURCE_READERF_ERROR.0 as u32)
                            != 0
                        {
                            return Err(Error::Platform("camera ended or failed".into()));
                        }
                        if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                            return Err(Error::Platform(
                                "camera format changed; reopen session".into(),
                            ));
                        }
                        if let Some(sample) = sample {
                            let start = Instant::now();
                            upload_camera_sample(&gpu, &sample, &texture, options.size)?;
                            if force_keyframe || key.swap(false, Ordering::AcqRel) {
                                encoder.keyframe()?;
                                force_keyframe = false;
                            }
                            let timestamp = time - *epoch.get_or_insert(time);
                            if timestamp <= last_timestamp {
                                return Err(Error::Platform("camera timestamp regressed".into()));
                            }
                            last_timestamp = timestamp;
                            let duration = 10_000_000 / i64::from(options.fps.get());
                            let data = encoder.encode(&texture, timestamp, duration)?;
                            if data.len() > 8 * 1024 * 1024 {
                                return Err(Error::Platform("camera access unit >8 MiB".into()));
                            }
                            if !data.is_empty() {
                                let frame = EncodedFrame {
                                    keyframe: annexb::contains(&data, 5),
                                    data,
                                    timestamp,
                                    duration,
                                    size: options.size,
                                    generation: 0,
                                    encode_latency: start.elapsed(),
                                };
                                if frames.try_send(frame).is_err() {
                                    force_keyframe = true;
                                }
                            }
                        }
                        if !stopped.load(Ordering::Acquire) {
                            reader.ReadSample(VIDEO, 0, None, None, None, None)?;
                        }
                    }
                }
            }
            Ok(())
        })();
        let _ = reader.Flush(VIDEO);
        result
    }
}
/// Prefer DXGI camera surfaces. USB cameras may deliver CPU NV12; only this
/// camera ingress uploads pixels. Screen capture remains zero-readback.
unsafe fn upload_camera_sample(
    gpu: &Gpu,
    sample: &IMFSample,
    texture: &ID3D11Texture2D,
    size: Size,
) -> Result<()> {
    let buffer = sample.GetBufferByIndex(0)?;
    if let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() {
        let mut src = None;
        dxgi.GetResource(&ID3D11Texture2D::IID, &mut src as *mut _ as *mut *mut _)?;
        let src: ID3D11Texture2D = required(src, "camera DXGI texture")?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        src.GetDesc(&mut desc);
        if desc.Format != DXGI_FORMAT_NV12 || desc.Width < size.width || desc.Height < size.height {
            return Err(Error::Platform("invalid camera DXGI surface".into()));
        }
        gpu.context.CopySubresourceRegion(
            texture,
            0,
            0,
            0,
            0,
            &src,
            dxgi.GetSubresourceIndex()?,
            Some(&D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: size.width,
                bottom: size.height,
                back: 1,
            }),
        );
        return Ok(());
    }
    // IMF2DBuffer handles padded device pitch correctly. Never assume a USB
    // camera's NV12 stride equals its visible width.
    if let Ok(buffer) = buffer.cast::<IMF2DBuffer>() {
        let mut ptr = std::ptr::null_mut();
        let mut pitch = 0;
        buffer.Lock2D(&mut ptr, &mut pitch)?;
        let result = if pitch < size.width as i32 {
            Err(Error::Platform(
                "unsupported negative/short camera stride".into(),
            ))
        } else {
            gpu.context
                .UpdateSubresource(texture, 0, None, ptr.cast(), pitch as u32, 0);
            Ok(())
        };
        buffer.Unlock2D()?;
        return result;
    }
    let buffer = sample.ConvertToContiguousBuffer()?;
    let mut ptr = std::ptr::null_mut();
    let mut len = 0;
    buffer.Lock(&mut ptr, None, Some(&mut len))?;
    let result = if len as u64 != u64::from(size.width) * u64::from(size.height) * 3 / 2 {
        Err(Error::Platform(
            "camera CPU NV12 is not tightly packed".into(),
        ))
    } else {
        gpu.context
            .UpdateSubresource(texture, 0, None, ptr.cast(), size.width, 0);
        Ok(())
    };
    buffer.Unlock()?;
    result
}

/// Device-free NV12 ingress for file/generated camera sources and deterministic
/// tests. Own/use on an MTA media thread, just like Decoder. Hardware H.264, same
/// encoder as CameraSession. No camera, source reader, UI or worker is created.
pub struct CameraFrameEncoder {
    encoder: Encoder,
    texture: ID3D11Texture2D,
    gpu: Gpu,
    options: CameraOptions,
    last_timestamp: i64,
    pub encoder_kind: EncoderKind,
    _runtime: Runtime,
}
impl CameraFrameEncoder {
    pub fn new(options: CameraOptions) -> Result<Self> {
        options.validate()?;
        let runtime = Runtime::new()?;
        let gpu = Gpu::new()?;
        let encoder = Encoder::new(
            &gpu,
            options.size,
            &ShareOptions {
                quality: Quality {
                    fps: options.fps,
                    resolution: Resolution::Source,
                },
                bitrate: options.bitrate,
                ..Default::default()
            },
            false,
        )?;
        let texture = gpu.texture(
            options.size,
            DXGI_FORMAT_NV12,
            D3D11_BIND_RENDER_TARGET.0 as u32,
        )?;
        Ok(Self {
            encoder_kind: encoder.kind.clone(),
            encoder,
            texture,
            gpu,
            options,
            last_timestamp: -1,
            _runtime: runtime,
        })
    }
    pub fn request_keyframe(&mut self) -> Result<()> {
        self.encoder.keyframe()
    }
    /// Pitched NV12 (Y then UV, same positive pitch), complete progressive frame.
    /// Data is uploaded once; compressed encoding remains hardware accelerated.
    pub fn encode_nv12(
        &mut self,
        data: &[u8],
        stride: u32,
        timestamp: i64,
    ) -> Result<EncodedFrame> {
        validate_nv12(self.options.size, data.len(), stride)?;
        if timestamp < 0 || timestamp <= self.last_timestamp {
            return Err(Error::Invalid(
                "video timestamp must be nonnegative and strictly increasing".into(),
            ));
        }
        let started = Instant::now();
        unsafe {
            self.gpu.context.UpdateSubresource(
                &self.texture,
                0,
                None,
                data.as_ptr().cast(),
                stride,
                0,
            );
        }
        let duration = 10_000_000 / i64::from(self.options.fps.get());
        let data = self.encoder.encode(&self.texture, timestamp, duration)?;
        self.last_timestamp = timestamp;
        if data.len() > 8 * 1024 * 1024 {
            return Err(Error::Platform("video access unit >8 MiB".into()));
        }
        Ok(EncodedFrame {
            keyframe: annexb::contains(&data, 5),
            data,
            timestamp,
            duration,
            size: self.options.size,
            generation: 0,
            encode_latency: started.elapsed(),
        })
    }
}
fn validate_nv12(size: Size, len: usize, stride: u32) -> Result<()> {
    if stride < size.width
        || stride > 65536
        || len as u64 != u64::from(stride) * u64::from(size.height) * 3 / 2
    {
        return Err(Error::Invalid(
            "invalid complete NV12 frame size/stride".into(),
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_camera_and_fake_frame_validation_do_not_open_devices() {
        let options = CameraOptions::default();
        assert_eq!(
            options.size,
            Size {
                width: 1280,
                height: 720
            }
        );
        assert_eq!(options.fps, FrameRate::Fps30);
        assert!(validate_nv12(options.size, 1280 * 720 * 3 / 2, 1280).is_ok());
        assert!(validate_nv12(options.size, 1344 * 720 * 3 / 2, 1344).is_ok());
        assert!(validate_nv12(options.size, 1280 * 720 * 3 / 2 - 1, 1280).is_err());
        assert!(validate_nv12(options.size, 1280 * 720 * 3 / 2, 1279).is_err());
        assert!(CameraOptions {
            size: Size {
                width: 1279,
                height: 720
            },
            ..Default::default()
        }
        .validate()
        .is_err());
    }
    #[test]
    fn fake_source_reader_callback_delivers_errors_without_a_camera() {
        let (tx, rx) = mpsc::sync_channel(4);
        let callback: IMFSourceReaderCallback = Callback { tx }.into();
        unsafe {
            callback
                .OnReadSample(HRESULT(0x80004005u32 as i32), VIDEO, 0, 0, None)
                .unwrap();
        }
        assert!(matches!(rx.recv().unwrap(), Message::Sample(Err(_))));
    }
}
