use super::{
    hotkeys,
    peer::{Peer, PeerOutput, Remote},
    rtp, Command, Config, Event,
};
use std::{
    collections::BTreeMap,
    net::UdpSocket,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};
use sticktalk_media::{self as media, voice::AudioPacket};
use sticktalk_proto::*;
use tokio::sync::mpsc::Sender;

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn emit(output: &Sender<Event>, event: Event) -> Result<(), String> {
    // Fail closed on overload, not an unbounded media/SDP backlog.
    output
        .try_send(event)
        .map_err(|_| "Call event queue unavailable".into())
}
struct Decode {
    assembler: rtp::Reassembler,
    decoder: media::Decoder,
    frames: u64,
    ssrc: Option<u32>,
    size: Option<media::Size>,
    generation: u64,
}

pub fn run(
    config: Config,
    commands: Receiver<Command>,
    output: &Sender<Event>,
) -> Result<(), String> {
    let mut settings: sticktalk_client_core::voice::VoiceSettings =
        serde_json::from_str(&config.settings_json).map_err(error)?;
    settings.validate()?;
    // Default/test hosts are loopback-only. Production may explicitly select a
    // reachable unicast/Tailscale interface; never bind wildcard or auto-detect.
    let socket = UdpSocket::bind((super::bind_address(&config.bind_ip)?, 0)).map_err(error)?;
    let mut peer = Peer::new(
        socket.local_addr().map_err(error)?,
        config.mtu,
        Instant::now(),
    )?;
    let voice = media::VoiceSession::start(
        media::VoiceDevices {
            input: settings.input_device_id.clone(),
            output: settings.output_device_id.clone(),
        },
        super::options(&settings)?,
    )
    .map_err(error)?;
    voice
        .set_gains(settings.input_volume, settings.output_volume)
        .map_err(error)?;
    let hooks = hotkeys::Hooks::start(settings.push_to_talk_key)?;
    let mut share: Option<media::ShareSession> = None;
    let mut loopback: Option<media::LoopbackSession> = None;
    let mut camera: Option<media::CameraSession> = None;
    let mut remote: BTreeMap<String, (Remote, u64)> = BTreeMap::new();
    let mut decoders: BTreeMap<String, Decode> = BTreeMap::new();
    let mut next_key = 1u64;
    let mut speaking = false;
    let mut gated = true;
    let mut buffer = [0; 65536];
    loop {
        for _ in 0..128 {
            let command = match commands.try_recv() {
                Ok(c) => c,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(_) => return Ok(()),
            };
            let result = (|| -> Result<(), String> {
                match command {
                    Command::Stop => return Err("__stop".into()),
                    Command::Gate(muted, deafened) => {
                        gated = muted || deafened;
                        voice.set_muted(muted).map_err(error)?;
                        voice.set_deafened(deafened).map_err(error)?;
                    }
                    Command::PushToTalk(pressed) => {
                        voice.set_push_to_talk(pressed).map_err(error)?
                    }
                    Command::Configure(s) => {
                        voice.configure(super::options(&s)?).map_err(error)?;
                        if s.push_to_talk_key != settings.push_to_talk_key {
                            voice.set_push_to_talk(false).map_err(error)?;
                            hooks.set_ptt_key(s.push_to_talk_key);
                        }
                        if s.input_device_id != settings.input_device_id
                            || s.output_device_id != settings.output_device_id
                        {
                            voice
                                .set_devices(media::VoiceDevices {
                                    input: s.input_device_id.clone(),
                                    output: s.output_device_id.clone(),
                                })
                                .map_err(error)?;
                        }
                        voice
                            .set_gains(s.input_volume, s.output_volume)
                            .map_err(error)?;
                        for (source, key) in remote.values() {
                            let u = s.user(source.publisher.user_id);
                            voice
                                .set_volume(*key, if u.muted { 0 } else { u.volume })
                                .map_err(error)?;
                        }
                        settings = s;
                    }
                    Command::Participants(own, participants) => {
                        let desired: Vec<_> = participants
                            .iter()
                            .filter(|p| Some(p.session) != own)
                            .flat_map(|p| {
                                p.tracks.iter().map(move |t| {
                                    format!(
                                        "{}:{}:{}",
                                        p.session.user_id.0, p.session.connection_id.0, t.mid
                                    )
                                })
                            })
                            .collect();
                        let removed: Vec<_> = remote
                            .keys()
                            .filter(|k| !desired.contains(k))
                            .cloned()
                            .collect();
                        for key in removed {
                            if let Some((_, id)) = remote.remove(&key) {
                                voice.remove_remote(id).map_err(error)?;
                            }
                            decoders.remove(&key);
                        }
                        decoders.retain(|key, _| desired.contains(key));
                        peer.remote_tracks(own, participants.into_iter());
                    }
                    Command::Event(VoiceEvent::Answer { sdp }) => peer.answer(&sdp)?,
                    Command::Event(VoiceEvent::Bitrate { mid, bps }) => {
                        match peer.publication_kind(&mid) {
                            Some(TrackKind::Audio) => voice
                                .adapt_bitrate(0, bps.min(u64::from(u32::MAX)) as u32)
                                .map_err(error)?,
                            Some(TrackKind::Screen) => {
                                if let Some(s) = &share {
                                    let q = super::quality(&settings)?;
                                    s.set_quality(q, bps.clamp(100_000, 100_000_000) as u32)
                                        .map_err(error)?;
                                }
                            }
                            _ => {}
                        }
                    }
                    Command::Event(VoiceEvent::Stats(stats)) => {
                        voice
                            .adapt_bitrate(
                                stats
                                    .loss_per_million
                                    .unwrap_or(0)
                                    .div_ceil(10_000)
                                    .min(100) as u8,
                                128_000,
                            )
                            .map_err(error)?;
                    }
                    Command::Event(VoiceEvent::ControlRevoked { .. }) => {
                        hooks.set_control_active(false)
                    }
                    Command::Event(VoiceEvent::ControlGranted { .. }) => {
                        hooks.set_control_active(true)
                    }
                    Command::Event(_) => {}
                    Command::Share(source, pid) => {
                        let q = super::quality(&settings)?;
                        let surface = match source.kind {
                            media::SourceKind::Monitor => ShareSurface::Monitor,
                            media::SourceKind::Window => ShareSurface::Window,
                        };
                        let session = media::ShareSession::start(
                            source.clone(),
                            q,
                            q.recommended_bitrate(source.size).map_err(error)?,
                        )
                        .map_err(error)?;
                        peer.publish(TrackKind::Screen, true, surface)?;
                        share = Some(session);
                        if let Some(pid) = pid {
                            // Process only: never silently substitute system-wide audio.
                            let audio = media::LoopbackSession::start(
                                media::LoopbackSource::Process { pid },
                                128_000,
                            )
                            .map_err(error)?;
                            loopback = Some(audio);
                        }
                        emit(output, Event::Sharing(true))?;
                    }
                    Command::StopShare => {
                        share = None;
                        loopback = None;
                        emit(output, Event::Signal(VoiceSignal::RevokeControl))?;
                        peer.publish(TrackKind::Screen, false, ShareSurface::Monitor)?;
                        peer.publish(TrackKind::SystemAudio, false, ShareSurface::Monitor)?;
                        emit(output, Event::Sharing(false))?;
                    }
                    Command::Quality(q) => {
                        if let Some(s) = &share {
                            s.set_quality(q, q.recommended_bitrate(s.size).map_err(error)?)
                                .map_err(error)?;
                        }
                    }
                    Command::Camera(device) => {
                        if let Some(d) = device {
                            let c = media::CameraSession::start(d, media::CameraOptions::default())
                                .map_err(error)?;
                            peer.publish(TrackKind::Camera, true, ShareSurface::Monitor)?;
                            camera = Some(c);
                        } else {
                            camera = None;
                            peer.publish(TrackKind::Camera, false, ShareSurface::Monitor)?;
                        }
                        emit(output, Event::Camera(camera.is_some()))?;
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                if e == "__stop" {
                    return Ok(());
                }
                emit(output, Event::Toast(e))?;
            }
            flush(
                &mut peer,
                &socket,
                output,
                &voice,
                &settings,
                &mut remote,
                &mut next_key,
                &mut decoders,
                &share,
                &camera,
            )?;
        }
        while let Ok(event) = hooks.receiver.try_recv() {
            if let hotkeys::Hotkey::PushToTalk(pressed) = event {
                voice.set_push_to_talk(pressed).map_err(error)?;
            }
            emit(output, Event::Hotkey(event))?;
        }
        for event in voice.take_device_events() {
            match event {
                media::AudioSessionEvent::DeviceSwitched { device, .. } => {
                    emit(output, Event::Toast(format!("Switched to {}", device.name)))?
                }
                // The engine is waiting on IMM notifications, not dead. Keep
                // the call joined so a replacement device can recover in place.
                media::AudioSessionEvent::Suspended(e) => emit(output, Event::Toast(e))?,
                _ => {}
            }
        }
        for event in voice.take_speaking_events() {
            if event.user.is_none() {
                speaking = event.speaking;
            }
            emit(output, Event::Speaking(event.user, event.speaking))?;
        }
        for error in voice.take_control_errors() {
            emit(output, Event::Toast(error.to_string()))?;
        }
        // Only active calls do this work. No media timer/device/thread survives Drop.
        for _ in 0..8 {
            match voice.recv_timeout(Duration::ZERO) {
                Ok(p) => {
                    // Do not leak pre-mute packets retained in the bounded
                    // capture queue while the audio worker applies its gate.
                    if gated {
                        continue;
                    }
                    peer.send(
                        TrackKind::Audio,
                        p.timestamp as u32,
                        &p.data,
                        true,
                        if speaking { -24 } else { -127 },
                        Instant::now(),
                    )?;
                    flush(
                        &mut peer,
                        &socket,
                        output,
                        &voice,
                        &settings,
                        &mut remote,
                        &mut next_key,
                        &mut decoders,
                        &share,
                        &camera,
                    )?;
                }
                Err(media::Error::Timeout(_)) => break,
                Err(e) => return Err(error(e)),
            }
        }
        if let Some(s) = &share {
            for _ in 0..3 {
                match s.recv_timeout(Duration::ZERO) {
                    Ok(frame) => send_frame(&mut peer, TrackKind::Screen, frame)?,
                    Err(media::Error::Timeout(_)) => break,
                    Err(e) => return Err(error(e)),
                }
            }
        }
        if let Some(c) = &camera {
            for _ in 0..3 {
                match c.recv_timeout(Duration::ZERO) {
                    Ok(frame) => send_frame(&mut peer, TrackKind::Camera, frame)?,
                    Err(media::Error::Timeout(_)) => break,
                    Err(e) => return Err(error(e)),
                }
            }
        }
        // Publish the separate stereo track after the screen offer is answered.
        if let Some(a) = &loopback {
            for _ in 0..8 {
                match a.recv_timeout(Duration::ZERO) {
                    Ok(p) => {
                        if peer
                            .send(
                                TrackKind::SystemAudio,
                                p.timestamp as u32,
                                &p.data,
                                true,
                                -24,
                                Instant::now(),
                            )
                            .is_err()
                        {
                            let _ =
                                peer.publish(TrackKind::SystemAudio, true, ShareSurface::Monitor);
                        }
                    }
                    Err(media::Error::Timeout(_)) => break,
                    Err(e) => return Err(error(e)),
                }
            }
        }
        flush(
            &mut peer,
            &socket,
            output,
            &voice,
            &settings,
            &mut remote,
            &mut next_key,
            &mut decoders,
            &share,
            &camera,
        )?;
        peer.reconcile()?;
        flush(
            &mut peer,
            &socket,
            output,
            &voice,
            &settings,
            &mut remote,
            &mut next_key,
            &mut decoders,
            &share,
            &camera,
        )?;
        let now = Instant::now();
        let wait = peer
            .deadline()
            .saturating_duration_since(now)
            .min(Duration::from_millis(5))
            .max(Duration::from_micros(100));
        socket.set_read_timeout(Some(wait)).map_err(error)?;
        match socket.recv_from(&mut buffer) {
            Ok((n, from)) => {
                let _ = peer.datagram(from, &buffer[..n], Instant::now());
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if Instant::now() >= peer.deadline() {
                    peer.timeout(Instant::now())?;
                }
            }
            Err(e) => return Err(error(e)),
        }
    }
}
fn send_frame(peer: &mut Peer, kind: TrackKind, frame: media::EncodedFrame) -> Result<(), String> {
    let packets = rtp::packetize(&frame.data, 1180)?;
    let count = packets.len();
    for (i, p) in packets.into_iter().enumerate() {
        peer.send(
            kind,
            (frame.timestamp as u64 * 9 / 1000) as u32,
            &p,
            i + 1 == count,
            -127,
            Instant::now(),
        )?;
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn flush(
    peer: &mut Peer,
    socket: &UdpSocket,
    output: &Sender<Event>,
    voice: &media::VoiceSession,
    settings: &sticktalk_client_core::voice::VoiceSettings,
    remote: &mut BTreeMap<String, (Remote, u64)>,
    next_key: &mut u64,
    decoders: &mut BTreeMap<String, Decode>,
    share: &Option<media::ShareSession>,
    camera: &Option<media::CameraSession>,
) -> Result<(), String> {
    for event in peer.drain()? {
        match event {
            PeerOutput::Signal(s) => emit(output, Event::Signal(s))?,
            PeerOutput::Datagram(to, bytes) => {
                socket.send_to(&bytes, to).map_err(error)?;
            }
            PeerOutput::Connected => emit(output, Event::Connected)?,
            PeerOutput::Keyframe(TrackKind::Screen) => {
                if let Some(s) = share {
                    s.request_keyframe();
                }
            }
            PeerOutput::Keyframe(TrackKind::Camera) => {
                if let Some(c) = camera {
                    c.request_keyframe();
                }
            }
            PeerOutput::Keyframe(_) => {}
            PeerOutput::Control(_) => {
                // Fail closed until an always-on-top native Stop banner and bounded
                // surface injector are installed together. No SendInput in this driver.
                emit(output, Event::Signal(VoiceSignal::RevokeControl))?;
            }
            PeerOutput::Packet(source, p) => {
                let key = format!(
                    "{}:{}:{}",
                    source.publisher.user_id.0, source.publisher.connection_id.0, source.track.mid
                );
                if matches!(source.track.kind, TrackKind::Audio | TrackKind::SystemAudio) {
                    if !remote.contains_key(&key) {
                        if remote.len() >= 32 {
                            continue;
                        }
                        let id = *next_key;
                        *next_key += 1;
                        voice.add_stereo_remote(id).map_err(error)?;
                        let u = settings.user(source.publisher.user_id);
                        voice
                            .set_volume(id, if u.muted { 0 } else { u.volume })
                            .map_err(error)?;
                        remote.insert(key.clone(), (source.clone(), id));
                    }
                    let id = remote[&key].1;
                    // str0m extends sequence numbers across RTP wrap for us. NetEQ
                    // unwraps the 32-bit 48k timestamp against extended sequence time.
                    let _ = voice.push_remote(
                        id,
                        AudioPacket {
                            sequence: *p.seq_no,
                            timestamp: u64::from(p.header.timestamp),
                            data: p.payload.to_vec(),
                        },
                    );
                } else {
                    if !decoders.contains_key(&key) {
                        if decoders.len() >= 8 {
                            continue;
                        }
                        decoders.insert(
                            key.clone(),
                            Decode {
                                assembler: rtp::Reassembler::default(),
                                decoder: media::Decoder::new().map_err(error)?,
                                frames: 0,
                                ssrc: None,
                                size: None,
                                generation: 0,
                            },
                        );
                    }
                    let d = decoders.get_mut(&key).unwrap();
                    let ssrc = *p.header.ssrc;
                    if d.ssrc != Some(ssrc) {
                        d.assembler = rtp::Reassembler::default();
                        d.ssrc = Some(ssrc);
                    }
                    if let Some((data, keyframe)) =
                        d.assembler
                            .push(*p.seq_no, p.header.timestamp, p.header.marker, &p.payload)
                    {
                        if keyframe {
                            if let Some(size) = rtp::dimensions(&data) {
                                if d.size != Some(size) {
                                    d.generation += 1;
                                    d.size = Some(size);
                                }
                            }
                        }
                        let Some(size) = d.size else {
                            continue;
                        };
                        let frame = media::EncodedFrame {
                            data,
                            timestamp: i64::from(p.header.timestamp) * 1000 / 9,
                            duration: 166_667,
                            keyframe,
                            size,
                            generation: d.generation,
                            encode_latency: Duration::ZERO,
                        };
                        let decoded = match d.decoder.decode(frame) {
                            Ok(frames) => frames,
                            Err(_) => {
                                peer.request_keyframe(&source);
                                continue;
                            }
                        };
                        d.frames += decoded.len() as u64;
                        if d.frames.is_multiple_of(60) {
                            emit(
                                output,
                                Event::Decoded {
                                    user: source.publisher.user_id,
                                    kind: source.track.kind,
                                    frames: d.frames,
                                },
                            )?;
                        }
                        // Surface leases are bounded to this iteration; renderer bridge
                        // must take ownership before this can be called display playback.
                    }
                }
            }
        }
    }
    Ok(())
}
