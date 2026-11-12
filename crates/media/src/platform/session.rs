use super::{
    capture::Capture,
    encoder::Encoder,
    gpu::{Converter, Gpu},
    runtime::Runtime,
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
                let result = run(source, options, state.clone(), &sender, &ready_tx);
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
    /// Coalesces rapid changes. Applied at the next scheduled frame as a new
    /// encoder generation starting with IDR/SPS/PPS; WGC is NOT torn down.
    /// Hardware MFT re-creation is intentional: not all vendors support dynamic
    /// SetInputType. Capture and its D3D device remain running throughout.
    pub fn set_quality(&self, quality: Quality, bitrate: u32) -> Result<()> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Stopped);
        }
        validate_bitrate(bitrate)?;
        quality.output_size(self.size)?;
        self.shared.controls.lock().unwrap().quality = Some((quality, bitrate));
        Ok(())
    }
    pub fn request_keyframe(&self) {
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

fn run(
    source: Source,
    mut options: ShareOptions,
    shared: Arc<Shared>,
    sender: &std::sync::mpsc::SyncSender<Result<EncodedFrame>>,
    ready: &std::sync::mpsc::SyncSender<Result<Started>>,
) -> Result<()> {
    let _runtime = Runtime::new()?;
    let gpu = Gpu::new()?;
    let mut capture = Capture::new(&gpu.device, &source, options.cursor)?;
    let mut size = options.quality.output_size(capture.size)?;
    let mut notice = None;
    let mut software = options.encoder_selection == EncoderSelection::SoftwareOnly;
    let mut encoder = if software {
        options.quality = options.quality.software_cap();
        size = options.quality.software_output_size(capture.size)?;
        notice = Some("Explicit OpenH264 software encoder; capped to 720p30".into());
        Encoder::new(&gpu, size, &options, true)?
    } else {
        match Encoder::new(&gpu, size, &options, false) {
            Ok(encoder) => encoder,
            Err(error) if options.encoder_selection == EncoderSelection::AllowSoftwareFallback => {
                notice = Some(format!(
                    "Hardware unavailable ({error}); OpenH264 fallback capped to 720p30"
                ));
                options.quality = options.quality.software_cap();
                size = options.quality.software_output_size(capture.size)?;
                software = true;
                Encoder::new(&gpu, size, &options, true)?
            }
            Err(error) => return Err(error),
        }
    };
    let mut converter = Converter::new(&gpu, capture.size, size, options.quality.fps.get())?;
    ready
        .send(Ok(Started {
            kind: encoder.kind.clone(),
            quality: options.quality,
            size,
            gpu: gpu.name.clone(),
            notice,
        }))
        .map_err(|_| Error::Stopped)?;
    let epoch = Instant::now();
    let mut tick = epoch;
    let mut generation = 0;
    let mut last_frame = None;
    let mut last_capture = Instant::now();
    let mut force_keyframe = true;
    while !shared.stop.load(Ordering::Acquire) {
        let now = Instant::now();
        if now < tick {
            let _ = capture
                .signals
                .recv_timeout((tick - now).min(Duration::from_millis(50)));
            continue;
        }
        let mut changed = false;
        {
            let mut controls = shared.controls.lock().unwrap();
            if let Some((quality, bitrate)) = controls.quality.take() {
                let quality = if software {
                    quality.software_cap()
                } else {
                    quality
                };
                changed = quality != options.quality || bitrate != options.bitrate;
                options.quality = quality;
                options.bitrate = bitrate;
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
        let native_changed = converter.input_size != capture.size;
        if changed || native_changed {
            size = if software {
                options.quality.software_output_size(capture.size)?
            } else {
                options.quality.output_size(capture.size)?
            };
            // Destroy old codec before allocating a replacement, bounding driver
            // surface pools. Every prior sample has already been emitted.
            drop(encoder);
            encoder = Encoder::new(&gpu, size, &options, software)?;
            converter = Converter::new(&gpu, capture.size, size, options.quality.fps.get())?;
            generation += 1;
            shared.stats.lock().unwrap().reconfigurations += 1;
            force_keyframe = true;
            if native_changed {
                last_frame = None;
            }
        }
        let fps = options.quality.fps.get();
        let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
        if let Some((_, texture)) = &last_frame {
            if force_keyframe || shared.keyframe.swap(false, Ordering::AcqRel) {
                encoder.keyframe()?;
                force_keyframe = false;
            }
            let timestamp = epoch.elapsed().as_nanos() as i64 / 100;
            let started = Instant::now();
            converter.convert(texture)?;
            let duration = 10_000_000 / i64::from(fps);
            let bytes = encoder.encode(&converter.texture, timestamp, duration)?;
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
                stats.bytes += bytes.len() as u64;
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
