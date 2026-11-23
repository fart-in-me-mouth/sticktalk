//! Views for rich content: polls, forwarded messages, link previews,
//! attachments (grid, spoilers, descriptions, voice messages), the GIF picker,
//! forwarding, scheduling and the composer's extra tools.
use super::*;
use crate::live::content::{local_input, presets, when};

/// "1:05" for a length in milliseconds.
fn clock(ms: u64) -> String {
    let s = ms.div_ceil(1000);
    format!("{}:{:02}", s / 60, s % 60)
}
fn left(ms: u64) -> String {
    let now = Browser.unix_ms();
    if ms <= now {
        return "Ended".into();
    }
    let minutes = (ms - now).div_ceil(60_000);
    match minutes {
        0..=59 => format!("{minutes}m left"),
        60..=1439 => format!("{}h left", minutes.div_ceil(60)),
        _ => format!("{}d left", minutes / 1440),
    }
}

/// The poll in a message: answers with live bars, your choice, the winner.
pub(super) fn poll(a: &App, m: &Message, p: &Poll) -> AnyView {
    let total = p.answers.iter().map(|v| v.votes).sum::<u32>().max(1);
    let winners = p.winners();
    let voted = !p.my_votes.is_empty();
    let show = voted || p.closed || a.content.results_shown.contains(&m.id);
    let own = a
        .model
        .me
        .as_ref()
        .is_some_and(|me| me.user.id == m.author_id);
    let can_end =
        !p.closed && (own || (a.guild.is_some() && settings::can(a, Permissions::MANAGE_MESSAGES)));
    view!{<section class="poll mt-1 w-full max-w-[min(100%,440px)] rounded-lg border bg-card p-4" aria-label=format!("Poll: {}",p.question)>
        <h3 class="poll-question text-base font-semibold wrap-anywhere">{p.question.clone()}</h3>
        <p class="mt-1 mb-3 text-[13px] text-subtle">{if p.multiple{"Select one or more answers"}else{"Select one answer"}}</p>
        <div class="poll-answers flex flex-col gap-2" role="group" aria-label="Answers">{p.answers.iter().enumerate().map(|(i,answer)|{
            let mine=p.my_votes.contains(&(i as u8));
            let won=winners.contains(&i);
            let share=answer.votes*100/total;
            let label=format!("{}{}{}",answer.text,if show{format!(", {} vote{}, {share}%",answer.votes,if answer.votes==1{""}else{"s"})}else{String::new()},if won{", winner"}else{""});
            view!{<div class="poll-answer relative flex min-h-11 items-center overflow-hidden rounded-md border" data-poll-answer=i.to_string()>
                {if show{view!{<span class=format!("poll-bar absolute inset-y-0 left-0 {}",if won{"bg-primary/30"}else if mine{"bg-primary/20"}else{"bg-accent"}) style=format!("width:{share}%") aria-hidden="true"></span>}.into_any()}else{().into_any()}}
                <button class=format!("relative flex min-h-11 flex-1 items-center gap-2 px-3 py-2 text-left text-sm {}",if mine{"font-semibold text-foreground"}else{"text-foreground"}) data-live=format!("poll-vote:{},{i}",m.id.0) disabled=p.closed aria-pressed=mine.to_string() aria-label=label>
                    <span class=format!("grid size-5 flex-none place-items-center border-2 {} {}",if p.multiple{"rounded-sm"}else{"rounded-full"},if mine{"border-primary bg-primary text-primary-foreground"}else{"border-muted-foreground"}) aria-hidden="true">{if mine{icon_with("check","icon size-3.5")}else{().into_any()}}</span>
                    {answer.emoji.clone().map(|e|view!{<span aria-hidden="true">{super::expression::emoji_text(a,&e)}</span>})}
                    <span class="min-w-0 flex-1 wrap-anywhere">{answer.text.clone()}</span>
                    {if won{view!{<span class="poll-winner rounded-sm bg-primary px-1.5 text-xs font-semibold text-primary-foreground">"Winner"</span>}.into_any()}else{().into_any()}}
                </button>
                {if show{view!{<button class="relative flex-none px-3 text-[13px] text-subtle hover:text-foreground hover:underline touch:min-h-11" data-live=format!("poll-voters:{},{i}",m.id.0) aria-label=format!("Show who voted for {}",answer.text)>{format!("{}%",share)}</button>}.into_any()}else{().into_any()}}
            </div>}
        }).collect_view()}</div>
        <div class="poll-footer mt-3 flex flex-wrap items-center gap-x-3 gap-y-1 text-[13px] text-subtle">
            <span>{format!("{} vote{}",p.voters,if p.voters==1{""}else{"s"})}</span><span aria-hidden="true">"·"</span>
            <span>{if p.closed{"Poll closed".to_owned()}else{left(p.expires_at)}}</span>
            {if voted&&!p.closed{view!{<button class="link-button text-[13px] touch:min-h-11" data-live=format!("poll-vote:{},{}",m.id.0,p.my_votes[0])>{if p.multiple{"Remove a vote"}else{"Remove vote"}}</button>}.into_any()}else if !p.closed{view!{<button class="link-button text-[13px] touch:min-h-11" data-live=format!("poll-results:{}",m.id.0) aria-pressed=show.to_string()>{if show{"Hide results"}else{"Show results"}}</button>}.into_any()}else{().into_any()}}
            {if can_end{view!{<button class="link-button ml-auto text-[13px] touch:min-h-11" data-live=format!("poll-end:{}",m.id.0)>"End Poll Now"</button>}.into_any()}else{().into_any()}}
        </div>
    </section>}.into_any()
}

/// A forwarded message: label, snapshot and (for readers of the source) a link back.
pub(super) fn forwarded(a: &App, m: &Message, f: &ForwardedMessage) -> AnyView {
    let source = f.source.as_ref().and_then(|s| {
        a.model.channels.get(&s.channel_id).map(|c| {
            let place = match c.guild_id.and_then(|g| a.model.guilds.get(&g)) {
                Some(g) => format!("#{} · {}", a.model.channel_name(c), g.guild.name),
                None => a.model.channel_name(c),
            };
            (
                place,
                format!("jump-message:{},{}", s.channel_id.0, s.message_id.0),
            )
        })
    });
    view!{<div class="forwarded mt-1 max-w-[min(100%,560px)] border-l-4 border-border pl-3">
        <p class="forwarded-label flex items-center gap-1 text-[13px] text-subtle italic">{icon_with("forward","icon size-4")}"Forwarded"</p>
        {markdown(a,m,&f.content)}
        {attachments(a,&f.attachments)}
        <p class="mt-1 flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">{match source{Some((place,jump))=>view!{<button class="forward-source hover:text-foreground hover:underline touch:min-h-11" data-live=jump title="Go to the original message">{place}</button><span aria-hidden="true">"·"</span>}.into_any(),None=>().into_any()}}<time>{super::timestamp(f.created_at)}</time></p>
    </div>}.into_any()
}

/// Website preview cards. Hidden by the "Show link previews" setting (CSS).
pub(super) fn embeds(a: &App, m: &Message) -> AnyView {
    if m.extra.embeds.is_empty() {
        return ().into_any();
    }
    let own = a
        .model
        .me
        .as_ref()
        .is_some_and(|me| me.user.id == m.author_id);
    let remove = own || (a.guild.is_some() && settings::can(a, Permissions::MANAGE_MESSAGES));
    view!{<div class="link-embeds mt-1 flex flex-col gap-2">{m.extra.embeds.iter().map(|e|{
        let host=sticktalk_client_core::urls::parse(&e.url).map(|u|u.host).unwrap_or_default();
        view!{<article class="link-embed relative grid max-w-[min(100%,432px)] gap-1 rounded-md border-l-4 border-primary bg-card p-3 pr-10">
            // The real host always; a page's own site name only beside it (G2).
            <p class="text-xs text-subtle">{match e.site_name.clone().filter(|n|!n.trim().is_empty()&&!n.eq_ignore_ascii_case(&host)){Some(n)=>format!("{host} · {n}"),None=>host.clone()}}</p>
            {e.title.clone().map(|t|view!{<a class="font-semibold text-primary wrap-anywhere hover:underline" href=e.url.clone() target="_blank" rel="noopener noreferrer nofollow" data-link-host=host.clone() title=host.clone()>{t}</a>})}
            {e.description.clone().map(|d|view!{<p class="text-sm text-foreground wrap-anywhere">{d}</p>})}
            {e.image.clone().map(|i|view!{<a href=e.url.clone() target="_blank" rel="noopener noreferrer nofollow" data-link-host=host.clone() title=host.clone() class="mt-1 block max-w-full overflow-hidden rounded-md" tabindex="-1"><img class="block h-auto max-h-[300px] max-w-full object-contain" src=i.url loading="lazy" width=i.width height=i.height alt=""/></a>})}
            {if e.video{view!{<a class="mt-1 inline-flex items-center gap-1 text-sm text-primary hover:underline" href=e.url.clone() target="_blank" rel="noopener noreferrer nofollow" data-link-host=host.clone() title=host.clone()>{icon_with("play","icon size-4")}"Watch video"</a>}.into_any()}else{().into_any()}}
            {if remove{view!{<button class="link-embed-remove absolute top-2 right-2 grid size-7 place-items-center rounded-sm text-muted-foreground hover:bg-accent hover:text-foreground touch:size-11" data-live=format!("embeds-remove:{}",m.id.0) aria-label="Remove link previews" title="Remove link previews">{icon_with("x","icon size-4")}</button>}.into_any()}else{().into_any()}}
        </article>}
    }).collect_view()}</div>}.into_any()
}

/// A GIF sent from the picker, inline, with a favorite star.
pub(super) fn gif(a: &App, m: &Message) -> Option<AnyView> {
    let url = m.content.trim();
    let media = url.starts_with("https://static.klipy.com/")
        || url.starts_with("https://static1.klipy.com/")
        || url.starts_with("https://static2.klipy.com/");
    if !media
        || url.contains(char::is_whitespace)
        || matches!(a.gif_capability, GifCapability::Disabled)
    {
        return None;
    }
    let video = url.ends_with(".mp4") || url.ends_with(".webm");
    let id = m.extra.gif.clone();
    let starred = id
        .as_ref()
        .is_some_and(|g| a.content.gif_favorites.contains(g));
    Some(view!{<div class="chat-gif group/gif relative mt-1 inline-block min-h-12 min-w-12 max-w-[min(100%,360px)] overflow-hidden rounded-lg bg-card">
        {if video{view!{<video class="block max-h-[300px] max-w-full" src=url.to_owned() autoplay muted loop playsinline preload="metadata" aria-label="GIF"></video>}.into_any()}else{view!{<img class="block h-auto max-h-[300px] max-w-full" src=url.to_owned() alt="GIF" loading="lazy"/>}.into_any()}}
        {id.map(|g|star(&g,starred,"absolute top-2 right-2 hidden group-hover/gif:grid group-focus-within/gif:grid touch:grid"))}
    </div>}.into_any())
}
fn star(id: &str, starred: bool, place: &str) -> AnyView {
    view!{<button class=format!("gif-star size-8 place-items-center rounded-full bg-black/60 text-white hover:bg-black/80 touch:size-11 {place} {}",if starred{"is-starred"}else{""}) data-live=format!("gif-star:{id}") aria-pressed=starred.to_string() aria-label=if starred{"Remove from Favorites"}else{"Add to Favorites"} title=if starred{"Remove from Favorites"}else{"Add to Favorites"}>{icon_with("star","icon size-4")}</button>}.into_any()
}

/// Attachments: images and videos in a grid sized for their count (1 to 10),
/// files and voice messages below. Spoilers stay covered until clicked.
pub(super) fn attachments(a: &App, files: &[Attachment]) -> AnyView {
    if files.is_empty() {
        return ().into_any();
    }
    let visual = |f: &Attachment| {
        !f.meta.voice()
            && ((f.content_type.starts_with("image/") && f.thumbnail.is_some())
                || crate::wire::video_hint(&f.content_type, &f.filename))
    };
    let (media, others): (Vec<&Attachment>, Vec<&Attachment>) =
        files.iter().partition(|f| visual(f));
    let cells = media.len();
    // Columns, and how many leading cells are wider: one full-width cell above
    // the rest (3, 7, 10) or two half-width cells (5, 8), as in Discord.
    let (grid, wide) = match cells {
        0 | 1 => ("", 0),
        2 | 4 => ("grid-cols-2", 0),
        3 => ("grid-cols-2", 1),
        5 | 8 => ("grid-cols-6", 2),
        7 | 10 => ("grid-cols-3", 1),
        _ => ("grid-cols-3", 0),
    };
    view!{
        {if media.is_empty(){().into_any()}else{view!{<div class=format!("attachment-media mt-2 max-w-[min(100%,520px)] {}",if cells>1{format!("attachment-grid grid gap-1 {grid}")}else{String::new()}) data-count=cells.to_string()>{media.into_iter().enumerate().map(|(i,f)|{
            let span=match (grid,i<wide) {
                ("grid-cols-6",true)=>"col-span-3",
                ("grid-cols-6",false)=>"col-span-2",
                ("grid-cols-2",true)=>"col-span-2 wide",
                ("grid-cols-3",true)=>"col-span-3 wide",
                _=>"",
            };
            one(a,f,cells>1,span)
        }).collect_view()}</div>}.into_any()}}
        {others.into_iter().map(|f|one(a,f,false,"")).collect_view()}
    }.into_any()
}

fn one(a: &App, f: &Attachment, cell: bool, span: &str) -> AnyView {
    let body = if f.meta.voice() {
        voice(f)
    } else if cell {
        grid_cell(f, span.ends_with("wide"))
    } else {
        super::attachment(f)
    };
    let alt = f.meta.description.clone().filter(|d| !d.trim().is_empty());
    let shown = a.content.alt_shown.contains(&f.id);
    let alt_view = alt.map(|d| view!{<div class="attachment-alt mt-1 flex flex-col items-start gap-1"><button class="rounded-sm bg-rail px-1.5 py-0.5 text-[11px] font-bold tracking-wide text-foreground hover:bg-accent touch:min-h-11" data-live=format!("alt-toggle:{}",f.id.0) aria-expanded=shown.to_string() aria-label=if shown{"Hide description"}else{"Show description"} title="Description">"ALT"</button>{if shown{view!{<p class="max-w-[420px] rounded-md bg-card px-2 py-1 text-[13px] text-subtle wrap-anywhere">{d}</p>}.into_any()}else{().into_any()}}</div>});
    if f.meta.spoiler && !a.content.revealed.contains(&f.id) {
        let kind = if f.content_type.starts_with("image/") {
            "image"
        } else if crate::wire::video_hint(&f.content_type, &f.filename) {
            "video"
        } else if f.meta.voice() {
            "voice message"
        } else {
            "file"
        };
        return view!{<div class=format!("attachment-spoiler relative min-w-0 overflow-hidden rounded-lg {}",if span.is_empty(){"mt-2 w-fit max-w-full [&>.spoiler-content>*]:mt-0"}else{span}) data-attachment=f.id.0.to_string()><div class="spoiler-content pointer-events-none blur-2xl select-none" aria-hidden="true" inert>{body}</div><button class="spoiler-cover absolute inset-0 grid min-h-11 place-items-center bg-black/40 text-sm font-bold tracking-wide text-white uppercase hover:bg-black/30" data-live=format!("spoiler-reveal:{}",f.id.0) aria-label=format!("Spoiler: reveal {kind} {}",f.filename)>"Spoiler"</button></div>}.into_any();
    }
    view!{<div class=format!("attachment-item min-w-0 {span}") data-attachment=f.id.0.to_string()>{body}{alt_view}</div>}.into_any()
}

/// An image or video filling a grid cell.
fn grid_cell(f: &Attachment, wide: bool) -> AnyView {
    let shape = if wide {
        "aspect-[2/1]"
    } else {
        "aspect-square"
    };
    let original = format!("/api/v1/files/{}", f.id.0);
    let alt = f
        .meta
        .description
        .clone()
        .unwrap_or_else(|| f.filename.clone());
    if f.content_type.starts_with("image/") {
        let src = if f.content_type == "image/gif" && f.byte_size <= 8 * 1024 * 1024 {
            original.clone()
        } else {
            format!("{original}/thumbnail")
        };
        view!{<button class=format!("attachment-image block {shape} w-full overflow-hidden rounded-md bg-card p-0") data-grid-cell="true" data-view-image=original data-filename=f.filename.clone() data-bytes=f.byte_size.to_string() aria-label=format!("Open {}",f.filename)><img class="block size-full object-cover" src=src loading="lazy" alt=alt/></button>}.into_any()
    } else {
        view!{<video class=format!("block {shape} w-full rounded-md bg-black object-cover") controls playsinline preload="none" src=original aria-label=alt></video>}.into_any()
    }
}

/// A voice message: play/pause, waveform (stored at record time; click to
/// seek), length and speed. Playback lives in `public/content.js`.
fn voice(f: &Attachment) -> AnyView {
    let ms = u64::from(f.meta.duration_ms.unwrap_or(0));
    let samples = &f.meta.waveform;
    const BARS: usize = 40;
    let bars = (0..BARS).map(|i| {
        let start = i * samples.len() / BARS;
        let end = ((i + 1) * samples.len() / BARS)
            .max(start + 1)
            .min(samples.len());
        let peak = samples
            .get(start..end)
            .and_then(|s| s.iter().max())
            .copied()
            .unwrap_or(0);
        12 + u32::from(peak) * 88 / 255
    });
    let src = format!("/api/v1/files/{}", f.id.0);
    view!{<div class="voice-message mt-2 flex w-[min(100%,360px)] items-center gap-3 rounded-full bg-card py-2 pr-4 pl-2" data-voice=f.id.0.to_string() data-src=src.clone() data-duration=ms.to_string() role="group" aria-label=format!("Voice message, {}",clock(ms))>
        <button class="voice-play grid size-10 flex-none place-items-center rounded-full bg-primary text-primary-foreground hover:bg-primary-hover touch:size-11" data-content="voice-play" aria-label="Play voice message"><span class="voice-icon-play contents">{icon_with("play","icon size-4")}</span><span class="voice-icon-pause hidden">{icon_with("pause","icon size-4")}</span></button>
        <div class="voice-wave flex h-8 min-w-0 flex-1 cursor-pointer items-center gap-[2px]" data-content="voice-seek" role="slider" tabindex="0" aria-label="Seek" aria-valuemin="0" aria-valuemax=ms.div_ceil(1000).to_string() aria-valuenow="0" aria-valuetext=format!("0:00 of {}",clock(ms))>{bars.map(|h|view!{<span class="voice-bar min-w-[2px] flex-1 rounded-full bg-muted-foreground/60" style=format!("height:{h}%")></span>}).collect_view()}</div>
        <span class="voice-time w-10 flex-none text-right text-xs text-subtle tabular-nums">{clock(ms)}</span>
        <button class="voice-speed flex-none rounded-sm px-1 text-xs font-semibold text-subtle hover:text-foreground touch:min-h-11" data-content="voice-speed" aria-label="Playback speed 1×">"1×"</button>
        <a class="sr-only" href=src.clone() download=f.filename.clone()>{format!("Download {}",f.filename)}</a>
    </div>}.into_any()
}

/// A file waiting in the composer, with its description and spoiler controls.
pub(super) fn attachment_chip(a: &App, i: usize, f: &Attachment) -> AnyView {
    let meta = a.content.meta.get(&f.id).cloned().unwrap_or_default();
    let image = f.content_type.starts_with("image/") && f.thumbnail.is_some();
    view!{<div class="live-upload mb-2 flex items-center gap-3 rounded-lg bg-muted p-2 text-xs" data-pending=f.id.0.to_string()>
        {if image{view!{<span class=format!("relative size-10 flex-none overflow-hidden rounded-md {}",if meta.spoiler{"[&>img]:blur-sm"}else{""})><img class="size-full object-cover" src=format!("/api/v1/files/{}/thumbnail",f.id.0) alt=""/></span>}.into_any()}else{icon("file")}}
        <span class="flex min-w-0 flex-1 flex-col"><span class="truncate" title=f.filename.clone()>{f.filename.clone()}</span><small class="truncate text-xs text-subtle">{match (&meta.description,meta.spoiler){(Some(d),true)=>format!("Spoiler · {d}"),(Some(d),false)=>d.clone(),(None,true)=>"Spoiler".into(),(None,false)=>crate::wire::file_size(f.byte_size)}}</small></span>
        <button class=format!("icon-button {}",if meta.spoiler{"text-primary"}else{""}) aria-pressed=meta.spoiler.to_string() aria-label=if meta.spoiler{"Unmark Spoiler"}else{"Mark as Spoiler"} title=if meta.spoiler{"Unmark Spoiler"}else{"Mark as Spoiler"} data-live=format!("attachment-spoiler:{}",f.id.0)>{icon("eye-off")}</button>
        {ib_plain("pencil","Edit Description",&format!("dialog:attachment:{}",f.id.0))}
        {ib_plain("x","Remove Attachment",&format!("remove-attachment:{i}"))}
    </div>}.into_any()
}

/// "2 scheduled messages" above the composer, when there are any here.
pub(super) fn scheduled_bar(a: &App, ch: Id) -> AnyView {
    let here = a
        .content
        .scheduled
        .iter()
        .filter(|s| s.channel_id == ch)
        .count();
    if here == 0 {
        return ().into_any();
    }
    let failed = a
        .content
        .scheduled
        .iter()
        .any(|s| s.channel_id == ch && s.failure.is_some());
    view!{<button class=format!("scheduled-bar mb-1 flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-[13px] hover:bg-accent touch:min-h-11 {}",if failed{"text-warning"}else{"text-subtle"}) data-live="dialog:scheduled">{icon_with("clock","icon size-4")}<span class="flex-1">{format!("{here} scheduled message{}{}",if here==1{""}else{"s"},if failed{" · one couldn’t be sent"}else{""})}</span><span class="text-primary">"View"</span></button>}.into_any()
}

/// The composer's extra tools: more (poll, send later, scheduled), and the mic.
pub(super) fn composer_tools(a: &App) -> AnyView {
    let _ = a;
    view!{
        <ui-menu class="contents"><ui-menu-trigger><button type="button" class="icon-button composer-more" aria-label="More Options" title="More Options">{icon("more-horizontal")}</button></ui-menu-trigger>
            <ui-menu-content class="menu" side="top" align="end" side-offset="8" hidden=true>
                // A narrow composer (phones, the chat beside a call) keeps room to
                // type: these buttons move here from the composer (ui.rs).
                <ui-menu-item class="menu-item @min-[32rem]/composer:hidden" value="dialog:gif">{icon("image")}"GIFs"</ui-menu-item>
                <ui-menu-item class="menu-item @min-[32rem]/composer:hidden" value="dialog:stickers">{icon("sticker")}"Stickers"</ui-menu-item>
                <ui-menu-item class="menu-item @min-[32rem]/composer:hidden" value="dialog:kaomoji">{icon("smile")}"Kaomoji"</ui-menu-item>
                <ui-menu-item class="menu-item" value="dialog:poll">{icon("chart-column")}"Create Poll"</ui-menu-item>
                <ui-menu-item class="menu-item" value="dialog:schedule">{icon("clock")}"Send Later…"</ui-menu-item>
                <ui-menu-item class="menu-item" value="dialog:scheduled">{icon("message-square")}"Scheduled Messages"</ui-menu-item>
            </ui-menu-content>
        </ui-menu>
        <button type="button" class="icon-button voice-record" data-content="voice-record" aria-label="Record Voice Message" title="Record Voice Message (click to start and stop; hold on a phone)">{icon("mic")}</button>
        <input type="file" id="live-voice-file" hidden/>
    }.into_any()
}

/// Dialogs of this module: (body, accessible label, extra class).
pub(super) fn dialog(a: &App) -> Option<(AnyView, &'static str, &'static str)> {
    let d = a.dialog.as_str();
    Some(match d {
        "poll" => (poll_form(a), "Create Poll", ""),
        "schedule" => (schedule_form(), "Send Later", ""),
        "scheduled" => (scheduled_list(a), "Scheduled Messages", ""),
        "gif" if !matches!(a.gif_capability, GifCapability::Disabled) => {
            (gif_picker(a), "GIFs", "gif-picker")
        }
        _ if d.starts_with("scheduled-edit:") => (
            scheduled_edit(a, id(&d[15..])?),
            "Edit Scheduled Message",
            "",
        ),
        _ if d.starts_with("forward:") => (forward_picker(a, id(&d[8..])?), "Forward Message", ""),
        _ if d.starts_with("attachment:") => {
            (attachment_form(a, id(&d[11..])?), "Attachment Details", "")
        }
        _ if d.starts_with("poll-voters:") => (voters(a), "Poll Voters", ""),
        _ => return None,
    })
}

fn poll_form(a: &App) -> AnyView {
    let n = a
        .content
        .poll_answers
        .clamp(MIN_POLL_ANSWERS, MAX_POLL_ANSWERS);
    let durations = [
        (1, "1 hour"),
        (4, "4 hours"),
        (8, "8 hours"),
        (24, "24 hours"),
        (72, "3 days"),
        (168, "1 week"),
        (336, "2 weeks"),
    ];
    view!{<form data-live-form="poll" class="poll-form"><h2>"Create a Poll"</h2>
        <div class="field"><label for="poll-question">"Question"</label><input id="poll-question" type="text" maxlength=MAX_POLL_QUESTION_CHARS.to_string() placeholder="What question do you want to ask?" required/></div>
        <fieldset class="my-4 flex flex-col gap-2"><legend class="mb-2 text-[13px] font-medium text-subtle">"Answers"</legend>
            {(0..MAX_POLL_ANSWERS).map(|i|{let hide=i>=n;view!{<div class="poll-answer-row flex items-center gap-2" hidden=hide>
                <label class="sr-only" for=format!("poll-emoji-{i}")>{format!("Emoji for answer {}",i+1)}</label><input class="w-12 flex-none text-center" id=format!("poll-emoji-{i}") type="text" maxlength="40" placeholder="🙂" autocomplete="off"/>
                <label class="sr-only" for=format!("poll-answer-{i}")>{format!("Answer {}",i+1)}</label><input class="min-w-0 flex-1" id=format!("poll-answer-{i}") type="text" maxlength=MAX_POLL_ANSWER_CHARS.to_string() placeholder="Type your answer"/>
                {if n>MIN_POLL_ANSWERS{view!{<button type="button" class="icon-button flex-none" data-live=format!("poll-answer-remove:{i}") aria-label=format!("Remove answer {}",i+1)>{icon("x")}</button>}.into_any()}else{().into_any()}}
            </div>}}).collect_view()}
            {if n<MAX_POLL_ANSWERS{view!{<button type="button" class="button link-button self-start" data-live="poll-answer-add">{icon_with("plus","icon size-4")}"Add another answer"</button>}.into_any()}else{().into_any()}}
        </fieldset>
        <label class="field"><span>"Duration"</span><select id="poll-duration">{durations.into_iter().map(|(h,label)|view!{<option value=h.to_string() selected=h==24>{label}</option>}).collect_view()}</select></label>
        <label class="live-checkbox"><span>"Allow multiple answers"</span><input id="poll-multiple" type="checkbox"/></label>
        <button type="submit" class="button primary mt-4">"Post"</button>
    </form>}.into_any()
}

fn schedule_form() -> AnyView {
    let now = Browser.unix_ms();
    view!{<form data-live-form="schedule"><h2>"Send Later"</h2><p class="muted-copy">"Your message is sent at the time you choose, even if you’re offline. You’ll be told if it can’t be sent."</p>
        <div class="my-4 flex flex-col gap-2">{presets().into_iter().map(|(label,at)|view!{<button type="button" class="button secondary justify-between" data-live=format!("schedule-at:{at}")><span>{label}</span><small class="text-subtle">{when(at)}</small></button>}).collect_view()}</div>
        <div class="field"><label for="schedule-at">"Or pick a date and time"</label><input id="schedule-at" type="datetime-local" min=local_input(now+60_000) max=local_input(now+MAX_SCHEDULE_AHEAD_MS) value=local_input(now+3_600_000)/></div>
        <button type="submit" class="button primary mt-4">"Schedule Message"</button>
    </form>}.into_any()
}

fn scheduled_list(a: &App) -> AnyView {
    let here: Vec<&ScheduledMessage> = a
        .content
        .scheduled
        .iter()
        .filter(|s| Some(s.channel_id) == a.channel)
        .collect();
    view!{<h2>"Scheduled Messages"</h2>
        {if here.is_empty(){view!{<div class=SETTINGS_EMPTY>{icon("clock")}<p>"Nothing scheduled here."</p><small>"Open More Options in the composer and choose Send Later."</small></div>}.into_any()}else{().into_any()}}
        <ul class="scheduled-list m-0 flex list-none flex-col gap-3 p-0">{here.into_iter().map(|s|view!{<li class="scheduled-item rounded-lg border bg-card p-3" data-scheduled=s.id.0.to_string()>
            <p class=format!("flex items-center gap-1 text-[13px] {}",if s.failure.is_some(){"text-warning"}else{"text-subtle"})>{icon_with("clock","icon size-4")}{s.failure.clone().map_or_else(||when(s.send_at),|f|format!("Not sent: {f}"))}</p>
            <p class="mt-1 whitespace-pre-wrap wrap-anywhere">{crate::wire::snippet(&a.model.content(&Message{id:s.id,channel_id:s.channel_id,author_id:Id(0),content:s.content.clone(),created_at:0,edited_at:None,reply_to:None,attachments:vec![],mention_ids:s.mention_ids.clone(),reactions:vec![],call:None,extra:Default::default()}))}</p>
            {(!s.attachments.is_empty()).then(||view!{<p class="scheduled-files mt-1 flex items-center gap-1 text-[13px] text-subtle wrap-anywhere">{icon_with("paperclip","icon size-4 shrink-0")}{s.attachments.iter().map(|f|f.filename.clone()).collect::<Vec<_>>().join(", ")}</p>})}
            <div class="mt-2 flex flex-wrap gap-2">{button("Edit",&format!("dialog:scheduled-edit:{}",s.id.0),"secondary")}{button("Send Now",&format!("scheduled-send:{}",s.id.0),"secondary")}{button("Cancel",&format!("scheduled-cancel:{}",s.id.0),"danger-text")}</div>
        </li>}).collect_view()}</ul>
    }.into_any()
}

fn scheduled_edit(a: &App, s: Id) -> AnyView {
    let Some(s) = a.content.scheduled.iter().find(|v| v.id == s) else {
        return view! {<p class="muted-copy">"That scheduled message has already been sent or cancelled."</p>}.into_any();
    };
    let now = Browser.unix_ms();
    let at = if s.send_at > now {
        s.send_at
    } else {
        now + 3_600_000
    };
    view!{<form data-live-form="scheduled-edit"><h2>"Edit Scheduled Message"</h2>
        <label class="field"><span>"Message"</span><textarea id="scheduled-text" rows="4" maxlength=MAX_MESSAGE_CHARS.to_string()>{s.content.clone()}</textarea></label>
        <div class="field"><label for="scheduled-at">"Send at"</label><input id="scheduled-at" type="datetime-local" min=local_input(now+60_000) max=local_input(now+MAX_SCHEDULE_AHEAD_MS) value=local_input(at)/></div>
        <button type="submit" class="button primary mt-4">{if s.failure.is_some(){"Reschedule"}else{"Save Changes"}}</button>
        <button type="button" class="button secondary mt-2 ml-2 phone:ml-0 phone:w-full" data-live="dialog:scheduled">"Back"</button>
    </form>}.into_any()
}

fn attachment_form(a: &App, f: Id) -> AnyView {
    let Some(file) = a.attachments.iter().find(|v| v.id == f) else {
        return view! {<p class="muted-copy">"That file is no longer in your message."</p>}
            .into_any();
    };
    let meta = a.content.meta.get(&f).cloned().unwrap_or_default();
    view!{<form data-live-form="attachment-meta"><h2>"Attachment Details"</h2><p class="muted-copy wrap-anywhere">{file.filename.clone()}</p>
        <label class="field"><span>"Description (alt text)"</span><textarea id="attachment-description" rows="3" maxlength=MAX_ATTACHMENT_DESCRIPTION_CHARS.to_string() aria-describedby="alt-help">{meta.description.clone().unwrap_or_default()}</textarea><small id="alt-help">"Describe it for people who use screen readers. Up to 1,024 characters."</small></label>
        <label class="live-checkbox"><span>"Mark as spoiler"</span><input id="attachment-spoiler" type="checkbox" checked=meta.spoiler/></label>
        <button type="submit" class="button primary mt-4">"Save"</button>
    </form>}.into_any()
}

fn voters(a: &App) -> AnyView {
    let target = a
        .dialog
        .strip_prefix("poll-voters:")
        .and_then(|v| v.split_once(','));
    let answer = target.and_then(|(m, i)| {
        let p = a
            .channel
            .and_then(|c| a.model.messages.get(&c))
            .and_then(|ms| ms.get(&id(m)?))?
            .extra
            .poll
            .clone()?;
        Some((p.answers.get(i.parse::<usize>().ok()?)?.text.clone(), p))
    });
    let Some((label, _)) = answer else {
        return view! {<p class="muted-copy">"This poll is no longer available."</p>}.into_any();
    };
    let list = a.content.voters.as_ref();
    view!{<h2>{format!("Voted for “{label}”")}</h2>
        {match list{None=>view!{<p role="status" class="muted-copy">"Loading…"</p>}.into_any(),Some((_,_,users,_)) if users.is_empty()=>view!{<p class="muted-copy">"No votes yet."</p>}.into_any(),Some((m,i,users,more))=>view!{<ul class="m-0 flex list-none flex-col p-0">{users.iter().map(|u|view!{<li><button class="member-row flex min-h-11 w-full items-center gap-3 px-2 py-1 hover:bg-accent" data-live=format!("profile:{}",u.id.0)>{avatar(u,"small")}<span class="truncate">{name(u)}</span></button></li>}).collect_view()}</ul>{if *more{button("Load More",&format!("poll-voters-more:{},{i}",m.0),"link-button")}else{().into_any()}}}.into_any()}}
    }.into_any()
}

/// Where to forward: your conversations (servers, DMs, groups), searchable,
/// up to five at once, with an optional comment.
fn forward_picker(a: &App, m: Id) -> AnyView {
    let source = a
        .channel
        .and_then(|c| a.model.messages.get(&c))
        .and_then(|ms| ms.get(&m));
    let q = a.content.forward_query.to_lowercase();
    let mut rows: Vec<(&ChannelSummary, String, String)> = a
        .model
        .channels
        .values()
        .filter(|c| {
            matches!(
                c.kind,
                ChannelKind::Text
                    | ChannelKind::Direct
                    | ChannelKind::GroupDm
                    | ChannelKind::Announcement
                    | ChannelKind::Thread
            )
        })
        .map(|c| {
            let name = a.model.channel_name(c);
            let place = c.guild_id.and_then(|g| a.model.guilds.get(&g)).map_or_else(
                || {
                    if c.kind == ChannelKind::GroupDm {
                        "Group".to_owned()
                    } else {
                        "Direct Message".to_owned()
                    }
                },
                |g| g.guild.name.clone(),
            );
            (c, name, place)
        })
        .filter(|(_, name, place)| {
            q.is_empty() || name.to_lowercase().contains(&q) || place.to_lowercase().contains(&q)
        })
        .collect();
    rows.sort_by_key(|(c, name, place)| {
        (
            !a.content.forward_selected.contains(&c.id),
            c.guild_id.is_some(),
            place.to_lowercase(),
            name.to_lowercase(),
        )
    });
    rows.truncate(60);
    let picked = a.content.forward_selected.len();
    view!{<form data-live-form="forward" class="forward-form"><h2>"Forward To"</h2>
        {source.map(|s|view!{<div class="forward-preview mb-3 rounded-md border-l-4 border-border bg-card px-3 py-2 text-[13px] text-subtle"><span class="line-clamp-2 wrap-anywhere">{super::preview(a,s)}</span></div>})}
        <label class="search flex"><span class="sr-only">"Search conversations"</span><input class="w-full" id="forward-search" type="search" placeholder="Search channels and people" value=a.content.forward_query.clone() data-forward-query autocomplete="off"/></label>
        <p class="mt-2 text-xs text-subtle" role="status">{format!("{picked} of {MAX_FORWARD_DESTINATIONS} selected")}</p>
        <ul class="forward-list m-0 mt-1 flex max-h-[min(320px,40vh)] list-none flex-col overflow-y-auto p-0" aria-label="Conversations">{rows.into_iter().map(|(c,name,place)|{let on=a.content.forward_selected.contains(&c.id);view!{<li><button type="button" class=format!("forward-row flex min-h-11 w-full items-center gap-3 rounded-md px-2 py-1 text-left hover:bg-accent {}",if on{"bg-secondary"}else{""}) data-live=format!("forward-pick:{}",c.id.0) aria-pressed=on.to_string()>{icon_with(if c.guild_id.is_some(){"hash"}else if c.kind==ChannelKind::GroupDm{"users"}else{"at-sign"},"icon flex-none text-muted-foreground")}<span class="flex min-w-0 flex-1 flex-col"><span class="truncate text-sm font-medium">{name}</span><small class="truncate text-xs text-subtle">{place}</small></span><span class=format!("grid size-5 flex-none place-items-center rounded-sm border-2 {}",if on{"border-primary bg-primary text-primary-foreground"}else{"border-muted-foreground"}) aria-hidden="true">{if on{icon_with("check","icon size-3.5")}else{().into_any()}}</span></button></li>}}).collect_view()}</ul>
        <label class="field mt-3"><span>"Add a message (optional)"</span><textarea id="forward-comment" rows="2" maxlength=MAX_MESSAGE_CHARS.to_string()></textarea></label>
        <button type="submit" class="button primary mt-2" disabled=picked==0>{if picked>1{format!("Send to {picked}")}else{"Send".into()}}</button>
    </form>}.into_any()
}

/// GIF picker: browse categories (or trending), your favorites, search.
fn gif_picker(a: &App) -> AnyView {
    let tab = a.content.gif_tab.as_str();
    let favorites = tab == "favorites";
    let items: &[GifItem] = if favorites {
        &a.content.favorite_items
    } else {
        &a.gifs
    };
    let browse = tab.is_empty() && a.gifs.is_empty() && !a.content.gif_categories.is_empty();
    let starred = |g: &GifItem| a.content.gif_favorites.contains(&g.id);
    view!{<h2>"GIFs"</h2>
        <div class="picker-tabs mb-3 flex gap-2" role="group" aria-label="GIF sections"><button class=format!("button {}",if favorites{"link-button"}else{"secondary"}) aria-pressed=(!favorites).to_string() data-live="gif-tab:browse">"GIFs"</button><button class=format!("button {}",if favorites{"secondary"}else{"link-button"}) aria-pressed=favorites.to_string() data-live="gif-tab:favorites">{icon_with("star","icon size-4")}{format!("Favorites ({})",a.content.gif_favorites.len())}</button></div>
        {if favorites{().into_any()}else{view!{<form data-gif-search class="flex items-center gap-2"><label class="min-w-0 flex-1"><span class="sr-only">"Search GIFs"</span><input class="w-full" id="gif-query" type="search" placeholder="Search KLIPY"/></label><button class="button secondary flex-none" type="submit">"Search"</button></form>}.into_any()}}
        {if browse{view!{<div class="gif-categories mt-4 grid grid-cols-2 gap-2">{a.content.gif_categories.iter().map(|c|view!{<button class="gif-category relative aspect-[16/9] overflow-hidden rounded-md bg-card" data-live=format!("gif-category:{}",c.query) aria-label=format!("{} GIFs",c.title)><video class="absolute inset-0 size-full object-cover opacity-70" src=c.preview.url.clone() autoplay muted loop playsinline preload="metadata" aria-hidden="true"></video><span class="relative z-[1] grid size-full place-items-center bg-black/30 px-2 text-sm font-semibold text-white">{c.title.clone()}</span></button>}).collect_view()}</div>}.into_any()}else{().into_any()}}
        {if favorites&&a.content.gif_busy{view!{<p class="muted-copy mt-4" role="status">"Loading favorites…"</p>}.into_any()}else if favorites&&items.is_empty(){view!{<div class=SETTINGS_EMPTY>{icon("star")}<p>"No favorites yet."</p><small>"Star a GIF to keep it here."</small></div>}.into_any()}else{().into_any()}}
        <div class="live-gif-results mt-4 grid grid-cols-2 gap-2 [&_video]:max-w-full">{items.iter().map(|g|view!{<div class="gif-result group/gif relative overflow-hidden rounded-md bg-card"><button class="block w-full" data-live=format!("gif-send:{}",g.id) aria-label=format!("Send {}",g.title)><video class="block w-full" src=g.preview.url.clone() preload="metadata" autoplay muted loop playsinline width=g.preview.width height=g.preview.height aria-hidden="true"></video></button>{star(&g.id,starred(g),"absolute top-1 right-1 grid")}</div>}).collect_view()}</div>
        <p class="mt-4 text-right text-xs text-subtle">"Powered by KLIPY"</p>
    }.into_any()
}
