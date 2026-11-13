//! Explicit one-shot picker readback. Never used by the streaming path.
use super::{
    capture::Capture,
    gpu::Gpu,
    runtime::{required, Runtime},
};
use crate::{Error, Result, Source, SourceKind};
use std::time::{Duration, Instant};
use windows::Win32::{
    Foundation::HWND, Graphics::Direct3D11::*, UI::WindowsAndMessaging::GetWindowThreadProcessId,
};

#[derive(Clone, Debug)]
pub struct SourceThumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
pub fn source_thumbnail(source: &Source) -> Result<SourceThumbnail> {
    let _runtime = Runtime::new()?;
    let gpu = Gpu::new()?;
    let mut capture = Capture::new(&gpu.device, source, false)?;
    let deadline = Instant::now() + Duration::from_millis(250);
    let (_lease, texture, _) = loop {
        if let Some(frame) = capture.next()? {
            break frame;
        }
        if Instant::now() >= deadline {
            return Err(Error::Timeout("source thumbnail"));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);
        if desc.Width == 0
            || desc.Height == 0
            || u64::from(desc.Width) * u64::from(desc.Height) > 4096 * 4096
        {
            return Err(Error::Invalid(
                "thumbnail surface exceeds readback bound".into(),
            ));
        }
        let width = 160;
        let height = (u64::from(desc.Height) * 160 / u64::from(desc.Width)).clamp(1, 160) as u32;
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.MiscFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        let mut staging = None;
        gpu.device
            .CreateTexture2D(&desc, None, Some(&mut staging))?;
        let staging = required(staging, "thumbnail staging")?;
        gpu.context.CopyResource(&staging, &texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        gpu.context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let bytes = std::slice::from_raw_parts(
            mapped.pData as *const u8,
            mapped.RowPitch as usize * desc.Height as usize,
        );
        let mut rgba = vec![0; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                let offset = (y as usize * desc.Height as usize / height as usize)
                    * mapped.RowPitch as usize
                    + (x as usize * desc.Width as usize / width as usize) * 4;
                let out = (y as usize * width as usize + x as usize) * 4;
                rgba[out..out + 4].copy_from_slice(&[
                    bytes[offset + 2],
                    bytes[offset + 1],
                    bytes[offset],
                    255,
                ]);
            }
        }
        gpu.context.Unmap(&staging, 0);
        Ok(SourceThumbnail {
            width,
            height,
            rgba,
        })
    }
}
pub fn source_process_id(source: &Source) -> Option<u32> {
    if source.kind != SourceKind::Window {
        return None;
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(HWND(source.id as *mut _), Some(&mut pid));
    }
    (pid != 0).then_some(pid)
}

/// GPU-scaled, one-shot picker snapshot. All WGC resources close before return.
pub fn source_gpu_thumbnail(source: &Source) -> Result<std::sync::Arc<super::DisplayFrame>> {
    GpuThumbnailer::new()?.thumbnail(source)
}
/// Reuse one device for an entire bounded picker batch. Apartment-affine.
pub struct GpuThumbnailer {
    gpu: Gpu,
    _runtime: Runtime,
}
impl GpuThumbnailer {
    pub fn new() -> Result<Self> {
        let runtime = Runtime::new()?;
        Ok(Self {
            gpu: Gpu::new()?,
            _runtime: runtime,
        })
    }
    pub fn thumbnail(&self, source: &Source) -> Result<std::sync::Arc<super::DisplayFrame>> {
        let gpu = &self.gpu;
        let mut capture = Capture::new(&gpu.device, source, false)?;
        let deadline = Instant::now() + Duration::from_millis(250);
        let (_lease, texture, _) = loop {
            if let Some(frame) = capture.next()? {
                break frame;
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout("GPU source thumbnail"));
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            texture.GetDesc(&mut desc);
        }
        let input = crate::Size {
            width: desc.Width,
            height: desc.Height,
        };
        input.validate()?;
        let output = crate::Size {
            width: 160,
            height: (u64::from(desc.Height) * 160 / u64::from(desc.Width)).clamp(2, 160) as u32,
        };
        let converter = super::gpu::Converter::rgb(gpu, input, output)?;
        converter.convert(&texture)?;
        super::display::share_rgb(&gpu.device, &gpu.context, &converter.texture, output)
    }
}
