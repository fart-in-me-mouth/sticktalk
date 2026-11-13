//! GPU-only production bridge qualification. Readback is confined to this test.
#![cfg(all(windows, feature = "wgpu"))]
use iced_wgpu::{
    graphics::Viewport,
    primitive::{Pipeline, Primitive},
    wgpu,
};
use sticktalk::voice::video;
use sticktalk_media::{CameraFrameEncoder, CameraOptions, Decoder, DisplayBridge, Size};

#[test]
#[ignore = "hardware encoder, decoder and offscreen DX12; no OS window"]
fn shared_nv12_shader_reaches_iced_pipeline_without_cpu_fallback() {
    let size = Size {
        width: 640,
        height: 480,
    };
    let mut encoder = CameraFrameEncoder::new(CameraOptions {
        size,
        ..Default::default()
    })
    .unwrap();
    let nv12 = vec![128u8; 640 * 480 * 3 / 2];
    let mut decoder = Decoder::new().unwrap();
    let mut output = None;
    for i in 0..8 {
        let frame = encoder.encode_nv12(&nv12, 640, i * 333333).unwrap();
        for frame in decoder.decode(frame).unwrap() {
            output = Some(frame);
        }
        if output.is_some() {
            break;
        }
    }
    let mut bridge = DisplayBridge::new(decoder.device()).unwrap();
    let frame = bridge
        .export(output.as_ref().expect("decoder surface"))
        .unwrap()
        .unwrap();
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..Default::default()
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let adapter = runtime
        .block_on(instance.request_adapter(&Default::default()))
        .unwrap();
    let (device, queue) = runtime
        .block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .unwrap();
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen iced video"),
        size: wgpu::Extent3d {
            width: 640,
            height: 480,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut pipeline = video::Pipeline::new(&device, &queue, format);
    let primitive = video::Frame(frame);
    let viewport = Viewport::with_physical_size(iced::Size::new(640, 480), 1.);
    primitive.prepare(
        &mut pipeline,
        &device,
        &queue,
        &iced::Rectangle::new(iced::Point::ORIGIN, iced::Size::new(640., 480.)),
        &viewport,
    );
    let mut command = device.create_command_encoder(&Default::default());
    {
        let mut pass = command.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("offscreen video"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        assert!(primitive.draw(&pipeline, &mut pass));
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("TEST ONLY pixel check"),
        size: 640 * 480 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    command.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(640 * 4),
                rows_per_image: Some(480),
            },
        },
        wgpu::Extent3d {
            width: 640,
            height: 480,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([command.finish()]);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = buffer.slice(..).get_mapped_range();
    let pixel = &bytes[(240 * 640 + 320) * 4..(240 * 640 + 320) * 4 + 4];
    println!("D3D11 NV12 -> RGB shader -> NT handle/fence -> DX12/wgpu -> iced pipeline pixel={pixel:?}; CPU pixel copies in production=0");
    for c in &pixel[..3] {
        assert!(
            (122..=139).contains(c),
            "bad neutral BT.709 conversion: {pixel:?}"
        );
    }
    assert_eq!(pixel[3], 255);
    drop(bytes);
    buffer.unmap();
    // Exhaust the bounded exporter while holding leases, then release and recover.
    let frame = output.unwrap();
    let b = bridge.export(&frame).unwrap().unwrap();
    let c = bridge.export(&frame).unwrap().unwrap();
    assert!(bridge.export(&frame).unwrap().is_none());
    drop((b, c));
    assert!(bridge.export(&frame).unwrap().is_some());
}

#[test]
#[ignore = "invisible one-shot WGC picker profiling; never writes captured pixels"]
fn gpu_picker_preview_and_measured_cpu_fallback() {
    let source = sticktalk_media::list_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.kind == sticktalk_media::SourceKind::Monitor && s.primary)
        .unwrap();
    let thumbnailer = sticktalk_media::GpuThumbnailer::new().unwrap();
    for i in 0..3 {
        let before = sticktalk_media::process_usage().unwrap();
        let at = std::time::Instant::now();
        let gpu = thumbnailer.thumbnail(&source).unwrap();
        let elapsed = at.elapsed();
        let after = sticktalk_media::process_usage().unwrap();
        println!(
            "picker GPU sample={i} wall_ms={:.3} cpu_ms={:.3} size={}x{}",
            elapsed.as_secs_f64() * 1000.,
            (after.cpu_seconds - before.cpu_seconds) * 1000.,
            gpu.size.width,
            gpu.size.height
        );
        let before = sticktalk_media::process_usage().unwrap();
        let at = std::time::Instant::now();
        let cpu = sticktalk_media::source_thumbnail(&source).unwrap();
        let elapsed = at.elapsed();
        let after = sticktalk_media::process_usage().unwrap();
        println!(
            "picker CPU FALLBACK sample={i} wall_ms={:.3} cpu_ms={:.3} output_bytes={}",
            elapsed.as_secs_f64() * 1000.,
            (after.cpu_seconds - before.cpu_seconds) * 1000.,
            cpu.rgba.len()
        );
    }
}
