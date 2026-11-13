//! Device-independent 48 kHz call engine. No threads, devices, timers or network.
//! Supply exactly 480 mono samples per capture/render tick (10 ms). Wire packets
//! are 480 or 960 samples (10/20 ms); sequences are extended, not wrapping RTP u16.
use crate::{Error, Result};
use std::collections::BTreeMap;
mod effects;
mod playout;
pub use effects::{benchmark_noise_quality, NoiseCpuProbe, DEEP_FILTER_ALGORITHMIC_DELAY_MS};
use effects::{DeepDenoiser, NativeApm};
pub use playout::{PlayoutStatistics, RepairStatistics, VoiceMixer};
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NoiseSuppressionMode {
    Off,
    Standard,
    HighQuality,
    #[default]
    Auto,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VoiceProfile {
    #[default]
    Voice,
    Studio,
}

pub const SAMPLE_RATE: u32 = 48_000;
pub const TICK: usize = 480;
pub const PACKET_SAMPLES: usize = 960;
pub const MAX_PACKET_BYTES: usize = 1275;
pub const MAX_REMOTES: usize = 128;
/// Negotiated WASAPI shared-mode ingress/egress sample representation. These
/// exact helpers are also used by the device-free end-to-end timing harness.
#[inline]
pub fn decode_device_float(bytes: [u8; 4]) -> f32 {
    f32::from_le_bytes(bytes)
}
#[inline]
pub fn encode_device_float(sample: f32) -> [u8; 4] {
    sample.to_le_bytes()
}
const MAX_JITTER_PACKETS: usize = 12;

#[cfg(test)]
mod low_latency_tests {
    use super::*;
    #[test]
    fn ten_ms_voice_packets_and_explicit_jitter_startup_floor() {
        let mut processor = VoiceProcessor::new(VoiceOptions {
            packet_duration_ms: 10,
            noise_mode: NoiseSuppressionMode::Off,
            echo_cancellation: false,
            automatic_gain: false,
            mode: InputMode::PushToTalk,
            ..Default::default()
        })
        .unwrap();
        processor.set_push_to_talk(true);
        let mut mixer = VoiceMixer::new(1).unwrap();
        mixer.add_remote(1).unwrap();
        assert_eq!(mixer.statistics(1).unwrap().target_delay_ms, 20);
        let mut energy = 0.;
        for tick in 0..80 {
            let pcm = std::array::from_fn(|i| ((tick * TICK + i) as f32 * 0.03).sin() * 0.1);
            let packet = processor.capture(&pcm).unwrap().unwrap();
            assert_eq!(packet.timestamp, (tick * TICK) as u64);
            assert_eq!(
                opus::packet::get_nb_samples(&packet.data, SAMPLE_RATE).unwrap(),
                TICK
            );
            mixer.push(1, packet).unwrap();
            energy += mixer.render().unwrap().iter().map(|s| s * s).sum::<f32>();
        }
        assert!(energy > 1.);
        mixer.flush();
        assert_eq!(mixer.statistics(1).unwrap().target_delay_ms, 10);
    }
}

fn codec_error(e: impl std::fmt::Display) -> Error {
    Error::Platform(format!("audio: {e}"))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InputMode {
    #[default]
    VoiceActivity,
    PushToTalk,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoiceOptions {
    /// Initial packetization. Network feedback prefers 10 ms on clean links and
    /// 20 ms under loss/jitter, with recovery hysteresis.
    pub packet_duration_ms: u16,
    pub bitrate: u32,
    pub profile: VoiceProfile,
    pub noise_mode: NoiseSuppressionMode,
    /// Compatibility bypass; false always forces Off regardless of noise_mode.
    pub noise_suppression: bool,
    pub automatic_sensitivity: bool,
    pub ptt_release_delay_ms: u16,
    pub echo_cancellation: bool,
    pub automatic_gain: bool,
    pub mode: InputMode,
    /// RMS dBFS threshold, -96..0, evaluated BEFORE automatic gain.
    pub sensitivity_dbfs: f32,
    /// NN VAD confidence, 0..1; active even when denoising is disabled.
    pub vad_probability: f32,
    /// Minimum adaptive jitter target: 1..6 packets, using their 10/20 ms duration.
    pub jitter_packets: usize,
}
impl Default for VoiceOptions {
    fn default() -> Self {
        Self {
            packet_duration_ms: 20,
            bitrate: 64_000,
            profile: VoiceProfile::Voice,
            noise_mode: NoiseSuppressionMode::Auto,
            automatic_sensitivity: true,
            ptt_release_delay_ms: 80,
            noise_suppression: true,
            echo_cancellation: true,
            automatic_gain: true,
            mode: InputMode::VoiceActivity,
            sensitivity_dbfs: -50.0,
            vad_probability: 0.5,
            jitter_packets: 1,
        }
    }
}
impl VoiceOptions {
    pub fn studio() -> Self {
        Self {
            profile: VoiceProfile::Studio,
            bitrate: 192_000,
            noise_mode: NoiseSuppressionMode::Off,
            noise_suppression: false,
            echo_cancellation: false,
            automatic_gain: false,
            ..Default::default()
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate_audio_bitrate(self.bitrate)?;
        let range = if self.profile == VoiceProfile::Studio {
            128_000..=256_000
        } else {
            32_000..=128_000
        };
        if !range.contains(&self.bitrate)
            || self.ptt_release_delay_ms > 1000
            || ![10, 20].contains(&self.packet_duration_ms)
        {
            return Err(Error::Invalid(
                "voice bitrate/profile or PTT release delay out of range".into(),
            ));
        }
        if !self.sensitivity_dbfs.is_finite()
            || !(-96.0..=0.0).contains(&self.sensitivity_dbfs)
            || !self.vad_probability.is_finite()
            || !(0.0..=1.0).contains(&self.vad_probability)
            || !(1..=6).contains(&self.jitter_packets)
        {
            return Err(Error::Invalid(
                "invalid voice sensitivity/VAD/jitter setting".into(),
            ));
        }
        Ok(())
    }
}
pub fn validate_audio_bitrate(bitrate: u32) -> Result<()> {
    if !(6_000..=510_000).contains(&bitrate) {
        return Err(Error::Invalid(
            "Opus bitrate must be 6000..510000 bit/s".into(),
        ));
    }
    Ok(())
}

/// Transport-neutral Opus packet. Caller maps extended sequence/timestamp to RTP.
#[derive(Clone, Debug)]
pub struct AudioPacket {
    pub sequence: u64,
    /// 48 kHz sample-clock timestamp; advances during mute, VAD and DTX.
    pub timestamp: u64,
    pub data: Vec<u8>,
}

/// Opus always has FEC/DTX enabled; FEC uses 20 ms packets and a 10% loss hint.
/// Complexity 10 favors quality. Stereo is for system/app audio or Studio.
pub struct OpusEncoder {
    inner: opus::Encoder,
    channels: usize,
    sequence: u64,
}
impl OpusEncoder {
    pub fn new(stereo: bool, bitrate: u32) -> Result<Self> {
        validate_audio_bitrate(bitrate)?;
        let mut inner = opus::Encoder::new(
            SAMPLE_RATE,
            if stereo {
                opus::Channels::Stereo
            } else {
                opus::Channels::Mono
            },
            if stereo {
                opus::Application::Audio
            } else {
                opus::Application::Voip
            },
        )
        .map_err(codec_error)?;
        inner.set_inband_fec(true).map_err(codec_error)?;
        inner.set_dtx(true).map_err(codec_error)?;
        inner.set_packet_loss_perc(10).map_err(codec_error)?;
        inner.set_complexity(10).map_err(codec_error)?;
        inner
            .set_bitrate(opus::Bitrate::Bits(bitrate as i32))
            .map_err(codec_error)?;
        Ok(Self {
            inner,
            channels: if stereo { 2 } else { 1 },
            sequence: 0,
        })
    }
    pub fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        validate_audio_bitrate(bitrate)?;
        self.inner
            .set_bitrate(opus::Bitrate::Bits(bitrate as i32))
            .map_err(codec_error)
    }
    pub fn encode(&mut self, pcm: &[f32], timestamp: u64) -> Result<AudioPacket> {
        if ![TICK * self.channels, PACKET_SAMPLES * self.channels].contains(&pcm.len())
            || pcm.iter().any(|s| !s.is_finite())
        {
            return Err(Error::Invalid(
                "Opus requires 10 or 20 ms finite 48 kHz PCM".into(),
            ));
        }
        let mut bytes = vec![0; MAX_PACKET_BYTES];
        let len = self
            .inner
            .encode_float(pcm, &mut bytes)
            .map_err(codec_error)?;
        bytes.truncate(len);
        let packet = AudioPacket {
            sequence: self.sequence,
            timestamp,
            data: bytes,
        };
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| Error::Invalid("audio sequence exhausted".into()))?;
        Ok(packet)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeakingEvent {
    /// None is the local microphone, Some is the caller's remote user key.
    pub user: Option<u64>,
    pub speaking: bool,
}

/// Native WebRTC AEC3/HPF, selected suppression, then native AGC2.
/// Feed *actual post-volume playback* to render(), not arriving network audio.
pub struct VoiceProcessor {
    options: VoiceOptions,
    apm: NativeApm,
    deep: Option<DeepDenoiser>,
    selected_noise: NoiseSuppressionMode,
    noise_floor: f32,
    ptt_release_ticks: u16,
    last_processed: [f32; TICK],
    last_stereo: [f32; TICK * 2],
    stereo_pcm: [f32; PACKET_SAMPLES * 2],
    noise: Box<nnnoiseless::DenoiseState<'static>>,
    encoder: OpusEncoder,
    packet_pcm: [f32; PACKET_SAMPLES],
    half: bool,
    clock: u64,
    muted: bool,
    ptt: bool,
    hangover: u8,
    speaking: bool,
    event: Option<SpeakingEvent>,
    quiet_ticks: u32,
    render_quiet_ticks: u16,
    fade: f32,
    wire_sequence: u64,
    packet_policy: PacketPolicy,
    ns_tail: u8,
}
fn selected_noise(options: &VoiceOptions) -> NoiseSuppressionMode {
    if !options.noise_suppression || options.profile == VoiceProfile::Studio {
        NoiseSuppressionMode::Off
    } else if options.noise_mode == NoiseSuppressionMode::Auto {
        benchmark_noise_quality().recommended
    } else {
        options.noise_mode
    }
}
impl VoiceProcessor {
    pub fn new(options: VoiceOptions) -> Result<Self> {
        options.validate()?;
        let apm = NativeApm::new(&options)?;
        let selected_noise = selected_noise(&options);
        let deep = if selected_noise == NoiseSuppressionMode::HighQuality {
            Some(DeepDenoiser::new()?)
        } else {
            None
        };
        let mut encoder =
            OpusEncoder::new(options.profile == VoiceProfile::Studio, options.bitrate)?;
        if options.profile == VoiceProfile::Studio {
            encoder.inner.set_dtx(false).map_err(codec_error)?;
            encoder
                .inner
                .set_signal(opus::Signal::Music)
                .map_err(codec_error)?;
        }
        Ok(Self {
            encoder,
            deep,
            selected_noise,
            noise_floor: -70.0,
            ptt_release_ticks: 0,
            last_processed: [0.0; TICK],
            last_stereo: [0.0; TICK * 2],
            stereo_pcm: [0.0; PACKET_SAMPLES * 2],
            apm,
            options,
            noise: nnnoiseless::DenoiseState::new(),
            packet_pcm: [0.0; PACKET_SAMPLES],
            half: false,
            clock: 0,
            muted: false,
            ptt: false,
            hangover: 0,
            speaking: false,
            event: None,
            quiet_ticks: 0,
            render_quiet_ticks: 0,
            fade: 0.0,
            wire_sequence: 0,
            packet_policy: PacketPolicy::default(),
            ns_tail: 0,
        })
    }
    pub fn configure(&mut self, options: VoiceOptions) -> Result<()> {
        options.validate()?;
        if options.profile != self.options.profile {
            return Err(Error::Invalid(
                "profile/channel changes require a new negotiated voice session".into(),
            ));
        }
        self.encoder.set_bitrate(options.bitrate)?;
        let selected = selected_noise(&options);
        if selected == NoiseSuppressionMode::HighQuality && self.deep.is_none() {
            self.deep = Some(DeepDenoiser::new()?);
        }
        if selected != NoiseSuppressionMode::HighQuality {
            self.deep = None;
        }
        self.selected_noise = selected;
        if options.packet_duration_ms != self.options.packet_duration_ms {
            self.half = false;
            self.packet_pcm.fill(0.0);
            self.stereo_pcm.fill(0.0);
        }
        if options.echo_cancellation != self.options.echo_cancellation
            || options.automatic_gain != self.options.automatic_gain
        {
            self.apm.configure(&options);
        }
        self.options = options;
        Ok(())
    }
    pub fn set_muted(&mut self, muted: bool) {
        if self.muted == muted {
            return;
        }
        self.muted = muted;
        self.apm.muted(muted);
        if muted {
            self.packet_pcm.fill(0.0);
            self.stereo_pcm.fill(0.0);
            self.ptt_release_ticks = 0;
            self.hangover = 0;
            self.set_speaking(false);
        } else {
            self.apm.reset();
            self.fade = 0.0;
        }
    }
    pub fn set_push_to_talk(&mut self, pressed: bool) {
        if self.ptt && !pressed {
            self.ptt_release_ticks = self.options.ptt_release_delay_ms.div_ceil(10);
        }
        self.ptt = pressed;
    }
    pub fn selected_noise_mode(&self) -> NoiseSuppressionMode {
        self.selected_noise
    }
    pub fn last_processed(&self) -> &[f32; TICK] {
        &self.last_processed
    }
    pub fn last_processed_stereo(&self) -> &[f32; TICK * 2] {
        &self.last_stereo
    }
    pub fn reset_device_state(&mut self) {
        self.apm.reset();
        self.packet_pcm.fill(0.0);
        self.stereo_pcm.fill(0.0);
        self.half = false;
    }
    pub fn sensitivity_dbfs(&self) -> f32 {
        if self.options.automatic_sensitivity {
            (self.noise_floor + 10.0).clamp(-65.0, -20.0)
        } else {
            self.options.sensitivity_dbfs
        }
    }
    /// Caller feeds congestion estimates; bounded AIMD, never below 32k voice.
    pub fn adapt_bitrate(&mut self, loss_percent: u8, available_bitrate: u32) -> Result<u32> {
        if loss_percent > 100 {
            return Err(Error::Invalid("loss must be 0..100%".into()));
        }
        let (min, max) = if self.options.profile == VoiceProfile::Studio {
            (128000, 256000)
        } else {
            (32000, 128000)
        };
        let target = if loss_percent >= 5 {
            self.options.bitrate * 4 / 5
        } else {
            self.options.bitrate.saturating_add(8000)
        }
        .min(available_bitrate)
        .clamp(min, max);
        self.encoder.set_bitrate(target)?;
        self.encoder
            .inner
            .set_packet_loss_perc(i32::from(loss_percent))
            .map_err(codec_error)?;
        self.options.bitrate = target;
        Ok(target)
    }
    pub fn set_stream_delay_ms(&mut self, delay: u32) -> Result<()> {
        if delay > 500 {
            return Err(Error::Invalid("AEC delay must be <=500 ms".into()));
        }
        self.apm.delay(delay as u16);
        Ok(())
    }
    pub fn render(&mut self, pcm: &[f32; TICK]) -> Result<()> {
        if pcm.iter().any(|s| !s.is_finite()) {
            return Err(Error::Invalid("non-finite render samples".into()));
        }
        let pcm = pcm.map(|s| s.clamp(-1.0, 1.0));
        self.render_quiet_ticks = if rms_dbfs(&pcm) < -90.0 {
            self.render_quiet_ticks.saturating_add(1)
        } else {
            0
        };
        if self.options.profile != VoiceProfile::Studio
            && (self.render_quiet_ticks < 10 || self.quiet_ticks <= 6)
        {
            self.apm.render(&pcm)?;
        }
        Ok(())
    }
    fn set_speaking(&mut self, speaking: bool) {
        if self.speaking != speaking {
            self.speaking = speaking;
            self.event = Some(SpeakingEvent {
                user: None,
                speaking,
            });
        }
    }
    pub fn take_speaking_event(&mut self) -> Option<SpeakingEvent> {
        self.event.take()
    }
    pub fn capture_stereo(&mut self, pcm: &[f32; TICK * 2]) -> Result<Option<AudioPacket>> {
        if self.options.profile != VoiceProfile::Studio {
            return self.capture(&std::array::from_fn(|i| {
                (pcm[i * 2] + pcm[i * 2 + 1]) * 0.5
            }));
        }
        if pcm.iter().any(|s| !s.is_finite()) {
            return Err(Error::Invalid("non-finite studio PCM".into()));
        }
        let open = !self.muted
            && (self.options.mode != InputMode::PushToTalk
                || self.ptt
                || self.ptt_release_ticks > 0);
        let offset = if self.half { TICK * 2 } else { 0 };
        for (n, s) in pcm.iter().enumerate() {
            self.stereo_pcm[offset + n] = if open { s.clamp(-1.0, 1.0) } else { 0.0 };
        }
        self.last_processed = std::array::from_fn(|i| (pcm[i * 2] + pcm[i * 2 + 1]) * 0.5);
        self.last_stereo = pcm.map(|s| s.clamp(-1.0, 1.0));
        self.ptt_release_ticks = self.ptt_release_ticks.saturating_sub(1);
        self.set_speaking(open && rms_dbfs(pcm) >= self.sensitivity_dbfs());
        self.clock += TICK as u64;
        if self.options.packet_duration_ms == 10 {
            self.half = false;
            return self
                .encoder
                .encode(&self.stereo_pcm[..TICK * 2], self.clock - TICK as u64)
                .map(Some);
        }
        self.half = !self.half;
        if self.half {
            Ok(None)
        } else {
            self.encoder
                .encode(&self.stereo_pcm, self.clock - PACKET_SAMPLES as u64)
                .map(Some)
        }
    }
    pub fn capture(&mut self, pcm: &[f32; TICK]) -> Result<Option<AudioPacket>> {
        if pcm.iter().any(|s| !s.is_finite()) {
            return Err(Error::Invalid("non-finite microphone samples".into()));
        }
        if self.options.profile == VoiceProfile::Studio {
            return Err(Error::Invalid("Studio requires capture_stereo".into()));
        }
        let pcm = pcm.map(|s| s.clamp(-1.0, 1.0));
        let level = rms_dbfs(&pcm);
        // Cheap energy gate precedes APM, neural VAD and DFN. Six tail hops
        // drain overlap/lookahead; a 20 ms fade prevents a gate-edge click.
        let floor = if self.options.mode == InputMode::PushToTalk {
            // A saved Voice Activity threshold must never silently mute PTT.
            // Only an inaudible noise floor may suspend the held-open path.
            (self.noise_floor + 6.0).min(-65.0)
        } else if self.options.automatic_sensitivity {
            (self.noise_floor + 6.0).min(self.sensitivity_dbfs() - 6.0)
        } else {
            self.options.sensitivity_dbfs - 6.0
        };
        let below = level < floor;
        if below {
            self.noise_floor =
                (self.noise_floor * 0.99 + level.clamp(-90.0, 0.0) * 0.01).clamp(-80.0, -25.0);
        }
        let permitted = !self.muted
            && (self.options.mode != InputMode::PushToTalk
                || self.ptt
                || self.ptt_release_ticks > 0);
        self.quiet_ticks = if below || !permitted {
            self.quiet_ticks.saturating_add(1)
        } else {
            0
        };
        let awake = self.quiet_ticks <= 6 && permitted;
        let mut processed = if awake { pcm } else { [0.0; TICK] };
        let probability = if awake {
            self.apm.capture_echo(&mut processed)?;
            // nnnoiseless expects i16-scale floats, not normalized [-1,1].
            let mut scaled = [0.0; TICK];
            for (o, i) in scaled.iter_mut().zip(processed) {
                *o = i * 32768.0;
            }
            let mut clean = [0.0; TICK];
            // PTT already supplies activity. Only Standard needs the RNNoise
            // pass for actual suppression; High does not run a second neural
            // model just to rediscover the user's held key.
            let probability = if self.selected_noise != NoiseSuppressionMode::Standard
                && (self.options.mode == InputMode::PushToTalk
                    || self.options.vad_probability == 0.0)
            {
                if level >= floor {
                    1.0
                } else {
                    0.0
                }
            } else {
                self.noise.process_frame(&mut clean, &scaled)
            };
            if (self.options.automatic_sensitivity || self.options.mode == InputMode::PushToTalk)
                && probability < 0.2
            {
                self.noise_floor =
                    (self.noise_floor * 0.99 + level.clamp(-90.0, 0.0) * 0.01).clamp(-80.0, -25.0);
            }
            if self.selected_noise == NoiseSuppressionMode::Standard {
                for i in 0..TICK {
                    processed[i] = clean[i] / 32768.0;
                }
            }
            if probability >= 0.1 {
                self.ns_tail = 6;
            } else {
                self.ns_tail = self.ns_tail.saturating_sub(1);
            }
            if let Some(df) = &mut self.deep {
                if self.ns_tail > 0 {
                    processed = df.process(&processed)?;
                } else {
                    processed.fill(0.0);
                }
            }
            self.apm.capture_gain(&mut processed)?;
            probability
        } else {
            0.0
        };
        let neural_closing = self.deep.is_some()
            && self.options.mode == InputMode::VoiceActivity
            && self.ns_tail <= 2;
        let target = if below || !permitted || neural_closing {
            0.0
        } else {
            1.0
        };
        for sample in &mut processed {
            self.fade += (target - self.fade).clamp(-1.0 / 960.0, 1.0 / 240.0);
            *sample *= self.fade;
        }
        self.last_processed = processed;
        self.last_stereo = std::array::from_fn(|i| processed[i / 2]);
        let threshold = self.sensitivity_dbfs();
        let detected = level >= threshold && probability >= self.options.vad_probability;
        if detected {
            self.hangover = 20;
        } else {
            self.hangover = self.hangover.saturating_sub(1);
        }
        let open = !self.muted
            && match self.options.mode {
                InputMode::PushToTalk => self.ptt || self.ptt_release_ticks > 0,
                InputMode::VoiceActivity => self.hangover != 0,
            };
        self.ptt_release_ticks = self.ptt_release_ticks.saturating_sub(1);
        let speaking_threshold = if self.options.mode == InputMode::PushToTalk {
            floor
        } else {
            threshold
        };
        self.set_speaking(open && level >= speaking_threshold);
        let offset = if self.half { TICK } else { 0 };
        for (n, sample) in processed.iter().enumerate() {
            self.packet_pcm[offset + n] = if open { sample.clamp(-1.0, 1.0) } else { 0.0 };
        }
        self.clock += TICK as u64;
        if self.options.packet_duration_ms == 10 {
            self.half = false;
            return self.encode_voice(TICK);
        }
        self.half = !self.half;
        if self.half {
            Ok(None)
        } else {
            self.encode_voice(PACKET_SAMPLES)
        }
    }
    fn encode_voice(&mut self, samples: usize) -> Result<Option<AudioPacket>> {
        // Prime Opus DTX, then only encode a comfort update every 400 ms.
        // Timestamp continues during silence. Wire sequence counts SENT packets,
        // so a DTX interval is never misrepresented as packet loss/FEC work.
        if self.quiet_ticks > 30 && !self.quiet_ticks.is_multiple_of(40) {
            return Ok(None);
        }
        let mut packet = self
            .encoder
            .encode(&self.packet_pcm[..samples], self.clock - samples as u64)?;
        packet.sequence = self.wire_sequence;
        self.wire_sequence += 1;
        Ok(Some(packet))
    }
    /// Feedback intervals are one second; degradation is immediate, recovery
    /// requires five consecutive clean intervals. Never discards a half packet.
    pub fn adapt_packet_duration(&mut self, loss_percent: u8, jitter_ms: u32) -> u16 {
        let duration = self.packet_policy.observe(loss_percent, jitter_ms);
        if !self.half {
            self.options.packet_duration_ms = duration;
        }
        self.options.packet_duration_ms
    }
    pub fn packet_duration_ms(&self) -> u16 {
        self.options.packet_duration_ms
    }
}
/// 10 ms buys 10 ms packetization at ~32 kbit/s extra IPv4/UDP/RTP/SRTP
/// overhead (80 bytes x 50 extra packets/s); 20 ms is safer on impaired links.
#[derive(Default)]
pub struct PacketPolicy {
    clean: u8,
    degraded: bool,
}
impl PacketPolicy {
    pub fn observe(&mut self, loss: u8, jitter_ms: u32) -> u16 {
        if loss >= 3 || jitter_ms >= 15 {
            self.degraded = true;
            self.clean = 0;
        } else if loss <= 1 && jitter_ms <= 5 {
            self.clean = self.clean.saturating_add(1);
            if self.clean >= 5 {
                self.degraded = false;
            }
        } else {
            self.clean = 0;
        }
        if self.degraded {
            20
        } else {
            10
        }
    }
}
pub fn rms_dbfs(pcm: &[f32]) -> f32 {
    if pcm.is_empty() {
        return -120.0;
    }
    let power = pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32;
    10.0 * power.max(1e-12).log10()
}

#[cfg(test)]
mod silence_tests {
    use super::*;
    #[test]
    fn ptt_does_not_inherit_a_hidden_voice_activity_threshold() {
        for noise_mode in [NoiseSuppressionMode::Off, NoiseSuppressionMode::Standard] {
            let mut p = VoiceProcessor::new(VoiceOptions {
                noise_mode,
                mode: InputMode::PushToTalk,
                automatic_sensitivity: false,
                sensitivity_dbfs: 0.,
                echo_cancellation: false,
                automatic_gain: false,
                packet_duration_ms: 10,
                ..Default::default()
            })
            .unwrap();
            for _ in 0..200 {
                p.capture(&[0.; TICK]).unwrap();
            }
            p.set_push_to_talk(true);
            for tick in 0..8 {
                let pcm = std::array::from_fn(|i| ((tick * TICK + i) as f32 * 0.07).sin() * 0.2);
                p.capture(&pcm).unwrap();
            }
            assert!(p.speaking);
            assert!(p.last_processed.iter().any(|s| *s != 0.));
        }
    }
    #[test]
    fn silent_unmuted_ptt_skips_dsp_and_bounds_dtx_packets() {
        for noise_mode in [NoiseSuppressionMode::Off, NoiseSuppressionMode::Standard] {
            let mut p = VoiceProcessor::new(VoiceOptions {
                noise_mode,
                packet_duration_ms: 10,
                mode: InputMode::PushToTalk,
                ..Default::default()
            })
            .unwrap();
            p.set_push_to_talk(true);
            let mut count = 0;
            let mut sequence = 0;
            for _ in 0..1000 {
                if let Some(packet) = p.capture(&[0.; TICK]).unwrap() {
                    assert_eq!(packet.sequence, sequence);
                    sequence += 1;
                    count += 1;
                }
            }
            assert!(count <= 55, "{count}");
            assert_eq!(p.quiet_ticks, 1000);
            assert!(p.last_processed().iter().all(|s| *s == 0.));
            let pcm = std::array::from_fn(|i| (i as f32 * 0.03).sin() * 0.2);
            let resumed = p.capture(&pcm).unwrap().unwrap();
            assert_eq!(resumed.timestamp, 480000);
            assert!(p.last_processed().iter().all(|s| s.is_finite()));
        }
    }
    #[test]
    fn adaptation_has_degrade_and_recovery_hysteresis() {
        let mut p = PacketPolicy::default();
        assert_eq!(p.observe(0, 1), 10);
        assert_eq!(p.observe(3, 1), 20);
        for _ in 0..4 {
            assert_eq!(p.observe(0, 1), 20);
        }
        assert_eq!(p.observe(0, 1), 10);
        assert_eq!(p.observe(0, 16), 20);
        for _ in 0..8 {
            assert_eq!(p.observe(2, 8), 20);
        }
    }
    #[test]
    fn ptt_release_delay_retains_exact_capture_hops() {
        let mut p = VoiceProcessor::new(VoiceOptions {
            noise_mode: NoiseSuppressionMode::Off,
            packet_duration_ms: 10,
            mode: InputMode::PushToTalk,
            automatic_sensitivity: false,
            ptt_release_delay_ms: 30,
            ..Default::default()
        })
        .unwrap();
        let pcm = std::array::from_fn(|i| (i as f32 * 0.07).sin() * 0.2);
        p.set_push_to_talk(true);
        p.capture(&pcm).unwrap();
        p.set_push_to_talk(false);
        for _ in 0..3 {
            p.capture(&pcm).unwrap();
            assert!(p.packet_pcm[..TICK].iter().any(|s| *s != 0.));
        }
        p.capture(&pcm).unwrap();
        assert!(p.packet_pcm[..TICK].iter().all(|s| *s == 0.));
    }
}

struct Remote {
    decoder: opus::Decoder,
    packets: BTreeMap<u64, AudioPacket>,
    next: Option<u64>,
    pcm: [f32; PACKET_SAMPLES * 2],
    channels: usize,
    offset: usize,
    volume: f32,
    losses: u8,
    speaking: bool,
}
/// Fixed-delay jitter buffer; ordered packets, duplicate/late rejection, one
/// packet lookahead for Opus FEC, bounded PLC then rebuffer. No internal clock.
pub struct FixedVoiceMixer {
    streams: BTreeMap<u64, Remote>,
    jitter: usize,
    deafened: bool,
    events: BTreeMap<u64, bool>,
}
impl FixedVoiceMixer {
    pub fn new(jitter_packets: usize) -> Result<Self> {
        if !(1..=6).contains(&jitter_packets) {
            return Err(Error::Invalid("jitter must be 1..6 packets".into()));
        }
        Ok(Self {
            streams: BTreeMap::new(),
            jitter: jitter_packets,
            deafened: false,
            events: BTreeMap::new(),
        })
    }
    pub fn add_remote(&mut self, user: u64) -> Result<()> {
        self.add_stream(user, false)
    }
    /// Stereo system/app audio shares the same bounded jitter/mixer/playback.
    /// Use distinct caller keys for a participant's voice and share-audio track.
    pub fn add_stereo_remote(&mut self, user: u64) -> Result<()> {
        self.add_stream(user, true)
    }
    fn add_stream(&mut self, user: u64, stereo: bool) -> Result<()> {
        if let Some(stream) = self.streams.get(&user) {
            return if stream.channels == if stereo { 2 } else { 1 } {
                Ok(())
            } else {
                Err(Error::Invalid(
                    "stream already registered with different channel count".into(),
                ))
            };
        }
        if self.streams.len() >= MAX_REMOTES {
            return Err(Error::Invalid("remote stream limit reached".into()));
        }
        self.streams.insert(
            user,
            Remote {
                decoder: opus::Decoder::new(
                    SAMPLE_RATE,
                    if stereo {
                        opus::Channels::Stereo
                    } else {
                        opus::Channels::Mono
                    },
                )
                .map_err(codec_error)?,
                packets: BTreeMap::new(),
                next: None,
                pcm: [0.0; PACKET_SAMPLES * 2],
                channels: if stereo { 2 } else { 1 },
                offset: PACKET_SAMPLES,
                volume: 1.0,
                losses: 0,
                speaking: false,
            },
        );
        Ok(())
    }
    /// The caller already owns roster removal; no speaking event is retained
    /// for removed users. This also bounds coalesced events under roster churn.
    pub fn remove_remote(&mut self, user: u64) {
        self.streams.remove(&user);
        self.events.remove(&user);
    }
    pub fn set_jitter_packets(&mut self, target: usize) -> Result<()> {
        if !(1..=6).contains(&target) {
            return Err(Error::Invalid("jitter must be 1..6 packets".into()));
        }
        if self.jitter != target {
            self.jitter = target;
            for stream in self.streams.values_mut() {
                stream.next = None;
                stream.offset = PACKET_SAMPLES;
            }
        }
        Ok(())
    }
    pub fn set_volume(&mut self, user: u64, percent: u16) -> Result<()> {
        if percent > 200 {
            return Err(Error::Invalid("remote volume must be 0..200%".into()));
        }
        self.streams
            .get_mut(&user)
            .ok_or_else(|| Error::Invalid("unknown remote".into()))?
            .volume = f32::from(percent) / 100.0;
        Ok(())
    }
    pub fn set_deafened(&mut self, deafened: bool) {
        self.deafened = deafened;
    }
    pub fn push(&mut self, user: u64, packet: AudioPacket) -> Result<()> {
        let s = self
            .streams
            .get_mut(&user)
            .ok_or_else(|| Error::Invalid("unknown remote".into()))?;
        if packet.data.is_empty()
            || packet.data.len() > MAX_PACKET_BYTES
            || s.decoder
                .get_nb_samples(&packet.data)
                .map_err(codec_error)?
                != PACKET_SAMPLES
            || (s.channels == 1
                && opus::packet::get_nb_channels(&packet.data).map_err(codec_error)?
                    != opus::Channels::Mono)
        {
            return Err(Error::Invalid(
                "remote must send 20 ms Opus packets <=1275 bytes with registered channels".into(),
            ));
        }
        if s.next.is_some_and(|next| packet.sequence < next) {
            return Ok(());
        }
        // Reject huge jumps rather than letting one hostile packet pin a buffer.
        if s.next
            .is_some_and(|next| packet.sequence.saturating_sub(next) > 250)
        {
            return Err(Error::Invalid(
                "audio sequence jump >5 seconds; remove/re-add stream".into(),
            ));
        }
        s.packets.entry(packet.sequence).or_insert(packet);
        while s.packets.len() > MAX_JITTER_PACKETS {
            s.packets.pop_last();
        }
        Ok(())
    }
    pub fn take_speaking_events(&mut self) -> Vec<SpeakingEvent> {
        std::mem::take(&mut self.events)
            .into_iter()
            .map(|(user, speaking)| SpeakingEvent {
                user: Some(user),
                speaking,
            })
            .collect()
    }
    pub fn render(&mut self) -> Result<[f32; TICK]> {
        let stereo = self.render_stereo()?;
        Ok(std::array::from_fn(|i| {
            (stereo[i * 2] + stereo[i * 2 + 1]) * 0.5
        }))
    }
    pub fn render_stereo(&mut self) -> Result<[f32; TICK * 2]> {
        let mut mix = [0.0; TICK * 2];
        for (&user, s) in &mut self.streams {
            if s.offset == PACKET_SAMPLES {
                s.pcm.fill(0.0);
                s.offset = 0;
                if s.next.is_none() && s.packets.len() >= self.jitter {
                    s.next = s.packets.first_key_value().map(|(&seq, _)| seq);
                }
                if let Some(seq) = s.next {
                    let result = if let Some(packet) = s.packets.remove(&seq) {
                        s.losses = 0;
                        s.decoder.decode_float(
                            &packet.data,
                            &mut s.pcm[..PACKET_SAMPLES * s.channels],
                            false,
                        )
                    } else {
                        s.losses = s.losses.saturating_add(1);
                        let fec = seq.checked_add(1).and_then(|n| s.packets.get(&n));
                        s.decoder.decode_float(
                            fec.map_or(&[], |p| p.data.as_slice()),
                            &mut s.pcm[..PACKET_SAMPLES * s.channels],
                            fec.is_some(),
                        )
                    };
                    if result.is_err() {
                        // A valid Opus TOC can still contain a malformed coded
                        // payload. Conceal it locally; never let one remote
                        // packet terminate capture/playback for the whole call.
                        if s.decoder
                            .decode_float(&[], &mut s.pcm[..PACKET_SAMPLES * s.channels], false)
                            .is_err()
                        {
                            s.pcm.fill(0.0);
                        }
                    }
                    s.next = seq.checked_add(1);
                    if s.losses >= 10 {
                        s.next = None;
                        s.pcm.fill(0.0);
                    }
                }
            }
            let pcm = &s.pcm[s.offset * s.channels..(s.offset + TICK) * s.channels];
            let speaking = rms_dbfs(pcm) >= -50.0;
            if speaking != s.speaking {
                s.speaking = speaking;
                self.events.insert(user, speaking);
            }
            if !self.deafened {
                for i in 0..TICK {
                    mix[i * 2] += pcm[i * s.channels] * s.volume;
                    mix[i * 2 + 1] += pcm[i * s.channels + s.channels - 1] * s.volume;
                }
            }
            s.offset += TICK;
        }
        for sample in &mut mix {
            *sample = sample.clamp(-1.0, 1.0);
        }
        Ok(mix)
    }
}

/// Stereo decoder for shared system/app audio; exactly 20 ms output, or PLC
/// on empty input. Caller owns pacing/jitter and mixing with voice playback.
pub struct StereoDecoder(opus::Decoder);
impl StereoDecoder {
    pub fn new() -> Result<Self> {
        Ok(Self(
            opus::Decoder::new(SAMPLE_RATE, opus::Channels::Stereo).map_err(codec_error)?,
        ))
    }
    pub fn decode(&mut self, bytes: &[u8], fec: bool) -> Result<[f32; PACKET_SAMPLES * 2]> {
        if bytes.len() > MAX_PACKET_BYTES
            || (!bytes.is_empty()
                && self.0.get_nb_samples(bytes).map_err(codec_error)? != PACKET_SAMPLES)
        {
            return Err(Error::Invalid("expected 20 ms Opus".into()));
        }
        let mut pcm = [0.0; PACKET_SAMPLES * 2];
        self.0
            .decode_float(bytes, &mut pcm, fec)
            .map_err(codec_error)?;
        Ok(pcm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(frame: usize) -> [f32; TICK] {
        std::array::from_fn(|i| {
            ((frame * TICK + i) as f32 * std::f32::consts::TAU * 220.0 / 48000.0).sin() * 0.1
        })
    }
    fn packets(n: usize) -> Vec<AudioPacket> {
        let mut enc = OpusEncoder::new(false, 32000).unwrap();
        (0..n)
            .map(|i| {
                let pcm: Vec<_> = tone(i * 2).into_iter().chain(tone(i * 2 + 1)).collect();
                enc.encode(&pcm, i as u64 * 960).unwrap()
            })
            .collect()
    }
    fn ptt_processor() -> VoiceProcessor {
        VoiceProcessor::new(VoiceOptions {
            mode: InputMode::PushToTalk,
            automatic_sensitivity: false,
            ptt_release_delay_ms: 0,
            noise_suppression: false,
            automatic_gain: false,
            echo_cancellation: false,
            ..Default::default()
        })
        .unwrap()
    }
    #[test]
    fn opus_fec_dtx_and_bitrate_are_real_controls() {
        let mut enc = OpusEncoder::new(false, 32000).unwrap();
        assert!(enc.inner.get_inband_fec().unwrap());
        assert!(enc.inner.get_dtx().unwrap());
        assert_eq!(enc.inner.get_packet_loss_perc().unwrap(), 10);
        enc.set_bitrate(24000).unwrap();
        assert_eq!(enc.inner.get_bitrate().unwrap(), opus::Bitrate::Bits(24000));
        assert!(enc.set_bitrate(0).is_err());
        for _ in 0..100 {
            enc.encode(&[0.0; 960], 0).unwrap();
        }
        assert!(enc.encode(&[0.0; 960], 0).unwrap().data.len() <= 3);
    }
    #[test]
    fn opus_rejects_invalid_pcm_and_stereo_roundtrips() {
        assert!(OpusEncoder::new(false, 0).is_err());
        let mut enc = OpusEncoder::new(true, 96000).unwrap();
        assert!(enc.encode(&[0.0; 958], 0).is_err());
        assert!(enc.encode(&[f32::NAN; 1920], 0).is_err());
        let pcm: Vec<_> = (0..960)
            .flat_map(|i| [(i as f32 * 0.03).sin() * 0.1, (i as f32 * 0.07).sin() * 0.2])
            .collect();
        let p = enc.encode(&pcm, 123).unwrap();
        let mut dec = StereoDecoder::new().unwrap();
        let out = dec.decode(&p.data, false).unwrap();
        assert_eq!(p.timestamp, 123);
        assert_eq!(p.sequence, 0);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(rms_dbfs(&out) > -40.0);
        assert!(dec.decode(&[0; MAX_PACKET_BYTES + 1], false).is_err());
        assert!(dec
            .decode(&[], false)
            .unwrap()
            .iter()
            .all(|s| s.is_finite()));
    }
    #[test]
    fn aec3_reduces_a_synthetic_delayed_echo() {
        let apm = NativeApm::new(&VoiceOptions {
            noise_mode: NoiseSuppressionMode::Off,
            automatic_gain: false,
            ..Default::default()
        })
        .unwrap();
        let mut history = std::collections::VecDeque::from(vec![[0.0; TICK]; 5]);
        let mut seed = 3u32;
        let mut before = 0.0f64;
        let mut after = 0.0f64;
        for tick in 0..600 {
            let reference = std::array::from_fn::<_, TICK, _>(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed as f32 / u32::MAX as f32 - 0.5) * 0.2
            });
            apm.render(&reference).unwrap();
            history.push_back(reference);
            let echo = history.pop_front().unwrap().map(|s| s * 0.5);
            let mut clean = echo;
            apm.capture_echo(&mut clean).unwrap();
            if tick >= 400 {
                before += echo.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
                after += clean.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
            }
        }
        assert!(
            after < before * 0.5,
            "AEC failed to attenuate echo: before={before} after={after}"
        );
    }
    #[test]
    fn settings_validate_nonfinite_and_bounds() {
        for value in [f32::NAN, f32::INFINITY, -100.0, 1.0] {
            assert!(VoiceOptions {
                sensitivity_dbfs: value,
                ..Default::default()
            }
            .validate()
            .is_err());
        }
        assert!(VoiceOptions {
            vad_probability: 1.1,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(VoiceOptions {
            jitter_packets: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(VoiceOptions {
            jitter_packets: 7,
            ..Default::default()
        }
        .validate()
        .is_err());
    }
    #[test]
    fn ptt_mute_and_event_crossings_no_per_frame_events() {
        let mut p = ptt_processor();
        for i in 0..4 {
            p.capture(&tone(i)).unwrap();
            assert!(p.take_speaking_event().is_none());
        }
        p.set_push_to_talk(true);
        p.capture(&tone(4)).unwrap();
        assert_eq!(
            p.take_speaking_event(),
            Some(SpeakingEvent {
                user: None,
                speaking: true
            })
        );
        p.capture(&tone(5)).unwrap();
        assert!(p.take_speaking_event().is_none());
        p.set_muted(true);
        assert_eq!(
            p.take_speaking_event(),
            Some(SpeakingEvent {
                user: None,
                speaking: false
            })
        );
        assert!(p.packet_pcm.iter().all(|&s| s == 0.0));
        p.capture(&tone(6)).unwrap();
        assert!(p.take_speaking_event().is_none());
        p.set_muted(false);
        p.set_push_to_talk(false);
        p.capture(&tone(7)).unwrap();
        assert!(p.packet_pcm.iter().all(|&s| s == 0.0));
    }
    #[test]
    fn vad_sensitivity_and_48k_sample_clock() {
        let mut p = ptt_processor();
        p.configure(VoiceOptions {
            sensitivity_dbfs: -10.0,
            automatic_sensitivity: false,
            vad_probability: 0.0,
            noise_suppression: false,
            automatic_gain: false,
            echo_cancellation: false,
            ..Default::default()
        })
        .unwrap();
        p.capture(&tone(0)).unwrap();
        let packet = p.capture(&tone(1)).unwrap().unwrap();
        assert_eq!(packet.timestamp, 0);
        assert!(p.packet_pcm.iter().all(|&s| s == 0.0));
        p.configure(VoiceOptions {
            sensitivity_dbfs: -60.0,
            automatic_sensitivity: false,
            vad_probability: 0.0,
            noise_suppression: false,
            automatic_gain: false,
            echo_cancellation: false,
            ..Default::default()
        })
        .unwrap();
        p.capture(&tone(2)).unwrap();
        assert!(p.take_speaking_event().unwrap().speaking);
        assert_eq!(p.capture(&tone(3)).unwrap().unwrap().timestamp, 960);
    }
    #[test]
    fn full_aec_agc_rnnoise_pipeline_handles_silence_and_toggle() {
        let mut p = VoiceProcessor::new(Default::default()).unwrap();
        for _ in 0..40 {
            p.render(&[0.0; 480]).unwrap();
            p.capture(&[0.0; 480]).unwrap();
        }
        assert!(p.packet_pcm.iter().all(|s| s.is_finite()));
        assert!(p.take_speaking_event().is_none());
        p.configure(VoiceOptions {
            noise_suppression: false,
            echo_cancellation: false,
            automatic_gain: false,
            ..Default::default()
        })
        .unwrap();
        p.render(&tone(0)).unwrap();
        p.capture(&tone(0)).unwrap();
        assert!(p.set_stream_delay_ms(501).is_err());
        assert!(p.set_stream_delay_ms(40).is_ok());
        assert!(p.capture(&[f32::NAN; 480]).is_err());
    }
    #[test]
    fn jitter_reorders_rejects_duplicates_late_and_bounds_queue() {
        let mut m = FixedVoiceMixer::new(2).unwrap();
        m.add_remote(1).unwrap();
        let p = packets(20);
        m.push(1, p[1].clone()).unwrap();
        m.push(1, p[1].clone()).unwrap();
        assert_eq!(m.streams[&1].packets.len(), 1);
        assert!(m.render().unwrap().iter().all(|&s| s == 0.0));
        m.push(1, p[0].clone()).unwrap();
        m.render().unwrap();
        m.render().unwrap();
        assert_eq!(m.streams[&1].next, Some(1));
        m.push(1, p[0].clone()).unwrap();
        assert!(!m.streams[&1].packets.contains_key(&0));
        for p in p.into_iter().skip(2) {
            m.push(1, p).unwrap();
        }
        assert_eq!(m.streams[&1].packets.len(), MAX_JITTER_PACKETS);
        assert!(m.streams[&1].packets.contains_key(&1));
    }
    #[test]
    fn fec_plc_loss_recovery_and_deafen() {
        let mut m = FixedVoiceMixer::new(1).unwrap();
        m.add_remote(1).unwrap();
        let p = packets(8);
        for packet in p.into_iter().filter(|p| p.sequence != 2) {
            m.push(1, packet).unwrap();
        }
        for _ in 0..16 {
            assert!(m.render().unwrap().iter().all(|s| s.is_finite()));
        }
        m.set_deafened(true);
        for _ in 0..24 {
            assert!(m.render().unwrap().iter().all(|&s| s == 0.0));
        }
        assert!(m.streams[&1].next.is_none());
    }
    #[test]
    fn stereo_share_tracks_preserve_left_and_right_in_mixer() {
        let mut mixer = FixedVoiceMixer::new(1).unwrap();
        mixer.add_stereo_remote(2).unwrap();
        let mut encoder = OpusEncoder::new(true, 96000).unwrap();
        for packet in 0..8 {
            let pcm: Vec<_> = (0..960)
                .flat_map(|i| {
                    let t = (packet * 960 + i) as f32 / 48000.0;
                    [
                        (t * 220.0 * std::f32::consts::TAU).sin() * 0.1,
                        (t * 770.0 * std::f32::consts::TAU).sin() * 0.2,
                    ]
                })
                .collect();
            mixer
                .push(2, encoder.encode(&pcm, packet as u64 * 960).unwrap())
                .unwrap();
        }
        let mut difference = 0.0;
        for _ in 0..16 {
            let stereo = mixer.render_stereo().unwrap();
            difference += stereo
                .as_chunks::<2>()
                .0
                .iter()
                .map(|lr| (lr[0] - lr[1]).abs())
                .sum::<f32>();
        }
        assert!(difference > 10.0, "stereo decoder/mixer collapsed channels");
        mixer.set_jitter_packets(3).unwrap();
        assert_eq!(mixer.jitter, 3);
        assert!(mixer.set_jitter_packets(0).is_err());
    }
    #[test]
    fn malformed_opus_payloads_are_concealed_not_call_fatal() {
        let mut mixer = FixedVoiceMixer::new(1).unwrap();
        mixer.add_remote(1).unwrap();
        let toc = packets(1)[0].data[0];
        for n in 0..100u64 {
            let packet = AudioPacket {
                sequence: n,
                timestamp: n * 960,
                data: vec![toc, n as u8, 0xff, 0xff],
            };
            let _ = mixer.push(1, packet);
            assert!(mixer.render().unwrap().iter().all(|s| s.is_finite()));
            mixer.render().unwrap();
        }
    }
    #[test]
    fn mixer_volume_zero_double_and_limit() {
        let mut a = FixedVoiceMixer::new(1).unwrap();
        let mut b = FixedVoiceMixer::new(1).unwrap();
        for m in [&mut a, &mut b] {
            m.add_remote(1).unwrap();
            for p in packets(3) {
                m.push(1, p).unwrap();
            }
        }
        b.set_volume(1, 200).unwrap();
        let one = a.render().unwrap();
        let two = b.render().unwrap();
        for (one, two) in one.into_iter().zip(two) {
            assert!((one * 2.0 - two).abs() < 1e-5);
        }
        a.set_volume(1, 0).unwrap();
        assert!(a.render().unwrap().iter().all(|&s| s == 0.0));
        assert!(a.set_volume(1, 201).is_err());
        assert!(a.set_volume(2, 100).is_err());
        for id in 2..=MAX_REMOTES as u64 {
            a.add_remote(id).unwrap();
        }
        assert!(a.add_remote(999).is_err());
        a.remove_remote(1);
        a.add_remote(999).unwrap();
    }
    #[test]
    fn malformed_packets_cannot_allocate_unbounded_or_create_streams() {
        let mut m = FixedVoiceMixer::new(1).unwrap();
        let mut p = packets(1).remove(0);
        assert!(m.push(1, p.clone()).is_err());
        assert!(m.streams.is_empty());
        m.add_remote(1).unwrap();
        let mut bad = p.clone();
        bad.data.resize(MAX_PACKET_BYTES + 1, 0);
        assert!(m.push(1, bad).is_err());
        let mut stereo = OpusEncoder::new(true, 64000).unwrap();
        assert!(m.push(1, stereo.encode(&[0.0; 1920], 0).unwrap()).is_err());
        m.push(1, p.clone()).unwrap();
        m.render().unwrap();
        p.sequence = 10_000;
        assert!(m.push(1, p).is_err());
    }
}

#[cfg(test)]
mod hq_tests {
    use super::*;
    #[test]
    fn studio_is_stereo_ungated_and_dsp_bypassed() {
        let mut p = VoiceProcessor::new(VoiceOptions::studio()).unwrap();
        let frame = std::array::from_fn(|i| if i % 2 == 0 { 0.123 } else { -0.234 });
        p.capture_stereo(&frame).unwrap();
        p.capture_stereo(&frame).unwrap();
        assert_eq!(&p.stereo_pcm[..TICK * 2], &frame);
        assert_eq!(p.selected_noise_mode(), NoiseSuppressionMode::Off);
        p.set_muted(true);
        assert!(p.stereo_pcm.iter().all(|&s| s == 0.0));
        assert_eq!(p.adapt_bitrate(0, 999999).unwrap(), 200000);
        assert_eq!(p.adapt_bitrate(100, 0).unwrap(), 128000);
    }
    #[test]
    fn native_apm_abi_and_standard_pipeline_work_without_devices() {
        assert!(webrtc_audio_processing::verify_native_layout());
        let mut p = VoiceProcessor::new(VoiceOptions {
            noise_mode: NoiseSuppressionMode::Standard,
            mode: InputMode::PushToTalk,
            ptt_release_delay_ms: 30,
            ..Default::default()
        })
        .unwrap();
        p.set_push_to_talk(true);
        let frame = [0.1; TICK];
        p.capture(&frame).unwrap();
        p.set_push_to_talk(false);
        assert_eq!(p.ptt_release_ticks, 3);
        for _ in 0..3 {
            p.capture(&frame).unwrap();
        }
        assert_eq!(p.ptt_release_ticks, 0);
        assert_eq!(p.adapt_bitrate(0, 128000).unwrap(), 72000);
        assert_eq!(p.adapt_bitrate(30, 50000).unwrap(), 50000);
        assert_eq!(p.adapt_bitrate(100, 0).unwrap(), 32000);
        assert!(p.adapt_bitrate(101, 128000).is_err());
    }
    #[test]
    #[ignore = "explicit embedded DFN3 inference diagnostic; no devices, but model loading is heavy in debug"]
    fn deepfilternet3_model_loads_and_processes_generated_pcm() {
        let mut p = VoiceProcessor::new(VoiceOptions {
            noise_mode: NoiseSuppressionMode::HighQuality,
            mode: InputMode::PushToTalk,
            ..Default::default()
        })
        .unwrap();
        p.set_push_to_talk(true);
        for n in 0..12 {
            p.capture(&std::array::from_fn(|i| {
                ((n * TICK + i) as f32 * 0.071).sin() * 0.1
            }))
            .unwrap();
        }
        assert!(p.last_processed.iter().all(|s| s.is_finite()));
        assert_eq!(p.selected_noise_mode(), NoiseSuppressionMode::HighQuality);
    }
}
