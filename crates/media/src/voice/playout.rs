//! Adaptive NetEQ-inspired playout, with real libopus and bounded packet ingress.
use super::{
    rms_dbfs, AudioPacket, SpeakingEvent, MAX_PACKET_BYTES, MAX_REMOTES, PACKET_SAMPLES,
    SAMPLE_RATE, TICK,
};
use crate::{Error, Result};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
/// Stable, correctly named playout telemetry. Upstream 0.9.1's operation-rate
/// counter indices are inconsistent, so those misleading counters are NOT exposed.
#[derive(Clone, Debug, Default)]
pub struct PlayoutStatistics {
    pub current_buffer_size_ms: u32,
    pub target_delay_ms: u32,
    pub packets_awaiting_decode: usize,
    pub packets_per_sec: u32,
}
#[derive(Clone, Debug, Default)]
pub struct RepairStatistics {
    pub fec_attempts: u64,
    pub plc_frames: u64,
    pub late_samples_discarded: u64,
}
#[derive(Default)]
struct RepairState {
    last_sequence: Option<u64>,
    expanded_samples: usize,
    statistics: RepairStatistics,
}
fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Platform(format!("NetEQ: {e}"))
}
struct OpusPlayout {
    decoder: opus::Decoder,
    channels: u8,
    repair: Arc<Mutex<RepairState>>,
}
impl neteq::codec::AudioDecoder for OpusPlayout {
    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }
    fn channels(&self) -> u8 {
        self.channels
    }
    fn decode(&mut self, bytes: &[u8]) -> neteq::Result<Vec<f32>> {
        // Internal sequence prefix is not sent on the wire. The upstream
        // adaptive buffer is not codec-gap aware: restore missing source time
        // using Opus FEC/PLC, then discard time NetEQ already concealed. This
        // also keeps Opus's predictive state in sync after burst loss.
        if bytes.len() < 8 {
            return Ok(Vec::new());
        }
        let seq = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        let bytes = &bytes[8..];
        let mut state = self.repair.lock().unwrap();
        let gap = state
            .last_sequence
            .map_or(0, |last| seq.saturating_sub(last).saturating_sub(1));
        let packet_samples = self
            .decoder
            .get_nb_samples(bytes)
            .map_err(|e| neteq::NetEqError::InvalidPacket(e.to_string()))?;
        let samples = packet_samples * usize::from(self.channels);
        let mut out = Vec::with_capacity(samples * (gap.min(10) as usize + 1));
        if gap > 10 {
            let _ = self.decoder.reset_state();
        } else {
            for missing in 0..gap {
                let mut recovered = vec![0.0; samples];
                let fec = missing + 1 == gap;
                if fec {
                    state.statistics.fec_attempts += 1;
                } else {
                    state.statistics.plc_frames += 1;
                }
                if self
                    .decoder
                    .decode_float(if fec { bytes } else { &[] }, &mut recovered, fec)
                    .is_err()
                {
                    recovered.fill(0.0);
                }
                out.extend(recovered);
            }
        }
        let mut current = vec![0.0; samples];
        if self
            .decoder
            .decode_float(bytes, &mut current, false)
            .is_err()
            && self.decoder.decode_float(&[], &mut current, false).is_err()
        {
            current.fill(0.0);
        }
        out.extend(current);
        let skip = state.expanded_samples.min(out.len());
        state.statistics.late_samples_discarded += skip as u64;
        out.drain(..skip);
        state.expanded_samples = 0;
        state.last_sequence = Some(seq);
        Ok(out)
    }
}
struct Stream {
    playout: neteq::NetEq,
    channels: u8,
    volume: f32,
    speaking: bool,
    highest: Option<u64>,
    repair: Arc<Mutex<RepairState>>,
    tail_ticks: u8,
    last_timestamp: Option<u64>,
    packet_ms: u32,
    arrival: Option<Instant>,
    jitter_us: u32,
}
/// Quantile-delay adaptive jitter, acceleration/preemptive expansion/PLC from
/// `neteq` (not claimed to be Chromium's native NetEq). Sans-I/O, 10ms rendering;
/// packet timestamps are source 48kHz sample positions; arrival time is local.
pub struct VoiceMixer {
    streams: BTreeMap<u64, Stream>,
    jitter: usize,
    deafened: bool,
    events: BTreeMap<u64, bool>,
}
impl VoiceMixer {
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
        self.add(user, 1)
    }
    pub fn add_stereo_remote(&mut self, user: u64) -> Result<()> {
        self.add(user, 2)
    }
    fn add(&mut self, user: u64, channels: u8) -> Result<()> {
        if let Some(s) = self.streams.get(&user) {
            return if s.channels == channels {
                Ok(())
            } else {
                Err(Error::Invalid(
                    "stream channel count changed; re-register".into(),
                ))
            };
        }
        if self.streams.len() >= MAX_REMOTES {
            return Err(Error::Invalid("remote stream limit reached".into()));
        }
        let minimum = (self.jitter * 20) as u32;
        let mut playout = neteq::NetEq::new(neteq::NetEqConfig {
            sample_rate: SAMPLE_RATE,
            channels,
            min_delay_ms: minimum,
            max_delay_ms: minimum.max(80),
            max_packets_in_buffer: 12,
            ..Default::default()
        })
        .map_err(invalid)?;
        let decoder = opus::Decoder::new(
            SAMPLE_RATE,
            if channels == 2 {
                opus::Channels::Stereo
            } else {
                opus::Channels::Mono
            },
        )
        .map_err(invalid)?;
        let repair = Arc::new(Mutex::new(RepairState::default()));
        playout.register_decoder(
            111,
            Box::new(OpusPlayout {
                decoder,
                channels,
                repair: repair.clone(),
            }),
        );
        // Upstream starts at 80 ms regardless of our floor. Start at the explicit
        // floor and retain its adaptive quantile/time-stretch + 80 ms ceiling.
        playout.set_start_delay(minimum);
        self.streams.insert(
            user,
            Stream {
                playout,
                channels,
                volume: 1.0,
                speaking: false,
                highest: None,
                repair,
                tail_ticks: 0,
                last_timestamp: None,
                packet_ms: 20,
                arrival: None,
                jitter_us: 0,
            },
        );
        Ok(())
    }
    pub fn remove_remote(&mut self, user: u64) {
        self.streams.remove(&user);
        self.events.remove(&user);
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
    pub fn set_deafened(&mut self, value: bool) {
        self.deafened = value;
    }
    pub fn set_jitter_packets(&mut self, target: usize) -> Result<()> {
        if !(1..=6).contains(&target) {
            return Err(Error::Invalid("jitter must be 1..6 packets".into()));
        }
        self.jitter = target;
        for s in self.streams.values_mut() {
            s.playout.set_minimum_delay(target as u32 * s.packet_ms);
            s.playout.set_maximum_delay((target * 20).max(80) as u32);
        }
        Ok(())
    }
    pub fn flush(&mut self) {
        for s in self.streams.values_mut() {
            s.playout.flush();
            s.playout.set_start_delay(self.jitter as u32 * s.packet_ms);
            s.highest = None;
            let mut repair = s.repair.lock().unwrap();
            repair.last_sequence = None;
            repair.expanded_samples = 0;
        }
    }
    pub fn statistics(&self, user: u64) -> Option<PlayoutStatistics> {
        self.streams.get(&user).map(|s| {
            let stats = s.playout.get_statistics();
            PlayoutStatistics {
                current_buffer_size_ms: stats.current_buffer_size_ms,
                target_delay_ms: stats.target_delay_ms,
                packets_awaiting_decode: stats.packets_awaiting_decode,
                packets_per_sec: stats.packets_per_sec,
            }
        })
    }
    pub fn repair_statistics(&self, user: u64) -> Option<RepairStatistics> {
        self.streams
            .get(&user)
            .map(|s| s.repair.lock().unwrap().statistics.clone())
    }
    pub fn push(&mut self, user: u64, packet: AudioPacket) -> Result<()> {
        self.push_at(user, packet, Instant::now())
    }
    pub fn push_at(&mut self, user: u64, mut packet: AudioPacket, arrival: Instant) -> Result<()> {
        let s = self
            .streams
            .get_mut(&user)
            .ok_or_else(|| Error::Invalid("unknown remote".into()))?;
        let samples = opus::packet::get_nb_samples(&packet.data, SAMPLE_RATE).map_err(invalid)?;
        // RTP's 48 kHz clock wraps every ~24.9 hours. Extended callers are
        // unchanged; wire-u32 callers are unwrapped around the high-water mark.
        if packet.timestamp <= u64::from(u32::MAX) {
            if let Some(last) = s.last_timestamp {
                let candidate = (last & !u64::from(u32::MAX)) | packet.timestamp;
                packet.timestamp = if candidate.saturating_add(1 << 31) < last {
                    candidate + (1 << 32)
                } else if candidate > last.saturating_add(1 << 31) {
                    candidate.saturating_sub(1 << 32)
                } else {
                    candidate
                };
            }
        }
        if packet.data.is_empty()
            || packet.data.len() > MAX_PACKET_BYTES
            || ![TICK, PACKET_SAMPLES].contains(&samples)
            || (s.channels == 1
                && opus::packet::get_nb_channels(&packet.data).map_err(invalid)?
                    != opus::Channels::Mono)
        {
            return Err(Error::Invalid(
                "expected bounded 10/20ms Opus with registered channels".into(),
            ));
        }
        let packet_ms = samples as u32 / 48;
        if s.last_timestamp.is_none() || packet_ms != s.packet_ms {
            s.packet_ms = packet_ms;
            let minimum = self.jitter as u32 * packet_ms;
            s.playout.set_base_minimum_delay(packet_ms);
            s.playout.set_minimum_delay(minimum);
            if s.last_timestamp.is_none() {
                s.playout.set_start_delay(minimum);
            }
        }
        if let Some(highest) = s.highest {
            if packet.sequence.saturating_add(12) < highest {
                return Ok(());
            }
            if packet.sequence.saturating_sub(highest) > 250 {
                // A long real outage must recover without a call rejoin. Reset
                // rather than allocating PLC for the missing interval.
                s.playout.flush();
                s.playout.set_start_delay(self.jitter as u32 * s.packet_ms);
                let mut repair = s.repair.lock().unwrap();
                repair.last_sequence = None;
                repair.expanded_samples = 0;
                s.last_timestamp = None;
                s.arrival = None;
            }
        }
        if let (Some(last), Some(timestamp)) = (s.arrival, s.last_timestamp) {
            if packet.timestamp > timestamp && packet.timestamp <= timestamp + 4800 {
                let source_us = (packet.timestamp - timestamp) * 1_000_000 / 48000;
                let delta = arrival
                    .saturating_duration_since(last)
                    .as_micros()
                    .min(1_000_000) as u64;
                let error = delta.abs_diff(source_us).min(1_000_000) as u32;
                s.jitter_us = (s.jitter_us * 15 + error) / 16;
            }
        }
        if s.last_timestamp.is_none_or(|t| packet.timestamp > t) {
            s.arrival = Some(arrival);
        }
        if (s.tail_ticks == 0 && packet.data.len() > 2 && s.last_timestamp.is_some())
            || s.last_timestamp
                .is_some_and(|t| packet.timestamp > t + 4800)
        {
            // Resume after DTX without generating concealment for intentionally
            // unsent silence or retaining an old target/buffer backlog.
            s.playout.flush();
            s.playout.set_start_delay(self.jitter as u32 * s.packet_ms);
            let mut repair = s.repair.lock().unwrap();
            repair.last_sequence = None;
            repair.expanded_samples = 0;
        }
        s.last_timestamp = Some(s.last_timestamp.unwrap_or(0).max(packet.timestamp));
        s.tail_ticks = if packet.data.len() <= 2 {
            s.tail_ticks.min(6)
        } else {
            20
        };
        s.highest = Some(s.highest.unwrap_or(packet.sequence).max(packet.sequence));
        let mut data = packet.sequence.to_le_bytes().to_vec();
        data.extend_from_slice(&packet.data);
        let mut packet = neteq::AudioPacket::new(
            neteq::RtpHeader::new(
                packet.sequence as u16,
                packet.timestamp as u32,
                1,
                111,
                false,
            ),
            data,
            SAMPLE_RATE,
            s.channels,
            (samples / 48) as u32,
        );
        packet.arrival_time = arrival;
        s.playout.insert_packet(packet).map_err(invalid)
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
    pub fn network_jitter_ms(&self) -> u32 {
        self.streams
            .values()
            .map(|s| s.jitter_us.div_ceil(1000))
            .max()
            .unwrap_or(0)
    }
    pub fn render(&mut self) -> Result<[f32; TICK]> {
        let s = self.render_stereo()?;
        Ok(std::array::from_fn(|i| (s[i * 2] + s[i * 2 + 1]) * 0.5))
    }
    pub fn render_stereo(&mut self) -> Result<[f32; TICK * 2]> {
        let mut mixed = [0.0; TICK * 2];
        for (&user, s) in &mut self.streams {
            if s.tail_ticks == 0 {
                if s.speaking {
                    s.speaking = false;
                    self.events.insert(user, false);
                }
                continue;
            }
            s.tail_ticks -= 1;
            let frame = match s.playout.get_audio() {
                Ok(frame) => frame,
                Err(_) => {
                    if s.speaking {
                        s.speaking = false;
                        self.events.insert(user, false);
                    }
                    continue;
                }
            };
            if matches!(frame.speech_type, neteq::neteq::SpeechType::Expand) {
                let mut repair = s.repair.lock().unwrap();
                if repair.last_sequence.is_some() {
                    repair.expanded_samples = repair
                        .expanded_samples
                        .saturating_add(TICK * usize::from(s.channels))
                        .min(48000 * usize::from(s.channels) * 5);
                }
            }
            if frame.samples.len() != TICK * usize::from(s.channels) {
                continue;
            }
            let speaking = rms_dbfs(&frame.samples) >= -50.0;
            if speaking != s.speaking {
                s.speaking = speaking;
                self.events.insert(user, speaking);
            }
            if !self.deafened {
                for i in 0..TICK {
                    let p = i * usize::from(s.channels);
                    mixed[i * 2] += frame.samples[p] * s.volume;
                    mixed[i * 2 + 1] += frame.samples[p + usize::from(s.channels) - 1] * s.volume;
                }
            }
        }
        for sample in &mut mixed {
            *sample = sample.clamp(-1.0, 1.0);
        }
        Ok(mixed)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_native_stereo_playout_preserves_both_packet_durations() {
        for packet_ticks in [1usize, 2] {
            let mut mixer = VoiceMixer::new(1).unwrap();
            // WASAPI renders stereo even when the remote voice codec is mono.
            mixer.add_stereo_remote(1).unwrap();
            let mut encoder = super::super::OpusEncoder::new(false, 64000).unwrap();
            let origin = Instant::now();
            let mut energetic_ticks = 0;
            for tick in 0..300usize {
                if tick.is_multiple_of(packet_ticks) {
                    let pcm: Vec<_> = (0..TICK * packet_ticks)
                        .map(|i| ((tick * TICK + i) as f32 * 0.035).sin() * 0.1)
                        .collect();
                    let packet = encoder.encode(&pcm, (tick * TICK) as u64).unwrap();
                    mixer
                        .push_at(
                            1,
                            packet,
                            origin + std::time::Duration::from_millis(tick as u64 * 10),
                        )
                        .unwrap();
                }
                let out = mixer.render_stereo().unwrap();
                if tick > 10 && rms_dbfs(&out) > -40.0 {
                    energetic_ticks += 1;
                }
                assert!(out
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|s| (s[0] - s[1]).abs() < 0.001));
            }
            assert!(
                energetic_ticks >= 285,
                "packet_ticks={packet_ticks}, energetic={energetic_ticks}"
            );
            assert_eq!(
                mixer.statistics(1).unwrap().target_delay_ms,
                packet_ticks as u32 * 10
            );
            assert_eq!(
                mixer.repair_statistics(1).unwrap().late_samples_discarded,
                0
            );
        }
    }
    #[test]
    fn clock_wrap_and_long_outage_recover_without_allocating_gap_audio() {
        let mut mixer = VoiceMixer::new(1).unwrap();
        mixer.add_remote(1).unwrap();
        let mut encoder = super::super::OpusEncoder::new(false, 64000).unwrap();
        let pcm = [0.1; TICK];
        mixer
            .push(1, encoder.encode(&pcm, (1u64 << 32) - 480).unwrap())
            .unwrap();
        mixer.push(1, encoder.encode(&pcm, 0).unwrap()).unwrap();
        assert_eq!(mixer.streams[&1].last_timestamp, Some(1u64 << 32));
        let mut resumed = encoder.encode(&pcm, 480000).unwrap();
        resumed.sequence = 1000;
        mixer.push(1, resumed).unwrap();
        assert!(mixer.statistics(1).unwrap().packets_awaiting_decode <= 12);
        assert!(mixer.render().unwrap().iter().all(|s| s.is_finite()));
    }
    #[test]
    fn telemetry_reports_per_channel_milliseconds_for_mono_and_stereo() {
        for stereo in [false, true] {
            let mut mixer = VoiceMixer::new(1).unwrap();
            if stereo {
                mixer.add_stereo_remote(1).unwrap();
            } else {
                mixer.add_remote(1).unwrap();
            }
            let mut encoder = super::super::OpusEncoder::new(stereo, 64000).unwrap();
            let pcm = vec![0.0; PACKET_SAMPLES * if stereo { 2 } else { 1 }];
            mixer.push(1, encoder.encode(&pcm, 0).unwrap()).unwrap();
            assert_eq!(mixer.statistics(1).unwrap().current_buffer_size_ms, 20);
        }
    }
    #[test]
    fn adaptive_bounds_stereo_deafen_and_loss_do_not_create_devices() {
        let mut mixer = VoiceMixer::new(1).unwrap();
        mixer.add_stereo_remote(7).unwrap();
        let mut encoder = super::super::OpusEncoder::new(true, 192000).unwrap();
        for n in 0..40 {
            let pcm =
                std::array::from_fn::<_, 1920, _>(|i| ((i + n * 1920) as f32 * 0.07).sin() * 0.1);
            if n % 7 != 3 {
                let _ = mixer.push(7, encoder.encode(&pcm, n as u64 * 960).unwrap());
            }
            let out = mixer.render_stereo().unwrap();
            assert!(out.iter().all(|s| s.is_finite()));
            assert!(mixer.statistics(7).unwrap().packets_awaiting_decode <= 12);
        }
        let stats = mixer.statistics(7).unwrap();
        assert!(stats.target_delay_ms <= 80);
        mixer.set_deafened(true);
        assert!(mixer.render_stereo().unwrap().iter().all(|&s| s == 0.0));
        assert!(mixer.set_volume(7, 201).is_err());
        mixer.set_volume(7, 200).unwrap();
        mixer.flush();
        mixer.remove_remote(7);
        assert!(mixer.statistics(7).is_none());
    }
}

#[cfg(test)]
mod repair_tests {
    use super::*;
    use neteq::codec::AudioDecoder;
    #[test]
    fn opus_fec_restores_gaps_without_replaying_already_concealed_time() {
        let repair = Arc::new(Mutex::new(RepairState::default()));
        let mut decoder = OpusPlayout {
            decoder: opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono).unwrap(),
            channels: 1,
            repair: repair.clone(),
        };
        let mut encoder = super::super::OpusEncoder::new(false, 64000).unwrap();
        let mut packets = Vec::new();
        for seq in 0..6u64 {
            let pcm = std::array::from_fn::<_, 960, _>(|i| {
                ((seq as usize * 960 + i) as f32 * 0.035).sin() * 0.1
            });
            let packet = encoder.encode(&pcm, seq * 960).unwrap();
            let mut data = seq.to_le_bytes().to_vec();
            data.extend(packet.data);
            packets.push(data);
        }
        assert_eq!(decoder.decode(&packets[0]).unwrap().len(), 960);
        assert_eq!(decoder.decode(&packets[2]).unwrap().len(), 1920);
        repair.lock().unwrap().expanded_samples = 960;
        assert_eq!(decoder.decode(&packets[4]).unwrap().len(), 960);
        repair.lock().unwrap().expanded_samples = 960;
        assert!(decoder.decode(&packets[5]).unwrap().is_empty());
        let state = repair.lock().unwrap();
        assert_eq!(state.statistics.fec_attempts, 2);
        assert_eq!(state.statistics.late_samples_discarded, 1920);
    }
}
