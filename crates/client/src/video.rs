//! Bounded, visibility-driven windowless MF playback. Paused/offscreen clips
//! retain only their poster and position; there is no idle timer or worker.
use crate::{
    app::{App, Msg},
    backend::Config,
};
use iced::{widget::image::Handle, Subscription, Task};
use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use sticktalk_proto::Attachment;

#[derive(Clone, Debug)]
pub enum Event {
    Toggle(String),
    Seek(String, f64),
    Volume(String, f32),
    Fullscreen(String),
    CloseFullscreen,
    Frame(String, u64, Arc<Frame>),
    Failed(String, u64, String),
}
#[derive(Debug)]
pub struct Frame {
    pub handle: Handle,
    pub position: f64,
    pub duration: f64,
    pub ended: bool,
}
pub struct Clip {
    pub frame: Option<Handle>,
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
    pub volume: f32,
    pub error: String,
    pub epoch: u64,
    pub running: bool,
    pub preview: bool,
    pub attachment: Option<Attachment>,
}
impl Clip {
    fn new(preview: bool, attachment: Option<Attachment>) -> Self {
        Self {
            frame: None,
            position: 0.,
            duration: 0.,
            playing: preview,
            volume: 0.5,
            error: String::new(),
            epoch: crate::backend::nonce(),
            running: false,
            preview,
            attachment,
        }
    }
}
#[derive(Default)]
pub struct State {
    pub clips: BTreeMap<String, Clip>,
    pub fullscreen: Option<String>,
}
pub fn attachment_key(a: &Attachment) -> String {
    format!("attachment:{}", a.id.0)
}
pub fn is_video(a: &Attachment) -> bool {
    matches!(a.content_type.as_str(), "video/mp4" | "video/webm")
}
pub fn is_url(url: &str) -> bool {
    url.split('?')
        .next()
        .is_some_and(|s| s.ends_with(".mp4") || s.ends_with(".webm"))
}
pub fn frame<'a>(app: &'a App, key: &str) -> Option<&'a Handle> {
    app.features
        .video
        .clips
        .get(key)
        .and_then(|c| c.frame.as_ref())
}
pub fn update(app: &mut App, event: Event) -> Task<Msg> {
    let state = &mut app.features.video;
    match event {
        Event::CloseFullscreen => {
            if let Some(key) = state.fullscreen.take() {
                if let Some(c) = state.clips.get_mut(&key) {
                    c.epoch = c.epoch.wrapping_add(1);
                }
            }
            return screen_mode(false, app.mock_mode);
        }
        Event::Fullscreen(key) => {
            if let Some(c) = state.clips.get_mut(&key) {
                c.epoch = c.epoch.wrapping_add(1);
            }
            state.fullscreen = if state.fullscreen.as_ref() == Some(&key) {
                None
            } else {
                Some(key)
            };
            return screen_mode(state.fullscreen.is_some(), app.mock_mode);
        }
        Event::Toggle(key) => {
            if let Some(c) = state.clips.get_mut(&key) {
                c.error.clear();
                c.playing = !c.playing;
                c.epoch = c.epoch.wrapping_add(1);
            }
        }
        Event::Seek(key, seconds) => {
            if let Some(c) = state.clips.get_mut(&key) {
                c.position = seconds.clamp(0., c.duration.max(0.));
                c.frame = None;
                c.epoch = c.epoch.wrapping_add(1);
            }
        }
        Event::Volume(key, volume) => {
            if let Some(c) = state.clips.get_mut(&key) {
                c.volume = volume.clamp(0., 1.);
                c.epoch = c.epoch.wrapping_add(1);
            }
        }
        Event::Frame(key, epoch, f) => {
            if let Some(c) = state.clips.get_mut(&key).filter(|c| c.epoch == epoch) {
                c.frame = Some(f.handle.clone());
                c.position = f.position;
                c.duration = if f.duration.is_finite() && f.duration > 0. {
                    f.duration
                } else {
                    c.duration
                };
                if f.ended && !c.preview {
                    c.playing = false;
                }
            }
        }
        Event::Failed(key, epoch, error) => {
            if let Some(c) = state.clips.get_mut(&key).filter(|c| c.epoch == epoch) {
                c.error = error;
                c.playing = false;
                c.running = false;
            }
        }
    }
    Task::none()
}
fn screen_mode(fullscreen: bool, dry: bool) -> Task<Msg> {
    if dry || crate::agent_window::enabled() || cfg!(test) {
        return Task::none();
    }
    iced::window::oldest().and_then(move |id| {
        iced::window::set_mode(
            id,
            if fullscreen {
                iced::window::Mode::Fullscreen
            } else {
                iced::window::Mode::Windowed
            },
        )
    })
}
/// Call from the feature reducer with the exact visible picker tiles. Message
/// visibility comes from the same virtual history geometry as the renderer.
pub fn reconcile(app: &mut App, previews: BTreeSet<String>, active_previews: &BTreeSet<String>) {
    let mut wanted: BTreeMap<String, (bool, Option<Attachment>)> = previews
        .into_iter()
        .filter(|u| is_url(u))
        .map(|u| (u, (true, None)))
        .collect();
    let (_, _, top, _) = app.visible_range();
    let mut y = crate::theme::VIRTUAL_HEADER + top;
    for (_, message, height, _) in app.visible_items() {
        let visible = y + height >= app.scroll_y && y <= app.scroll_y + app.viewport_h;
        y += height;
        if !visible {
            continue;
        }
        if let Some(m) = message {
            if crate::gifs::inline_url(&m.content) && is_url(m.content.trim()) {
                wanted.insert(m.content.trim().into(), (true, None));
            }
            for a in m.attachments.iter().filter(|a| is_video(a)) {
                wanted.insert(attachment_key(a), (false, Some(a.clone())));
            }
        }
    }
    let focused = app.focused && app.features.viewer.current().is_none();
    // Preserve attachment controls/posters across a scroll, bounded to 16 entries.
    while app.features.video.clips.len() > 16 {
        let victim = app
            .features
            .video
            .clips
            .keys()
            .find(|k| !wanted.contains_key(*k))
            .cloned();
        if let Some(k) = victim {
            app.features.video.clips.remove(&k);
        } else {
            break;
        }
    }
    for (key, c) in &mut app.features.video.clips {
        if !wanted.contains_key(key) && c.running {
            c.running = false;
            c.playing = false;
            c.epoch = c.epoch.wrapping_add(1);
        }
    }
    let wanted_keys: BTreeSet<_> = wanted.keys().cloned().collect();
    let mut running = 0;
    for (key, (preview, attachment)) in wanted {
        let c = app
            .features
            .video
            .clips
            .entry(key.clone())
            .or_insert_with(|| Clip::new(preview, attachment));
        if preview {
            c.playing = active_previews.contains(&key);
        }
        let on = focused && running < 8 && c.error.is_empty() && (c.playing || c.frame.is_none());
        if on != c.running {
            c.epoch = c.epoch.wrapping_add(1);
            c.running = on;
        }
        if on {
            running += 1;
        }
    }
    while app.features.video.clips.len() > 16 {
        let victim = app
            .features
            .video
            .clips
            .iter()
            .find(|(k, c)| !c.running && !wanted_keys.contains(*k))
            .or_else(|| app.features.video.clips.iter().find(|(_, c)| !c.running))
            .map(|(k, _)| k.clone());
        if let Some(k) = victim {
            app.features.video.clips.remove(&k);
        } else {
            break;
        }
    }
}
#[derive(Clone)]
struct Run {
    key: String,
    epoch: u64,
    config: Config,
    attachment: Option<Attachment>,
    position: f64,
    volume: f32,
    preview: bool,
    poster: bool,
    fullscreen: bool,
}
impl Hash for Run {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.key.hash(h);
        self.epoch.hash(h);
    }
}
pub fn subscription(app: &App) -> Subscription<Msg> {
    let Some(config) = app
        .state
        .session
        .as_ref()
        .map(|s| s.config.clone())
        .filter(|c| !c.mock)
    else {
        return Subscription::none();
    };
    Subscription::batch(
        app.features
            .video
            .clips
            .iter()
            .filter(|(_, c)| c.running)
            .take(8)
            .map(|(key, c)| {
                Subscription::run_with(
                    Run {
                        key: key.clone(),
                        epoch: c.epoch,
                        config: config.clone(),
                        attachment: c.attachment.clone(),
                        position: c.position,
                        volume: c.volume,
                        preview: c.preview,
                        poster: !c.playing,
                        fullscreen: app.features.video.fullscreen.as_ref() == Some(key),
                    },
                    run,
                )
                .map(|e| Msg::feature(crate::features::Event::Video(e)))
            }),
    )
}
fn run(input: &Run) -> impl iced::futures::Stream<Item = Event> {
    use iced::futures::SinkExt;
    let input = input.clone();
    iced::stream::channel(
        1,
        move |mut output: iced::futures::channel::mpsc::Sender<Event>| async move {
            let key = input.key.clone();
            let epoch = input.epoch;
            let result = async {
                static SLOTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> =
                    std::sync::OnceLock::new();
                let permit = SLOTS
                    .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(8)))
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| "Video decoder unavailable".to_string())?;
                let file = download(&input).await?;
                let (tx, mut rx) = tokio::sync::mpsc::channel(1);
                let stop = Arc::new(AtomicBool::new(false));
                struct Stop(Arc<AtomicBool>);
                impl Drop for Stop {
                    fn drop(&mut self) {
                        self.0.store(true, Ordering::Release);
                    }
                }
                let guard = Stop(stop.clone());
                let worker = input.clone();
                // Long-lived decoders must not occupy the shared four-slot
                // blocking pool: that would starve thumbnails, uploads and
                // notification work, and leave additional visible tiles stuck.
                let failed = tx.clone();
                let spawned = std::thread::Builder::new()
                    .name("sticktalk-video".into())
                    .spawn(move || {
                        let _permit = permit;
                        #[cfg(windows)]
                        let result = (|| -> Result<(), String> {
                            let player = sticktalk_media::FilePlayer::open_cancellable(
                                &file,
                                sticktalk_media::Size {
                                    width: if worker.preview {
                                        320
                                    } else if worker.fullscreen {
                                        1920
                                    } else {
                                        640
                                    },
                                    height: if worker.preview {
                                        320
                                    } else if worker.fullscreen {
                                        1080
                                    } else {
                                        360
                                    },
                                },
                                worker.preview || worker.poster,
                                f64::from(worker.volume),
                                worker.preview,
                                &stop,
                            )
                            .map_err(|e| e.to_string())?;
                            if stop.load(Ordering::Acquire) {
                                return Ok(());
                            }
                            player.play(worker.position).map_err(|e| e.to_string())?;
                            let mut last_frame: Option<Handle> = None;
                            while !stop.load(Ordering::Acquire) {
                                if player.ended() {
                                    if let Some(handle) = last_frame.take() {
                                        let _ = tx.blocking_send(Ok(Arc::new(Frame {
                                            handle,
                                            position: 0.,
                                            duration: 0.,
                                            ended: true,
                                        })));
                                    }
                                    break;
                                }
                                if let Some(f) = player.frame().map_err(|e| e.to_string())? {
                                    let ended = f.ended;
                                    let frame = Frame {
                                        handle: Handle::from_rgba(
                                            f.size.width,
                                            f.size.height,
                                            f.rgba,
                                        ),
                                        position: f.position,
                                        duration: f.duration,
                                        ended,
                                    };
                                    last_frame = Some(frame.handle.clone());
                                    if tx.blocking_send(Ok(Arc::new(frame))).is_err()
                                        || worker.poster
                                        || ended
                                    {
                                        break;
                                    }
                                }
                                std::thread::sleep(std::time::Duration::from_millis(33));
                            }
                            Ok(())
                        })();
                        #[cfg(not(windows))]
                        let result: Result<(), String> =
                            Err("Video playback is available on Windows".into());
                        if let Err(e) = result {
                            let _ = tx.blocking_send(Err(e));
                        }
                    });
                if spawned.is_err() {
                    let _ = failed
                        .send(Err("Could not start video decoder".into()))
                        .await;
                }
                drop(failed);
                while let Some(frame) = rx.recv().await {
                    output
                        .send(Event::Frame(key.clone(), epoch, frame?))
                        .await
                        .map_err(|_| "Video view closed".to_string())?;
                }
                drop(guard);
                Ok::<_, String>(())
            }
            .await;
            if let Err(e) = result {
                let _ = output.send(Event::Failed(key, epoch, e)).await;
            }
        },
    )
}
fn namespace(config: &Config) -> String {
    use sha2::Digest;
    format!(
        "{:x}",
        sha2::Sha256::digest(format!("{}:{}", config.server, config.token).as_bytes())
    )
}
/// Session-isolated file names prevent cache hits across accounts. Removing
/// access/logout also drops all retained frames and this session's disk files.
pub fn purge(config: &Config) {
    if config.mock {
        return;
    }
    let prefix = format!("{}-", namespace(config));
    let root = crate::storage::root().join("video-cache");
    if let Ok(files) = std::fs::read_dir(root) {
        for f in files
            .flatten()
            .filter(|f| f.file_name().to_string_lossy().starts_with(&prefix))
        {
            let _ = std::fs::remove_file(f.path());
        }
    }
}
async fn download(input: &Run) -> Result<std::path::PathBuf, String> {
    use futures_util::StreamExt;
    use sha2::Digest;
    use tokio::io::AsyncWriteExt;
    static CACHE: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    let _cache = CACHE
        .get_or_init(|| tokio::sync::Semaphore::new(1))
        .acquire()
        .await
        .map_err(|_| "Video cache unavailable")?;
    let root = crate::storage::root().join("video-cache");
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|_| "Could not open video cache")?;
    let path = root.join(format!(
        "{}-{:x}.{}",
        namespace(&input.config),
        sha2::Sha256::digest(input.key.as_bytes()),
        if input
            .attachment
            .as_ref()
            .is_some_and(|a| a.content_type == "video/webm")
            || input
                .key
                .split('?')
                .next()
                .is_some_and(|p| p.ends_with(".webm"))
        {
            "webm"
        } else {
            "mp4"
        }
    ));
    if path.is_file() {
        return Ok(path);
    }
    let http = crate::backend::Http::new(&input.config.server, &input.config.token)?;
    let request = if let Some(a) = &input.attachment {
        http.get(&format!("/files/{}", a.id.0))
    } else {
        if !crate::gifs::inline_url(&input.key) {
            return Err("Unsupported video origin".into());
        }
        http.client.get(&input.key)
    };
    let response = request.send().await.map_err(|_| "Could not load video")?;
    if !response.status().is_success() {
        return Err("Video is no longer available".into());
    }
    let max = if input.preview {
        8 * 1024 * 1024
    } else {
        128 * 1024 * 1024
    };
    if response.content_length().is_some_and(|n| n > max) {
        return Err("Video exceeds the 128 MB playback cache limit".into());
    }
    let reserve = response.content_length().unwrap_or(max).min(max);
    // Cache is globally bounded, eviction only on a new download (no idle scan).
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .map_err(|_| "Could not read video cache")?
        .filter_map(Result::ok)
        .filter_map(|f| {
            f.metadata()
                .ok()
                .map(|m| (f.path(), m.len(), m.modified().ok()))
        })
        .collect();
    files.sort_by_key(|f| f.2);
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    for (p, size, _) in files {
        if total + reserve <= 128 * 1024 * 1024 {
            break;
        }
        if tokio::fs::remove_file(p).await.is_ok() {
            total = total.saturating_sub(size);
        }
    }
    if total + reserve > 128 * 1024 * 1024 {
        return Err("Video cache is full while other clips are playing".into());
    }
    let temporary = path.with_extension(format!("{}.part", crate::backend::nonce()));
    struct Partial(std::path::PathBuf);
    impl Drop for Partial {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _partial = Partial(temporary.clone());
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|_| "Could not cache video")?;
    let mut stream = response.bytes_stream();
    let mut received = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Video download interrupted")?;
        received += chunk.len() as u64;
        if received > max {
            return Err("Video exceeds playback cache limit".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|_| "Could not cache video")?;
    }
    file.flush().await.map_err(|_| "Could not cache video")?;
    drop(file);
    tokio::fs::rename(temporary, &path)
        .await
        .map_err(|_| "Could not cache video")?;
    Ok(path)
}

pub fn fixture(app: &mut App, screen: &str) {
    if !screen.starts_with("inline-video-") {
        return;
    }
    let a = Attachment {
        id: sticktalk_proto::Id(99900),
        filename: "afternoon-clip.mp4".into(),
        content_type: "video/mp4".into(),
        byte_size: 4_185_600,
        width: Some(1920),
        height: Some(1080),
        thumbnail: None,
    };
    let key = attachment_key(&a);
    let mut clip = Clip::new(false, Some(a.clone()));
    let pixels = image::load_from_memory(sticktalk_client_core::mock::IMAGE)
        .unwrap()
        .into_rgba8();
    clip.frame = Some(Handle::from_rgba(
        pixels.width(),
        pixels.height(),
        pixels.into_raw(),
    ));
    clip.position = 12.;
    clip.duration = 42.;
    clip.playing = screen == "inline-video-playing";
    app.features.video.clips.insert(key.clone(), clip);
    if let crate::app::Screen::Channel(channel) = app.state.screen {
        if let Some((_, m)) = app
            .state
            .model
            .messages
            .get_mut(&channel)
            .and_then(|ms| ms.iter_mut().next_back())
        {
            m.attachments = vec![a];
            m.content = "Here's the clip from this afternoon.".into();
        }
    }
    if screen == "inline-video-fullscreen" {
        app.features.video.fullscreen = Some(key);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn picker_video_workers_are_visible_bounded_and_stop_on_close_or_blur() {
        let mut app = App::headless("gif-search", iced::Size::new(1280., 800.));
        for item in &mut app.features.gifs.items {
            item.preview.url = item.preview.url.replace(".webp", ".mp4");
            item.preview.content_type = "video/mp4".into();
        }
        let _ = crate::features::reconcile(&mut app);
        assert!(app.features.video.clips.values().any(|c| c.running));
        assert!(
            app.features
                .video
                .clips
                .values()
                .filter(|c| c.running)
                .count()
                <= 8
        );
        app.focused = false;
        let _ = crate::features::reconcile(&mut app);
        assert!(app.features.video.clips.values().all(|c| !c.running));
        app.features.picker = crate::features::Picker::None;
        app.focused = true;
        let _ = crate::features::reconcile(&mut app);
        assert!(app.features.video.clips.values().all(|c| !c.running));
    }
    #[test]
    fn stale_frames_do_not_reanimate_closed_or_replaced_views() {
        let mut app = App::headless("guild-chat", iced::Size::new(1280., 800.));
        let clip = Clip::new(true, None);
        let epoch = clip.epoch;
        app.features.video.clips.insert("test".into(), clip);
        let frame = Arc::new(Frame {
            handle: Handle::from_rgba(1, 1, vec![0, 0, 0, 255]),
            position: 1.,
            duration: 2.,
            ended: false,
        });
        let _ = update(
            &mut app,
            Event::Frame("test".into(), epoch.wrapping_sub(1), frame),
        );
        assert!(app.features.video.clips["test"].frame.is_none());
    }
}
