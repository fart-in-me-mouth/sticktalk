//! Deterministic tests: no runtime, clock waits, network, filesystem or OS windows.
use crate::{
    backend::{Config, Data, Request},
    http::{self, Body, HttpRequest, HttpResponse, HttpTransport},
    io::{Clock, Entropy, MaybeSend, Spawner, Task},
    protocol::{self, ConnectError, EventSink, Frame, Incoming, Outgoing, RestService, Transport},
    upload::UploadSource,
};
use futures_channel::{mpsc, oneshot};
use futures_util::{task::noop_waker_ref, StreamExt};
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
};
use sticktalk_proto::*;
fn poll<T>(future: Pin<&mut impl Future<Output = T>>) -> Poll<T> {
    future.poll(&mut Context::from_waker(noop_waker_ref()))
}
fn complete<T>(future: impl Future<Output = T>) -> T {
    let mut future = Box::pin(future);
    match poll(future.as_mut()) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected external wait"),
    }
}
#[derive(Default)]
struct ManualClock {
    now: AtomicU64,
    random: AtomicU64,
    sleepers: Mutex<Vec<(u64, oneshot::Sender<()>)>>,
}
impl ManualClock {
    fn advance(&self, ms: u64) {
        let now = self.now.fetch_add(ms, Ordering::Relaxed) + ms;
        let mut sleepers = self.sleepers.lock().unwrap();
        let mut i = 0;
        while i < sleepers.len() {
            if sleepers[i].0 <= now {
                let (_, tx) = sleepers.swap_remove(i);
                let _ = tx.send(());
            } else {
                i += 1;
            }
        }
    }
}
impl Clock for ManualClock {
    fn monotonic_ms(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }
    fn unix_ms(&self) -> u64 {
        1_799_999_999_000 + self.monotonic_ms()
    }
    fn sleep(&self, ms: u64) -> Task<'static, ()> {
        let (tx, rx) = oneshot::channel();
        self.sleepers
            .lock()
            .unwrap()
            .push((self.monotonic_ms() + ms, tx));
        Box::pin(async move {
            let _ = rx.await;
        })
    }
}
impl Entropy for ManualClock {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), String> {
        let seed = self.random.fetch_add(1, Ordering::Relaxed) as u8;
        bytes.fill(seed);
        Ok(())
    }
}
#[derive(Clone)]
struct Inline;
impl Spawner for Inline {
    fn spawn<T: MaybeSend + 'static>(
        &self,
        future: Task<'static, T>,
    ) -> Task<'static, Result<T, String>> {
        Box::pin(async move { Ok(future.await) })
    }
}
impl RestService<String, ()> for Inline {
    fn request(&self, _: Request) -> Task<'static, Result<Data, String>> {
        Box::pin(async { Err("unused REST operation".into()) })
    }
}
struct Wire(Mutex<VecDeque<(Outgoing, Incoming)>>);
impl Transport for Wire {
    fn connect(&self, url: String) -> Task<'_, Result<(Outgoing, Incoming), ConnectError>> {
        assert_eq!(url, "wss://example.com/api/v1/ws");
        let pair = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected reconnect");
        Box::pin(async move { Ok(pair) })
    }
}
#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<Data>>>);
impl EventSink<()> for Events {
    fn emit(&mut self, data: Data) -> Task<'_, bool> {
        self.0.lock().unwrap().push(data);
        Box::pin(async { true })
    }
}
fn server(tx: &mut mpsc::Sender<Result<Frame, String>>, msg: ServerMsg) {
    tx.try_send(Ok(Frame::Binary(rmp_serde::to_vec_named(&msg).unwrap())))
        .unwrap();
}
fn hello() -> ServerMsg {
    ServerMsg::Hello(Hello {
        version: PROTOCOL_VERSION,
        heartbeat_interval_ms: 1000,
        heartbeat_timeout_ms: 5000,
        max_frame_bytes: MAX_WS_FRAME_BYTES,
        server_time: 0,
    })
}
#[test]
fn injected_driver_correlates_heartbeats_retries_applied_cursor_and_expires_receipts() {
    let clock = ManualClock::default();
    let log = Arc::new(Mutex::new(Vec::<(usize, ClientMsg)>::new()));
    let mut peers = Vec::new();
    let mut pairs = VecDeque::new();
    for index in 0..3 {
        let (tx, rx) = mpsc::channel(32);
        peers.push(tx);
        let log = log.clone();
        let write = futures_util::sink::unfold(log, move |log, frame| async move {
            if let Frame::Binary(bytes) = frame {
                log.lock()
                    .unwrap()
                    .push((index, rmp_serde::from_slice(&bytes).unwrap()));
            }
            Ok::<_, String>(log)
        });
        pairs.push_back((Box::pin(write) as Outgoing, Box::pin(rx) as Incoming));
    }
    let wire = Wire(Mutex::new(pairs));
    let events = Events::default();
    let mut output = events.clone();
    let config = Config {
        server: "https://example.com".into(),
        token: "secret".into(),
        mock: false,
        generation: 1,
        scenario: String::new(),
    };
    let (mut tx, rx) = mpsc::channel(32);
    let ready = crate::mock::Mock::new().ready();
    let cursor = ready.cursor.clone();
    let mut driver = Box::pin(protocol::run(
        &config,
        &wire,
        &clock,
        &Inline,
        Inline,
        rx.map(|request| (request, ())),
        &mut output,
    ));
    server(&mut peers[0], hello());
    server(&mut peers[0], ServerMsg::Ready(Box::new(ready.clone())));
    assert!(poll(driver.as_mut()).is_pending());
    tx.try_send(Request::Applied(cursor.clone())).unwrap();
    for nonce in [42, 43] {
        tx.try_send(Request::Command(CommandRequest {
            nonce,
            command: Command::SetPresence {
                status: PresenceStatus::Idle,
            },
        }))
        .unwrap();
    }
    assert!(poll(driver.as_mut()).is_pending());
    clock.advance(1000);
    assert!(poll(driver.as_mut()).is_pending());
    let beat = log
        .lock()
        .unwrap()
        .iter()
        .find_map(|(_, msg)| {
            if let ClientMsg::Heartbeat(v) = msg {
                Some(v.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(beat.last_seq, cursor.seq);
    server(
        &mut peers[0],
        ServerMsg::HeartbeatAck(HeartbeatAck {
            nonce: beat.nonce + 1,
            server_time: 0,
        }),
    );
    clock.advance(1000);
    assert!(poll(driver.as_mut()).is_pending());
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|(_, msg)| matches!(msg, ClientMsg::Heartbeat(_)))
            .count(),
        1
    );
    server(
        &mut peers[0],
        ServerMsg::HeartbeatAck(HeartbeatAck {
            nonce: beat.nonce,
            server_time: 0,
        }),
    );
    server(
        &mut peers[0],
        ServerMsg::Response(Box::new(CommandResponse {
            nonce: 43,
            cursor: cursor.clone(),
            outcome: CommandOutcome::Ok(CommandResult::Ack),
            replayed: false,
        })),
    );
    assert!(poll(driver.as_mut()).is_pending());
    clock.advance(1000);
    assert!(poll(driver.as_mut()).is_pending());
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|(_, msg)| matches!(msg, ClientMsg::Heartbeat(_)))
            .count(),
        2
    );
    peers[0].try_send(Ok(Frame::Close(None))).unwrap();
    assert!(poll(driver.as_mut()).is_pending());
    tx.try_send(Request::Command(CommandRequest {
        nonce: 99,
        command: Command::SetPresence {
            status: PresenceStatus::Online,
        },
    }))
    .unwrap();
    assert!(poll(driver.as_mut()).is_pending());
    assert!(events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|data| matches!(data, Data::Rejected { nonce: 99, .. })));
    clock.advance(1251);
    server(&mut peers[1], hello());
    server(
        &mut peers[1],
        ServerMsg::Resumed(Resumed {
            cursor: cursor.clone(),
            replayed: 0,
        }),
    );
    assert!(poll(driver.as_mut()).is_pending());
    assert!(log.lock().unwrap().iter().any(|(index, msg)| *index == 1
        && matches!(msg,ClientMsg::Identify(v) if v.resume.as_ref()==Some(&cursor))));
    let retried: Vec<_> = log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(index, msg)| {
            if let (1, ClientMsg::Command(v)) = (index, msg) {
                Some(v.nonce)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(retried, vec![42]);
    peers[1].try_send(Ok(Frame::Close(None))).unwrap();
    assert!(poll(driver.as_mut()).is_pending());
    clock.advance(590_001);
    server(&mut peers[2], hello());
    server(&mut peers[2], ServerMsg::Ready(Box::new(ready)));
    assert!(poll(driver.as_mut()).is_pending());
    assert!(events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|data| matches!(data,Data::Uncertain(nonces) if nonces==&vec![42])));
    assert!(!log
        .lock()
        .unwrap()
        .iter()
        .any(|(index, msg)| *index == 2 && matches!(msg, ClientMsg::Command(_))));
    peers[2].try_send(Ok(Frame::Close(Some(4001)))).unwrap();
    assert!(poll(driver.as_mut()).is_ready());
    assert!(events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|data| matches!(data, Data::State(crate::model::ConnectionState::Stopped(_)))));
}
type HttpLog = Vec<(String, String, Vec<(String, String)>, Vec<u8>)>;
#[derive(Clone)]
struct ScriptedHttp {
    responses: Arc<Mutex<VecDeque<HttpResponse>>>,
    log: Arc<Mutex<HttpLog>>,
}
impl HttpTransport for ScriptedHttp {
    fn execute(&self, request: HttpRequest) -> Task<'_, Result<HttpResponse, String>> {
        Box::pin(async move {
            assert_eq!(request.max_response, 2 * 1024 * 1024);
            let bytes = match request.body {
                Body::Empty => vec![],
                Body::Bytes(v) => v,
                Body::Stream { chunks, length } => {
                    let bytes = http::limited(chunks, length as usize).await?;
                    assert_eq!(bytes.len(), length as usize);
                    bytes
                }
            };
            self.log.lock().unwrap().push((
                request.method.into(),
                request.url,
                request.headers,
                bytes,
            ));
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected HTTP request"))
        })
    }
}
struct MemorySource(Vec<u8>);
impl UploadSource for MemorySource {
    fn filename(&self) -> String {
        "fixture.txt".into()
    }
    fn byte_size(&self) -> Task<'_, Result<u64, String>> {
        Box::pin(async { Ok(self.0.len() as u64) })
    }
    fn chunks(&self, offset: u64, count: u64) -> Task<'_, Result<http::ByteStream, String>> {
        let bytes = self.0[offset as usize..(offset + count) as usize].to_vec();
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::iter([Ok(bytes)])) as http::ByteStream)
        })
    }
}
#[test]
fn http_upload_hashes_and_uses_durable_offset_then_finishes() {
    let result = UploadResponse {
        attachment: Attachment {
            id: Id(123),
            filename: "fixture.txt".into(),
            content_type: "text/plain".into(),
            byte_size: 8,
            width: None,
            height: None,
            thumbnail: None,
        },
        expires_at: 0,
    };
    let upload = ResumableUpload {
        upload_id: "ticket".into(),
        offset: 0,
        byte_size: 8,
        chunk_size: 3,
        expires_at: 0,
        completed: None,
    };
    let mut responses = VecDeque::new();
    for offset in [0, 2, 5, 8] {
        let mut upload = upload.clone();
        upload.offset = offset;
        responses.push_back(HttpResponse {
            status: 200,
            bytes: serde_json::to_vec(&upload).unwrap(),
        });
    }
    responses.push_back(HttpResponse {
        status: 200,
        bytes: serde_json::to_vec(&result).unwrap(),
    });
    let adapter = ScriptedHttp {
        responses: Arc::new(Mutex::new(responses)),
        log: Arc::default(),
    };
    let http = http::Http::new("https://example.com/", "token", adapter.clone()).unwrap();
    assert_eq!(
        complete(http.upload_resumable(
            &MemorySource(b"abcdefgh".to_vec()),
            UploadPurpose::Message,
            Some(Id(12))
        ))
        .unwrap(),
        result
    );
    let log = adapter.log.lock().unwrap();
    assert_eq!(log.len(), 5);
    let create: CreateUploadRequest = serde_json::from_slice(&log[0].3).unwrap();
    use sha2::Digest;
    assert_eq!(
        create.sha256,
        format!("{:x}", sha2::Sha256::digest(b"abcdefgh"))
    );
    assert_eq!(log[2].0, "PUT");
    assert!(log[2].2.contains(&("Upload-Offset".into(), "2".into())));
    assert_eq!(log[2].3, b"cde");
    assert_eq!(log[3].3, b"fgh");
    assert!(log[4].1.ends_with("/finish"));
    assert!(log.iter().all(|request| request
        .2
        .contains(&("Authorization".into(), "Bearer token".into()))));
}
#[test]
fn state_read_permissions_mentions_and_timestamp_formatting_are_portable() {
    let mock = crate::mock::Mock::new();
    let page = mock.history(Id(12), None);
    let id = page.items[0].id;
    let mut state = crate::state::ClientState {
        screen: crate::state::Screen::Channel(Id(12)),
        connection: crate::model::ConnectionState::Ready,
        ..Default::default()
    };
    state.model.ready(mock.ready());
    assert!(state.older_history(Id(12), page));
    state.read_id = Some(id);
    assert!(state.read_command().is_some());
    state.mark_unread(id);
    assert!(state.read_command().is_none());
    assert!(state
        .model
        .permissions(Some(Id(10)))
        .contains(Permissions::MANAGE_GUILD));
    assert_eq!(state.model.mention_candidates("hi @mi"), vec![Id(2)]);
    assert!(state.model.mention_candidates("@mi ka").is_empty());
    let today = crate::datetime::Parts {
        year: 2026,
        month: 11,
        day: 12,
        hour: 16,
        minute: 12,
    };
    let yesterday = crate::datetime::Parts { day: 11, ..today };
    assert_eq!(
        crate::datetime::label(today, today, yesterday, false),
        "Today at 4:12 PM"
    );
    assert_eq!(
        crate::datetime::label(yesterday, today, yesterday, false),
        "Yesterday at 4:12 PM"
    );
    assert_eq!(
        crate::datetime::label(today, today, yesterday, true),
        "4:12 PM"
    );
}
