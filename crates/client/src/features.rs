//! Additive coordinator for media surfaces. Main-window layout remains owned by ui.rs.
use crate::{
    app::{App, Dialog, Msg, Screen},
    backend::Config,
    ui::{emoji_picker, gif_picker, viewer},
    uploads,
};
use iced::{
    widget::{operation, text_editor},
    Subscription, Task,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use sticktalk_proto::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Picker {
    #[default]
    None,
    Emoji,
    Gif,
}
#[derive(Debug, Clone)]
pub enum Event {
    Video(crate::video::Event),
    Desktop(crate::desktop_host::Event),
    Update(crate::platform_glue::UpdateEvent),
    Trim(u64),
    ConfirmUpdate,
    StartMinimized(bool),
    ApplyUpdate,
    Restarted(Result<(), String>),
    Key(iced::keyboard::Key),
    Picker(Picker),
    EmojiSearch(String),
    EmojiCategory(usize),
    EmojiTone(Option<char>),
    EmojiHover(sticktalk_emoji::Emoji),
    EmojiScroll(f32),
    EmojiComplete(String),
    GifSearch(String),
    GifDue(u64),
    GifTab(bool),
    GifResult(u64, Result<crate::gifs::ResultData, String>),
    GifFavorite(String, bool),
    GifSend(GifItem),
    GifScroll(f32, f32),
    GifHover(Option<String>),
    GifOpen(String),
    InlineHover(Option<Id>),
    ViewerOpen(Attachment),
    ViewerLoaded(u64, Result<Arc<crate::media::Decoded>, String>),
    ViewerZoom(f32),
    ViewerTransform(f32, iced::Vector),
    ViewerReset,
    ViewerActual,
    ViewerNext(i32),
    ViewerClose,
    ViewerMore,
    ViewerCopy,
    ViewerSave,
    ViewerSaveTo(String, Option<std::path::PathBuf>),
    ViewerForwardMenu,
    ViewerForward(Id),
    ViewerForwarded(Id, Result<std::path::PathBuf, String>),
    UploadStart(Id, std::path::PathBuf, bool),
    UploadPrepared(Result<uploads::Job, String>),
    Upload(uploads::Event),
    UploadCancel(u64),
    UploadRetry(u64),
    Restored(String, Id, Vec<uploads::Job>, String),
    AnimationLoaded(String, Result<Arc<crate::animated::Frames>, String>),
    Advance(String),
    Autoplay(sticktalk_client_core::gifs::Autoplay),
    Notice(Result<(), String>),
    Activation(sticktalk_desktop_platform::activation::Activation),
}
pub struct Transfer {
    pub job: uploads::Job,
    pub offset: u64,
    pub speed: f64,
    pub error: Option<String>,
}
#[derive(Default)]
pub struct State {
    pub video: crate::video::State,
    pub trim_epoch: u64,
    pub trim_done: bool,
    pub picker: Picker,
    pub emoji: emoji_picker::State,
    pub gifs: gif_picker::State,
    pub viewer: viewer::State,
    pub uploads: BTreeMap<u64, Transfer>,
    pub animations: BTreeMap<String, crate::animated::Animation>,
    pub autoplay: sticktalk_client_core::gifs::Autoplay,
    pub inline_hover: Option<Id>,
    pub desktop: crate::platform_glue::State,
    pub loaded: Option<(String, Id)>,
    pub update_ready: Option<String>,
    pub crash_notice: bool,
    pub start_minimized: bool,
    pub confirm_update: bool,
    pub last_badge: u32,
    pub flash_pending: bool,
}
impl Msg {
    pub fn feature(e: Event) -> Self {
        Self::Feature(Box::new(e))
    }
}
fn config(app: &App) -> Option<Config> {
    app.state.session.as_ref().map(|s| s.config.clone())
}
fn emit(f: impl std::future::Future<Output = Event> + Send + 'static) -> Task<Msg> {
    Task::perform(f, Msg::feature)
}
pub fn subscription(app: &App) -> Subscription<Msg> {
    let mut subs = vec![
        crate::video::subscription(app),
        crate::desktop_host::subscription(app),
    ];
    if cfg!(feature = "compact-allocator") && !app.features.trim_done {
        subs.push(Subscription::run_with(app.features.trim_epoch, |epoch| {
            let epoch = *epoch;
            iced::futures::stream::once(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                Msg::feature(Event::Trim(epoch))
            })
        }));
    }
    if crate::platform_glue::has_instance() {
        subs.push(
            Subscription::run_with((), crate::platform_glue::activations)
                .map(|a| Msg::feature(Event::Activation(a))),
        );
    }
    if !app.mock_mode && option_env!("STICKTALK_UPDATE_PUBLIC_KEY").is_some() {
        subs.push(
            Subscription::run_with(crate::storage::update_origin(), |s| {
                crate::platform_glue::updates(s.as_str())
            })
            .map(|v| Msg::feature(Event::Update(v))),
        );
    }

    if let (Some(c), Some(account)) = (config(app), app.me()) {
        for (&id, t) in app
            .features
            .uploads
            .iter()
            .filter(|(_, t)| t.error.is_none() && t.job.attachment.is_none())
            .take(2)
        {
            if !c.mock {
                subs.push(
                    Subscription::run_with(
                        uploads::Worker {
                            config: c.clone(),
                            account,
                            id,
                            journal: serde_json::to_string(&t.job).unwrap_or_default(),
                        },
                        uploads::subscription,
                    )
                    .map(|e| Msg::feature(Event::Upload(e))),
                );
            }
        }
    }
    for (url, a) in &app.features.animations {
        if a.active && a.frames.as_ref().is_some_and(|f| f.images.len() > 1) {
            subs.push(Subscription::run_with(
                (url.clone(), a.index, a.delay()),
                |v| {
                    let (url, _, delay) = v.clone();
                    iced::futures::stream::once(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        Msg::feature(Event::Advance(url))
                    })
                },
            ));
        }
    }
    Subscription::batch(subs)
}
fn gif_request(app: &mut App, q: crate::gifs::Query) -> Task<Msg> {
    let (Some(c), GifCapability::Enabled(cap)) =
        (config(app), app.features.gifs.capability.clone())
    else {
        return Task::none();
    };
    let serial = app.features.gifs.serial;
    app.features.gifs.loading = true;
    if c.mock {
        app.features.gifs.loading = false;
        return Task::none();
    }
    emit(async move { Event::GifResult(serial, crate::gifs::request(c, cap, q).await) })
}
pub fn update(app: &mut App, e: Event) -> Task<Msg> {
    match e {
        Event::Video(e) => return crate::video::update(app, e),
        Event::Desktop(e) => return crate::desktop_host::update(app, e),
        Event::Trim(epoch) => {
            if epoch == app.features.trim_epoch {
                app.features.trim_done = true;
                sticktalk::resources::collect();
            }
        }
        Event::Update(crate::platform_glue::UpdateEvent::Ready(version)) => {
            app.features.update_ready = Some(version)
        }
        Event::Update(crate::platform_glue::UpdateEvent::Failed(_)) => {
            // Offline startup must not replace sign-in errors or interrupt typing.
            let root = crate::storage::root().join("Updates");
            if !app.mock_mode && std::fs::create_dir_all(&root).is_ok() {
                let _ = std::fs::write(
                    root.join("check-error.log"),
                    "Could not check for updates. Will retry at the next scheduled check.",
                );
            }
        }
        Event::StartMinimized(enabled) => {
            if !app.mock_mode {
                if let Err(error) = crate::storage::save_start_minimized(enabled) {
                    app.error = error;
                    return Task::none();
                }
            }
            app.features.start_minimized = enabled;
        }
        Event::ConfirmUpdate => app.features.confirm_update = !app.features.confirm_update,
        Event::ApplyUpdate => {
            if app.features.confirm_update {
                app.features.confirm_update = false;
                if !app.mock_mode {
                    let origin = crate::storage::update_origin();
                    return emit(async move {
                        Event::Restarted(crate::platform_glue::restart(origin).await)
                    });
                }
            }
        }
        Event::Restarted(result) => match result {
            Ok(()) => return iced::exit(),
            Err(e) => app.error = e,
        },
        Event::Key(key) => {
            use iced::keyboard::{key::Named, Key};
            if app.features.viewer.current().is_some() {
                if key == Key::Named(Named::Escape) && !matches!(app.dialog, Dialog::None) {
                    return app.update(Msg::Escape);
                }
                let event = match key {
                    Key::Named(Named::Escape) => Some(Event::ViewerClose),
                    Key::Named(Named::ArrowLeft) => Some(Event::ViewerNext(-1)),
                    Key::Named(Named::ArrowRight) => Some(Event::ViewerNext(1)),
                    Key::Character(ref c) if c == "+" || c == "=" => Some(Event::ViewerZoom(1.25)),
                    Key::Character(ref c) if c == "-" => Some(Event::ViewerZoom(0.8)),
                    Key::Character(ref c) if c == "0" => Some(Event::ViewerReset),
                    _ => None,
                };
                if let Some(event) = event {
                    return update(app, event);
                }
            } else if app.features.picker == Picker::Emoji {
                let entries = app.features.emoji.entries();
                let current = app
                    .features
                    .emoji
                    .hover
                    .and_then(|e| entries.iter().position(|v| *v == e))
                    .unwrap_or(0);
                let delta = match key {
                    Key::Named(Named::ArrowDown) => 8,
                    Key::Named(Named::ArrowUp) => -8,
                    Key::Named(Named::ArrowRight) => 1,
                    Key::Named(Named::ArrowLeft) => -1,
                    _ => 0,
                };
                if delta != 0 && !entries.is_empty() {
                    let index = (current as i32 + delta).rem_euclid(entries.len() as i32) as usize;
                    app.features.emoji.hover = Some(entries[index]);
                    let y = (index / 8) as f32 * (crate::theme::AVATAR + crate::theme::S4);
                    app.features.emoji.y = y;
                    return operation::scroll_to(
                        "emoji-grid",
                        iced::widget::scrollable::AbsoluteOffset { x: 0., y },
                    );
                }
                if key == Key::Named(Named::Enter) {
                    if let Some(e) = app
                        .features
                        .emoji
                        .hover
                        .or_else(|| entries.first().copied())
                    {
                        return app.update(Msg::Emoji(app.features.emoji.glyph(e)));
                    }
                }
            } else if app.features.picker == Picker::Gif {
                let s = &mut app.features.gifs;
                let len = s.items().len();
                match key {
                    Key::Named(Named::ArrowDown) => {
                        s.selected = (s.selected + 2).min(len.saturating_sub(1))
                    }
                    Key::Named(Named::ArrowUp) => s.selected = s.selected.saturating_sub(2),
                    Key::Named(Named::ArrowRight) => {
                        s.selected = (s.selected + 1).min(len.saturating_sub(1))
                    }
                    Key::Named(Named::ArrowLeft) => s.selected = s.selected.saturating_sub(1),
                    Key::Named(Named::Enter) => {
                        if let Some(item) = s.items().get(s.selected).cloned() {
                            return update(app, Event::GifSend(item));
                        }
                    }
                    _ => {}
                }
            }
        }
        Event::Picker(p) => {
            app.features.picker = if app.features.picker == p {
                Picker::None
            } else {
                p
            };
            app.dialog = Dialog::None;
            match app.features.picker {
                Picker::Emoji => return operation::focus("emoji-search"),
                Picker::Gif => {
                    app.features.gifs.serial += 1;
                    app.features.gifs.error.clear();
                    let cats = gif_request(app, crate::gifs::Query::Categories);
                    let favorites = gif_request(app, crate::gifs::Query::Favorites);
                    return Task::batch([cats, favorites, operation::focus("gif-search")]);
                }
                Picker::None => {
                    app.features.gifs.serial += 1;
                    app.features.gifs.request = None;
                    app.reaction_target = None;
                    return operation::focus("composer-editor");
                }
            }
        }
        Event::EmojiSearch(q) => {
            app.features.emoji.query = q;
            app.features.emoji.hover = None;
            app.features.emoji.y = 0.;
            return operation::scroll_to(
                "emoji-grid",
                iced::widget::scrollable::AbsoluteOffset { x: 0., y: 0. },
            );
        }
        Event::EmojiCategory(c) => {
            app.features.emoji.category = c.min(7);
            app.features.emoji.hover = None;
            app.features.emoji.query.clear();
            app.features.emoji.y = 0.;
            return operation::scroll_to(
                "emoji-grid",
                iced::widget::scrollable::AbsoluteOffset { x: 0., y: 0. },
            );
        }
        Event::EmojiTone(t) => {
            app.features.emoji.tone = t;
            return persist(app);
        }
        Event::EmojiHover(e) => app.features.emoji.hover = Some(e),
        Event::EmojiScroll(y) => app.features.emoji.y = y,
        Event::EmojiComplete(glyph) => {
            let text = app.editor.text();
            if let Some(text) =
                sticktalk_client_core::emoji::complete(&text, app.composer_caret(), &glyph)
            {
                app.editor.perform(text_editor::Action::SelectAll);
                app.editor
                    .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                        Arc::new(text),
                    )));
                app.suggestions_dismissed = true;
                app.features.emoji.record(&glyph);
                return Task::batch([persist(app), operation::focus("composer-editor")]);
            }
        }
        Event::GifSearch(q) => {
            let s = &mut app.features.gifs;
            s.query = q.chars().take(200).collect();
            s.tab = false;
            s.serial += 1;
            s.cursor = None;
            s.items.clear();
            s.error.clear();
            s.selected = 0;
            s.loading = true;
            let serial = s.serial;
            let (task, handle) = emit(async move {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                Event::GifDue(serial)
            })
            .abortable();
            s.request = Some(handle.abort_on_drop());
            return task;
        }
        Event::GifDue(serial) => {
            if serial == app.features.gifs.serial {
                app.features.gifs.request = None;
                return gif_request(
                    app,
                    crate::gifs::Query::Page {
                        q: app.features.gifs.query.clone(),
                        cursor: None,
                    },
                );
            }
        }
        Event::GifTab(tab) => {
            app.features.gifs.tab = tab;
            app.features.gifs.serial += 1;
            app.features.gifs.selected = 0;
            app.features.gifs.loading = false;
            return if tab {
                gif_request(app, crate::gifs::Query::Favorites)
            } else {
                gif_request(
                    app,
                    crate::gifs::Query::Page {
                        q: app.features.gifs.query.clone(),
                        cursor: None,
                    },
                )
            };
        }
        Event::GifResult(serial, result) => {
            if serial != app.features.gifs.serial {
                return Task::none();
            }
            app.features.gifs.loading = false;
            match result {
                Ok(crate::gifs::ResultData::Categories(items)) => {
                    app.features.gifs.categories = items
                }
                Ok(crate::gifs::ResultData::Favorites(ids)) => {
                    app.features.gifs.favorites = ids.clone();
                    if app.features.gifs.tab {
                        return gif_request(app, crate::gifs::Query::Items(ids));
                    }
                }
                Ok(crate::gifs::ResultData::Favorite(id, add)) => {
                    app.features.gifs.favorites.retain(|v| v != &id);
                    if add {
                        app.features.gifs.favorites.push(id);
                    } else {
                        app.features.gifs.favorite_items.retain(|v| v.id != id);
                    }
                }
                Ok(crate::gifs::ResultData::Page(page, favs)) => {
                    let s = &mut app.features.gifs;
                    if favs {
                        s.favorite_items = page.items;
                    } else {
                        for item in page.items {
                            if !s.items.iter().any(|v| v.id == item.id) {
                                s.items.push(item);
                            }
                        }
                        s.cursor = page.next_cursor;
                        if s.items.len() > 96 {
                            s.items.drain(..24);
                            s.scroll_y = (s.scroll_y
                                - 12.
                                    * (crate::theme::ATTACHMENT_CARD
                                        + crate::theme::S24
                                        + crate::theme::S8))
                                .max(0.);
                            return operation::scroll_to(
                                "gif-results",
                                iced::widget::scrollable::AbsoluteOffset {
                                    x: 0.,
                                    y: s.scroll_y,
                                },
                            );
                        }
                    }
                }
                Err(e) => app.features.gifs.error = e,
            }
        }
        Event::GifFavorite(id, add) => {
            return gif_request(app, crate::gifs::Query::Favorite(id, add))
        }
        Event::GifSend(item) => {
            if let Screen::Channel(channel_id) = app.state.screen {
                if app.send(Command::SendMessage(MessageDraft {
                    channel_id,
                    content: item.send.url,
                    reply_to: None,
                    attachment_ids: vec![],
                    mention_ids: vec![],
                })) {
                    app.features.picker = Picker::None;
                }
            }
        }
        Event::GifScroll(y, offset) => {
            app.features.gifs.scroll_y = offset;
            if y >= 0.85 && !app.features.gifs.loading && !app.features.gifs.tab {
                if let Some(cursor) = app.features.gifs.cursor.clone() {
                    return gif_request(
                        app,
                        crate::gifs::Query::Page {
                            q: app.features.gifs.query.clone(),
                            cursor: Some(cursor),
                        },
                    );
                }
            }
        }
        Event::GifHover(id) => app.features.gifs.hovered = id,
        Event::InlineHover(id) => app.features.inline_hover = id,
        Event::GifOpen(url) => {
            if !crate::gifs::inline_url(&url) {
                return Task::none();
            }
            let thumbnail = app
                .features
                .animations
                .get(&url)
                .and_then(|a| a.frame())
                .cloned();
            app.features.viewer.close();
            app.features.viewer.external = Some(url);
            app.features.viewer.handle = thumbnail;
            app.features.viewer.items = vec![Attachment {
                id: Id(0),
                filename: "image.gif".into(),
                byte_size: 0,
                content_type: "image/gif".into(),
                width: None,
                height: None,
                thumbnail: None,
            }];
            app.features.picker = Picker::None;
            return load_viewer(app);
        }
        Event::ViewerOpen(a) => {
            let mut items: Vec<_> = if let Screen::Channel(c) = app.state.screen {
                app.state
                    .model
                    .messages
                    .get(&c)
                    .into_iter()
                    .flat_map(|ms| ms.values())
                    .flat_map(|m| &m.attachments)
                    .filter(|a| a.content_type.starts_with("image/"))
                    .cloned()
                    .collect()
            } else {
                vec![]
            };
            if !items.iter().any(|i| i.id == a.id) {
                items.push(a.clone());
            }
            let index = items.iter().position(|i| i.id == a.id).unwrap();
            app.features.viewer.close();
            app.features.viewer.items = items;
            app.features.viewer.index = index;
            app.features.picker = Picker::None;
            return load_viewer(app);
        }
        Event::ViewerLoaded(serial, result) => {
            let s = &mut app.features.viewer;
            if serial == s.serial && s.current().is_some() {
                s.loading = false;
                s.task = None;
                match result {
                    Ok(v) => {
                        s.handle = Some(iced::widget::image::Handle::from_rgba(
                            v.width,
                            v.height,
                            v.rgba.clone(),
                        ));
                        s.decoded = Some(v);
                    }
                    Err(e) => s.error = e,
                }
            }
        }
        Event::ViewerClose => app.features.viewer.close(),
        Event::ViewerMore => app.features.viewer.more = !app.features.viewer.more,
        Event::ViewerForwardMenu => {
            app.features.viewer.forwarding = !app.features.viewer.forwarding
        }
        Event::ViewerZoom(factor) => {
            let fit = app.features.viewer.fit(app);
            let s = &mut app.features.viewer;
            s.scale = Some((s.scale.unwrap_or(fit) * factor).clamp(0.1, 8.));
        }
        Event::ViewerTransform(scale, pan) => {
            let s = &mut app.features.viewer;
            s.scale = Some(scale.clamp(0.1, 8.));
            s.pan = pan;
        }
        Event::ViewerReset => {
            app.features.viewer.scale = None;
            app.features.viewer.pan = iced::Vector::ZERO;
        }
        Event::ViewerActual => {
            app.features.viewer.scale = if app.features.viewer.scale == Some(1.) {
                None
            } else {
                Some(1.)
            };
            app.features.viewer.pan = iced::Vector::ZERO;
        }
        Event::ViewerNext(delta) => {
            let s = &mut app.features.viewer;
            if !s.items.is_empty() {
                s.index = (s.index as i32 + delta).rem_euclid(s.items.len() as i32) as usize;
                s.handle = None;
                s.decoded = None;
                s.scale = None;
                s.pan = iced::Vector::ZERO;
                s.task = None;
                return load_viewer(app);
            }
        }
        Event::ViewerSave => {
            if let Some(url) = app.features.viewer.external.clone() {
                if config(app).is_some_and(|c| c.mock) {
                    return Task::none();
                }
                return emit(async move {
                    let path = rfd::AsyncFileDialog::new()
                        .set_file_name("image.gif")
                        .save_file()
                        .await
                        .map(|f| f.path().to_owned());
                    Event::ViewerSaveTo(url, path)
                });
            }
            if let Some(a) = app.features.viewer.current().cloned() {
                return app.update(Msg::Download(a));
            }
        }
        Event::ViewerSaveTo(url, Some(path)) => {
            return emit(async move { Event::Notice(crate::gifs::save_original(url, path).await) })
        }
        Event::ViewerSaveTo(_, None) => {}
        Event::ViewerCopy => {
            if let Some(decoded) = app.features.viewer.decoded.clone() {
                if config(app).is_some_and(|c| c.mock) {
                    app.notice = "Image copied (mock).".into();
                } else {
                    return emit(async move {
                        Event::Notice(
                            tokio::task::spawn_blocking(move || {
                                let mut c = arboard::Clipboard::new()
                                    .map_err(|_| "Clipboard unavailable")?;
                                c.set_image(arboard::ImageData {
                                    width: decoded.width as usize,
                                    height: decoded.height as usize,
                                    bytes: std::borrow::Cow::Borrowed(&decoded.rgba),
                                })
                                .map_err(|_| "Could not copy image".into())
                            })
                            .await
                            .unwrap_or_else(|_| Err("Clipboard worker stopped".into())),
                        )
                    });
                }
            }
        }
        Event::ViewerForward(channel) => {
            if let Some(url) = app.features.viewer.external.clone() {
                if app.send(Command::SendMessage(MessageDraft {
                    channel_id: channel,
                    content: url,
                    reply_to: None,
                    attachment_ids: vec![],
                    mention_ids: vec![],
                })) {
                    app.features.viewer.close();
                }
                return Task::none();
            }
            if let (Some(c), Some(a)) = (config(app), app.features.viewer.current().cloned()) {
                let Some(account) = app.me() else {
                    return Task::none();
                };
                app.features.viewer.forwarding = false;
                return emit(async move {
                    let path = crate::media::namespace(&c, account).join(format!(
                        "forward-{}-{}",
                        crate::backend::nonce(),
                        a.filename.replace(['/', '\\'], "_")
                    ));
                    let result = if c.mock {
                        Err("Forwarding is mocked".into())
                    } else {
                        let result = crate::backend::Http::new(&c.server, &c.token);
                        match result {
                            Ok(h) => h.download(a.id, path.clone()).await.map(|_| path),
                            Err(e) => Err(e),
                        }
                    };
                    Event::ViewerForwarded(channel, result)
                });
            }
        }
        Event::ViewerForwarded(channel, result) => match result {
            Ok(path) => {
                app.features.viewer.close();
                app.notice = "Image added to the upload queue for the selected channel.".into();
                return update(app, Event::UploadStart(channel, path, true));
            }
            Err(e) => app.error = e,
        },
        Event::UploadStart(channel, path, forward) => {
            let pending = app
                .features
                .uploads
                .values()
                .filter(|t| t.job.channel == channel)
                .count();
            let other = if app.state.screen == Screen::Channel(channel) {
                app.state
                    .attachments
                    .iter()
                    .filter(|a| {
                        !app.features
                            .uploads
                            .values()
                            .any(|t| t.job.attachment.as_ref().is_some_and(|v| v.id == a.id))
                    })
                    .count()
            } else {
                0
            };
            if pending + other >= 4 {
                app.error = "You can attach up to four files per channel.".into();
                return Task::none();
            }
            if app.features.uploads.len() >= 16 {
                app.error = "The upload queue is full.".into();
                return Task::none();
            }
            let (Some(c), Some(account)) = (config(app), app.me()) else {
                return Task::none();
            };
            return emit(async move {
                let job = tokio::task::spawn_blocking(move || {
                    let (size, modified_ms) = uploads::fingerprint(&path)?;
                    let job = uploads::Job {
                        id: crate::backend::nonce(),
                        path,
                        channel,
                        forward,
                        size,
                        modified_ms,
                        upload: None,
                        attachment: None,
                    };
                    uploads::save(&c, account, &job)?;
                    Ok(job)
                })
                .await
                .unwrap_or_else(|_| Err("Could not queue upload".into()));
                Event::UploadPrepared(job)
            });
        }
        Event::UploadPrepared(result) => match result {
            Ok(job) => {
                app.features.uploads.insert(
                    job.id,
                    Transfer {
                        job,
                        offset: 0,
                        speed: 0.,
                        error: None,
                    },
                );
            }
            Err(e) => app.error = e,
        },
        Event::Upload(e) => match e {
            uploads::Event::Prepared(job) => {
                if let Some(t) = app.features.uploads.get_mut(&job.id) {
                    t.job = *job;
                }
            }
            uploads::Event::Progress(id, offset, _, speed) => {
                if let Some(t) = app.features.uploads.get_mut(&id) {
                    t.offset = offset;
                    t.speed = speed;
                }
            }
            uploads::Event::Complete(id, a) => {
                let mut forward = None;
                if let Some(t) = app.features.uploads.get_mut(&id) {
                    t.offset = t.job.size;
                    t.job.attachment = Some(a.clone());
                    if t.job.forward {
                        forward = Some((t.job.channel, a.id));
                    }
                }
                if let Some((channel_id, id)) = forward {
                    app.send(Command::SendMessage(MessageDraft {
                        channel_id,
                        content: String::new(),
                        reply_to: None,
                        attachment_ids: vec![id],
                        mention_ids: vec![],
                    }));
                }
            }
            uploads::Event::Failed(id, error) => {
                if let Some(t) = app.features.uploads.get_mut(&id) {
                    t.error = Some(error);
                }
            }
        },
        Event::UploadCancel(id) => {
            if app
                .features
                .uploads
                .get(&id)
                .and_then(|t| t.job.attachment.as_ref())
                .is_some_and(|a| {
                    app.state.pending.values().any(
                        |c| matches!(c,Command::SendMessage(d)if d.attachment_ids.contains(&a.id)),
                    )
                })
            {
                return Task::none();
            }
            if let (Some(t), Some(c), Some(account)) =
                (app.features.uploads.remove(&id), config(app), app.me())
            {
                app.state
                    .attachments
                    .retain(|a| t.job.attachment.as_ref().is_none_or(|v| v.id != a.id));
                uploads::remove(&c, account, id);
                return emit(async move {
                    crate::uploads::cancel(c, t.job.upload, t.job.attachment).await;
                    Event::Notice(Ok(()))
                });
            }
        }
        Event::UploadRetry(id) => {
            if let Some(t) = app.features.uploads.get_mut(&id) {
                t.error = None;
            }
        }
        Event::Restored(server, account, jobs, prefs) => {
            if config(app).is_some_and(|c| c.server == server) && app.me() == Some(account) {
                for job in jobs {
                    app.features.uploads.insert(
                        job.id,
                        Transfer {
                            offset: job.upload.as_ref().map_or(0, |u| u.offset),
                            job,
                            speed: 0.,
                            error: None,
                        },
                    );
                }
                if let Ok(v) = serde_json::from_str::<Preferences>(&prefs) {
                    app.features.autoplay = v.autoplay;
                    app.features.emoji.tone = v.tone;
                    app.features.emoji.usage = v
                        .usage
                        .into_iter()
                        .take(32)
                        .filter(|v| v.weight.is_finite())
                        .map(|u| sticktalk_emoji::Usage {
                            shortcode: u.code,
                            weight: u.weight,
                            updated_at: u.at,
                        })
                        .collect();
                }
            }
        }
        Event::AnimationLoaded(url, result) => {
            if let Some(a) = app.features.animations.get_mut(&url) {
                a.task = None;
                match result {
                    Ok(frames) => a.frames = Some(frames),
                    Err(_) => a.failed = true,
                }
            }
        }
        Event::Advance(url) => {
            if let Some(a) = app.features.animations.get_mut(&url) {
                if let Some(f) = &a.frames {
                    a.index = (a.index + 1) % f.images.len();
                }
            }
        }
        Event::Autoplay(mode) => {
            app.features.autoplay = mode;
            return persist(app);
        }
        Event::Notice(result) => {
            if let Err(e) = result {
                app.error = e;
            }
        }
        Event::Activation(a) => {
            use sticktalk_desktop_platform::activation::Activation;
            let focus = crate::desktop_host::update(
                app,
                crate::desktop_host::Event::Tray(
                    sticktalk_desktop_platform::tray::TrayAction::Open,
                ),
            );
            if a == Activation::Open {
                return focus;
            }
            if app.state.session.is_none() || app.me().is_none() {
                app.features.desktop.pending_activation = Some(a);
                return focus;
            } else {
                match a {
                    Activation::Open => return focus,
                    Activation::Invite(code) => {
                        app.features.picker = Picker::None;
                        app.dialog = Dialog::JoinGuild;
                        app.form.code = code;
                        return focus;
                    }
                    Activation::Message {
                        channel_id,
                        message_id,
                    } => {
                        if app.state.model.channels.contains_key(&Id(channel_id)) {
                            let nav = app.update(Msg::Select(Id(channel_id)));
                            return Task::batch([
                                focus,
                                nav,
                                app.update(Msg::JumpTo(Id(message_id))),
                            ]);
                        }
                    }
                }
            }
        }
    }
    Task::none()
}
fn load_viewer(app: &mut App) -> Task<Msg> {
    let (Some(c), Some(a)) = (config(app), app.features.viewer.current().cloned()) else {
        return Task::none();
    };
    let s = &mut app.features.viewer;
    s.serial += 1;
    s.loading = true;
    s.error.clear();
    let serial = s.serial;
    let size = ((app.width * 2.) as u32, (app.height * 2.) as u32);
    let external = s.external.clone();
    let (task, handle) =
        emit(async move { Event::ViewerLoaded(serial, viewer::load(c, a, size, external).await) })
            .abortable();
    s.task = Some(handle.abort_on_drop());
    task
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Usage {
    code: String,
    weight: f64,
    at: u64,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Preferences {
    autoplay: sticktalk_client_core::gifs::Autoplay,
    tone: Option<char>,
    usage: Vec<Usage>,
}
pub fn persist(app: &App) -> Task<Msg> {
    let (Some(c), Some(account)) = (config(app), app.me()) else {
        return Task::none();
    };
    if c.mock {
        return Task::none();
    }
    let path = crate::uploads::preferences(&c, account);
    let prefs = Preferences {
        autoplay: app.features.autoplay,
        tone: app.features.emoji.tone,
        usage: app
            .features
            .emoji
            .usage
            .iter()
            .map(|u| Usage {
                code: u.shortcode.clone(),
                weight: u.weight,
                at: u.updated_at,
            })
            .collect(),
    };
    emit(async move {
        Event::Notice(
            tokio::task::spawn_blocking(move || {
                std::fs::create_dir_all(path.parent().unwrap())
                    .map_err(|_| "Could not save media preferences")?;
                std::fs::write(path, serde_json::to_vec(&prefs).unwrap())
                    .map_err(|_| "Could not save media preferences".into())
            })
            .await
            .unwrap_or_else(|_| Err("Preference worker stopped".into())),
        )
    })
}
pub fn reconcile(app: &mut App) -> Task<Msg> {
    crate::desktop_host::sync(app);
    app.features.desktop.disabled = !app.settings.preferences.desktop_notifications;
    if app.features.desktop.disabled {
        app.features.flash_pending = false;
    }
    let (Some(c), Some(account)) = (config(app), app.me()) else {
        return Task::none();
    };
    let mut tasks = Vec::new();
    app.features.desktop.dry_run = c.mock
        || crate::agent_window::enabled()
        || std::env::args().any(|s| s == "--dry-run-platform");
    let count = app
        .state
        .model
        .reads
        .values()
        .map(|r| r.unread_count)
        .fold(0u32, u32::saturating_add);
    if !app.features.desktop.dry_run
        && (count != app.features.last_badge || app.features.flash_pending)
    {
        let flash = std::mem::take(&mut app.features.flash_pending);
        app.features.last_badge = count;
        let dnd = app.state.model.me.as_ref().is_some_and(|m| {
            matches!(
                m.desired_presence,
                PresenceStatus::Dnd | PresenceStatus::Invisible
            )
        });
        tasks.push(
            iced::window::oldest()
                .and_then(move |id| {
                    iced::window::run(id, move |w| {
                        crate::platform_glue::badge(w, count, flash, dnd)
                    })
                })
                .map(|_| Msg::Activity),
        );
    }
    let identity = (c.server.clone(), account);
    if app.features.loaded.as_ref() != Some(&identity) {
        app.features.loaded = Some(identity);
        app.features.desktop.dry_run = c.mock
            || crate::agent_window::enabled()
            || std::env::args().any(|s| s == "--dry-run-platform");
        if !c.mock {
            let c = c.clone();
            tasks.push(emit(async move {
                let server = c.server.clone();
                let (jobs, prefs) = tokio::task::spawn_blocking(move || {
                    (
                        uploads::restore(&c, account),
                        std::fs::read_to_string(crate::uploads::preferences(&c, account))
                            .ok()
                            .filter(|s| s.len() < 16384)
                            .unwrap_or_default(),
                    )
                })
                .await
                .unwrap_or_default();
                Event::Restored(server, account, jobs, prefs)
            }));
        }
    }
    if let Some(a) = app.features.desktop.pending_activation.take() {
        tasks.push(update(app, Event::Activation(a)));
    }
    if let Screen::Channel(channel) = app.state.screen {
        for t in app.features.uploads.values() {
            if t.job.channel == channel {
                if let Some(a) = &t.job.attachment {
                    if !app.state.attachments.iter().any(|v| v.id == a.id)
                        && !app
                            .state
                            .pending_files
                            .values()
                            .flatten()
                            .any(|v| v.id == a.id)
                    {
                        app.state.attachments.push(a.clone());
                    }
                }
            }
        }
    }
    let mut wanted = BTreeSet::new();
    let mut active = BTreeSet::new();
    if let Screen::Channel(_) = app.state.screen {
        let (_, _, top, _) = app.visible_range();
        let mut row_y = crate::theme::VIRTUAL_HEADER + top;
        let rows = app.visible_items();
        for (_, m, height, _) in rows {
            let visible = row_y + height >= app.scroll_y && row_y <= app.scroll_y + app.viewport_h;
            row_y += height;
            if !visible {
                continue;
            }
            let Some(m) = m else { continue };
            if crate::gifs::inline_url(&m.content) {
                let url = m.content.trim().to_owned();
                wanted.insert(url.clone());
                if app.features.autoplay.animates(
                    true,
                    app.focused,
                    app.features.inline_hover == Some(m.id),
                    false,
                ) {
                    active.insert(url);
                }
            }
        }
    }
    if app.features.picker == Picker::Gif {
        let s = &app.features.gifs;
        for (url, hovered) in gif_picker::visible(s) {
            if crate::gifs::inline_url(url) {
                wanted.insert(url.to_owned());
                if app
                    .features
                    .autoplay
                    .animates(true, app.focused, hovered, false)
                {
                    active.insert(url.to_owned());
                }
            }
        }
    }
    crate::video::reconcile(
        app,
        wanted
            .iter()
            .filter(|u| crate::video::is_url(u))
            .cloned()
            .collect(),
        &active,
    );
    wanted.retain(|url| !crate::video::is_url(url));
    let wanted: BTreeSet<_> = wanted.into_iter().take(8).collect();
    // Drop offscreen decoders and all frame handles immediately. Maximum eight 4 MiB clips.
    app.features
        .animations
        .retain(|url, _| wanted.contains(url));
    for url in wanted.into_iter().take(8) {
        let a = app.features.animations.entry(url.clone()).or_default();
        a.active = active.contains(&url) && app.features.viewer.current().is_none();
        if a.frames.is_none() && a.task.is_none() && !a.failed {
            let target = url.clone();
            let (task, handle) = emit(async move {
                Event::AnimationLoaded(target.clone(), crate::animated::load(target).await)
            })
            .abortable();
            a.task = Some(handle.abort_on_drop());
            tasks.push(task);
        }
    }
    Task::batch(tasks)
}
pub fn overlay(app: &App) -> Option<iced::Element<'_, Msg>> {
    use crate::theme as t;
    use iced::{
        widget::{container, mouse_area, opaque, space, stack},
        Fill, Padding,
    };
    if app.features.confirm_update {
        return Some(
            container(opaque(
                container(
                    iced::widget::column![
                        super::ui::title("Restart to update?"),
                        super::ui::hint("StickTalk will close and install the verified update."),
                        super::ui::action("Cancel", Msg::feature(Event::ConfirmUpdate)),
                        super::ui::primary("Restart", Msg::feature(Event::ApplyUpdate))
                    ]
                    .spacing(t::S16),
                )
                .padding(t::S24)
                .width(t::LOGIN_W)
                .style(t::modal),
            ))
            .center_x(Fill)
            .center_y(Fill)
            .style(t::dim)
            .into(),
        );
    }
    if app.features.viewer.current().is_some() {
        return Some(viewer::view(app));
    }
    let picker = match app.features.picker {
        Picker::None => return None,
        Picker::Emoji => emoji_picker::view(app),
        Picker::Gif => gif_picker::view(app),
    };
    Some(
        stack![
            mouse_area(container(space()).width(Fill).height(Fill))
                .on_press(Msg::feature(Event::Picker(Picker::None))),
            container(opaque(picker))
                .align_right(Fill)
                .align_bottom(Fill)
                .padding(Padding {
                    right: if app
                        .members_open
                        .unwrap_or(app.width >= t::MEMBER_BREAKPOINT)
                    {
                        t::MEMBERS + t::S16
                    } else {
                        t::S16
                    },
                    bottom: t::ATTACHMENT_CARD + t::S24,
                    top: 0.,
                    left: 0.
                })
        ]
        .into(),
    )
}

impl App {
    pub fn composer_caret(&self) -> usize {
        let p = self.editor.cursor().position;
        let text = self.editor.text();
        let mut offset = 0;
        for (i, line) in text.split('\n').enumerate() {
            if i == p.line {
                // iced/cosmic-text's column is a UTF-8 byte index, not a character count.
                return offset + p.column.min(line.len());
            }
            offset += line.len() + 1;
        }
        text.len()
    }
    pub fn emoji_candidates(&self) -> Vec<sticktalk_emoji::Emoji> {
        if self.suggestions_dismissed {
            vec![]
        } else {
            sticktalk_client_core::emoji::candidates(&self.editor.text(), self.composer_caret())
                .into_iter()
                .filter(|e| emoji_picker::has_artwork(e.emoji()))
                .collect()
        }
    }
}
pub fn committed(app: &mut App, m: &Message) {
    let ids: Vec<_> = app
        .features
        .uploads
        .iter()
        .filter(|(_, t)| {
            t.job
                .attachment
                .as_ref()
                .is_some_and(|a| m.attachments.iter().any(|v| v.id == a.id))
        })
        .map(|(id, _)| *id)
        .collect();
    if let (Some(c), Some(account)) = (config(app), app.me()) {
        for id in ids {
            app.features.uploads.remove(&id);
            crate::uploads::remove(&c, account, id);
        }
    }
}

pub fn update_pill(app: &App) -> iced::Element<'_, Msg> {
    if app.features.update_ready.is_some() {
        super::ui::action("Update ready — restart", Msg::feature(Event::ConfirmUpdate))
            .width(iced::Fill)
            .style(crate::theme::primary)
            .into()
    } else {
        iced::widget::space().height(0).into()
    }
}
pub fn fixture(app: &mut App, screen: &str) {
    crate::video::fixture(app, screen);
    app.features.desktop.dry_run = true;
    app.features.gifs.capability = GifCapability::Enabled(GifConfig::Proxy {
        provider: GifProviderName::Klipy,
    });
    if screen.starts_with("emoji-") || screen == "reaction-picker" {
        app.features.picker = Picker::Emoji;
        app.features.emoji.hover = sticktalk_emoji::search("smile", 1).first().copied();
        if screen == "emoji-search" {
            app.features.emoji.query = "cat".into();
            app.features.emoji.hover = None;
        }
        if screen == "reaction-picker" {
            app.reaction_target = Some(Id(2279));
        }
        if screen == "emoji-autocomplete" {
            app.features.picker = Picker::None;
            app.editor
                .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                    Arc::new("Thanks :smi".into()),
                )));
        }
    }
    if screen.starts_with("gif-") {
        app.features.picker = Picker::Gif;
        let image = ::image::load_from_memory(sticktalk_client_core::mock::IMAGE)
            .unwrap()
            .into_rgba8();
        let handle =
            iced::widget::image::Handle::from_rgba(image.width(), image.height(), image.into_raw());
        for (i, name) in [
            "Celebrate",
            "Happy dance",
            "Thank you",
            "Good morning",
            "Well done",
            "See you soon",
        ]
        .iter()
        .enumerate()
        {
            let media = GifMedia {
                url: format!("https://static.klipy.com/mock-{i}.webp"),
                width: 256,
                height: [144, 256, 192, 128, 200, 160][i],
                content_type: "image/webp".into(),
            };
            let item = GifItem {
                id: format!("klipy:mock{i}"),
                title: (*name).into(),
                preview: media.clone(),
                send: media,
            };
            let a = crate::animated::Animation {
                frames: Some(Arc::new(crate::animated::Frames {
                    images: vec![(handle.clone(), 100)],
                    bytes: 256 * 144 * 4,
                })),
                ..Default::default()
            };
            app.features.animations.insert(item.preview.url.clone(), a);
            app.features.gifs.items.push(item.clone());
            app.features.gifs.categories.push(GifCategory {
                title: (*name).into(),
                query: (*name).into(),
                preview: item.preview.clone(),
            });
            if i < 2 {
                app.features.gifs.favorites.push(item.id.clone());
                app.features.gifs.favorite_items.push(item);
            }
        }
        match screen {
            "gif-search" => app.features.gifs.query = "celebrate".into(),
            "gif-favorites" => app.features.gifs.tab = true,
            _ => {}
        }
    }
    if screen.starts_with("image-viewer") {
        let a = app
            .state
            .model
            .messages
            .values()
            .flat_map(|ms| ms.values())
            .flat_map(|m| &m.attachments)
            .find(|a| a.thumbnail.is_some())
            .cloned()
            .unwrap();
        app.features.viewer.items = vec![
            a.clone(),
            Attachment {
                filename: "photo-02.jpg".into(),
                ..a.clone()
            },
        ];
        app.features.viewer.handle = app.images.get(a.id).cloned();
        app.features.viewer.loading = screen == "image-viewer-loading";
        if screen == "image-viewer-zoom" {
            app.features.viewer.scale = Some(1.5);
        }
    }
    if screen.starts_with("upload-") {
        let (size, offset) = (2_147_483_648, 838_860_800);
        let job = uploads::Job {
            id: 42,
            path: std::path::PathBuf::from("C:/Clips/evening-session.mp4"),
            channel: Id(12),
            forward: false,
            size,
            modified_ms: 0,
            upload: None,
            attachment: None,
        };
        app.features.uploads.insert(
            42,
            Transfer {
                job,
                offset,
                speed: 12_582_912.,
                error: (screen == "upload-storage-full").then(|| "Server storage is full.".into()),
            },
        );
        if screen == "upload-other-channel" {
            app.features.uploads.get_mut(&42).unwrap().job.channel = Id(20);
        }
    }
    if screen == "restart-update" {
        app.features.update_ready = Some("0.2.0".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ready_uploads_survive_channel_switches() {
        let mut app = App::headless("upload-progress", iced::Size::new(1280., 800.));
        let attachment = Attachment {
            id: Id(9898),
            filename: "clip.mp4".into(),
            content_type: "video/mp4".into(),
            byte_size: 100,
            width: None,
            height: None,
            thumbnail: None,
        };
        let _ = app.update(Msg::feature(Event::Upload(uploads::Event::Complete(
            42,
            attachment.clone(),
        ))));
        assert!(app.state.attachments.iter().any(|a| a.id == attachment.id));
        let _ = app.update(Msg::Select(Id(20)));
        assert!(app.features.uploads.contains_key(&42));
        assert!(app.state.attachments.is_empty());
        let _ = app.update(Msg::Select(Id(12)));
        assert_eq!(
            app.state
                .attachments
                .iter()
                .filter(|a| a.id == attachment.id)
                .count(),
            1
        );
        committed(
            &mut app,
            &Message {
                id: Id(9900),
                channel_id: Id(12),
                author_id: Id(1),
                content: String::new(),
                created_at: 0,
                edited_at: None,
                reply_to: None,
                attachments: vec![attachment],
                mention_ids: vec![],
                reactions: vec![],
            },
        );
        assert!(app.features.uploads.is_empty());
    }
    #[test]
    fn old_gif_queries_cannot_replace_new_results() {
        let mut app = App::headless("gif-search", iced::Size::new(1280., 800.));
        app.features.gifs.serial = 10;
        let len = app.features.gifs.items.len();
        let _ = update(
            &mut app,
            Event::GifResult(
                9,
                Ok(crate::gifs::ResultData::Page(
                    GifPage {
                        items: vec![],
                        next_cursor: None,
                    },
                    false,
                )),
            ),
        );
        assert_eq!(app.features.gifs.items.len(), len);
    }
    #[test]
    fn closing_viewer_releases_original_and_ignores_late_results() {
        let mut app = App::headless("image-viewer", iced::Size::new(1280., 800.));
        let serial = app.features.viewer.serial;
        let _ = update(&mut app, Event::ViewerClose);
        let _ = update(
            &mut app,
            Event::ViewerLoaded(
                serial,
                Ok(Arc::new(crate::media::Decoded {
                    id: Id(1),
                    width: 1,
                    height: 1,
                    rgba: vec![255; 4],
                })),
            ),
        );
        assert!(app.features.viewer.handle.is_none());
        assert!(app.features.viewer.decoded.is_none());
        assert!(app.features.viewer.current().is_none());
    }
    #[test]
    fn shortcode_completion_preserves_tail_and_picker_inserts_at_caret() {
        let mut app = App::headless("emoji-autocomplete", iced::Size::new(1280., 800.));
        assert!(!app.emoji_candidates().is_empty());
        let _ = update(&mut app, Event::EmojiComplete("😄".into()));
        assert_eq!(app.editor.text(), "Thanks 😄 ");
    }
}

#[cfg(test)]
mod caret_tests {
    use super::*;
    #[test]
    fn unicode_caret_in_middle_uses_cosmic_byte_index() {
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        app.editor
            .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                Arc::new("你好 :smi tail".into()),
            )));
        for _ in 0..5 {
            app.editor
                .perform(text_editor::Action::Move(text_editor::Motion::Left));
        }
        assert_eq!(app.composer_caret(), "你好 :smi".len());
        assert!(!app.emoji_candidates().is_empty());
        let _ = update(&mut app, Event::EmojiComplete("😄".into()));
        assert_eq!(app.editor.text(), "你好 😄  tail");
    }
}

#[cfg(test)]
mod picker_lifecycle_tests {
    use super::*;
    #[test]
    fn search_clears_the_previous_hover_selection() {
        let mut app = App::headless("emoji-picker", iced::Size::new(1280., 800.));
        assert!(app.features.emoji.hover.is_some());
        let _ = update(&mut app, Event::EmojiSearch("cat".into()));
        assert!(app.features.emoji.hover.is_none());
        let expected = app.features.emoji.entries()[0].emoji();
        let _ = update(
            &mut app,
            Event::Key(iced::keyboard::Key::Named(
                iced::keyboard::key::Named::Enter,
            )),
        );
        assert_eq!(app.editor.text(), expected);
    }
    #[test]
    fn cancel_cannot_delete_an_attachment_in_a_pending_send() {
        let mut app = App::headless("upload-progress", iced::Size::new(1280., 800.));
        let a = Attachment {
            id: Id(99),
            filename: "file".into(),
            content_type: "application/octet-stream".into(),
            byte_size: 1,
            width: None,
            height: None,
            thumbnail: None,
        };
        app.features.uploads.get_mut(&42).unwrap().job.attachment = Some(a);
        app.state.pending.insert(
            9,
            Command::SendMessage(MessageDraft {
                channel_id: Id(12),
                content: String::new(),
                reply_to: None,
                attachment_ids: vec![Id(99)],
                mention_ids: vec![],
            }),
        );
        let _ = update(&mut app, Event::UploadCancel(42));
        assert!(app.features.uploads.contains_key(&42));
    }
}
