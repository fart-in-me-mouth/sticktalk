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
#[derive(Debug, Clone, Copy)]
pub enum FolderDialog {
    Add(Id),
    Edit(u32),
}
#[derive(Default)]
pub struct State {
    pub folder_dialog: Option<FolderDialog>,
    pub folder_name: String,
    pub folder_color: u32,
    folders_requested: Option<u64>,
    pub folders_loaded: bool,
    pub cooldowns: BTreeMap<Id, sticktalk_client_core::features3::Cooldown>,
    slow_requested: Option<(Id, u64)>,
    pub search: sticktalk_client_core::features3::SearchState,
    search_task: Option<iced::task::Handle>,
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
    task: Option<iced::task::Handle>,
    requested_full: bool,
    pub failed: bool,
    pub index: usize,
    pub active: bool,
}
#[derive(Debug, Clone)]
pub enum Event {
    FolderAdd(Id),
    FolderEdit(u32),
    FolderClose,
    FolderName(String),
    FolderColor(u32),
    FolderChoose(Id, u32),
    FolderCreate(Id),
    FolderSave(u32),
    FolderUngroup(u32),
    FolderRemove(Id),
    FolderToggle(u32),
    Slowmode(u32),
    SlowTick,
    SearchSubmit,
    SearchPage(bool),
    SearchClose,
    SearchJump(Id, Id),
    SearchLoaded(String, u64, Result<Snapshot<MessageSearchPage>, String>),
    Name(String),
    Rename(Id, String),
    Save(Id),
    Delete(Id),
    Pick,
    Picked(Option<std::path::PathBuf>),
    Uploaded(u64, Result<Snapshot<Vec<GuildEmoji>>, String>),
    Loaded(u64, Id, bool, Result<Arc<crate::animated::Frames>, String>),
    Hover(Option<Id>),
    Hidden(Id),
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
    if app.suggestions_dismissed
        || !matches!(app.dialog, Dialog::None)
        || !matches!(app.state.screen, Screen::Channel(_))
    {
        return Vec::new();
    }
    sticktalk_client_core::features3::candidates(
        &app.editor.text(),
        app.composer_caret(),
        entries(app),
    )
}
pub fn completion_count(app: &App) -> usize {
    let custom = candidates(app).len();
    (custom + app.emoji_candidates().len()).min(8)
}
pub fn update(app: &mut App, e: Event) -> Task<Msg> {
    match e {
        Event::FolderAdd(g) => {
            if !app.text_features.folders_loaded {
                app.text_features.folders_requested = None;
            }
            app.main.popover = None;
            app.text_features.folder_dialog = Some(FolderDialog::Add(g));
            app.text_features.folder_name = "Servers".into();
            app.text_features.folder_color = color_value(crate::theme::ACCENT);
            return iced::widget::operation::focus("folder-name");
        }
        Event::FolderEdit(id) => {
            if let Some(f) = app
                .state
                .model
                .launch
                .server_folders
                .iter()
                .find(|f| f.id == id)
            {
                app.text_features.folder_name = f.name.clone();
                app.text_features.folder_color = f.color;
                app.text_features.folder_dialog = Some(FolderDialog::Edit(id));
                app.main.popover = None;
                return iced::widget::operation::focus("folder-name");
            }
        }
        Event::FolderClose => app.text_features.folder_dialog = None,
        Event::FolderName(s) => app.text_features.folder_name = s.chars().take(32).collect(),
        Event::FolderColor(color) => app.text_features.folder_color = color,
        Event::FolderChoose(g, id) => {
            let mut f = app.state.model.launch.server_folders.clone();
            if sticktalk_client_core::features3::move_server(&mut f, g, Some(id)) {
                save_folders(app, f);
            }
        }
        Event::FolderCreate(g) => {
            if !valid_folder_name(&app.text_features.folder_name) {
                return Task::none();
            }
            let mut f = app.state.model.launch.server_folders.clone();
            if f.len() >= 10 {
                return Task::none();
            }
            let id = (1..=11)
                .find(|id| !f.iter().any(|f| f.id == *id))
                .unwrap_or(1);
            f.push(ServerFolder {
                id,
                name: app.text_features.folder_name.trim().into(),
                color: app.text_features.folder_color,
                collapsed: false,
                guild_ids: vec![],
            });
            sticktalk_client_core::features3::move_server(&mut f, g, Some(id));
            save_folders(app, f);
        }
        Event::FolderSave(id) => {
            let mut f = app.state.model.launch.server_folders.clone();
            if valid_folder_name(&app.text_features.folder_name) {
                if let Some(f) = f.iter_mut().find(|f| f.id == id) {
                    f.name = app.text_features.folder_name.trim().into();
                    f.color = app.text_features.folder_color;
                }
                save_folders(app, f);
            }
        }
        Event::FolderUngroup(id) => {
            let mut f = app.state.model.launch.server_folders.clone();
            f.retain(|f| f.id != id);
            save_folders(app, f);
        }
        Event::FolderRemove(g) => {
            let mut f = app.state.model.launch.server_folders.clone();
            sticktalk_client_core::features3::move_server(&mut f, g, None);
            save_folders(app, f);
            app.main.popover = None;
        }
        Event::FolderToggle(id) => {
            let mut f = app.state.model.launch.server_folders.clone();
            if let Some(f) = f.iter_mut().find(|f| f.id == id) {
                f.collapsed = !f.collapsed;
            }
            save_folders(app, f);
        }
        Event::SlowTick => {}
        Event::Slowmode(seconds) => {
            if let Dialog::ChannelSettings(channel_id) = app.dialog {
                if SLOWMODE_SECONDS.contains(&seconds)
                    && app
                        .channel_permissions(channel_id)
                        .contains(Permissions::MANAGE_CHANNELS)
                {
                    return app.update(Msg::Command(Command::Launch(LaunchCommand::SetSlowmode {
                        channel_id,
                        seconds,
                    })));
                }
            }
        }
        Event::SearchSubmit => {
            app.text_features.search.cursors.clear();
            return search(app, None);
        }
        Event::SearchPage(newer) => {
            let s = &mut app.text_features.search;
            let before = if newer {
                if s.cursors.len() < 2 {
                    return Task::none();
                }
                s.cursors.pop();
                s.cursors.last().copied().flatten()
            } else {
                s.page.as_ref().and_then(|p| p.next_before)
            };
            if !newer && before.is_none() {
                return Task::none();
            }
            app.main.search = s.query.clone();
            return search(app, before);
        }
        Event::SearchClose => {
            app.text_features.search.close();
            app.text_features.search_task = None;
            app.main.search.clear();
        }
        Event::SearchJump(channel, message) => {
            if !app.state.model.channels.contains_key(&channel) {
                return Task::none();
            }
            let navigate = if app.state.screen != Screen::Channel(channel) {
                app.update(Msg::Select(channel))
            } else {
                Task::none()
            };
            let jump = app.update(Msg::JumpTo(message));
            app.main.search = app.text_features.search.query.clone();
            return Task::batch([navigate, jump]);
        }
        Event::SearchLoaded(stream, serial, result) => {
            if app
                .state
                .model
                .cursor
                .as_ref()
                .is_some_and(|c| c.stream_id == stream)
                && serial == app.text_features.search.serial
                && app.text_features.search.guild == app.state.guild
            {
                app.text_features.search_task = None;
                let result = result.and_then(|v| {
                    if app.state.model.snapshot_valid(&v.cursor) {
                        Ok(v.data)
                    } else {
                        Err("Search changed. Try again.".into())
                    }
                });
                app.text_features.search.apply(serial, result);
            }
        }
        Event::Name(s) => app.text_features.emoji_name = s.chars().take(32).collect(),
        Event::Rename(id, s) => {
            let edits = &mut app.text_features.emoji_edits;
            if edits.len() >= MAX_GUILD_EMOJI && !edits.contains_key(&id) {
                edits.pop_first();
            }
            edits.insert(id, s.chars().take(32).collect());
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
        Event::Hidden(id) => {
            if app.text_features.hover == Some(id) {
                app.text_features.hover = None;
            }
        }
        Event::Loaded(generation, id, full, result) => {
            if generation == app.state.generation {
                if let Some(a) = app.text_features.assets.get_mut(&id) {
                    a.pending = false;
                    a.task = None;
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
        return Err(e.map_or("Emoji upload failed".into(), |e| match e.code {
            ErrorCode::PayloadTooLarge => "Emoji must be 256 KB or smaller after resizing.".into(),
            ErrorCode::UnsupportedMediaType => "Choose a valid PNG, GIF, or WebP image.".into(),
            ErrorCode::LimitExceeded => {
                "This emoji exceeds the server's image or emoji limits.".into()
            }
            ErrorCode::Forbidden => "You need Manage Emoji permission to upload emoji.".into(),
            _ => e.message,
        }));
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid emoji response".into())
}
pub fn reconcile(app: &mut App) -> Task<Msg> {
    // The offscreen harness has no transport. Keep its explicitly seeded assets/results
    // and never turn fixture geometry measurements into mock network requests.
    if app.mock_mode && app.tx.is_none() {
        return Task::none();
    }
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
    if let Some(p) = &mut app.text_features.search.page {
        p.items
            .retain(|m| app.state.model.channels.contains_key(&m.channel_id));
    }
    if app.text_features.search.guild.is_some() && app.text_features.search.guild != app.state.guild
    {
        app.text_features.search.close();
        app.text_features.search_task = None;
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
    if app.text_features.folders_requested != Some(app.state.generation) {
        app.text_features.folders_requested = Some(app.state.generation);
        app.request(Request::Launch {
            read: sticktalk_client_core::launch::Read::Folders,
            generation: app.state.generation,
        });
    }
    let current = match app.dialog {
        Dialog::ChannelSettings(id) => Some(id),
        _ if timeline_visible(app) => {
            if let Screen::Channel(id) = app.state.screen {
                Some(id)
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(id) = current {
        let key = (id, app.state.generation);
        if app.text_features.slow_requested != Some(key) {
            app.text_features.slow_requested = Some(key);
            app.request(Request::Launch {
                read: sticktalk_client_core::launch::Read::Slowmode(id),
                generation: app.state.generation,
            });
        }
    }
    app.text_features
        .cooldowns
        .retain(|id, _| app.state.model.channels.contains_key(id));
    let mut wanted = BTreeSet::new();
    if matches!(app.dialog, Dialog::GuildSettings(GuildTab::Emoji))
        || app.features.picker == crate::features::Picker::Emoji
    {
        wanted.extend(entries(app).iter().map(|e| e.id));
    }
    let (_, _, top, _) = app.visible_range();
    let mut row_y = crate::theme::VIRTUAL_HEADER + top;
    for (_, m, height, _) in app.visible_items() {
        let visible = row_y + height >= app.scroll_y && row_y <= app.scroll_y + app.viewport_h;
        row_y += height;
        if !visible || !timeline_visible(app) {
            continue;
        }
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
        if !active && a.pending && a.requested_full {
            a.task = None;
            a.pending = false;
        }
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
            let (task, handle) = Task::perform(load(config, id, active), move |r| {
                msg(Event::Loaded(generation, id, active, r))
            })
            .abortable();
            a.requested_full = active;
            a.task = Some(handle.abort_on_drop());
            tasks.push(task);
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
    static DECODERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let permit = DECODERS
        .acquire()
        .await
        .map_err(|_| "Emoji decoder stopped")?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        decode_asset(&bytes, full)
    })
    .await
    .map_err(|_| "Emoji decoder stopped")?
}
fn decode_asset(bytes: &[u8], full: bool) -> Result<Arc<crate::animated::Frames>, String> {
    use image::{AnimationDecoder, ImageDecoder};
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(128);
    limits.max_image_height = Some(128);
    limits.max_alloc = Some(8 * 1024 * 1024);
    let mut images = Vec::new();
    let mut retained = 0;
    if full && bytes.starts_with(b"GIF8") {
        let mut decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))
            .map_err(|_| "Invalid emoji")?;
        decoder
            .set_limits(limits)
            .map_err(|_| "Emoji exceeds decoder budget")?;
        for frame in decoder.into_frames() {
            if images.len() >= 120 {
                return Err("Emoji has too many frames".into());
            }
            let frame = frame.map_err(|_| "Invalid emoji frame")?;
            let (n, d) = frame.delay().numer_denom_ms();
            let delay = (u64::from(n) / u64::from(d.max(1))).clamp(20, 10000);
            let image = frame.into_buffer();
            retained += image.as_raw().len();
            if retained > 8 * 1024 * 1024 {
                return Err("Emoji exceeds decoder budget".into());
            }
            images.push((
                Handle::from_rgba(image.width(), image.height(), image.into_raw()),
                delay,
            ));
        }
    } else {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "Invalid emoji")?;
        reader.limits(limits);
        let image = reader.decode().map_err(|_| "Invalid emoji")?.into_rgba8();
        retained = image.as_raw().len();
        images.push((
            Handle::from_rgba(image.width(), image.height(), image.into_raw()),
            100,
        ));
    }
    if images.is_empty() {
        return Err("Emoji is empty".into());
    }
    Ok(Arc::new(crate::animated::Frames {
        images,
        bytes: retained,
    }))
}

pub fn subscription(app: &App) -> Subscription<Msg> {
    let mut subscriptions: Vec<_> = app
        .text_features
        .assets
        .iter()
        .filter_map(|(&id, a)| {
            let f = a.frames.as_ref()?;
            if !app.focused || !a.active || f.images.len() < 2 {
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
        })
        .collect();
    if app.focused && timeline_visible(app) {
        if let Screen::Channel(id) = app.state.screen {
            let remaining = remaining(app, id);
            if remaining > 0
                && app
                    .channel_permissions(id)
                    .contains(Permissions::SEND_MESSAGES)
            {
                subscriptions.push(Subscription::run_with(
                    (
                        id,
                        app.text_features.cooldowns[&id].deadline(),
                        remaining.div_ceil(1000),
                    ),
                    |v| {
                        let (_, deadline, _) = *v;
                        iced::futures::stream::once(async move {
                            let remaining = deadline.saturating_sub(time());
                            let delay = remaining.saturating_sub(1) % 1000 + 1;
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            msg(Event::SlowTick)
                        })
                    },
                ));
            }
        }
    }
    Subscription::batch(subscriptions)
}
pub fn fixture(app: &mut App, screen: &str) {
    if screen == "notifications-disabled" {
        app.dialog = Dialog::UserSettings;
        app.user_tab = crate::settings_controller::UserPage::Notifications.index();
        app.settings.preferences.desktop_notifications = false;
        return;
    }
    if screen.starts_with("server-folder") {
        app.text_features.folders_loaded = true;
        let guilds: Vec<_> = app.state.model.guilds.keys().copied().take(2).collect();
        app.state.model.launch.server_folders = vec![ServerFolder {
            id: 1,
            name: "Friends".into(),
            color: color_value(crate::theme::ACCENT),
            collapsed: screen == "server-folders",
            guild_ids: guilds,
        }];
        if screen == "server-folder-edit" {
            app.text_features.folder_dialog = Some(FolderDialog::Edit(1));
            app.text_features.folder_name = "Friends".into();
            app.text_features.folder_color = color_value(crate::theme::ACCENT);
        }
        if screen == "server-folder-add" {
            app.text_features.folder_dialog = Some(FolderDialog::Add(Id(10)));
            app.text_features.folder_name = "Work".into();
            app.text_features.folder_color = color_value(crate::theme::WARNING);
        }
        return;
    }
    if screen == "channel-slowmode" || screen == "slowmode-countdown" {
        let status = SlowmodeStatus {
            channel_id: Id(12),
            seconds: 30,
            bypassed: false,
            retry_after_ms: 17000,
            server_time: crate::mock::now(),
        };
        let mut clock = sticktalk_client_core::features3::Cooldown::default();
        clock.receive(&status, time());
        app.state.model.launch.slowmode.insert(Id(12), status);
        app.text_features.cooldowns.insert(Id(12), clock);
        if screen == "channel-slowmode" {
            app.dialog = Dialog::ChannelSettings(Id(12));
            app.settings.channel = Some(Id(12));
            app.settings.channel_name = "general".into();
            app.settings.channel_topic = "Project updates and discussion.".into();
        }
        return;
    }
    if screen == "server-search" || screen == "server-search-empty" {
        let mut items: Vec<_> = app
            .state
            .model
            .messages
            .get(&Id(12))
            .into_iter()
            .flat_map(|m| m.values())
            .rev()
            .take(3)
            .cloned()
            .collect();
        for m in &mut items {
            m.content =
                "The launch sketches are ready. Please review the updated layout before Friday."
                    .into();
        }
        if screen.ends_with("empty") {
            items.clear();
        }
        let s = &mut app.text_features.search;
        s.open = true;
        s.query = "launch in:general".into();
        s.guild = Some(Id(10));
        s.cursors = vec![None];
        s.page = Some(MessageSearchPage {
            items,
            users: app.state.model.users.values().cloned().collect(),
            next_before: Some(Id(900)),
            has_more: screen != "server-search-empty",
            terms: vec!["launch".into()],
        });
        app.main.search = s.query.clone();
        return;
    }
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
            app.dialog = Dialog::GuildSettings(GuildTab::Emoji);
        }
        "custom-emoji-picker" => app.features.picker = crate::features::Picker::Emoji,
        "custom-emoji-completion" => {
            app.editor = iced::widget::text_editor::Content::new();
            app.editor.perform(iced::widget::text_editor::Action::Edit(
                iced::widget::text_editor::Edit::Paste(Arc::new(":stick".into())),
            ));
        }
        "custom-emoji-message" | "custom-emoji-inline" => {
            if let Some(m) = app
                .state
                .model
                .messages
                .get_mut(&Id(12))
                .and_then(|v| v.last_entry())
                .map(|e| e.into_mut())
            {
                m.content = if screen == "custom-emoji-inline" {
                    format!(
                        "Nice work {} — this is ready to ship. {}",
                        list[3].token(),
                        list[4].token()
                    )
                } else {
                    format!("{} {}", list[0].token(), list[1].token())
                };
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
fn search(app: &mut App, before: Option<Id>) -> Task<Msg> {
    let (Some(guild), Some(config)) = (
        app.state.guild,
        app.state.session.as_ref().map(|s| s.config.clone()),
    ) else {
        app.error = "Select a server to search messages.".into();
        return Task::none();
    };
    let query = app.main.search.trim().to_owned();
    if query.is_empty() {
        return Task::none();
    }
    app.text_features.search_task = None;
    let serial = app.text_features.search.start(guild, query.clone(), before);
    let stream = app
        .state
        .model
        .cursor
        .as_ref()
        .map_or(String::new(), |c| c.stream_id.clone());
    if config.mock {
        let mut items: Vec<_> = app
            .state
            .model
            .messages
            .values()
            .flat_map(|m| m.values())
            .filter(|m| {
                app.state
                    .model
                    .channels
                    .get(&m.channel_id)
                    .is_some_and(|c| c.guild_id == Some(guild))
            })
            .filter(|m| m.content.to_lowercase().contains(&query.to_lowercase()))
            .cloned()
            .collect();
        items.sort_by_key(|m| std::cmp::Reverse(m.id));
        items.truncate(25);
        let page = MessageSearchPage {
            items,
            users: app.state.model.users.values().cloned().collect(),
            next_before: None,
            has_more: false,
            terms: vec![query],
        };
        app.text_features.search.apply(serial, Ok(page));
        return Task::none();
    }
    let (task, handle) = Task::perform(
        async move {
            Http::new(&config.server, &config.token)?
                .core()
                .search(
                    guild,
                    &MessageSearchQuery {
                        q: query,
                        before,
                        limit: Some(25),
                    },
                )
                .await
        },
        move |v| msg(Event::SearchLoaded(stream.clone(), serial, v)),
    )
    .abortable();
    app.text_features.search_task = Some(handle.abort_on_drop());
    task
}
/// Milliseconds from a process-local monotonic origin; never the device wall clock.
pub fn time() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
pub fn remaining(app: &App, id: Id) -> u64 {
    app.text_features
        .cooldowns
        .get(&id)
        .map_or(0, |c| c.remaining(time()))
}
pub fn snapshot(app: &mut App, data: &sticktalk_client_core::launch::SnapshotData) {
    if matches!(
        data,
        sticktalk_client_core::launch::SnapshotData::Folders(_)
    ) {
        app.text_features.folders_loaded = true;
    }
    if matches!(data, sticktalk_client_core::launch::SnapshotData::Emoji(..)) {
        app.state.model.layout_revision = app.state.model.layout_revision.wrapping_add(1);
    }
    if let sticktalk_client_core::launch::SnapshotData::Slowmode(s) = data {
        app.text_features
            .cooldowns
            .entry(s.data.channel_id)
            .or_default()
            .receive(&s.data, time());
    }
}
pub fn live_event(app: &mut App, event: &sticktalk_proto::Event) {
    match event {
        sticktalk_proto::Event::ServerFoldersUpdated(_) => app.text_features.folders_loaded = true,
        sticktalk_proto::Event::MessageCreated { message, .. }
            if Some(message.author_id) == app.me() =>
        {
            if let Some(c) = app.text_features.cooldowns.get_mut(&message.channel_id) {
                c.observe_send(message.created_at, time());
            }
        }
        sticktalk_proto::Event::SlowmodeUpdated {
            channel_id,
            seconds,
        } => {
            if let Some(c) = app.text_features.cooldowns.get_mut(channel_id) {
                c.seconds = *seconds;
            }
            app.text_features.slow_requested = None;
        }
        sticktalk_proto::Event::RolesUpdated { guild_id, .. }
            if Some(*guild_id) == app.state.guild =>
        {
            app.text_features.slow_requested = None
        }
        sticktalk_proto::Event::ChannelUpsert(c)
            if app.state.screen == Screen::Channel(c.summary.id)
                || matches!(app.dialog,Dialog::ChannelSettings(id) if id==c.summary.id) =>
        {
            app.text_features.slow_requested = None
        }
        sticktalk_proto::Event::MemberUpsert(m)
            if Some(m.user.id) == app.me() && Some(m.guild_id) == app.state.guild =>
        {
            app.text_features.slow_requested = None
        }
        _ => {}
    }
}
pub fn rejected(app: &mut App, id: Id, retry_ms: u32) {
    if let Some(c) = app.text_features.cooldowns.get_mut(&id) {
        c.rejected(retry_ms, time());
    } else {
        app.text_features.slow_requested = None;
    }
}
fn save_folders(app: &mut App, folders: Vec<ServerFolder>) {
    if !app.text_features.folders_loaded {
        app.error = "Wait for your server folders to load.".into();
        return;
    }
    if app.send(Command::Launch(LaunchCommand::SetServerFolders {
        folders: folders.clone(),
    })) {
        app.state.model.launch.server_folders = folders;
        app.text_features.folder_dialog = None;
    }
}
pub fn color_value(c: iced::Color) -> u32 {
    ((c.r * 255.).round() as u32) << 16
        | ((c.g * 255.).round() as u32) << 8
        | (c.b * 255.).round() as u32
}
pub fn ready(app: &mut App) {
    app.text_features.requested = None;
    app.text_features.slow_requested = None;
    app.text_features.folders_requested = None;
    app.text_features.folders_loaded = false;
    app.text_features.cooldowns.clear();
    app.text_features.search.close();
    app.text_features.search_task = None;
}
pub fn command_failed(app: &mut App, command: Option<&Command>) {
    if matches!(
        command,
        Some(Command::Launch(LaunchCommand::SetServerFolders { .. }))
    ) {
        app.text_features.folders_requested = None;
    }
}
fn timeline_visible(app: &App) -> bool {
    !matches!(
        app.dialog,
        Dialog::UserSettings | Dialog::GuildSettings(_) | Dialog::ChannelSettings(_)
    ) && app.settings_return.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixtures_keep_real_feature_state_and_timers_stop_when_inactive() {
        let size = iced::Size::new(1280., 800.);
        let mut app = App::headless("server-search", size);
        assert_eq!(
            app.text_features.search.page.as_ref().unwrap().items.len(),
            3
        );
        let _ = reconcile(&mut app);
        assert!(app.text_features.search.open);
        let app = App::headless("custom-emoji-message", size);
        assert!(app.state.model.messages[&Id(12)]
            .values()
            .any(|m| m.content.contains("<:stick_wave:")));
        assert_eq!(app.text_features.assets.len(), 6);
        assert_eq!(subscription(&app).units(), 0);
        let mut app = App::headless("slowmode-countdown", size);
        app.focused = true;
        assert_eq!(subscription(&app).units(), 1);
        app.state.model.launch.effective_permissions.insert(
            Id(12),
            Permissions::VIEW_CHANNEL | Permissions::READ_HISTORY,
        );
        assert_eq!(subscription(&app).units(), 0);
        app.state.model.launch.effective_permissions.remove(&Id(12));
        app.dialog = Dialog::UserSettings;
        assert_eq!(subscription(&app).units(), 0);
        app.dialog = Dialog::None;
        app.focused = false;
        assert_eq!(subscription(&app).units(), 0);
        app.focused = true;
        app.text_features
            .cooldowns
            .get_mut(&Id(12))
            .unwrap()
            .bypassed = true;
        assert_eq!(subscription(&app).units(), 0);
        for id in 1..=1000 {
            let _ = update(&mut app, Event::Rename(Id(id), "edited_name".into()));
        }
        assert!(app.text_features.emoji_edits.len() <= MAX_GUILD_EMOJI);
        let mut app = App::headless("server-folder-edit", size);
        assert!(app.text_features.folder_dialog.is_some());
        let _ = app.update(Msg::Escape);
        assert!(app.text_features.folder_dialog.is_none());
    }
}

#[cfg(test)]
mod emoji_safety_tests {
    use super::*;
    #[test]
    fn unknown_tokens_are_plain_text_and_shortcodes_offer_keyboard_choices() {
        let size = iced::Size::new(1280., 800.);
        let mut app = App::headless("custom-emoji-completion", size);
        assert!(!candidates(&app).is_empty());
        for _ in 0..10 {
            let _ = app.update(Msg::Suggest(1));
            assert!(app.suggestion < completion_count(&app));
        }
        let m = app.state.model.messages[&Id(12)]
            .values()
            .next()
            .unwrap()
            .clone();
        app.state.model.launch.emoji.clear();
        let value = crate::markdown::Inline {
            text: "<:deleted_emoji:999>".into(),
            style: Default::default(),
            target: None,
            spoiler: None,
        };
        // Regression: routing an unknown token back through inline_content recursively overflowed.
        assert!(crate::ui::features3::inline(&app, &m, &[value], crate::theme::BODY).is_some());
    }
    #[test]
    fn folder_saves_wait_for_the_snapshot_and_search_results_are_stream_scoped() {
        let mut app = App::headless("server-folder-edit", iced::Size::new(1280., 800.));
        app.text_features.folders_loaded = false;
        save_folders(&mut app, vec![]);
        assert_eq!(app.state.model.launch.server_folders.len(), 1);
        assert!(app.error.starts_with("Wait for"));
        let serial = app
            .text_features
            .search
            .start(Id(10), "new account".into(), None);
        let _ = update(
            &mut app,
            Event::SearchLoaded("old-account-stream".into(), serial, Err("old error".into())),
        );
        assert!(app.text_features.search.loading);
        assert!(app.text_features.search.error.is_empty());
    }
    #[test]
    fn glyph_cache_survives_preview_load_and_eviction() {
        use iced_runtime::user_interface::{Cache, UserInterface};
        let mut app = App::headless("custom-emoji-message", iced::Size::new(1280., 800.));
        let cached = crate::message_layout::Cache::default();
        #[cfg(feature = "wgpu")]
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            crate::theme::regular(),
            iced::Pixels(crate::theme::BODY),
        ));
        #[cfg(not(feature = "wgpu"))]
        let mut renderer = iced_tiny_skia::Renderer::new(
            crate::theme::regular(),
            iced::Pixels(crate::theme::BODY),
        );
        for loaded in [true, false, true, false] {
            if loaded {
                app.text_features.assets.insert(
                    Id(999),
                    Asset {
                        frames: Some(Arc::new(crate::animated::Frames {
                            images: vec![(Handle::from_rgba(1, 1, vec![61, 220, 151, 255]), 100)],
                            bytes: 4,
                        })),
                        ..Default::default()
                    },
                );
            } else {
                app.text_features.assets.remove(&Id(999));
            }
            let glyph = crate::ui::features3::glyph(&app, Id(999), crate::theme::BODY);
            let mut ui = UserInterface::build(
                cached.wrap(Id(999), 1, glyph),
                iced::Size::new(100., 100.),
                Cache::new(),
                &mut renderer,
            );
            ui.draw(
                &mut renderer,
                &crate::theme::theme(&app),
                &iced::advanced::renderer::Style {
                    text_color: crate::theme::TEXT_PRIMARY,
                },
                iced::mouse::Cursor::Unavailable,
            );
        }
        assert_eq!(cached.hits.get(), 3);
    }
    #[test]
    fn emoji_decoder_keeps_every_accepted_frame_and_rejects_large_dimensions() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for i in 0..120 {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(128, 128, image::Rgba([i, 0, 0, 255])),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(40, 1),
                    ))
                    .unwrap();
            }
        }
        let frames = decode_asset(&bytes, true).unwrap();
        assert_eq!(frames.images.len(), 120);
        assert!(frames.bytes <= 8 * 1024 * 1024);
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1024, 1024)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        assert!(decode_asset(png.get_ref(), false).is_err());
    }
}
