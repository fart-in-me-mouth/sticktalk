//! All-inputs-in-memory/file voice qualification. Never opens audio devices.
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use sticktalk_media::voice::*;
    let args: Vec<_> = std::env::args().collect();
    let seconds = args
        .windows(2)
        .find(|a| a[0] == "--seconds")
        .and_then(|a| a[1].parse::<usize>().ok())
        .unwrap_or(10)
        .clamp(2, 60);
    println!("AUTO probe: {:?}", benchmark_noise_quality());
    for mode in [
        NoiseSuppressionMode::Off,
        NoiseSuppressionMode::Standard,
        NoiseSuppressionMode::HighQuality,
    ] {
        for talking in [true, false] {
            run(mode, false, talking, seconds)?;
        }
    }
    run(NoiseSuppressionMode::Off, true, true, seconds)?;
    if !args.iter().any(|s| s == "--cpu-only") {
        for mode in [
            NoiseSuppressionMode::Off,
            NoiseSuppressionMode::Standard,
            NoiseSuppressionMode::HighQuality,
        ] {
            latency(mode)?;
        }
    }
    if let Some(path) = args
        .windows(2)
        .find(|a| a[0] == "--clean-wav")
        .map(|a| &a[1])
    {
        quality(path)?;
    }
    let before = sticktalk_media::process_usage()?;
    let start = std::time::Instant::now();
    std::thread::sleep(std::time::Duration::from_secs(5));
    let after = sticktalk_media::process_usage()?;
    print(
        "all processors dropped",
        before,
        after,
        start.elapsed().as_secs_f64(),
    );
    Ok(())
}
#[cfg(windows)]
fn waveform(tick: usize) -> [f32; 480] {
    std::array::from_fn(|i| {
        let t = (tick * 480 + i) as f32 / 48000.0;
        let env = (t * 7.0).sin().max(0.0) * 0.6 + 0.2;
        let f = 120.0 + (t * 3.0).sin() * 30.0;
        env * ((t * f * std::f32::consts::TAU).sin() * 0.12
            + (t * f * 2.0 * std::f32::consts::TAU).sin() * 0.04
            + (t * 1733.0).sin() * 0.005)
    })
}
#[cfg(windows)]
fn options(
    mode: sticktalk_media::voice::NoiseSuppressionMode,
    studio: bool,
) -> sticktalk_media::voice::VoiceOptions {
    use sticktalk_media::voice::*;
    let mut options = if studio {
        VoiceOptions::studio()
    } else {
        VoiceOptions {
            packet_duration_ms: if std::env::args().any(|s| s == "--ten-ms") {
                10
            } else {
                20
            },
            noise_mode: mode,
            mode: InputMode::PushToTalk,
            automatic_sensitivity: false,
            ..Default::default()
        }
    };
    options.packet_duration_ms = if std::env::args().any(|s| s == "--ten-ms") {
        10
    } else {
        20
    };
    options
}
#[cfg(windows)]
fn run(
    mode: sticktalk_media::voice::NoiseSuppressionMode,
    studio: bool,
    talking: bool,
    seconds: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};
    use sticktalk_media::{process_usage, voice::*};
    let mut processor = VoiceProcessor::new(options(mode, studio))?;
    // Open PTT in both runs: silent idle includes all DSP and Opus DTX work,
    // not the cheaper explicitly-muted gate.
    processor.set_push_to_talk(true);
    let mut mixer = VoiceMixer::new(1)?;
    mixer.add_stereo_remote(1)?;
    // Pre-encode an independent far-end signal; its encoder CPU is excluded.
    // Looping the near-end back as its own AEC reference would let the AEC
    // cancel the benchmark stimulus and understate denoiser CPU.
    let mut far = OpusEncoder::new(studio, if studio { 192000 } else { 64000 })?;
    let mut far_packets = Vec::new();
    let packet_ticks = usize::from(options(mode, studio).packet_duration_ms / 10);
    let packet_samples = TICK * packet_ticks;
    for packet in 0..seconds * 100 / packet_ticks {
        let mono: Vec<_> = (0..packet_samples)
            .map(|i| {
                if talking {
                    let t = (packet * packet_samples + i) as f32 / 48000.0;
                    (t * std::f32::consts::TAU * 243.0).sin() * 0.08
                        + (t * std::f32::consts::TAU * 487.0).sin() * 0.02
                } else {
                    0.0
                }
            })
            .collect();
        let pcm = if studio {
            mono.into_iter().flat_map(|s| [s, s * 0.8]).collect()
        } else {
            mono
        };
        far_packets.push(far.encode(&pcm, (packet * packet_samples) as u64)?);
    }
    let mut echo = std::collections::VecDeque::from(vec![[0.0; TICK]; 5]);
    let mut bytes = 0usize;
    let before = process_usage()?;
    let start = Instant::now();
    for tick in 0..seconds * 100 {
        let mut pcm = if talking { waveform(tick) } else { [0.0; TICK] };
        if tick % packet_ticks == packet_ticks - 1 {
            mixer.push(1, far_packets[tick / packet_ticks].clone())?;
        }
        let playback = mixer.render()?;
        processor.render(&playback)?;
        echo.push_back(playback);
        let previous = echo.pop_front().unwrap();
        for (sample, echo) in pcm.iter_mut().zip(previous) {
            *sample += echo * 0.2;
        }
        let packet = if studio {
            processor.capture_stereo(&std::array::from_fn(|i| pcm[i / 2]))?
        } else {
            processor.capture(&pcm)?
        };
        if let Some(packet) = packet {
            bytes += packet.data.len();
        }
        let deadline = start + Duration::from_millis((tick as u64 + 1) * 10);
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    let after = process_usage()?;
    let wall = start.elapsed().as_secs_f64();
    print(
        &format!(
            "mode={mode:?}, studio={studio}, talking={talking}, bitrate={:.3} kbit/s",
            bytes as f64 * 8.0 / wall / 1000.0
        ),
        before,
        after,
        wall,
    );
    println!(
        "playout: {:?}",
        mixer.statistics(1).map(|s| (
            s.target_delay_ms,
            s.current_buffer_size_ms,
            s.packets_awaiting_decode
        ))
    );
    Ok(())
}
#[cfg(windows)]
fn print(
    label: &str,
    b: sticktalk_media::ProcessUsage,
    a: sticktalk_media::ProcessUsage,
    wall: f64,
) {
    let core = (a.cpu_seconds - b.cpu_seconds) / wall * 100.0;
    println!(
        "{label}: seconds={wall:.3}, CPU machine={:.4}% / one-core={core:.4}%, RSS={:.2} MiB, private={:.2} MiB",
        core / a.logical_cpus as f64,
        a.rss_bytes as f64 / 1048576.0,
        a.private_bytes as f64 / 1048576.0
    );
}
#[cfg(windows)]
fn latency(
    mode: sticktalk_media::voice::NoiseSuppressionMode,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};
    use sticktalk_media::voice::*;
    let mut p = VoiceProcessor::new(options(mode, false))?;
    p.set_push_to_talk(true);
    let mut mixer = VoiceMixer::new(1)?;
    mixer.add_remote(1)?;
    let mut input = Vec::new();
    let mut output = Vec::new();
    let start = Instant::now();
    for tick in 0..160 {
        let pcm = if (40..65).contains(&tick) {
            waveform(tick)
        } else {
            [0.0; 480]
        };
        input.extend_from_slice(&pcm);
        if let Some(packet) = p.capture(&pcm)? {
            mixer.push(1, packet)?;
        }
        output.extend_from_slice(&mixer.render()?);
        if let Some(wait) = (start + Duration::from_millis((tick as u64 + 1) * 10))
            .checked_duration_since(Instant::now())
        {
            std::thread::sleep(wait);
        }
    }
    let (lag, correlation) = correlate(&input, &output, 40 * 480, 65 * 480, 9600);
    if correlation <= 0.0 {
        return Err("digital playout produced no correlated signal".into());
    }
    println!("Digital PCM->Opus->adaptive playout->PCM: mode={mode:?}, correlation delay={:.3} ms; EXCLUDES WASAPI/acoustics/network",lag as f64/48.0);
    Ok(())
}
#[cfg(windows)]
fn correlate(input: &[f32], output: &[f32], start: usize, end: usize, max: usize) -> (usize, f64) {
    let mut best = (0, 0.0);
    for lag in (0..max).step_by(4) {
        let sum: f64 = (start..end)
            .step_by(4)
            .map(|i| f64::from(input[i]) * f64::from(output[i + lag]))
            .sum();
        if sum > best.1 {
            best = (lag, sum);
        }
    }
    let mut precise = (best.0, 0.0);
    for lag in best.0.saturating_sub(4)..(best.0 + 5).min(max) {
        let sum: f64 = (start..end)
            .map(|i| f64::from(input[i]) * f64::from(output[i + lag]))
            .sum();
        if sum > precise.1 {
            precise = (lag, sum);
        }
    }
    precise
}
#[cfg(windows)]
fn quality(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use sticktalk_media::voice::*;
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.sample_rate != 48000
        || spec.channels != 1
        || spec.sample_format != hound::SampleFormat::Int
        || spec.bits_per_sample != 16
    {
        return Err("quality fixture must be mono 48kHz PCM16".into());
    }
    let mut clean: Vec<f32> = reader
        .samples::<i16>()
        .take(48000 * 30)
        .map(|s| s.map(|s| f32::from(s) / 32768.0))
        .collect::<Result<_, _>>()?;
    if clean.len() < 48000 {
        return Err("quality fixture too short".into());
    }
    let n = clean.len() / 480 * 480;
    clean.truncate(n);
    let power = clean.iter().map(|s| s * s).sum::<f32>() / n as f32;
    let amplitude = (3.0 * power).sqrt();
    let mut seed = 42u32;
    let noisy: Vec<_> = clean
        .iter()
        .map(|s| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            s + (seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * amplitude
        })
        .collect();
    let scale = 0.8 / noisy.iter().fold(0.8f32, |m, s| m.max(s.abs()));
    for s in &mut clean {
        *s *= scale;
    }
    let noisy: Vec<_> = noisy.into_iter().map(|s| s * scale).collect();
    for mode in [
        NoiseSuppressionMode::Off,
        NoiseSuppressionMode::Standard,
        NoiseSuppressionMode::HighQuality,
    ] {
        let mut opts = options(mode, false);
        opts.echo_cancellation = false;
        opts.automatic_gain = false; // isolate suppression for the aligned SI-SDR score
        let mut p = VoiceProcessor::new(opts)?;
        p.set_push_to_talk(true);
        let mut output = Vec::with_capacity(n + 9600);
        for frame in noisy.as_chunks::<480>().0 {
            p.capture(frame)?;
            output.extend_from_slice(p.last_processed());
        }
        for _ in 0..20 {
            p.capture(&[0.0; 480])?;
            output.extend_from_slice(p.last_processed());
        }
        let (lag, _) = correlate(&clean, &output, 48000.min(n / 4), n.min(48000 * 3), 9600);
        let start = 24000;
        let end = n - 24000;
        let energy: f64 = clean[start..end]
            .iter()
            .map(|s| f64::from(*s).powi(2))
            .sum();
        let dot: f64 = (start..end)
            .map(|i| f64::from(clean[i]) * f64::from(output[i + lag]))
            .sum();
        let gain = dot / energy;
        let error: f64 = (start..end)
            .map(|i| (f64::from(output[i + lag]) - gain * f64::from(clean[i])).powi(2))
            .sum();
        let score = 10.0 * ((gain * gain * energy) / error.max(1e-20)).log10();
        println!("File enhancement SI-SDR: mode={mode:?}, aligned delay={:.3} ms, score={score:.3} dB; deterministic approximately 0dB additive noise, NOT DNSMOS or a Discord comparison",lag as f64/48.0);
    }
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows qualification only");
}
