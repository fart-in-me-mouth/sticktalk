//! Real-time FILE speech -> DSP -> Opus -> adaptive playout. No OS audio.
use std::time::{Duration, Instant};
use sticktalk_media::voice::*;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = std::env::args()
        .nth(1)
        .ok_or("usage: voice_file_latency <mono-48k-PCM16.wav>")?;
    let mut reader = hound::WavReader::open(file)?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != 48000
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err("expected mono 48k PCM16".into());
    }
    let speech = reader
        .samples::<i16>()
        .take(4 * 48000)
        .map(|s| s.map(|s| f32::from(s) / 32768.))
        .collect::<Result<Vec<_>, _>>()?;
    let mut input = vec![0.; 50 * TICK];
    input.extend_from_slice(&speech[..speech.len() / TICK * TICK]);
    input.extend(vec![0.; 50 * TICK]);
    for mode in [
        NoiseSuppressionMode::Off,
        NoiseSuppressionMode::Standard,
        NoiseSuppressionMode::HighQuality,
    ] {
        let mut processor = VoiceProcessor::new(VoiceOptions {
            packet_duration_ms: 10,
            noise_mode: mode,
            mode: InputMode::PushToTalk,
            automatic_sensitivity: false,
            ..Default::default()
        })?;
        processor.set_push_to_talk(true);
        let mut mixer = VoiceMixer::new(1)?;
        mixer.add_remote(1)?;
        let mut output = vec![];
        let begin = Instant::now();
        for (tick, pcm) in input.as_chunks::<TICK>().0.iter().enumerate() {
            if let Some(packet) = processor.capture(pcm)? {
                mixer.push(1, packet)?;
            }
            output.extend_from_slice(&mixer.render()?);
            if let Some(wait) = (begin + Duration::from_millis((tick as u64 + 1) * 10))
                .checked_duration_since(Instant::now())
            {
                std::thread::sleep(wait);
            }
        }
        let start = 50 * TICK;
        let end = input.len() - 50 * TICK;
        // Adaptive playout intentionally time-stretches. One fixed lag across
        // an entire utterance is not a valid estimator when delay changes.
        // Report short-window delay distribution AND uncorrelated coverage;
        // never silently select only the easiest speech windows.
        let mut delays = vec![];
        let mut eligible = 0;
        for start in (start..end.saturating_sub(1920)).step_by(960) {
            let power = input[start..start + 1920]
                .iter()
                .map(|s| s * s)
                .sum::<f32>()
                / 1920.;
            if power < 1e-6 {
                continue;
            }
            eligible += 1;
            let mut best = (0, 0f64);
            for lag in (0..9600).step_by(4) {
                let (mut xy, mut xx, mut yy) = (0., 0., 0.);
                for i in (start..start + 1920).step_by(8) {
                    let x = f64::from(input[i]);
                    let y = f64::from(output[i + lag]);
                    xy += x * y;
                    xx += x * x;
                    yy += y * y;
                }
                let correlation = xy / (xx * yy).sqrt().max(1e-12);
                if correlation > best.1 {
                    best = (lag, correlation);
                }
            }
            if best.1 >= 0.5 {
                delays.push(best.0);
            }
        }
        delays.sort_unstable();
        if delays.len() < 10 {
            return Err("too few correlated speech windows".into());
        }
        println!("FILE speech digital loopback: mode={mode:?}, correlated={}/{eligible} voiced windows, delay p50={:.3} ms, p95={:.3} ms, max={:.3} ms, playout={:?}; excludes devices/acoustics/network",delays.len(),delays[delays.len()/2] as f64/48.,delays[(delays.len()-1)*95/100] as f64/48.,delays[delays.len()-1] as f64/48.,mixer.statistics(1));
        if delays.len() * 100 < eligible * 70 {
            return Err("speech correlation coverage below 70%; latency not qualified".into());
        }
    }
    Ok(())
}
