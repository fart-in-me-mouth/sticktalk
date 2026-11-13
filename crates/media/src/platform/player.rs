//! Windowless Media Foundation frame-server. The host owns pacing and lifetime;
//! no window, audio device or decoder is created until `open` is called.
use super::{
    encoder::manager,
    gpu::Gpu,
    runtime::{required, Runtime},
};
use crate::{Error, Result, Size};
use std::{
    path::Path,
    sync::mpsc::{self, Receiver, SyncSender},
    time::Duration,
};
use windows::{
    core::{implement, BSTR},
    Win32::{
        Foundation::RECT,
        Graphics::{Direct3D11::*, Dxgi::Common::*},
        Media::MediaFoundation::*,
        System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
    },
};

#[implement(IMFMediaEngineNotify)]
struct Notify(SyncSender<u32>);
impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _: usize, _: u32) -> windows::core::Result<()> {
        let _ = self.0.try_send(event);
        Ok(())
    }
}

pub struct PlayerFrame {
    pub size: Size,
    pub rgba: Vec<u8>,
    pub position: f64,
    pub duration: f64,
    pub ended: bool,
}

/// Apartment-affine. Construct and use on one bounded blocking worker. Drop
/// shuts MF down before releasing the apartment, including every error path.
pub struct FilePlayer {
    engine: IMFMediaEngine,
    gpu: Gpu,
    texture: ID3D11Texture2D,
    staging: ID3D11Texture2D,
    size: Size,
    events: Receiver<u32>,
    _runtime: Runtime,
}
impl FilePlayer {
    pub fn open(
        path: &Path,
        max_size: Size,
        muted: bool,
        volume: f64,
        looped: bool,
    ) -> Result<Self> {
        Self::open_cancellable(
            path,
            max_size,
            muted,
            volume,
            looped,
            &std::sync::atomic::AtomicBool::new(false),
        )
    }
    pub fn open_cancellable(
        path: &Path,
        max_size: Size,
        muted: bool,
        volume: f64,
        looped: bool,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<Self> {
        let runtime = Runtime::new()?;
        let gpu = Gpu::new()?;
        unsafe {
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 4)?;
            let attrs = required(attrs, "player attributes")?;
            let (tx, events) = mpsc::sync_channel(32);
            let callback: IMFMediaEngineNotify = Notify(tx).into();
            attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &callback)?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager(&gpu)?)?;
            attrs.SetUINT32(
                &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
            )?;
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
            let engine = factory.CreateInstance(0, &attrs)?;
            // Shut down even if opening/negotiation fails.
            struct Opening(Option<IMFMediaEngine>);
            impl Drop for Opening {
                fn drop(&mut self) {
                    if let Some(e) = &self.0 {
                        unsafe {
                            let _ = e.Shutdown();
                        }
                    }
                }
            }
            let mut opening = Opening(Some(engine.clone()));
            engine.SetMuted(muted)?;
            engine.SetVolume(volume.clamp(0., 1.))?;
            engine.SetLoop(looped)?;
            engine.SetSource(&BSTR::from(path.to_string_lossy().as_ref()))?;
            engine.Load()?;
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(Error::Stopped);
                }
                let timeout = deadline.saturating_duration_since(std::time::Instant::now());
                if timeout.is_zero() {
                    return Err(Error::Timeout("video metadata"));
                }
                let event = match events.recv_timeout(timeout.min(Duration::from_millis(100))) {
                    Ok(e) => e,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => return Err(Error::Stopped),
                };
                if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
                    return Err(Error::Platform("Unsupported or corrupt video".into()));
                }
                if event == MF_MEDIA_ENGINE_EVENT_LOADEDDATA.0 as u32
                    || event == MF_MEDIA_ENGINE_EVENT_CANPLAY.0 as u32
                {
                    break;
                }
            }
            let (mut w, mut h) = (0, 0);
            engine.GetNativeVideoSize(Some(&mut w), Some(&mut h))?;
            if w == 0 || h == 0 || w > 16384 || h > 16384 {
                return Err(Error::Invalid("invalid video dimensions".into()));
            }
            let ratio = (max_size.width as f64 / w as f64)
                .min(max_size.height as f64 / h as f64)
                .min(1.);
            let size = Size {
                width: (w as f64 * ratio).max(1.) as u32,
                height: (h as f64 * ratio).max(1.) as u32,
            };
            let texture = gpu.texture(
                size,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                D3D11_BIND_RENDER_TARGET.0 as u32,
            )?;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: size.width,
                Height: size.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..Default::default()
            };
            let mut staging = None;
            gpu.device
                .CreateTexture2D(&desc, None, Some(&mut staging))?;
            let staging = required(staging, "video readback")?;
            opening.0 = None;
            Ok(Self {
                engine,
                gpu,
                texture,
                staging,
                size,
                events,
                _runtime: runtime,
            })
        }
    }
    pub fn play(&self, position: f64) -> Result<()> {
        unsafe {
            if position > 0. {
                self.engine.SetCurrentTime(position)?;
            }
            self.engine.Play()?;
        }
        Ok(())
    }
    pub fn ended(&self) -> bool {
        unsafe { self.engine.IsEnded().as_bool() }
    }
    /// One hardware-decoded frame, scaled on the GPU before bounded RGBA
    /// readback for iced. No full-resolution CPU frame history is retained.
    pub fn frame(&self) -> Result<Option<PlayerFrame>> {
        unsafe {
            while let Ok(e) = self.events.try_recv() {
                if e == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
                    return Err(Error::Platform("video playback failed".into()));
                }
            }
            let mut pts = 0;
            // S_FALSE means there is no new presentation frame, not an error.
            let hr = (windows::core::Interface::vtable(&self.engine).OnVideoStreamTick)(
                windows::core::Interface::as_raw(&self.engine),
                &mut pts,
            );
            hr.ok()?;
            if hr.0 != 0 {
                return Ok(None);
            }
            self.engine.TransferVideoFrame(
                &self.texture,
                None,
                &RECT {
                    left: 0,
                    top: 0,
                    right: self.size.width as i32,
                    bottom: self.size.height as i32,
                },
                None,
            )?;
            self.gpu.context.CopyResource(&self.staging, &self.texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.gpu
                .context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let mut rgba = vec![0; self.size.width as usize * self.size.height as usize * 4];
            for (y, row) in rgba
                .chunks_exact_mut(self.size.width as usize * 4)
                .enumerate()
            {
                let src = std::slice::from_raw_parts(
                    mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                    row.len(),
                );
                for (dst, bgra) in row
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(src.as_chunks::<4>().0)
                {
                    dst.copy_from_slice(&[bgra[2], bgra[1], bgra[0], 255]);
                }
            }
            self.gpu.context.Unmap(&self.staging, 0);
            Ok(Some(PlayerFrame {
                size: self.size,
                rgba,
                position: self.engine.GetCurrentTime(),
                duration: self.engine.GetDuration(),
                ended: self.engine.IsEnded().as_bool(),
            }))
        }
    }
}
impl Drop for FilePlayer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.engine.Pause();
            let _ = self.engine.Shutdown();
        }
    }
}
