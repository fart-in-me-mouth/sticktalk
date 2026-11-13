//! Additive text features controller; avoids changing unrelated settings/call behavior.
use crate::{
    app::{App, Dialog, GuildTab, Msg, Screen},
    backend::{Config, Http, Request},
};
use iced::{widget::image::Handle, Subscription, Task};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use sticktalk_proto::*;
#[derive(Default)]
pub struct State {
    pub emoji_name: String,
    pub emoji_edits: BTreeMap<Id, String>,
    pub uploading: bool,
    pub assets: BTreeMap<Id, Asset>,
    pub hover: Option<Id>,
    account: Option<(String, Id)>,
    requested: Option<(Id, u64)>,
}
#[derive(Default)]
pub struct Asset {
    pub frames: Option<Arc<crate::animated::Frames>>,
    pub full: bool,
    pub pending: bool,
    pub failed: bool,
    pub index: usize,
    pub active: bool,
}
#[derive(Debug, Clone)]
pub enum Event {
    Name(String),
    Rename(Id, String),
    Save(Id),
    Delete(Id),
    Pick,
    Picked(Option<std::path::PathBuf>),
    Uploaded(u64, Result<Snapshot<Vec<GuildEmoji>>, String>),
    Loaded(u64, Id, bool, Result<Arc<crate::animated::Frames>, String>),
    Hover(Option<Id>),
    Advance(Id),
}
pub fn msg(e: Event) -> Msg {
    Msg::TextFeature(Box::new(e))
}
pub fn entries(app: &App) -> &[GuildEmoji] {
    app.state
        .guild
        .and_then(|g| app.state.model.launch.emoji.get(&g))
        .map_or(&[], Vec::as_slice)
}
pub fn candidates(app: &App) -> Vec<&GuildEmoji> {
    if app.suggestions_dismissed || !matches!(app.state.screen, Screen::Channel(_)) {
        return Vec::new();
    }
    sticktalk_client_core::features3::candidates(
        &app.editor.text(),
        app.composer_caret(),
        entries(app),
    )
}
pub fn update(app: &mut App, e: Event) -> Task<Msg> {
    match e {
        Event::Name(s) => app.text_features.emoji_name = s.chars().take(32).collect(),
        Event::Rename(id, s) => {
            app.text_features
                .emoji_edits
                .insert(id, s.chars().take(32).collect());
        }
        Event::Save(id) => {
            if let Some(guild_id) = app.state.guild {
                let name = app
                    .text_features
                    .emoji_edits
                    .remove(&id)
                    .unwrap_or_default();
                return app.update(Msg::Command(Command::Launch(LaunchCommand::RenameEmoji {
                    guild_id,
                    emoji_id: id,
                    name,
                })));
            }
        }
        Event::Delete(id) => {
            if let Some(guild_id) = app.state.guild {
                return app.update(Msg::Confirm(Command::Launch(LaunchCommand::DeleteEmoji {
                    guild_id,
                    emoji_id: id,
                })));
            }
        }
        Event::Pick => {
            if app.text_features.uploading || !app.permissions().contains(Permissions::MANAGE_EMOJI)
            {
                return Task::none();
            }
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .add_filter("Emoji", &["png", "gif", "webp"])
                        .pick_file()
                        .await
                        .map(|f| f.path().to_owned())
                },
                |p| msg(Event::Picked(p)),
            );
        }
        Event::Picked(Some(path)) => {
            let (Some(config), Some(guild)) = (
                app.state.session.as_ref().map(|s| s.config.clone()),
                app.state.guild,
            ) else {
                return Task::none();
            };
            let name = if app.text_features.emoji_name.is_empty() {
                path.file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("emoji")
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .take(32)
                    .collect()
            } else {
                app.text_features.emoji_name.clone()
            };
            if !valid_emoji_name(&name) {
                app.error = "Emoji names need 2–32 letters, numbers, or underscores.".into();
                return Task::none();
            }
            let generation = app.state.generation;
            app.text_features.uploading = true;
            return Task::perform(
                async move { upload(config, guild, name, path).await },
                move |v| msg(Event::Uploaded(generation, v)),
            );
        }
        Event::Picked(None) => {}
        Event::Uploaded(generation, result) => {
            app.text_features.uploading = false;
            if generation == app.state.generation {
                match result {
                    Ok(v) => {
                        if let Some(g) = app.state.guild {
                            app.state
                                .model
                                .launch
                                .snapshot(sticktalk_client_core::launch::SnapshotData::Emoji(g, v));
                            app.text_features.emoji_name.clear();
                        }
                    }
                    Err(e) => app.error = e,
                }
            }
        }
        Event::Hover(id) => app.text_features.hover = id,
        Event::Loaded(generation, id, full, result) => {
            if generation == app.state.generation {
                if let Some(a) = app.text_features.assets.get_mut(&id) {
                    a.pending = false;
                    match result {
                        Ok(f) => {
                            a.frames = Some(f);
                            a.full = full;
                        }
                        Err(_) => a.failed = true,
                    }
                }
            }
        }
        Event::Advance(id) => {
            if let Some(a) = app.text_features.assets.get_mut(&id).filter(|a| a.active) {
                if let Some(f) = &a.frames {
                    a.index = (a.index + 1) % f.images.len();
                }
            }
        }
    }
    Task::none()
}
async fn upload(
    c: Config,
    g: Id,
    name: String,
    path: std::path::PathBuf,
) -> Result<Snapshot<Vec<GuildEmoji>>, String> {
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|_| "Could not read emoji")?;
    if meta.len() > MAX_EMOJI_BYTES as u64 {
        return Err("Emoji must be 256 KB or smaller.".into());
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| "Could not read emoji")?;
    let http = Http::new(&c.server, &c.token)?;
    let r = http
        .client
        .post(format!("{}/api/v1/guilds/{}/emoji", http.base, g.0))
        .bearer_auth(&c.token)
        .query(&[("name", name)])
        .body(bytes)
        .send()
        .await
        .map_err(|_| "Emoji upload failed")?;
    let success = r.status().is_success();
    let bytes = crate::backend::limited(r, 65536).await?;
    if !success {
        let e = serde_json::from_slice::<ApiError>(&bytes).ok();
        return Err(e.map_or("Emoji upload failed".into(), |e| e.message));
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid emoji response".into())
}
pub fn reconcile(app: &mut App) -> Task<Msg> {
    let account = app
        .state
        .session
        .as_ref()
        .zip(app.me())
        .map(|(s, u)| (s.config.server.clone(), u));
    if app.text_features.account != account {
        app.text_features = State {
            account,
            ..Default::default()
        };
    }
    let Some(c) = app.state.session.as_ref().map(|s| s.config.clone()) else {
        return Task::none();
    };
    if let Some(g) = app.state.guild {
        let key = (g, app.state.generation);
        if app.text_features.requested != Some(key) {
            app.text_features.requested = Some(key);
            app.request(Request::Launch {
                read: sticktalk_client_core::launch::Read::Emoji(g),
                generation: app.state.generation,
            });
        }
    }
    let mut wanted = BTreeSet::new();
    if matches!(app.dialog, Dialog::GuildSettings(GuildTab::Emoji))
        || app.features.picker == crate::features::Picker::Emoji
    {
        wanted.extend(entries(app).iter().map(|e| e.id));
    }
    for (_, m, _, _) in app.visible_items() {
        if let Some(m) = m {
            for p in sticktalk_client_core::features3::parts(&m.content) {
                if let sticktalk_client_core::features3::Part::Emoji(id, _) = p {
                    wanted.insert(id);
                }
            }
            wanted.extend(m.reactions.iter().filter_map(|r| custom_emoji_id(&r.emoji)));
        }
    }
    wanted.extend(candidates(app).iter().map(|e| e.id));
    let known: BTreeMap<_, _> = entries(app).iter().map(|e| (e.id, e.animated)).collect();
    wanted.retain(|id| known.contains_key(id));
    app.text_features.assets.retain(|id, _| wanted.contains(id));
    let mut tasks = Vec::new();
    let pending = app
        .text_features
        .assets
        .values()
        .filter(|a| a.pending)
        .count();
    for id in wanted.into_iter().take(50) {
        let active = app.focused && app.text_features.hover == Some(id) && known[&id];
        let a = app.text_features.assets.entry(id).or_default();
        a.active = active;
        if !active && a.full && !a.pending {
            if let Some(f) = a.frames.as_ref() {
                let mut f = (**f).clone();
                f.images.truncate(1);
                f.bytes = f.images.len() * 128 * 128 * 4;
                a.frames = Some(Arc::new(f));
            }
            a.index = 0;
            a.full = false;
        }
        if !a.pending
            && !a.failed
            && (a.frames.is_none() || (active && !a.full))
            && tasks.len() + pending < 2
            && !c.mock
        {
            a.pending = true;
            let config = c.clone();
            let generation = app.state.generation;
            tasks.push(Task::perform(load(config, id, active), move |r| {
                msg(Event::Loaded(generation, id, active, r))
            }));
        }
    }
    Task::batch(tasks)
}
async fn load(c: Config, id: Id, full: bool) -> Result<Arc<crate::animated::Frames>, String> {
    let http = Http::new(&c.server, &c.token)?;
    let response = http
        .get(&format!(
            "/emoji/{}{}",
            id.0,
            if full { "" } else { "/preview" }
        ))
        .send()
        .await
        .map_err(|_| "Emoji unavailable")?;
    if !response.status().is_success() {
        return Err("Emoji unavailable".into());
    }
    let bytes = crate::backend::limited(response, MAX_EMOJI_BYTES).await?;
    tokio::task::spawn_blocking(move || {
        if full && bytes.starts_with(b"GIF8") {
            return crate::animated::decode(&bytes);
        }
        let image = image::load_from_memory(&bytes)
            .map_err(|_| "Invalid emoji")?
            .into_rgba8();
        let (w, h) = (image.width(), image.height());
        if w > 128 || h > 128 {
            return Err("Emoji too large".into());
        }
        Ok(Arc::new(crate::animated::Frames {
            bytes: image.as_raw().len(),
            images: vec![(Handle::from_rgba(w, h, image.into_raw()), 100)],
        }))
    })
    .await
    .map_err(|_| "Emoji decoder stopped")?
}
pub fn subscription(app: &App) -> Subscription<Msg> {
    Subscription::batch(app.text_features.assets.iter().filter_map(|(&id, a)| {
        let f = a.frames.as_ref()?;
        if !a.active || f.images.len() < 2 {
            return None;
        }
        let delay = f.images[a.index % f.images.len()].1;
        Some(Subscription::run_with((id, a.index, delay), |v| {
            let (id, _, delay) = *v;
            iced::futures::stream::once(async move {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                msg(Event::Advance(id))
            })
        }))
    }))
}
pub fn fixture(app: &mut App, screen: &str) {
    if !screen.starts_with("custom-emoji") && screen != "guild-settings-emoji" {
        return;
    }
    let g = Id(10);
    app.state.guild = Some(g);
    let names = [
        "stick_wave",
        "party_stick",
        "cozy",
        "ship_it",
        "great_work",
        "thinking_stick",
    ];
    let list: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, n)| GuildEmoji {
            id: Id(800 + i as u64),
            guild_id: g,
            name: (*n).into(),
            animated: i == 1,
        })
        .collect();
    for (e, glyph) in list.iter().zip(["👋", "🎉", "☕", "🚀", "💚", "🤔"]) {
        // Real bundled artwork is used only by the offline fixture, never by live assets.
        if let Some(h) = crate::ui::emoji_picker::artwork(glyph) {
            app.text_features.assets.insert(
                e.id,
                Asset {
                    frames: Some(Arc::new(crate::animated::Frames {
                        images: vec![(h, 100)],
                        bytes: 0,
                    })),
                    ..Default::default()
                },
            );
        }
    }
    app.state.model.launch.emoji.insert(g, list.clone());
    match screen {
        "guild-settings-emoji" => {
            let _ = app.update(Msg::Open(Dialog::GuildSettings(GuildTab::Emoji)));
        }
        "custom-emoji-picker" => app.features.picker = crate::features::Picker::Emoji,
        "custom-emoji-completion" => {
            app.editor = iced::widget::text_editor::Content::with_text(":stick");
        }
        "custom-emoji-message" => {
            if let Some(m) = app
                .state
                .model
                .messages
                .get_mut(&Id(11))
                .and_then(|v| v.last_entry())
                .map(|e| e.into_mut())
            {
                m.content = format!("{} {}", list[0].token(), list[1].token());
                m.reactions.push(ReactionSummary {
                    emoji: list[3].token(),
                    count: 4,
                    me: true,
                });
            }
        }
        _ => {}
    }
}
