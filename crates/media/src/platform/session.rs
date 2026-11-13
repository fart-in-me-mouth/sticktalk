use super::{
    encoder::Encoder,
    gpu::{Converter, Gpu},
    runtime::Runtime,
    source::Input,
};
use crate::{
    annexb, quality::validate_bitrate, EncodedFrame, EncoderKind, EncoderSelection, Error, Quality,
    Result, SessionStats, ShareOptions, Size, Source,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, Receiver, RecvTimeoutError, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Controls {
    quality: Option<(Quality, u32)>,
    bitrate: Option<u32>,
    cursor: Option<bool>,
}
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    keyframe: AtomicBool,
    controls: Mutex<Controls>,
    stats: Mutex<SessionStats>,
    failure: Mutex<Option<Error>>,
}

/// One worker and a three-frame bounded output channel. Dropping or stopping a
/// session closes WGC and releases codecs/device. `recv_timeout` reports worker
/// failures; it does not turn a stalled capture into fake successful frames.
pub struct ShareSession {
    receiver: Receiver<Result<EncodedFrame>>,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    pub encoder_kind: EncoderKind,
    pub effective_quality: Quality,
    pub size: Size,
    pub gpu_name: String,
    pub fallback_notice: Option<String>,
}
struct Started {
    kind: EncoderKind,
    quality: Quality,
    size: Size,
    gpu: String,
    notice: Option<String>,
}
impl ShareSession {
    pub fn start(source: Source, quality: Quality, bitrate: u32) -> Result<Self> {
        Self::start_with_options(
            source,
            ShareOptions {
                quality,
                bitrate,
                ..ShareOptions::default()
            },
        )
    }
    pub fn start_with_options(source: Source, options: ShareOptions) -> Result<Self> {
        Self::start_input(source, options, false)
    }
    /// Headless GPU-generated moving frames through the production paced
    /// BGRA -> NV12 -> hardware MFT path. Never opens WGC or an OS window.
    pub fn start_synthetic(size: Size, options: ShareOptions) -> Result<Self> {
        size.validate()?;
        Self::start_input(
            Source {
                kind: crate::SourceKind::Monitor,
                id: 0,
                name: "Synthetic GPU motion".into(),
                size,
                primary: false,
            },
            options,
            true,
        )
    }
    fn start_input(source: Source, options: ShareOptions, synthetic: bool) -> Result<Self> {
        validate_bitrate(options.bitrate)?;
        if options.gop_frames == 0 || options.gop_frames > 36_000 {
            return Err(Error::Invalid("GOP must be 1..36000 frames".into()));
        }
        options.quality.output_size(source.size)?;
        let shared = Arc::new(Shared::default());
        let state = shared.clone();
        let (sender, receiver) = sync_channel(3);
        let (ready_tx, ready_rx) = sync_channel(1);
        let worker = thread::Builder::new()
            .name("sticktalk-share".into())
            .spawn(move || {
                let result = run(
                    source,
                    options,
                    state.clone(),
                    &sender,
                    &ready_tx,
                    synthetic,
                );
                if let Err(error) = result {
                    // Preserve terminal failures even if the bounded frame queue
                    // is full or start() has not yet dropped its handshake receiver.
                    *state.failure.lock().unwrap() = Some(error.clone());
                    let _ = ready_tx.try_send(Err(error.clone()));
                    let _ = sender.try_send(Err(error));
                }
            })
            .map_err(|e| Error::Platform(e.to_string()))?;
        let started = match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(started)) => started,
            Ok(Err(error)) => {
                let _ = worker.join();
                return Err(error);
            }
            Err(_) => {
                shared.stop.store(true, Ordering::Release);
                let _ = worker.join();
                return Err(Error::Timeout("starting capture/encoder"));
            }
        };
        Ok(Self {
            receiver,
            shared,
            worker: Some(worker),
            encoder_kind: started.kind,
            effective_quality: started.quality,
            size: started.size,
            gpu_name: started.gpu,
            fallback_notice: started.notice,
        })
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<EncodedFrame> {
        match self.receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(Error::Timeout("waiting for encoded frame")),
            Err(RecvTimeoutError::Disconnected) => Err(self
                .shared
                .failure
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Error::Stopped)),
        }
    }
    /// Coalesces rapid changes. Bitrate-only updates use in-place rate control
    /// with 15% hysteresis and a two-second minimum interval. Format changes
    /// prepare a replacement on an MTA worker while the old encoder continues,
    /// committing only on the replacement IDR/SPS/PPS;
    /// capture is never restarted. Briefly owns two encoder surface pools.
    /// Failure keeps the old stream and reports an error through recv_timeout.
    pub fn set_quality(&self, quality: Quality, bitrate: u32) -> Result<()> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Stopped);
        }
        validate_bitrate(bitrate)?;
        quality.output_size(self.size)?;
        self.shared.stats.lock().unwrap().quality_requests += 1;
        self.shared.controls.lock().unwrap().quality = Some((quality, bitrate));
        Ok(())
    }
    /// A congestion ceiling, independent of the user's chosen quality target.
    /// Coalesced with hysteresis on the encoder worker; never rebuilds the MFT.
    pub fn set_bitrate(&self, bitrate: u32) -> Result<()> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Stopped);
        }
        validate_bitrate(bitrate)?;
        self.shared.controls.lock().unwrap().bitrate = Some(bitrate);
        Ok(())
    }
    pub fn request_keyframe(&self) {
        self.shared.stats.lock().unwrap().keyframe_requests += 1;
        self.shared.keyframe.store(true, Ordering::Release);
    }
    pub fn set_cursor(&self, enabled: bool) {
        self.shared.controls.lock().unwrap().cursor = Some(enabled);
    }
    pub fn stats(&self) -> SessionStats {
        self.shared.stats.lock().unwrap().clone()
    }
    pub fn stop(&mut self) -> Result<()> {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| Error::Platform("capture worker panicked".into()))?;
        }
        Ok(())
    }
}
impl Drop for ShareSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct Prepared {
    encoder: Encoder,
    converter: Converter,
    options: ShareOptions,
    native: Size,
    revision: u64,
}
// SAFETY: both preparation and capture threads enter the SAME process MTA.
// MF transforms are used exclusively after join, never concurrently. D3D11's
// immediate context is explicitly multithread-protected by Gpu::new.
unsafe impl Send for Prepared {}
/// Always join preparation on cancellation/stop; never leave a detached codec
/// creation task or its GPU/MF references running after ShareSession::stop.
struct Preparation(Option<JoinHandle<Result<Prepared>>>);
impl Drop for Preparation {
    fn drop(&mut self) {
        if let Some(worker) = self.0.take() {
            let _ = worker.join();
        }
    }
}
struct Retirement(Option<JoinHandle<()>>);
impl Drop for Retirement {
    fn drop(&mut self) {
        if let Some(worker) = self.0.take() {
            let _ = worker.join();
        }
    }
}
fn retire(old: Prepared) -> Result<Retirement> {
    thread::Builder::new()
        .name("sticktalk-encoder-retire".into())
        .spawn(move || {
            let _runtime = Runtime::new();
            drop(old);
        })
        .map(|w| Retirement(Some(w)))
        .map_err(|e| Error::Platform(e.to_string()))
}
fn prepare(
    gpu: Gpu,
    native: Size,
    options: ShareOptions,
    software: bool,
    revision: u64,
) -> Result<Preparation> {
    let worker = thread::Builder::new()
        .name("sticktalk-encoder-prepare".into())
        .spawn(move || {
            let _runtime = Runtime::new()?;
            let size = if software {
                options.quality.software_output_size(native)?
            } else {
                options.quality.output_size(native)?
            };
            let mut encoder = Encoder::new(&gpu, size, &options, software)?;
            encoder.keyframe()?;
            let converter = Converter::new(&gpu, native, size, options.quality.fps.get())?;
            Ok(Prepared {
                encoder,
                converter,
                options,
                native,
                revision,
            })
        })
        .map_err(|e| Error::Platform(e.to_string()))?;
    Ok(Preparation(Some(worker)))
}

fn run(
    source: Source,
    mut options: ShareOptions,
    shared: Arc<Shared>,
    sender: &std::sync::mpsc::SyncSender<Result<EncodedFrame>>,
    ready: &std::sync::mpsc::SyncSender<Result<Started>>,
    synthetic: bool,
) -> Result<()> {
    let _runtime = Runtime::new()?;
    let gpu = Gpu::new()?;
    let mut capture = Input::new(&gpu, &source, options.cursor, synthetic)?;
    let mut size = options.quality.output_size(capture.size())?;
    let mut notice = None;
    let mut software = options.encoder_selection == EncoderSelection::SoftwareOnly;
    let mut encoder = crate::selection::select_with_fallback(
        options.encoder_selection,
        || Encoder::new(&gpu, size, &options, false),
        |error| {
            let mut fallback = options.clone();
            fallback.quality = fallback.quality.software_cap();
            let fallback_size = fallback.quality.software_output_size(capture.size())?;
            notice = Some(match error {
                Some(error) => {
                    format!("Hardware unavailable ({error}); OpenH264 fallback capped to 720p30")
                }
                None => "Explicit OpenH264 software encoder; capped to 720p30".into(),
            });
            Encoder::new(&gpu, fallback_size, &fallback, true)
        },
    )?;
    if encoder.kind == EncoderKind::Software {
        software = true;
        options.quality = options.quality.software_cap();
        size = options.quality.software_output_size(capture.size())?;
    }
    let mut converter = Converter::new(&gpu, capture.size(), size, options.quality.fps.get())?;
    ready
        .send(Ok(Started {
            kind: encoder.kind.clone(),
            quality: options.quality,
            size,
            gpu: gpu.name.clone(),
            notice,
        }))
        .map_err(|_| Error::Stopped)?;
    let pacer = super::pacer::Pacer::new()?;
    let epoch = Instant::now();
    let mut tick = epoch;
    let mut generation = 0;
    let mut last_frame = None;
    let mut last_capture = Instant::now();
    let mut force_keyframe = true;
    let mut desired = options.clone();
    let mut quality_bitrate = options.bitrate;
    let mut last_bitrate_update = Duration::ZERO;
    let mut bitrate_supported = true;
    let mut revision = 0;
    let mut pending: Option<Preparation> = None;
    let mut candidate: Option<Prepared> = None;
    let mut native = capture.size();
    let mut failed_revision = None;
    let mut retired: Option<Retirement> = None;
    while !shared.stop.load(Ordering::Acquire) {
        pacer.until(tick)?;
        // WGC notifications coalesce; frame acquisition is paced, not a second
        // high-refresh wake loop. The free-threaded frame pool owns arrivals.
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        {
            let mut controls = shared.controls.lock().unwrap();
            if let Some((quality, bitrate)) = controls.quality.take() {
                let quality = if software {
                    quality.software_cap()
                } else {
                    quality
                };
                if quality != desired.quality {
                    desired.quality = quality;
                    revision += 1;
                }
                quality_bitrate = bitrate;
                desired.bitrate = bitrate;
            }
            if let Some(bitrate) = controls.bitrate.take() {
                desired.bitrate = bitrate.min(quality_bitrate);
            }
            if let Some(cursor) = controls.cursor.take() {
                capture.set_cursor(cursor)?;
            }
        }
        if let Some((frame, texture, count)) = capture.next()? {
            shared.stats.lock().unwrap().captured += count;
            last_capture = Instant::now();
            last_frame = Some((frame, texture));
        } else {
            shared.stats.lock().unwrap().repeated += 1;
        }
        if converter.input_size != capture.size() {
            native = capture.size();
            revision += 1;
            // Keep the old output format streaming while the new encoder is
            // built. Only the converter's source rectangle must follow resize.
            converter = Converter::new(
                &gpu,
                native,
                converter.output_size,
                options.quality.fps.get(),
            )?;
        }
        if pending
            .as_ref()
            .is_some_and(|p| p.0.as_ref().is_some_and(|w| w.is_finished()))
        {
            let mut preparation = pending.take().unwrap();
            let result = preparation
                .0
                .take()
                .unwrap()
                .join()
                .map_err(|_| Error::Platform("encoder preparation panicked".into()))?;
            match result {
                Ok(prepared) if prepared.revision == revision => candidate = Some(prepared),
                Ok(stale) => {
                    retired = Some(retire(stale)?);
                } // Superseded request.
                Err(error) => {
                    // Keep the working encoder on a failed reconfiguration.
                    // Report once; this is NOT a terminal session failure.
                    let _ = sender.try_send(Err(error));
                    desired = options.clone();
                    failed_revision = Some(revision);
                }
            }
        }
        if candidate.as_ref().is_some_and(|p| p.revision != revision) {
            retired = Some(retire(candidate.take().unwrap())?);
        }
        if retired
            .as_ref()
            .is_some_and(|r| r.0.as_ref().is_some_and(|w| w.is_finished()))
        {
            retired = None;
        }
        let desired_size = if software {
            desired.quality.software_output_size(native)?
        } else {
            desired.quality.output_size(native)?
        };
        if bitrate_supported
            && desired.quality == options.quality
            && desired_size == converter.output_size
            && crate::bitrate::update_due(
                options.bitrate,
                desired.bitrate,
                epoch.elapsed().saturating_sub(last_bitrate_update),
            )
        {
            last_bitrate_update = epoch.elapsed();
            match encoder.set_bitrate(desired.bitrate) {
                Ok(()) => {
                    if std::env::var_os("STICKTALK_VIDEO_TRACE").is_some() {
                        eprintln!(
                            "VIDEO_TRACE bitrate_update seconds={:.3} from={} to={}",
                            epoch.elapsed().as_secs_f64(),
                            options.bitrate,
                            desired.bitrate
                        );
                    }
                    options.bitrate = desired.bitrate;
                    shared.stats.lock().unwrap().bitrate_updates += 1;
                }
                Err(error) => {
                    // Never turn unsupported live rate control into MFT restart
                    // churn. Preserve the reference chain and diagnose once.
                    bitrate_supported = false;
                    shared.stats.lock().unwrap().bitrate_update_failures += 1;
                    eprintln!("Live encoder bitrate update unavailable: {error}");
                }
            }
        }
        if pending.is_none()
            && candidate.is_none()
            && retired.is_none()
            && failed_revision != Some(revision)
            && (desired.quality != options.quality || desired_size != converter.output_size)
        {
            pending = Some(prepare(
                gpu.clone(),
                native,
                desired.clone(),
                software,
                revision,
            )?);
        }
        let fps = options.quality.fps.get();
        if let Some((_, texture)) = &last_frame {
            if force_keyframe || shared.keyframe.swap(false, Ordering::AcqRel) {
                encoder.keyframe()?;
                force_keyframe = false;
            }
            let timestamp = epoch.elapsed().as_nanos() as i64 / 100;
            let started = Instant::now();
            converter.convert(texture)?;
            let duration = 10_000_000 / i64::from(fps);
            let mut bytes = None;
            if let Some(next) = &mut candidate {
                next.converter.convert(texture)?;
                let encoded = next.encoder.encode(
                    &next.converter.texture,
                    timestamp,
                    10_000_000 / i64::from(next.options.quality.fps.get()),
                )?;
                if annexb::contains(&encoded, 5)
                    && annexb::contains(&encoded, 7)
                    && annexb::contains(&encoded, 8)
                {
                    // Commit only after an actual IDR + parameter sets, never
                    // emit a generation whose reference chain is unavailable.
                    bytes = Some(encoded);
                }
            }
            if bytes.is_some() {
                let next = candidate.take().unwrap();
                debug_assert_eq!(next.native, native);
                let old = Prepared {
                    encoder: std::mem::replace(&mut encoder, next.encoder),
                    converter: std::mem::replace(&mut converter, next.converter),
                    options: std::mem::replace(&mut options, next.options),
                    native,
                    revision,
                };
                // Driver shutdown can block: retire off the paced output path.
                // Do not start another preparation until retirement is joined,
                // bounding simultaneous encoder surface pools to two.
                retired = Some(retire(old)?);
                generation += 1;
                bitrate_supported = true;
                last_bitrate_update = epoch.elapsed();
                shared.stats.lock().unwrap().reconfigurations += 1;
            }
            let bytes = match bytes {
                Some(bytes) => bytes,
                None => encoder.encode(&converter.texture, timestamp, duration)?,
            };
            let duration = 10_000_000 / i64::from(options.quality.fps.get());
            let latency = started.elapsed();
            if bytes.len() > 8 * 1024 * 1024 {
                return Err(Error::Platform(
                    "encoded access unit exceeds 8 MiB bound".into(),
                ));
            }
            if !bytes.is_empty() {
                let keyframe = annexb::contains(&bytes, 5);
                let mut stats = shared.stats.lock().unwrap();
                stats.encoded += 1;
                stats.keyframes += u64::from(keyframe);
                stats.bytes += bytes.len() as u64;
                stats.max_access_unit_bytes = stats.max_access_unit_bytes.max(bytes.len());
                drop(stats);
                let frame = EncodedFrame {
                    data: bytes,
                    timestamp,
                    duration,
                    keyframe,
                    size: converter.output_size,
                    generation,
                    encode_latency: latency,
                };
                match sender.try_send(Ok(frame)) {
                    Ok(()) => (),
                    Err(TrySendError::Full(_)) => {
                        shared.stats.lock().unwrap().dropped += 1;
                        // Broken reference chain: the next delivered frame must
                        // be an IDR, not a dependent P frame after a queue drop.
                        force_keyframe = true;
                    }
                    Err(TrySendError::Disconnected(_)) => return Ok(()),
                }
            }
        } else if last_capture.elapsed() > Duration::from_secs(3) {
            return Err(Error::Timeout(
                "Windows Graphics Capture produced no frames",
            ));
        }
        let interval = Duration::from_secs_f64(1.0 / f64::from(options.quality.fps.get()));
        tick += interval;
        let now = Instant::now();
        if now > tick + interval {
            let missed = ((now - tick).as_secs_f64() / interval.as_secs_f64()) as u64;
            shared.stats.lock().unwrap().dropped += missed;
            tick += interval * missed.min(u64::from(u32::MAX)) as u32;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "headless GPU/MFT rate-control and format-change qualification"]
    fn live_bitrate_preserves_reference_chain_and_format_change_rebuilds_once() {
        let session = ShareSession::start_synthetic(
            Size {
                width: 1920,
                height: 1080,
            },
            ShareOptions::default(),
        )
        .unwrap();
        let mut before = 0u64;
        for _ in 0..120 {
            before += session
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .data
                .len() as u64;
        }
        // Sub-threshold noise cannot rebuild or change the encoder.
        for _ in 0..60 {
            session.set_bitrate(7_900_000).unwrap();
            session.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        assert_eq!(session.stats().bitrate_updates, 0);
        session.set_bitrate(2_000_000).unwrap();
        for _ in 0..180 {
            session.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        let mut after = 0u64;
        for _ in 0..120 {
            let frame = session.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(frame.generation, 0);
            after += frame.data.len() as u64;
        }
        let stats = session.stats();
        println!("live bitrate payload before={before} after={after}, stats={stats:?}");
        assert_eq!(stats.reconfigurations, 0);
        assert_eq!(stats.bitrate_updates, 1);
        assert_eq!(stats.bitrate_update_failures, 0);
        assert!(
            after < before * 3 / 5,
            "driver did not reduce actual encoded payload"
        );
        session
            .set_quality(
                Quality {
                    fps: crate::FrameRate::Fps30,
                    resolution: crate::Resolution::P720,
                },
                2_000_000,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let frame = session.recv_timeout(Duration::from_secs(3)).unwrap();
            if frame.generation != 0 {
                assert_eq!(frame.generation, 1);
                assert!(frame.keyframe);
                assert_eq!(
                    frame.size,
                    Size {
                        width: 1280,
                        height: 720
                    }
                );
                assert_eq!(session.stats().reconfigurations, 1);
                break;
            }
            assert!(Instant::now() < deadline, "format change timed out");
        }
    }

    #[test]
    fn invalid_config_is_rejected_before_creating_any_device() {
        let source = Source {
            kind: crate::SourceKind::Monitor,
            id: 0,
            name: String::new(),
            size: Size {
                width: 1920,
                height: 1080,
            },
            primary: true,
        };
        assert!(matches!(
            ShareSession::start(source.clone(), Quality::default(), 0),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            ShareSession::start_with_options(
                source,
                ShareOptions {
                    gop_frames: 0,
                    ..ShareOptions::default()
                }
            ),
            Err(Error::Invalid(_))
        ));
    }
}
