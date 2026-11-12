//! Transport boundary: one iced/Tokio executor, one WebSocket, bounded queues.
use crate::{
    mock::Mock,
    model::{ConnectionState, Recovery},
};
use futures_util::{SinkExt, Stream, StreamExt};
use iced::{futures::channel::mpsc, stream};
use reqwest::Client;
use serde::de::DeserializeOwned;
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use sticktalk_proto::*;
use tokio::sync::{mpsc as channel, OwnedSemaphorePermit, Semaphore};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{client::IntoClientRequest, Message as Frame},
};

#[derive(Clone, Hash, PartialEq, Eq)]
pub struct Config {
    pub server: String,
    pub token: String,
    pub mock: bool,
    pub generation: u64,
    pub scenario: String,
}
#[derive(Clone)]
pub struct Session {
    pub config: Config,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session(<redacted>)")
    }
}
#[derive(Debug, Clone)]
pub enum Request {
    Command(CommandRequest),
    Applied(ResumeCursor),
    View,
    History {
        seek: Option<u64>,
        channel: Id,
        before: Option<Id>,
        generation: u64,
    },
    NewerHistory {
        channel: Id,
        after: Id,
        generation: u64,
    },
    AroundHistory {
        channel: Id,
        anchor: HistoryAnchor,
        seek: u64,
        generation: u64,
    },
    Channel {
        channel: Id,
        generation: u64,
    },
    Members {
        guild: Id,
        after: Option<Id>,
        generation: u64,
    },
    Invites {
        after: Option<Id>,
        guild: Id,
        generation: u64,
    },
    Bans {
        after: Option<Id>,
        guild: Id,
        generation: u64,
    },
    Upload {
        form_revision: u64,
        path: PathBuf,
        purpose: UploadPurpose,
        channel: Option<Id>,
        generation: u64,
    },
    Users {
        ids: Vec<Id>,
        generation: u64,
    },
    Message {
        channel: Id,
        message: Id,
        generation: u64,
    },
    DiscardUpload(Id),
    SaveOriginal {
        attachment: Id,
        path: PathBuf,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryAnchor {
    Message(Id),
    Timestamp(UnixMillis),
}
impl HistoryAnchor {
    /// A message ID or an ISO calendar date at midnight UTC, bounded to Snowflake time.
    pub fn parse(value: &str) -> Result<Self, String> {
        let invalid = || "Enter a message ID or a date as YYYY-MM-DD.".to_string();
        if let Ok(id) = value.parse::<u64>() {
            return if (1..=i64::MAX as u64).contains(&id) {
                Ok(Self::Message(Id(id)))
            } else {
                Err(invalid())
            };
        }
        let parts: Vec<_> = value.split('-').collect();
        if parts.len() != 3
            || !parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
            || parts[0].len() != 4
            || parts[1].len() != 2
            || parts[2].len() != 2
        {
            return Err(invalid());
        }
        let year: u64 = parts[0].parse().map_err(|_| invalid())?;
        let month: usize = parts[1].parse().map_err(|_| invalid())?;
        let day: u64 = parts[2].parse().map_err(|_| invalid())?;
        if !(2025..=2094).contains(&year) || !(1..=12).contains(&month) {
            return Err(invalid());
        }
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let months = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        if day == 0 || day > months[month - 1] {
            return Err(invalid());
        }
        let leaps = |y: u64| (y - 1) / 4 - (y - 1) / 100 + (y - 1) / 400;
        let days = 365 * (year - 1970) + leaps(year) - leaps(1970)
            + months[..month - 1].iter().sum::<u64>()
            + day
            - 1;
        let time = days * 86_400_000;
        if time - SNOWFLAKE_EPOCH_MS > (i64::MAX as u64 >> 22) {
            return Err(invalid());
        }
        Ok(Self::Timestamp(time))
    }

    fn query(self) -> String {
        match self {
            Self::Message(id) => format!("around={}", id.0),
            Self::Timestamp(time) => format!("around_timestamp={time}"),
        }
    }
}
#[derive(Debug)]
struct Outbound {
    request: Request,
    _permit: OwnedSemaphorePermit,
}
#[derive(Debug, Clone)]
pub struct Handle {
    sender: channel::Sender<Outbound>,
    budget: Arc<Semaphore>,
}
impl Handle {
    pub fn try_send(&self, request: Request) -> Result<(), String> {
        let bytes = match &request {
            Request::Command(v) => rmp_serde::to_vec_named(&ClientMsg::Command(v.clone()))
                .map_err(|_| "Could not encode action".to_string())?
                .len(),
            Request::Upload { path, .. } | Request::SaveOriginal { path, .. } => {
                path.to_string_lossy().len() + 512
            }
            _ => 1024,
        };
        if bytes > MAX_WS_FRAME_BYTES as usize {
            return Err("Action exceeds the protocol frame limit".into());
        }
        let permit = self
            .budget
            .clone()
            .try_acquire_many_owned(bytes.max(1) as u32)
            .map_err(|_| "The 2 MiB network queue is full. Please wait.".to_string())?;
        self.sender
            .try_send(Outbound {
                request,
                _permit: permit,
            })
            .map_err(|_| "Connection busy or unavailable. Please retry.".into())
    }
}

#[derive(Debug, Clone)]
pub enum Data {
    Connected(Handle),
    Rejected {
        nonce: Nonce,
        reason: String,
    },
    Uncertain(Vec<Nonce>),
    State(ConnectionState),
    Frame(ServerMsg),
    History {
        seek: Option<u64>,
        channel: Id,
        page: Page<Message>,
        generation: u64,
    },
    NewerHistory {
        channel: Id,
        page: Page<Message>,
        generation: u64,
    },
    AroundHistory {
        channel: Id,
        anchor: HistoryAnchor,
        seek: u64,
        page: Page<Message>,
        generation: u64,
    },
    Channel {
        snapshot: Snapshot<Channel>,
        generation: u64,
    },
    Members {
        guild: Id,
        after: Option<Id>,
        page: Page<Member>,
        generation: u64,
    },
    Invites {
        guild: Id,
        after: Option<Id>,
        page: Page<Invite>,
        generation: u64,
    },
    Bans {
        guild: Id,
        after: Option<Id>,
        page: Page<Ban>,
        generation: u64,
    },
    Uploaded {
        form_revision: u64,
        response: UploadResponse,
        purpose: UploadPurpose,
        channel: Option<Id>,
        generation: u64,
    },
    Error(String),
    Saved,
    Users {
        snapshot: Snapshot<Vec<UserSummary>>,
        generation: u64,
    },
    Message {
        snapshot: Snapshot<Message>,
        generation: u64,
    },
}
#[derive(Debug, Clone)]
pub struct Delivery {
    pub data: Arc<Data>,
    _permit: Arc<OwnedSemaphorePermit>,
}
async fn emit(output: &mut mpsc::Sender<Delivery>, budget: &Arc<Semaphore>, data: Data) -> bool {
    // Every payload-bearing variant contributes its actual named-map size;
    // small-control estimates must not undercount bulk users or error strings.
    let encoded =
        |v: Result<Vec<u8>, rmp_serde::encode::Error>| v.map_or(2 * 1024 * 1024 + 1, |v| v.len());
    let bytes = match &data {
        Data::Frame(v) => encoded(rmp_serde::to_vec_named(v)),
        Data::History { page, .. }
        | Data::AroundHistory { page, .. }
        | Data::NewerHistory { page, .. } => encoded(rmp_serde::to_vec_named(page)),
        Data::Members { page, .. } => encoded(rmp_serde::to_vec_named(page)),
        Data::Invites { page, .. } => encoded(rmp_serde::to_vec_named(page)),
        Data::Bans { page, .. } => encoded(rmp_serde::to_vec_named(page)),
        Data::Channel { snapshot, .. } => encoded(rmp_serde::to_vec_named(snapshot)),
        Data::Users { snapshot, .. } => encoded(rmp_serde::to_vec_named(snapshot)),
        Data::Message { snapshot, .. } => encoded(rmp_serde::to_vec_named(snapshot)),
        Data::Uploaded { response, .. } => encoded(rmp_serde::to_vec_named(response)),
        Data::Error(v)
        | Data::Rejected { reason: v, .. }
        | Data::State(ConnectionState::Stopped(v)) => v.len(),
        Data::Uncertain(v) => v.len() * std::mem::size_of::<Nonce>(),
        _ => 128,
    }
    .saturating_add(128);
    if bytes > 2 * 1024 * 1024 {
        return false;
    }
    let Ok(permit) = budget.clone().acquire_many_owned(bytes as u32).await else {
        return false;
    };
    output
        .send(Delivery {
            data: Arc::new(data),
            _permit: Arc::new(permit),
        })
        .await
        .is_ok()
}
pub fn subscription(config: &Config) -> impl Stream<Item = Delivery> {
    let config = config.clone();
    stream::channel(128, move |mut output| async move {
        let budget = Arc::new(Semaphore::new(2 * 1024 * 1024));
        let (tx, mut rx) = channel::channel::<Outbound>(128);
        if !emit(
            &mut output,
            &budget,
            Data::Connected(Handle {
                sender: tx,
                budget: Arc::new(Semaphore::new(2 * 1024 * 1024)),
            }),
        )
        .await
        {
            return;
        }
        if config.mock {
            let mut mock = Mock::new();
            mock.configure(&config.scenario);
            let dir = crate::media::namespace(&config, Id(1));
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(
                dir.join("900001.thumb"),
                include_bytes!("../assets/mock-workspace.jpg"),
            );
            if config.scenario == "image-heavy" || config.scenario == "stress-scroll" {
                for n in 0..100u64 {
                    let _ = std::fs::write(
                        dir.join(format!("{}.thumb", 910_000 + n)),
                        include_bytes!("../assets/mock-workspace.jpg"),
                    );
                }
            }
            if !config.token.is_empty() {
                mock.model.me.as_mut().unwrap().user.username = config.token.clone();
            }
            if !emit(
                &mut output,
                &budget,
                Data::Frame(ServerMsg::Ready(Box::new(mock.ready()))),
            )
            .await
            {
                return;
            }
            if config.scenario == "disconnected" {
                emit(
                    &mut output,
                    &budget,
                    Data::State(ConnectionState::Backoff(4)),
                )
                .await;
            }
            for user_id in [Id(2), Id(3)] {
                let seq = mock.model.cursor.as_ref().unwrap().seq + 1;
                mock.model.cursor.as_mut().unwrap().seq = seq;
                emit(
                    &mut output,
                    &budget,
                    Data::Frame(ServerMsg::Event(Box::new(EventEnvelope {
                        seq,
                        event: Event::Typing {
                            channel_id: Id(12),
                            user_id,
                            expires_at: crate::mock::now() + 8000,
                        },
                    }))),
                )
                .await;
            }
            while let Some(outbound) = rx.recv().await {
                let request = outbound.request;
                if config.scenario == "loading" && matches!(request, Request::History { .. }) {
                    continue;
                }
                let data = match request {
                    Request::Command(v) => {
                        for frame in mock.command(v) {
                            if !emit(&mut output, &budget, Data::Frame(frame)).await {
                                return;
                            }
                        }
                        continue;
                    }
                    Request::Applied(_) | Request::View => continue,
                    Request::NewerHistory {
                        channel,
                        after,
                        generation,
                    } => {
                        let mut items: Vec<_> = mock
                            .model
                            .messages
                            .get(&channel)
                            .into_iter()
                            .flat_map(|m| m.values())
                            .filter(|m| m.id > after)
                            .take(51)
                            .cloned()
                            .collect();
                        let has_more = items.len() > 50;
                        items.truncate(50);
                        let next_after = items.last().map(|m| m.id).filter(|_| has_more);
                        Data::NewerHistory {
                            channel,
                            generation,
                            page: Page {
                                items,
                                users: mock.model.users.values().cloned().collect(),
                                next_before: None,
                                next_after,
                                has_more,
                                cursor: mock.cursor(),
                            },
                        }
                    }
                    Request::AroundHistory {
                        channel,
                        anchor,
                        seek,
                        generation,
                    } => {
                        let pivot = match anchor {
                            HistoryAnchor::Message(id) => id.0,
                            HistoryAnchor::Timestamp(t) => {
                                t.saturating_sub(SNOWFLAKE_EPOCH_MS) << 22
                            }
                        };
                        let messages = mock.model.messages.get(&channel);
                        let mut left: Vec<_> = messages
                            .into_iter()
                            .flat_map(|m| m.values())
                            .filter(|m| m.id.0 <= pivot)
                            .rev()
                            .take(25)
                            .cloned()
                            .collect();
                        let right: Vec<_> = messages
                            .into_iter()
                            .flat_map(|m| m.values())
                            .filter(|m| m.id.0 > pivot)
                            .take(50 - left.len())
                            .cloned()
                            .collect();
                        left.extend(right);
                        left.sort_by_key(|m| std::cmp::Reverse(m.id));
                        let mut page = Page {
                            items: left,
                            users: mock.model.users.values().cloned().collect(),
                            next_before: None,
                            next_after: None,
                            has_more: false,
                            cursor: mock.cursor(),
                        };
                        page.next_before = page
                            .items
                            .last()
                            .map(|m| m.id)
                            .filter(|id| messages.is_some_and(|ms| ms.keys().any(|v| v < id)));
                        page.next_after = page
                            .items
                            .first()
                            .map(|m| m.id)
                            .filter(|id| messages.is_some_and(|ms| ms.keys().any(|v| v > id)));
                        page.has_more = page.next_before.is_some() || page.next_after.is_some();
                        Data::AroundHistory {
                            channel,
                            anchor,
                            seek,
                            page,
                            generation,
                        }
                    }
                    Request::History {
                        seek,
                        channel,
                        before,
                        generation,
                    } => Data::History {
                        seek,
                        channel,
                        page: mock.history(channel, before),
                        generation,
                    },
                    Request::Channel {
                        channel,
                        generation,
                    } => {
                        let Some(c) = mock.model.channels.get(&channel) else {
                            continue;
                        };
                        Data::Channel {
                            snapshot: Snapshot {
                                data: Channel {
                                    summary: c.clone(),
                                    topic: mock.model.topics.get(&channel).cloned().flatten(),
                                },
                                cursor: mock.cursor(),
                            },
                            generation,
                        }
                    }
                    Request::Members {
                        guild,
                        after,
                        generation,
                    } => Data::Members {
                        guild,
                        after,
                        page: mock.page(
                            mock.model
                                .members
                                .values()
                                .filter(|m| {
                                    m.guild_id == guild && after.is_none_or(|v| m.user.id > v)
                                })
                                .cloned()
                                .collect(),
                        ),
                        generation,
                    },
                    Request::Invites {
                        guild,
                        after,
                        generation,
                    } => Data::Invites {
                        guild,
                        after,
                        page: mock.page(
                            mock.model
                                .invites
                                .values()
                                .filter(|v| v.guild_id == guild && after.is_none_or(|id| v.id > id))
                                .cloned()
                                .collect(),
                        ),
                        generation,
                    },
                    Request::Bans {
                        guild,
                        after,
                        generation,
                    } => Data::Bans {
                        guild,
                        after,
                        page: mock.page(
                            mock.model
                                .bans
                                .values()
                                .filter(|v| {
                                    v.guild_id == guild && after.is_none_or(|id| v.user.id > id)
                                })
                                .cloned()
                                .collect(),
                        ),
                        generation,
                    },
                    Request::Upload {
                        form_revision,
                        path,
                        purpose,
                        channel,
                        generation,
                    } => match mock.upload(&path, purpose, &config) {
                        Ok(response) => Data::Uploaded {
                            form_revision,
                            response,
                            purpose,
                            channel,
                            generation,
                        },
                        Err(v) => Data::Error(v),
                    },
                    Request::Users { ids, generation } => Data::Users {
                        snapshot: Snapshot {
                            data: ids
                                .iter()
                                .filter_map(|id| mock.model.users.get(id).cloned())
                                .collect(),
                            cursor: mock.cursor(),
                        },
                        generation,
                    },
                    Request::Message {
                        channel,
                        message,
                        generation,
                    } => match mock
                        .model
                        .messages
                        .get(&channel)
                        .and_then(|m| m.get(&message))
                    {
                        Some(m) => Data::Message {
                            snapshot: Snapshot {
                                data: m.clone(),
                                cursor: mock.cursor(),
                            },
                            generation,
                        },
                        None => {
                            Data::Error("The original message was deleted or is unavailable".into())
                        }
                    },
                    Request::DiscardUpload(_) => continue,
                    Request::SaveOriginal { attachment, path } => {
                        if attachment == Id(900001) {
                            let saved = tokio::task::spawn_blocking(move || {
                                use std::io::Write;
                                std::fs::OpenOptions::new()
                                    .write(true)
                                    .create_new(true)
                                    .open(path)?
                                    .write_all(include_bytes!("../assets/mock-workspace.jpg"))
                            })
                            .await;
                            match saved {
                                Ok(Ok(())) => Data::Saved,
                                _ => Data::Error(
                                    "Could not save original. Choose a new filename.".into(),
                                ),
                            }
                        } else if let Some(source) = mock.originals.get(&attachment) {
                            let source = source.clone();
                            let saved = tokio::task::spawn_blocking(move || {
                                use std::io::Write;
                                let mut dest = std::fs::OpenOptions::new()
                                    .create_new(true)
                                    .write(true)
                                    .open(path)?;
                                let mut file = std::fs::File::open(source)?;
                                std::io::copy(&mut file, &mut dest)?;
                                dest.flush()
                            })
                            .await;
                            match saved {
                                Ok(Ok(())) => Data::Saved,
                                _ => Data::Error(
                                    "Could not save mock original; destination must not exist"
                                        .into(),
                                ),
                            }
                        } else {
                            Data::Error("Mock original is unavailable".into())
                        }
                    }
                };
                if !emit(&mut output, &budget, data).await {
                    return;
                }
            }
            return;
        }
        let http = match Http::new(&config.server, &config.token) {
            Ok(v) => v,
            Err(v) => {
                emit(&mut output, &budget, Data::Error(v)).await;
                return;
            }
        };
        let mut recovery = Recovery::default();
        let mut pending: BTreeMap<Nonce, (CommandRequest, Instant)> = BTreeMap::new();
        loop {
            emit(
                &mut output,
                &budget,
                Data::State(ConnectionState::Connecting),
            )
            .await;
            let url = format!(
                "{}/api/v1/ws",
                config
                    .server
                    .replacen("https://", "wss://", 1)
                    .replacen("http://", "ws://", 1)
            );
            let request = url.into_client_request().map(|mut v| {
                v.headers_mut()
                    .insert("Sec-WebSocket-Protocol", "sticktalk.v1".parse().unwrap());
                v
            });
            let ws = match request {
                Ok(r) => tokio::time::timeout(
                    Duration::from_secs(15),
                    connect_async_with_config(
                        r,
                        Some(
                            tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                                .max_message_size(Some(MAX_WS_FRAME_BYTES as usize))
                                .max_frame_size(Some(MAX_WS_FRAME_BYTES as usize)),
                        ),
                        false,
                    ),
                )
                .await
                .ok()
                .and_then(Result::ok),
                Err(_) => None,
            };
            let mut close = None;
            if let Some((mut socket, response)) = ws {
                if response
                    .headers()
                    .get("Sec-WebSocket-Protocol")
                    .and_then(|v| v.to_str().ok())
                    != Some("sticktalk.v1")
                {
                    emit(
                        &mut output,
                        &budget,
                        Data::Error("Server did not negotiate sticktalk.v1".into()),
                    )
                    .await;
                    return;
                }
                let connected_at = Instant::now();
                let mut identified = false;
                let mut ready = false;
                let mut beat = Duration::from_secs(30);
                let mut timeout = Duration::from_secs(75);
                let mut deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                let mut ack_at = Instant::now();
                let mut outstanding: Option<Nonce> = None;
                let mut jobs = tokio::task::JoinSet::new();
                let mut rest_queue = VecDeque::new();
                loop {
                    tokio::select! {
                                       frame=socket.next()=> {
                                           let Some(Ok(frame))=frame else {break};
                                           match frame {
                                               Frame::Binary(bytes)=> {
                                                   let frame=match decode(&bytes){Ok(v)=>v,Err(v)=>{emit(&mut output,&budget,Data::Error(v)).await;close=Some(1002);break;}};
                                                   match &frame {
                                                       ServerMsg::Hello(v)=> {
                                                           if identified||v.version!=PROTOCOL_VERSION {close=Some(4002);break;}
                                                           beat=Duration::from_millis(v.heartbeat_interval_ms.max(1000) as u64);
                                                           timeout=Duration::from_millis(v.heartbeat_timeout_ms.max(1000) as u64);
                                                           let id=ClientMsg::Identify(Identify{version:PROTOCOL_VERSION,token:config.token.clone(),resume:recovery.cursor.clone()});
                                                           if send(&mut socket,&id).await.is_err(){break;}
                                                           identified=true;
                                                       }
                                                       ServerMsg::Ready(_)|ServerMsg::Resumed(_)=> {
                                                           ready=true;recovery.state=ConnectionState::Ready;deadline=tokio::time::Instant::now()+beat;ack_at=Instant::now();outstanding=None;
                                                           // Receipt retries retain their original nonce and monotonic deadline.
                                                           for (v,started) in pending.values() {if started.elapsed()<Duration::from_secs(590)&&send(&mut socket,&ClientMsg::Command(v.clone())).await.is_err(){break;}}
                                                           let expired:Vec<_>=pending.iter().filter(|(_,(_,started))|started.elapsed()>=Duration::from_secs(590)).map(|(n,_)|*n).collect();
                                                               pending.retain(|_,(_,started)|started.elapsed()<Duration::from_secs(590));
                                                               if !expired.is_empty(){emit(&mut output,&budget,Data::Uncertain(expired)).await;}
                                                       }
                                                       ServerMsg::ResyncRequired(_)=> {ready=false;recovery.resync();}
                                                       ServerMsg::HeartbeatAck(v)=> {if outstanding==Some(v.nonce){outstanding=None;ack_at=Instant::now();}}
                                                       ServerMsg::Response(v)=> {pending.remove(&v.nonce);}
                                                       ServerMsg::Error(v) if matches!(v.code,ErrorCode::Unauthenticated|ErrorCode::SessionExpired)=>{close=Some(4001);}
                                                       _=>{}
                                                   }
                                                   if !emit(&mut output,&budget,Data::Frame(frame)).await{return;}
                                                   if close==Some(4001){break;}
                                               }
                                               Frame::Close(v)=>{close=v.map(|v|u16::from(v.code));break;}
                                               Frame::Ping(v)=> {if socket.send(Frame::Pong(v)).await.is_err(){break;}}
                                               Frame::Text(_)=>{close=Some(1003);break;}
                                               _=>{}
                                           }
                                       }
                                       request=rx.recv()=> {
                                           let Some(outbound)=request else {return};
                                               let request=outbound.request;
                                           match request {
                                               Request::Applied(v)=>{if recovery.cursor.is_none(){recovery.fresh(v);}else{recovery.applied(v);}},
                                                   Request::View=>{jobs.abort_all();rest_queue.clear();},
                                               Request::Command(v)=> {
                                                   if !ready {emit(&mut output,&budget,Data::Rejected{nonce:v.nonce,reason:"You are offline. This action was not queued.".into()}).await;continue;}
                                                   if pending.len()>=32 {emit(&mut output,&budget,Data::Rejected{nonce:v.nonce,reason:"32 actions are in flight. Please wait.".into()}).await;continue;}
                                                   if !matches!(v.command,Command::Typing{..}|Command::SubscribeGuild{..}){pending.insert(v.nonce,(v.clone(),Instant::now()));}
                                                   if send(&mut socket,&ClientMsg::Command(v)).await.is_err(){break;}
                                               }
                                               v=> {if jobs.len()>=2 {if rest_queue.len()<16{rest_queue.push_back(v);}else{emit(&mut output,&budget,Data::Error("Download queue is full. Try again shortly.".into())).await;}}else{let http=http.clone();jobs.spawn(async move{http.request(v).await});}}
                                           }
                                       }
                                       result=jobs.join_next(),if !jobs.is_empty()=>{
                        if let Some(Ok(result))=result {let data=result.unwrap_or_else(Data::Error);if !emit(&mut output,&budget,data).await{return;}}
                        if let Some(v)=rest_queue.pop_front(){let http=http.clone();jobs.spawn(async move{http.request(v).await});}
                    }
                    _=tokio::time::sleep_until(deadline)=>{
                                           if !ready {break;}
                                           if ack_at.elapsed()>=timeout {break;}
                                           if outstanding.is_none(){let nonce=nonce();outstanding=Some(nonce);let msg=ClientMsg::Heartbeat(Heartbeat{nonce,last_seq:recovery.cursor.as_ref().map_or(0,|v|v.seq)});if send(&mut socket,&msg).await.is_err(){break;}}
                                           if connected_at.elapsed()>=Duration::from_secs(60){recovery.healthy();}
                                           deadline=tokio::time::Instant::now()+beat.min(timeout.saturating_sub(ack_at.elapsed()));
                                       }
                                   }
                }
                jobs.abort_all();
            }
            let delay = recovery.disconnected(close);
            if !emit(&mut output, &budget, Data::State(recovery.state.clone())).await {
                return;
            }
            let Some(delay) = delay else { return };
            let jitter = (nonce() % 251) as f64 / 1000.0;
            let wait = tokio::time::sleep(Duration::from_secs_f64(delay as f64 + jitter));
            tokio::pin!(wait);
            loop {
                tokio::select! {_=&mut wait=>break,request=rx.recv()=>match request.map(|v|v.request) {Some(Request::Applied(v))=>recovery.applied(v),Some(Request::Command(v))=>{emit(&mut output,&budget,Data::Rejected{nonce:v.nonce,reason:"Offline. This action was not queued.".into()}).await;},None=>return,_=>{emit(&mut output,&budget,Data::Error("Offline. Action not queued.".into())).await;}}}
            }
        }
    })
}
async fn send<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    msg: &ClientMsg,
) -> Result<(), ()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let bytes = rmp_serde::to_vec_named(msg).map_err(|_| ())?;
    if bytes.len() > MAX_WS_FRAME_BYTES as usize {
        return Err(());
    }
    socket
        .send(Frame::Binary(bytes.into()))
        .await
        .map_err(|_| ())
}
pub fn decode(bytes: &[u8]) -> Result<ServerMsg, String> {
    if bytes.len() > MAX_WS_FRAME_BYTES as usize {
        return Err("Server frame exceeds 512 KiB".into());
    }
    let mut de = rmp_serde::Deserializer::new(std::io::Cursor::new(bytes));
    de.set_max_depth(32);
    let v = serde::Deserialize::deserialize(&mut de)
        .map_err(|_| "Invalid server protocol frame".to_string())?;
    if de.position() != bytes.len() as u64 {
        return Err("Trailing bytes in server frame".into());
    }
    Ok(v)
}
pub fn nonce() -> u64 {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).expect("OS random source unavailable");
    (u64::from_le_bytes(bytes) & (i64::MAX as u64)).max(1)
}

#[derive(Clone)]
pub struct Http {
    pub base: String,
    token: String,
    pub client: Client,
}
impl Http {
    pub fn new(server: &str, token: &str) -> Result<Self, String> {
        let url = reqwest::Url::parse(server)
            .map_err(|_| "Enter a valid http(s) server address".to_string())?;
        if url.scheme() != "https"
            && !(url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
        {
            return Err("Use HTTPS; HTTP is allowed only on loopback".into());
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Server address must not contain credentials, query or fragment".into());
        }
        Ok(Self {
            base: server.trim_end_matches('/').to_string(),
            token: token.into(),
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Could not initialize HTTPS".to_string())?,
        })
    }
    pub fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .get(format!("{}/api/v1{}", self.base, path))
            .bearer_auth(&self.token)
    }
    pub async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl serde::Serialize,
    ) -> Result<T, String> {
        json(
            self.client
                .post(format!("{}/api/v1{}", self.base, path))
                .bearer_auth(&self.token)
                .json(body)
                .send()
                .await
                .map_err(|_| "Server request failed".to_string())?,
        )
        .await
    }
    pub async fn json<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        json(
            self.get(path)
                .send()
                .await
                .map_err(|_| "Server request failed".to_string())?,
        )
        .await
    }
    pub async fn request(&self, request: Request) -> Result<Data, String> {
        match request {
            Request::NewerHistory {
                channel,
                after,
                generation,
            } => Ok(Data::NewerHistory {
                channel,
                generation,
                page: self
                    .json(&format!(
                        "/channels/{}/messages?limit=50&after={}",
                        channel.0, after.0
                    ))
                    .await?,
            }),
            Request::AroundHistory {
                channel,
                anchor,
                seek,
                generation,
            } => Ok(Data::AroundHistory {
                channel,
                anchor,
                seek,
                generation,
                page: self
                    .json(&format!(
                        "/channels/{}/messages?limit=50&{}",
                        channel.0,
                        anchor.query()
                    ))
                    .await?,
            }),
            Request::History {
                seek,
                channel,
                before,
                generation,
            } => Ok(Data::History {
                seek,
                channel,
                page: self
                    .json(&format!(
                        "/channels/{}/messages?limit=50{}",
                        channel.0,
                        before.map_or(String::new(), |v| format!("&before={}", v.0))
                    ))
                    .await?,
                generation,
            }),
            Request::Channel {
                channel,
                generation,
            } => Ok(Data::Channel {
                snapshot: self.json(&format!("/channels/{}", channel.0)).await?,
                generation,
            }),
            Request::Members {
                guild,
                after,
                generation,
            } => Ok(Data::Members {
                guild,
                after,
                page: self
                    .json(&format!(
                        "/guilds/{}/members?limit=50{}",
                        guild.0,
                        after.map_or(String::new(), |v| format!("&after={}", v.0))
                    ))
                    .await?,
                generation,
            }),
            Request::Invites {
                guild,
                after,
                generation,
            } => Ok(Data::Invites {
                guild,
                after,
                page: self
                    .json(&format!(
                        "/guilds/{}/invites?limit=50{}",
                        guild.0,
                        after.map_or(String::new(), |v| format!("&after={}", v.0))
                    ))
                    .await?,
                generation,
            }),
            Request::Bans {
                guild,
                after,
                generation,
            } => Ok(Data::Bans {
                guild,
                after,
                page: self
                    .json(&format!(
                        "/guilds/{}/bans?limit=50{}",
                        guild.0,
                        after.map_or(String::new(), |v| format!("&after={}", v.0))
                    ))
                    .await?,
                generation,
            }),
            Request::Upload {
                form_revision,
                path,
                purpose,
                channel,
                generation,
            } => {
                let response = self.upload_resumable(&path, purpose, channel).await?;
                Ok(Data::Uploaded {
                    form_revision,
                    response,
                    purpose,
                    channel,
                    generation,
                })
            }
            Request::SaveOriginal { attachment, path } => {
                self.download(attachment, path).await?;
                Ok(Data::Saved)
            }
            Request::Users { ids, generation } => Ok(Data::Users {
                snapshot: self
                    .json(&format!(
                        "/users?ids={}",
                        ids.iter()
                            .map(|v| v.0.to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    ))
                    .await?,
                generation,
            }),
            Request::Message {
                channel,
                message,
                generation,
            } => Ok(Data::Message {
                snapshot: self
                    .json(&format!("/channels/{}/messages/{}", channel.0, message.0))
                    .await?,
                generation,
            }),
            Request::DiscardUpload(id) => {
                self.client
                    .delete(format!("{}/api/v1/uploads/{}", self.base, id.0))
                    .bearer_auth(&self.token)
                    .send()
                    .await
                    .map_err(|_| "Could not discard upload".to_string())?;
                Ok(Data::Error(String::new()))
            }
            _ => Err("Invalid HTTP operation".into()),
        }
    }
    /// Stream/hash once, then stream bounded chunks; never buffer a whole file.
    pub async fn upload_resumable(
        &self,
        path: &std::path::Path,
        purpose: UploadPurpose,
        channel: Option<Id>,
    ) -> Result<UploadResponse, String> {
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncReadExt;
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|_| "Could not open selected file".to_string())?;
        let len = file
            .metadata()
            .await
            .map_err(|_| "Could not read file size".to_string())?
            .len();
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let count = file
                .read(&mut buffer)
                .await
                .map_err(|_| "Could not hash file".to_string())?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        let sha256 = format!("{:x}", hash.finalize());
        let upload: ResumableUpload = self
            .post(
                "/uploads/resumable",
                &CreateUploadRequest {
                    purpose,
                    channel_id: channel,
                    filename: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    content_type: "application/octet-stream".into(),
                    byte_size: len,
                    sha256,
                },
            )
            .await?;
        self.resume_upload(path, upload).await
    }
    /// Resume from the server's durable offset, not a locally assumed chunk boundary.
    pub async fn resume_upload(
        &self,
        path: &std::path::Path,
        upload: ResumableUpload,
    ) -> Result<UploadResponse, String> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let route = format!("/uploads/resumable/{}", upload.upload_id);
        let mut upload: ResumableUpload = self.json(&route).await?;
        if let Some(response) = upload.completed {
            return Ok(response);
        }
        while upload.offset < upload.byte_size {
            let mut file = tokio::fs::File::open(path)
                .await
                .map_err(|_| "Could not reopen selected file".to_string())?;
            file.seek(std::io::SeekFrom::Start(upload.offset))
                .await
                .map_err(|_| "Could not seek file".to_string())?;
            let count = (upload.byte_size - upload.offset).min(u64::from(upload.chunk_size));
            let body =
                reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file.take(count)));
            upload = json(
                self.client
                    .put(format!("{}/api/v1{route}", self.base))
                    .bearer_auth(&self.token)
                    .header("Upload-Offset", upload.offset)
                    .header("Content-Length", count)
                    .body(body)
                    .send()
                    .await
                    .map_err(|_| "Upload interrupted; retry to resume".to_string())?,
            )
            .await?;
        }
        json(
            self.client
                .post(format!("{}/api/v1{route}/finish", self.base))
                .bearer_auth(&self.token)
                .timeout(Duration::from_secs(3600))
                .send()
                .await
                .map_err(|_| "Could not finish upload".to_string())?,
        )
        .await
    }
    /// Compatibility path for older servers; bounded by their legacy multipart limit.
    pub async fn upload_legacy(
        &self,
        path: &std::path::Path,
        purpose: UploadPurpose,
        channel: Option<Id>,
    ) -> Result<UploadResponse, String> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|_| "Could not open selected file".to_string())?;
        let len = file
            .metadata()
            .await
            .map_err(|_| "Could not read file size".to_string())?
            .len();
        let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file));
        let purpose = match purpose {
            UploadPurpose::Message => "message",
            UploadPurpose::Avatar => "avatar",
            UploadPurpose::GuildIcon => "guild_icon",
        };
        let mut form = reqwest::multipart::Form::new().text("purpose", purpose.to_string());
        if let Some(id) = channel {
            form = form.text("channel_id", id.0.to_string());
        }
        form = form.part(
            "file",
            reqwest::multipart::Part::stream_with_length(body, len).file_name(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        json(
            self.client
                .post(format!("{}/api/v1/uploads", self.base))
                .bearer_auth(&self.token)
                .multipart(form)
                .send()
                .await
                .map_err(|_| "Upload failed".to_string())?,
        )
        .await
    }
    pub async fn logout(&self) {
        let _ = self
            .client
            .post(format!("{}/api/v1/auth/logout", self.base))
            .bearer_auth(&self.token)
            .send()
            .await;
    }
    pub async fn download(&self, id: Id, path: PathBuf) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        let response = self
            .get(&format!("/files/{}", id.0))
            .send()
            .await
            .map_err(|_| "Download failed".to_string())?;
        if !response.status().is_success() {
            return Err("No longer allowed to download this file".into());
        }
        let mut out = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(|_| "Destination already exists or cannot be written".to_string())?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "Download interrupted".to_string())?;
            out.write_all(&chunk)
                .await
                .map_err(|_| "Could not save file".to_string())?;
        }
        Ok(())
    }
}
async fn json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, String> {
    let status = response.status();
    let bytes = limited(response, 2 * 1024 * 1024).await?;
    if !status.is_success() {
        return Err(serde_json::from_slice::<ApiError>(&bytes)
            .map(|v| {
                if v.code==ErrorCode::LastLoginMethod {return "Set a password before unlinking Google. It is your only sign-in method.".into();}
                format!(
                    "{}{}",
                    v.message,
                    v.retry_after_ms.map_or(String::new(), |t| format!(
                        " Try again in {}s.",
                        t.div_ceil(1000)
                    ))
                )
            })
            .unwrap_or_else(|_| format!("Server returned HTTP {}", status.as_u16())));
    }
    serde_json::from_slice(&bytes).map_err(|_| "Server sent an invalid response".into())
}
pub async fn limited(response: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err("Server response exceeds limit".into());
    }
    let mut bytes = vec![];
    let mut chunks = response.bytes_stream();
    while let Some(c) = chunks.next().await {
        let c = c.map_err(|_| "Network interrupted".to_string())?;
        if bytes.len() + c.len() > max {
            return Err("Server response exceeds limit".into());
        }
        bytes.extend_from_slice(&c);
    }
    Ok(bytes)
}
pub async fn authenticate(
    server: String,
    username: String,
    password: String,
    register: bool,
    display: String,
) -> Result<Session, String> {
    let http = Http::new(&server, "")?;
    let auth = authenticate_unstored(&server, username, password, register, display).await?;
    let token = auth.token.clone();
    let saved_server = http.base.clone();
    tokio::task::spawn_blocking(move || {
        crate::storage::save_server(&saved_server);
        crate::storage::save_token(&saved_server, &token)
    })
    .await
    .map_err(|_| "Credential storage failed".to_string())??;
    Ok(Session {
        config: Config {
            server: http.base,
            token: auth.token,
            mock: false,
            generation: nonce(),
            scenario: String::new(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jump_targets_validate_dates_and_ids_without_a_timezone_or_clock() {
        assert_eq!(
            HistoryAnchor::parse("2025-01-01"),
            Ok(HistoryAnchor::Timestamp(SNOWFLAKE_EPOCH_MS))
        );
        assert_eq!(
            HistoryAnchor::parse("2028-03-01"),
            Ok(HistoryAnchor::Timestamp(
                SNOWFLAKE_EPOCH_MS + 1155 * 86_400_000
            ))
        );
        assert_eq!(
            HistoryAnchor::parse("42"),
            Ok(HistoryAnchor::Message(Id(42)))
        );
        for value in [
            "0",
            "9223372036854775808",
            "2024-12-31",
            "2026-02-29",
            "2026-13-01",
            "2026-01-00",
            "2026-1-01",
            "2095-01-01",
        ] {
            assert!(HistoryAnchor::parse(value).is_err(), "{value}");
        }
    }
    #[test]
    fn rejects_trailing_and_large_frames() {
        let msg = ServerMsg::HeartbeatAck(HeartbeatAck {
            nonce: 1,
            server_time: 0,
        });
        let mut bytes = rmp_serde::to_vec_named(&msg).unwrap();
        assert!(decode(&bytes).is_ok());
        bytes.push(0);
        assert!(decode(&bytes).is_err());
        assert!(decode(&vec![0; 524289]).is_err());
    }
    #[test]
    fn outgoing_queue_obeys_byte_budget_and_accepts_maximum_message() {
        let (sender, mut receiver) = channel::channel(128);
        let budget = Arc::new(Semaphore::new(2 * 1024 * 1024));
        let handle = Handle {
            sender,
            budget: budget.clone(),
        };
        let draft = MessageDraft {
            channel_id: Id(1),
            content: "🌱".repeat(4000),
            reply_to: None,
            attachment_ids: vec![Id(2), Id(3), Id(4), Id(5)],
            mention_ids: (1..=100).map(Id).collect(),
        };
        assert!(handle
            .try_send(Request::Command(CommandRequest {
                nonce: 1,
                command: Command::SendMessage(draft)
            }))
            .is_ok());
        drop(receiver.try_recv().unwrap());
        let mut sent = 0;
        for nonce in 1..=16 {
            let request = Request::Command(CommandRequest {
                nonce,
                command: Command::SendMessage(MessageDraft {
                    channel_id: Id(1),
                    content: "x".repeat(200_000),
                    reply_to: None,
                    attachment_ids: vec![],
                    mention_ids: vec![],
                }),
            });
            if handle.try_send(request).is_ok() {
                sent += 1;
            } else {
                break;
            }
        }
        assert!((2..=10).contains(&sent));
        while let Ok(packet) = receiver.try_recv() {
            drop(packet);
        }
        assert_eq!(budget.available_permits(), 2 * 1024 * 1024);
    }
    #[test]
    fn requires_secure_server() {
        assert!(Http::new("http://example.com", "").is_err());
        assert!(Http::new("http://127.0.0.1:8080", "").is_ok());
        assert!(Http::new("https://example.com", "").is_ok());
        assert!(Http::new("https://user:pass@example.com", "").is_err());
    }
    #[tokio::test]
    async fn inbound_budget_counts_large_non_frame_payloads_and_releases_on_drop() {
        use futures_util::StreamExt;
        let (mut tx, mut rx) = mpsc::channel(128);
        let budget = Arc::new(Semaphore::new(2 * 1024 * 1024));
        assert!(emit(&mut tx, &budget, Data::Error("x".repeat(32768))).await);
        assert_eq!(budget.available_permits(), 2 * 1024 * 1024 - 32768 - 128);
        let packet = rx.next().await.unwrap();
        drop(packet);
        assert_eq!(budget.available_permits(), 2 * 1024 * 1024);
        assert!(!emit(&mut tx, &budget, Data::Error("x".repeat(2 * 1024 * 1024))).await);
    }
}

#[derive(Clone)]
pub enum AccountAction {
    Read,
    Link(String),
    Unlink,
    Password {
        current: Option<String>,
        new: String,
    },
}
impl std::fmt::Debug for AccountAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccountAction(<redacted>)")
    }
}
pub type MockAccount = Arc<std::sync::Mutex<crate::mock::MockAuth>>;
pub async fn meta(server: String, mock: bool) -> Result<MetaResponse, String> {
    if mock {
        return Ok(MetaResponse {
            version: "0.1".into(),
            protocol_version: PROTOCOL_VERSION,
            google_client_id: Some("mock-desktop".into()),
            gifs: Default::default(),
        });
    }
    let http = Http::new(&server, "")?;
    json(
        http.client
            .get(format!("{}/api/meta", http.base))
            .send()
            .await
            .map_err(|_| "Could not read server capabilities".to_string())?,
    )
    .await
}
pub async fn account(
    config: Config,
    action: AccountAction,
    mock: MockAccount,
) -> Result<AuthMethodsResponse, String> {
    if config.mock {
        return mock
            .lock()
            .map_err(|_| "Account unavailable".to_string())?
            .action(&action);
    }
    let http = Http::new(&config.server, &config.token)?;
    let base = format!("{}/api", http.base);
    let request = match action {
        AccountAction::Read => http.client.get(format!("{base}/auth/methods")),
        AccountAction::Link(id_token) => http
            .client
            .post(format!("{base}/auth/google/link"))
            .json(&GoogleAuthRequest { id_token }),
        AccountAction::Unlink => http
            .client
            .post(format!("{base}/auth/google/unlink"))
            .json(&GoogleUnlinkRequest {}),
        AccountAction::Password { current, new } => http
            .client
            .post(format!("{base}/auth/password"))
            .json(&SetPasswordRequest {
                current_password: current,
                new_password: new,
            }),
    };
    json(
        request
            .bearer_auth(config.token)
            .send()
            .await
            .map_err(|_| "Could not update sign-in methods. Please try again.".to_string())?,
    )
    .await
}
#[derive(Clone)]
pub enum GoogleOutcome {
    Session(Session),
    Signup(GoogleSignupRequired),
}
impl std::fmt::Debug for GoogleOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GoogleOutcome(<redacted>)")
    }
}
pub async fn google(
    server: String,
    id_token: String,
    mock: bool,
    fixture: MockAccount,
) -> Result<GoogleOutcome, String> {
    let response = if mock {
        fixture
            .lock()
            .map_err(|_| "Account unavailable".to_string())?
            .google(&id_token)?
    } else {
        let http = Http::new(&server, "")?;
        json(
            http.client
                .post(format!("{}/api/auth/google", http.base))
                .json(&GoogleAuthRequest { id_token })
                .send()
                .await
                .map_err(|_| "Could not reach your server. Please try again.".to_string())?,
        )
        .await?
    };
    match response {
        GoogleAuthResponse::Session(auth) => save_auth(server, auth, mock)
            .await
            .map(GoogleOutcome::Session),
        GoogleAuthResponse::SignupRequired(ticket) => Ok(GoogleOutcome::Signup(ticket)),
    }
}
pub async fn google_complete(
    server: String,
    ticket: String,
    username: String,
    mock: bool,
    fixture: MockAccount,
) -> Result<Session, String> {
    let auth = if mock {
        fixture
            .lock()
            .map_err(|_| "Account unavailable".to_string())?
            .complete(&ticket, &username)?
    } else {
        let http = Http::new(&server, "")?;
        json(
            http.client
                .post(format!("{}/api/auth/google/complete", http.base))
                .json(&GoogleCompleteRequest { ticket, username })
                .send()
                .await
                .map_err(|_| "Could not finish signup. Please try again.".to_string())?,
        )
        .await?
    };
    save_auth(server, auth, mock).await
}
async fn save_auth(server: String, auth: AuthResponse, mock: bool) -> Result<Session, String> {
    let server = server.trim().trim_end_matches('/').to_owned();
    if !mock {
        let saved_server = server.clone();
        let token = auth.token.clone();
        tokio::task::spawn_blocking(move || {
            crate::storage::save_server(&saved_server);
            crate::storage::save_token(&saved_server, &token)
        })
        .await
        .map_err(|_| "Could not save session".to_string())??;
    }
    Ok(Session {
        config: Config {
            server,
            token: auth.token,
            mock,
            generation: nonce(),
            scenario: String::new(),
        },
    })
}

/// Authenticate without writing OS credentials/preferences (headless clients and tests).
pub async fn authenticate_unstored(
    server: &str,
    username: String,
    password: String,
    register: bool,
    display: String,
) -> Result<AuthResponse, String> {
    let http = Http::new(server, "")?;
    let url = format!(
        "{}/api/v1/auth/{}",
        http.base,
        if register { "register" } else { "login" }
    );
    let request = http.client.post(url);
    let response = if register {
        request.json(&RegisterRequest {
            username,
            password,
            display_name: if display.is_empty() {
                None
            } else {
                Some(display)
            },
        })
    } else {
        request.json(&LoginRequest { username, password })
    }
    .send()
    .await
    .map_err(|_| "Cannot reach that server. Check the address and connection.".to_string())?;
    let auth: AuthResponse = json(response).await?;
    Ok(auth)
}
