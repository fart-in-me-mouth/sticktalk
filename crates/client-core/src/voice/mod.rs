//! Ephemeral call reducer. No device activation, OS services, clocks or timers.
pub mod control;
pub mod settings;

pub use settings::VoiceSettings;
use std::collections::{BTreeMap, BTreeSet};
use sticktalk_proto::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Joining,
    Negotiating,
    Connected,
    Reconnecting,
}

#[derive(Clone, Debug, Default)]
pub struct Call {
    pub phase: Phase,
    pub desired_channel: Option<Id>,
    pub channel: Option<Id>,
    pub own: Option<VoiceSession>,
    pub participants: BTreeMap<VoiceSession, VoiceParticipant>,
    pub guild_states: BTreeMap<VoiceSession, GuildVoiceState>,
    pub speaking: BTreeSet<VoiceSession>,
    pub muted: bool,
    pub deafened: bool,
    /// Effective server flags. Never overwrite the user's independent choices.
    pub server_muted: bool,
    pub server_deafened: bool,
    pub stats: Option<VoiceStats>,
    pub settings: VoiceSettings,
    pub control: control::Consent,
    join_nonce: Option<Nonce>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Signal(VoiceSignal),
    StartTransport { max_rtp_bytes: u16 },
    StopTransport,
    Gate { muted: bool, deafened: bool },
    ReleaseInput,
}

pub fn encode(nonce: Nonce, signal: VoiceSignal) -> Result<VoicePayload, String> {
    let payload = rmp_serde::to_vec_named(&signal).map_err(|_| "Invalid voice signal")?;
    if payload.len() > 72 * 1024 {
        return Err("Voice signal exceeds limit".into());
    }
    Ok(VoicePayload {
        nonce,
        version: VOICE_VERSION,
        kind: "signal".into(),
        payload,
    })
}

pub fn decode(payload: &VoicePayload) -> Result<VoiceEvent, String> {
    if payload.version != VOICE_VERSION
        || payload.kind != "event"
        || payload.payload.len() > 72 * 1024
    {
        return Err("Unsupported voice envelope".into());
    }
    let mut decoder = rmp_serde::Deserializer::new(std::io::Cursor::new(&payload.payload));
    decoder.set_max_depth(16);
    let event = serde::Deserialize::deserialize(&mut decoder).map_err(|_| "Invalid voice event")?;
    if decoder.position() != payload.payload.len() as u64 {
        return Err("Trailing voice bytes".into());
    }
    Ok(event)
}

impl Call {
    pub fn join(&mut self, channel: Id, nonce: Nonce) -> Vec<Effect> {
        let mut effects = self.leave();
        self.desired_channel = Some(channel);
        self.phase = Phase::Joining;
        self.join_nonce = Some(nonce);
        effects.push(Effect::Signal(VoiceSignal::Join {
            channel_id: channel,
            capabilities: VoiceCapabilities {
                video: vec![VideoCodec::H264],
                hardware_decode: vec![VideoCodec::H264],
            },
        }));
        effects
    }
    pub fn leave(&mut self) -> Vec<Effect> {
        let joined = self.desired_channel.take().is_some();
        self.reset_media();
        self.phase = Phase::Idle;
        let mut effects = vec![Effect::ReleaseInput, Effect::StopTransport];
        if joined {
            effects.push(Effect::Signal(VoiceSignal::Leave));
        }
        effects
    }
    fn reset_media(&mut self) {
        self.channel = None;
        self.own = None;
        self.participants.clear();
        self.speaking.clear();
        self.stats = None;
        self.join_nonce = None;
        self.server_muted = false;
        self.server_deafened = false;
        self.control.clear();
    }
    pub fn disconnected(&mut self) -> Vec<Effect> {
        self.reset_media();
        self.guild_states.clear();
        self.phase = if self.desired_channel.is_some() {
            Phase::Reconnecting
        } else {
            Phase::Idle
        };
        vec![Effect::ReleaseInput, Effect::StopTransport]
    }
    /// Ready AND Resumed create a fresh ICE/DTLS lifetime; no old offer/grant replay.
    pub fn ready(&mut self, nonce: Nonce) -> Vec<Effect> {
        match self.desired_channel {
            Some(channel) if self.phase == Phase::Reconnecting => {
                // The old WS lifetime is already gone: do not send its Leave on
                // the new connection, where it could race the correlated Join.
                self.desired_channel = None;
                self.join(channel, nonce)
            }
            _ => vec![],
        }
    }
    pub fn gate(&self) -> Effect {
        Effect::Gate {
            muted: self.muted || self.server_muted || self.deafened || self.server_deafened,
            deafened: self.deafened || self.server_deafened,
        }
    }
    pub fn set_state(&mut self, muted: bool, deafened: bool) -> Vec<Effect> {
        self.muted = muted;
        self.deafened = deafened;
        let mut effects = vec![self.gate()];
        if self.channel.is_some() {
            effects.push(Effect::Signal(VoiceSignal::State { muted, deafened }));
        }
        effects
    }
    pub fn event(&mut self, nonce: Nonce, event: &VoiceEvent, me: Id, now_ms: u64) -> Vec<Effect> {
        let mut effects = vec![];
        match event {
            VoiceEvent::GuildSnapshot { states } => {
                self.guild_states = states
                    .iter()
                    .take(320)
                    .map(|s| (s.participant.session, s.clone()))
                    .collect();
            }
            VoiceEvent::GuildState(s) => {
                if self.guild_states.len() < 320
                    || self.guild_states.contains_key(&s.participant.session)
                {
                    self.guild_states.insert(s.participant.session, s.clone());
                }
            }
            VoiceEvent::GuildLeft { session, .. } => {
                self.guild_states.remove(session);
            }
            VoiceEvent::Joined {
                channel_id,
                participants,
                max_rtp_bytes,
            } if self.desired_channel == Some(*channel_id) && self.join_nonce == Some(nonce) => {
                self.channel = Some(*channel_id);
                self.phase = Phase::Negotiating;
                self.participants = participants
                    .iter()
                    .take(32)
                    .map(|p| (p.session, p.clone()))
                    .collect();
                self.own = participants
                    .iter()
                    .find(|p| p.session.user_id == me)
                    .map(|p| p.session);
                effects.push(Effect::StartTransport {
                    max_rtp_bytes: *max_rtp_bytes,
                });
                effects.extend(self.set_state(self.muted, self.deafened));
                effects.push(Effect::Signal(VoiceSignal::Activity {
                    voice_activity: !self.settings.push_to_talk,
                    priority: false,
                }));
            }
            VoiceEvent::Participant(p) if self.channel.is_some() => {
                if self.participants.len() < 32 || self.participants.contains_key(&p.session) {
                    self.participants.insert(p.session, p.clone());
                }
                if p.session.user_id == me {
                    self.own = Some(p.session);
                    self.server_muted = p.muted;
                    self.server_deafened = p.deafened;
                    effects.push(self.gate());
                }
            }
            VoiceEvent::Left { session } => {
                self.participants.remove(session);
                self.speaking.remove(session);
                if self.own == Some(*session) {
                    // A server kick must not automatically rejoin and fight moderation.
                    effects.extend(self.leave());
                }
            }
            VoiceEvent::Speaking {
                session, speaking, ..
            } if self.participants.contains_key(session) => {
                if *speaking {
                    self.speaking.insert(*session);
                } else {
                    self.speaking.remove(session);
                }
            }
            VoiceEvent::Stats(stats) if self.channel.is_some() => self.stats = Some(stats.clone()),
            VoiceEvent::ControlRequested {
                requester,
                timeout_ms,
            } if self.own.is_some() => {
                self.control
                    .request(*requester, now_ms, (*timeout_ms).min(30_000));
            }
            VoiceEvent::ControlGranted {
                sharer,
                controller,
                grant,
            } if self.channel.is_some() => {
                if self.participants.contains_key(sharer)
                    && self.participants.contains_key(controller)
                {
                    self.control.granted(*sharer, *controller, *grant);
                }
            }
            VoiceEvent::ControlRevoked { grant, .. }
                if self.control.grant.is_some_and(|g| g.token == *grant) =>
            {
                self.control.clear();
                effects.push(Effect::ReleaseInput);
            }
            VoiceEvent::Error { .. }
                if self.phase == Phase::Joining && self.join_nonce == Some(nonce) =>
            {
                effects.extend(self.leave());
            }
            _ => {}
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn participant() -> VoiceParticipant {
        VoiceParticipant {
            session: VoiceSession {
                user_id: Id(1),
                connection_id: Id(2),
            },
            muted: false,
            deafened: false,
            tracks: vec![],
        }
    }
    fn joined(c: &mut Call, nonce: u64) {
        c.event(
            nonce,
            &VoiceEvent::Joined {
                channel_id: Id(10),
                participants: vec![participant()],
                max_rtp_bytes: 1232,
            },
            Id(1),
            0,
        );
    }
    #[test]
    fn reconnect_rejoins_but_kick_does_not() {
        let mut c = Call::default();
        c.join(Id(10), 1);
        joined(&mut c, 1);
        c.disconnected();
        assert_eq!(c.phase, Phase::Reconnecting);
        let effects = c.ready(2);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::Signal(VoiceSignal::Join { .. }))));
        joined(&mut c, 1);
        assert!(c.channel.is_none()); // late old join
        joined(&mut c, 2);
        assert_eq!(c.channel, Some(Id(10)));
        c.event(
            0,
            &VoiceEvent::Left {
                session: participant().session,
            },
            Id(1),
            0,
        );
        assert!(c.desired_channel.is_none());
        assert!(c.ready(3).is_empty());
    }
    #[test]
    fn server_flags_cannot_be_locally_bypassed() {
        let mut c = Call::default();
        c.join(Id(10), 1);
        joined(&mut c, 1);
        let mut p = participant();
        p.muted = true;
        c.event(0, &VoiceEvent::Participant(p), Id(1), 0);
        c.set_state(false, false);
        assert_eq!(
            c.gate(),
            Effect::Gate {
                muted: true,
                deafened: false
            }
        );
        c.set_state(true, true);
        c.set_state(true, false);
        assert!(c.muted);
    }
    #[test]
    fn envelope_is_bounded_and_rejects_trailing_data() {
        let mut p = VoicePayload {
            nonce: 0,
            version: VOICE_VERSION,
            kind: "event".into(),
            payload: rmp_serde::to_vec_named(&VoiceEvent::Ack).unwrap(),
        };
        assert!(decode(&p).is_ok());
        p.payload.push(0);
        assert!(decode(&p).is_err());
    }
}
