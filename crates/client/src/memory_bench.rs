//! Opt-in native resource probe. No timers or allocations exist in ordinary runs.
//! Uses the real daemon, renderer, swapchain, subscriptions, and bounded history.
use crate::{
    app::{App, Msg, Screen},
    backend::Request,
    features::Picker,
};
use iced::{widget::operation, Task};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy)]
pub enum Step {
    Start,
    Sample(&'static str),
    Scroll,
    ScrollView(u8),
    Emoji,
    Gif,
    ClosePickers,
    Finish,
}

pub struct State {
    output: PathBuf,
    started: Instant,
    cpu: u64,
    samples: Vec<serde_json::Value>,
    pub scrolling: bool,
}

impl State {
    pub fn requested() -> Option<Self> {
        let args: Vec<_> = std::env::args().collect();
        if !crate::agent_window::enabled() || !args.iter().any(|a| a == "--mock") {
            return None;
        }
        let output = args
            .iter()
            .position(|a| a == "--memory-bench")
            .and_then(|i| args.get(i + 1))?;
        Some(Self {
            output: output.into(),
            started: Instant::now(),
            cpu: 0,
            samples: Vec::new(),
            scrolling: false,
        })
    }
}

pub fn delay(step: Step, millis: u64) -> Task<Msg> {
    Task::perform(
        async move {
            tokio::time::sleep(Duration::from_millis(millis)).await;
            step
        },
        Msg::MemoryBench,
    )
}

pub fn update(app: &mut App, step: Step) -> Task<Msg> {
    let Some(mut probe) = app.memory_bench.take() else {
        return Task::none();
    };
    let task = match step {
        Step::Start => {
            probe.cpu = cpu_ticks();
            probe.started = Instant::now();
            delay(Step::Sample("idle"), 60_000)
        }
        Step::Sample(label) => {
            // Capture counters before VM enumeration allocates scratch buffers.
            let elapsed = probe.started.elapsed().as_secs_f64();
            let cpu = cpu_ticks().saturating_sub(probe.cpu) as f64 / 10_000_000.;
            let mut sample = snapshot();
            sample["scenario"] = label.into();
            sample["seconds"] = elapsed.into();
            sample["cpu_percent_one_core"] = (cpu / elapsed * 100.).into();
            sample["messages_seen"] = app.state.history_seen.into();
            sample["retained_messages"] = app.state.model.retained_count().into();
            sample["retained_message_bytes"] = app.state.model.retained_bytes().into();
            sample["audio_active"] = app.calls.config.is_some().into();
            sample["image_cache"] = app.images.metrics();
            probe.samples.push(sample);
            let report = serde_json::json!({
                "method": "release native iced daemon, wgpu/DX12 swapchain, 1280x800 secondary noactivate window; mock transport; 60s idle samples; no working-set eviction",
                "allocator": if cfg!(feature="compact-allocator") { "mimalloc" } else { "system" },
                "samples": probe.samples,
            });
            if let Err(error) =
                std::fs::write(&probe.output, serde_json::to_vec_pretty(&report).unwrap())
            {
                eprintln!("Resource report: {error}");
            }
            match label {
                "idle" => {
                    probe.scrolling = true;
                    delay(Step::Scroll, 100)
                }
                "after-2000-messages" => delay(Step::Emoji, 100),
                "emoji-open" => delay(Step::Gif, 100),
                "gif-open" => delay(Step::ClosePickers, 100),
                _ => delay(Step::Finish, 100),
            }
        }
        Step::Scroll => {
            if app.state.history_seen >= 2000 || !app.state.history_more {
                probe.scrolling = false;
                delay(Step::ScrollView(5), 10_000)
            } else if let Screen::Channel(channel) = app.state.screen {
                app.state.at_present = false;
                app.state.loading = true;
                app.request(Request::History {
                    seek: Some(app.state.history_seek),
                    channel,
                    before: app.state.before,
                    generation: app.state.generation,
                });
                Task::none()
            } else {
                delay(Step::Finish, 0)
            }
        }
        Step::ScrollView(frame) => {
            if frame == 5 {
                probe.cpu = cpu_ticks();
                probe.started = Instant::now();
                delay(Step::Sample("after-2000-messages"), 60_000)
            } else {
                app.state.at_present = false;
                app.scroll_y = (4 - frame) as f32 * app.viewport_h * 0.3;
                let scroll = operation::scroll_to(
                    "messages",
                    iced::widget::scrollable::AbsoluteOffset {
                        x: 0.,
                        y: app.scroll_y,
                    },
                );
                let next = if frame == 4 {
                    Step::Scroll
                } else {
                    Step::ScrollView(frame + 1)
                };
                Task::batch([scroll, delay(next, 50)])
            }
        }
        Step::Emoji | Step::Gif => {
            if matches!(step, Step::Emoji) {
                app.features.picker = Picker::Emoji;
            } else {
                crate::features::fixture(app, "gif-picker");
            }
            probe.cpu = cpu_ticks();
            probe.started = Instant::now();
            delay(
                Step::Sample(if matches!(step, Step::Emoji) {
                    "emoji-open"
                } else {
                    "gif-open"
                }),
                60_000,
            )
        }
        Step::ClosePickers => {
            app.features.picker = Picker::None;
            probe.cpu = cpu_ticks();
            probe.started = Instant::now();
            delay(Step::Sample("pickers-closed"), 60_000)
        }
        Step::Finish => return iced::exit(),
    };
    app.memory_bench = Some(probe);
    task
}

#[cfg(windows)]
fn cpu_ticks() -> u64 {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let mut times = [FILETIME::default(); 4];
    // SAFETY: four writable FILETIMEs and our own process pseudo-handle.
    unsafe {
        let [created, exited, kernel, user] = &mut times;
        GetProcessTimes(GetCurrentProcess(), created, exited, kernel, user);
    }
    times[2..]
        .iter()
        .map(|t| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64)
        .sum()
}
#[cfg(not(windows))]
fn cpu_ticks() -> u64 {
    0
}

#[cfg(windows)]
fn snapshot() -> serde_json::Value {
    use std::{collections::BTreeMap, ffi::c_void};
    use windows_sys::Win32::System::{ProcessStatus::*, Threading::GetCurrentProcess};
    // Local VirtualQuery declaration keeps this diagnostic independent of extra Windows features.
    #[repr(C)]
    #[derive(Default)]
    struct Region {
        base: *mut c_void,
        allocation_base: *mut c_void,
        allocation_protect: u32,
        partition: u16,
        size: usize,
        state: u32,
        protect: u32,
        kind: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualQuery(address: *const c_void, region: *mut Region, length: usize) -> usize;
    }
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: EX starts with PROCESS_MEMORY_COUNTERS; exact writable size supplied.
    unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            counters.cb,
        );
    }
    let mut regions: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut pages = vec![PSAPI_WORKING_SET_EX_INFORMATION::default(); 4096];
    let mut address = 0usize;
    loop {
        let mut region = Region::default();
        // SAFETY: queries address metadata, never dereferences the queried address.
        if unsafe {
            VirtualQuery(
                address as *const _,
                &mut region,
                std::mem::size_of::<Region>(),
            )
        } == 0
            || region.size == 0
        {
            break;
        }
        if region.state == 0x1000 {
            let mut filename = [0u16; 1024];
            // SAFETY: own process handle; output capacity matches array.
            let length = unsafe {
                K32GetMappedFileNameW(
                    GetCurrentProcess(),
                    region.base,
                    filename.as_mut_ptr(),
                    filename.len() as u32,
                )
            };
            let kind = match region.kind {
                0x1000000 => "image",
                0x40000 => "mapped",
                _ => "private",
            };
            let label = if length == 0 {
                kind.to_owned()
            } else {
                format!(
                    "{kind}:{}",
                    String::from_utf16_lossy(&filename[..length as usize])
                )
            };
            let tally = regions.entry(label).or_default();
            tally.0 += region.size;
            for offset in (0..region.size).step_by(4096 * pages.len()) {
                let n = ((region.size - offset) / 4096).min(pages.len());
                for (i, page) in pages[..n].iter_mut().enumerate() {
                    page.VirtualAddress = (region.base as usize + offset + i * 4096) as *mut _;
                    page.VirtualAttributes = Default::default();
                }
                // SAFETY: contiguous initialized PSAPI array; byte count is exact.
                if unsafe {
                    K32QueryWorkingSetEx(
                        GetCurrentProcess(),
                        pages.as_mut_ptr().cast(),
                        (n * std::mem::size_of::<PSAPI_WORKING_SET_EX_INFORMATION>()) as u32,
                    )
                } != 0
                {
                    for page in &pages[..n] {
                        // SAFETY: Flags is the documented raw representation of this union.
                        let flags = unsafe { page.VirtualAttributes.Flags };
                        if flags & 1 != 0 {
                            tally.1 += 4096;
                            if flags & (1 << 15) == 0 {
                                tally.2 += 4096;
                            }
                        }
                    }
                }
            }
        }
        let Some(next) = (region.base as usize).checked_add(region.size) else {
            break;
        };
        if next <= address {
            break;
        }
        address = next;
    }
    let maps: Vec<_> = regions.into_iter().map(|(name,(commit,resident,private_resident))| serde_json::json!({"name":name,"committed_bytes":commit,"resident_bytes":resident,"private_resident_bytes":private_resident})).collect();
    let font_faces = iced_tiny_skia::graphics::text::font_system()
        .write()
        .unwrap()
        .raw()
        .db()
        .faces()
        .count();
    serde_json::json!({"working_set_bytes":counters.WorkingSetSize,"private_bytes":counters.PrivateUsage,"page_faults":counters.PageFaultCount,"font_faces":font_faces,"vm_regions":maps})
}
#[cfg(not(windows))]
fn snapshot() -> serde_json::Value {
    serde_json::json!({"unsupported":true})
}
