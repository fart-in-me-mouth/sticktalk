//! Lazily loaded features (docs/features/perf.md).
//!
//! Each feature's controller, its REST decoders and the decoders of its own
//! events and command results are reachable only from one split function,
//! `<feature>::run`, so the WASM splitter puts them in that feature's own
//! module. The core talks to a feature only through [`Call`]s: nothing of a
//! feature the user has not used is downloaded, compiled or kept in memory.
//!
//! Order: the socket awaits a feature's event (`event`) before it reads the
//! next frame, so events and answers keep the server's order even while a
//! module is still downloading. The cursor only moves once an event is
//! applied, so a failed download ends the connection and the resumed stream
//! replays the event.
use super::*;
use crate::inbound::Raw;
use std::future::Future;
use std::task::{Context, Poll};

pub(crate) use crate::inbound::Feature;

/// A feature by its name, as `prefetch:<name>` gives it.
pub(super) fn from_name(name: &str) -> Option<Feature> {
    {
        Some(match name {
            "content" => Feature::Content,
            "threads" => Feature::Threads,
            "expression" => Feature::Expression,
            "moderation" => Feature::Moderation,
            "privacy" => Feature::Privacy,
            "nav" => Feature::Nav,
            "profiles" => Feature::Profiles,
            "authsec" => Feature::Authsec,
            "community" => Feature::Community,
            "stage" => Feature::Stage,
            "themes" => Feature::Themes,
            _ => return None,
        })
    }
}

/// What the core asks of a feature. Everything is owned, so a call can wait
/// for the feature's module.
// One short-lived value per call; boxing the command would touch every match.
#[allow(clippy::large_enum_variant)]
pub(super) enum Call {
    /// A `data-live` action of the feature: kind and argument.
    Action(String, String),
    /// A form of the feature was submitted.
    Submit(String),
    /// A dialog opened; the feature loads what it shows if it is one of its own.
    Opened(String),
    /// A fresh snapshot (Ready): subscribe and fetch what the feature keeps.
    Ready,
    /// One of the feature's events (the `data` of its `Event` variant).
    Event(Raw),
    /// The answer to a command the feature cares about.
    Answer(Option<Command>, Reply),

    /// Anything else, named by the feature (`"card"`, `"gif"`, ...): runs in its `run` module.
    Other(&'static str, String),
    /// Keeping the feature's state current, named by the feature (`"guild"`,
    /// `"paint"`, ...): runs in its small `state` module, like events.
    State(&'static str, String),
}

/// A command's result: decoded by the core, or the feature's own result
/// payload for the feature to decode.
pub(super) enum Reply {
    Core(CommandResult),
    Raw(Raw),
}

/// A feature's module could not be downloaded (offline, or a deploy removed it).
pub(crate) struct Unavailable;
impl From<leptos::wasm_split::SplitLoaderError> for Unavailable {
    fn from(_: leptos::wasm_split::SplitLoaderError) -> Self {
        Self
    }
}

/// `Ok(false)` means the server sent something the feature cannot decode.
pub(super) type Outcome = Result<bool, Unavailable>;

/// Keeping a feature's state current (its events, the answers to its commands,
/// the fresh snapshot) runs in the feature's small `state` module; everything a
/// person does with it in its `run` module. The first screen then loads only
/// the state of the features it shows.
fn is_state(call: &Call) -> bool {
    matches!(
        call,
        Call::Event(_) | Call::Ready | Call::Answer(..) | Call::State(..)
    )
}
fn dispatch(
    app: Shared,
    f: Feature,
    call: Call,
) -> std::pin::Pin<Box<dyn Future<Output = Outcome>>> {
    match (f, is_state(&call)) {
        (Feature::Content, true) => Box::pin(super::content::state(app, call)),
        (Feature::Content, false) => Box::pin(super::content::run(app, call)),
        (Feature::Threads, true) => Box::pin(super::threads::state(app, call)),
        (Feature::Threads, false) => Box::pin(super::threads::run(app, call)),
        (Feature::Expression, true) => Box::pin(super::expression::state(app, call)),
        (Feature::Expression, false) => Box::pin(super::expression::run(app, call)),
        (Feature::Moderation, true) => Box::pin(super::moderation::state(app, call)),
        (Feature::Moderation, false) => Box::pin(super::moderation::run(app, call)),
        (Feature::Privacy, true) => Box::pin(super::privacy::state(app, call)),
        (Feature::Privacy, false) => Box::pin(super::privacy::run(app, call)),
        (Feature::Nav, true) => Box::pin(super::nav::state(app, call)),
        (Feature::Nav, false) => Box::pin(super::nav::run(app, call)),
        (Feature::Profiles, _) => Box::pin(super::profiles::run(app, call)),
        (Feature::Authsec, _) => Box::pin(super::authsec::run(app, call)),
        (Feature::Community, true) => Box::pin(super::community::state(app, call)),
        (Feature::Community, false) => Box::pin(super::community::run(app, call)),
        (Feature::Stage, true) => Box::pin(super::stage::state(app, call)),
        (Feature::Stage, false) => Box::pin(super::stage::run(app, call)),
        (Feature::Themes, true) => Box::pin(super::themes::state(app, call)),
        (Feature::Themes, false) => Box::pin(super::themes::run(app, call)),
    }
}

/// Run a call and wait for it (the socket uses this for events).
pub(super) async fn run(app: Shared, f: Feature, call: Call) -> Outcome {
    dispatch(app, f, call).await
}

/// Run a call now if the feature is loaded, otherwise as soon as it is.
pub(super) fn spawn(app: Shared, f: Feature, call: Call) {
    let owner = Rc::downgrade(&app);
    let mut work = dispatch(app, f, call);
    // A loaded module answers on the first poll, so an action of a feature
    // in use runs synchronously, exactly as when it was in the core.
    let waker = futures_util::task::noop_waker();
    if let Poll::Ready(outcome) = work.as_mut().poll(&mut Context::from_waker(&waker)) {
        return report(&owner, outcome);
    }
    spawn_local(async move { report(&owner, work.await) });
}

fn report(owner: &std::rc::Weak<RefCell<App>>, outcome: Outcome) {
    if outcome.is_err() {
        if let Some(app) = owner.upgrade() {
            let mut a = app.borrow_mut();
            a.error = "That didn’t load. Check your connection and try again.".into();
            a.touch();
        }
    }
}

/// Download a feature's module ahead of its first use (on hover or focus of
/// its entry point). Never on a metered or slow connection.
pub(super) fn prefetch(f: Feature) {
    if !fast_connection() {
        return;
    }
    spawn_local(async move {
        let _ = match f {
            Feature::Content => super::content::prefetch().await,
            Feature::Threads => super::threads::prefetch().await,
            Feature::Expression => super::expression::prefetch().await,
            Feature::Moderation => super::moderation::prefetch().await,
            Feature::Privacy => super::privacy::prefetch().await,
            Feature::Nav => super::nav::prefetch().await,
            Feature::Profiles => super::profiles::prefetch().await,
            Feature::Authsec => super::authsec::prefetch().await,
            Feature::Community => super::community::prefetch().await,
            Feature::Stage => super::stage::prefetch().await,
            Feature::Themes => super::themes::prefetch().await,
        };
    });
}

pub(super) fn fast_connection() -> bool {
    let Ok(connection) = js_sys::Reflect::get(&window().navigator(), &"connection".into()) else {
        return true;
    };
    if connection.is_undefined() || connection.is_null() {
        return true;
    }
    let get = |key: &str| js_sys::Reflect::get(&connection, &key.into()).ok();
    let save_data = get("saveData").and_then(|v| v.as_bool()).unwrap_or(false);
    let effective = get("effectiveType").and_then(|v| v.as_string());
    let metered = get("type")
        .and_then(|v| v.as_string())
        .is_some_and(|t| t == "cellular");
    !save_data && !metered && effective.is_none_or(|t| t == "4g")
}

/// Like [`spawn`], but never synchronously: for callers that hold the app
/// borrowed (rendering, event handlers).
pub(super) fn defer(app: Shared, f: Feature, call: Call) {
    let owner = Rc::downgrade(&app);
    spawn_local(async move { report(&owner, run(app, f, call).await) });
}
