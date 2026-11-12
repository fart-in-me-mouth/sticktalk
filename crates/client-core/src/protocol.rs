//! Single-socket request correlation, reconnect/resume and heartbeat driver.
//! Hosts provide transport, clock, entropy, cancellable tasks, REST and bounded queues.
use crate::{
    backend::{decode, Config, Data, Request},
    io::{nonce, Clock, Entropy, MaybeSend, Spawner, Task},
    model::{ConnectionState, Recovery},
};
use futures_util::{stream::FuturesUnordered, FutureExt, Sink, SinkExt, Stream, StreamExt};
use std::{
    collections::{BTreeMap, VecDeque},
    pin::Pin,
};
use sticktalk_proto::*;
#[derive(Debug)]
pub enum Frame {
    Binary(Vec<u8>),
    Close(Option<u16>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Text,
    Other,
}
#[cfg(not(target_arch = "wasm32"))]
pub type Incoming = Pin<Box<dyn Stream<Item = Result<Frame, String>> + Send>>;
#[cfg(target_arch = "wasm32")]
pub type Incoming = Pin<Box<dyn Stream<Item = Result<Frame, String>>>>;
#[cfg(not(target_arch = "wasm32"))]
pub type Outgoing = Pin<Box<dyn Sink<Frame, Error = String> + Send>>;
#[cfg(target_arch = "wasm32")]
pub type Outgoing = Pin<Box<dyn Sink<Frame, Error = String>>>;
#[derive(Debug)]
pub enum ConnectError {
    Retry(String),
    Stop(String),
}
impl From<String> for ConnectError {
    fn from(value: String) -> Self {
        Self::Retry(value)
    }
}
pub trait Transport {
    /// Must negotiate sticktalk.v1 and bound frames to MAX_WS_FRAME_BYTES.
    fn connect(&self, url: String) -> Task<'_, Result<(Outgoing, Incoming), ConnectError>>;
}
pub trait EventSink<H> {
    /// False means the consumer has gone away. Hosts enforce queue byte budgets.
    fn emit(&mut self, data: Data<H>) -> Task<'_, bool>;
}
pub trait RestService<R, H>: Clone {
    fn request(&self, request: Request<R>) -> Task<'static, Result<Data<H>, String>>;
}
async fn send(socket: &mut Outgoing, msg: &ClientMsg) -> Result<(), ()> {
    let bytes = rmp_serde::to_vec_named(msg).map_err(|_| ())?;
    if bytes.len() > MAX_WS_FRAME_BYTES as usize {
        return Err(());
    }
    socket.send(Frame::Binary(bytes)).await.map_err(|_| ())
}
#[allow(clippy::too_many_arguments)]
pub async fn run<R: MaybeSend + 'static, H: MaybeSend + 'static, Lease>(
    config: &Config,
    transport: &impl Transport,
    clock: &(impl Clock + Entropy),
    spawner: &impl Spawner,
    http: impl RestService<R, H>,
    mut rx: impl Stream<Item = (Request<R>, Lease)> + Unpin,
    output: &mut impl EventSink<H>,
) {
    let mut recovery = Recovery::default();
    let mut pending: BTreeMap<Nonce, (CommandRequest, u64)> = BTreeMap::new();
    loop {
        if !output.emit(Data::State(ConnectionState::Connecting)).await {
            return;
        }
        let url = format!(
            "{}/api/v1/ws",
            config
                .server
                .replacen("https://", "wss://", 1)
                .replacen("http://", "ws://", 1)
        );
        let ws = {
            let connection = transport.connect(url).fuse();
            let timeout = clock.sleep(15_000).fuse();
            futures_util::pin_mut!(connection, timeout);
            futures_util::select! { value = connection => Some(value), _ = timeout => None }
        };
        let ws = match ws {
            Some(Ok(socket)) => Some(socket),
            Some(Err(ConnectError::Stop(reason))) => {
                output.emit(Data::Error(reason)).await;
                return;
            }
            _ => None,
        };
        let mut close = None;
        if let Some((mut socket, mut incoming)) = ws {
            let connected_at = clock.monotonic_ms();
            let mut identified = false;
            let mut ready = false;
            let mut beat = 30_000u64;
            let mut timeout = 75_000u64;
            let mut deadline = clock.monotonic_ms() + 10_000;
            let mut ack_at = clock.monotonic_ms();
            let mut outstanding = None;
            let mut jobs = FuturesUnordered::new();
            let mut rest_queue = VecDeque::new();
            loop {
                enum Wake<R, H, L> {
                    Frame(Option<Result<Frame, String>>),
                    Request(Option<(Request<R>, L)>),
                    Job(Option<Result<Result<Data<H>, String>, String>>),
                    Timer,
                }
                // Poll all active work on the host executor; no idle ticker or polling.
                let wake = {
                    let frame = incoming.next().fuse();
                    let request = rx.next().fuse();
                    let job = async {
                        if jobs.is_empty() {
                            std::future::pending().await
                        } else {
                            jobs.next().await
                        }
                    }
                    .fuse();
                    let timer = clock
                        .sleep(deadline.saturating_sub(clock.monotonic_ms()))
                        .fuse();
                    futures_util::pin_mut!(frame, request, job, timer);
                    futures_util::select! {
                        value = frame => Wake::Frame(value), value = request => Wake::Request(value),
                        value = job => Wake::Job(value), _ = timer => Wake::Timer,
                    }
                };
                match wake {
                    Wake::Frame(frame) => {
                        let Some(Ok(frame)) = frame else { break };
                        match frame {
                            Frame::Binary(bytes) => {
                                let frame = match decode(&bytes) {
                                    Ok(v) => v,
                                    Err(v) => {
                                        output.emit(Data::Error(v)).await;
                                        close = Some(1002);
                                        break;
                                    }
                                };
                                match &frame {
                                    ServerMsg::Hello(v) => {
                                        if identified || v.version != PROTOCOL_VERSION {
                                            close = Some(4002);
                                            break;
                                        }
                                        beat = u64::from(v.heartbeat_interval_ms.max(1000));
                                        timeout = u64::from(v.heartbeat_timeout_ms.max(1000));
                                        if send(
                                            &mut socket,
                                            &ClientMsg::Identify(Identify {
                                                version: PROTOCOL_VERSION,
                                                token: config.token.clone(),
                                                resume: recovery.cursor.clone(),
                                                ..Default::default()
                                            }),
                                        )
                                        .await
                                        .is_err()
                                        {
                                            break;
                                        }
                                        identified = true;
                                    }
                                    ServerMsg::Ready(_) | ServerMsg::Resumed(_) => {
                                        ready = true;
                                        recovery.state = ConnectionState::Ready;
                                        deadline = clock.monotonic_ms() + beat;
                                        ack_at = clock.monotonic_ms();
                                        outstanding = None;
                                        for (v, started) in pending.values() {
                                            if clock.monotonic_ms().saturating_sub(*started)
                                                < 590_000
                                                && send(&mut socket, &ClientMsg::Command(v.clone()))
                                                    .await
                                                    .is_err()
                                            {
                                                break;
                                            }
                                        }
                                        let expired: Vec<_> = pending
                                            .iter()
                                            .filter(|(_, (_, started))| {
                                                clock.monotonic_ms().saturating_sub(*started)
                                                    >= 590_000
                                            })
                                            .map(|(n, _)| *n)
                                            .collect();
                                        pending.retain(|_, (_, started)| {
                                            clock.monotonic_ms().saturating_sub(*started) < 590_000
                                        });
                                        if !expired.is_empty()
                                            && !output.emit(Data::Uncertain(expired)).await
                                        {
                                            return;
                                        }
                                    }
                                    ServerMsg::ResyncRequired(_) => {
                                        ready = false;
                                        recovery.resync();
                                    }
                                    ServerMsg::HeartbeatAck(v) => {
                                        if outstanding == Some(v.nonce) {
                                            outstanding = None;
                                            ack_at = clock.monotonic_ms();
                                        }
                                    }
                                    ServerMsg::Response(v) => {
                                        pending.remove(&v.nonce);
                                    }
                                    ServerMsg::Error(v)
                                        if matches!(
                                            v.code,
                                            ErrorCode::Unauthenticated | ErrorCode::SessionExpired
                                        ) =>
                                    {
                                        close = Some(4001);
                                    }
                                    _ => {}
                                }
                                if !output.emit(Data::Frame(frame)).await {
                                    return;
                                }
                                if close == Some(4001) {
                                    break;
                                }
                            }
                            Frame::Close(v) => {
                                close = v;
                                break;
                            }
                            Frame::Ping(v) => {
                                if socket.send(Frame::Pong(v)).await.is_err() {
                                    break;
                                }
                            }
                            Frame::Text => {
                                close = Some(1003);
                                break;
                            }
                            _ => {}
                        }
                    }
                    Wake::Request(request) => {
                        let Some((request, _lease)) = request else {
                            return;
                        };
                        match request {
                            Request::Applied(v) => {
                                if recovery.cursor.is_none() {
                                    recovery.fresh(v);
                                } else {
                                    recovery.applied(v);
                                }
                            }
                            Request::View => {
                                jobs.clear();
                                rest_queue.clear();
                            }
                            Request::Command(v) => {
                                let reason = if !ready {
                                    Some("You are offline. This action was not queued.")
                                } else if pending.len() >= 32 {
                                    Some("32 actions are in flight. Please wait.")
                                } else {
                                    None
                                };
                                if let Some(reason) = reason {
                                    if !output
                                        .emit(Data::Rejected {
                                            nonce: v.nonce,
                                            reason: reason.into(),
                                        })
                                        .await
                                    {
                                        return;
                                    }
                                    continue;
                                }
                                if !matches!(
                                    v.command,
                                    Command::Typing { .. } | Command::SubscribeGuild { .. }
                                ) {
                                    pending.insert(v.nonce, (v.clone(), clock.monotonic_ms()));
                                }
                                if send(&mut socket, &ClientMsg::Command(v)).await.is_err() {
                                    break;
                                }
                            }
                            v => {
                                if jobs.len() >= 2 {
                                    if rest_queue.len() < 16 {
                                        rest_queue.push_back(v);
                                    } else if !output
                                        .emit(Data::Error(
                                            "Download queue is full. Try again shortly.".into(),
                                        ))
                                        .await
                                    {
                                        return;
                                    }
                                } else {
                                    jobs.push(spawner.spawn(http.request(v)));
                                }
                            }
                        }
                    }
                    Wake::Job(result) => {
                        if let Some(Ok(result)) = result {
                            if !output.emit(result.unwrap_or_else(Data::Error)).await {
                                return;
                            }
                        }
                        if let Some(v) = rest_queue.pop_front() {
                            jobs.push(spawner.spawn(http.request(v)));
                        }
                    }
                    Wake::Timer => {
                        if !ready || clock.monotonic_ms().saturating_sub(ack_at) >= timeout {
                            break;
                        }
                        if outstanding.is_none() {
                            let nonce = nonce(clock);
                            outstanding = Some(nonce);
                            if send(
                                &mut socket,
                                &ClientMsg::Heartbeat(Heartbeat {
                                    nonce,
                                    last_seq: recovery.cursor.as_ref().map_or(0, |v| v.seq),
                                }),
                            )
                            .await
                            .is_err()
                            {
                                break;
                            }
                        }
                        if clock.monotonic_ms().saturating_sub(connected_at) >= 60_000 {
                            recovery.healthy();
                        }
                        deadline = clock.monotonic_ms()
                            + beat.min(
                                timeout.saturating_sub(clock.monotonic_ms().saturating_sub(ack_at)),
                            );
                    }
                }
            }
            // Dropping task handles cancels REST work, matching native JoinSet teardown.
            drop(jobs);
        }
        let delay = recovery.disconnected(close);
        if !output.emit(Data::State(recovery.state.clone())).await {
            return;
        }
        let Some(delay) = delay else {
            return;
        };
        let wait = clock.sleep(delay * 1000 + nonce(clock) % 251).fuse();
        futures_util::pin_mut!(wait);
        loop {
            let request = rx.next().fuse();
            futures_util::pin_mut!(request);
            futures_util::select! {
                _=wait => break,
                value=request => match value {
                    Some((Request::Applied(v),_lease)) => recovery.applied(v),
                    Some((Request::Command(v),_lease)) => { if !output.emit(Data::Rejected {nonce:v.nonce,reason:"Offline. This action was not queued.".into()}).await {return;} }
                    None => return,
                    _ => { if !output.emit(Data::Error("Offline. Action not queued.".into())).await {return;} }
                }
            }
        }
    }
}
