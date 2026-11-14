//! Reproducible, separate-process loopback load; never weakens production limits.
//! cargo build --release -p sticktalk-server --bins --example loadtest
//! cargo run --release -p sticktalk-server --example loadtest -- --server <exe>
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteConnectOptions, Connection};
use std::{
    path::PathBuf,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use sticktalk_proto::*;
use sticktalk_server::{App, Config};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as Frame},
};
type AnyError = Box<dyn std::error::Error + Send + Sync>;
#[derive(Parser)]
struct Options {
    #[arg(long)]
    server: PathBuf,
    #[arg(long, default_value_t = 60)]
    seconds: u64,
    #[arg(long, default_value = "crates/server/loadtest-results.json")]
    output: PathBuf,
}
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(Default)]
struct Samples {
    sent: u64,
    responses: u64,
    latencies_us: Vec<u64>,
}

#[tokio::main(worker_threads = 4)]
async fn main() -> Result<(), AnyError> {
    let options = Options::parse();
    let dir = tempfile::tempdir()?;
    // Only fixture initialization uses the library in this process. The measured
    // server is a separate release binary, so client memory/CPU is excluded.
    let app = App::open(Config {
        operations: Default::default(),
        browser: Default::default(),
        bind: "127.0.0.1:0".parse()?,
        data_dir: dir.path().into(),
        public_url: "http://localhost".into(),
        max_upload_mb: 25,
        max_total_upload_mb: 10240,
        worker_id: 0,
        trusted_proxy: "127.0.0.1".parse()?,
        google_client_id: None,
        mock_google_jwks: None,
        mock_google_issuer: None,
        gifs: Default::default(),
        voice: sticktalk_server::voice::VoiceOptions {
            voice_bind_ip: "127.0.0.1".parse()?,
            voice_advertised_ip: Some("127.0.0.1".parse()?),
            ..Default::default()
        },
    })
    .await
    .map_err(|_| "fixture initialization failed")?;
    drop(app);
    let mut c = sqlx::SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(dir.path().join("sticktalk.sqlite3"))
            .foreign_keys(true),
    )
    .await?;
    let mut tx = c.begin().await?;
    let mut tokens = Vec::new();
    for i in 0..300_i64 {
        let mut bytes = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let token = URL_SAFE_NO_PAD.encode(bytes);
        let user = 10000 + i;
        sqlx::query("INSERT INTO users(id,username,password_hash,desired_presence,created_at) VALUES(?,?,?,'online',0)").bind(user).bind(format!("load{i:03}")).bind("load-fixture-only-no-password-login").execute(&mut *tx).await?;
        sqlx::query("INSERT INTO sessions VALUES(?,?,0,?,0)")
            .bind(Sha256::digest(token.as_bytes()).to_vec())
            .bind(user)
            .bind(i64::MAX)
            .execute(&mut *tx)
            .await?;
        tokens.push(token);
    }
    for g in 0..30_i64 {
        let owner = 10000 + g * 10;
        sqlx::query("INSERT INTO guilds VALUES(?,?,?,NULL,0)")
            .bind(20000 + g)
            .bind(owner)
            .bind(format!("load guild {g}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(?,'text',?,'general',0,0)").bind(30000+g).bind(20000+g).execute(&mut *tx).await?;
        for n in 0..10_i64 {
            let u = owner + n;
            sqlx::query("INSERT INTO guild_members VALUES(?,?,?,0)")
                .bind(20000 + g)
                .bind(u)
                .bind(if n == 0 { "owner" } else { "member" })
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO read_states(user_id,channel_id) VALUES(?,?)")
                .bind(u)
                .bind(30000 + g)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    c.close().await?;
    let probe = std::net::TcpListener::bind("127.0.0.1:0")?;
    let bind = probe.local_addr()?;
    drop(probe);
    let mut server = Server(
        ProcessCommand::new(&options.server)
            .args([
                "--voice-bind-ip",
                "127.0.0.1",
                "--voice-advertised-ip",
                "127.0.0.1",
                "--bind",
                &bind.to_string(),
                "--data-dir",
            ])
            .arg(dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let http = reqwest::Client::new();
    for _ in 0..100 {
        if server.0.try_wait()?.is_some() {
            return Err("server exited during startup".into());
        }
        if http
            .get(format!("http://{bind}/api/v1/health"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let origin = Arc::new(Instant::now());
    let (ready_tx, mut ready_rx) = mpsc::channel(300);
    let (start_tx, start_rx) = watch::channel(None::<Instant>);
    let mut clients = Vec::new();
    for (i, token) in tokens.into_iter().enumerate() {
        let origin = origin.clone();
        let ready = ready_tx.clone();
        let mut start = start_rx.clone();
        let seconds = options.seconds;
        clients.push(tokio::spawn(async move {
            let mut req=format!("ws://{bind}/api/v1/ws").into_client_request()?;
            req.headers_mut().insert("sec-websocket-protocol","sticktalk.v1".parse()?);
            let (mut ws,_)=connect_async(req).await?;
            let hello=ws.next().await.ok_or("missing Hello")??;
            if !matches!(hello,Frame::Binary(ref b) if matches!(rmp_serde::from_slice::<ServerMsg>(b),Ok(ServerMsg::Hello(_)))) {return Err::<Samples,AnyError>("bad Hello".into());}
            ws.send(Frame::Binary(rmp_serde::to_vec_named(&ClientMsg::Identify(Identify { text_features:false, launch_features:true,version:1,token,resume:None}))?.into())).await?;
            let mut seq=0;
            loop {
                let frame=ws.next().await.ok_or("closed before Ready")??;
                if let Frame::Binary(b)=frame {
                    match rmp_serde::from_slice::<ServerMsg>(&b)? {
                        ServerMsg::Ready(r)=>{seq=seq.max(r.cursor.seq);break;},
                        ServerMsg::Event(e)=>seq=seq.max(e.seq),
                        _=>return Err("unexpected setup message".into()),
                    }
                }
            }
            ready.send(()).await?; start.changed().await?;
            let begin=(*start.borrow()).ok_or("missing start")?;
            let end=begin+Duration::from_secs(seconds);
            let mut send_tick=tokio::time::interval_at((begin+Duration::from_millis(i as u64*10000/300)).into(),Duration::from_secs(10));
            send_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut heartbeats=tokio::time::interval_at((begin+Duration::from_secs(30)).into(),Duration::from_secs(30));
            let mut samples=Samples::default();
            loop {
                tokio::select! {
                    _=tokio::time::sleep_until((end+Duration::from_secs(2)).into())=>break,
                    _=send_tick.tick(), if Instant::now()<end=> {
                        if Instant::now()>=end {continue;}
                        let content=format!("lt:{i}:{}:{}:{}",samples.sent,origin.elapsed().as_nanos(),"chat text ".repeat(20));
                        let command=ClientMsg::Command(CommandRequest { nonce:samples.sent+1,command:Command::SendMessage(MessageDraft {channel_id:Id(30000+i as u64/10),content,reply_to:None,attachment_ids:vec![],mention_ids:vec![]}) });
                        ws.send(Frame::Binary(rmp_serde::to_vec_named(&command)?.into())).await?; samples.sent+=1;
                    },
                    _=heartbeats.tick()=>{ws.send(Frame::Binary(rmp_serde::to_vec_named(&ClientMsg::Heartbeat(Heartbeat {nonce:999,last_seq:seq}))?.into())).await?;},
                    frame=ws.next()=>{
                        let frame=frame.ok_or("unexpected close")??;
                        let Frame::Binary(b)=frame else {return Err("nonbinary/close during load".into());};
                        match rmp_serde::from_slice::<ServerMsg>(&b)? {
                            ServerMsg::Response(r)=>{if !matches!(r.outcome,CommandOutcome::Ok(_)) {return Err(format!("command rejected: {:?}",r.outcome).into());} samples.responses+=1;},
                            ServerMsg::Event(e)=>{
                                if e.seq<=seq {return Err("non-increasing sequence".into());} seq=e.seq;
                                if let Event::MessageCreated {message,..}=e.event {
                                    let mut fields=message.content.splitn(5,':');
                                    if fields.next()!=Some("lt") {return Err("foreign fixture message".into());}
                                    let sender=fields.next().ok_or("missing sender")?.parse::<usize>()?;
                                    let _message_number=fields.next();
                                    let sent=fields.next().ok_or("missing timestamp")?.parse::<u128>()?;
                                    if sender!=i {samples.latencies_us.push((origin.elapsed().as_nanos().saturating_sub(sent)/1000) as u64);}
                                }
                            },
                            ServerMsg::HeartbeatAck(_)=>{},
                            other=>return Err(format!("unexpected server message: {other:?}").into()),
                        }
                    }
                }
            }
            let _=ws.close(None).await;
            Ok::<Samples,AnyError>(samples)
        }));
    }
    drop(ready_tx);
    tokio::time::timeout(Duration::from_secs(60), async {
        for _ in 0..300 {
            ready_rx.recv().await.ok_or("client setup failed")?;
        }
        Ok::<_, AnyError>(())
    })
    .await??;
    let (cpu0, rss0) = process_metrics(&server.0)?;
    let begin = Instant::now();
    start_tx.send(Some(begin))?;
    let mut rss = vec![rss0];
    for _ in 0..options.seconds {
        tokio::time::sleep(Duration::from_secs(1)).await;
        rss.push(process_metrics(&server.0)?.1);
    }
    let duration = begin.elapsed().as_secs_f64();
    let (cpu1, _) = process_metrics(&server.0)?;
    let mut sent = 0;
    let mut responses = 0;
    let mut latencies = Vec::new();
    for task in clients {
        let s = task.await??;
        sent += s.sent;
        responses += s.responses;
        latencies.extend(s.latencies_us);
    }
    latencies.sort_unstable();
    let percentile = |p: f64| {
        latencies
            .get(((latencies.len() as f64 * p).ceil() as usize).saturating_sub(1))
            .copied()
            .unwrap_or(0) as f64
            / 1000.
    };
    let result = serde_json::json!({
        "platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"server_profile":"release","clients":300,"guilds":30,"members_per_guild":10,
        "requested_seconds":options.seconds,"measured_seconds":duration,"messages_sent":sent,"successful_responses":responses,"nonself_deliveries":latencies.len(),"expected_nonself_deliveries":sent*9,
        "messages_per_second":sent as f64/options.seconds as f64,"fanout_p50_ms":percentile(0.50),"fanout_p99_ms":percentile(0.99),"fanout_max_ms":percentile(1.0),
        "server_rss_min_mib":*rss.iter().min().unwrap() as f64/1048576.,"server_rss_max_mib":*rss.iter().max().unwrap() as f64/1048576.,"server_rss_mean_mib":rss.iter().sum::<u64>() as f64/rss.len() as f64/1048576.,
        "server_cpu_seconds":cpu1-cpu0,"server_cpu_percent_one_core":100.*(cpu1-cpu0)/duration,"logical_cpus":std::thread::available_parallelism()?.get(),"rss_samples":rss.len(),
        "notes":"Separate server process; Windows RSS is working set. Loopback clients and unrelated concurrent work share this host. Send-to-nonself-event latency includes scheduling, transport, commit and fan-out. Empty initial history; no uploads. Two-second drain is excluded from CPU sampling."
    });
    let json = serde_json::to_string_pretty(&result)?;
    std::fs::write(options.output, json.as_bytes())?;
    println!("{json}");
    if responses != sent || latencies.len() as u64 != sent * 9 {
        return Err("lost/rejected messages or fan-out deliveries".into());
    }
    Ok(())
}

#[cfg(windows)]
fn process_metrics(child: &Child) -> Result<(f64, u64), AnyError> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle};
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        lo: u32,
        hi: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Memory {
        cb: u32,
        faults: u32,
        peak_working_set: usize,
        working_set: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetProcessTimes(
            h: *mut c_void,
            created: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(h: *mut c_void, memory: *mut Memory, size: u32) -> i32;
    }
    let (mut created, mut exit, mut kernel, mut user) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    let mut memory = Memory {
        cb: std::mem::size_of::<Memory>() as u32,
        ..Default::default()
    };
    // SAFETY: Child owns a live process handle; repr(C) buffers match Win32 and
    // remain valid throughout both synchronous calls. No handle is closed here.
    let ok = unsafe {
        GetProcessTimes(
            child.as_raw_handle(),
            &mut created,
            &mut exit,
            &mut kernel,
            &mut user,
        ) != 0
            && GetProcessMemoryInfo(child.as_raw_handle(), &mut memory, memory.cb) != 0
    };
    if !ok {
        return Err(std::io::Error::last_os_error().into());
    }
    let ticks = |t: FileTime| u64::from(t.lo) + (u64::from(t.hi) << 32);
    Ok((
        (ticks(kernel) + ticks(user)) as f64 / 10000000.,
        memory.working_set as u64,
    ))
}
#[cfg(not(windows))]
fn process_metrics(_child: &Child) -> Result<(f64, u64), AnyError> {
    Err("This audit harness measures Win32 working set/CPU; add /proc metrics before running on Linux".into())
}
