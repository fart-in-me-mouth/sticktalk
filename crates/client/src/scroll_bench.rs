//! Deterministic, window-free scrolling profile of the actual application/view/renderer.
use crate::{
    app::{App, Msg},
    theme as t,
};
use iced::{
    advanced::{widget::operation, Renderer as _},
    mouse, Size,
};
use iced_runtime::user_interface::{Cache, UserInterface};
use std::time::Instant;

fn summary(mut samples: Vec<f64>) -> serde_json::Value {
    samples.sort_by(f64::total_cmp);
    let at = |p: f64| samples[((samples.len() - 1) as f64 * p).ceil() as usize];
    serde_json::json!({"p50_ms":at(0.5),"p99_ms":at(0.99),"max_ms":at(1.),"samples":samples.len()})
}
pub fn run(args: &[String], index: usize) -> Result<(), String> {
    if !args.iter().any(|v| v == "--mock") {
        return Err("--bench-scroll requires --mock".into());
    }
    let output = args.get(index + 1).ok_or("Missing benchmark output")?;
    crate::headless::load_fonts();
    let size = Size::new(2560., 1440.);
    let viewport =
        iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(2560, 1440), 1.);
    let use_gpu = args.iter().any(|v| v == "--gpu");
    #[cfg(not(feature = "wgpu"))]
    if use_gpu {
        return Err("Build with wgpu for --gpu".into());
    }
    let mut results = Vec::new();
    for mode in ["wheel-deltas", "scrollbar-seek", "scrollbar-drag-skeleton"] {
        let mut app = App::headless("scroll-5000", size);
        app.state.at_present = false;
        app.fast_scrolling = mode == "scrollbar-drag-skeleton";
        let mut cache = Cache::new();
        #[cfg(not(feature = "wgpu"))]
        let mut renderer = iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY));
        #[cfg(feature = "wgpu")]
        let mut gpu = if use_gpu { Some(Gpu::new()?) } else { None };
        #[cfg(feature = "wgpu")]
        let mut renderer = if let Some(gpu) = &mut gpu {
            iced::Renderer::Primary(gpu.renderer.take().unwrap())
        } else {
            iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
                t::regular(),
                iced::Pixels(t::BODY),
            ))
        };
        let mut pixmap = tiny_skia::Pixmap::new(2560, 1440).unwrap();
        let mut mask = tiny_skia::Mask::new(2560, 1440).unwrap();
        let mut layout = Vec::new();
        let mut draw = Vec::new();
        let mut raster = Vec::new();
        let mut total = Vec::new();
        let mut max_visible = 0;
        for n in 0..660 {
            let start = Instant::now();
            let max_y = (app.history_height() - app.viewport_h).max(0.);
            app.scroll_y = if mode == "wheel-deltas" {
                max_y * 0.5 + (n as f32 * 19.25).sin() * 0.25 + n as f32 * 14.25
            } else {
                max_y * ((n * 173 % 659) as f32 / 659.)
            };
            app.scroll_y = app.scroll_y.min(max_y);
            let (a, b, _, _) = app.visible_range();
            max_visible = max_visible.max(b - a);
            let mut ui = UserInterface::build(crate::ui::view(&app), size, cache, &mut renderer);
            let mut scroll = operation::scrollable::scroll_to::<()>(
                "messages".into(),
                operation::scrollable::AbsoluteOffset {
                    x: Some(0.),
                    y: Some(app.scroll_y),
                },
            );
            ui.operate(&renderer, &mut scroll);
            let mut messages = Vec::new();
            let _ = ui.update(
                &[iced::Event::Window(iced::window::Event::RedrawRequested(
                    Instant::now(),
                ))],
                mouse::Cursor::Unavailable,
                &mut renderer,
                &mut iced::advanced::graphics::core::clipboard::Null,
                &mut messages,
            );
            let layout_ms = start.elapsed().as_secs_f64() * 1000.;
            let drawing = Instant::now();
            ui.draw(
                &mut renderer,
                &t::theme(&app),
                &iced::advanced::renderer::Style {
                    text_color: t::TEXT_PRIMARY,
                },
                mouse::Cursor::Unavailable,
            );
            cache = ui.into_cache();
            let draw_ms = drawing.elapsed().as_secs_f64() * 1000.;
            let rendering = Instant::now();
            // No screenshot allocation, encoding, texture readback or OS surface.
            let damage = iced::Rectangle {
                x: t::RAIL + t::SIDEBAR,
                y: t::HEADER,
                width: app.viewport_w,
                height: app.viewport_h,
            };
            #[cfg(feature = "wgpu")]
            match &mut renderer {
                iced::Renderer::Primary(renderer) => {
                    let gpu = gpu.as_ref().unwrap();
                    let submission =
                        renderer.present(Some(t::BG_CHAT), gpu.format, &gpu.target, &viewport);
                    gpu.device
                        .poll(iced_wgpu::wgpu::PollType::Wait {
                            submission_index: Some(submission),
                            timeout: Some(std::time::Duration::from_secs(5)),
                        })
                        .map_err(|e| e.to_string())?;
                }
                iced::Renderer::Secondary(tiny) => tiny.draw(
                    &mut pixmap.as_mut(),
                    &mut mask,
                    &viewport,
                    &[damage],
                    t::BG_CHAT,
                ),
            }
            #[cfg(not(feature = "wgpu"))]
            renderer.draw(
                &mut pixmap.as_mut(),
                &mut mask,
                &viewport,
                &[damage],
                t::BG_CHAT,
            );
            let raster_ms = rendering.elapsed().as_secs_f64() * 1000.;
            renderer.reset(iced::Rectangle::with_size(size));
            for m in messages {
                if matches!(m, Msg::Measured { .. } | Msg::HistoryViewport(_)) {
                    let _ = app.update(m);
                }
            }
            if n >= 60 {
                layout.push(layout_ms);
                draw.push(draw_ms);
                raster.push(raster_ms);
                total.push(start.elapsed().as_secs_f64() * 1000.);
            }
        }
        results.push(serde_json::json!({"input":mode,"layout":summary(layout),"draw_primitives":summary(draw),"render":summary(raster),"total":summary(total),"max_visible_rows":max_visible,"message_layout_hits":app.message_layouts.hits.get(),"message_layout_misses":app.message_layouts.misses.get()}));
    }
    std::fs::write(output,serde_json::to_vec_pretty(&serde_json::json!({"renderer":if use_gpu { if std::env::var("STICKTALK_BENCH_BACKEND").is_ok_and(|v|v=="vulkan") {"wgpu-vulkan"}else{"wgpu-dx12"} } else { "tiny-skia" },"window_backend":"none","size":[2560,1440],"messages":5000,"warmup_frames":60,"method":"actual iced view; synthetic fractional wheel offsets, random cold seeks and fast-drag skeleton offsets; tiny-skia persistent pixmap + damaged chat region only; GPU full offscreen texture and synchronous submission completion; no PNG encoding/readback/OS window; not physical monitor cadence","results":results})).unwrap()).map_err(|e|e.to_string())
}

#[cfg(feature = "wgpu")]
struct Gpu {
    renderer: Option<iced_wgpu::Renderer>,
    device: iced_wgpu::wgpu::Device,
    target: iced_wgpu::wgpu::TextureView,
    format: iced_wgpu::wgpu::TextureFormat,
}
#[cfg(feature = "wgpu")]
impl Gpu {
    fn new() -> Result<Self, String> {
        Self::with_size(iced::Size::new(2560, 1440))
    }
    fn with_size(size: iced::Size<u32>) -> Result<Self, String> {
        use iced_wgpu::{graphics, wgpu};
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: if std::env::var("STICKTALK_BENCH_BACKEND").is_ok_and(|v| v == "vulkan") {
                    wgpu::Backends::VULKAN
                } else {
                    wgpu::Backends::DX12
                },
                flags: wgpu::InstanceFlags::empty(),
                ..Default::default()
            });
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await
                .map_err(|e| e.to_string())?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("StickTalk offscreen scroll benchmark"),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    ..Default::default()
                })
                .await
                .map_err(|e| e.to_string())?;
            let format = wgpu::TextureFormat::Bgra8UnormSrgb;
            let target = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("offscreen scroll target"),
                    size: wgpu::Extent3d {
                        width: size.width,
                        height: size.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let engine = iced_wgpu::Engine::new(
                &adapter,
                device.clone(),
                queue,
                format,
                None,
                graphics::Shell::headless(),
            );
            Ok(Self {
                renderer: Some(iced_wgpu::Renderer::new(
                    engine,
                    t::regular(),
                    iced::Pixels(t::BODY),
                )),
                device,
                target,
                format,
            })
        })
    }
}
/// Idle offscreen resource harness. No winit surface, OS window or periodic timers.
pub fn resources(args: &[String], index: usize) -> Result<(), String> {
    if !args.iter().any(|s| s == "--mock") {
        return Err("--bench-resources requires --mock".into());
    }
    let path = args.get(index + 1).ok_or("Missing resource output path")?;
    crate::headless::load_fonts();
    let size = Size::new(1280., 800.);
    let viewport = iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(1280, 800), 1.);
    let app = App::headless("guild-chat", size);
    let mut cache = Cache::new();
    #[cfg(feature = "wgpu")]
    let mut gpu = Gpu::with_size(Size::new(1280, 800))?;
    #[cfg(feature = "wgpu")]
    let mut renderer = iced::Renderer::Primary(gpu.renderer.take().unwrap());
    #[cfg(not(feature = "wgpu"))]
    let mut renderer = iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY));
    #[cfg(not(feature = "wgpu"))]
    let mut pixmap = tiny_skia::Pixmap::new(1280, 800).unwrap();
    #[cfg(not(feature = "wgpu"))]
    let mut mask = tiny_skia::Mask::new(1280, 800).unwrap();
    for _ in 0..30 {
        let mut ui = UserInterface::build(crate::ui::view(&app), size, cache, &mut renderer);
        ui.draw(
            &mut renderer,
            &t::theme(&app),
            &iced::advanced::renderer::Style {
                text_color: t::TEXT_PRIMARY,
            },
            mouse::Cursor::Unavailable,
        );
        cache = ui.into_cache();
        #[cfg(feature = "wgpu")]
        if let iced::Renderer::Primary(r) = &mut renderer {
            let sub = r.present(Some(t::BG_CHAT), gpu.format, &gpu.target, &viewport);
            gpu.device
                .poll(iced_wgpu::wgpu::PollType::Wait {
                    submission_index: Some(sub),
                    timeout: Some(std::time::Duration::from_secs(5)),
                })
                .map_err(|e| e.to_string())?;
        }
        #[cfg(not(feature = "wgpu"))]
        renderer.draw(
            &mut pixmap.as_mut(),
            &mut mask,
            &viewport,
            &[iced::Rectangle::with_size(size)],
            t::BG_CHAT,
        );
        renderer.reset(iced::Rectangle::with_size(size));
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    sticktalk::resources::collect();
    #[cfg(windows)]
    let memory = unsafe {
        use windows_sys::Win32::System::{
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::GetCurrentProcess,
        };
        let mut m: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        m.cb = std::mem::size_of_val(&m) as u32;
        if K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            m.cb,
        ) == 0
        {
            return Err("Could not measure process memory".into());
        }
        serde_json::json!({"working_set_bytes":m.WorkingSetSize,"private_bytes":m.PrivateUsage})
    };
    #[cfg(not(windows))]
    let memory = serde_json::Value::Null;
    let output = serde_json::json!({"memory":memory,"size":[1280,800],"renderer":if cfg!(feature="wgpu"){std::env::var("STICKTALK_BENCH_BACKEND").unwrap_or_else(|_|"dx12".into())}else{"tiny-skia".to_string()},"allocator":if cfg!(feature="compact-allocator"){"mimalloc"}else{"system"},"method":"30 warm frames, 2s idle and one allocator collection, live App/view/cache/renderer retained; one offscreen target; NO OS window or swapchain; not directly comparable to native-window working set"});
    std::fs::write(path, serde_json::to_vec_pretty(&output).unwrap()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use crate::scroll::test_geometry::Geometry;
    use iced::Point;
    #[test]
    fn native_iced_fractional_wheel_and_scrollbar_drag_are_headless() {
        let size = Size::new(2560., 1440.);
        let mut app = App::headless("scroll-5000", size);
        app.state.at_present = false;
        app.scroll_y = app.history_height() * 0.5;
        #[cfg(feature = "wgpu")]
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            t::regular(),
            iced::Pixels(t::BODY),
        ));
        #[cfg(not(feature = "wgpu"))]
        let mut renderer = iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY));
        let mut ui = UserInterface::build(crate::ui::view(&app), size, Cache::new(), &mut renderer);
        let mut scroll = operation::scrollable::scroll_to::<()>(
            "messages".into(),
            operation::scrollable::AbsoluteOffset {
                x: Some(0.),
                y: Some(app.scroll_y),
            },
        );
        ui.operate(&renderer, &mut scroll);
        let mut geometry = Geometry::default();
        ui.operate(&renderer, &mut geometry);
        let mut messages = Vec::new();
        let event = iced::Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Pixels { x: 0., y: -0.25 },
        });
        let _ = ui.update(
            &[
                event,
                iced::Event::Window(iced::window::Event::RedrawRequested(Instant::now())),
            ],
            mouse::Cursor::Available(geometry.bounds.center()),
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        let offset = messages
            .iter()
            .find_map(|m| {
                if let Msg::Scroll(v) = m {
                    Some(v.absolute_offset().y)
                } else {
                    None
                }
            })
            .expect("wheel callback");
        assert!(
            (offset - app.scroll_y - 0.25).abs() < 0.05,
            "fractional delta lost: {}",
            offset - app.scroll_y
        );
        messages.clear();
        ui.operate(&renderer, &mut geometry);
        let ratio = geometry.bounds.height / geometry.content.height;
        let thumb = Point::new(
            geometry.bounds.x + geometry.bounds.width - 2.,
            geometry.bounds.y + offset * ratio + (geometry.bounds.height * ratio).max(2.) / 2.,
        );
        let _ = ui.update(
            &[iced::Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))],
            mouse::Cursor::Available(thumb),
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        let target = Point::new(thumb.x, thumb.y + 120.);
        let _ = ui.update(
            &[
                iced::Event::Mouse(mouse::Event::CursorMoved { position: target }),
                iced::Event::Window(iced::window::Event::RedrawRequested(Instant::now())),
            ],
            mouse::Cursor::Available(target),
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        let dragged = messages
            .iter()
            .filter_map(|m| {
                if let Msg::Scroll(v) = m {
                    Some(v.absolute_offset().y)
                } else {
                    None
                }
            })
            .last()
            .expect("drag callback");
        assert!(dragged > offset + 1000., "scrollbar did not drag");
    }
}
