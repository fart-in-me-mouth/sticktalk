use super::runtime::required;
use crate::{Result, Size};
use std::mem::ManuallyDrop;
use windows::{
    core::Interface,
    Win32::{
        Foundation::{HMODULE, RECT},
        Graphics::{
            Direct3D::*,
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
    },
};

pub(super) struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub name: String,
}
impl Gpu {
    pub fn new() -> Result<Self> {
        unsafe {
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device = required(device, "D3D11CreateDevice")?;
            let mt: ID3D11Multithread = required(context.clone(), "context")?.cast()?;
            let _ = mt.SetMultithreadProtected(true);
            let dxgi: IDXGIDevice = device.cast()?;
            let desc = dxgi.GetAdapter()?.GetDesc()?;
            let end = desc
                .Description
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(desc.Description.len());
            Ok(Self {
                device,
                context: required(context, "context")?,
                name: String::from_utf16_lossy(&desc.Description[..end]),
            })
        }
    }
    pub fn texture(&self, size: Size, format: DXGI_FORMAT, bind: u32) -> Result<ID3D11Texture2D> {
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: size.width,
                Height: size.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: format,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: bind,
                ..Default::default()
            };
            let mut texture = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut texture))?;
            required(texture, "CreateTexture2D")
        }
    }
}

/// BGRA→NV12 and resizing are a single GPU video-processor blit. There are no
/// staging textures, Map calls, or CPU pixel buffers in this hardware path.
pub(super) struct Converter {
    video: ID3D11VideoDevice,
    context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    pub texture: ID3D11Texture2D,
    output: ID3D11VideoProcessorOutputView,
    pub input_size: Size,
    pub output_size: Size,
}
impl Converter {
    pub fn new(gpu: &Gpu, input: Size, output: Size, fps: u32) -> Result<Self> {
        unsafe {
            let video: ID3D11VideoDevice = gpu.device.cast()?;
            let context: ID3D11VideoContext = gpu.context.cast()?;
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: DXGI_RATIONAL {
                    Numerator: fps,
                    Denominator: 1,
                },
                InputWidth: input.width,
                InputHeight: input.height,
                OutputFrameRate: DXGI_RATIONAL {
                    Numerator: fps,
                    Denominator: 1,
                },
                OutputWidth: output.width,
                OutputHeight: output.height,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };
            let enumerator = video.CreateVideoProcessorEnumerator(&desc)?;
            let processor = video.CreateVideoProcessor(&enumerator, 0)?;
            let texture =
                gpu.texture(output, DXGI_FORMAT_NV12, D3D11_BIND_RENDER_TARGET.0 as u32)?;
            let view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                ..Default::default()
            };
            let mut view = None;
            video.CreateVideoProcessorOutputView(
                &texture,
                &enumerator,
                &view_desc,
                Some(&mut view),
            )?;
            let src = RECT {
                left: 0,
                top: 0,
                right: input.width as i32,
                bottom: input.height as i32,
            };
            let dst = RECT {
                left: 0,
                top: 0,
                right: output.width as i32,
                bottom: output.height as i32,
            };
            context.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&src));
            context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dst));
            context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&dst));
            context.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            // RGB full-range in, BT.709 studio-range NV12 out (YCbCr_Matrix=1).
            let colors: ID3D11VideoContext1 = context.cast()?;
            colors.VideoProcessorSetStreamColorSpace1(
                &processor,
                0,
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            );
            colors.VideoProcessorSetOutputColorSpace1(
                &processor,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
            );
            Ok(Self {
                video,
                context,
                enumerator,
                processor,
                texture,
                output: required(view, "output view")?,
                input_size: input,
                output_size: output,
            })
        }
    }
    pub fn convert(&self, input: &ID3D11Texture2D) -> Result<()> {
        unsafe {
            let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                ..Default::default()
            };
            let mut view = None;
            self.video.CreateVideoProcessorInputView(
                input,
                &self.enumerator,
                &desc,
                Some(&mut view),
            )?;
            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(view),
                ..Default::default()
            };
            let result = self.context.VideoProcessorBlt(
                &self.processor,
                &self.output,
                0,
                std::slice::from_ref(&stream),
            );
            // windows-rs uses ManuallyDrop for embedded COM pointers; release once.
            ManuallyDrop::drop(&mut stream.pInputSurface);
            result?;
            Ok(())
        }
    }
}
