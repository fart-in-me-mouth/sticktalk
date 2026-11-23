//! Surfaces for direct and group calls and for voice channel chat: the
//! incoming-call card, the call buttons in a conversation's header, the "call in
//! progress" bar, the record a finished call leaves, the people a call can ring,
//! the chat panel beside a call, and the push-to-talk button.
use super::*;

fn me(a: &App) -> Option<Id> {
    a.model.me.as_ref().map(|me| me.user.id)
}

fn user(a: &App, id: Id) -> UserSummary {
    a.model
        .users
        .get(&id)
        .cloned()
        .unwrap_or_else(|| UserSummary {
            id,
            username: a.model.name(id),
            ..Default::default()
        })
}

fn conversation(a: &App, c: Id) -> Option<&ChannelSummary> {
    a.model
        .channels
        .get(&c)
        .filter(|c| matches!(c.kind, ChannelKind::Direct | ChannelKind::GroupDm))
}

/// "Ana, Ben and 2 others".
fn names(a: &App, users: &[Id]) -> String {
    let named: Vec<String> = users.iter().take(2).map(|u| a.model.name(*u)).collect();
    match users.len() {
        0 => String::new(),
        1 => named[0].clone(),
        2 => format!("{} and {}", named[0], named[1]),
        n => format!("{}, {} and {} others", named[0], named[1], n - 2),
    }
}

/// The incoming call: who is calling, and Answer, Answer with Video, Decline.
pub(super) fn incoming(a: &App) -> AnyView {
    let Some(call) = crate::live::calls::incoming(a) else {
        return ().into_any();
    };
    let Some(channel) = conversation(a, call.channel_id) else {
        return ().into_any();
    };
    let caller = user(a, call.started_by);
    let group = channel.kind == ChannelKind::GroupDm;
    let title = if group {
        a.model.channel_name(channel)
    } else {
        name(&caller)
    };
    let detail = if group {
        format!("{} is calling the group", name(&caller))
    } else {
        "Incoming call".into()
    };
    let c = call.channel_id.0;
    view!{<section class="incoming-call fixed top-[calc(var(--safe-top)+16px)] left-1/2 z-[220] flex w-[min(380px,calc(100vw-32px))] -translate-x-1/2 flex-col items-center gap-4 rounded-xl border bg-popover p-6 text-popover-foreground shadow-lg phone:top-auto phone:bottom-0 phone:w-full phone:rounded-b-none phone:pb-[max(24px,var(--safe-bottom))]" role="alertdialog" aria-labelledby="incoming-call-title" aria-describedby="incoming-call-detail" data-call=c.to_string()>
        <span class="incoming-call-avatar rounded-full ring-2 ring-primary ring-offset-4 ring-offset-popover">{avatar(&caller,"large")}</span>
        <div class="flex min-w-0 max-w-full flex-col items-center gap-1 text-center"><h2 id="incoming-call-title" class="max-w-full truncate text-lg font-semibold" title=title.clone()>{title.clone()}</h2><p id="incoming-call-detail" class="m-0 text-sm text-subtle" aria-live="assertive">{detail}</p></div>
        <div class="incoming-call-actions flex w-full justify-center gap-4">
            <button class="incoming-decline grid size-14 place-items-center rounded-full bg-destructive text-primary-foreground hover:opacity-90" data-live=format!("call:decline,{c}") aria-label="Decline" title="Decline">{icon("phone-off")}</button>
            <button class="incoming-video grid size-14 place-items-center rounded-full bg-secondary text-foreground hover:bg-secondary-hover" data-live=format!("call:answer,{c},video") aria-label="Answer with Video" title="Answer with Video">{icon("video")}</button>
            <button class="incoming-answer grid size-14 place-items-center rounded-full bg-primary text-primary-foreground hover:bg-primary-hover" data-live=format!("call:answer,{c}") aria-label="Answer" title="Answer">{icon("phone")}</button>
        </div>
    </section>}.into_any()
}

/// Call buttons for a DM or group DM header: start a voice or video call, or
/// join the one in progress.
pub(super) fn header(a: &App, c: Id) -> AnyView {
    if conversation(a, c).is_none() || a.voice_channel == Some(c) {
        return ().into_any();
    }
    if a.calls.live.contains_key(&c) {
        return view!{<button class="button primary call-join h-8 gap-2 px-3 text-sm touch:min-h-11" data-live=format!("call:answer,{}",c.0)>{icon("phone")}<span class="phone:hidden">"Join Call"</span></button>}.into_any();
    }
    view! {{ib("phone","Start Voice Call",&format!("call:start,{}",c.0))}{ib("video","Start Video Call",&format!("call:start,{},video",c.0))}}.into_any()
}

/// Above a conversation's messages: the call in progress there, or, in a voice
/// channel's chat, who is connected and a way to join.
pub(super) fn banner(a: &App, c: Id) -> AnyView {
    let Some(channel) = a.model.channels.get(&c) else {
        return ().into_any();
    };
    if a.screen == "voice" {
        return ().into_any();
    }
    if channel.kind == ChannelKind::Stage {
        return super::stage::chat_banner(a, c);
    }
    if channel.kind == ChannelKind::Voice {
        let mut people: Vec<Id> = a
            .model
            .launch
            .voice
            .values()
            .filter(|v| v.channel_id == c)
            .map(|v| v.participant.session.user_id)
            .collect();
        people.sort();
        people.dedup();
        let here = a.voice_channel == Some(c);
        let text = if people.is_empty() {
            "No one is in this voice channel.".to_owned()
        } else {
            format!("In voice: {}", names(a, &people))
        };
        return view!{<div class="call-banner voice-chat-banner flex flex-none items-center gap-3 border-b bg-sidebar px-4 py-2 text-sm phone:px-2">{icon_with("volume-2","icon flex-none text-muted-foreground")}<span class="min-w-0 flex-1 truncate text-subtle" title=text.clone()>{text.clone()}</span>{if here{button("Return to Call",&format!("voice:{}",c.0),"secondary")}else{view!{<button class="button primary gap-2" data-live=format!("voice:{}",c.0)>{icon("volume-2")}"Join Voice"</button>}.into_any()}}</div>}.into_any();
    }
    let Some(call) = a.calls.live.get(&c) else {
        return ().into_any();
    };
    let mut people: Vec<Id> = call
        .participants
        .iter()
        .map(|p| p.session.user_id)
        .collect();
    people.sort();
    people.dedup();
    let here = a.voice_channel == Some(c);
    let text = if people.is_empty() {
        "Call starting…".to_owned()
    } else {
        format!("In the call: {}", names(a, &people))
    };
    view!{<div class="call-banner flex flex-none items-center gap-3 border-b bg-sidebar px-4 py-2 text-sm phone:flex-wrap phone:px-2" role="status">
        <span class="flex flex-none -space-x-2">{people.iter().take(4).map(|u|view!{<span class="rounded-full ring-2 ring-sidebar">{avatar(&user(a,*u),"small")}</span>}).collect_view()}</span>
        <span class="min-w-0 flex-1"><strong class="block text-primary">"Call in progress"</strong><small class="block truncate text-subtle" title=text.clone()>{text.clone()}</small></span>
        {if here{button("Return to Call",&format!("voice:{}",c.0),"secondary")}else{view!{<span class="flex gap-2">{button("Join with Video",&format!("call:answer,{},video",c.0),"secondary")}<button class="button primary gap-2" data-live=format!("call:answer,{}",c.0)>{icon("phone")}"Join Call"</button></span>}.into_any()}}
    </div>}.into_any()
}

fn duration(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => "less than a minute".into(),
        1 => "a minute".into(),
        n if n < 60 => format!("{n} minutes"),
        n => {
            let (h, m) = (n / 60, n % 60);
            let hours = if h == 1 {
                "1 hour".to_owned()
            } else {
                format!("{h} hours")
            };
            match m {
                0 => hours,
                1 => format!("{hours} 1 minute"),
                m => format!("{hours} {m} minutes"),
            }
        }
    }
}

/// The system message a finished call leaves, drawn as a call record rather
/// than as something its starter said.
pub(super) fn record(a: &App, m: &Message) -> Option<AnyView> {
    let call = m.call.as_ref()?;
    let starter = a.model.name(m.author_id);
    let mine = me(a) == Some(m.author_id);
    let (glyph, tone, text) = if call.missed {
        (
            "phone-missed",
            "text-destructive",
            if mine {
                "You started a call. No one answered.".to_owned()
            } else {
                format!("You missed a call from {starter}.")
            },
        )
    } else {
        (
            "phone",
            "text-primary",
            format!(
                "{} started a call that lasted {}.",
                if mine { "You".to_owned() } else { starter },
                duration(call.duration_ms())
            ),
        )
    };
    Some(view!{<article class="message call-record mt-4 flex items-center gap-4 px-4 py-1 text-[15px] text-subtle" data-live-message=m.id.0.to_string() tabindex="0" aria-label=text.clone()>
        <span class=format!("flex w-10 flex-none justify-center {tone}")>{icon(glyph)}</span>
        <span class="min-w-0 flex-1">{text.clone()}" "<time class="text-xs">{timestamp(m.created_at)}</time></span>
    </article>}.into_any())
}

/// Everyone a direct or group call shows a tile for, so their name and picture
/// are known before their media arrives.
pub(super) fn people(a: &App) -> Vec<Id> {
    let Some(c) = a.voice_channel else {
        return vec![];
    };
    let mut out: Vec<Id> = a.calls.live.get(&c).map_or(vec![], |call| {
        call.participants
            .iter()
            .map(|p| p.session.user_id)
            .collect()
    });
    if let Some(channel) = conversation(a, c) {
        out.extend(channel.recipient_ids.iter().copied());
    }
    out
}

/// In a direct or group call: the members who are not in it, each with a Ring
/// button (or "Ringing…" while their devices ring).
pub(super) fn ring_list(a: &App) -> AnyView {
    let Some(c) = a.voice_channel else {
        return ().into_any();
    };
    let (Some(channel), Some(call)) = (conversation(a, c), a.calls.live.get(&c)) else {
        return ().into_any();
    };
    let present: Vec<Id> = call
        .participants
        .iter()
        .map(|p| p.session.user_id)
        .collect();
    let others: Vec<Id> = channel
        .recipient_ids
        .iter()
        .copied()
        .filter(|u| Some(*u) != me(a) && !present.contains(u))
        .collect();
    if others.is_empty() {
        return ().into_any();
    }
    view!{<ul class="call-ring-list m-0 flex flex-none list-none flex-wrap justify-center gap-2 p-0 pb-2" aria-label="Not in the call">{others.into_iter().map(|u|{
        let person=user(a,u);
        let ringing=call.ringing.contains(&u);
        view!{<li class="flex min-h-11 items-center gap-2 rounded-full bg-muted py-1 pr-1 pl-1 text-sm">{avatar(&person,"small")}<span class="max-w-32 truncate">{name(&person)}</span>{if ringing{view!{<span class="ringing px-2 text-xs text-primary" role="status">"Ringing…"</span>}.into_any()}else{view!{<button class="button secondary h-8 min-h-8 gap-1 rounded-full px-3 text-xs touch:min-h-11" data-live=format!("call:ring,{},{}",c.0,u.0) aria-label=format!("Ring {}",name(&person))>{icon_with("phone","icon size-4")}"Ring"</button>}.into_any()}}</li>}
    }).collect_view()}</ul>}.into_any()
}

/// Call controls added by this module: the push-to-talk button (shown by the
/// media host only in push-to-talk mode) and the chat toggle.
pub(super) fn controls(a: &App) -> AnyView {
    let Some(c) = a.voice_channel else {
        return ().into_any();
    };
    let unread = a.model.reads.get(&c).is_some_and(|r| r.unread_count > 0);
    let open = a.calls.chat == Some(c);
    view!{
        <button id="live-ptt" type="button" class="ptt-button hidden" aria-label="Hold to talk" aria-pressed="false" hidden><svg class="icon" aria-hidden="true" viewBox="0 0 24 24"><use href="#i-mic"></use></svg><span class="ptt-label">"Hold to Talk"</span></button>
        <span class="relative inline-flex">{ib("message-square",if open{"Hide Chat"}else{"Show Chat"},&format!("call:chat,{}",c.0))}{if unread&&!open{view!{<span class="call-chat-unread pointer-events-none absolute top-0 right-0 size-3 rounded-full border-2 border-rail bg-destructive" aria-label="Unread messages"></span>}.into_any()}else{().into_any()}}</span>
    }.into_any()
}

/// The call screen, with its conversation's chat beside it (desktop) or over it
/// as a sheet (phone) while the chat is open.
pub(super) fn beside(a: &App, call: AnyView) -> AnyView {
    let open =
        a.calls.chat.is_some() && a.calls.chat == a.voice_channel && a.channel == a.voice_channel;
    if !open {
        return call;
    }
    let title = a
        .channel
        .and_then(|c| a.model.channels.get(&c))
        .map_or_else(String::new, |c| a.model.channel_name(c));
    view!{<div class="call-with-chat flex min-h-0 flex-1">{call}
        <aside class="call-chat flex w-[380px] flex-none flex-col border-l bg-background narrow:w-[320px] phone:fixed phone:inset-x-0 phone:bottom-0 phone:z-40 phone:h-[min(75vh,calc(var(--viewport-height)-var(--header-height)-var(--safe-top)))] phone:w-full phone:rounded-t-xl phone:border-t phone:border-l-0 phone:shadow-lg" aria-label=format!("{title} chat")>
            <header class="flex h-12 flex-none items-center gap-2 border-b px-4 phone:px-2"><span class="sheet-handle hidden phone:block phone:absolute phone:top-1 phone:left-1/2 phone:m-0 phone:-translate-x-1/2" aria-hidden="true"></span>{icon_with("message-square","icon text-muted-foreground")}<h2 class="min-w-0 flex-1 truncate text-base font-semibold" title=title.clone()>{title.clone()}</h2>{ib_plain("x","Close Chat","call:chat-close")}</header>
            {super::chat(a,&title)}
        </aside>
    </div>}.into_any()
}

/// On a voice channel's row: its chat, reachable without joining the call.
pub(super) fn row_chat(a: &App, c: &ChannelSummary) -> AnyView {
    if c.kind != ChannelKind::Voice {
        return ().into_any();
    }
    let selected = a.channel == Some(c.id) && a.calls.chat == Some(c.id);
    let label = format!("Open {} chat", a.model.channel_name(c));
    view!{<button class=format!("voice-row-chat mr-1 size-6 flex-none touch:size-11 place-items-center rounded-sm text-muted-foreground hover:text-foreground {}",if selected{"grid text-foreground"}else{"hidden group-focus-within:grid group-hover:grid coarse:grid"}) aria-label=label.clone() title=label data-live=format!("call:chat,{}",c.id.0)>{icon_with("message-square","icon size-4")}</button>}.into_any()
}
