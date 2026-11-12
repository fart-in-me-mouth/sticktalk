use crate::{
    commands, db,
    error::{require, Error, Result},
    state::{self, encode, App, Connection, Packet},
};
use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket},
        OriginalUri, State, WebSocketUpgrade,
    },
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use std::error::Error as _;
use std::{sync::Arc, time::Duration};
use sticktalk_proto::*;
use tokio::sync::{mpsc, watch, Semaphore};
pub async fn upgrade(
    State(app): State<App>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    ws: std::result::Result<
        WebSocketUpgrade,
        axum::extract::ws::rejection::WebSocketUpgradeRejection,
    >,
) -> Result<Response> {
    require(uri.query().is_none(), ErrorCode::InvalidRequest)?;
    let permit = app
        .0
        .sockets
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::new(ErrorCode::Unavailable))?;
    require(
        !headers.contains_key("origin")
            || headers.get("origin").and_then(|v| v.to_str().ok())
                == Some(app.0.config.public_url.trim_end_matches('/')),
        ErrorCode::Forbidden,
    )?;
    require(
        headers
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(',').any(|s| s.trim() == "sticktalk.v1")),
        ErrorCode::VersionMismatch,
    )?;
    let cookie_token = if !headers.contains_key("authorization") {
        crate::browser::session_cookie(&headers)?.map(str::to_owned)
    } else {
        None
    };
    if let Some(token) = &cookie_token {
        require(
            headers.get("origin").and_then(|v| v.to_str().ok())
                == Some(app.0.config.public_url.trim_end_matches('/')),
            ErrorCode::Forbidden,
        )?;
        db::auth(&mut *app.0.pool.acquire().await?, token).await?;
    }
    // Native Identify stays unchanged. Browser Identify uses an empty token;
    // its credential is captured from the HttpOnly cookie at upgrade.
    let ws = ws.map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    Ok(ws
        .protocols(["sticktalk.v1"])
        .read_buffer_size(8 * 1024)
        .write_buffer_size(0)
        .max_write_buffer_size(2 * 1024 * 1024)
        .max_frame_size(MAX_WS_FRAME_BYTES as usize)
        .max_message_size(MAX_WS_FRAME_BYTES as usize)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            run(app, socket, cookie_token).await
        })
        .into_response())
}
async fn send(socket: &mut WebSocket, msg: &ServerMsg) -> Result<()> {
    socket
        .send(Message::Binary(encode(msg)?))
        .await
        .map_err(|_| Error::new(ErrorCode::Unavailable))
}
async fn fail(socket: &mut WebSocket, error: Error, code: u16) {
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        send(socket, &ServerMsg::Error(error.0)),
    )
    .await;
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: "".into(),
        })))
        .await;
}
async fn run(app: App, mut socket: WebSocket, cookie_token: Option<String>) {
    if send(
        &mut socket,
        &ServerMsg::Hello(Hello {
            version: 1,
            heartbeat_interval_ms: 30000,
            heartbeat_timeout_ms: 75000,
            max_frame_bytes: MAX_WS_FRAME_BYTES,
            server_time: state::now(),
        }),
    )
    .await
    .is_err()
    {
        return;
    }
    let first = tokio::select! {
        _ = app.0.shutdown.cancelled() => {
            let _ = tokio::time::timeout(Duration::from_secs(1),socket.send(Message::Close(Some(CloseFrame {code:1001,reason:"".into()})))).await;
            return;
        },
        frame = tokio::time::timeout(Duration::from_secs(10), socket.recv()) => frame,
    };
    let mut identify = match first {
        Ok(Some(Ok(Message::Binary(bytes)))) => match decode(&bytes) {
            Ok(ClientMsg::Identify(i)) => i,
            Ok(_) => {
                fail(&mut socket, Error::new(ErrorCode::InvalidRequest), 1008).await;
                return;
            }
            Err(e) => {
                let code = if e.0.code == ErrorCode::PayloadTooLarge {
                    1009
                } else {
                    1002
                };
                fail(&mut socket, e, code).await;
                return;
            }
        },
        Err(_) => {
            fail(&mut socket, Error::new(ErrorCode::InvalidRequest), 4008).await;
            return;
        }
        Ok(Some(Err(e))) => {
            let code = if matches!(
                e.source()
                    .and_then(|e| e.downcast_ref::<tungstenite::Error>()),
                Some(tungstenite::Error::Capacity(_))
            ) {
                1009
            } else {
                1002
            };
            fail(
                &mut socket,
                Error::new(if code == 1009 {
                    ErrorCode::PayloadTooLarge
                } else {
                    ErrorCode::InvalidRequest
                }),
                code,
            )
            .await;
            return;
        }
        _ => {
            fail(&mut socket, Error::new(ErrorCode::InvalidRequest), 1003).await;
            return;
        }
    };
    if let Some(token) = cookie_token {
        if !identify.token.is_empty() {
            fail(&mut socket, Error::new(ErrorCode::InvalidRequest), 1008).await;
            return;
        }
        identify.token = token;
    }
    if identify.version != 1 {
        fail(&mut socket, Error::new(ErrorCode::VersionMismatch), 4002).await;
        return;
    }
    if identify.resume.as_ref().is_some_and(|r| {
        r.stream_id.len() != 32
            || !r
                .stream_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        fail(&mut socket, Error::new(ErrorCode::InvalidRequest), 1002).await;
        return;
    }
    let (tx, mut rx) = mpsc::channel::<Packet>(128);
    let (close, mut close_rx) = watch::channel(None);
    let connection_id = state::stream_id();
    let setup=async {
        let mut core=app.0.core.lock().await;let mut c=app.0.pool.acquire().await?;let (key,u)=db::auth(&mut c,&identify.token).await?;
        let expires:i64=sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token_hash=?").bind(&key).fetch_one(&mut *c).await?;
        let was_online=core.streams.values().any(|s|s.user==u&&(s.connection.is_some()||(s.ever_connected&&state::now().saturating_sub(s.touched)<5000)));
        crate::voice::disconnect(&app,&mut core,&mut c,&key,None).await?;
        crate::voice::reconcile(&app,&mut core,&mut c).await?;
        core.trim();core.cursor(&key,u);let s=core.streams.get_mut(&key).ok_or_else(||Error::new(ErrorCode::Internal))?;
        if let Some(old)=s.connection.take(){let _=old.close.send(Some(4009));}
        let reason=if let Some(cursor)=&identify.resume {if cursor.stream_id!=s.id {Some(ResyncReason::StreamExpired)}else if cursor.seq>s.seq{Some(ResyncReason::CursorAhead)}else if cursor.seq<s.floor||s.ring.front().is_some_and(|e|state::now().saturating_sub(e.1)>600_000){Some(ResyncReason::CursorTooOld)}else{None}}else{None};
        let mut initial=vec![];
        let resume_ok=identify.resume.is_some()&&reason.is_none();
        if resume_ok {let cursor=identify.resume.as_ref().expect("checked");let mut count=0;for (seq,_,b) in &s.ring{if *seq>cursor.seq{let decoded:ServerMsg=rmp_serde::from_slice(b).map_err(|_|Error::new(ErrorCode::Internal))?;if matches!(decoded,ServerMsg::Event(ref e) if matches!(e.event,Event::Typing{expires_at,..} if expires_at<state::now())){continue;}initial.push(b.clone());count+=1;}}initial.push(encode(&ServerMsg::Resumed(Resumed{cursor:s.cursor(),replayed:count}))?);}else{if let Some(reason)=reason{initial.push(encode(&ServerMsg::ResyncRequired(ResyncRequired{reason}))?);}s.guild=None;s.interests.clear();}
        s.connection=Some(Connection{id:connection_id.clone(),focused:false,alive:Arc::new(std::sync::atomic::AtomicBool::new(true)),tx,budget:Arc::new(Semaphore::new(2*1024*1024)),close:close.clone()});s.touched=state::now();s.ever_connected=true;
        let ready=if !resume_ok{Some(Box::new(db::ready(&mut c,&mut core,&key,u).await?))}else{None};
        if resume_ok { crate::voice::snapshot(&mut core,&mut c,&key,u).await?; }
        if !was_online {commands::broadcast_presence(&mut c,&mut core,u).await?;}Ok::<_,Error>((key,u,expires,initial,ready))
    }.await;
    drop(identify);
    let (key, u, expires, mut initial, ready) = match setup {
        Ok(v) => v,
        Err(e) => {
            // Setup may fail after attaching (e.g. a storage read fails).
            let mut core = app.0.core.lock().await;
            for s in core.streams.values_mut() {
                if s.connection.as_ref().is_some_and(|c| c.id == connection_id) {
                    s.connection = None;
                    s.touched = state::now();
                }
            }
            drop(core);
            let code = if matches!(
                e.0.code,
                ErrorCode::Unauthenticated | ErrorCode::SessionExpired
            ) {
                4001
            } else {
                1011
            };
            fail(&mut socket, e, code).await;
            return;
        }
    };
    if let Some(ready) = ready {
        match encode(&ServerMsg::Ready(ready)) {
            Ok(bytes) => initial.push(bytes),
            Err(error) => {
                fail(&mut socket, error, 1011).await;
                disconnect(&app, &key, &connection_id, u).await;
                return;
            }
        }
    }
    let initial_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    for b in initial {
        // One deadline for the entire replay, not ten seconds per event. Also
        // honor replacement/logout/shutdown while a replay is being written.
        let sent = tokio::select! {
            _ = app.0.shutdown.cancelled() => Err(1001),
            _ = close_rx.changed() => Err(close_rx.borrow_and_update().unwrap_or(1000)),
            result = tokio::time::timeout_at(initial_deadline, socket.send(Message::Binary(b))) => {
                if result.is_ok_and(|r|r.is_ok()) {Ok(())} else {Err(4010)}
            },
        };
        if let Err(code) = sent {
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                socket.send(Message::Close(Some(CloseFrame {
                    code,
                    reason: "".into(),
                }))),
            )
            .await;
            disconnect(&app, &key, &connection_id, u).await;
            return;
        }
    }
    let (mut sink, mut source) = socket.split();
    let mut writer = tokio::spawn(async move {
        loop {
            tokio::select! {biased;
                changed=close_rx.changed()=>{if changed.is_err(){break;}let code=*close_rx.borrow_and_update();if let Some(code)=code{if matches!(code,1002|1008|1009|4001){while let Ok(packet)=rx.try_recv(){if rmp_serde::from_slice::<ServerMsg>(&packet.bytes).is_ok_and(|m|matches!(m,ServerMsg::Error(_))){let _=tokio::time::timeout(Duration::from_secs(1),sink.send(Message::Binary(packet.bytes))).await;}}}else if code==4010{if let Ok(b)=encode(&ServerMsg::ResyncRequired(ResyncRequired{reason:ResyncReason::SlowConsumer})){let _=tokio::time::timeout(Duration::from_secs(1),sink.send(Message::Binary(b))).await;}}let _=tokio::time::timeout(Duration::from_secs(1),sink.send(Message::Close(Some(CloseFrame{code,reason:"".into()})))).await;break;}},
                packet=rx.recv()=>{let Some(packet)=packet else{break;};if !tokio::time::timeout(Duration::from_secs(10),sink.send(Message::Binary(packet.bytes))).await.is_ok_and(|r|r.is_ok()){break;}}
            }
        }
    });
    let mut heartbeat = tokio::time::Instant::now() + Duration::from_millis(75000);
    let mut writer_done = false;
    loop {
        let expiry = Duration::from_millis((expires as u64).saturating_sub(state::now()));
        let deadline = heartbeat.min(tokio::time::Instant::now() + expiry);
        let frame = tokio::select! {biased;_=app.0.shutdown.cancelled()=>{let _=close.send(Some(1001));break;},_=&mut writer=>{writer_done=true;break;},frame=tokio::time::timeout_at(deadline,source.next())=>frame};
        let bytes = match frame {
            Err(_) => {
                let _ = close.send(Some(if state::now() >= expires as u64 {
                    4001
                } else {
                    4008
                }));
                break;
            }
            Ok(None) => break,
            Ok(Some(Err(e))) => {
                let code = if matches!(
                    e.source()
                        .and_then(|e| e.downcast_ref::<tungstenite::Error>()),
                    Some(tungstenite::Error::Capacity(_))
                ) {
                    1009
                } else {
                    1002
                };
                let _ = close.send(Some(code));
                break;
            }
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(Message::Binary(b)))) => b,
            Ok(Some(Ok(_))) => {
                let _ = close.send(Some(1003));
                break;
            }
        };
        let mut core = app.0.core.lock().await;
        let active = core
            .streams
            .get(&key)
            .and_then(|s| s.connection.as_ref())
            .is_some_and(|c| c.id == connection_id);
        if !active {
            break;
        }
        if let Err(e) = core.rate(format!("control:{connection_id}"), 600., 20.) {
            let _ = core.send(&key, &ServerMsg::Error(e.0));
            let _ = close.send(Some(1008));
            break;
        }
        let msg = match decode(&bytes) {
            Ok(m) => m,
            Err(e) => {
                if e.0.code == ErrorCode::InvalidRequest {
                    if let Some(nonce) = invalid_command_nonce(&bytes) {
                        let cursor = core.cursor(&key, u);
                        let _ = core.send(
                            &key,
                            &ServerMsg::Response(Box::new(CommandResponse {
                                nonce,
                                cursor,
                                replayed: false,
                                outcome: CommandOutcome::Err(e.0),
                            })),
                        );
                        continue;
                    }
                }
                let code = if e.0.code == ErrorCode::PayloadTooLarge {
                    1009
                } else {
                    1002
                };
                let _ = core.send(&key, &ServerMsg::Error(e.0));
                let _ = close.send(Some(code));
                break;
            }
        };
        match msg {
            ClientMsg::Heartbeat(h) => {
                let alive = async {
                    let mut c = app.0.pool.acquire().await?;
                    sqlx::query_scalar::<_, bool>(
                        "SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=? AND expires_at>?)",
                    )
                    .bind(&key)
                    .bind(state::now() as i64)
                    .fetch_one(&mut *c)
                    .await
                }
                .await;
                if !alive.unwrap_or(false) {
                    let _ = close.send(Some(4001));
                    break;
                }

                if h.nonce == 0 || h.nonce > i64::MAX as u64 || h.last_seq > core.streams[&key].seq
                {
                    let _ = core.send(
                        &key,
                        &ServerMsg::Error(Error::new(ErrorCode::InvalidSequence).0),
                    );
                    let _ = close.send(Some(1002));
                    break;
                }
                heartbeat = tokio::time::Instant::now() + Duration::from_millis(75000);
                let _ = core.send(
                    &key,
                    &ServerMsg::HeartbeatAck(HeartbeatAck {
                        nonce: h.nonce,
                        server_time: state::now(),
                    }),
                );
            }
            ClientMsg::Command(req) => {
                match commands::execute(&app, &mut core, &key, u, req).await {
                    Ok(response) => {
                        let _ = core.send(&key, &ServerMsg::Response(Box::new(response)));
                    }
                    Err(e) => {
                        let _ = core.send(&key, &ServerMsg::Error(e.0));
                        let _ = close.send(Some(4001));
                        break;
                    }
                }
            }
            ClientMsg::Voice(v) => {
                let _ = crate::voice::signal(&app, &mut core, &key, u, &connection_id, v).await;
            }
            ClientMsg::Focus { focused } => {
                if let Some(c) = core
                    .streams
                    .get_mut(&key)
                    .and_then(|s| s.connection.as_mut())
                {
                    c.focused = focused;
                }
            }
            ClientMsg::Identify(_) => {
                let _ = close.send(Some(1002));
                break;
            }
        }
    }
    if close.borrow().is_none() {
        let _ = close.send(Some(1000));
    }
    // Revoke media before waiting on a potentially blocked WS close writer.
    disconnect(&app, &key, &connection_id, u).await;
    if !writer_done {
        let _ = writer.await;
    }
}
fn invalid_command_nonce(bytes: &[u8]) -> Option<u64> {
    #[derive(serde::Deserialize)]
    struct LooseCommand {
        nonce: u64,
        command: serde_json::Value,
    }
    #[derive(serde::Deserialize)]
    struct LooseClient {
        #[serde(rename = "type")]
        kind: String,
        data: LooseCommand,
    }
    let m: LooseClient = rmp_serde::from_slice(bytes).ok()?;
    let name = m.data.command.get("type")?.as_str()?;
    if m.kind != "command" || m.data.nonce == 0 || m.data.nonce > i64::MAX as u64 {
        return None;
    }
    let known = matches!(
        name,
        "update_profile"
            | "set_presence"
            | "send_friend_request"
            | "accept_friend_request"
            | "decline_friend_request"
            | "cancel_friend_request"
            | "remove_friend"
            | "block_user"
            | "unblock_user"
            | "open_dm"
            | "create_group_dm"
            | "add_group_recipient"
            | "remove_group_recipient"
            | "leave_group_dm"
            | "transfer_group_ownership"
            | "close_dm"
            | "create_guild"
            | "update_guild"
            | "delete_guild"
            | "join_guild"
            | "leave_guild"
            | "transfer_guild_ownership"
            | "set_member_role"
            | "kick_member"
            | "ban_member"
            | "unban_member"
            | "create_invite"
            | "revoke_invite"
            | "set_voice_permission"
            | "create_channel"
            | "update_channel"
            | "reorder_channels"
            | "delete_channel"
            | "send_message"
            | "edit_message"
            | "delete_message"
            | "add_reaction"
            | "remove_reaction"
            | "typing"
            | "mark_read"
            | "subscribe_guild"
    );
    known.then_some(m.data.nonce)
}
async fn disconnect(app: &App, key: &[u8], connection_id: &str, u: Id) {
    {
        let mut core = app.0.core.lock().await;
        if let Ok(mut c) = app.0.pool.acquire().await {
            let _ =
                crate::voice::disconnect(app, &mut core, &mut c, key, Some(connection_id)).await;
        }
        if let Some(s) = core.streams.get_mut(key) {
            if s.connection.as_ref().is_none_or(|c| c.id == connection_id) {
                s.connection = None;
                s.touched = state::now();
            } else {
                return;
            }
        } else {
            return;
        }
    }
    let app = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let mut core = app.0.core.lock().await;
        if core.streams.values().any(|s| {
            s.user == u
                && (s.connection.is_some()
                    || (s.ever_connected && state::now().saturating_sub(s.touched) < 5000))
        }) {
            return;
        }
        if let Ok(mut c) = app.0.pool.acquire().await {
            let _ = commands::broadcast_presence(&mut c, &mut core, u).await;
        }
    });
}
// Validate the raw value before serde: depth, complete consumption, collection bounds,
// forbidden extensions and unchecked allocation lengths. No recursive value tree is built.
fn decode(bytes: &[u8]) -> Result<ClientMsg> {
    require(
        bytes.len() <= MAX_WS_FRAME_BYTES as usize,
        ErrorCode::PayloadTooLarge,
    )?;
    require(
        bytes
            .first()
            .is_some_and(|b| (0x80..=0x8f).contains(b) || *b == 0xde || *b == 0xdf),
        ErrorCode::InvalidRequest,
    )?;
    let mut pos = 0;
    scan(bytes, &mut pos, 0)?;
    require(pos == bytes.len(), ErrorCode::InvalidRequest)?;
    // Serde also accepts positional structs. V1 only permits named maps.
    let raw: serde_json::Value =
        rmp_serde::from_slice(bytes).map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    named_shapes(&raw)?;
    rmp_serde::from_slice(bytes).map_err(|_| Error::new(ErrorCode::InvalidRequest))
}
fn named_shapes(v: &serde_json::Value) -> Result<()> {
    use serde_json::Value;
    fn object(v: &Value) -> Result<&serde_json::Map<String, Value>> {
        v.as_object()
            .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))
    }
    let root = object(v)?;
    let kind = root.get("type").and_then(Value::as_str).unwrap_or_default();
    let data = object(root.get("data").unwrap_or(&Value::Null))?;
    if kind == "identify" {
        if let Some(resume) = data.get("resume").filter(|v| !v.is_null()) {
            object(resume)?;
        }
    } else if kind == "command" {
        let command = object(data.get("command").unwrap_or(&Value::Null))?;
        let name = command
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let fields = object(command.get("data").unwrap_or(&Value::Null))?;
        let patch = match name {
            "update_profile" => Some(fields),
            "update_guild" | "update_channel" => {
                Some(object(fields.get("patch").unwrap_or(&Value::Null))?)
            }
            _ => None,
        };
        if let Some(patch) = patch {
            for (key, value) in patch {
                if matches!(
                    key.as_str(),
                    "username"
                        | "display_name"
                        | "avatar_id"
                        | "status_text"
                        | "name"
                        | "icon_id"
                        | "topic"
                ) {
                    object(value)?;
                }
            }
        }
        if name == "reorder_channels" {
            if let Some(placements) = fields.get("placements").and_then(Value::as_array) {
                for p in placements {
                    object(p)?;
                }
            }
        }
    }
    Ok(())
}
pub(crate) fn scan(b: &[u8], p: &mut usize, depth: u8) -> Result<()> {
    require(depth <= 32, ErrorCode::PayloadTooLarge)?;
    let tag = *b
        .get(*p)
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    *p += 1;
    let (len, children) = match tag {
        0x00..=0x7f | 0xe0..=0xff | 0xc0 | 0xc2 | 0xc3 => (0, 0),
        0x80..=0x8f => (0, usize::from(tag & 15) * 2),
        0x90..=0x9f => (0, usize::from(tag & 15)),
        0xa0..=0xbf => (usize::from(tag & 31), 0),
        0xcc | 0xd0 => (1, 0),
        0xcd | 0xd1 => (2, 0),
        0xca | 0xce | 0xd2 => (4, 0),
        0xcb | 0xcf | 0xd3 => (8, 0),
        0xc4 | 0xd9 => (length(b, p, 1)?, 0),
        0xc5 | 0xda => (length(b, p, 2)?, 0),
        0xc6 | 0xdb => (length(b, p, 4)?, 0),
        0xdc => (0, length(b, p, 2)?),
        0xdd => (0, length(b, p, 4)?),
        0xde => (0, length(b, p, 2)?.saturating_mul(2)),
        0xdf => (0, length(b, p, 4)?.saturating_mul(2)),
        _ => return Err(Error::new(ErrorCode::InvalidRequest)),
    };
    require(
        children <= MAX_WS_FRAME_BYTES as usize && len <= b.len().saturating_sub(*p),
        ErrorCode::PayloadTooLarge,
    )?;
    *p += len;
    for _ in 0..children {
        scan(b, p, depth + 1)?;
    }
    Ok(())
}
fn length(b: &[u8], p: &mut usize, n: usize) -> Result<usize> {
    require(n <= b.len().saturating_sub(*p), ErrorCode::InvalidRequest)?;
    let mut len = 0usize;
    for x in &b[*p..*p + n] {
        len = (len << 8) | usize::from(*x);
    }
    *p += n;
    Ok(len)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positional_nested_structs_and_hostile_lengths_are_rejected() {
        let positional = serde_json::json!({"type":"heartbeat","data":[1,0]});
        let bytes = rmp_serde::to_vec_named(&positional).unwrap();
        assert!(decode(&bytes).is_err());
        let positional = serde_json::json!({"type":"command","data":{"nonce":1,"command":{"type":"mark_read","data":[1,2]}}});
        assert!(decode(&rmp_serde::to_vec_named(&positional).unwrap()).is_err());
        for value in [
            serde_json::json!({"type":"identify","data":{"version":1,"token":"x","resume":["0",0]}}),
            serde_json::json!({"type":"command","data":{"nonce":1,"command":{"type":"update_profile","data":{"display_name":["set","x"]}}}}),
            serde_json::json!({"type":"command","data":{"nonce":1,"command":{"type":"reorder_channels","data":{"guild_id":1,"placements":[[2,null,0]]}}}}),
        ] {
            assert!(decode(&rmp_serde::to_vec_named(&value).unwrap()).is_err());
        }
        assert!(decode(&[0x81, 0xa1, b'x', 0xdd, 0xff, 0xff, 0xff, 0xff]).is_err());
        let mut deep = vec![0x81, 0xa1, b'x'];
        deep.extend(vec![0x91; 34]);
        deep.push(0xc0);
        assert_eq!(
            decode(&deep).unwrap_err().0.code,
            ErrorCode::PayloadTooLarge
        );
        // No parser panic on arbitrary byte sequences (release aborts on panic).
        use rand::RngCore;
        for n in 0..2048 {
            let mut b = vec![0; n % 513];
            rand::rngs::OsRng.fill_bytes(&mut b);
            let _ = decode(&b);
        }
    }
    #[test]
    fn framing() {
        let m = ClientMsg::Heartbeat(Heartbeat {
            nonce: 1,
            last_seq: 0,
        });
        let mut b = rmp_serde::to_vec_named(&m).unwrap();
        assert!(decode(&b).is_ok());
        b.push(0);
        assert!(decode(&b).is_err());
        assert!(decode(&[0x81, 0xa1, b'x', 0xd4, 0, 0]).is_err());
    }
}
