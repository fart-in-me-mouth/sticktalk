//! Iced primitive importing an immutable D3D11 NT shared RGB texture into DX12.
//! NV12 conversion runs in DisplayBridge's GPU shader. CPU never sees pixels.
use iced::{mouse, Rectangle};
use iced_wgpu::{
    graphics::Viewport,
    primitive::{self, Primitive},
    wgpu,
};
use std::{collections::BTreeMap, sync::Arc};
use sticktalk_media::DisplayFrame;
use windows58::Win32::{Foundation::HANDLE, Graphics::Direct3D12::*};

pub struct Video(pub Arc<DisplayFrame>);
impl<M> iced::widget::shader::Program<M> for Video {
    type State = ();
    type Primitive = Frame;
    fn draw(&self, _: &(), _: mouse::Cursor, _: Rectangle) -> Frame {
        Frame(self.0.clone())
    }
}
#[derive(Debug)]
pub struct Frame(pub Arc<DisplayFrame>);
struct Imported {
    bind: wgpu::BindGroup,
    _texture: wgpu::Texture,
    _lease: Arc<DisplayFrame>,
    touched: bool,
}
pub struct Pipeline {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    frames: BTreeMap<usize, Imported>,
    reported_error: bool,
}
impl primitive::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shared video"),
            source: wgpu::ShaderSource::Wgsl(include_str!("video.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("video"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("video"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            sampler,
            pipeline,
            frames: BTreeMap::new(),
            reported_error: false,
        }
    }
    fn trim(&mut self) {
        self.frames
            .retain(|_, f| std::mem::replace(&mut f.touched, false));
    }
}
impl Primitive for Frame {
    type Pipeline = Pipeline;
    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _: &Rectangle,
        _: &Viewport,
    ) {
        let key = Arc::as_ptr(&self.0) as usize;
        if let Some(f) = pipeline.frames.get_mut(&key) {
            f.touched = true;
            return;
        }
        if pipeline.frames.len() >= 24 {
            return;
        }
        let texture = match import(device, queue, &self.0) {
            Ok(texture) => texture,
            Err(error) => {
                if !pipeline.reported_error {
                    eprintln!("GPU video import unavailable: {error}");
                    pipeline.reported_error = true;
                }
                return;
            }
        };
        {
            let view = texture.create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shared video"),
                layout: &pipeline.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                    },
                ],
            });
            pipeline.frames.insert(
                key,
                Imported {
                    bind,
                    _texture: texture,
                    _lease: self.0.clone(),
                    touched: true,
                },
            );
        }
    }
    fn draw(&self, pipeline: &Pipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        if let Some(f) = pipeline.frames.get(&(Arc::as_ptr(&self.0) as usize)) {
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &f.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        true
    }
}
/// Import fails closed on a non-DX12 renderer or adapter mismatch. No silent readback.
pub fn import(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    frame: &DisplayFrame,
) -> Result<wgpu::Texture, String> {
    unsafe {
        let raw = device
            .as_hal::<wgpu::hal::api::Dx12>()
            .ok_or("Video requires the DX12 renderer")?;
        let mut resource: Option<ID3D12Resource> = None;
        raw.raw_device()
            .OpenSharedHandle(HANDLE(frame.texture_handle() as *mut _), &mut resource)
            .map_err(|e| e.to_string())?;
        let mut fence: Option<ID3D12Fence> = None;
        raw.raw_device()
            .OpenSharedHandle(HANDLE(frame.fence_handle() as *mut _), &mut fence)
            .map_err(|e| e.to_string())?;
        raw.raw_queue()
            .Wait(&fence.ok_or("No shared fence")?, 1)
            .map_err(|e| e.to_string())?;
        let size = wgpu::Extent3d {
            width: frame.size.width,
            height: frame.size.height,
            depth_or_array_layers: 1,
        };
        let hal = wgpu::hal::dx12::Device::texture_from_raw(
            resource.ok_or("No shared resource")?,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureDimension::D2,
            size,
            1,
            1,
        );
        let desc = wgpu::TextureDescriptor {
            label: Some("D3D11 decoded video"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        drop(raw);
        Ok(device.create_texture_from_hal::<wgpu::hal::api::Dx12>(hal, &desc))
    }
}
