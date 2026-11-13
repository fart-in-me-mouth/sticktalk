//! GPU-only NV12 -> BT.709 RGB -> NT shared texture. No Map/readback path.
//! At most three exported leases. Textures are immutable; wgpu additionally
//! retains imported resources through GPU completion.
use super::{decoder::DecodedFrame, runtime::required};
use crate::{Error, Result, Size};
use std::sync::{Arc, Weak};
use windows::{
    core::{s, Interface, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        Graphics::{
            Direct3D::{Fxc::D3DCompile, *},
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
    },
};
const CONVERT: &str = r#"
Texture2D<float> yplane : register(t0);
Texture2D<float2> uvplane : register(t1);
float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}
float4 ps(float4 pos : SV_Position) : SV_Target {
    int2 p = int2(pos.xy);
    float y = (yplane.Load(int3(p,0)) * 255 - 16) / 219;
    float2 uv = (uvplane.Load(int3(p/2,0)) * 255 - 128) / 224;
    return float4(saturate(float3(y + 1.5748*uv.y,
        y - 0.187324*uv.x - 0.468124*uv.y, y + 1.8556*uv.x)), 1);
}
"#;
#[derive(Debug)]
struct OwnedHandle(usize);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(HANDLE(self.0 as *mut _));
        }
    }
}
/// Keep alive through GPU submission completion. Handles are process-local only.
#[derive(Debug)]
pub struct DisplayFrame {
    texture: OwnedHandle,
    fence: OwnedHandle,
    _resource: ID3D11Texture2D,
    _fence: ID3D11Fence,
    pub size: Size,
    pub timestamp: i64,
}
impl DisplayFrame {
    pub fn texture_handle(&self) -> usize {
        self.texture.0
    }
    pub fn fence_handle(&self) -> usize {
        self.fence.0
    }
}
type Planes = (
    Size,
    ID3D11Texture2D,
    ID3D11ShaderResourceView,
    ID3D11ShaderResourceView,
);
pub struct DisplayBridge {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    raster: ID3D11RasterizerState,
    input: Option<Planes>,
    live: Vec<Weak<DisplayFrame>>,
}
impl DisplayBridge {
    pub fn new(device: &ID3D11Device) -> Result<Self> {
        unsafe {
            let _: ID3D11Device5 = device.cast()?;
            let context = device.GetImmediateContext()?;
            let compile =
                |entry, target| -> Result<Vec<u8>> {
                    let mut code = None;
                    D3DCompile(
                        CONVERT.as_ptr().cast(),
                        CONVERT.len(),
                        None,
                        None,
                        None,
                        entry,
                        target,
                        1 << 15,
                        0,
                        &mut code,
                        None,
                    )?;
                    let code = required(code, "NV12 shader")?;
                    Ok(std::slice::from_raw_parts(
                        code.GetBufferPointer().cast(),
                        code.GetBufferSize(),
                    )
                    .to_vec())
                };
            let mut vs = None;
            let mut ps = None;
            device.CreateVertexShader(&compile(s!("vs"), s!("vs_5_0"))?, None, Some(&mut vs))?;
            device.CreatePixelShader(&compile(s!("ps"), s!("ps_5_0"))?, None, Some(&mut ps))?;
            let mut raster = None;
            device.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    DepthClipEnable: true.into(),
                    ..Default::default()
                },
                Some(&mut raster),
            )?;
            Ok(Self {
                device: device.clone(),
                context,
                vs: required(vs, "vertex shader")?,
                ps: required(ps, "pixel shader")?,
                raster: required(raster, "rasterizer")?,
                input: None,
                live: vec![],
            })
        }
    }
    /// A slow/hidden consumer drops instead of blocking decode or growing memory.
    pub fn export(&mut self, frame: &DecodedFrame) -> Result<Option<Arc<DisplayFrame>>> {
        self.live.retain(|f| f.strong_count() != 0);
        if self.live.len() >= 3 {
            return Ok(None);
        }
        frame.size.validate()?;
        unsafe {
            let mut source = D3D11_TEXTURE2D_DESC::default();
            frame.texture.GetDesc(&mut source);
            if source.Format != DXGI_FORMAT_NV12
                || frame.size.width > source.Width
                || frame.size.height > source.Height
            {
                return Err(Error::Invalid(
                    "display expects a bounded NV12 surface".into(),
                ));
            }
            let padded = Size {
                width: source.Width,
                height: source.Height,
            };
            if self.input.as_ref().is_none_or(|(s, ..)| *s != padded) {
                let mut texture = None;
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: padded.width,
                    Height: padded.height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_NV12,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    ..Default::default()
                };
                self.device
                    .CreateTexture2D(&desc, None, Some(&mut texture))?;
                let texture = required(texture, "NV12 shader surface")?;
                let view = |format| -> Result<ID3D11ShaderResourceView> {
                    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
                        Format: format,
                        ViewDimension: D3D_SRV_DIMENSION_TEXTURE2D,
                        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                            Texture2D: D3D11_TEX2D_SRV {
                                MostDetailedMip: 0,
                                MipLevels: 1,
                            },
                        },
                    };
                    let mut view = None;
                    self.device
                        .CreateShaderResourceView(&texture, Some(&desc), Some(&mut view))?;
                    required(view, "NV12 plane view")
                };
                let y = view(DXGI_FORMAT_R8_UNORM)?;
                let uv = view(DXGI_FORMAT_R8G8_UNORM)?;
                self.input = Some((padded, texture, y, uv));
            }
            let (_, input, y, uv) = self.input.as_ref().unwrap();
            self.context.CopySubresourceRegion(
                input,
                0,
                0,
                0,
                0,
                &frame.texture,
                frame.subresource,
                None,
            );
            let mut texture = None;
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: frame.size.width,
                    Height: frame.size.height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
                    MiscFlags: (D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED).0
                        as u32,
                    ..Default::default()
                },
                None,
                Some(&mut texture),
            )?;
            let texture = required(texture, "shared RGB surface")?;
            let mut target = None;
            self.device
                .CreateRenderTargetView(&texture, None, Some(&mut target))?;
            self.context.OMSetRenderTargets(Some(&[target]), None);
            self.context.OMSetBlendState(None, None, u32::MAX);
            self.context.OMSetDepthStencilState(None, 0);
            self.context.RSSetState(&self.raster);
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                Width: frame.size.width as f32,
                Height: frame.size.height as f32,
                MaxDepth: 1.,
                ..Default::default()
            }]));
            self.context.IASetInputLayout(None);
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vs, None);
            self.context.PSSetShader(&self.ps, None);
            self.context
                .PSSetShaderResources(0, Some(&[Some(y.clone()), Some(uv.clone())]));
            self.context.Draw(3, 0);
            self.context.PSSetShaderResources(0, Some(&[None, None]));
            self.context.OMSetRenderTargets(None, None);
            let resource: IDXGIResource1 = texture.cast()?;
            let handle = OwnedHandle(
                resource
                    .CreateSharedHandle(None, DXGI_SHARED_RESOURCE_READ.0, PCWSTR::null())?
                    .0 as usize,
            );
            let device: ID3D11Device5 = self.device.cast()?;
            let mut fence: Option<ID3D11Fence> = None;
            device.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence)?;
            let fence = required(fence, "display fence")?;
            let fence_handle = OwnedHandle(
                fence
                    .CreateSharedHandle(None, 0x10000000, PCWSTR::null())?
                    .0 as usize,
            );
            let context: ID3D11DeviceContext4 = self.context.cast()?;
            context.Signal(&fence, 1)?;
            self.context.Flush();
            let output = Arc::new(DisplayFrame {
                texture: handle,
                fence: fence_handle,
                _resource: texture,
                _fence: fence,
                size: frame.size,
                timestamp: frame.timestamp,
            });
            self.live.push(Arc::downgrade(&output));
            Ok(Some(output))
        }
    }
}

/// Share an already scaled RGB picker preview. No CPU staging or pixel buffers.
pub(super) fn share_rgb(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    source: &ID3D11Texture2D,
    size: Size,
) -> Result<Arc<DisplayFrame>> {
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        source.GetDesc(&mut desc);
        if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            || desc.Width != size.width
            || desc.Height != size.height
            || size.width > 160
            || size.height > 160
        {
            return Err(Error::Invalid("GPU thumbnail exceeds bound".into()));
        }
        desc.MiscFlags =
            (D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED).0 as u32;
        desc.BindFlags = (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32;
        let mut texture = None;
        device.CreateTexture2D(&desc, None, Some(&mut texture))?;
        let texture = required(texture, "shared preview")?;
        context.CopyResource(&texture, source);
        let resource: IDXGIResource1 = texture.cast()?;
        let handle = OwnedHandle(
            resource
                .CreateSharedHandle(None, DXGI_SHARED_RESOURCE_READ.0, PCWSTR::null())?
                .0 as usize,
        );
        let device5: ID3D11Device5 = device.cast()?;
        let mut fence: Option<ID3D11Fence> = None;
        device5.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence)?;
        let fence = required(fence, "preview fence")?;
        let fence_handle = OwnedHandle(
            fence
                .CreateSharedHandle(None, 0x10000000, PCWSTR::null())?
                .0 as usize,
        );
        let context4: ID3D11DeviceContext4 = context.cast()?;
        context4.Signal(&fence, 1)?;
        context.Flush();
        Ok(Arc::new(DisplayFrame {
            texture: handle,
            fence: fence_handle,
            _resource: texture,
            _fence: fence,
            size,
            timestamp: 0,
        }))
    }
}
