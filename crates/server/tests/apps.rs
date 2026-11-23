//! End to end: a real server on loopback, the reference bot from
//! `examples/reference_bot.rs` connected with its token, and a person using
//! its slash command, buttons and form over their own WebSocket.
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use sticktalk_proto::*;
use sticktalk_server::{App, Config};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as Frame},
    MaybeTlsStream, WebSocketStream,
};

#[path = "../examples/reference_bot.rs"]
#[allow(dead_code)]
mod reference_bot;

struct Peer {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    nonce: u64,
    events: Vec<Event>,
}
impl Peer {
    async fn connect(url: &str, token: &str) -> Self {
        let mut req = format!("{}/api/v1/ws", url.replace("http:", "ws:"))
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
        let (mut ws, _) = connect_async(req).await.unwrap();
        ws.send(Frame::Binary(
            rmp_serde::to_vec_named(&ClientMsg::Identify(Identify {
                version: 1,
                token: token.into(),
                resume: None,
                launch_features: true,
                text_features: false,
            }))
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        let mut peer = Self {
            ws,
            nonce: 1,
            events: vec![],
        };
        loop {
            if let ServerMsg::Ready(_) = peer.recv().await {
                return peer;
            }
        }
    }
    async fn recv(&mut self) -> ServerMsg {
        let m = tokio::time::timeout(Duration::from_secs(10), self.ws.next())
            .await
            .expect("WS timeout")
            .expect("WS closed")
            .unwrap();
        let Frame::Binary(b) = m else {
            panic!("not binary: {m:?}")
        };
        let msg: ServerMsg = rmp_serde::from_slice(&b).unwrap();
        if let ServerMsg::Event(e) = &msg {
            self.events.push(e.event.clone());
        }
        msg
    }
    async fn command(&mut self, command: Command) -> CommandResult {
        self.nonce += 1;
        let nonce = self.nonce;
        self.ws
            .send(Frame::Binary(
                rmp_serde::to_vec_named(&ClientMsg::Command(CommandRequest { nonce, command }))
                    .unwrap()
                    .into(),
            ))
            .await
            .unwrap();
        loop {
            if let ServerMsg::Response(r) = self.recv().await {
                if r.nonce == nonce {
                    match r.outcome {
                        CommandOutcome::Ok(v) => return v,
                        CommandOutcome::Err(e) => panic!("command failed: {e:?}"),
                    }
                }
            }
        }
    }
    async fn event<T>(&mut self, pick: impl Fn(&Event) -> Option<T>) -> T {
        loop {
            if let Some(i) = self.events.iter().position(|e| pick(e).is_some()) {
                return pick(&self.events.remove(i)).unwrap();
            }
            self.recv().await;
        }
    }
}

#[tokio::test]
async fn the_reference_bot_answers_a_slash_command_buttons_and_a_form() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(Config {
        operations: sticktalk_server::OperationsConfig {
            min_free_disk_gb: 0,
            ..Default::default()
        },
        browser: Default::default(),
        bind: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().into(),
        public_url: "http://localhost".into(),
        max_upload_mb: 25,
        max_total_upload_mb: 10240,
        worker_id: 0,
        trusted_proxy: "127.0.0.1".parse().unwrap(),
        google_client_id: None,
        mock_google_jwks: None,
        mock_google_issuer: None,
        gifs: Default::default(),
        voice: sticktalk_server::voice::VoiceOptions {
            voice_bind_ip: "127.0.0.1".parse().unwrap(),
            voice_advertised_ip: Some("127.0.0.1".parse().unwrap()),
            ..Default::default()
        },
    })
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let serving = app.clone();
    let server = tokio::spawn(async move { serving.serve(listener).await });
    let http = reqwest::Client::new();
    let auth: AuthResponse = http
        .post(format!("{url}/api/v1/auth/register"))
        .json(&RegisterRequest {
            username: "owner".into(),
            password: "correct horse battery".into(),
            display_name: None,
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut me = Peer::connect(&url, &auth.token).await;
    let CommandResult::Guild(guild) = me
        .command(Command::CreateGuild {
            name: "Bots".into(),
            icon_id: None,
        })
        .await
    else {
        panic!()
    };
    let g = guild.summary.guild.id;
    let ch = guild
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::Text)
        .unwrap()
        .id;
    me.command(Command::Apps(AppCommand::Subscribe)).await;
    let CommandResult::Apps(AppResult::Token { app: info, token }) = me
        .command(Command::Apps(AppCommand::CreateApp(AppDraft {
            name: "Reference".into(),
            permissions: Permissions::VIEW_CHANNEL
                | Permissions::SEND_MESSAGES
                | Permissions::EMBED_LINKS,
            ..Default::default()
        })))
        .await
    else {
        panic!()
    };
    me.command(Command::Apps(AppCommand::Install(InstallRequest {
        app_id: info.id,
        guild_id: g,
        permissions: Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES | Permissions::EMBED_LINKS,
        message_content: false,
        confirm_administrator: false,
    })))
    .await;
    // The bot connects with its token and registers /hello.
    me.events.clear();
    let bot_url = url.clone();
    let bot = tokio::spawn(async move { reference_bot::run(&bot_url, &token).await });
    me.event(|e| match e {
        Event::Apps(AppEvent::CommandsUpdated { guild_id }) if *guild_id == g => Some(()),
        _ => None,
    })
    .await;
    let catalog: CommandCatalog = http
        .get(format!("{url}/api/v1/channels/{}/commands", ch.0))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let hello = catalog.apps[0].commands[0].clone();
    assert_eq!((hello.name.as_str(), catalog.apps[0].name.as_str()), ("hello", "Reference"));
    // Autocomplete.
    me.command(Command::Apps(AppCommand::Autocomplete {
        invocation: Invocation {
            channel_id: ch,
            command_id: hello.id,
            options: vec![InvokedOption {
                name: "name".into(),
                value: Some(OptionValue::String("Gr".into())),
                options: vec![],
                focused: true,
            }],
            target_id: None,
        },
        request: 1,
    }))
    .await;
    let choices = me
        .event(|e| match e {
            Event::Apps(AppEvent::Autocomplete { request: 1, choices }) => Some(choices.clone()),
            _ => None,
        })
        .await;
    assert_eq!(choices[0].name, "Grace");
    // The command, answered with a message carrying buttons.
    me.command(Command::Apps(AppCommand::Invoke(Invocation {
        channel_id: ch,
        command_id: hello.id,
        options: vec![InvokedOption {
            name: "name".into(),
            value: Some(OptionValue::String("Grace".into())),
            options: vec![],
            focused: false,
        }],
        target_id: None,
    })))
    .await;
    let reply = me
        .event(|e| match e {
            Event::MessageCreated { message, author } if author.style.bot => Some(message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(reply.content, "Hello, Grace! 👋");
    let extra = reply.extra.app.clone().unwrap();
    assert_eq!(extra.interaction.unwrap().name, "hello");
    assert_eq!(extra.components[0].components.len(), 3);
    // "Wave" answers privately.
    me.command(Command::Apps(AppCommand::Component {
        channel_id: ch,
        message_id: reply.id,
        custom_id: "wave".into(),
        values: vec![],
    }))
    .await;
    let waved = me
        .event(|e| match e {
            Event::Apps(AppEvent::Ephemeral(m)) => Some(m.content.clone()),
            _ => None,
        })
        .await;
    assert!(waved.contains("waves back"), "{waved}");
    // "Feedback" opens a form; its submission is thanked privately.
    me.command(Command::Apps(AppCommand::Component {
        channel_id: ch,
        message_id: reply.id,
        custom_id: "feedback".into(),
        values: vec![],
    }))
    .await;
    let (interaction, modal) = me
        .event(|e| match e {
            Event::Apps(AppEvent::Modal {
                interaction_id,
                modal,
                ..
            }) => Some((*interaction_id, modal.clone())),
            _ => None,
        })
        .await;
    assert_eq!(modal.inputs[0].custom_id, "comment");
    me.command(Command::Apps(AppCommand::SubmitModal {
        interaction_id: interaction,
        values: vec![ModalValue {
            custom_id: "comment".into(),
            value: "More weather".into(),
        }],
    }))
    .await;
    let thanks = me
        .event(|e| match e {
            Event::Apps(AppEvent::Ephemeral(m)) if m.content.contains("Thanks") => {
                Some(m.content.clone())
            }
            _ => None,
        })
        .await;
    assert!(thanks.contains("More weather"));
    bot.abort();
    app.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
}
