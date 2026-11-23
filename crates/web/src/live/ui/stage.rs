//! Stage channels and scheduled events: the Stage view, the channel list's
//! live Stages and Events entry, the Start and Topic dialogs, the events
//! list, detail and form, and the cards in chat. State is in `live/stage.rs`.
use super::*;
use crate::live::stage::{can_create_events, moderator, my_entry, roster, when};

const LIVE_BADGE: &str = "stage-live inline-flex flex-none items-center rounded-sm bg-destructive px-1.5 py-0.5 text-[11px] leading-none font-bold tracking-wide text-primary-foreground";

/// The channel list icon for a Stage.
pub(super) fn kind_icon(kind: ChannelKind) -> Option<&'static str> {
    (kind == ChannelKind::Stage).then_some("audio-lines")
}

/// Under a live Stage's row: its topic and how many speak and listen.
pub(super) fn row_extra(a: &App, c: &ChannelSummary, depth: usize) -> AnyView {
    if c.kind != ChannelKind::Stage {
        return ().into_any();
    }
    let Some(live) = a.stage.live.get(&c.id) else {
        return ().into_any();
    };
    let label = format!(
        "Live: {}. {} speaking, {} listening.",
        live.topic, live.speakers, live.listeners
    );
    view!{<div class="stage-row-live flex min-w-0 items-center gap-2 py-1 pr-2 text-xs text-subtle" style=indent(depth+1,12) aria-label=label.clone() title=label>
        <span class=LIVE_BADGE>"LIVE"</span>
        <span class="min-w-0 flex-1 truncate">{live.topic.clone()}</span>
        <span class="flex flex-none items-center gap-1">{icon_with("mic","icon size-3")}{live.speakers.to_string()}</span>
        <span class="flex flex-none items-center gap-1">{icon_with("headphones","icon size-3")}{live.listeners.to_string()}</span>
    </div>}.into_any()
}

/// On a Stage's row: its chat, without joining.
pub(super) fn row_chat(a: &App, c: &ChannelSummary) -> AnyView {
    if c.kind != ChannelKind::Stage {
        return ().into_any();
    }
    let selected = a.channel == Some(c.id) && a.calls.chat == Some(c.id);
    let label = format!("Open {} chat", a.model.channel_name(c));
    view!{<button class=format!("voice-row-chat mr-1 size-6 flex-none touch:size-11 place-items-center rounded-sm text-muted-foreground hover:text-foreground {}",if selected{"grid text-foreground"}else{"hidden group-focus-within:grid group-hover:grid coarse:grid"}) aria-label=label.clone() title=label data-live=format!("call:chat,{}",c.id.0)>{icon_with("message-square","icon size-4")}</button>}.into_any()
}

/// The Events entry at the top of a server's channel list.
pub(super) fn events_entry(a: &App) -> AnyView {
    let Some(g) = a.guild else {
        return ().into_any();
    };
    let summary = a.stage.summaries.get(&g);
    let count = summary.map_or(0, |s| s.count);
    let live = summary.is_some_and(|s| !s.live.is_empty());
    if count == 0 && !can_create_events(a, g) {
        return ().into_any();
    }
    let label = match count {
        0 => "Events".to_owned(),
        1 => "Events, 1 event".to_owned(),
        n => format!("Events, {n} events"),
    };
    view!{<button class="nav-row stage-events-entry flex w-full items-center justify-start gap-2 rounded-md p-2 text-sm font-medium text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground touch:min-h-11 phone:text-base" data-live=format!("stage:events,{}",g.0) aria-label=label>
        {icon_with("clock","icon text-muted-foreground")}<span class="flex-1 text-left">"Events"</span>
        {if live{view!{<span class=LIVE_BADGE>"LIVE"</span>}.into_any()}else{().into_any()}}
        {if count>0{view!{<span class="live-unread quiet-count">{count.to_string()}</span>}.into_any()}else{().into_any()}}
    </button>}.into_any()
}

/// A Stage's chat, opened without joining: what is happening there.
pub(super) fn chat_banner(a: &App, c: Id) -> AnyView {
    let here = a.voice_channel == Some(c);
    let text = match a.stage.live.get(&c) {
        Some(l) => format!("Live: {} · {} listening", l.topic, l.listeners),
        None => "This Stage isn’t live.".to_owned(),
    };
    let join = if here {
        button("Return to Stage", &format!("voice:{}", c.0), "secondary")
    } else if a.stage.live.contains_key(&c) || moderator(a, c) {
        view!{<button class="button primary gap-2" data-live=format!("stage:open,{}",c.0)>{icon("headphones")}{if a.stage.live.contains_key(&c){"Join Stage"}else{"Start Stage"}}</button>}.into_any()
    } else {
        ().into_any()
    };
    view!{<div class="call-banner stage-chat-banner flex flex-none items-center gap-3 border-b bg-sidebar px-4 py-2 text-sm phone:px-2">{icon_with("audio-lines","icon flex-none text-muted-foreground")}<span class="min-w-0 flex-1 truncate text-subtle" title=text.clone()>{text.clone()}</span>{join}</div>}.into_any()
}

fn speaker_names(a: &App, ch: Id) -> Vec<(Id, String, Option<Id>)> {
    roster(a, ch)
        .map(|r| {
            r.members
                .iter()
                .map(|m| (m.user_id, m.name.clone(), m.avatar_id))
                .collect()
        })
        .unwrap_or_default()
}

/// The Stage screen. Speakers' tiles are the media host's (`#live-media-stage`);
/// the audience is a virtual list drawn by `stage.js` (`#stage-audience`).
pub(super) fn view(a: &App) -> AnyView {
    let Some(ch) = a.voice_channel else {
        return ().into_any();
    };
    let r = roster(a, ch);
    let instance = r.and_then(|r| r.instance.clone());
    let mine = my_entry(a).cloned();
    let speaking = mine.as_ref().is_some_and(|m| m.role == StageRole::Speaker);
    let moderating = mine.as_ref().is_some_and(|m| m.moderator) || moderator(a, ch);
    let raised = mine.as_ref().is_some_and(|m| m.hand_raised_at.is_some());
    let invited = mine.as_ref().is_some_and(|m| m.invited);
    let can_raise = sticktalk_client_core::launch::permissions(&a.model, ch)
        .contains(Permissions::REQUEST_TO_SPEAK);
    let speakers = r.map_or(0, |r| {
        r.members
            .iter()
            .filter(|m| m.role == StageRole::Speaker)
            .count()
    });
    let audience = r.map_or(0, |r| r.members.len() - speakers);
    let mut hands: Vec<&StageMember> = r
        .map(|r| {
            r.members
                .iter()
                .filter(|m| m.hand_raised_at.is_some())
                .collect()
        })
        .unwrap_or_default();
    hands.sort_by_key(|m| m.hand_raised_at);
    let people = speaker_names(a, ch);
    let name = a
        .model
        .channels
        .get(&ch)
        .map(|c| a.model.channel_name(c))
        .unwrap_or_default();
    let me = mine.as_ref().map(|m| m.user_id);
    let header = match &instance {
        Some(i) => view!{<header class="stage-header flex flex-none flex-wrap items-start gap-3 rounded-lg bg-panel p-4 phone:p-3">
            <div class="min-w-0 flex-1">
                <div class="flex items-center gap-2"><span class=LIVE_BADGE>"LIVE"</span><span class="truncate text-xs text-subtle">{name.clone()}</span></div>
                <h2 class="stage-topic m-0 mt-1 text-lg font-semibold wrap-anywhere">{i.topic.clone()}</h2>
                {i.privacy_note.clone().map_or_else(||().into_any(),|n|view!{<p class="stage-note m-0 mt-1 flex items-center gap-1 text-[13px] text-subtle">{icon_with("shield","icon size-3.5 flex-none")}<span class="wrap-anywhere">{n}</span></p>}.into_any())}
                <p class="m-0 mt-2 flex gap-4 text-[13px] text-subtle"><span class="flex items-center gap-1">{icon_with("mic","icon size-3.5")}{format!("{} speaking",i.speakers)}</span><span class="flex items-center gap-1">{icon_with("headphones","icon size-3.5")}{format!("{} listening",i.listeners)}</span></p>
                <p class="stage-e2ee m-0 mt-1 flex items-center gap-1 text-[13px] text-subtle" title="One-to-many audiences are not end-to-end encrypted; voice channels and calls are.">{icon_with("lock-open","icon size-3.5 flex-none")}"Stage channels aren’t end-to-end encrypted."</p>
            </div>
            {if moderating{view!{<div class="flex flex-none gap-2">{button("Edit Topic","stage:topic","secondary")}{button("End Stage","stage:end","danger")}</div>}.into_any()}else{().into_any()}}
        </header>}.into_any(),
        None => view!{<header class="stage-header flex flex-none flex-col items-start gap-2 rounded-lg bg-panel p-4">
            <h2 class="m-0 text-lg font-semibold">{format!("{name} isn’t live yet")}</h2>
            <p class="m-0 text-sm text-subtle">"You’re backstage. Start the Stage when you’re ready: you’ll be its first speaker."</p>
            {if moderating{button("Start Stage",&format!("stage:open,{}",ch.0),"primary")}else{().into_any()}}
        </header>}.into_any(),
    };
    let invite = if invited {
        view!{<div class="stage-invite flex flex-none flex-wrap items-center gap-3 rounded-lg border border-primary bg-panel p-3 text-sm" role="alertdialog" aria-labelledby="stage-invite-text">
            {icon_with("mic","icon flex-none text-primary")}<span id="stage-invite-text" class="min-w-0 flex-1">"You’re invited to speak. Your microphone stays off until you accept."</span>
            <span class="flex gap-2">{button("Not Now","stage:answer,no","secondary")}{button("Accept","stage:answer,yes","primary")}</span>
        </div>}.into_any()
    } else {
        ().into_any()
    };
    let queue = if moderating && !hands.is_empty() {
        let total = hands.len();
        view!{<section class="stage-requests flex flex-none flex-col gap-1 rounded-lg bg-panel p-3" aria-label="Requests to speak">
            <h3 class="category m-0 mb-1">{format!("Requests to Speak — {total}")}</h3>
            <ol class="m-0 flex max-h-48 list-none flex-col gap-1 overflow-y-auto p-0">{hands.iter().take(50).map(|m|{let u=m.user_id.0;view!{<li class="stage-request flex min-h-10 items-center gap-2 rounded-md px-2 hover:bg-sidebar-accent">{icon_with("star","icon size-4 flex-none text-warning")}<span class="min-w-0 flex-1 truncate">{m.name.clone()}</span><button class="button secondary h-8 px-3 text-sm" data-live=format!("stage:dismiss,{u}") aria-label=format!("Dismiss {}",m.name)>"Dismiss"</button><button class="button primary h-8 px-3 text-sm" data-live=format!("stage:accept,{u}") aria-label=format!("Let {} speak",m.name)>"Accept"</button></li>}}).collect_view()}</ol>
            {if total>50{view!{<p class="m-0 text-xs text-subtle">{format!("and {} more",total-50)}</p>}.into_any()}else{().into_any()}}
        </section>}.into_any()
    } else {
        ().into_any()
    };
    let controls = if speaking {
        view!{
            {ib("mic","Mute","media:mute")}{ib("headphones","Deafen","media:deafen")}{ib("video","Camera","media:camera")}
            <span class="desktop-share contents">{ib("monitor","Share Screen","media:share")}</span>
            <span class="contents phone:hidden">{ib("volume-2","Audio Devices","media:output")}</span>
            {call_views::controls(a)}{call_extras::controls(a)}
            {me.map_or_else(||().into_any(),|u|ib("arrow-down","Move to Audience",&format!("stage:audience,{}",u.0)))}
            {ib("phone-off","Leave Stage","media:leave")}
        }.into_any()
    } else {
        view!{
            {if instance.is_some()&&can_raise{if raised{ib("x","Lower Hand","stage:lower")}else{ib("star","Request to Speak","stage:raise")}}else{().into_any()}}
            {if moderating&&instance.is_some(){ib("mic","Become a Speaker","stage:speak")}else{().into_any()}}
            {ib("headphones","Deafen","media:deafen")}
            <span class="contents phone:hidden">{ib("volume-2","Audio Devices","media:output")}</span>
            {call_views::controls(a)}
            {ib("phone-off","Leave Stage","media:leave")}
        }.into_any()
    };
    view!{<div class="voice-view stage-view flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto bg-rail p-4 phone:p-2 phone:pb-[max(8px,var(--safe-bottom))]" data-call-channel=ch.0.to_string() data-stage-role=if speaking{"speaker"}else{"audience"}>
        <ul id="live-call-people" hidden>{people.into_iter().map(|(u,n,avatar)|view!{<li data-user=u.0.to_string() data-name=n data-avatar=avatar.map_or(String::new(),|id|format!("/api/v1/files/{}/thumbnail",id.0))></li>}).collect_view()}</ul>
        <button class="call-minimize flex min-h-11 flex-none items-center gap-2 px-2 text-[13px] text-subtle" data-live="media:minimize" aria-label="Minimize Stage">{icon("chevron-down")}<span>"Minimize"</span></button>
        {header}{invite}
        <h3 class="category m-0 flex-none">{format!("Speakers — {speakers}")}</h3>
        // A direct child of the view: call extras place their notices before it.
        <div id="live-media-stage" class="participant-grid stage-speakers min-h-[140px]" aria-label="Speakers"></div>
        {queue}
        <section class="stage-audience-section flex min-h-[160px] flex-1 flex-col gap-2" aria-label="Audience">
            <h3 class="category m-0">{format!("Audience — {audience}")}</h3>
            <div id="stage-audience" class="stage-audience min-h-[120px] flex-1"></div>
        </section>
        <div class="call-controls flex flex-none flex-wrap justify-center gap-3 p-3 phone:gap-2 phone:px-0 [&_.icon-button]:size-12 [&_.icon-button]:rounded-full [&_.icon-button]:bg-muted [&_[data-live='media:leave']]:w-20 [&_[data-live='media:leave']]:rounded-[32px] [&_[data-live='media:leave']]:bg-destructive [&_[data-live='media:leave']]:text-primary-foreground">{controls}</div>
        <p id="live-call-status" class="call-connection m-0 min-h-4 text-center text-[13px] text-subtle" role="status">"Connecting…"</p>
    </div>}.into_any()
}

// ---- dialogs -------------------------------------------------------------------------

pub(super) fn dialog(a: &App) -> Option<(AnyView, &'static str, &'static str)> {
    let d = a.dialog.as_str();
    Some(match d {
        "stage-start" => (start(a), "Start Stage", ""),
        "stage-topic" => (topic(a), "Edit Topic", ""),
        "events" => (events(a), "Events", "stage-events"),
        "event-form" => (event_form(a), "Event", ""),
        _ if d.starts_with("event:") => (detail(a), "Event", "stage-event"),
        _ => return None,
    })
}

fn submit_button(label: &str) -> AnyView {
    view! {<button type="submit" class="button primary mt-4">{label.to_owned()}</button>}.into_any()
}

fn start(a: &App) -> AnyView {
    let ch = a.stage.starting.unwrap_or(Id(0));
    let name = a
        .model
        .channels
        .get(&ch)
        .map(|c| a.model.channel_name(c))
        .unwrap_or_default();
    // Starting from an event's page links the Stage to that event.
    let event = a
        .stage
        .detail
        .as_ref()
        .filter(|d| d.event.location == (EventLocation::Stage { channel_id: ch }))
        .map(|d| (d.event.id, d.event.name.clone()));
    view!{<form data-live-form="stage-start" class="flex flex-col gap-3">
        <h2 class="m-0 text-xl font-semibold">"Start Stage"</h2>
        <p class="m-0 text-sm text-subtle">{format!("Everyone who can see {name} can listen. You’ll be the first speaker.")}</p>
        <label class="field">"Topic"<input id="stage-topic" required maxlength=MAX_STAGE_TOPIC_CHARS.to_string() value=event.as_ref().map(|e|e.1.clone()).unwrap_or_default() autocomplete="off"/></label>
        <label class="field">"Note for listeners (optional)"<input id="stage-note" maxlength=MAX_STAGE_NOTE_CHARS.to_string() placeholder="For example: This Stage is recorded." autocomplete="off"/><small>"Shown to everyone before and while they listen."</small></label>
        <input id="stage-event" type="hidden" value=event.map_or(String::new(),|e|e.0.0.to_string())/>
        {submit_button("Start Stage")}
    </form>}.into_any()
}

fn topic(a: &App) -> AnyView {
    let live = a.voice_channel.and_then(|c| a.stage.live.get(&c));
    view!{<form data-live-form="stage-topic" class="flex flex-col gap-3">
        <h2 class="m-0 text-xl font-semibold">"Edit Topic"</h2>
        <label class="field">"Topic"<input id="stage-topic" required maxlength=MAX_STAGE_TOPIC_CHARS.to_string() value=live.map(|l|l.topic.clone()).unwrap_or_default()/></label>
        <label class="field">"Note for listeners (optional)"<input id="stage-note" maxlength=MAX_STAGE_NOTE_CHARS.to_string() value=live.and_then(|l|l.privacy_note.clone()).unwrap_or_default()/></label>
        {submit_button("Save")}
    </form>}.into_any()
}

fn location_text(a: &App, l: &EventLocation, name: Option<&str>) -> (String, &'static str) {
    match l {
        EventLocation::External { place } => (place.clone(), "external-link"),
        EventLocation::Stage { channel_id } | EventLocation::Voice { channel_id } => {
            let n = name.map(str::to_owned).or_else(|| {
                a.model
                    .channels
                    .get(channel_id)
                    .map(|c| a.model.channel_name(c))
            });
            (
                n.unwrap_or_else(|| "A channel".into()),
                if matches!(l, EventLocation::Stage { .. }) {
                    "audio-lines"
                } else {
                    "volume-2"
                },
            )
        }
    }
}
fn repeat_text(r: EventRecurrence) -> &'static str {
    match r {
        EventRecurrence::None => "",
        EventRecurrence::Daily => "Repeats daily",
        EventRecurrence::Weekly => "Repeats weekly",
        EventRecurrence::EveryTwoWeeks => "Repeats every two weeks",
        EventRecurrence::Monthly => "Repeats monthly",
        EventRecurrence::Yearly => "Repeats yearly",
        EventRecurrence::Weekdays => "Repeats on weekdays",
    }
}
fn time_range(start: u64, end: Option<u64>) -> String {
    match end {
        Some(e) => format!("{} – {}", when(start), when(e)),
        None => when(start),
    }
}
fn cover(e_id: Id, version: Option<u32>, class: &'static str) -> AnyView {
    match version {
        Some(v) => view!{<img class=class src=format!("/api/v1/events/{}/cover?v={v}",e_id.0) alt="" loading="lazy"/>}.into_any(),
        None => ().into_any(),
    }
}

fn events(a: &App) -> AnyView {
    let g = a.guild.unwrap_or(Id(0));
    let page = a.stage.page.as_ref().filter(|p| p.0 == g).map(|p| &p.1);
    let create = page.is_some_and(|p| p.can_create) || can_create_events(a, g);
    let body = match page {
        None => view! {<p role="status">"Loading…"</p>}.into_any(),
        Some(p) if p.events.is_empty() => view!{<div class="flex flex-col items-start gap-3 py-6">{icon_with("clock","icon size-8 text-subtle")}<p class="m-0">"No upcoming events."</p><p class="m-0 text-sm text-subtle">"Events for Stages, voice channels and places outside StickTalk show up here."</p></div>}.into_any(),
        Some(p) => view!{<ul class="stage-event-list m-0 flex list-none flex-col gap-2 p-0">{p.events.iter().map(|e|{
            let (place,glyph)=location_text(a,&e.location,None);
            let interested=a.stage.mine.contains(&e.id);
            view!{<li class="stage-event-item flex gap-3 rounded-lg border bg-card p-3">
                {cover(e.id,e.cover,"hidden size-16 flex-none rounded-md object-cover sm:block")}
                <button class="min-w-0 flex-1 text-left" data-live=format!("stage:event,{},{}",e.id.0,e.occurrence)>
                    <span class="flex items-center gap-2 text-xs">{if e.status==EventStatus::Live{view!{<span class=LIVE_BADGE>"LIVE"</span>}.into_any()}else{view!{<span class="text-primary">{when(e.start_at)}</span>}.into_any()}}<span class="text-subtle">{repeat_text(e.recurrence)}</span></span>
                    <strong class="mt-1 block truncate text-base">{e.name.clone()}</strong>
                    <span class="mt-1 flex items-center gap-1 text-[13px] text-subtle">{icon_with(glyph,"icon size-3.5 flex-none")}<span class="truncate">{place}</span></span>
                </button>
                <div class="flex flex-none flex-col items-end justify-between gap-2">
                    <button class=format!("button h-8 gap-1 px-3 text-sm {}",if interested{"primary"}else{"secondary"}) data-live=format!("stage:interest,{},{}",e.id.0,if interested{0}else{1}) aria-pressed=interested.to_string()>{icon_with(if interested{"check"}else{"star"},"icon size-4")}"Interested"</button>
                    <small class="text-subtle">{format!("{} interested",e.interested)}</small>
                </div>
            </li>}}).collect_view()}</ul>}.into_any(),
    };
    view!{<div class="flex flex-col gap-4">
        <div class="flex items-center gap-3"><h2 class="m-0 flex-1 text-xl font-semibold">"Events"</h2>{if create{view!{<button class="button primary gap-2" data-live="stage:event-new">{icon("plus")}"Create Event"</button>}.into_any()}else{().into_any()}}</div>
        {body}
    </div>}.into_any()
}

fn detail(a: &App) -> AnyView {
    let Some(d) = a.stage.detail.as_ref() else {
        return view! {<p role="status">"Loading…"</p>}.into_any();
    };
    let e = &d.event;
    let (place, glyph) = location_text(a, &e.location, d.location_name.as_deref());
    let id = e.id.0;
    let interested = d.me_interested;
    let live = e.status == EventStatus::Live;
    let joinable = live && e.location.channel().is_some();
    let ics = format!("/api/v1/events/{id}/calendar.ics");
    let link = format!("{}/?event={id}", a.origin);
    let invite = a
        .stage
        .invite_link
        .as_ref()
        .filter(|(e, _)| e.0 == id)
        .map(|(_, l)| l.clone());
    let targets: Vec<&ChannelSummary> = a
        .model
        .channels
        .values()
        .filter(|c| {
            c.guild_id == Some(e.guild_id)
                && matches!(
                    c.kind,
                    ChannelKind::Text
                        | ChannelKind::Announcement
                        | ChannelKind::Voice
                        | ChannelKind::Stage
                )
        })
        .take(200)
        .collect();
    let people: Vec<UserSummary> = a
        .stage
        .interested
        .as_ref()
        .filter(|p| p.0 == e.id)
        .map(|p| p.1.users.clone())
        .unwrap_or_default();
    let status = match e.status {
        EventStatus::Live => view! {<span class=LIVE_BADGE>"LIVE"</span>}.into_any(),
        EventStatus::Cancelled => {
            view! {<span class="text-sm text-destructive">"Cancelled"</span>}.into_any()
        }
        EventStatus::Ended => view! {<span class="text-sm text-subtle">"Ended"</span>}.into_any(),
        EventStatus::Scheduled => ().into_any(),
    };
    let stage_event = matches!(e.location, EventLocation::Stage { .. });
    let manage = if d.can_manage {
        let occ = e.occurrence;
        view!{<div class="stage-event-manage flex flex-wrap gap-2 border-t pt-3">
            {button("Edit","stage:event-edit","secondary")}
            {if e.status==EventStatus::Scheduled{
                if stage_event{button("Start Stage",&format!("stage:open,{}",e.location.channel().map_or(0,|c|c.0)),"primary")}
                else if e.location.channel().is_some(){button("Start Event",&format!("stage:event-start,{id}"),"primary")}
                else{().into_any()}
            }else{().into_any()}}
            {if live{button("End Event",&format!("stage:event-end,{id}"),"secondary")}else{().into_any()}}
            {if e.status==EventStatus::Scheduled{button(if e.recurrence==EventRecurrence::None{"Cancel Event"}else{"Cancel This Event"},&format!("stage:event-cancel,{id},{occ},this"),"secondary")}else{().into_any()}}
            {if e.status==EventStatus::Scheduled&&e.recurrence!=EventRecurrence::None{button("Cancel This and Following",&format!("stage:event-cancel,{id},{occ},following"),"secondary")}else{().into_any()}}
            <button class="button danger" data-live=format!("stage:event-delete,{id}")>"Delete Event"</button>
            <div class="flex w-full flex-wrap items-end gap-2"><label class="field mb-0 min-w-48 flex-1">"Cover image (PNG, JPEG, WebP or GIF, up to 4 MB)"<input id="event-cover-file" type="file" accept="image/png,image/jpeg,image/webp,image/gif"/></label>{button("Upload Cover",&format!("stage:event-cover,{id}"),"secondary")}{if e.cover.is_some(){button("Remove Cover",&format!("stage:event-cover-remove,{id}"),"secondary")}else{().into_any()}}</div>
        </div>}.into_any()
    } else {
        ().into_any()
    };
    view!{<article class="stage-event-detail flex flex-col gap-3">
        {cover(e.id,e.cover,"max-h-48 w-full rounded-lg object-cover")}
        <div class="flex items-center gap-2 text-sm">{status}<span class="text-primary">{time_range(e.start_at,e.end_at)}</span></div>
        <h2 class="m-0 text-xl font-semibold wrap-anywhere">{e.name.clone()}</h2>
        <p class="m-0 flex items-center gap-1 text-sm text-subtle">{icon_with(glyph,"icon size-4 flex-none")}<span class="wrap-anywhere">{place}</span></p>
        {if e.recurrence!=EventRecurrence::None{view!{<p class="m-0 text-sm text-subtle">{format!("{} · {}",repeat_text(e.recurrence),e.time_zone)}</p>}.into_any()}else{().into_any()}}
        {if e.description.is_empty(){().into_any()}else{view!{<p class="m-0 text-sm whitespace-pre-wrap wrap-anywhere">{e.description.clone()}</p>}.into_any()}}
        <div class="flex flex-wrap gap-2">
            {if joinable{view!{<button class="button primary gap-2" data-live=format!("stage:event-join,{}",e.location.channel().map_or(0,|c|c.0))>{icon("headphones")}"Join"</button>}.into_any()}else{().into_any()}}
            {if matches!(e.status,EventStatus::Scheduled|EventStatus::Live){view!{<button class=format!("button gap-2 {}",if interested{"primary"}else{"secondary"}) data-live=format!("stage:interest,{id},{}",if interested{0}else{1}) aria-pressed=interested.to_string()>{icon(if interested{"check"}else{"star"})}"Interested"</button>}.into_any()}else{().into_any()}}
            <a class="button secondary gap-2" href=ics download="event.ics">{icon("download")}"Add to Calendar"</a>
            <button class="button secondary gap-2" data-copy=link.clone() aria-label="Copy event link">{icon("link")}"Copy Link"</button>
            <button class="button secondary gap-2" data-live=format!("stage:event-invite,{id}")>{icon("user-plus")}"Invite"</button>
        </div>
        {invite.map_or_else(||().into_any(),|l|view!{<div class="flex items-center gap-2 rounded-md bg-muted p-2 text-sm">{super::invite_text(&l,"min-w-0 flex-1 truncate")}<button class="button secondary h-8 px-3" data-copy=l>"Copy"</button></div>}.into_any())}
        <div class="flex flex-wrap items-end gap-2"><label class="field mb-0 min-w-48 flex-1">"Share to a channel"<select id="event-share-channel"><option value="">"Choose a channel"</option>{targets.into_iter().map(|c|view!{<option value=c.id.0.to_string()>{a.model.channel_name(c)}</option>}).collect_view()}</select></label>{button("Share",&format!("stage:event-share,{id}"),"secondary")}</div>
        {if !d.upcoming.is_empty(){view!{<section><h3 class="category m-0 mb-1">"Next"</h3><ul class="m-0 list-none p-0 text-sm text-subtle">{d.upcoming.iter().map(|o|view!{<li><button class="link-button" data-live=format!("stage:event,{id},{o}")>{when(*o)}</button></li>}).collect_view()}</ul></section>}.into_any()}else{().into_any()}}
        <section><h3 class="category m-0 mb-2">{format!("Interested — {}",e.interested)}</h3><ul class="m-0 flex list-none flex-wrap gap-2 p-0">{people.iter().map(|u|view!{<li class="flex items-center gap-1 text-sm">{avatar(u,"small")}<span>{name(u)}</span></li>}).collect_view()}</ul></section>
        {manage}
    </article>}.into_any()
}

fn event_form(a: &App) -> AnyView {
    let e = a.stage.editing.as_ref();
    let g = a.guild.unwrap_or(Id(0));
    let channels: Vec<&ChannelSummary> = a
        .model
        .channels
        .values()
        .filter(|c| {
            c.guild_id == Some(g) && matches!(c.kind, ChannelKind::Stage | ChannelKind::Voice)
        })
        .take(200)
        .collect();
    let (kind, channel, place) = match e.map(|e| &e.location) {
        Some(EventLocation::Stage { channel_id }) => ("stage", Some(*channel_id), String::new()),
        Some(EventLocation::Voice { channel_id }) => ("voice", Some(*channel_id), String::new()),
        Some(EventLocation::External { place }) => ("external", None, place.clone()),
        None => ("stage", None, String::new()),
    };
    let start = e.map_or_else(
        || {
            crate::live::stage::local_input(
                Browser.unix_ms() + 3_600_000 - Browser.unix_ms() % 900_000,
            )
        },
        |e| crate::live::stage::local_input(e.start_at),
    );
    let end = e
        .and_then(|e| e.end_at)
        .map(crate::live::stage::local_input)
        .unwrap_or_default();
    let repeat = e.map_or(EventRecurrence::None, |e| e.recurrence);
    let recurring = e.is_some_and(|e| e.recurrence != EventRecurrence::None);
    let opt = |value: &'static str, label: &'static str, on: bool| view! {<option value=value selected=on>{label}</option>};
    view!{<form data-live-form="event-form" class="flex flex-col gap-3">
        <h2 class="m-0 text-xl font-semibold">{if e.is_some(){"Edit Event"}else{"Create Event"}}</h2>
        <label class="field">"Name"<input id="event-name" required maxlength=MAX_EVENT_NAME_CHARS.to_string() value=e.map(|e|e.name.clone()).unwrap_or_default() autocomplete="off"/></label>
        <fieldset class="flex flex-col gap-2"><legend class="mb-1 text-sm font-semibold">"Where"</legend>
            <label class="field">"Location"<select id="event-location-kind">{opt("stage","A Stage channel",kind=="stage")}{opt("voice","A voice channel",kind=="voice")}{opt("external","Somewhere else",kind=="external")}</select></label>
            <label class="field">"Channel"<select id="event-channel"><option value="">"Choose a channel"</option>{channels.into_iter().map(|c|view!{<option value=c.id.0.to_string() selected=Some(c.id)==channel>{format!("{} ({})",a.model.channel_name(c),if c.kind==ChannelKind::Stage{"Stage"}else{"voice"})}</option>}).collect_view()}</select><small>"For a Stage or voice channel."</small></label>
            <label class="field">"Place or link"<input id="event-place" maxlength=MAX_EVENT_PLACE_CHARS.to_string() value=place placeholder="For example: Library, 2nd floor"/><small>"For somewhere else. These events need an end time."</small></label>
        </fieldset>
        <div class="flex flex-wrap gap-3">
            <label class="field min-w-48 flex-1">"Starts"<input id="event-start" type="datetime-local" required value=start/></label>
            <label class="field min-w-48 flex-1">"Ends (optional)"<input id="event-end" type="datetime-local" value=end/></label>
        </div>
        <p class="m-0 text-xs text-subtle">{format!("Times are in your time zone, {}.",crate::live::stage::zone())}</p>
        <label class="field">"Repeats"<select id="event-repeat">{opt("none","Does not repeat",repeat==EventRecurrence::None)}{opt("daily","Daily",repeat==EventRecurrence::Daily)}{opt("weekly","Weekly",repeat==EventRecurrence::Weekly)}{opt("every_two_weeks","Every two weeks",repeat==EventRecurrence::EveryTwoWeeks)}{opt("monthly","Monthly",repeat==EventRecurrence::Monthly)}{opt("yearly","Yearly",repeat==EventRecurrence::Yearly)}{opt("weekdays","Every weekday (Monday to Friday)",repeat==EventRecurrence::Weekdays)}</select></label>
        {if recurring{view!{<fieldset class="flex flex-col gap-1"><legend class="mb-1 text-sm font-semibold">"Apply changes to"</legend>
            <label class="live-checkbox flex items-center gap-2"><input type="radio" name="event-scope" id="event-scope-this" checked=true/>"This event"</label>
            <label class="live-checkbox flex items-center gap-2"><input type="radio" name="event-scope" id="event-scope-following"/>"This and following events"</label>
        </fieldset>}.into_any()}else{().into_any()}}
        <label class="field">"Description (optional)"<textarea id="event-description" rows="4" maxlength=MAX_EVENT_DESCRIPTION_CHARS.to_string()>{e.map(|e|e.description.clone()).unwrap_or_default()}</textarea></label>
        {submit_button(if e.is_some(){"Save"}else{"Create Event"})}
    </form>}.into_any()
}

// ---- chat ------------------------------------------------------------------------------

fn duration(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => "less than a minute".into(),
        1 => "1 minute".into(),
        n if n < 60 => format!("{n} minutes"),
        n => format!("{}h {}m", n / 60, n % 60),
    }
}

/// A finished Stage's record, or a shared event's card, in a message.
pub(super) fn message_extra(a: &App, m: &Message) -> AnyView {
    let Some(s) = m.extra.stage.as_deref() else {
        return ().into_any();
    };
    if let Some(r) = &s.record {
        return view!{<div class="stage-record mt-1 flex max-w-[min(100%,440px)] items-center gap-3 rounded-lg border bg-card p-3 text-sm">
            {icon_with("audio-lines","icon size-5 flex-none text-subtle")}
            <div class="min-w-0"><strong class="block truncate">{format!("Stage ended: {}",r.topic)}</strong>
            <small class="block text-subtle">{format!("{} · {} listening at most · {} {}",duration(r.duration_ms()),r.peak_listeners,r.speakers,if r.speakers==1{"speaker"}else{"speakers"})}</small></div>
        </div>}.into_any();
    }
    let Some(card) = &s.event else {
        return ().into_any();
    };
    let live = a
        .stage
        .summaries
        .get(&card.guild_id)
        .is_some_and(|g| g.live.contains(&card.event_id));
    let (place, glyph) = location_text(a, &card.location, card.location_name.as_deref());
    let id = card.event_id.0;
    view!{<div class="stage-event-card mt-1 flex max-w-[min(100%,440px)] flex-col gap-2 overflow-hidden rounded-lg border bg-card">
        {cover(card.event_id,card.cover,"max-h-32 w-full object-cover")}
        <div class="flex flex-col gap-1 p-3">
            <span class="flex items-center gap-2 text-xs">{if live{view!{<span class=LIVE_BADGE>"LIVE"</span>}.into_any()}else{view!{<span class="text-primary">{when(card.start_at)}</span>}.into_any()}}<span class="text-subtle">{repeat_text(card.recurrence)}</span></span>
            <strong class="truncate text-base">{card.name.clone()}</strong>
            <span class="flex items-center gap-1 text-[13px] text-subtle">{icon_with(glyph,"icon size-3.5 flex-none")}<span class="truncate">{place}</span></span>
            <span class="mt-1 flex gap-2">
                {if live&&card.location.channel().is_some(){view!{<button class="button primary h-8 gap-1 px-3 text-sm" data-live=format!("stage:event-join,{}",card.location.channel().map_or(0,|c|c.0))>{icon_with("headphones","icon size-4")}"Join"</button>}.into_any()}else{().into_any()}}
                <button class="button secondary h-8 px-3 text-sm" data-live=format!("stage:event,{id}")>"View Event"</button>
            </span>
        </div>
    </div>}.into_any()
}

/// In the Join dialog: the event this invite leads to.
pub(super) fn invite_event(a: &App) -> AnyView {
    let Some((code, e)) = a.stage.invite_event.as_ref() else {
        return ().into_any();
    };
    if *code != a.invite_wanted {
        return ().into_any();
    }
    let (place, glyph) = location_text(a, &e.event.location, e.event.location_name.as_deref());
    view!{<div class="invite-event flex flex-col gap-1 rounded-lg border bg-card p-3 text-sm" role="status">
        <span class="text-xs text-primary">{if e.status==EventStatus::Live{"Happening now".to_owned()}else{when(e.event.start_at)}}</span>
        <strong class="truncate">{e.event.name.clone()}</strong>
        <span class="flex items-center gap-1 text-subtle">{icon_with(glyph,"icon size-3.5")}{place}</span>
        <small class="text-subtle">"Joining marks you as interested."</small>
    </div>}.into_any()
}
