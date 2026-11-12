//! Headless benchmarks: never opens a window or plays a signal. DSP inputs are
//! generated in memory; device tests are explicit CLI options, never cargo test.
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    use sticktalk_media::{process_usage, voice::*};
    let args: Vec<_> = std::env::args().collect();
    let seconds = args
        .windows(2)
        .find(|a| a[0] == "--seconds")
        .and_then(|a| a[1].parse::<u64>().ok())
        .unwrap_or(10)
        .clamp(1, 120);
    if args.iter().any(|a| a == "--list") {
        println!("Audio: {:#?}", sticktalk_media::audio_devices()?);
        println!("Cameras: {:#?}", sticktalk_media::camera_devices()?);
        return Ok(());
    }
    if args.iter().any(|a| a == "--watch-smoke") {
        let watcher = sticktalk_media::AudioDeviceWatcher::start()?;
        drop(watcher);
        println!("Notification registration/unregistration: OK");
        return Ok(());
    }
    if args.iter().any(|a| a == "--video-fixture") {
        let options = sticktalk_media::CameraOptions::default();
        let mut encoder = sticktalk_media::CameraFrameEncoder::new(options)?;
        let mut decoder = sticktalk_media::Decoder::new()?;
        let mut nv12 = vec![128u8; 1280 * 720 * 3 / 2];
        for n in 0..30 {
            let frame = encoder.encode_nv12(&nv12, 1280, n * 333333)?;
            let _ = decoder.decode(frame)?;
        }
        let before = process_usage()?;
        let start = Instant::now();
        let mut frames = 0;
        for n in 0..seconds * 30 {
            // Small moving luma bar; no camera, visible window or saved pixels.
            nv12[..1280 * 720].fill(32);
            let col = (n as usize * 16) % 1200;
            for y in 0..720 {
                nv12[y * 1280 + col..y * 1280 + col + 80].fill(180);
            }
            let frame = encoder.encode_nv12(&nv12, 1280, (n as i64 + 30) * 333333)?;
            if decoder.decode(frame)?.is_empty() {
                return Err("fixture decoder produced no texture".into());
            }
            frames += 1;
            let deadline = start + Duration::from_secs_f64((n + 1) as f64 / 30.0);
            if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
        }
        let after = process_usage()?;
        print_usage(
            "Synthetic NV12 720p30 upload + HW encode/decode (NOT camera capture)",
            before,
            after,
            start.elapsed().as_secs_f64(),
            frames,
        );
        return Ok(());
    }
    if args.iter().any(|a| a == "--camera" || a == "--wasapi-idle")
        && !args.iter().any(|a| a == "--devices-already-permitted")
    {
        return Err("real device benchmark disabled: use fake sources; --devices-already-permitted requires prior user/device permission (never requests consent)".into());
    }
    if args.iter().any(|a| a == "--wasapi-idle") {
        let mut call =
            sticktalk_media::VoiceSession::start(Default::default(), Default::default())?;
        call.set_muted(true)?;
        let start = Instant::now();
        let before = process_usage()?;
        let mut packets = 0;
        while start.elapsed() < Duration::from_secs(seconds) {
            call.recv_timeout(Duration::from_secs(2))?;
            packets += 1;
        }
        let after = process_usage()?;
        let elapsed = start.elapsed().as_secs_f64();
        call.stop()?;
        print_usage(
            "WASAPI muted call (no remotes; silent playback)",
            before,
            after,
            elapsed,
            packets,
        );
        return Ok(());
    }
    if args.iter().any(|a| a == "--process-loopback-self") {
        let mut audio = sticktalk_media::LoopbackSession::start(
            sticktalk_media::LoopbackSource::Process {
                pid: std::process::id(),
            },
            96000,
        )?;
        let start = Instant::now();
        let mut packets = 0;
        while start.elapsed() < Duration::from_secs(seconds) {
            match audio.recv_timeout(Duration::from_millis(500)) {
                Ok(_) => packets += 1,
                Err(sticktalk_media::Error::Timeout(_)) => (),
                Err(e) => return Err(e.into()),
            }
        }
        audio.stop()?;
        println!("Process loopback activation/stop OK, packets={packets} (self emits no audio)");
        return Ok(());
    }
    if args.iter().any(|a| a == "--system-loopback") {
        let mut audio = sticktalk_media::LoopbackSession::start(
            sticktalk_media::LoopbackSource::System {
                output_device: None,
            },
            96000,
        )?;
        let start = Instant::now();
        let mut packets = 0;
        while start.elapsed() < Duration::from_secs(seconds) {
            match audio.recv_timeout(Duration::from_millis(500)) {
                Ok(_) => packets += 1,
                Err(sticktalk_media::Error::Timeout(_)) => (),
                Err(e) => return Err(e.into()),
            }
        }
        audio.stop()?;
        println!("System loopback activation/stop OK, packets={packets}; samples discarded");
        return Ok(());
    }
    if args.iter().any(|a| a == "--camera") {
        let devices = sticktalk_media::camera_devices()?;
        let Some(device) = devices.into_iter().next() else {
            println!("Camera: NOT MEASURED (MF enumeration empty)");
            return Ok(());
        };
        println!("Camera device: {}", device.name);
        let mut camera = sticktalk_media::CameraSession::start(device, Default::default())?;
        for _ in 0..30 {
            camera.recv_timeout(Duration::from_secs(3))?;
        }
        let start = Instant::now();
        let before = process_usage()?;
        let mut frames = 0;
        while start.elapsed() < Duration::from_secs(seconds) {
            camera.recv_timeout(Duration::from_secs(3))?;
            frames += 1;
        }
        let after = process_usage()?;
        let elapsed = start.elapsed().as_secs_f64();
        camera.stop()?;
        print_usage(
            "Camera 720p30 hardware H264",
            before,
            after,
            elapsed,
            frames,
        );
        return Ok(());
    }
    for talking in [true, false] {
        let mut processor = VoiceProcessor::new(VoiceOptions {
            mode: InputMode::PushToTalk,
            noise_mode: NoiseSuppressionMode::Standard,
            ..Default::default()
        })?;
        processor.set_push_to_talk(talking);
        let mut mixer = VoiceMixer::new(2)?;
        mixer.add_remote(1)?;
        let mut bytes = 0;
        let mut packets = 0;
        let before = process_usage()?;
        let start = Instant::now();
        for tick in 0..seconds as usize * 100 {
            let pcm = if talking { signal(tick) } else { [0.0; TICK] };
            let playback = mixer.render()?;
            processor.render(&playback)?;
            if let Some(packet) = processor.capture(&pcm)? {
                bytes += packet.data.len();
                packets += 1;
                mixer.push(1, packet)?;
            }
            let deadline = start + Duration::from_millis((tick as u64 + 1) * 10);
            if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
        }
        let after = process_usage()?;
        print_usage(
            if talking {
                "Synthetic talk DSP + 1 remote loopback (AEC3/AGC2/RNNoise)"
            } else {
                "Silent DSP + 1 remote loopback (AEC3/AGC2/RNNoise)"
            },
            before,
            after,
            start.elapsed().as_secs_f64(),
            packets,
        );
        println!(
            "Opus bytes={bytes}, rate={:.3} kbit/s",
            bytes as f64 * 8.0 / start.elapsed().as_secs_f64() / 1000.0
        );
    }
    digital_latency()?;
    let before = process_usage()?;
    let start = Instant::now();
    thread::sleep(Duration::from_secs(seconds));
    let after = process_usage()?;
    print_usage(
        "Outside call (all engines dropped)",
        before,
        after,
        start.elapsed().as_secs_f64(),
        0,
    );
    Ok(())
}
#[cfg(windows)]
fn print_usage(
    label: &str,
    before: sticktalk_media::ProcessUsage,
    after: sticktalk_media::ProcessUsage,
    seconds: f64,
    frames: u64,
) {
    let one = (after.cpu_seconds - before.cpu_seconds) / seconds * 100.0;
    println!("{label}: seconds={seconds:.3}, CPU machine={:.4}% / one-core={one:.4}%, RSS={:.2} MiB, delivered={frames}, rate={:.2}/s",one/after.logical_cpus as f64,after.rss_bytes as f64/1048576.0,frames as f64/seconds);
}
#[cfg(windows)]
fn signal(tick: usize) -> [f32; 480] {
    std::array::from_fn(|i| {
        let t = (tick * 480 + i) as f32 / 48000.0;
        let envelope = (t * 7.0).sin().max(0.0) * 0.6 + 0.2;
        let f = 120.0 + (t * 3.0).sin() * 30.0;
        envelope
            * ((t * f * std::f32::consts::TAU).sin() * 0.12
                + (t * f * 2.0 * std::f32::consts::TAU).sin() * 0.04
                + (t * 1733.0).sin() * 0.005)
    })
}
#[cfg(windows)]
fn digital_latency() -> Result<(), Box<dyn std::error::Error>> {
    use sticktalk_media::voice::*;
    for ns in [false, true] {
        let mut processor = VoiceProcessor::new(VoiceOptions {
            mode: InputMode::PushToTalk,
            noise_suppression: ns,
            noise_mode: NoiseSuppressionMode::Standard,
            echo_cancellation: false,
            automatic_gain: false,
            ..Default::default()
        })?;
        processor.set_push_to_talk(true);
        let mut mixer = VoiceMixer::new(2)?;
        mixer.add_remote(1)?;
        let mut input = Vec::new();
        let mut output = Vec::new();
        for tick in 0..160 {
            let pcm = if (40..65).contains(&tick) {
                signal(tick)
            } else {
                [0.0; 480]
            };
            input.extend_from_slice(&pcm);
            if let Some(packet) = processor.capture(&pcm)? {
                mixer.push(1, packet)?;
            }
            output.extend_from_slice(&mixer.render()?);
        }
        let mut best = (0, 0.0f64);
        for lag in 0..9600 {
            let correlation: f64 = input[40 * 480..65 * 480]
                .iter()
                .zip(&output[40 * 480 + lag..65 * 480 + lag])
                .map(|(a, b)| f64::from(*a) * f64::from(*b))
                .sum();
            if correlation > best.1 {
                best = (lag, correlation);
            }
        }
        println!("Digital PCM->20ms Opus->40ms jitter->PCM correlation: RNNoise={ns}, delay={:.3} ms ({} samples); NO WASAPI/acoustics/network, not physical mouth-to-ear",best.0 as f64/48.0,best.0);
    }
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows benchmark only");
}
