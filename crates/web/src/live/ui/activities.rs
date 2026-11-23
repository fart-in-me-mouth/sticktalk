//! Activity surfaces drawn by the app: the Activities button in the call
//! controls, "X started Y" in the call view, the place the activity's tile
//! takes in the call view (`public/activities.js` puts the frame there),
//! invite cards in chat, and the server's Activities settings page.
use super::*;

/// The call controls' Activities button (not on a Stage).
pub(super) fn controls(a: &App) -> AnyView {
    let Some(c) = a.voice_channel else {
        return ().into_any();
    };
    if crate::live::stage::is_stage(a, c) {
        return ().into_any();
    }
    let running = crate::live::activities::current(a).is_some();
    view! {<span class="relative inline-flex">{ib("sparkles","Activities","activity:launcher")}{if running{view!{<span class="pointer-events-none absolute top-0 right-0 size-3 rounded-full border-2 border-rail bg-primary" aria-label="An activity is running"></span>}.into_any()}else{().into_any()}}</span>}.into_any()
}

fn glyph(icon: &str) -> AnyView {
    if icon
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        icon_with(icon, "icon size-5")
    } else {
        view! {<span class="text-xl leading-none" aria-hidden="true">{icon.to_owned()}</span>}
            .into_any()
    }
}

/// Above the people in a call: the call's activity for someone not in it yet.
pub(super) fn banner(a: &App) -> AnyView {
    let (Some(c), Some(s)) = (a.voice_channel, crate::live::activities::current(a)) else {
        return ().into_any();
    };
    if s.you.is_some() || a.activities.dismissed == Some(s.session_id) {
        return ().into_any();
    }
    let starter = a.model.name(s.started_by);
    let text = format!("{starter} started {}", s.activity.name);
    let people = s.participants.len();
    view!{<div class="activity-banner mx-auto mb-2 flex w-full max-w-xl flex-none items-center gap-3 rounded-lg border bg-card px-3 py-2 text-sm" role="status">
        <span class="grid size-9 flex-none place-items-center rounded-md bg-muted text-primary">{glyph(&s.activity.icon)}</span>
        <span class="min-w-0 flex-1"><strong class="block truncate" title=text.clone()>{text.clone()}</strong><small class="block text-subtle">{format!("{people} {} in it",if people==1{"person"}else{"people"})}</small></span>
        <button class="button primary h-8 px-3 text-sm touch:min-h-11" data-live=format!("activity:join,{},{}",c.0,s.session_id.0)>"Join"</button>
        {ib_plain("x","Dismiss","activity:dismiss")}
    </div>}.into_any()
}

/// Where the activity's tile goes in the call view while this client takes
/// part, and the conversations an invite can be posted in.
pub(super) fn slot(a: &App) -> AnyView {
    let (Some(c), Some(s)) = (a.voice_channel, crate::live::activities::current(a)) else {
        return ().into_any();
    };
    if s.you.is_none() {
        return ().into_any();
    }
    let layout = if a.activities.layout.is_empty() {
        "split"
    } else {
        a.activities.layout
    };
    let guild = a.model.channels.get(&c).and_then(|ch| ch.guild_id);
    let mut targets: Vec<(Id, String)> = vec![];
    if let Some(ch) = a.model.channels.get(&c) {
        targets.push((c, a.model.channel_name(ch)));
    }
    if guild.is_some() {
        let mut rest: Vec<&ChannelSummary> = a
            .model
            .channels
            .values()
            .filter(|ch| {
                ch.id != c
                    && (ch.guild_id == guild
                        && matches!(
                            ch.kind,
                            ChannelKind::Text | ChannelKind::Voice | ChannelKind::Announcement
                        )
                        || matches!(ch.kind, ChannelKind::Direct | ChannelKind::GroupDm))
            })
            .collect();
        rest.sort_by_key(|ch| (ch.guild_id.is_none(), ch.position, ch.id));
        targets.extend(
            rest.into_iter()
                .take(40)
                .map(|ch| (ch.id, a.model.channel_name(ch))),
        );
    }
    view!{<div id="live-activity-slot" class="activity-slot" data-layout=layout data-session=s.session_id.0.to_string() data-call=c.0.to_string() aria-hidden="true"></div>
        <ul id="live-activity-targets" hidden>{targets.into_iter().map(|(id,label)|view!{<li data-channel=id.0.to_string() data-label=label></li>}).collect_view()}</ul>}.into_any()
}

fn minutes(ms: u64) -> String {
    match ms / 60_000 {
        0 => "under a minute".into(),
        1 => "1 minute".into(),
        n => format!("{n} minutes"),
    }
}

/// An invite card in chat (the message's `extra.activity`). Its renderer is
/// a split module loaded with the first card on screen; until then the card
/// is a placeholder of its size.
pub(super) fn card(a: &App, m: &Message) -> AnyView {
    if m.extra.activity.is_none() {
        return ().into_any();
    }
    match lazy_view(a, |a| &a.views.activity_card, || Box::pin(load_activity_card())) {
        Some(render) => render(a, m),
        None => view!{<div class="activity-card mt-1 min-h-[66px] max-w-[min(100%,440px)] rounded-lg border bg-card" role="status" aria-label="Loading activity"></div>}.into_any(),
    }
}
#[lazy]
pub(in crate::live) fn load_activity_card() -> fn(&App, &Message) -> AnyView {
    card_view
}
fn card_view(a: &App, m: &Message) -> AnyView {
    let Some(card) = m.extra.activity.as_ref().and_then(|c| c.get()) else {
        return ().into_any();
    };
    let here = a.voice_channel == Some(card.call_channel_id);
    let joined = here
        && crate::live::activities::current(a)
            .is_some_and(|s| s.session_id == card.session_id && s.you.is_some());
    let detail = match (&card.summary, card.live) {
        (Some(s), _) => {
            let mut t = format!(
                "Ended · {} {} · {}",
                s.people,
                if s.people == 1 { "person" } else { "people" },
                minutes(s.ended_at.saturating_sub(s.started_at))
            );
            if let Some(r) = &s.result {
                t.push_str(" · ");
                t.push_str(r);
            }
            t
        }
        (None, true) if card.people.is_empty() => "Starting…".into(),
        (None, true) => {
            let more = (card.count as usize).saturating_sub(card.people.len());
            let mut t = card.people.join(", ");
            if more > 0 {
                t.push_str(&format!(" and {more} more"));
            }
            format!("In it: {t}")
        }
        (None, false) => "Ended".into(),
    };
    let live = card.live && card.summary.is_none();
    view!{<div class="activity-card mt-1 flex max-w-[min(100%,440px)] items-center gap-3 rounded-lg border bg-card p-3 text-sm phone:flex-wrap" data-activity-card=card.session_id.0.to_string()>
        <span class=format!("grid size-10 flex-none place-items-center rounded-lg bg-muted {}",if live{"text-primary"}else{"text-subtle"})>{glyph(&card.icon)}</span>
        <div class="min-w-0 flex-1"><span class="flex items-center gap-2">{if live{view!{<span class="flex-none rounded bg-primary px-1.5 text-[11px] font-semibold whitespace-nowrap text-primary-foreground">"LIVE"</span>}.into_any()}else{().into_any()}}<strong class="truncate text-base">{card.name.clone()}</strong></span><small class="block text-subtle wrap-anywhere">{detail}</small></div>
        {if live&&!joined{view!{<button class="button primary h-9 gap-2 px-4 touch:min-h-11 phone:w-full" data-live=format!("activity:join,{},{}",card.call_channel_id.0,card.session_id.0) aria-label=format!("Join {}",card.name)>{icon_with("phone","icon size-4")}"Join"</button>}.into_any()}else if live{view!{<span class="text-xs text-primary">"You are in it"</span>}.into_any()}else{().into_any()}}
    </div>}.into_any()
}

/// Server Settings › Activities: filled in by `activities.js`.
pub(super) fn settings_page(a: &App) -> AnyView {
    let Some(g) = a.guild else {
        return ().into_any();
    };
    view!{<h2>"Activities"</h2><p class="muted-copy">"Choose which activities members can start in this server's voice channels. Use Activities in Roles decides who can start and join them."</p>
        <section class="activity-settings mt-4" data-activity-settings=g.0.to_string() aria-label="Activities"><p class="muted-copy" role="status">"Loading activities…"</p></section>}.into_any()
}

pub(super) fn is_server_page(page: &str) -> bool {
    page == "server-activities"
}
