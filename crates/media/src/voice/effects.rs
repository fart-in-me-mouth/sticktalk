//! Native WebRTC APM and pinned DeepFilterNet3 inference.
use super::{NoiseSuppressionMode, VoiceOptions, TICK};
use crate::{Error, Result};
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};
use webrtc_audio_processing::{config::*, Processor};
fn err(e: impl std::fmt::Display) -> Error {
    Error::Platform(format!("voice effects: {e:#}"))
}
/// Separate native APM stages put external suppression BETWEEN AEC/HPF and AGC2.
pub(super) struct NativeApm {
    echo: Processor,
    gain: Processor,
    gain_enabled: bool,
}
impl NativeApm {
    pub fn new(o: &VoiceOptions) -> Result<Self> {
        if !webrtc_audio_processing::verify_native_layout() {
            return Err(err("native APM ABI layout mismatch"));
        }
        let mut apm = Self {
            echo: Processor::new(48000).map_err(err)?,
            gain: Processor::new(48000).map_err(err)?,
            gain_enabled: false,
        };
        apm.configure(o);
        Ok(apm)
    }
    pub fn configure(&mut self, o: &VoiceOptions) {
        self.echo.set_config(Config {
            echo_canceller: o.echo_cancellation.then(EchoCanceller::default),
            high_pass_filter: Some(HighPassFilter::default()),
            ..Default::default()
        });
        self.gain_enabled = o.automatic_gain;
        self.gain.set_config(Config {
            gain_controller: o.automatic_gain.then(|| {
                GainController::GainController2(GainController2 {
                    input_volume_controller_enabled: false,
                    adaptive_digital: Some(AdaptiveDigital::default()),
                    ..Default::default()
                })
            }),
            ..Default::default()
        });
    }
    pub fn render(&self, pcm: &[f32; TICK]) -> Result<()> {
        self.echo
            .analyze_render_frame([pcm.as_slice()])
            .map_err(err)
    }
    pub fn capture_echo(&self, pcm: &mut [f32; TICK]) -> Result<()> {
        self.echo
            .process_capture_frame([pcm.as_mut_slice()])
            .map_err(err)
    }
    pub fn capture_gain(&self, pcm: &mut [f32; TICK]) -> Result<()> {
        if self.gain_enabled {
            self.gain
                .process_capture_frame([pcm.as_mut_slice()])
                .map_err(err)?;
        }
        Ok(())
    }
    pub fn delay(&self, ms: u16) {
        self.echo.set_stream_delay_ms(ms);
    }
    pub fn muted(&self, muted: bool) {
        self.echo.set_output_will_be_muted(muted);
        self.gain.set_output_will_be_muted(muted);
    }
    pub fn reset(&self) {
        self.echo.reinitialize();
        self.gain.reinitialize();
    }
}
// Recursive model state can decay into subnormals during silence. Flush only
// inaudible underflows while inference runs, then restore the caller's thread
// FP environment. Without this guard, silent HQ inference is substantially
// more expensive than speech on x86_64 (see BENCH.md).
#[cfg(target_arch = "x86_64")]
struct FlushSubnormals(u32);
#[cfg(target_arch = "x86_64")]
impl FlushSubnormals {
    #[allow(deprecated)]
    fn enter() -> Self {
        unsafe {
            let original = std::arch::x86_64::_mm_getcsr();
            std::arch::x86_64::_mm_setcsr(original | (1 << 15) | (1 << 6));
            Self(original)
        }
    }
}
#[cfg(target_arch = "x86_64")]
impl Drop for FlushSubnormals {
    #[allow(deprecated)]
    fn drop(&mut self) {
        unsafe { std::arch::x86_64::_mm_setcsr(self.0) }
    }
}

pub(super) struct DeepDenoiser {
    model: df::tract::DfTract,
}
impl DeepDenoiser {
    pub fn new() -> Result<Self> {
        let model = df::tract::DfTract::new(
            df::tract::DfParams::default(),
            &df::tract::RuntimeParams::default(),
        )
        .map_err(err)?;
        if model.hop_size != TICK || model.sr != 48000 {
            return Err(err("DFN3 model must be 48kHz/480-sample hop"));
        }
        Ok(Self { model })
    }
    pub fn process(&mut self, frame: &[f32; TICK]) -> Result<[f32; TICK]> {
        #[cfg(target_arch = "x86_64")]
        let _fp = FlushSubnormals::enter();
        let mut output = [0.0; TICK];
        self.model
            .process(
                ndarray::ArrayView2::from_shape((1, TICK), frame).map_err(err)?,
                ndarray::ArrayViewMut2::from_shape((1, TICK), &mut output).map_err(err)?,
            )
            .map_err(err)?;
        Ok(output)
    }
}
#[derive(Clone, Debug)]
pub struct NoiseCpuProbe {
    pub recommended: NoiseSuppressionMode,
    pub mean_frame: Duration,
    pub p95_frame: Duration,
    /// A fallback reason is never silently labeled as a high-quality result.
    pub failure: Option<String>,
}
/// Explicit, generated-signal probe, lazily used on first AUTO call. Cache/persist
/// this result in the host's settings to avoid repeating after process restarts.
/// No audio device, socket, worker or periodic activity is created.
pub fn benchmark_noise_quality() -> NoiseCpuProbe {
    static PROBE: OnceLock<NoiseCpuProbe> = OnceLock::new();
    PROBE
        .get_or_init(|| {
            let result = (|| -> Result<(Duration, Duration)> {
                let mut df = DeepDenoiser::new()?;
                let frame = std::array::from_fn(|i| {
                    ((i as f32 * 0.037).sin() + (i as f32 * 0.193).cos()) * 0.05
                });
                for _ in 0..4 {
                    df.process(&frame)?;
                }
                let mut times = Vec::with_capacity(24);
                for _ in 0..24 {
                    let start = Instant::now();
                    df.process(&frame)?;
                    times.push(start.elapsed());
                }
                let sum: Duration = times.iter().sum();
                times.sort();
                Ok((sum / 24, times[22]))
            })();
            match result {
                Ok((mean, p95)) => NoiseCpuProbe {
                    recommended: if p95 < Duration::from_millis(6) {
                        NoiseSuppressionMode::HighQuality
                    } else {
                        NoiseSuppressionMode::Standard
                    },
                    mean_frame: mean,
                    p95_frame: p95,
                    failure: None,
                },
                Err(e) => NoiseCpuProbe {
                    recommended: NoiseSuppressionMode::Standard,
                    mean_frame: Duration::ZERO,
                    p95_frame: Duration::ZERO,
                    failure: Some(e.to_string()),
                },
            }
        })
        .clone()
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;
    #[test]
    #[allow(deprecated)]
    fn inference_fp_guard_restores_callers_thread_state() {
        unsafe {
            let before = std::arch::x86_64::_mm_getcsr();
            {
                let _guard = FlushSubnormals::enter();
                assert_eq!(std::arch::x86_64::_mm_getcsr() & 0x8040, 0x8040);
            }
            assert_eq!(std::arch::x86_64::_mm_getcsr(), before);
        }
    }
}
