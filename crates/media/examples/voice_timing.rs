//! Device-free impulse-train timing, including the native f32 stereo format seam.
//! Each short chirp is independently matched: no global speech-window alignment.
use std::time::{Duration, Instant};
use sticktalk_media::voice::*;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut qualified = true;
    for mode in [
        NoiseSuppressionMode::Off,
        NoiseSuppressionMode::Standard,
        NoiseSuppressionMode::HighQuality,
    ] {
        for packet_duration_ms in [10, 20] {
            qualified &= run(mode, packet_duration_ms)?;
        }
    }
    if qualified {
        Ok(())
    } else {
        Err("marker coverage failed; distributions are diagnostics, not qualified latency".into())
    }
}
fn marker(i: usize) -> f32 {
    let t = i as f32 / 48000.;
    let phase = std::f32::consts::TAU * (500. * t + 45000. * t * t);
    phase.sin() * (std::f32::consts::PI * i as f32 / 479.).sin().powi(2) * 0.55
}
fn run(
    mode: NoiseSuppressionMode,
    packet_duration_ms: u16,
) -> Result<bool, Box<dyn std::error::Error>> {
    let impaired = std::env::args().any(|a| a == "--impaired");
    let mut pending = Vec::<(usize, AudioPacket)>::new();
    let mut p = VoiceProcessor::new(VoiceOptions {
        packet_duration_ms,
        noise_mode: mode,
        mode: InputMode::PushToTalk,
        ..Default::default()
    })?;
    p.set_push_to_talk(true);
    let mut mixer = VoiceMixer::new(1)?;
    // Native calls use a stereo decoder for both mono voice and Studio, so a
    // negotiated stream can receive either Opus channel layout correctly.
    mixer.add_stereo_remote(1)?;
    let starts: Vec<usize> = (0..16).map(|i| (100 + i * 35) * TICK).collect();
    let mut input = Vec::new();
    let mut output = Vec::new();
    let start = Instant::now();
    for tick in 0..700 {
        pending.sort_by_key(|p| p.0);
        while pending.first().is_some_and(|p| p.0 <= tick) {
            let (delivery, packet) = pending.remove(0);
            mixer.push_at(
                1,
                packet,
                start + Duration::from_millis(delivery as u64 * 10),
            )?;
        }
        // Render first, exactly as WASAPI does: a newly captured block cannot
        // travel backwards in time into a buffer already handed to the device.
        let rendered = mixer.render_stereo()?;
        let mono: [f32; TICK] =
            std::array::from_fn(|i| (rendered[i * 2] + rendered[i * 2 + 1]) * 0.5);
        // This is sender -> remote receiver, not acoustic self-monitoring.
        // Sender's independent far-end reference is silent; feeding the remote
        // receiver's copy back into sender AEC would falsely cancel the marker.
        p.render(&[0.; TICK])?;
        // Float stereo LE is the negotiated shared-mode capture/playout format.
        // Serialize/parse instead of shortcutting format ingress with mono PCM.
        let frame: [f32; TICK * 2] = std::array::from_fn(|i| {
            let n = tick * TICK + i / 2;
            let t = n as f32 / 48000.;
            let bed = (t * std::f32::consts::TAU * 143.).sin() * 0.04
                + (t * std::f32::consts::TAU * 287.).sin() * 0.02;
            let m = starts
                .iter()
                .find_map(|s| (n >= *s && n < *s + TICK).then(|| marker(n - *s)))
                .unwrap_or(0.);
            bed + m
        });
        let bytes: Vec<u8> = frame.iter().flat_map(|s| encode_device_float(*s)).collect();
        let frame = std::array::from_fn(|i| {
            decode_device_float(bytes[i * 4..i * 4 + 4].try_into().unwrap())
        });
        input.extend((0..TICK).map(|i| (frame[i * 2] + frame[i * 2 + 1]) * 0.5));
        if let Some(packet) = p.capture_stereo(&frame)? {
            if !impaired || packet.sequence % 20 != 9 {
                let delay = if !impaired {
                    0
                } else if packet.sequence.is_multiple_of(7) {
                    3
                } else if packet.sequence.is_multiple_of(3) {
                    2
                } else {
                    0
                };
                pending.push((tick + 1 + delay, packet));
                assert!(pending.len() <= 16);
            }
        }
        output.extend(mono.map(|s| decode_device_float(encode_device_float(s))));
        if let Some(wait) = (start + Duration::from_millis((tick + 1) as u64 * 10))
            .checked_duration_since(Instant::now())
        {
            std::thread::sleep(wait);
        }
    }
    let mut lags = Vec::new();
    let mut rejected = 0;
    for &s in &starts {
        let reference = &input[s..s + TICK];
        let score = |lag: usize| {
            let candidate = &output[s + lag..s + lag + TICK];
            let (mut xy, mut xx, mut yy) = (0f64, 0f64, 0f64);
            for i in (0..TICK).step_by(2) {
                let x = f64::from(reference[i]);
                let y = f64::from(candidate[i]);
                xy += x * y;
                xx += x * x;
                yy += y * y;
            }
            xy / (xx * yy).sqrt().max(1e-20)
        };
        let coarse = (0..7200)
            .step_by(4)
            .map(|lag| (lag, score(lag)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let best = (coarse.0.saturating_sub(4)..(coarse.0 + 5).min(7200))
            .map(|lag| (lag, score(lag)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        // Reject weak/absent markers, never quietly publish a selected subset
        // as an accepted latency distribution.
        println!("marker mode={mode:?} packet={packet_duration_ms} start_ms={} delay_ms={:.3} correlation={:.4}",s/48,best.0 as f64/48.,best.1);
        if best.1 >= 0.45 {
            lags.push(best.0 as f64 / 48.);
        } else {
            rejected += 1;
        }
    }
    lags.sort_by(f64::total_cmp);
    println!("LINK impaired={impaired} (impaired: deterministic 5% loss, 0/20/30 ms jitter); playout={:?}, repair={:?}",mixer.statistics(1),mixer.repair_statistics(1));
    if !lags.is_empty() {
        println!("TIMING {mode:?} {packet_duration_ms}ms: matched={}/{} rejected={rejected} qualified={} p50={:.3} p95={:.3} max={:.3} ms; DIGITAL ONLY, no devices/network/acoustics",lags.len(),starts.len(),rejected==0,lags[lags.len()/2],lags[(lags.len()*95).div_ceil(100).saturating_sub(1)],lags[lags.len()-1]);
    }
    Ok(rejected == 0)
}
