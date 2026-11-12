use crate::{Error, Out, Sfu};
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
    Shutdown,
}
#[derive(Debug)]
pub struct SignalOut {
    pub session: VoiceSession,
    pub envelope: VoicePayload,
}

impl Sfu {
    /// Drive the one UDP socket on the host's existing Tokio runtime. No extra
    /// runtime, task per peer, fixed polling loop, capture, browser or OS UI.
    /// Closing commands or sending Shutdown revokes grants and stops the SFU.
    pub async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        signals: mpsc::Sender<SignalOut>,
    ) -> Result<(), Error> {
        let socket = tokio::net::UdpSocket::from_std(self.socket.take().ok_or(Error::Config)?)?;
        let mut buf = vec![0u8; 65536];
        loop {
            let output: Vec<_> = self.drain().collect();
            for out in output {
                match out {
                    Out::Datagram { to, bytes } => {
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
                            // The host must not retain a live ACL/Ready entry
                            // after losing its only cleanup notification.
                            return Err(Error::Backpressure);
                        }
                    }
                }
            }
            if self.has_output() {
                continue;
            }
            let deadline = self.next_timeout();
            tokio::select! {
                command = commands.recv() => {
                    match command {
                        Some(Command::Signal { session, envelope }) => {
                            // Failures are correlated voice errors, not actor crashes.
                            // Version/decode errors occur before handle_signal.
                            let nonce = envelope.nonce;
                            if let Err(error) = self.handle_envelope(session, envelope, Instant::now()) {
                                if matches!(error, Error::Backpressure) { return Err(error); }
                                tracing::debug!(?session, nonce, %error, "SFU signal rejected");
                            }
                        }
                        Some(Command::Disconnect(session)) => self.disconnect(session)?,
                        Some(Command::Path { session, path }) => { let _ = self.set_path(session, path); }
                        Some(Command::Shutdown) | None => break,
                    }
                }
                received = socket.recv_from(&mut buf) => {
                    match received {
                        Ok((len, source)) => self.handle_datagram(source, &buf[..len], Instant::now())?,
                        // Windows can report an ICMP port-unreachable from a
                        // departed peer as a receive-side connection reset.
                        // That must not tear down every other participant.
                        Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock) => {},
                        Err(error) => return Err(error.into()),
                    }
                }
                _ = async {
                    match deadline {
                        Some(t) => tokio::time::sleep_until(t.into()).await,
                        None => std::future::pending::<()>().await,
                    }
                } => self.handle_timeout(Instant::now())?,
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
