//! Deterministic GPU-only moving workload for capture/encode qualification.
//! No window, swapchain, desktop pixels, staging texture or CPU pixel buffer.
use super::{gpu::Gpu, runtime::required};
use crate::{Result, Size};
use windows::{
    core::s,
    Win32::Graphics::{
        Direct3D::{Fxc::D3DCompile, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST},
        Direct3D11::*,
        Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
    },
};

const SHADER: &str = r#"
cbuffer Clock : register(b0) { uint frame; uint width; uint height; uint unused; };
float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}
float4 ps(float4 pos : SV_Position) : SV_Target {
    uint2 p = uint2(pos.xy);
    uint2 moving = p + uint2(frame * 4, frame * 2);
    float grid = ((moving.x / 48 + moving.y / 48) & 1) ? 0.72 : 0.18;
    float3 color = float3(grid, float(p.y) / height, float(p.x) / width);
    uint x = (frame * 7) % width;
    uint y = (frame * 3) % height;
    if (abs(int(p.x) - int(x)) < 96 && abs(int(p.y) - int(y)) < 96)
        color = float3(1, 0.3, 0.05);
    // Binary frame barcode changes every frame and survives lossy H.264.
    if (p.y < 32) color = ((frame >> ((p.x / 32) % 16)) & 1) ? 1 : 0;
    return float4(color, 1);
}
"#;

pub(super) struct Pattern {
    gpu: Gpu,
    pub size: Size,
    texture: ID3D11Texture2D,
    target: ID3D11RenderTargetView,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    constants: ID3D11Buffer,
    raster: ID3D11RasterizerState,
    frame: u32,
}
impl Pattern {
    pub fn new(gpu: &Gpu, size: Size) -> Result<Self> {
        size.validate()?;
        unsafe {
            let compile =
                |entry, target| -> Result<Vec<u8>> {
                    let mut code = None;
                    D3DCompile(
                        SHADER.as_ptr().cast(),
                        SHADER.len(),
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
                    let code = required(code, "pattern shader")?;
                    Ok(std::slice::from_raw_parts(
                        code.GetBufferPointer().cast(),
                        code.GetBufferSize(),
                    )
                    .to_vec())
                };
            let mut vertex = None;
            let mut pixel = None;
            gpu.device.CreateVertexShader(
                &compile(s!("vs"), s!("vs_5_0"))?,
                None,
                Some(&mut vertex),
            )?;
            gpu.device.CreatePixelShader(
                &compile(s!("ps"), s!("ps_5_0"))?,
                None,
                Some(&mut pixel),
            )?;
            let texture = gpu.texture(
                size,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                D3D11_BIND_RENDER_TARGET.0 as u32,
            )?;
            let mut target = None;
            gpu.device
                .CreateRenderTargetView(&texture, None, Some(&mut target))?;
            let mut constants = None;
            gpu.device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: 16,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut constants),
            )?;
            let mut raster = None;
            gpu.device.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    DepthClipEnable: true.into(),
                    ..Default::default()
                },
                Some(&mut raster),
            )?;
            Ok(Self {
                gpu: gpu.clone(),
                size,
                texture,
                target: required(target, "pattern target")?,
                vertex: required(vertex, "pattern vertex shader")?,
                pixel: required(pixel, "pattern pixel shader")?,
                constants: required(constants, "pattern constants")?,
                raster: required(raster, "pattern rasterizer")?,
                frame: 0,
            })
        }
    }
    pub fn next(&mut self) -> ID3D11Texture2D {
        unsafe {
            let clock = [self.frame, self.size.width, self.size.height, 0];
            let context = &self.gpu.context;
            context.UpdateSubresource(&self.constants, 0, None, clock.as_ptr().cast(), 0, 0);
            context.OMSetRenderTargets(Some(&[Some(self.target.clone())]), None);
            context.RSSetState(&self.raster);
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                Width: self.size.width as f32,
                Height: self.size.height as f32,
                MaxDepth: 1.,
                ..Default::default()
            }]));
            context.IASetInputLayout(None);
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.VSSetShader(&self.vertex, None);
            context.PSSetShader(&self.pixel, None);
            context.PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            context.Draw(3, 0);
            context.OMSetRenderTargets(None, None);
            self.frame = self.frame.wrapping_add(1);
        }
        self.texture.clone()
    }
}
