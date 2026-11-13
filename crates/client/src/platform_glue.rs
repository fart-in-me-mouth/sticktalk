//! Platform notification projection and explicit effects. Dry run never touches the desktop.
use sticktalk_desktop_platform::{
    activation::Activation,
    notification::{MessageToast, NotificationContext, NotificationPolicy, NotifyLevel},
};
use sticktalk_proto::*;
#[derive(Default)]
pub struct State {
    pub settings: Vec<NotificationSettings>,
    pub pending_activation: Option<Activation>,
    pub dry_run: bool,
    pub last_toast: Option<String>,
}
impl State {
    pub fn observe(&mut self, event: &Event) {
        if let Event::NotificationSettingsUpdated(s) = event {
            self.settings
                .retain(|x| x.guild_id != s.guild_id || x.channel_id != s.channel_id);
            self.settings.push(s.clone());
        }
    }
    pub fn notification(
        &mut self,
        model: &crate::model::Model,
        message: &Message,
        focused: bool,
        viewing: bool,
    ) -> Option<MessageToast> {
        let me = model.me.as_ref()?;
        let channel = model.channels.get(&message.channel_id)?;
        let direct = matches!(channel.kind, ChannelKind::Direct | ChannelKind::GroupDm);
        let mentioned = message.mention_ids.contains(&me.user.id);
        let blocked = model
            .relationships
            .get(&message.author_id)
            .is_some_and(|r| r.kind == RelationshipKind::Blocked);
        let c = sticktalk_client_core::notifications::Context {
            guild: channel.guild_id,
            channel: Some(channel.id),
            direct,
            mentioned,
            everyone_only: !message.content.contains(&format!("<@{}>", me.user.id.0))
                && (message.content.contains("@everyone") || message.content.contains("@here"))
                && !message.content.contains("<@&"),
            focused,
            viewing,
            dnd: matches!(
                me.desired_presence,
                PresenceStatus::Dnd | PresenceStatus::Invisible
            ),
            own: message.author_id == me.user.id,
            blocked,
            now: crate::mock::now(),
        };
        if !sticktalk_client_core::notifications::permits(&self.settings, c) {
            return None;
        }
        let toast = MessageToast {
            sender: model.name(message.author_id),
            server_channel: channel
                .name
                .clone()
                .unwrap_or_else(|| "Direct message".into()),
            preview: message.content.chars().take(240).collect(),
            avatar_uri: None,
            channel_id: channel.id.0,
            message_id: message.id.0,
        };
        self.last_toast = toast.xml().ok();
        Some(toast)
    }
}
pub async fn notify(toast: MessageToast, dry_run: bool, sound: bool) -> Result<(), String> {
    if dry_run || cfg!(test) {
        return toast.xml().map(|_| ()).map_err(|e| e.to_string());
    }
    tokio::task::spawn_blocking(move || {
        #[cfg(windows)]
        {
            let _apartment = sticktalk_desktop_platform::windows::Apartment::initialize()
                .map_err(|e| e.to_string())?;
            sticktalk_desktop_platform::windows::show_toast(
                &toast,
                NotificationPolicy {
                    server: NotifyLevel::All,
                    channel: None,
                    muted: false,
                },
                NotificationContext::default(),
            )
            .map_err(|e| e.to_string())?;
            if sound {
                sticktalk_desktop_platform::windows::play_sound(
                    sticktalk_desktop_platform::sound::Sound::Message,
                    true,
                )
                .map_err(|e| e.to_string())?;
            }
        }
        #[cfg(not(windows))]
        let _ = sound;
        Ok(())
    })
    .await
    .map_err(|_| "Notification worker stopped".to_string())?
}

/// Bounded best-effort effects; no idle worker.
pub fn dispatch(toast: MessageToast, dry: bool, sound: bool) {
    static GATE: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    let Ok(permit) = GATE
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
        .try_acquire_owned()
    else {
        return;
    };
    tokio::spawn(async move {
        let _permit = permit;
        let _ = notify(toast, dry, sound).await;
    });
}
/// One shared runtime for pre-window instance IPC and iced tasks.
pub struct Executor;
static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(4)
            .enable_all()
            .build()
            .expect("desktop runtime")
    })
}
impl iced::Executor for Executor {
    fn new() -> Result<Self, std::io::Error> {
        let _ = runtime();
        Ok(Self)
    }
    fn spawn(&self, f: impl std::future::Future<Output = ()> + Send + 'static) {
        drop(runtime().spawn(f));
    }
    fn block_on<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        runtime().block_on(f)
    }
    fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = runtime().enter();
        f()
    }
}
static INITIAL: std::sync::Mutex<Option<Activation>> = std::sync::Mutex::new(None);
pub fn initial_activation() -> Option<Activation> {
    INITIAL.lock().ok()?.take()
}
#[cfg(windows)]
static INSTANCE: std::sync::OnceLock<
    std::sync::Arc<tokio::sync::Mutex<sticktalk_desktop_platform::windows::instance::Instance>>,
> = std::sync::OnceLock::new();
pub struct Host {
    pub should_run: bool,
    #[cfg(windows)]
    _apartment: Option<sticktalk_desktop_platform::windows::Apartment>,
}
pub fn bootstrap() -> Result<Host, String> {
    let dry = std::env::args().any(|a| matches!(a.as_str(), "--mock" | "--dry-run-platform"))
        || std::env::var("STICKTALK_AGENT_WINDOW").is_ok_and(|v| v == "secondary");
    #[cfg(windows)]
    {
        if dry {
            return Ok(Host {
                should_run: true,
                _apartment: None,
            });
        }
        let activation = std::env::args()
            .find(|s| s.starts_with("sticktalk:"))
            .map(|s| Activation::parse(&s))
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or(Activation::Open);
        match runtime()
            .block_on(
                sticktalk_desktop_platform::windows::instance::Instance::launch(
                    &crate::storage::root(),
                    activation.clone(),
                ),
            )
            .map_err(|e| e.to_string())?
        {
            sticktalk_desktop_platform::windows::instance::Launch::Forwarded => {
                return Ok(Host {
                    should_run: false,
                    _apartment: None,
                })
            }
            sticktalk_desktop_platform::windows::instance::Launch::Primary(instance) => {
                let _ = INSTANCE.set(std::sync::Arc::new(tokio::sync::Mutex::new(instance)));
            }
        }
        *INITIAL.lock().map_err(|_| "Activation queue unavailable")? = Some(activation);
        let apartment = sticktalk_desktop_platform::windows::Apartment::initialize()
            .map_err(|e| e.to_string())?;
        sticktalk_desktop_platform::windows::set_app_id().map_err(|e| e.to_string())?;
        Ok(Host {
            should_run: true,
            _apartment: Some(apartment),
        })
    }
    #[cfg(not(windows))]
    {
        let _ = dry;
        Ok(Host { should_run: true })
    }
}
pub fn has_instance() -> bool {
    #[cfg(windows)]
    {
        INSTANCE.get().is_some()
    }
    #[cfg(not(windows))]
    {
        false
    }
}
pub fn activations(_: &()) -> impl futures_util::Stream<Item = Activation> {
    iced::stream::channel(
        1,
        |mut out: iced::futures::channel::mpsc::Sender<Activation>| async move {
            #[cfg(windows)]
            if let Some(instance) = INSTANCE.get() {
                use futures_util::SinkExt;
                loop {
                    match instance.lock().await.receive().await {
                        Ok(a) => {
                            if out.send(a).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => continue,
                    }
                }
            }
            #[cfg(not(windows))]
            let _ = &mut out;
        },
    )
}
/// Explicit UI-thread effect; headless/dry-run callers do not invoke this function.
pub fn badge(
    window: &dyn iced_runtime::window::Window,
    count: u32,
    flash: bool,
    dnd: bool,
) -> bool {
    #[cfg(windows)]
    {
        use iced_runtime::window::raw_window_handle::RawWindowHandle;
        let Ok(h) = window.window_handle() else {
            return false;
        };
        let RawWindowHandle::Win32(h) = h.as_raw() else {
            return false;
        };
        let hwnd = windows::Win32::Foundation::HWND(h.hwnd.get() as *mut _);
        let result = sticktalk_desktop_platform::windows::set_unread_badge(hwnd, count).is_ok();
        if flash {
            sticktalk_desktop_platform::windows::flash_mention(hwnd, dnd);
        }
        result
    }
    #[cfg(not(windows))]
    {
        let _ = (window, count, flash, dnd);
        false
    }
}
#[derive(Clone, Debug)]
pub enum UpdateEvent {
    Ready(String),
    Failed(String),
}
pub fn updates(server: &str) -> impl futures_util::Stream<Item = UpdateEvent> {
    let server = server.to_owned();
    iced::stream::channel(
        1,
        move |mut out: iced::futures::channel::mpsc::Sender<UpdateEvent>| async move {
            use futures_util::SinkExt;
            let Ok(updater) = sticktalk_desktop_platform::updater::Updater::embedded(
                &server,
                env!("CARGO_PKG_VERSION"),
                crate::storage::root().join("Updates"),
            ) else {
                return;
            };
            if let Ok(Some(pending)) = updater.pending().await {
                if out
                    .send(UpdateEvent::Ready(pending.version().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            let worker = updater.clone();
            let task = tokio::spawn(async move { worker.watch(tx).await });
            struct Abort(tokio::task::JoinHandle<()>);
            impl Drop for Abort {
                fn drop(&mut self) {
                    self.0.abort();
                }
            }
            let _worker = Abort(task);
            while let Some(result) = rx.recv().await {
                let e = match result {
                    Ok(Some(update)) => match updater.download(&update).await {
                        Ok(stage) => UpdateEvent::Ready(stage.version().into()),
                        Err(e) => UpdateEvent::Failed(e.to_string()),
                    },
                    Ok(None) => continue,
                    Err(e) => UpdateEvent::Failed(e.to_string()),
                };
                if out.send(e).await.is_err() {
                    break;
                }
            }
        },
    )
}
pub async fn restart(server: String) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|_| "Could not locate installed client")?;
    sticktalk_desktop_platform::updater::Updater::embedded(
        &server,
        env!("CARGO_PKG_VERSION"),
        crate::storage::root().join("Updates"),
    )
    .map_err(|e| e.to_string())?
    .restart_to_apply(&exe)
    .await
    .map_err(|e| e.to_string())
}

/// Focus is an independent sound guard even when another channel permits a toast.
pub fn sound_requested(enabled: bool, focused: bool) -> bool {
    enabled && !focused
}
#[cfg(test)]
mod sound_tests {
    use super::*;
    #[tokio::test]
    async fn test_notification_and_sound_effects_are_dry() {
        assert!(!sound_requested(true, true));
        assert!(!sound_requested(false, false));
        assert!(sound_requested(true, false));
        let toast = MessageToast {
            sender: "A".into(),
            server_channel: "general".into(),
            preview: "hello".into(),
            avatar_uri: None,
            channel_id: 1,
            message_id: 2,
        };
        // cfg(test) also forces dry-run if a caller accidentally passes false.
        notify(toast, false, true).await.unwrap();
    }
}
