//! Surface-pacing harness: actual iced app/update/view, native secondary HWND,
//! synthetic iced events only. Never injects input or reads back screenshots.
use crate::{app::App, theme as t};
use iced::{advanced::Renderer as _, mouse, Event, Point, Size};
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_wgpu::{graphics, wgpu};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

pub fn run(args: &[String], index: usize) -> Result<(), String> {
    if !args.iter().any(|a| a == "--mock") {
        return Err("--scroll-bench requires --mock".into());
    }
    let output = PathBuf::from(args.get(index + 1).ok_or("Missing benchmark JSON path")?);
    std::env::set_var("STICKTALK_AGENT_WINDOW", "secondary");
    if args.iter().any(|a| a == "--baseline") {
        std::env::set_var("STICKTALK_SCROLL_BASELINE", "1");
    }
    crate::headless::load_fonts();
    let mut bench = Bench::new(output);
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut bench).map_err(|e| e.to_string())?;
    if let Some(error) = bench.error {
        Err(error)
    } else {
        Ok(())
    }
}
struct Gpu {
    renderer: iced::Renderer,
    device: wgpu::Device,
    surface: Option<wgpu::Surface<'static>>,
    texture: wgpu::TextureView,
    format: wgpu::TextureFormat,
    present_mode: wgpu::PresentMode,
}
impl Gpu {
    fn new(window: Arc<Window>, visible: bool) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::DX12,
                flags: wgpu::InstanceFlags::empty(),
                ..Default::default()
            });
            let surface = if visible {
                Some(
                    instance
                        .create_surface(window.clone())
                        .map_err(|e| e.to_string())?,
                )
            } else {
                None
            };
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: surface.as_ref(),
                    force_fallback_adapter: false,
                })
                .await
                .map_err(|e| e.to_string())?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("Scroll surface bench"),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    ..Default::default()
                })
                .await
                .map_err(|e| e.to_string())?;
            let format = surface
                .as_ref()
                .map(|s| {
                    s.get_capabilities(&adapter)
                        .formats
                        .into_iter()
                        .find(|f| f.is_srgb())
                        .unwrap()
                })
                .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb);
            let present_mode = if std::env::var_os("STICKTALK_SCROLL_BASELINE").is_none()
                && surface.as_ref().is_some_and(|s| {
                    s.get_capabilities(&adapter)
                        .present_modes
                        .contains(&wgpu::PresentMode::Mailbox)
                }) {
                wgpu::PresentMode::Mailbox
            } else {
                wgpu::PresentMode::Fifo
            };
            if let Some(surface) = &surface {
                surface.configure(
                    &device,
                    &wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format,
                        width: 1280,
                        height: 800,
                        present_mode,
                        desired_maximum_frame_latency: 1,
                        alpha_mode: wgpu::CompositeAlphaMode::Auto,
                        view_formats: vec![],
                    },
                );
            }
            let texture = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Fallback offscreen target"),
                    size: wgpu::Extent3d {
                        width: 1280,
                        height: 800,
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
                queue.clone(),
                format,
                None,
                graphics::Shell::headless(),
            );
            Ok(Self {
                renderer: iced::Renderer::Primary(iced_wgpu::Renderer::new(
                    engine,
                    t::regular(),
                    iced::Pixels(t::BODY),
                )),
                device,
                surface,
                texture,
                format,
                present_mode,
            })
        })
    }
    fn present(&mut self) -> Result<(), String> {
        let frame = self
            .surface
            .as_ref()
            .map(|s| s.get_current_texture())
            .transpose()
            .map_err(|e| e.to_string())?;
        let view = frame
            .as_ref()
            .map(|f| f.texture.create_view(&Default::default()));
        let iced::Renderer::Primary(renderer) = &mut self.renderer else {
            unreachable!()
        };
        let submission = renderer.present(
            Some(t::BG_CHAT),
            self.format,
            view.as_ref().unwrap_or(&self.texture),
            &graphics::Viewport::with_physical_size(Size::new(1280, 800), 1.),
        );
        if let Some(frame) = frame {
            frame.present();
        } else {
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(5)),
                })
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
struct Bench {
    output: PathBuf,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    app: App,
    cache: Cache,
    start: Option<Instant>,
    idle: Option<(Instant, f64)>,
    cpu_start: f64,
    frames: Vec<f64>,
    offsets: Vec<f32>,
    refresh: f64,
    visible: bool,
    next_input: f64,
    last_time: f64,
    jumps: u8,
    error: Option<String>,
    render_ms: Vec<f64>,
    idle_cpu: Option<f64>,
    active_cpu: Option<f64>,
    max_rows: usize,
    gpu_counter: Option<crate::scroll::GpuCounter>,
    gpu_percent: Option<f64>,
}
impl Bench {
    fn new(output: PathBuf) -> Self {
        let mut app = App::headless("scroll-5000", Size::new(1280., 800.));
        // Keep 10k bodies only in this synthetic fixture, not in the production cache.
        let image = crate::mock::Mock::new().model.messages[&sticktalk_proto::Id(12)]
            [&sticktalk_proto::Id(2274)]
            .attachments
            .clone();
        let messages = app
            .state
            .model
            .messages
            .get_mut(&sticktalk_proto::Id(12))
            .unwrap();
        let sample = messages.first_key_value().unwrap().1.clone();
        for n in 0..10_000u64 {
            let mut m = sample.clone();
            m.id = sticktalk_proto::Id(1_000_000 + n);
            m.created_at += n * 60_000;
            m.content=match n%6 { 0=>format!("Message {n}: {}","Long wrapped context to exercise cached paragraphs and stable measurement. ".repeat(12)),1=>format!("```rust\nlet row = {n};\nprintln!(\"cached text\");\n```"),2=>format!("https://example.com/posts/{n}\n**Embed-like preview**\nA description that reserves its layout before media decoding."),_=>format!("Message {n}: a short reply with `inline code`.") };
            if n % 6 == 3 || n % 6 == 4 {
                m.attachments = image.clone();
                if n % 6 == 4 {
                    for a in &mut m.attachments {
                        a.content_type = "image/gif".into();
                        a.filename = "clip.gif".into();
                    }
                }
            }
            messages.insert(m.id, m);
        }
        app.state.at_present = false;
        app.scroll_y = 0.;
        app.state.history_more = false;
        Self {
            output,
            window: None,
            gpu: None,
            app,
            cache: Cache::new(),
            start: None,
            idle: None,
            cpu_start: 0.,
            frames: vec![],
            offsets: vec![],
            refresh: 60.,
            visible: false,
            next_input: 0.,
            last_time: 0.,
            jumps: 0,
            error: None,
            render_ms: vec![],
            idle_cpu: None,
            active_cpu: None,
            max_rows: 0,
            gpu_counter: None,
            gpu_percent: None,
        }
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        self.window = None;
        event_loop.exit();
    }
    fn frame(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let now = Instant::now();
        let start = *self.start.get_or_insert_with(|| {
            self.cpu_start = cpu_seconds();
            now
        });
        let elapsed = now.duration_since(start).as_secs_f64();
        if elapsed >= 11. {
            self.gpu_percent = self.gpu_counter.as_ref().and_then(|c| c.sample());
            self.active_cpu = Some((cpu_seconds() - self.cpu_start) / elapsed * 100.);
            self.idle = Some((Instant::now(), f64::NAN));
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_secs(1),
            ));
            return Ok(());
        }
        let mut events = Vec::new();
        while elapsed < 6. && elapsed >= self.next_input {
            events.push(Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines {
                    x: 0.,
                    y: if elapsed < 3. { -1. } else { -5. },
                },
            }));
            self.next_input += if self.next_input < 3. { 0.11 } else { 0.016 };
        }
        if (6. ..9.).contains(&elapsed) {
            events.push(Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Pixels {
                    x: 0.,
                    y: -((elapsed - self.last_time) * 2400.) as f32,
                },
            }));
        }
        for (n, at, key) in [
            (0, 9., iced::keyboard::key::Named::Home),
            (1, 9.4, iced::keyboard::key::Named::End),
            (2, 9.8, iced::keyboard::key::Named::PageUp),
            (3, 10.2, iced::keyboard::key::Named::PageDown),
        ] {
            if self.jumps == n && elapsed >= at {
                self.jumps += 1;
                // Same widget keyboard path; construct the complete iced keyboard event.
                events.push(Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(key),
                    modified_key: iced::keyboard::Key::Named(key),
                    physical_key: iced::keyboard::key::Physical::Unidentified(
                        iced::keyboard::key::NativeCode::Unidentified,
                    ),
                    location: iced::keyboard::Location::Standard,
                    modifiers: iced::keyboard::Modifiers::empty(),
                    text: None,
                    repeat: false,
                }));
            }
        }
        self.last_time = elapsed;
        events.push(Event::Window(iced::window::Event::RedrawRequested(now)));
        let gpu = self.gpu.as_mut().unwrap();
        let size = Size::new(1280., 800.);
        let cursor = mouse::Cursor::Available(Point::new(700., 350.));
        let render_start = Instant::now();
        let mut ui = UserInterface::build(
            crate::ui::view(&self.app),
            size,
            std::mem::take(&mut self.cache),
            &mut gpu.renderer,
        );
        let mut out = Vec::new();
        let _ = ui.update(
            &events,
            cursor,
            &mut gpu.renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut out,
        );
        self.cache = ui.into_cache();
        for msg in out {
            let _task = self.app.update(msg);
        }
        // Rebuild before drawing, as iced's runtime does after application messages.
        let mut ui = UserInterface::build(
            crate::ui::view(&self.app),
            size,
            std::mem::take(&mut self.cache),
            &mut gpu.renderer,
        );
        ui.draw(
            &mut gpu.renderer,
            &t::theme(&self.app),
            &iced::advanced::renderer::Style {
                text_color: t::TEXT_PRIMARY,
            },
            cursor,
        );
        self.cache = ui.into_cache();
        let (a, b, _, _) = self.app.visible_range();
        self.max_rows = self.max_rows.max(b - a);
        gpu.present()?;
        gpu.renderer.reset(iced::Rectangle::with_size(size));
        self.render_ms
            .push(render_start.elapsed().as_secs_f64() * 1000.);
        if elapsed >= 1. {
            self.frames.push(start.elapsed().as_secs_f64() * 1000.);
            self.offsets.push(self.app.scroll_y);
        }
        if self.visible && std::env::var_os("STICKTALK_SCROLL_BASELINE").is_some() {
            self.window.as_ref().unwrap().request_redraw();
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                now + Duration::from_secs_f64(1. / self.refresh),
            ));
        }
        Ok(())
    }
    fn save(&self) -> Result<(), String> {
        let interval = 1000. / self.refresh;
        let intervals: Vec<_> = self.frames.windows(2).map(|w| w[1] - w[0]).collect();
        let missed: usize = intervals
            .iter()
            .map(|v| ((v / interval).round() as usize).saturating_sub(1))
            .sum();
        let json = serde_json::json!({"mode":if std::env::var_os("STICKTALK_SCROLL_BASELINE").is_some(){"before-native-jumps"}else{"after-smooth-wheel"},"backend":if self.visible {"secondary-noactivate-dx12-surface"}else{"no-secondary-offscreen-dx12"},"refresh_hz":self.refresh,"refresh_interval_ms":interval,"present_mode":format!("{:?}",self.gpu.as_ref().unwrap().present_mode),"maximum_frame_latency":1,"messages":10000,"duration_seconds":10,"intervals":summary(&intervals),"dropped_frame_percent":100.*missed as f64/(intervals.len()+missed).max(1) as f64,"oversubmitted_interval_percent":100.*intervals.iter().filter(|v|**v<interval*0.8).count() as f64/intervals.len().max(1) as f64,"cpu_percent_one_logical_core":self.active_cpu,"idle_cpu_percent_one_logical_core":self.idle_cpu,"gpu_percent":self.gpu_percent,"gpu_measurement":"Windows PDH per-process 3D engine utilization, mean across benchmark; null if unavailable","memory":memory(),"render_and_present":summary(&self.render_ms),"max_visible_rows":self.max_rows,"layout_hits":self.app.message_layouts.hits.get(),"layout_misses":self.app.message_layouts.misses.get(),"timestamp_kind":"CPU timestamps immediately after SurfaceTexture::present returns; NOT physical scanout/ETW displayed-frame timestamps", "limitations":["synthetic embed-like markdown, static GIF preview; no full embed/GIF player exists in this client", "tasks are not executed in this isolated fixture; no network, media IO, browser or subscriptions"],"present_return_timestamps_ms":self.frames,"scroll_offsets":self.offsets});
        std::fs::write(&self.output, serde_json::to_vec_pretty(&json).unwrap())
            .map_err(|e| e.to_string())
    }
}
impl ApplicationHandler for Bench {
    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) && self.idle.is_none() {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let primary = event_loop.primary_monitor();
        let secondary = event_loop
            .available_monitors()
            .find(|m| Some(m) != primary.as_ref());
        let monitor = secondary.as_ref().or(primary.as_ref());
        self.refresh = monitor
            .and_then(|m| m.refresh_rate_millihertz())
            .map_or(60., |v| v as f64 / 1000.);
        let mut attrs = Window::default_attributes()
            .with_title("StickTalk internal scroll benchmark")
            .with_visible(false)
            .with_active(false)
            .with_inner_size(winit::dpi::PhysicalSize::new(1280, 800))
            .with_resizable(false);
        if let Some(m) = &secondary {
            let p = m.position();
            attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(p.x + 32, p.y + 32));
        }
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.fail(event_loop, e.to_string());
                return;
            }
        };
        self.visible = secondary.is_some() && crate::agent_window::show_secondary(window.as_ref());
        match Gpu::new(window.clone(), self.visible) {
            Ok(gpu) => self.gpu = Some(gpu),
            Err(e) => {
                self.fail(event_loop, e);
                return;
            }
        }
        self.gpu_counter = crate::scroll::GpuCounter::new();
        self.refresh = window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map_or(self.refresh, |v| v as f64 / 1000.);
        crate::scroll::pacing::set_refresh_millihertz((self.refresh * 1000.) as u32);
        self.window = Some(window);
        self.window.as_ref().unwrap().request_redraw();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested if self.idle.is_none() => {
                if let Err(error) = self.frame(event_loop) {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::CloseRequested => {
                self.fail(event_loop, "Benchmark closed early".into());
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some((start, cpu)) = self.idle {
            if cpu.is_nan() && start.elapsed() >= Duration::from_secs(1) {
                self.idle = Some((Instant::now(), cpu_seconds()));
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    Instant::now() + Duration::from_secs(2),
                ));
                return;
            }
            if !cpu.is_nan() && start.elapsed() >= Duration::from_secs(2) {
                self.idle_cpu = Some((cpu_seconds() - cpu) / start.elapsed().as_secs_f64() * 100.);
                if let Err(error) = self.save() {
                    self.error = Some(error);
                }
                // Winit may call about_to_wait again while draining Exit. Do not
                // save a second time after the renderer has been released.
                self.idle = None;
                if let Some(gpu) = &mut self.gpu {
                    if let Err(error) = gpu.device.poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(Duration::from_secs(5)),
                    }) {
                        self.error = Some(error.to_string());
                    }
                    gpu.surface = None;
                }
                self.gpu = None;
                self.window = None;
                event_loop.exit();
            }
        }
    }
}
fn summary(samples: &[f64]) -> serde_json::Value {
    if samples.is_empty() {
        return serde_json::json!(null);
    }
    let mut s = samples.to_vec();
    s.sort_by(f64::total_cmp);
    let at = |p: f64| s[((s.len() - 1) as f64 * p).ceil() as usize];
    serde_json::json!({"p50_ms":at(0.5),"p99_ms":at(0.99),"max_ms":at(1.),"samples":s.len()})
}
#[cfg(windows)]
fn cpu_seconds() -> f64 {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let mut times = [FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    }; 4];
    // SAFETY: the pseudo-handle is always valid; all four output pointers are live and distinct.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut times[0],
            &mut times[1],
            &mut times[2],
            &mut times[3],
        );
    }
    let ticks = |v: FILETIME| ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64;
    (ticks(times[2]) + ticks(times[3])) as f64 / 10_000_000.
}
#[cfg(not(windows))]
fn cpu_seconds() -> f64 {
    0.
}
#[cfg(windows)]
fn memory() -> serde_json::Value {
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
        Threading::GetCurrentProcess,
    };
    // SAFETY: POD output buffer is initialized and sized exactly as required by PSAPI.
    unsafe {
        let mut m: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        m.cb = std::mem::size_of_val(&m) as u32;
        let ok = GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            m.cb,
        );
        if ok == 0 {
            serde_json::Value::Null
        } else {
            serde_json::json!({"working_set_bytes":m.WorkingSetSize,"private_bytes":m.PrivateUsage})
        }
    }
}
#[cfg(not(windows))]
fn memory() -> serde_json::Value {
    serde_json::Value::Null
}
