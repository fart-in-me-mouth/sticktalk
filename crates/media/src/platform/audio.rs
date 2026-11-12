//! Event-driven WASAPI ownership. No device is opened until a session starts.
use crate::{voice::*, Error, Result};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use wasapi::{AudioClient, DeviceEnumerator, Direction, Role, SampleType, StreamMode, WaveFormat};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0},
    System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE},
};

fn audio_error(e: impl std::fmt::Display) -> Error {
    Error::Platform(format!("WASAPI: {e}"))
}
struct Apartment;
impl Apartment {
    fn new() -> Result<Self> {
        wasapi::initialize_mta().ok().map_err(audio_error)?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        wasapi::deinitialize();
    }
}

/// Auto-reset control/stop event, not a polling timer.
struct Wake(HANDLE);
unsafe impl Send for Wake {}
unsafe impl Sync for Wake {}
impl Wake {
    fn new() -> Result<Self> {
        Ok(Self(unsafe { CreateEventW(None, false, false, None)? }))
    }
    fn signal(&self) {
        let _ = unsafe { SetEvent(self.0) };
    }
}
impl Drop for Wake {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
fn wait(handles: &[HANDLE]) -> Result<()> {
    let result = unsafe { WaitForMultipleObjects(handles, false, INFINITE) };
    if result == WAIT_FAILED {
        return Err(windows::core::Error::from_win32().into());
    }
    if result.0 >= WAIT_OBJECT_0.0 + handles.len() as u32 {
        return Err(Error::Platform("unexpected WASAPI wait result".into()));
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioDirection {
    Input,
    Output,
}
#[derive(Clone, Debug)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub direction: AudioDirection,
    pub default_communications: bool,
    pub default_console: bool,
}
/// Enumeration does not open a stream; call from a media worker, not STA UI.
pub fn audio_devices() -> Result<Vec<AudioDevice>> {
    let _apartment = Apartment::new()?;
    let enumerator = DeviceEnumerator::new().map_err(audio_error)?;
    let mut found = Vec::new();
    for (direction, public) in [
        (Direction::Capture, AudioDirection::Input),
        (Direction::Render, AudioDirection::Output),
    ] {
        let default = enumerator
            .get_default_device_for_role(direction, Role::Communications)
            .ok()
            .and_then(|d| d.get_id().ok());
        let console = enumerator
            .get_default_device(direction)
            .ok()
            .and_then(|d| d.get_id().ok());
        let devices = enumerator
            .get_device_collection(direction)
            .map_err(audio_error)?;
        for device in &devices {
            let device = device.map_err(audio_error)?;
            let id = device.get_id().map_err(audio_error)?;
            found.push(AudioDevice {
                default_communications: default.as_ref() == Some(&id),
                default_console: console.as_ref() == Some(&id),
                id,
                name: device.get_friendlyname().map_err(audio_error)?,
                direction: public,
            });
        }
    }
    Ok(found)
}
#[derive(Clone, Debug)]
pub enum AudioDeviceEvent {
    Added(String),
    Removed(String),
    Changed(String),
    DefaultChanged {
        direction: AudioDirection,
        role: String,
        id: Option<String>,
    },
    /// Queue overflow: enumerate again. No silently stale device list.
    Rescan,
}
/// Opt-in IMMNotificationClient registration; thread sleeps on recv(), no
/// timers/devices. Drop unregisters callbacks and joins. Not needed outside calls
/// unless the settings UI is actively showing a device picker.
pub struct AudioDeviceWatcher {
    receiver: Receiver<AudioDeviceEvent>,
    overflow: Arc<AtomicBool>,
    stop: Option<SyncSender<()>>,
    worker: Option<JoinHandle<()>>,
}
impl AudioDeviceWatcher {
    pub fn start() -> Result<Self> {
        let (tx, receiver) = mpsc::sync_channel(32);
        let (stop, stopped) = mpsc::sync_channel(1);
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let overflow = Arc::new(AtomicBool::new(false));
        let lost = overflow.clone();
        let worker = thread::Builder::new()
            .name("sticktalk-audio-devices".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let _apartment = Apartment::new()?;
                    let enumerator = DeviceEnumerator::new().map_err(audio_error)?;
                    let emit = move |event| {
                        if tx.try_send(event).is_err() {
                            lost.store(true, Ordering::Release);
                        }
                    };
                    let emit = Arc::new(emit);
                    let mut cb = wasapi::DeviceEventCallbacks::new();
                    let e = emit.clone();
                    cb.set_device_added_callback(move |id| e(AudioDeviceEvent::Added(id)));
                    let e = emit.clone();
                    cb.set_device_removed_callback(move |id| e(AudioDeviceEvent::Removed(id)));
                    let e = emit.clone();
                    cb.set_device_state_callback(move |id, _| e(AudioDeviceEvent::Changed(id)));
                    let e = emit.clone();
                    cb.set_property_value_callback(move |id, _| e(AudioDeviceEvent::Changed(id)));
                    cb.set_default_device_callback(move |direction, role, id| {
                        emit(AudioDeviceEvent::DefaultChanged {
                            direction: if direction == Direction::Capture {
                                AudioDirection::Input
                            } else {
                                AudioDirection::Output
                            },
                            role: role.to_string(),
                            id,
                        })
                    });
                    let _registration = enumerator
                        .register_notification_callback(cb)
                        .map_err(audio_error)?;
                    let _ = ready_tx.send(Ok(()));
                    let _ = stopped.recv();
                    Ok(())
                })();
                if let Err(e) = result {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(audio_error)?;
        if let Err(error) = ready.recv().unwrap_or(Err(Error::Stopped)) {
            let _ = worker.join();
            return Err(error);
        }
        Ok(Self {
            receiver,
            overflow,
            stop: Some(stop),
            worker: Some(worker),
        })
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<AudioDeviceEvent> {
        if self.overflow.swap(false, Ordering::AcqRel) {
            return Ok(AudioDeviceEvent::Rescan);
        }
        self.receiver.recv_timeout(timeout).map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => Error::Timeout("audio device event"),
            _ => Error::Stopped,
        })
    }
}
impl Drop for AudioDeviceWatcher {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

struct Stream {
    client: AudioClient,
    event: wasapi::Handle,
}
impl Stream {
    fn open(mut client: AudioClient, direction: Direction, channels: usize) -> Result<Self> {
        let format = WaveFormat::new(
            32,
            32,
            SampleType::Float,
            SAMPLE_RATE as usize,
            channels,
            None,
        );
        client
            .initialize_client(
                &format,
                direction,
                StreamMode::EventsShared {
                    autoconvert: true,
                    buffer_duration_hns: 200_000,
                },
                None,
            )
            .map_err(audio_error)?;
        let event = client.set_get_eventhandle().map_err(audio_error)?;
        Ok(Self { client, event })
    }
    fn handle(&self) -> HANDLE {
        HANDLE(self.event.as_handle().0)
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}
fn endpoint(id: Option<&str>, direction: Direction) -> Result<AudioClient> {
    let e = DeviceEnumerator::new().map_err(audio_error)?;
    let d = if let Some(id) = id {
        e.get_device(id)
    } else {
        e.get_default_device_for_role(direction, Role::Communications)
    }
    .map_err(audio_error)?;
    if d.get_direction() != direction {
        return Err(Error::Invalid("audio endpoint direction mismatch".into()));
    }
    d.get_iaudioclient().map_err(audio_error)
}
#[derive(Clone, Debug, Default)]
pub struct VoiceDevices {
    pub input: Option<String>,
    pub output: Option<String>,
}
enum Command {
    Configure(VoiceOptions),
    Feedback(u8, u32),
    Add(u64, bool),
    Remove(u64),
    Volume(u64, u16),
    Packet(u64, AudioPacket, Instant),
}
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    muted: AtomicBool,
    deafened: AtomicBool,
    ptt: AtomicBool,
    failure: Mutex<Option<Error>>,
    speaking: Mutex<BTreeMap<Option<u64>, bool>>,
    control_errors: Mutex<VecDeque<Error>>,
    device_dirty: AtomicBool,
    devices: Mutex<Option<VoiceDevices>>,
    device_events: Mutex<VecDeque<AudioSessionEvent>>,
    meter: Mutex<Option<SyncSender<MicLevel>>>,
    monitor_gain: std::sync::atomic::AtomicU32,
}
/// A call owns exactly one WASAPI capture/render worker. Stop/Drop signals a
/// Win32 event, joins, then all clients close. No idle singleton audio engine.
/// Device invalidation is terminal: UI receives the error and may reopen using
/// its selected/default device after an IMMNotificationClient event.
pub struct VoiceSession {
    commands: SyncSender<Command>,
    receiver: Receiver<AudioPacket>,
    shared: Arc<Shared>,
    wake: Arc<Wake>,
    worker: Option<JoinHandle<()>>,
}
impl VoiceSession {
    pub fn start(devices: VoiceDevices, options: VoiceOptions) -> Result<Self> {
        Self::start_internal(devices, options, None)
    }
    fn start_internal(
        devices: VoiceDevices,
        options: VoiceOptions,
        meter: Option<SyncSender<MicLevel>>,
    ) -> Result<Self> {
        options.validate()?;
        let (commands, rx) = mpsc::sync_channel(128);
        let (tx, receiver) = mpsc::sync_channel(8);
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let shared = Arc::new(Shared::default());
        shared.ptt.store(meter.is_some(), Ordering::Release);
        *shared.meter.lock().unwrap() = meter;
        let state = shared.clone();
        let wake = Arc::new(Wake::new()?);
        let signal = wake.clone();
        let worker = thread::Builder::new()
            .name("sticktalk-voice".into())
            .spawn(move || {
                let result = voice_loop(devices, options, &state, &signal, rx, tx, &ready_tx);
                if let Err(error) = result {
                    *state.failure.lock().unwrap() = Some(error.clone());
                    let _ = ready_tx.try_send(Err(error));
                }
            })
            .map_err(audio_error)?;
        if let Err(error) = ready.recv().unwrap_or(Err(Error::Stopped)) {
            let _ = worker.join();
            return Err(error);
        }
        Ok(Self {
            commands,
            receiver,
            shared,
            wake,
            worker: Some(worker),
        })
    }
    fn command(&self, cmd: Command) -> Result<()> {
        if let Some(e) = self.shared.failure.lock().unwrap().as_ref() {
            return Err(e.clone());
        }
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Stopped);
        }
        self.commands.try_send(cmd).map_err(|e| match e {
            TrySendError::Full(_) => {
                Error::Invalid("audio command queue full; retry from network worker".into())
            }
            _ => Error::Stopped,
        })?;
        self.wake.signal();
        Ok(())
    }
    pub fn set_devices(&self, devices: VoiceDevices) -> Result<()> {
        for id in [&devices.input, &devices.output].into_iter().flatten() {
            if id.len() > 4096 || id.contains('\0') {
                return Err(Error::Invalid("invalid endpoint ID".into()));
            }
        }
        *self.shared.devices.lock().unwrap() = Some(devices);
        self.shared.device_dirty.store(true, Ordering::Release);
        self.wake.signal();
        Ok(())
    }
    pub fn take_device_events(&self) -> Vec<AudioSessionEvent> {
        self.shared
            .device_events
            .lock()
            .unwrap()
            .drain(..)
            .collect()
    }
    pub fn adapt_bitrate(&self, loss_percent: u8, available_bitrate: u32) -> Result<()> {
        if loss_percent > 100 {
            return Err(Error::Invalid("loss must be 0..100%".into()));
        }
        self.command(Command::Feedback(loss_percent, available_bitrate))
    }
    pub fn configure(&self, options: VoiceOptions) -> Result<()> {
        options.validate()?;
        self.command(Command::Configure(options))
    }
    pub fn set_muted(&self, muted: bool) -> Result<()> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Stopped);
        }
        self.shared.muted.store(muted, Ordering::Release);
        self.wake.signal();
        Ok(())
    }
    /// Deafen also gates the microphone, without overriding the user's own mute.
    pub fn set_deafened(&self, deafened: bool) -> Result<()> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Stopped);
        }
        self.shared.deafened.store(deafened, Ordering::Release);
        self.wake.signal();
        Ok(())
    }
    pub fn set_push_to_talk(&self, pressed: bool) -> Result<()> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Stopped);
        }
        self.shared.ptt.store(pressed, Ordering::Release);
        self.wake.signal();
        Ok(())
    }
    /// Bounded asynchronous control errors (unknown user, stream limit, etc.).
    /// Network packet validation failures are dropped, not call-fatal errors.
    pub fn take_control_errors(&self) -> Vec<Error> {
        self.shared
            .control_errors
            .lock()
            .unwrap()
            .drain(..)
            .collect()
    }
    pub fn add_remote(&self, user: u64) -> Result<()> {
        self.command(Command::Add(user, false))
    }
    pub fn add_stereo_remote(&self, track: u64) -> Result<()> {
        self.command(Command::Add(track, true))
    }
    pub fn remove_remote(&self, user: u64) -> Result<()> {
        self.command(Command::Remove(user))
    }
    pub fn set_volume(&self, user: u64, percent: u16) -> Result<()> {
        if percent > 200 {
            return Err(Error::Invalid("remote volume must be 0..200%".into()));
        }
        self.command(Command::Volume(user, percent))
    }
    pub fn push_remote(&self, user: u64, packet: AudioPacket) -> Result<()> {
        if packet.data.len() > MAX_PACKET_BYTES {
            return Err(Error::Invalid("Opus packet too large".into()));
        }
        self.command(Command::Packet(user, packet, Instant::now()))
    }
    pub fn take_speaking_events(&self) -> Vec<SpeakingEvent> {
        std::mem::take(&mut *self.shared.speaking.lock().unwrap())
            .into_iter()
            .map(|(user, speaking)| SpeakingEvent { user, speaking })
            .collect()
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<AudioPacket> {
        self.receiver.recv_timeout(timeout).map_err(|e| {
            self.shared
                .failure
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(match e {
                    mpsc::RecvTimeoutError::Timeout => Error::Timeout("voice packet"),
                    _ => Error::Stopped,
                })
        })
    }
    pub fn stop(&mut self) -> Result<()> {
        self.shared.stop.store(true, Ordering::Release);
        self.wake.signal();
        if let Some(w) = self.worker.take() {
            w.join().map_err(|_| audio_error("voice worker panicked"))?;
        }
        Ok(())
    }
}
impl Drop for VoiceSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
#[derive(Clone, Debug)]
pub enum AudioSessionEvent {
    DeviceSwitched {
        direction: AudioDirection,
        device: AudioDevice,
        fallback: bool,
    },
    Suspended(String),
    DeviceUnavailable {
        direction: AudioDirection,
        message: String,
    },
    Recovered,
}
#[derive(Clone, Copy, Debug)]
pub struct MicLevel {
    pub rms_dbfs: f32,
    pub peak_dbfs: f32,
    pub threshold_dbfs: f32,
    pub clipping: bool,
}
/// Explicit settings-test lifetime. Default monitor gain is ZERO: no sound until
/// the user enables loopback. The meter exists only while this object is open.
pub struct MicTestSession {
    voice: VoiceSession,
    levels: Receiver<MicLevel>,
}
impl MicTestSession {
    pub fn start(devices: VoiceDevices, mut options: VoiceOptions) -> Result<Self> {
        options.mode = InputMode::PushToTalk;
        options.echo_cancellation = false; // Monitoring the near-end must not train AEC to cancel itself.
        let (tx, levels) = mpsc::sync_channel(1);
        let voice = VoiceSession::start_internal(devices, options, Some(tx))?;
        Ok(Self { voice, levels })
    }
    pub fn set_loopback_gain(&self, gain: f32) -> Result<()> {
        if !gain.is_finite() || !(0.0..=1.0).contains(&gain) {
            return Err(Error::Invalid("mic-test monitor gain must be 0..1".into()));
        }
        self.voice
            .shared
            .monitor_gain
            .store(gain.to_bits(), Ordering::Release);
        self.voice.wake.signal();
        Ok(())
    }
    pub fn configure(&self, mut options: VoiceOptions) -> Result<()> {
        options.mode = InputMode::PushToTalk;
        options.echo_cancellation = false;
        self.voice.configure(options)
    }
    pub fn recv_level_timeout(&self, timeout: Duration) -> Result<MicLevel> {
        if self.voice.shared.stop.load(Ordering::Acquire) {
            return Err(Error::Stopped);
        }
        self.levels.recv_timeout(timeout).map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => Error::Timeout("mic-test meter"),
            _ => Error::Stopped,
        })
    }
    pub fn set_devices(&self, devices: VoiceDevices) -> Result<()> {
        self.voice.set_devices(devices)
    }
    pub fn take_device_events(&self) -> Vec<AudioSessionEvent> {
        self.voice.take_device_events()
    }
    pub fn stop(&mut self) -> Result<()> {
        self.voice.stop()
    }
}
fn device_event(shared: &Shared, event: AudioSessionEvent) {
    let mut events = shared.device_events.lock().unwrap();
    if events.len() == 16 {
        events.pop_front();
    }
    events.push_back(event);
}
fn choose_device(
    devices: &[AudioDevice],
    direction: AudioDirection,
    pinned: Option<&str>,
    fallback: bool,
) -> Option<AudioDevice> {
    let eligible = || devices.iter().filter(|d| d.direction == direction);
    if !fallback {
        if let Some(id) = pinned {
            if let Some(d) = eligible().find(|d| d.id == id) {
                return Some(d.clone());
            }
        }
    }
    eligible()
        .find(|d| d.default_console)
        .or_else(|| eligible().next())
        .cloned()
}
fn commands(
    commands: &Receiver<Command>,
    shared: &Shared,
    processor: &mut VoiceProcessor,
    mixer: &mut VoiceMixer,
) -> Result<()> {
    processor
        .set_muted(shared.muted.load(Ordering::Acquire) || shared.deafened.load(Ordering::Acquire));
    processor.set_push_to_talk(shared.ptt.load(Ordering::Acquire));
    mixer.set_deafened(shared.deafened.load(Ordering::Acquire));
    for command in commands.try_iter().take(128) {
        let result = match command {
            Command::Configure(o) => mixer
                .set_jitter_packets(o.jitter_packets)
                .and_then(|_| processor.configure(o)),
            Command::Feedback(loss, available) => {
                processor.adapt_bitrate(loss, available).map(|_| ())
            }
            Command::Add(id, stereo) => {
                if stereo {
                    mixer.add_stereo_remote(id)
                } else {
                    mixer.add_remote(id)
                }
            }
            Command::Remove(id) => {
                mixer.remove_remote(id);
                shared.speaking.lock().unwrap().remove(&Some(id));
                Ok(())
            }
            Command::Volume(id, v) => mixer.set_volume(id, v),
            Command::Packet(id, p, arrival) => {
                let _ = mixer.push_at(id, p, arrival);
                Ok(())
            }
        };
        if let Err(error) = result {
            let mut errors = shared.control_errors.lock().unwrap();
            if errors.len() == 16 {
                errors.pop_front();
            }
            errors.push_back(error);
        }
    }
    Ok(())
}
fn voice_loop(
    mut devices: VoiceDevices,
    options: VoiceOptions,
    shared: &Arc<Shared>,
    wake: &Arc<Wake>,
    inbox: Receiver<Command>,
    packets: SyncSender<AudioPacket>,
    ready: &SyncSender<Result<()>>,
) -> Result<()> {
    let _apartment = Apartment::new()?;
    let mut processor = VoiceProcessor::new(options.clone())?;
    let mut mixer = VoiceMixer::new(options.jitter_packets)?;
    let enumerator = DeviceEnumerator::new().map_err(audio_error)?;
    let dirty = {
        let state = shared.clone();
        let wake = wake.clone();
        Arc::new(move || {
            state.device_dirty.store(true, Ordering::Release);
            wake.signal();
        })
    };
    let mut callbacks = wasapi::DeviceEventCallbacks::new();
    let d = dirty.clone();
    callbacks.set_device_added_callback(move |_| d());
    let d = dirty.clone();
    callbacks.set_device_removed_callback(move |_| d());
    let d = dirty.clone();
    callbacks.set_device_state_callback(move |_, _| d());
    let d = dirty.clone();
    callbacks.set_default_device_callback(move |_, _, _| d());
    callbacks.set_property_value_callback(move |_, _| dirty());
    let _registration = enumerator
        .register_notification_callback(callbacks)
        .map_err(audio_error)?;
    let mut previous: Option<(Option<String>, Option<String>)> = None;
    let mut failures = 0;
    let mut suspended = false;
    let mut force_default = false;
    let mut ready_sent = false;
    while !shared.stop.load(Ordering::Acquire) {
        if suspended {
            wait(&[wake.0])?;
            if shared.stop.load(Ordering::Acquire) {
                break;
            }
            commands(&inbox, shared, &mut processor, &mut mixer)?;
            // Incoming RTP cannot cause an open/retry loop. Only a native device
            // notification or explicit selection wakes suspended device recovery.
            if !shared.device_dirty.swap(false, Ordering::AcqRel) {
                continue;
            }
            suspended = false;
            failures = 0;
            force_default = false;
        }
        shared.device_dirty.swap(false, Ordering::AcqRel);
        if let Some(update) = shared.devices.lock().unwrap().take() {
            devices = update;
            force_default = false;
        }
        let available = audio_devices()?;
        let input = choose_device(
            &available,
            AudioDirection::Input,
            devices.input.as_deref(),
            force_default,
        );
        let output = choose_device(
            &available,
            AudioDirection::Output,
            devices.output.as_deref(),
            force_default,
        );
        if input.is_none() && output.is_none() {
            device_event(
                shared,
                AudioSessionEvent::Suspended(
                    "No usable endpoints; waiting for device notification".into(),
                ),
            );
            if !ready_sent {
                let _ = ready.send(Ok(()));
                ready_sent = true;
            }
            suspended = true;
            continue;
        }
        let open = |device: Option<&AudioDevice>, direction: Direction| -> Option<Stream> {
            let device = device?;
            match Stream::open(endpoint(Some(&device.id), direction).ok()?, direction, 2).and_then(
                |stream| {
                    stream.client.start_stream().map_err(audio_error)?;
                    Ok(stream)
                },
            ) {
                Ok(stream) => Some(stream),
                Err(error) => {
                    device_event(
                        shared,
                        AudioSessionEvent::DeviceUnavailable {
                            direction: if direction == Direction::Capture {
                                AudioDirection::Input
                            } else {
                                AudioDirection::Output
                            },
                            message: error.to_string(),
                        },
                    );
                    None
                }
            }
        };
        let input_stream = open(input.as_ref(), Direction::Capture);
        let output_stream = open(output.as_ref(), Direction::Render);
        if input_stream.is_none() && output_stream.is_none() {
            if failures == 0 {
                failures = 1;
                force_default = true;
                continue;
            }
            device_event(
                shared,
                AudioSessionEvent::Suspended(
                    "Cannot open permitted endpoints; waiting for device notification".into(),
                ),
            );
            if !ready_sent {
                let _ = ready.send(Ok(()));
                ready_sent = true;
            }
            suspended = true;
            continue;
        }
        let current = (
            if input_stream.is_some() {
                input.as_ref().map(|d| d.id.clone())
            } else {
                None
            },
            if output_stream.is_some() {
                output.as_ref().map(|d| d.id.clone())
            } else {
                None
            },
        );
        if previous.as_ref().map(|p| &p.0) != Some(&current.0) {
            if let Some(device) = input.clone().filter(|_| input_stream.is_some()) {
                device_event(
                    shared,
                    AudioSessionEvent::DeviceSwitched {
                        direction: AudioDirection::Input,
                        fallback: devices.input.as_ref().is_some_and(|p| p != &device.id),
                        device,
                    },
                );
            } else {
                device_event(
                    shared,
                    AudioSessionEvent::DeviceUnavailable {
                        direction: AudioDirection::Input,
                        message: "Microphone unavailable; playback remains active".into(),
                    },
                );
            }
        }
        if previous.as_ref().map(|p| &p.1) != Some(&current.1) {
            if let Some(device) = output.clone().filter(|_| output_stream.is_some()) {
                device_event(
                    shared,
                    AudioSessionEvent::DeviceSwitched {
                        direction: AudioDirection::Output,
                        fallback: devices.output.as_ref().is_some_and(|p| p != &device.id),
                        device,
                    },
                );
            } else {
                device_event(
                    shared,
                    AudioSessionEvent::DeviceUnavailable {
                        direction: AudioDirection::Output,
                        message: "Output unavailable; capture remains active".into(),
                    },
                );
            }
        }
        if previous.is_some() {
            device_event(shared, AudioSessionEvent::Recovered);
            processor.reset_device_state();
            mixer.flush();
        }
        previous = Some(current.clone());
        if !ready_sent {
            ready.send(Ok(())).map_err(|_| Error::Stopped)?;
            ready_sent = true;
        }
        let capture = input_stream
            .as_ref()
            .map(|s| s.client.get_audiocaptureclient().map_err(audio_error))
            .transpose()?;
        let render = output_stream
            .as_ref()
            .map(|s| s.client.get_audiorenderclient().map_err(audio_error))
            .transpose()?;
        let mut pcm = VecDeque::new();
        let mut playback = VecDeque::new();
        let mut monitor = VecDeque::new();
        let mut raw = vec![0; 48000 * 8];
        let mut meter_ticks = 0u8;
        let device_started = Instant::now();
        let running = (|| -> Result<()> {
            while !shared.stop.load(Ordering::Acquire) {
                let mut handles = vec![wake.0];
                if let Some(stream) = &input_stream {
                    handles.push(stream.handle());
                }
                if let Some(stream) = &output_stream {
                    handles.push(stream.handle());
                }
                wait(&handles)?;
                if shared.stop.load(Ordering::Acquire) {
                    break;
                }
                commands(&inbox, shared, &mut processor, &mut mixer)?;
                if device_started.elapsed() > Duration::from_secs(1) {
                    failures = 0;
                }
                if shared.device_dirty.swap(false, Ordering::AcqRel) {
                    if let Some(update) = shared.devices.lock().unwrap().take() {
                        devices = update;
                        force_default = false;
                    }
                    let all = audio_devices()?;
                    let next_in = choose_device(
                        &all,
                        AudioDirection::Input,
                        devices.input.as_deref(),
                        force_default,
                    );
                    let next_out = choose_device(
                        &all,
                        AudioDirection::Output,
                        devices.output.as_deref(),
                        force_default,
                    );
                    if next_in.as_ref().map(|d| &d.id) != current.0.as_ref()
                        || next_out.as_ref().map(|d| &d.id) != current.1.as_ref()
                    {
                        return Ok(());
                    }
                }
                if shared.deafened.load(Ordering::Acquire) {
                    playback.clear();
                }
                if shared.monitor_gain.load(Ordering::Acquire) == 0 {
                    monitor.clear();
                }
                let count = match &output_stream {
                    Some(stream) => stream
                        .client
                        .get_available_space_in_frames()
                        .map_err(audio_error)? as usize,
                    None => 0,
                };
                if count > 48000 {
                    return Err(audio_error("render buffer >1 second"));
                }
                while playback.len() < count * 8 {
                    let mut mixed = mixer.render_stereo()?;
                    let gain = f32::from_bits(shared.monitor_gain.load(Ordering::Acquire));
                    if gain > 0.0 {
                        for i in 0..TICK {
                            let left = monitor.pop_front().unwrap_or(0.0) * gain;
                            let right = monitor.pop_front().unwrap_or(0.0) * gain;
                            mixed[i * 2] = (mixed[i * 2] + left).clamp(-1.0, 1.0);
                            mixed[i * 2 + 1] = (mixed[i * 2 + 1] + right).clamp(-1.0, 1.0);
                        }
                    }
                    processor.render(&std::array::from_fn(|i| {
                        (mixed[i * 2] + mixed[i * 2 + 1]) * 0.5
                    }))?;
                    for sample in mixed {
                        playback.extend(sample.to_le_bytes());
                    }
                }
                if let Some(render) = &render {
                    if count > 0 {
                        render
                            .write_to_device_from_deque(count, &mut playback, None)
                            .map_err(audio_error)?;
                    }
                }
                // AEC3's delay estimator remains enabled. A manual calibrated
                // delay is available on the sans-I/O API, not guessed here.
                if let Some(capture) = &capture {
                    while let Some(count) = capture.get_next_packet_size().map_err(audio_error)? {
                        if shared.stop.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        if count == 0 {
                            break;
                        }
                        if count > 48000 {
                            return Err(audio_error("capture buffer >1 second"));
                        }
                        let (frames, _) =
                            capture.read_from_device(&mut raw).map_err(audio_error)?;
                        for b in raw[..frames as usize * 8].as_chunks::<4>().0 {
                            pcm.push_back(f32::from_le_bytes(*b));
                        }
                        while pcm.len() >= TICK * 2 {
                            if shared.stop.load(Ordering::Acquire) {
                                return Ok(());
                            }
                            let mut frame = [0.0; TICK * 2];
                            for sample in &mut frame {
                                *sample = pcm.pop_front().unwrap();
                            }
                            if let Some(packet) = processor.capture_stereo(&frame)? {
                                let _ = packets.try_send(packet);
                            }
                            if let Some(event) = processor.take_speaking_event() {
                                shared
                                    .speaking
                                    .lock()
                                    .unwrap()
                                    .insert(event.user, event.speaking);
                            }
                            meter_ticks = (meter_ticks + 1) % 3;
                            if let Some(meter) = shared.meter.lock().unwrap().as_ref() {
                                if meter_ticks == 0 {
                                    let peak = frame.iter().fold(0.0f32, |p, s| p.max(s.abs()));
                                    let _ = meter.try_send(MicLevel {
                                        rms_dbfs: rms_dbfs(&frame),
                                        peak_dbfs: 20.0 * peak.max(1e-6).log10(),
                                        threshold_dbfs: processor.sensitivity_dbfs(),
                                        clipping: peak >= 0.999,
                                    });
                                }
                                if shared.monitor_gain.load(Ordering::Acquire) != 0 {
                                    monitor.extend(processor.last_processed_stereo());
                                    while monitor.len() > TICK * 8 {
                                        monitor.pop_front();
                                    }
                                }
                            }
                        }
                    }
                }
                for event in mixer.take_speaking_events() {
                    shared
                        .speaking
                        .lock()
                        .unwrap()
                        .insert(event.user, event.speaking);
                }
            }
            Ok(())
        })();
        drop(render);
        drop(capture);
        drop(output_stream);
        drop(input_stream);
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        match running {
            Ok(()) => {
                failures = 0;
                force_default = false;
            }
            Err(error) => {
                failures += 1;
                force_default = true;
                if failures > 1 {
                    device_event(shared, AudioSessionEvent::Suspended(error.to_string()));
                    suspended = true;
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub enum LoopbackSource {
    /// None uses the default render endpoint; Some pins an endpoint ID.
    System { output_device: Option<String> },
    /// Windows build 20348+. Captures ONLY this process tree, never silently
    /// substitutes whole-system capture on older OS/activation failure.
    Process { pid: u32 },
}
/// Optional screen-share audio, stereo 48 kHz Opus. Separate from microphone.
pub struct LoopbackSession {
    receiver: Receiver<AudioPacket>,
    shared: Arc<Shared>,
    wake: Arc<Wake>,
    worker: Option<JoinHandle<()>>,
}
impl LoopbackSession {
    pub fn start(source: LoopbackSource, bitrate: u32) -> Result<Self> {
        validate_audio_bitrate(bitrate)?;
        if matches!(source, LoopbackSource::Process { pid: 0 }) {
            return Err(Error::Invalid("process PID must be nonzero".into()));
        }
        let shared = Arc::new(Shared::default());
        let state = shared.clone();
        let wake = Arc::new(Wake::new()?);
        let signal = wake.clone();
        let (tx, receiver) = mpsc::sync_channel(8);
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("sticktalk-loopback".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let _apartment = Apartment::new()?;
                    let client = match source {
                        LoopbackSource::System { output_device } => {
                            let e = DeviceEnumerator::new().map_err(audio_error)?;
                            let d = if let Some(id) = output_device {
                                e.get_device(&id)
                            } else {
                                e.get_default_device(Direction::Render)
                            }
                            .map_err(audio_error)?;
                            if d.get_direction() != Direction::Render {
                                return Err(Error::Invalid(
                                    "system loopback requires render endpoint".into(),
                                ));
                            }
                            d.get_iaudioclient().map_err(audio_error)?
                        }
                        LoopbackSource::Process { pid } => {
                            AudioClient::new_application_loopback_client(pid, true)
                                .map_err(audio_error)?
                        }
                    };
                    let stream = Stream::open(client, Direction::Capture, 2)?;
                    let capture = stream
                        .client
                        .get_audiocaptureclient()
                        .map_err(audio_error)?;
                    let mut encoder = OpusEncoder::new(true, bitrate)?;
                    let mut raw = vec![0; 48_000 * 8];
                    let mut pcm = VecDeque::new();
                    let mut clock = 0;
                    let mut base_index = None;
                    let mut expected_index = 0;
                    stream.client.start_stream().map_err(audio_error)?;
                    ready_tx.send(Ok(())).map_err(|_| Error::Stopped)?;
                    while !state.stop.load(Ordering::Acquire) {
                        wait(&[signal.0, stream.handle()])?;
                        if state.stop.load(Ordering::Acquire) {
                            break;
                        }
                        while let Some(count) =
                            capture.get_next_packet_size().map_err(audio_error)?
                        {
                            if state.stop.load(Ordering::Acquire) {
                                return Ok(());
                            }
                            if count == 0 {
                                break;
                            }
                            if count > 48_000 {
                                return Err(audio_error("loopback buffer exceeds one second"));
                            }
                            let (frames, info) =
                                capture.read_from_device(&mut raw).map_err(audio_error)?;
                            let base = *base_index.get_or_insert(info.index);
                            if info.index != expected_index {
                                pcm.clear();
                                clock = info.index.saturating_sub(base);
                            }
                            expected_index = info.index + u64::from(frames);
                            for b in raw[..frames as usize * 8].as_chunks::<4>().0 {
                                pcm.push_back(f32::from_le_bytes(*b));
                            }
                            while pcm.len() >= PACKET_SAMPLES * 2 {
                                if state.stop.load(Ordering::Acquire) {
                                    return Ok(());
                                }
                                let frame: Vec<_> = pcm.drain(..PACKET_SAMPLES * 2).collect();
                                let packet = encoder.encode(&frame, clock)?;
                                clock += PACKET_SAMPLES as u64;
                                let _ = tx.try_send(packet);
                            }
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    *state.failure.lock().unwrap() = Some(error.clone());
                    let _ = ready_tx.try_send(Err(error));
                }
            })
            .map_err(audio_error)?;
        if let Err(e) = ready.recv().unwrap_or(Err(Error::Stopped)) {
            let _ = worker.join();
            return Err(e);
        }
        Ok(Self {
            receiver,
            shared,
            wake,
            worker: Some(worker),
        })
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<AudioPacket> {
        self.receiver.recv_timeout(timeout).map_err(|e| {
            self.shared
                .failure
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(match e {
                    mpsc::RecvTimeoutError::Timeout => {
                        Error::Timeout("loopback packet (source may be silent)")
                    }
                    _ => Error::Stopped,
                })
        })
    }
    pub fn stop(&mut self) -> Result<()> {
        self.shared.stop.store(true, Ordering::Release);
        self.wake.signal();
        if let Some(w) = self.worker.take() {
            w.join()
                .map_err(|_| audio_error("loopback worker panicked"))?;
        }
        Ok(())
    }
}
impl Drop for LoopbackSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    fn d(id: &str, direction: AudioDirection, default: bool) -> AudioDevice {
        AudioDevice {
            id: id.into(),
            name: format!("Friendly {id}"),
            direction,
            default_console: default,
            default_communications: false,
        }
    }
    #[test]
    fn follows_defaults_pins_falls_back_and_retains_output_without_mic() {
        let mut devices = vec![
            d("usb", AudioDirection::Input, true),
            d("bluetooth-a2dp", AudioDirection::Output, true),
            d("other-mic", AudioDirection::Input, false),
        ];
        assert_eq!(
            choose_device(&devices, AudioDirection::Input, None, false)
                .unwrap()
                .id,
            "usb"
        );
        assert_eq!(
            choose_device(&devices, AudioDirection::Input, Some("other-mic"), false)
                .unwrap()
                .id,
            "other-mic"
        );
        assert_eq!(
            choose_device(&devices, AudioDirection::Output, None, false)
                .unwrap()
                .id,
            "bluetooth-a2dp"
        );
        devices.retain(|d| d.id != "other-mic");
        assert_eq!(
            choose_device(&devices, AudioDirection::Input, Some("other-mic"), false)
                .unwrap()
                .id,
            "usb"
        );
        devices.retain(|d| d.direction != AudioDirection::Input);
        assert!(choose_device(&devices, AudioDirection::Input, None, false).is_none());
        assert!(choose_device(&devices, AudioDirection::Output, None, false).is_some());
    }
}
