//! Standalone capture→encode→decode benchmark (no network or app UI).
//! cargo run -p sticktalk-media --release --example sharebench -- --seconds 30 --resolution 1080 --fps 60
//! --matrix benchmarks 720p30, 1080p60, 1440p60, Source60 sequentially.
//! --no-decode measures sender-only resource use. --readback does one diagnostic
//! CPU RGBA readback AFTER measurement; it is never part of production playback.

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("STICKTALK_MEDIA_TRACE").is_some() {
        crash_trace::install();
    }
    bench::main()
}
#[cfg(not(windows))]
fn main() {
    eprintln!("sharebench requires Windows 10 1903+");
}

#[cfg(windows)]
mod bench {
    use std::{
        io::Write,
        time::{Duration, Instant},
    };
    use sticktalk_media::*;

    pub fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        let mut seconds = 30u64;
        let mut quality = Quality::default();
        let mut bitrate = None;
        let mut matrix = false;
        let mut decode = true;
        let mut readback = false;
        let mut reconfigure = false;
        let mut all_qualities = false;
        let mut cursor = true;
        let mut vbr = false;
        let mut encoder_selection = EncoderSelection::HardwareOnly;
        let mut report = None;
        let mut window_title = None;
        let mut i = 0;
        while i < args.len() {
            let flag = &args[i];
            let value = |index: usize| {
                args.get(index)
                    .ok_or_else(|| format!("missing argument for {flag}"))
            };
            match args[i].as_str() {
                "--seconds" => {
                    i += 1;
                    seconds = value(i)?.parse()?;
                }
                "--fps" => {
                    i += 1;
                    quality.fps = value(i)?.parse::<u32>()?.try_into()?;
                }
                "--resolution" => {
                    i += 1;
                    quality.resolution = match value(i)?.as_str() {
                        "480" => Resolution::P480,
                        "720" => Resolution::P720,
                        "1080" => Resolution::P1080,
                        "1440" => Resolution::P1440,
                        "source" => Resolution::Source,
                        _ => return Err("resolution must be 480/720/1080/1440/source".into()),
                    };
                }
                "--bitrate" => {
                    i += 1;
                    bitrate = Some(value(i)?.parse::<u32>()?);
                }
                "--report" => {
                    i += 1;
                    report = Some(value(i)?.to_owned());
                }
                "--window" => {
                    i += 1;
                    window_title = Some(value(i)?.to_owned());
                }
                "--matrix" => matrix = true,
                "--no-decode" => decode = false,
                "--readback" => readback = true,
                "--reconfigure" => reconfigure = true,
                "--all-qualities" => all_qualities = true,
                "--no-cursor" => cursor = false,
                "--vbr" => vbr = true,
                "--allow-software" => encoder_selection = EncoderSelection::AllowSoftwareFallback,
                "--software" => encoder_selection = EncoderSelection::SoftwareOnly,
                "--list" => {
                    println!("Hardware MFTs: {:#?}", hardware_encoders()?);
                    for source in list_sources()? {
                        println!("{source:?}");
                        if source.kind == SourceKind::Monitor {
                            use windows::{core::PCWSTR, Win32::Graphics::Gdi::*};
                            let wide: Vec<u16> =
                                source.name.encode_utf16().chain(Some(0)).collect();
                            let mut mode = DEVMODEW {
                                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                                ..Default::default()
                            };
                            unsafe {
                                if EnumDisplaySettingsW(
                                    PCWSTR(wide.as_ptr()),
                                    ENUM_CURRENT_SETTINGS,
                                    &mut mode,
                                )
                                .as_bool()
                                {
                                    println!(
                                        "  Display mode: {}x{} @ {} Hz",
                                        mode.dmPelsWidth,
                                        mode.dmPelsHeight,
                                        mode.dmDisplayFrequency
                                    );
                                }
                            }
                        }
                    }
                    return Ok(());
                }
                "--help" => {
                    println!("sharebench [--seconds N] [--resolution 480|720|1080|1440|source] [--fps 15|30|60] [--bitrate BPS] [--matrix] [--no-decode] [--readback] [--reconfigure] [--all-qualities] [--no-cursor] [--vbr] [--allow-software] [--software] [--report PATH] [--window TITLE] [--list]");
                    return Ok(());
                }
                other => return Err(format!("unknown argument {other}").into()),
            }
            i += 1;
        }
        if seconds == 0 || seconds > 3600 {
            return Err("seconds must be 1..3600".into());
        }
        let source = list_sources()?
            .into_iter()
            .find(|s| match &window_title {
                Some(title) => s.kind == SourceKind::Window && s.name.contains(title),
                None => s.primary && s.kind == SourceKind::Monitor,
            })
            .ok_or("no matching capture source")?;
        println!(
            "Source: {} ({}x{}), cursor={cursor}",
            source.name, source.size.width, source.size.height
        );
        println!("Detected hardware encoders: {:#?}", hardware_encoders()?);
        let choices = if matrix {
            vec![
                Quality {
                    fps: FrameRate::Fps30,
                    resolution: Resolution::P720,
                },
                Quality::default(),
                Quality {
                    fps: FrameRate::Fps60,
                    resolution: Resolution::P1440,
                },
                Quality {
                    fps: FrameRate::Fps60,
                    resolution: Resolution::Source,
                },
            ]
        } else if all_qualities {
            [FrameRate::Fps15, FrameRate::Fps30, FrameRate::Fps60]
                .into_iter()
                .flat_map(|fps| {
                    [
                        Resolution::P480,
                        Resolution::P720,
                        Resolution::P1080,
                        Resolution::P1440,
                        Resolution::Source,
                    ]
                    .map(|resolution| Quality { fps, resolution })
                })
                .collect()
        } else {
            vec![quality]
        };
        let mut rows = String::from("| Quality | Size | Seconds | Capture fps (unique WGC) | Encode fps | Decode fps | Mean/p99 encode ms | Drops | Repeats | Actual Mbps | CPU total % / one-core % | RSS / peak MiB | Encoder |\n|---|---|---:|---:|---:|---:|---|---:|---:|---:|---|---|---|\n");
        for quality in choices {
            let options = ShareOptions {
                quality,
                bitrate: bitrate.unwrap_or(quality.recommended_bitrate(source.size)?),
                cursor,
                encoder_selection,
                rate_control: if vbr {
                    RateControl::Vbr
                } else {
                    RateControl::Cbr
                },
                ..ShareOptions::default()
            };
            rows.push_str(&run(
                source.clone(),
                options,
                seconds,
                decode,
                readback,
                reconfigure,
            )?);
        }
        println!("\n{rows}");
        if let Some(path) = report {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?
                .write_all(rows.as_bytes())?;
        }
        Ok(())
    }

    fn run(
        source: Source,
        options: ShareOptions,
        seconds: u64,
        decode: bool,
        readback: bool,
        reconfigure: bool,
    ) -> std::result::Result<String, Box<dyn std::error::Error>> {
        let label = format!(
            "{:?}@{}",
            options.quality.resolution,
            options.quality.fps.get()
        );
        println!(
            "Starting {label}, target {} bps, decode={decode}",
            options.bitrate
        );
        let mut decoder = if decode { Some(Decoder::new()?) } else { None };
        let mut session = ShareSession::start_with_options(source, options.clone())?;
        println!(
            "GPU: {}; encoder: {:?}; actual size: {:?}; quality: {:?}",
            session.gpu_name, session.encoder_kind, session.size, session.effective_quality
        );
        if let Some(notice) = &session.fallback_notice {
            println!("WARNING: {notice}");
        }
        // Two-second warmup initializes the decoder's GPU surface pool and drops
        // initial codec/driver startup costs. Reported wall time excludes it.
        let warmup = Instant::now();
        while warmup.elapsed() < Duration::from_secs(2) {
            let frame = session.recv_timeout(Duration::from_secs(3))?;
            if let Some(d) = &mut decoder {
                let _ = d.decode(frame)?;
            }
        }
        let baseline = session.stats();
        let usage0 = process_usage()?;
        let start = Instant::now();
        let mut latencies = Vec::new();
        let mut decoded = 0u64;
        let mut bytes = 0u64;
        let mut peak = usage0.rss_bytes;
        let mut last_sample = Instant::now();
        let mut last_frame = None;
        let mut change_stage = 0;
        let mut keyframe_requested_at = None;
        let mut keyframe_confirmed = false;
        let mut last_timestamp = -1;
        let mut previous_generation = None;
        let mut max_pts_gap_ms = 0.0f64;
        let mut max_switch_gap_ms = 0.0f64;
        let mut max_delivery_gap_ms = 0.0f64;
        let mut last_delivery = None;
        while start.elapsed() < Duration::from_secs(seconds) {
            let frame = session.recv_timeout(Duration::from_secs(3))?;
            if frame.timestamp <= last_timestamp {
                return Err("non-monotonic encoded timestamps".into());
            }
            if last_timestamp >= 0 {
                let gap = (frame.timestamp - last_timestamp) as f64 / 10_000.0;
                max_pts_gap_ms = max_pts_gap_ms.max(gap);
                if previous_generation != Some(frame.generation) {
                    if !frame.keyframe {
                        return Err("generation switch did not start with IDR".into());
                    }
                    max_switch_gap_ms = max_switch_gap_ms.max(gap);
                    println!(
                        "Generation {}: IDR size={:?}, PTS gap={gap:.3} ms",
                        frame.generation, frame.size
                    );
                }
            }
            let delivery = Instant::now();
            if let Some(last) = last_delivery {
                max_delivery_gap_ms =
                    max_delivery_gap_ms.max(delivery.duration_since(last).as_secs_f64() * 1000.0);
            }
            last_delivery = Some(delivery);
            previous_generation = Some(frame.generation);
            last_timestamp = frame.timestamp;
            if let Some(pts) = keyframe_requested_at {
                if frame.timestamp > pts && frame.keyframe {
                    keyframe_confirmed = true;
                }
            }
            bytes += frame.data.len() as u64;
            latencies.push(frame.encode_latency.as_secs_f64() * 1000.0);
            let elapsed = start.elapsed().as_secs_f64();
            if reconfigure && change_stage == 0 && elapsed > seconds as f64 / 3.0 {
                session.set_quality(
                    Quality {
                        fps: FrameRate::Fps30,
                        resolution: Resolution::P720,
                    },
                    options.bitrate / 2,
                )?;
                session.set_cursor(false);
                change_stage = 1;
            } else if reconfigure && change_stage == 1 && elapsed > seconds as f64 * 2.0 / 3.0 {
                session.set_quality(options.quality, options.bitrate)?;
                session.set_cursor(true);
                session.request_keyframe();
                keyframe_requested_at = Some(frame.timestamp);
                change_stage = 2;
            }
            if let Some(d) = &mut decoder {
                for frame in d.decode(frame)? {
                    decoded += 1;
                    // Retain at most ONE texture/sample lease, never a growing queue.
                    last_frame = Some(frame);
                }
            }
            if last_sample.elapsed() >= Duration::from_secs(1) {
                peak = peak.max(process_usage()?.rss_bytes);
                last_sample = Instant::now();
            }
        }
        let elapsed = start.elapsed().as_secs_f64();
        let usage1 = process_usage()?;
        let stats = session.stats();
        session.stop()?;
        let cpu_core = (usage1.cpu_seconds - usage0.cpu_seconds) / elapsed * 100.0;
        let cpu_total = cpu_core / usage1.logical_cpus as f64;
        peak = peak.max(usage1.rss_bytes);
        let mean = latencies.iter().sum::<f64>() / latencies.len().max(1) as f64;
        latencies.sort_by(f64::total_cmp);
        let p99 = latencies
            .get((latencies.len().saturating_sub(1) as f64 * 0.99).ceil() as usize)
            .copied()
            .unwrap_or(0.0);
        let capture_fps = (stats.captured - baseline.captured) as f64 / elapsed;
        let encode_fps = latencies.len() as f64 / elapsed;
        let decode_fps = decoded as f64 / elapsed;
        let actual = bytes as f64 * 8.0 / elapsed / 1_000_000.0;
        println!("Completed {label}: {elapsed:.3}s, unique capture {capture_fps:.2} fps, encode {encode_fps:.2} fps, decode {decode_fps:.2} fps, latency mean={mean:.3} p99={p99:.3}ms, drops={}, repeats={}, {actual:.3} Mbps", stats.dropped - baseline.dropped, stats.repeated - baseline.repeated);
        println!("CPU {cpu_total:.3}% machine / {cpu_core:.3}% one-core ({} logical CPUs); RSS {:.2}MiB, sampled peak {:.2}MiB, lifetime peak {:.2}MiB; generation changes {}", usage1.logical_cpus, usage1.rss_bytes as f64 / 1048576.0, peak as f64 / 1048576.0, usage1.peak_rss_bytes as f64 / 1048576.0, stats.reconfigurations - baseline.reconfigurations);
        println!("Max media PTS gap={max_pts_gap_ms:.3} ms; switch-only PTS gap={max_switch_gap_ms:.3} ms; max receive gap={max_delivery_gap_ms:.3} ms");
        if reconfigure
            && (!keyframe_confirmed || stats.reconfigurations - baseline.reconfigurations < 2)
        {
            return Err("reconfiguration/keyframe smoke test did not complete".into());
        }
        if readback {
            let d = decoder.as_ref().ok_or("--readback requires decode")?;
            let frame = last_frame.as_ref().ok_or("decoder produced no textures")?;
            let rgba = d.readback_rgba_for_test(frame)?;
            println!(
                "DIAGNOSTIC ONLY: read back {} RGBA bytes, checksum {} (outside measured interval)",
                rgba.len(),
                rgba.iter().map(|b| u64::from(*b)).sum::<u64>()
            );
        }
        Ok(format!("| {label} | {}x{} | {elapsed:.3} | {capture_fps:.2} | {encode_fps:.2} | {decode_fps:.2} | {mean:.3}/{p99:.3} | {} | {} | {actual:.3} | {cpu_total:.3}/{cpu_core:.3} | {:.2}/{:.2} | {:?} |\n", session.size.width, session.size.height, stats.dropped - baseline.dropped, stats.repeated - baseline.repeated, usage1.rss_bytes as f64 / 1048576.0, peak as f64 / 1048576.0, session.encoder_kind))
    }
}

#[cfg(windows)]
#[path = "support/crash_trace.rs"]
mod crash_trace;
