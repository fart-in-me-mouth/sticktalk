use crate::{control::AgentStep, Error, Out, Sfu};
use std::time::Instant;
use sticktalk_proto::*;
use tokio::sync::mpsc;

/// Send through a BOUNDED host mpsc (recommended capacity 128). The trusted
/// host must bind session identities and validate its WS frame size first.
#[derive(Debug)]
pub enum Command {
    Signal {
        session: VoiceSession,
        envelope: VoicePayload,
    },
    Disconnect(VoiceSession),
    Path {
        session: VoiceSession,
        path: MediaPath,
    },
    Refresh(VoiceSession),
    Move {
        session: VoiceSession,
        channel: ChannelId,
    },
    /// Remote control for `session`'s AI agent. The host must have checked that
    /// the agent may act for that session. The reply says whether it was taken.
    Agent {
        session: VoiceSession,
        action: AgentAction,
        reply: tokio::sync::oneshot::Sender<Result<(), Error>>,
    },
    Shutdown,
}
#[derive(Debug)]
pub enum AgentAction {
    /// Ask the sharer; the answer arrives as the usual control events.
    Request {
        sharer: VoiceSession,
    },
    /// See `Sfu::agent_script`.
    Script {
        steps: Vec<(std::time::Duration, AgentStep)>,
        keep_held: std::time::Duration,
    },
    Revoke,
}
#[derive(Debug)]
pub struct SignalOut {
    pub session: VoiceSession,
    pub envelope: VoicePayload,
}

/// Mandatory cleanup never competes with routine state/ACK output.
#[derive(Debug)]
pub enum ControlOut {
    Revoked(VoiceSession),
    Signal(SignalOut),
}

impl Sfu {
    async fn overloaded(
        &mut self,
        session: VoiceSession,
        socket: &tokio::net::UdpSocket,
        control: &Option<mpsc::Sender<ControlOut>>,
    ) {
        for out in self.overload_cleanup(session) {
            match out {
                Out::Signal { to, nonce, event } => {
                    if let (Some(control), Ok(envelope)) = (control, encode_event(nonce, &event)) {
                        let _ = control
                            .send(ControlOut::Signal(SignalOut {
                                session: to,
                                envelope,
                            }))
                            .await;
                    }
                }
                Out::Datagram { to, bytes } => {
                    let _ = socket.send_to(&bytes, to).await;
                }
            }
        }
        if let Some(control) = control {
            let _ = control.send(ControlOut::Revoked(session)).await;
        }
    }
    /// Drive the one UDP socket on the host's existing Tokio runtime. No extra
    /// runtime, task per peer, fixed polling loop, capture, browser or OS UI.
    /// Closing commands or sending Shutdown revokes grants and stops the SFU.
    pub async fn run(
        self,
        commands: mpsc::Receiver<Command>,
        signals: mpsc::Sender<SignalOut>,
    ) -> Result<(), Error> {
        let (_tx, rx) = mpsc::channel(128);
        self.run_driver(commands, signals, rx, None).await
    }
    /// Separate bounded cleanup lane: signaling congestion cannot lose revocation.
    pub async fn run_with_control(
        self,
        commands: mpsc::Receiver<Command>,
        signals: mpsc::Sender<SignalOut>,
        disconnects: mpsc::Receiver<VoiceSession>,
        revoked: mpsc::Sender<ControlOut>,
    ) -> Result<(), Error> {
        self.run_driver(commands, signals, disconnects, Some(revoked))
            .await
    }
    async fn run_driver(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        signals: mpsc::Sender<SignalOut>,
        mut disconnects: mpsc::Receiver<VoiceSession>,
        revoked: Option<mpsc::Sender<ControlOut>>,
    ) -> Result<(), Error> {
        let socket = tokio::net::UdpSocket::from_std(self.socket.take().ok_or(Error::Config)?)?;
        let mut buf = vec![0u8; 65536];
        let mut clock = None;
        loop {
            for session in self.take_overloaded() {
                self.overloaded(session, &socket, &revoked).await;
            }
            let generation = self.policy_generation();
            let output: Vec<_> = self.drain().collect();
            for out in output {
                match out {
                    Out::Datagram { to, bytes } => {
                        if generation != self.policy_generation() {
                            continue;
                        }
                        // A single unreachable peer must not stop everyone.
                        if let Err(error) = socket.send_to(&bytes, to).await {
                            tracing::debug!(%error, "SFU UDP send failed");
                        }
                    }
                    Out::Signal { to, nonce, event } => {
                        let envelope = encode_event(nonce, &event)?;
                        if signals
                            .try_send(SignalOut {
                                session: to,
                                envelope,
                            })
                            .is_err()
                        {
                            // Only the destination voice session is affected. Drain
                            // output before cleanup, which may itself emit signals.
                            self.overloaded(to, &socket, &revoked).await;
                        }
                    }
                }
            }
            if self.has_output() {
                continue;
            }
            let deadline = self.next_timeout();
            if deadline.is_none() {
                clock = None;
            } else if clock.is_none() {
                clock = Some(crate::pacing::Clock::new()?);
            }
            tokio::select! {
                biased;
                Some(session) = disconnects.recv() => { self.overloaded(session,&socket,&revoked).await; }
                command = commands.recv() => {
                    match command {
                        Some(Command::Signal { session, envelope }) => {
                            // Failures are correlated voice errors, not actor crashes.
                            // Version/decode errors occur before handle_signal.
                            let nonce = envelope.nonce;
                            if let Err(error) = self.handle_envelope(session, envelope, Instant::now()) {
                                if matches!(error, Error::Backpressure) {
                                    // Internal output congestion: discard queued
                                    // signals for this peer, then reserve cleanup.
                                    self.overloaded(session,&socket,&revoked).await;
                                }
                                tracing::debug!(?session, nonce, %error, "SFU signal rejected");
                            }
                        }
                        Some(Command::Refresh(session)) => {if self.refresh_policy(session).is_err(){self.overloaded(session,&socket,&revoked).await;}},
                        Some(Command::Move {session,channel}) => { if self.move_session(session,channel).is_err() { self.overloaded(session,&socket,&revoked).await; } },
                        Some(Command::Disconnect(session)) => self.overloaded(session,&socket,&revoked).await,
                        Some(Command::Path { session, path }) => { let _ = self.set_path(session, path); }
                        Some(Command::Agent { session, action, reply }) => {
                            let now = Instant::now();
                            let result = match action {
                                AgentAction::Request { sharer } => self.agent_request(session, sharer, now),
                                AgentAction::Script { steps, keep_held } => self.agent_script(session, steps, keep_held, now),
                                AgentAction::Revoke => self.agent_revoke(session, now),
                            };
                            if matches!(result, Err(Error::Backpressure)) {
                                self.overloaded(session,&socket,&revoked).await;
                            }
                            let _ = reply.send(result);
                        }
                        Some(Command::Shutdown) | None => break,
                    }
                }
                received = socket.recv_from(&mut buf) => {
                    match received {
                        Ok((len, source)) => {
                            if let Err(error)=self.handle_datagram(source,&buf[..len],Instant::now()) {
                                if !matches!(error,Error::Backpressure){return Err(error);}
                            }
                        },
                        // Windows can report an ICMP port-unreachable from a
                        // departed peer as a receive-side connection reset.
                        // That must not tear down every other participant.
                        Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock) => {},
                        Err(error) => return Err(error.into()),
                    }
                }
                timer = async {
                    match deadline {
                        Some(t) => clock.as_mut().unwrap().wait(t).await?,
                        None => std::future::pending::<()>().await,
                    }
                    Ok::<(), std::io::Error>(())
                } => {
                    timer?;
                    if let Err(error) = self.handle_timeout(Instant::now()) {
                        if !matches!(error, Error::Backpressure) { return Err(error); }
                    }
                },
            }
        }
        // Notify host and sharers before dropping all cryptographic sessions.
        // The host must still locally release on its transport disconnect.
        let keys = self.sessions();
        for key in keys {
            self.disconnect(key)?;
        }
        for out in self.drain() {
            match out {
                Out::Datagram { to, bytes } => {
                    let _ = socket.send_to(&bytes, to).await;
                }
                Out::Signal { to, nonce, event } => {
                    let _ = signals.try_send(SignalOut {
                        session: to,
                        envelope: encode_event(nonce, &event)?,
                    });
                }
            }
        }
        Ok(())
    }
}

pub fn encode_signal(nonce: Nonce, signal: &VoiceSignal) -> Result<VoicePayload, Error> {
    Ok(VoicePayload {
        nonce,
        version: VOICE_VERSION,
        kind: "signal".into(),
        payload: rmp_serde::to_vec_named(signal).map_err(|_| Error::Invalid)?,
    })
}
pub fn encode_event(nonce: Nonce, event: &VoiceEvent) -> Result<VoicePayload, Error> {
    Ok(VoicePayload {
        nonce,
        version: VOICE_VERSION,
        kind: "event".into(),
        payload: rmp_serde::to_vec_named(event).map_err(|_| Error::Invalid)?,
    })
}
