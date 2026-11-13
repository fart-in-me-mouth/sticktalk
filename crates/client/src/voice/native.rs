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
    display: Option<media::DisplayBridge>,
    dropped: u64,
    publisher: VoiceSession,
    kind: TrackKind,
    rate_at: Instant,
    rate_bytes: u64,
    rx_bps: u64,
}

pub fn run(
    config: Config,
    commands: Receiver<Command>,
    output: &Sender<Event>,
    emergency: super::Emergency,
) -> Result<(), String> {
    let mut settings: sticktalk_client_core::voice::VoiceSettings =
        serde_json::from_str(&config.settings_json).map_err(error)?;
    settings.validate()?;
    // Default/test hosts are loopback-only. Production may explicitly select a
    // reachable unicast/Tailscale interface; never bind wildcard or auto-detect.
    let socket = UdpSocket::bind((super::bind_address(&config.bind_ip)?, 0)).map_err(error)?;
    super::peer::configure_socket(&socket)?;
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
    let mut share_quality = super::quality(&settings)?;
    let mut loopback: Option<media::LoopbackSession> = None;
    let mut camera: Option<media::CameraSession> = None;
    let mut remote: BTreeMap<String, (Remote, u64)> = BTreeMap::new();
    let mut decoders: BTreeMap<String, Decode> = BTreeMap::new();
    let mut control = ControlState::default();
    let mut next_key = 1u64;
    let mut visible = Vec::new();
    let mut local_preview: Option<Preview> = None;
    let mut roster = (None, Vec::new());
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
                    Command::ReleaseInput => {
                        control.revoked = control.consent.grant.map(|g| g.token);
                        control.unit = None;
                        control.consent.clear();
                        hooks.set_control_active(false);
                    }
                    Command::ControlInput(data) => send_control(&mut peer, &data, output)?,
                    Command::ControlState(consent, own, name) => {
                        control.own = own;
                        control.consent = consent;
                        let permitted = own.zip(control.consent.grant).is_some_and(|(o, g)| {
                            control.revoked != Some(g.token)
                                && control.consent.may_inject(o, g.token)
                        });
                        if permitted && control.unit.is_none() {
                            if let Some(source) = control.source.clone() {
                                match super::control_banner::SafetyUnit::start(
                                    source,
                                    name,
                                    hooks.control_flag(),
                                ) {
                                    Ok(unit) => {
                                        let stop = unit.emergency();
                                        hooks.set_emergency(stop.clone());
                                        *emergency
                                            .lock()
                                            .map_err(|_| "Emergency state poisoned")? = Some(stop);
                                        control.unit = Some(unit)
                                    }
                                    Err(e) => {
                                        emit(output, Event::Signal(VoiceSignal::RevokeControl))?;
                                        return Err(e);
                                    }
                                }
                            } else {
                                emit(output, Event::Signal(VoiceSignal::RevokeControl))?;
                            }
                        } else if !permitted {
                            control.unit = None;
                            hooks.set_control_active(false);
                        }
                    }
                    Command::Visible(v) => {
                        visible = v;
                        local_preview = None;
                        if let Some(c) = &camera {
                            c.request_keyframe();
                        }
                        let (own, participants) = &roster;
                        peer.remote_tracks(*own, visible_participants(participants, &visible));
                        decoders.retain(|_, d| visible.contains(&(d.publisher, d.kind)));
                    }
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
                        roster = (own, participants.clone());
                        let participants: Vec<_> =
                            visible_participants(&participants, &visible).collect();
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
                                peer.set_send_bitrate(TrackKind::Screen, bps);
                                if let Some(s) = &share {
                                    let q = share_quality;
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
                        control.unit = None;
                        control.consent.clear();
                        hooks.set_control_active(false)
                    }
                    Command::Event(VoiceEvent::ControlGranted { .. }) => {}
                    Command::Event(_) => {}
                    Command::Share(source, pid, q) => {
                        control.unit = None;
                        control.consent.clear();
                        control.source = Some(source.clone());
                        share_quality = q;
                        let bps = q.recommended_bitrate(source.size).map_err(error)?;
                        peer.set_send_bitrate(TrackKind::Screen, u64::from(bps));
                        let surface = match source.kind {
                            media::SourceKind::Monitor => ShareSurface::Monitor,
                            media::SourceKind::Window => ShareSurface::Window,
                        };
                        let session =
                            media::ShareSession::start(source.clone(), q, bps).map_err(error)?;
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
                        control.unit = None;
                        control.source = None;
                        control.consent.clear();
                        share = None;
                        loopback = None;
                        emit(output, Event::Signal(VoiceSignal::RevokeControl))?;
                        peer.publish(TrackKind::Screen, false, ShareSurface::Monitor)?;
                        peer.publish(TrackKind::SystemAudio, false, ShareSurface::Monitor)?;
                        emit(output, Event::Sharing(false))?;
                    }
                    Command::Quality(q) => {
                        share_quality = q;
                        if let Some(s) = &share {
                            let source_size =
                                control.source.as_ref().map_or(s.size, |source| source.size);
                            let bps = q.recommended_bitrate(source_size).map_err(error)?;
                            peer.set_send_bitrate(TrackKind::Screen, u64::from(bps));
                            s.set_quality(q, bps).map_err(error)?;
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
                &visible,
                &mut control,
            )?;
        }
        if control
            .unit
            .as_ref()
            .is_some_and(|unit| !unit.active.load(std::sync::atomic::Ordering::Acquire))
        {
            control.revoked = control.consent.grant.map(|g| g.token);
            control.unit = None;
            control.consent.clear();
            emit(output, Event::Signal(VoiceSignal::RevokeControl))?;
        }
        while let Ok(event) = hooks.receiver.try_recv() {
            if event == hotkeys::Hotkey::Revoke {
                control.revoked = control.consent.grant.map(|g| g.token);
                control.unit = None;
                control.consent.clear();
            }

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
                        &visible,
                        &mut control,
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
                    Ok(frame) => {
                        if let Some(own) = roster
                            .0
                            .filter(|own| visible.contains(&(*own, TrackKind::Camera)))
                        {
                            if local_preview.is_none() && frame.keyframe {
                                local_preview = Some(Preview::new()?);
                            }
                            if let Some(preview) = &mut local_preview {
                                preview.present(frame.clone(), own, output)?;
                            }
                        }
                        send_frame(&mut peer, TrackKind::Camera, frame)?;
                    }
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
            &visible,
            &mut control,
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
            &visible,
            &mut control,
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
    visible: &[(VoiceSession, TrackKind)],
    control: &mut ControlState,
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
            PeerOutput::Control(bytes) => {
                let result = (|| -> Result<(), String> {
                    let own = control.own.ok_or("Not joined")?;
                    let data: ControlData = rmp_serde::from_slice(&bytes).map_err(error)?;
                    let unit = control
                        .unit
                        .as_mut()
                        .ok_or("No active control safety unit")?;
                    unit.receiver
                        .lock()
                        .map_err(|_| "Control state poisoned")?
                        .receive(&control.consent, own, data, crate::mock::now())
                })();
                if result.is_err() {
                    control.revoked = control.consent.grant.map(|g| g.token);
                    control.unit = None;
                    control.consent.clear();
                    emit(output, Event::Signal(VoiceSignal::RevokeControl))?;
                }
            }
            PeerOutput::Packet(source, p) => {
                if matches!(source.track.kind, TrackKind::Camera | TrackKind::Screen)
                    && !visible.contains(&(source.publisher, source.track.kind))
                {
                    continue;
                }
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
                                display: None,
                                dropped: 0,
                                publisher: source.publisher,
                                kind: source.track.kind,
                                rate_at: Instant::now(),
                                rate_bytes: 0,
                                rx_bps: 0,
                            },
                        );
                    }
                    let d = decoders.get_mut(&key).unwrap();
                    d.rate_bytes += p.payload.len() as u64;
                    let elapsed = d.rate_at.elapsed().as_secs_f64();
                    if elapsed >= 1. {
                        d.rx_bps = (d.rate_bytes as f64 * 8. / elapsed) as u64;
                        d.rate_bytes = 0;
                        d.rate_at = Instant::now();
                    }
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
                        let started = Instant::now();
                        let decoded = match d.decoder.decode(frame) {
                            Ok(frames) => frames,
                            Err(_) => {
                                peer.request_keyframe(&source);
                                continue;
                            }
                        };
                        d.frames += decoded.len() as u64;
                        let decode_ms = started.elapsed().as_secs_f32() * 1000.;
                        if d.display.is_none() {
                            d.display =
                                Some(media::DisplayBridge::new(d.decoder.device()).map_err(error)?);
                        }
                        for frame in &decoded {
                            match d.display.as_mut().unwrap().export(frame).map_err(error)? {
                                Some(frame) => {
                                    // Display is best-effort/latest-only. Never close a call because
                                    // the UI cannot keep up with video. Export caps live leases at 3.
                                    if output
                                        .try_send(Event::Video {
                                            session: source.publisher,
                                            kind: source.track.kind,
                                            frame,
                                            decode_ms,
                                            dropped: d.dropped,
                                            bps: d.rx_bps,
                                        })
                                        .is_err()
                                    {
                                        d.dropped += 1;
                                    }
                                }
                                None => d.dropped += 1,
                            }
                        }
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
                    }
                }
            }
        }
    }
    Ok(())
}

/// A failed/reliable-channel-full key-up must not leave the sharer holding it.
/// WS revocation is independent of SCTP and releases the owner's entire grant.
fn send_control(peer: &mut Peer, data: &ControlData, output: &Sender<Event>) -> Result<(), String> {
    let bytes = rmp_serde::to_vec_named(data).map_err(error)?;
    if let Err(error) = peer.control(&bytes) {
        emit(output, Event::Signal(VoiceSignal::RevokeControl))?;
        return Err(error);
    }
    Ok(())
}
fn visible_participants<'a>(
    participants: &'a [VoiceParticipant],
    visible: &'a [(VoiceSession, TrackKind)],
) -> impl Iterator<Item = VoiceParticipant> + 'a {
    participants.iter().cloned().map(|mut p| {
        p.tracks.retain(|t| match t.kind {
            TrackKind::Camera | TrackKind::Screen => visible.contains(&(p.session, t.kind)),
            TrackKind::SystemAudio => visible.contains(&(p.session, TrackKind::Screen)),
            TrackKind::Audio => true,
        });
        p
    })
}

#[derive(Default)]
struct ControlState {
    own: Option<VoiceSession>,
    consent: sticktalk_client_core::voice::control::Consent,
    source: Option<media::Source>,
    revoked: Option<u64>,
    unit: Option<super::control_banner::SafetyUnit>,
}

/// Local camera feedback reuses the exact hardware decode/shared-texture path.
/// It is created only for a visible self tile and dropped on visibility changes.
struct Preview {
    decoder: media::Decoder,
    display: media::DisplayBridge,
}
impl Preview {
    fn new() -> Result<Self, String> {
        let decoder = media::Decoder::new().map_err(error)?;
        let display = media::DisplayBridge::new(decoder.device()).map_err(error)?;
        Ok(Self { decoder, display })
    }
    fn present(
        &mut self,
        frame: media::EncodedFrame,
        own: VoiceSession,
        output: &Sender<Event>,
    ) -> Result<(), String> {
        let start = Instant::now();
        if let Ok(frames) = self.decoder.decode(frame) {
            let decode_ms = start.elapsed().as_secs_f32() * 1000.;
            for frame in frames {
                if let Some(frame) = self.display.export(&frame).map_err(error)? {
                    let _ = output.try_send(Event::Video {
                        session: own,
                        kind: TrackKind::Camera,
                        frame,
                        decode_ms,
                        dropped: 0,
                        bps: 0,
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod visibility_tests {
    use super::*;
    #[test]
    fn unavailable_control_channel_revokes_over_independent_signaling() {
        let mut peer = Peer::new("127.0.0.1:23457".parse().unwrap(), 1200, Instant::now()).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        assert!(send_control(&mut peer, &ControlData::ReleaseAll { grant: 1 }, &tx).is_err());
        assert!(matches!(
            rx.try_recv().unwrap(),
            Event::Signal(VoiceSignal::RevokeControl)
        ));
    }
    #[test]
    fn hidden_video_and_its_audio_are_removed_without_muting_voice() {
        let session = VoiceSession {
            user_id: Id(1),
            connection_id: Id(2),
        };
        let p = VoiceParticipant {
            session,
            muted: false,
            deafened: false,
            tracks: [
                TrackKind::Audio,
                TrackKind::Screen,
                TrackKind::SystemAudio,
                TrackKind::Camera,
            ]
            .into_iter()
            .enumerate()
            .map(|(i, kind)| PublishedTrack {
                mid: i.to_string(),
                kind,
                codec: None,
                layers: vec![],
                surface: None,
            })
            .collect(),
        };
        let participants = [p];
        let hidden = visible_participants(&participants, &[]).next().unwrap();
        assert_eq!(
            hidden.tracks.iter().map(|t| t.kind).collect::<Vec<_>>(),
            vec![TrackKind::Audio]
        );
        let visible = [(session, TrackKind::Screen)];
        let screen = visible_participants(&participants, &visible)
            .next()
            .unwrap();
        assert_eq!(
            screen.tracks.iter().map(|t| t.kind).collect::<Vec<_>>(),
            vec![TrackKind::Audio, TrackKind::Screen, TrackKind::SystemAudio]
        );
    }
}
