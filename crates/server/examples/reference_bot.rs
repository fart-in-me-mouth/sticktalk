//! StickTalk reference bot: a complete, minimal bot over the WebSocket.
//!
//! It registers `/hello` (with an autocompleting `name` option) and answers
//! it with a message carrying buttons; "Wave" answers privately, "Feedback"
//! opens a form, and the form's submission is thanked privately.
//!
//! ```text
//! STICKTALK_URL=http://127.0.0.1:8080 STICKTALK_BOT_TOKEN=stkb_... \
//!   cargo run -p sticktalk-server --example reference_bot
//! ```
//! See `docs/BOTS.md` for the protocol this follows.
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use sticktalk_proto::*;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as Frame};

const NAMES: &[&str] = &["Ada", "Alan", "Grace", "Linus", "Margaret", "Barbara", "Ken"];

#[allow(dead_code)]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let url = std::env::var("STICKTALK_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    let token = std::env::var("STICKTALK_BOT_TOKEN").expect("set STICKTALK_BOT_TOKEN");
    if let Err(e) = run(&url, &token).await {
        eprintln!("reference bot stopped: {e}");
        std::process::exit(1);
    }
}

/// The slash command the bot offers.
pub fn commands() -> Vec<CommandDraft> {
    vec![CommandDraft {
        kind: CommandKind::ChatInput,
        name: "hello".into(),
        description: "Say hello, with buttons and a form".into(),
        options: vec![CommandOption {
            kind: OptionKind::String,
            name: "name".into(),
            description: "Who to greet".into(),
            required: false,
            choices: vec![],
            options: vec![],
            min_value: None,
            max_value: None,
            min_length: Some(1),
            max_length: Some(32),
            autocomplete: true,
        }],
        default_member_permissions: None,
    }]
}

fn button(style: ButtonStyle, label: &str, custom_id: Option<&str>, url: Option<&str>) -> Component {
    Component::Button(Button {
        style,
        label: Some(label.into()),
        emoji: None,
        custom_id: custom_id.map(Into::into),
        url: url.map(Into::into),
        disabled: false,
    })
}

fn feedback_form() -> Modal {
    Modal {
        custom_id: "feedback".into(),
        title: "Tell us more".into(),
        inputs: vec![TextInput {
            custom_id: "comment".into(),
            label: "Your feedback".into(),
            style: TextInputStyle::Paragraph,
            placeholder: Some("What should this bot do next?".into()),
            value: None,
            required: true,
            min_length: 2,
            max_length: Some(500),
        }],
    }
}

fn string_option<'a>(options: &'a [InvokedOption], name: &str) -> Option<&'a str> {
    options.iter().find(|o| o.name == name).and_then(|o| match &o.value {
        Some(OptionValue::String(s)) => Some(s.as_str()),
        _ => None,
    })
}

/// What the bot answers to one interaction.
pub fn answer(i: &Interaction) -> Option<InteractionResponse> {
    let private = |content: String| {
        InteractionResponse::Message(AppMessageDraft {
            content,
            ephemeral: true,
            ..Default::default()
        })
    };
    Some(match &i.data {
        InteractionData::Command(c) if c.name == "hello" => {
            let name = string_option(&c.options, "name").unwrap_or("there");
            InteractionResponse::Message(AppMessageDraft {
                content: format!("Hello, {name}! 👋"),
                embeds: vec![EmbedDraft {
                    title: Some("StickTalk reference bot".into()),
                    description: Some("Press a button below.".into()),
                    color: Some(0x5865F2),
                    ..Default::default()
                }],
                components: vec![ActionRow {
                    components: vec![
                        button(ButtonStyle::Primary, "Wave", Some("wave"), None),
                        button(ButtonStyle::Secondary, "Feedback", Some("feedback"), None),
                        button(ButtonStyle::Link, "Docs", None, Some("https://github.com/")),
                    ],
                }],
                // Greet without pinging anyone.
                allowed_mentions: Some(AllowedMentions::default()),
                ..Default::default()
            })
        }
        InteractionData::Autocomplete(c) => {
            let typed = c
                .options
                .iter()
                .find(|o| o.focused)
                .and_then(|o| match &o.value {
                    Some(OptionValue::String(s)) => Some(s.to_lowercase()),
                    _ => None,
                })
                .unwrap_or_default();
            InteractionResponse::Autocomplete {
                choices: NAMES
                    .iter()
                    .filter(|n| n.to_lowercase().starts_with(&typed))
                    .map(|n| Choice {
                        name: (*n).into(),
                        value: OptionValue::String((*n).into()),
                    })
                    .collect(),
            }
        }
        InteractionData::Component(c) if c.custom_id == "wave" => {
            private(format!("{} waves back. 👋", i.user.display_name.clone().unwrap_or_else(|| i.user.username.clone())))
        }
        InteractionData::Component(c) if c.custom_id == "feedback" => {
            InteractionResponse::Modal(feedback_form())
        }
        InteractionData::ModalSubmit(m) if m.custom_id == "feedback" => {
            let text = m
                .values
                .iter()
                .find(|v| v.custom_id == "comment")
                .map(|v| v.value.as_str())
                .unwrap_or_default();
            private(format!("Thanks for the feedback: “{text}”"))
        }
        _ => return None,
    })
}

fn encode(m: &ClientMsg) -> Frame {
    Frame::Binary(rmp_serde::to_vec_named(m).expect("encodable").into())
}

/// Connects, registers the command and answers interactions until the
/// connection closes.
pub async fn run(url: &str, token: &str) -> Result<(), String> {
    let ws_url = format!("{}/api/v1/ws", url.trim_end_matches('/').replacen("http", "ws", 1));
    let mut request = ws_url.into_client_request().map_err(|e| e.to_string())?;
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "sticktalk.v1".parse().expect("header"));
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| e.to_string())?;
    let (mut tx, mut rx) = ws.split();
    // 1. Hello, then Identify with the bot token.
    tx.send(encode(&ClientMsg::Identify(Identify {
        version: PROTOCOL_VERSION,
        token: token.into(),
        resume: None,
        launch_features: true,
        text_features: false,
    })))
    .await
    .map_err(|e| e.to_string())?;
    let mut nonce: u64 = 1;
    let mut last_seq = 0;
    let mut heartbeat = tokio::time::interval(Duration::from_millis(u64::from(HEARTBEAT_INTERVAL_MS)));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                nonce += 1;
                tx.send(encode(&ClientMsg::Heartbeat(Heartbeat { nonce, last_seq }))).await.map_err(|e| e.to_string())?;
            }
            frame = rx.next() => {
                let Some(frame) = frame else { return Ok(()) };
                let bytes = match frame.map_err(|e| e.to_string())? {
                    Frame::Binary(b) => b,
                    Frame::Close(_) => return Ok(()),
                    _ => continue,
                };
                let Ok(msg) = rmp_serde::from_slice::<ServerMsg>(&bytes) else { continue };
                match msg {
                    // 2. Ready: register the commands (replaces earlier ones).
                    ServerMsg::Ready(_) => {
                        nonce += 1;
                        tx.send(encode(&ClientMsg::Command(CommandRequest {
                            nonce,
                            command: Command::Apps(AppCommand::SetCommands { guild_id: None, commands: commands() }),
                        }))).await.map_err(|e| e.to_string())?;
                    }
                    // 3. Interactions: answer within three seconds.
                    ServerMsg::Event(e) => {
                        last_seq = e.seq;
                        if let Event::Apps(AppEvent::Interaction(i)) = e.event {
                            if let Some(response) = answer(&i) {
                                nonce += 1;
                                tx.send(encode(&ClientMsg::Command(CommandRequest {
                                    nonce,
                                    command: Command::Apps(AppCommand::Respond {
                                        answer: InteractionAnswer { interaction_id: i.id, token: i.token.clone() },
                                        response,
                                    }),
                                }))).await.map_err(|e| e.to_string())?;
                            }
                        }
                    }
                    ServerMsg::Response(r) => {
                        if let CommandOutcome::Err(e) = r.outcome {
                            eprintln!("command {} failed: {} ({:?})", r.nonce, e.message, e.code);
                        }
                    }
                    ServerMsg::Error(e) => return Err(e.message),
                    _ => {}
                }
            }
        }
    }
}
