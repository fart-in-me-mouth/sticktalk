//! Views for threads, forum channels and announcement channels: thread rows in
//! the sidebar, the header's thread controls, the preview under a message a
//! thread started from, crossposted announcements, the forum view and the
//! dialogs (start a thread, Threads browser, members, forum posts and
//! settings, following).
use super::*;
use crate::live::threads::{joined, manage, rights, thread};

const ROW: &str = "thread-row channel-row group flex min-h-[30px] items-center rounded-md";

/// "5m ago", "3h ago", "2d ago".
fn ago(ms: u64) -> String {
    let now = Browser.unix_ms();
    let s = now.saturating_sub(ms) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}
fn count(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
fn archive_label(m: u32) -> &'static str {
    match m {
        60 => "1 hour",
        1440 => "24 hours",
        4320 => "3 days",
        _ => "1 week",
    }
}
fn archive_select(selected: u32) -> AnyView {
    view!{<div class="field"><label for="thread-archive">"Hide after inactivity"</label><select id="thread-archive">{THREAD_ARCHIVE_MINUTES.iter().map(|m|view!{<option value=m.to_string() selected=*m==selected>{archive_label(*m)}</option>}).collect_view()}</select></div>}.into_any()
}

/// The sidebar icon for a channel kind this feature adds.
pub(super) fn kind_icon(kind: ChannelKind) -> Option<&'static str> {
    match kind {
        ChannelKind::Announcement => Some("megaphone"),
        ChannelKind::Forum => Some("messages-square"),
        ChannelKind::Thread => Some("message-square-text"),
        _ => None,
    }
}

/// Threads you joined (and the one open) under their channel in the sidebar.
pub(super) fn rows(a: &App, c: &ChannelSummary, depth: usize) -> AnyView {
    if !matches!(
        c.kind,
        ChannelKind::Text | ChannelKind::Announcement | ChannelKind::Forum
    ) {
        return ().into_any();
    }
    let mut list: Vec<&ThreadInfo> = a
        .threads
        .threads
        .values()
        .filter(|t| {
            t.parent_id == c.id && !t.archived && (joined(a, t.id) || a.channel == Some(t.id))
        })
        .collect();
    if list.is_empty() {
        return ().into_any();
    }
    list.sort_by_key(|t| std::cmp::Reverse(t.last_activity_at));
    list.truncate(25);
    view!{<ul class="thread-rows m-0 list-none p-0" aria-label=format!("Threads in {}",a.model.channel_name(c))>{list.into_iter().map(|t|{
        let selected=a.channel==Some(t.id);
        let read=a.model.reads.get(&t.id);
        let unread=read.is_some_and(|r|r.unread_count>0);
        let mentions=read.map_or(0,|r|r.mention_count);
        view!{<li class=format!("{ROW} {}",if selected{"selected bg-secondary"}else{"hover:bg-sidebar-accent"}) style=indent(depth+1,12)>
            <span class="ml-3 h-4 w-3 flex-none rounded-bl-md border-b-2 border-l-2 border-border" aria-hidden="true"></span>
            <button class=format!("channel-main flex min-w-0 flex-1 items-center gap-2 px-2 py-1 text-sm touch:min-h-11 phone:text-base {}",if selected||unread{"font-semibold text-foreground"}else{"font-medium text-sidebar-foreground"}) title=t.name.clone() data-live=format!("thread-open:{}",t.id.0)>
                <span class="truncate">{t.name.clone()}</span>
                {if mentions>0{view!{<span class="live-unread">{mentions.to_string()}</span>}.into_any()}else if unread{view!{<span class="live-unread quiet" aria-label="Unread messages"></span>}.into_any()}else{().into_any()}}
            </button>
        </li>}
    }).collect_view()}</ul>}.into_any()
}

/// The title row for a thread: its parent, then its name.
pub(super) fn title(a: &App) -> Option<AnyView> {
    let t = thread(a, a.channel?)?;
    let parent = a
        .model
        .channels
        .get(&t.parent_id)
        .map(|c| a.model.channel_name(c))
        .unwrap_or_default();
    Some(view!{<div class="thread-title flex min-w-0 flex-1 items-center gap-1">
        <button class="max-w-[40%] truncate text-base font-semibold text-subtle hover:text-foreground phone:hidden" data-live=format!("thread-parent:{}",t.id.0) title=format!("Back to #{parent}")>{format!("#{parent}")}</button>
        {icon_with("chevron-right","icon size-4 flex-none text-muted-foreground phone:hidden")}
        <h1 class="min-w-0 truncate text-base font-semibold" title=t.name.clone()>{t.name.clone()}</h1>
        {if t.private{view!{<span class="flex-none text-muted-foreground" title="Private thread" aria-label="Private thread">{icon_with("lock","icon size-4")}</span>}.into_any()}else{().into_any()}}
        {if t.archived{view!{<span class="thread-state ml-1 flex-none rounded-sm bg-muted px-1.5 text-xs text-subtle">"Archived"</span>}.into_any()}else{().into_any()}}
        {if t.locked{view!{<span class="thread-state ml-1 flex-none rounded-sm bg-muted px-1.5 text-xs text-subtle">"Locked"</span>}.into_any()}else{().into_any()}}
    </div>}.into_any())
}

/// One entry of a header menu: icon, label, action, destructive.
type Item = (&'static str, String, String, bool);

/// A "More" menu. `class` decides where it shows (all widths, or phones only).
fn more_menu(label: &str, class: &'static str, items: Vec<Item>) -> AnyView {
    if items.is_empty() {
        return ().into_any();
    }
    view!{<div class=class><ui-menu class="contents"><ui-menu-trigger><button class="icon-button" aria-label=label.to_owned() title=label.to_owned()>{icon("more-horizontal")}</button></ui-menu-trigger>
        <ui-menu-content class="menu" side="bottom" align="end" side-offset="6" hidden=true>
            {items.into_iter().map(|(glyph,label,action,danger)|view!{<ui-menu-item class=if danger{"menu-item danger-text"}else{"menu-item"} value=action>{icon(glyph)}{label}</ui-menu-item>}).collect_view()}
        </ui-menu-content></ui-menu></div>}.into_any()
}

/// Header buttons: a thread's controls, or a channel's threads and follows.
/// Desktop shows the common ones as buttons; phones get one menu, so the
/// title keeps its room.
pub(super) fn header(a: &App, ch: Id) -> AnyView {
    if let Some(t) = thread(a, ch) {
        let me = a.model.me.as_ref().map(|m| m.user.id);
        let owner = Some(t.owner_id) == me;
        let manage = manage(a, ch);
        let member = joined(a, ch);
        let notify = a.threads.joined.get(&ch).map(|m| m.notify);
        let edit = (owner || manage) && (!t.locked || manage);
        let noun = if t.forum_post { "Post" } else { "Thread" };
        let mut tools: Vec<Item> = vec![];
        if edit {
            tools.push((
                "settings",
                format!("{noun} Settings"),
                "dialog:thread-rename".into(),
                false,
            ));
            tools.push(if t.archived {
                (
                    "archive",
                    format!("Reopen {noun}"),
                    format!("thread-unarchive:{}", ch.0),
                    false,
                )
            } else {
                (
                    "archive",
                    if t.forum_post {
                        "Close Post".into()
                    } else {
                        "Archive Thread".into()
                    },
                    format!("thread-archive:{}", ch.0),
                    false,
                )
            });
        }
        if manage {
            tools.push(if t.locked {
                (
                    "lock-open",
                    format!("Unlock {noun}"),
                    format!("thread-unlock:{}", ch.0),
                    false,
                )
            } else {
                (
                    "lock",
                    format!("Lock {noun}"),
                    format!("thread-lock:{}", ch.0),
                    false,
                )
            });
            if t.forum_post {
                tools.push((
                    "pin",
                    if t.pinned {
                        "Unpin Post".into()
                    } else {
                        "Pin Post".into()
                    },
                    format!("forum-pin:{}", ch.0),
                    false,
                ));
            }
        }
        let leave: Option<Item> = member.then(|| {
            (
                "log-out",
                format!("Leave {noun}"),
                format!("thread-leave:{}", ch.0),
                true,
            )
        });
        let delete: Option<Item> = manage.then(|| {
            (
                "trash-2",
                format!("Delete {noun}"),
                format!("dialog:thread-delete:{}", ch.0),
                true,
            )
        });
        let levels = [
            ("default", "Use the channel setting", ThreadNotify::Default),
            ("all", "All messages", ThreadNotify::All),
            ("mentions", "Only @mentions", ThreadNotify::Mentions),
            ("nothing", "Nothing", ThreadNotify::Nothing),
        ];
        let mut desktop = tools.clone();
        desktop.extend(leave.clone());
        desktop.extend(delete.clone());
        let mut phone: Vec<Item> = vec![(
            "arrow-left",
            "Back to Channel".into(),
            format!("thread-parent:{}", ch.0),
            false,
        )];
        if member {
            phone.extend(levels.iter().map(|(v, label, n)| {
                (
                    if notify == Some(*n) { "check" } else { "bell" },
                    format!("Notify: {label}"),
                    format!("thread-notify:{},{v}", ch.0),
                    false,
                )
            }));
        } else {
            phone.push((
                "plus",
                format!("Join {noun}"),
                format!("thread-join:{}", ch.0),
                false,
            ));
        }
        phone.push((
            "user-plus",
            format!("{noun} Members"),
            format!("thread-members:{}", ch.0),
            false,
        ));
        phone.extend(tools);
        phone.extend(leave);
        phone.extend(delete);
        return view!{
            <div class="contents phone:hidden">
                {ib("arrow-left","Back to Channel",&format!("thread-parent:{}",ch.0))}
                {if member{
                    view!{<ui-menu class="contents"><ui-menu-trigger><button class="icon-button" aria-label="Thread Notifications" title="Thread Notifications">{icon(if notify==Some(ThreadNotify::Nothing){"bell-off"}else{"bell"})}</button></ui-menu-trigger>
                        <ui-menu-content class="menu" side="bottom" align="end" side-offset="6" hidden=true>
                            {levels.into_iter().map(|(v,label,n)|view!{<ui-menu-item class="menu-item" value=format!("thread-notify:{},{v}",ch.0)>{if notify==Some(n){icon("check")}else{view!{<span class="icon" aria-hidden="true"></span>}.into_any()}}{label}</ui-menu-item>}).collect_view()}
                        </ui-menu-content></ui-menu>}.into_any()
                }else{
                    view!{<button class="button secondary h-8 px-3 text-sm" data-live=format!("thread-join:{}",ch.0)>{format!("Join {noun}")}</button>}.into_any()
                }}
                {ib("user-plus",&format!("{noun} Members"),&format!("thread-members:{}",ch.0))}
                {more_menu(&format!("{noun} Options"),"contents",desktop)}
            </div>
            {more_menu(&format!("{noun} Options"),"hidden phone:contents",phone)}
        }.into_any();
    }
    let Some(c) = a.model.channels.get(&ch) else {
        return ().into_any();
    };
    let p = rights(a, ch);
    let mut items: Vec<Item> = vec![];
    match c.kind {
        ChannelKind::Text | ChannelKind::Announcement => {
            items.push((
                "layout-list",
                "Threads".into(),
                format!("threads-browser:{}", ch.0),
                false,
            ));
            if p.intersects(
                Permissions::CREATE_PUBLIC_THREADS | Permissions::CREATE_PRIVATE_THREADS,
            ) {
                items.push((
                    "message-square-text",
                    "Create Thread".into(),
                    format!("thread-new:{}", ch.0),
                    false,
                ));
            }
            if c.kind == ChannelKind::Announcement {
                items.push((
                    "megaphone",
                    "Follow Channel".into(),
                    format!("follow:{}", ch.0),
                    false,
                ));
            } else if settings::can(a, Permissions::MANAGE_GUILD) {
                items.push((
                    "arrow-up-right",
                    "Followed Channels".into(),
                    format!("follows:{}", ch.0),
                    false,
                ));
            }
        }
        ChannelKind::Forum if p.contains(Permissions::MANAGE_CHANNELS) => {
            items.push((
                "settings",
                "Forum Settings".into(),
                "dialog:forum-settings".into(),
                false,
            ));
        }
        _ => {}
    }
    view!{
        <div class="contents phone:hidden">{items.iter().map(|(glyph,label,action,_)|ib(glyph,label,action)).collect_view()}</div>
        {more_menu("Channel Options","hidden phone:contents",items)}
    }.into_any()
}

/// Above the composer in a thread: why you cannot post, or what posting does.
pub(super) fn composer_note(a: &App, ch: Id) -> AnyView {
    let Some(t) = thread(a, ch) else {
        return ().into_any();
    };
    let can = rights(a, ch).contains(Permissions::SEND_MESSAGES);
    let text = if t.locked && !can {
        "This thread is locked. Only moderators can post in it."
    } else if !can {
        "You do not have permission to post in this thread."
    } else if t.archived {
        "This thread is archived. Sending a message reopens it."
    } else if !joined(a, ch) {
        "You are viewing this thread. Join it, or post in it, to follow new messages."
    } else {
        return ().into_any();
    };
    view!{<div class="thread-note mb-2 flex items-center gap-2 rounded-md bg-muted px-3 py-2 text-[13px] text-subtle" role="status">{icon_with(if t.locked{"lock"}else if t.archived{"archive"}else{"message-square-text"},"icon size-4 flex-none")}<span class="min-w-0 flex-1">{text}</span>{if !joined(a,ch)&&!t.locked{view!{<button class="link-button text-[13px] touch:min-h-11" data-live=format!("thread-join:{}",ch.0)>"Join"</button>}.into_any()}else{().into_any()}}</div>}.into_any()
}

/// Under a message: the thread started from it, and whether it was published.
pub(super) fn extras(a: &App, m: &Message) -> AnyView {
    let preview = m.extra.thread().map(|t| {
        view!{<button class="thread-preview mt-1 flex w-full max-w-[min(100%,440px)] items-center gap-3 rounded-lg border bg-card px-3 py-2 text-left hover:bg-accent touch:min-h-11" data-live=format!("thread-open:{}",t.thread_id.0) aria-label=format!("Open thread {}",t.name)>
            {icon_with("message-square-text","icon size-5 flex-none text-muted-foreground")}
            <span class="flex min-w-0 flex-1 flex-col">
                <span class="flex items-center gap-1 truncate text-sm font-semibold">{t.name.clone()}{if t.private{icon_with("lock","icon size-3.5 flex-none")}else{().into_any()}}</span>
                <span class="text-xs text-subtle">{format!("{} · {}{}",count(t.message_count,"message","messages"),ago(t.last_activity_at),if t.locked{" · Locked"}else if t.archived{" · Archived"}else{""})}</span>
            </span>
            {icon_with("chevron-right","icon size-4 flex-none text-muted-foreground")}
        </button>}
    });
    let published = (m.extra.published()
        && a.model.channels.get(&m.channel_id).is_some_and(|c| c.kind == ChannelKind::Announcement))
    .then(|| view!{<span class="published mt-1 inline-flex items-center gap-1 text-xs text-subtle">{icon_with("megaphone","icon size-3.5")}"Published"</span>});
    view! {{preview}{published}}.into_any()
}

/// Extra hover actions on a message: start a thread from it, publish it.
pub(super) fn message_actions(a: &App, m: &Message) -> AnyView {
    let Some(c) = a.model.channels.get(&m.channel_id) else {
        return ().into_any();
    };
    let p = rights(a, c.id);
    let me = a.model.me.as_ref().map(|me| me.user.id);
    let thread = matches!(c.kind, ChannelKind::Text | ChannelKind::Announcement)
        && m.extra.thread().is_none()
        && m.call.is_none()
        && p.contains(Permissions::CREATE_PUBLIC_THREADS);
    let publish = c.kind == ChannelKind::Announcement
        && !m.extra.published()
        && m.extra.crosspost().is_none()
        && m.extra.poll.is_none()
        && (p.contains(Permissions::MANAGE_MESSAGES)
            || (Some(m.author_id) == me && p.contains(Permissions::PUBLISH_MESSAGES)));
    view!{
        {if thread{ib_plain("message-square-text","Create Thread",&format!("thread-new:{},{}",c.id.0,m.id.0))}else{().into_any()}}
        {if publish{ib_plain("megaphone","Publish",&format!("publish:{}",m.id.0))}else{().into_any()}}
    }.into_any()
}

/// A crossposted announcement: from which server and channel, then the text.
pub(super) fn crosspost(a: &App, m: &Message, x: &Crosspost) -> AnyView {
    let files = m
        .extra
        .forwarded
        .as_ref()
        .map(|f| f.attachments.clone())
        .unwrap_or_default();
    let source = m.extra.forwarded.as_ref().and_then(|f| f.source.clone());
    view!{<div class="crosspost mt-1 max-w-[min(100%,560px)] border-l-4 border-primary pl-3">
        <p class="crosspost-source flex flex-wrap items-center gap-1 text-[13px] text-subtle">{icon_with("megaphone","icon size-4")}<span class="rounded-sm bg-primary px-1 text-[11px] font-semibold text-primary-foreground">"SERVER"</span><strong class="text-foreground">{x.source_guild_name.clone()}</strong><span>{format!("#{}",x.source_channel_name)}</span></p>
        {markdown(a,m,&m.content)}
        {content::attachments(a,&files)}
        {source.map(|s|view!{<button class="link-button mt-1 text-xs touch:min-h-11" data-live=format!("jump-message:{},{}",s.channel_id.0,s.message_id.0)>"Go to the original"</button>})}
    </div>}.into_any()
}

fn tag_chip(t: &ForumTag, pressed: Option<bool>, action: String) -> AnyView {
    let label = format!(
        "{}{}",
        t.emoji.clone().map(|e| format!("{e} ")).unwrap_or_default(),
        t.name
    );
    match pressed {
        Some(on) => view!{<button class=format!("forum-tag inline-flex h-7 items-center gap-1 rounded-full border px-2.5 text-xs touch:min-h-11 {}",if on{"border-primary bg-primary/15 text-foreground"}else{"bg-muted text-subtle hover:text-foreground"}) aria-pressed=on.to_string() data-live=action>{label}</button>}.into_any(),
        None => view!{<span class="forum-tag inline-flex h-6 items-center rounded-full bg-muted px-2 text-xs text-subtle">{label}</span>}.into_any(),
    }
}

/// The forum view, in place of a message history.
pub(super) fn forum(a: &App, title: &str) -> AnyView {
    let f = &a.threads.forum;
    let s = f.settings.clone().unwrap_or_default();
    let layout = f.layout.unwrap_or(s.layout);
    let sort = f.sort.unwrap_or(s.sort);
    let can_post = f
        .channel
        .is_some_and(|c| rights(a, c).contains(Permissions::SEND_MESSAGES));
    let cover = |p: &ForumPost| {
        p.first.as_ref().and_then(|m| {
            m.attachments
                .iter()
                .find(|f| {
                    f.content_type.starts_with("image/") && f.thumbnail.is_some() && !f.meta.spoiler
                })
                .map(|f| (f.id, f.meta.description.clone().unwrap_or_default()))
        })
    };
    let posts = f.posts.iter().map(|p| {
        let t = &p.thread;
        let author = f.users.get(&t.owner_id).or_else(|| a.model.users.get(&t.owner_id)).map(name).unwrap_or_else(|| "Someone".into());
        let tags: Vec<&ForumTag> = t.tag_ids.iter().filter_map(|id| s.tags.iter().find(|x| x.id == *id)).collect();
        let excerpt = p.first.as_ref().map(|m| preview(a, m)).unwrap_or_default();
        let unread = a.model.reads.get(&t.id).is_some_and(|r| r.unread_count > 0);
        let replies = t.message_count.saturating_sub(1);
        let image = (layout == ForumLayout::Gallery).then(|| cover(p)).flatten();
        let react = s.default_reaction.clone().zip(p.first.as_ref().map(|m| m.id));
        view!{<li class=format!("forum-post relative rounded-lg border bg-card hover:bg-accent {}",if layout==ForumLayout::Gallery{"flex flex-col overflow-hidden"}else{"flex items-start gap-3 p-3"}) data-post=t.id.0.to_string()>
            {image.map(|(id,alt)|view!{<img class="forum-cover aspect-video w-full object-cover" src=format!("/api/v1/files/{}/thumbnail",id.0) alt=alt loading="lazy"/>})}
            <div class=if layout==ForumLayout::Gallery{"flex min-w-0 flex-1 flex-col gap-1 p-3"}else{"flex min-w-0 flex-1 flex-col gap-1"}>
                <div class="flex flex-wrap items-center gap-1">{if t.pinned{view!{<span class="text-primary" title="Pinned" aria-label="Pinned">{icon_with("pin","icon size-4")}</span>}.into_any()}else{().into_any()}}{tags.into_iter().map(|x|tag_chip(x,None,String::new())).collect_view()}</div>
                <button class=format!("forum-post-title text-left text-base after:absolute after:inset-0 after:content-[''] {}",if unread{"font-bold text-foreground"}else{"font-semibold"}) data-live=format!("thread-open:{}",t.id.0)>{t.name.clone()}</button>
                <p class="line-clamp-2 m-0 text-sm text-subtle wrap-anywhere">{format!("{author}: {excerpt}")}</p>
                <p class="m-0 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                    <span class="inline-flex items-center gap-1">{icon_with("message-square","icon size-3.5")}{count(replies,"reply","replies")}</span>
                    <span>{count(p.reaction_count,"reaction","reactions")}</span>
                    <span>{format!("Active {}",ago(t.last_activity_at))}</span>
                    {if t.locked{view!{<span>"Locked"</span>}.into_any()}else if t.archived{view!{<span>"Closed"</span>}.into_any()}else{().into_any()}}
                </p>
            </div>
            {react.map(|(e,m)|view!{<button class="relative z-10 m-2 rounded-lg border bg-muted px-2 py-1 text-sm touch:min-h-11 touch:min-w-11" aria-label=format!("React with {e}") data-live=format!("forum-react:{},{}",t.id.0,m.0)>{e.clone()}</button>})}
        </li>}
    }).collect_view();
    view!{<div class="forum-view flex min-h-0 flex-1 flex-col">
        <div class="forum-toolbar flex flex-wrap items-center gap-2 border-b px-4 py-3 phone:px-2">
            <form data-live-form="forum-search" class="flex min-w-[200px] flex-1 items-center gap-2" role="search"><label class="sr-only" for="forum-query">"Search posts"</label><input id="forum-query" type="search" class="h-9 flex-1 rounded-md border bg-input px-3 text-sm touch:min-h-11" placeholder=format!("Search {title}") value=f.query.clone() maxlength="200"/></form>
            <div class="flex items-center gap-1" role="group" aria-label="Sort posts">
                {[(ForumSort::Activity,"activity","Latest Activity"),(ForumSort::Created,"created","Newest")].into_iter().map(|(v,arg,label)|view!{<button class=format!("h-8 rounded-md px-2 text-sm touch:min-h-11 {}",if sort==v{"bg-secondary text-foreground"}else{"text-subtle hover:text-foreground"}) aria-pressed=(sort==v).to_string() data-live=format!("forum-sort:{arg}")>{label}</button>}).collect_view()}
            </div>
            <div class="flex items-center gap-1" role="group" aria-label="Layout">
                <button class=format!("icon-button {}",if layout==ForumLayout::List{"bg-secondary"}else{""}) aria-label="List" title="List" aria-pressed=(layout==ForumLayout::List).to_string() data-live="forum-layout:list">{icon("layout-list")}</button>
                <button class=format!("icon-button {}",if layout==ForumLayout::Gallery{"bg-secondary"}else{""}) aria-label="Gallery" title="Gallery" aria-pressed=(layout==ForumLayout::Gallery).to_string() data-live="forum-layout:gallery">{icon("layout-grid")}</button>
            </div>
            {if can_post{view!{<button class="button primary h-9 touch:min-h-11" data-live="forum-new">{icon("plus")}"New Post"</button>}.into_any()}else{().into_any()}}
        </div>
        {if s.tags.is_empty(){().into_any()}else{view!{<div class="forum-tags flex flex-wrap items-center gap-1 px-4 py-2 phone:px-2" role="group" aria-label="Filter by tag">{icon_with("tag","icon size-4 text-muted-foreground")}{s.tags.iter().map(|t|tag_chip(t,Some(f.tags.contains(&t.id)),format!("forum-tag:{}",t.id))).collect_view()}{if f.tags.is_empty(){().into_any()}else{view!{<button class="link-button text-xs touch:min-h-11" data-live="forum-tag:clear">"Clear"</button>}.into_any()}}</div>}.into_any()}}
        <div class="forum-scroll min-h-0 flex-1 touch-pan-y overflow-y-auto overscroll-contain px-4 py-3 phone:px-2">
            {if s.guidelines.trim().is_empty(){().into_any()}else{view!{<details class="forum-guidelines mb-3 rounded-lg border bg-card p-3 text-sm" open=f.posts.is_empty()><summary class="cursor-pointer font-semibold">"Post Guidelines"</summary><p class="mt-2 mb-0 whitespace-pre-wrap text-subtle">{s.guidelines.clone()}</p></details>}.into_any()}}
            {if f.posts.is_empty(){view!{<div class="conversation-intro px-2 py-8"><span class="intro-icon inline-flex rounded-full bg-muted p-4">{icon("messages-square")}</span><h2 class="mt-4 mb-2">{if f.busy{"Loading posts…".to_owned()}else if !f.query.is_empty()||!f.tags.is_empty(){"No posts match.".to_owned()}else{format!("No posts in {title} yet.")}}</h2>{if can_post&&!f.busy&&f.query.is_empty(){view!{<p class="text-subtle">"Start the first one."</p>}.into_any()}else{().into_any()}}</div>}.into_any()}else{().into_any()}}
            <ul class=if layout==ForumLayout::Gallery{"forum-posts m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-3 p-0"}else{"forum-posts m-0 flex list-none flex-col gap-2 p-0"} aria-label="Posts">{posts}</ul>
            {if f.has_more{view!{<div class="mt-3 flex justify-center">{button(if f.busy{"Loading…"}else{"Load More"},"forum-more","secondary")}</div>}.into_any()}else{().into_any()}}
        </div>
    </div>}.into_any()
}

pub(super) fn dialog(a: &App) -> Option<(AnyView, &'static str, &'static str)> {
    let d = a.dialog.as_str();
    Some(match d {
        "thread-new" => (new_thread(a), "Create Thread", ""),
        "thread-rename" => (thread_settings(a)?, "Thread Settings", ""),
        "threads" => (browser(a), "Threads", "threads-browser"),
        "forum-post" => (new_post(a), "New Post", ""),
        "forum-settings" => (forum_settings(a), "Forum Settings", ""),
        _ if d.starts_with("thread-delete:") => (delete(a, id(&d[14..])?)?, "Delete Thread", ""),
        _ if d.starts_with("thread-members:") => (members(a, id(&d[15..])?), "Thread Members", ""),
        _ if d.starts_with("follows:") => (follows(a, id(&d[8..])?), "Followed Channels", ""),
        _ if d.starts_with("follow:") => (follow(a, id(&d[7..])?), "Follow Channel", ""),
        _ => return None,
    })
}

fn submit(label: &str, class: &str) -> AnyView {
    view! {<button type="submit" class=format!("button {class} mt-4")>{label.to_owned()}</button>}
        .into_any()
}

fn new_thread(a: &App) -> AnyView {
    let (parent, message) = a.threads.creating.unwrap_or((Id(0), None));
    let source = message.and_then(|m| a.model.messages.get(&parent).and_then(|ms| ms.get(&m)));
    let suggested: String = source
        .map(|m| a.model.content(m))
        .unwrap_or_default()
        .chars()
        .take(40)
        .collect();
    let p = rights(a, parent);
    let private = message.is_none() && p.contains(Permissions::CREATE_PRIVATE_THREADS);
    let public = p.contains(Permissions::CREATE_PUBLIC_THREADS);
    let parent_name = a
        .model
        .channels
        .get(&parent)
        .map(|c| a.model.channel_name(c))
        .unwrap_or_default();
    view!{<form data-live-form="thread-new"><h2>"Create Thread"</h2>
        <p class="muted-copy">{match source{Some(m)=>format!("From {}’s message in #{parent_name}",a.model.name(m.author_id)),None=>format!("In #{parent_name}")}}</p>
        <div class="field"><label for="thread-name">"Thread Name"</label><input id="thread-name" type="text" maxlength=MAX_THREAD_NAME_CHARS.to_string() value=suggested placeholder="What is it about?" required/></div>
        {archive_select(1440)}
        {if private{view!{
            <label class="live-checkbox"><span class="mr-auto">"Private Thread"<small class="block text-xs text-subtle">"Only people you invite and moderators can see it."</small></span><input type="checkbox" id="thread-private" checked=!public disabled=!public/></label>
            <label class="live-checkbox"><span class="mr-auto">"Anyone in it can invite others"</span><input type="checkbox" id="thread-invitable" checked=true/></label>
        }.into_any()}else{().into_any()}}
        {submit("Create Thread","primary")}
    </form>}.into_any()
}

fn thread_settings(a: &App) -> Option<AnyView> {
    let t = thread(a, a.channel?)?;
    let forum_tags = if t.forum_post {
        a.threads
            .forum
            .settings
            .as_ref()
            .filter(|s| s.channel_id == t.parent_id)
            .map(|s| s.tags.clone())
    } else {
        None
    };
    let manage = manage(a, t.id);
    Some(view!{<form data-live-form="thread-rename"><h2>{if t.forum_post{"Post Settings"}else{"Thread Settings"}}</h2>
        <div class="field"><label for="thread-name">{if t.forum_post{"Title"}else{"Thread Name"}}</label><input id="thread-name" type="text" maxlength=MAX_THREAD_NAME_CHARS.to_string() value=t.name.clone() required/></div>
        {archive_select(t.auto_archive_minutes)}
        {forum_tags.map(|tags|view!{<fieldset class="my-4 flex flex-wrap gap-2"><legend class="mb-2 text-[13px] font-medium text-subtle">"Tags"</legend>{tags.into_iter().map(|tag|view!{<label class="inline-flex items-center gap-1 rounded-full border px-2 py-1 text-sm touch:min-h-11"><input type="checkbox" id=format!("post-tag-{}",tag.id) checked=t.tag_ids.contains(&tag.id) disabled=tag.moderated&&!manage/>{format!("{}{}",tag.emoji.clone().map(|e|format!("{e} ")).unwrap_or_default(),tag.name)}</label>}).collect_view()}</fieldset>})}
        {if t.private{view!{<label class="live-checkbox"><span class="mr-auto">"Anyone in it can invite others"</span><input type="checkbox" id="thread-invitable" checked=t.invitable/></label>}.into_any()}else{().into_any()}}
        {submit("Save Changes","primary")}
    </form>}.into_any())
}

fn delete(a: &App, t: Id) -> Option<AnyView> {
    let info = thread(a, t)?;
    Some(view!{<div><h2>{if info.forum_post{"Delete Post"}else{"Delete Thread"}}</h2>
        <p>{format!("Delete “{}” and its {}? This cannot be undone.",info.name,count(info.message_count,"message","messages"))}</p>
        <div class="mt-4 flex gap-2 phone:flex-col">{button("Delete",&format!("thread-delete:{}",t.0),"danger")}{button("Cancel","close","secondary")}</div>
    </div>}.into_any())
}

fn browser(a: &App) -> AnyView {
    let b = &a.threads.browser;
    let parent = b.parent.and_then(|p| a.model.channels.get(&p));
    let parent_name = parent.map(|c| a.model.channel_name(c)).unwrap_or_default();
    let create = b.parent.is_some_and(|p| {
        rights(a, p)
            .intersects(Permissions::CREATE_PUBLIC_THREADS | Permissions::CREATE_PRIVATE_THREADS)
    });
    view!{<div class="threads-browser"><h2>{format!("Threads in #{parent_name}")}</h2>
        <div class="mb-3 flex items-center gap-2" role="tablist" aria-label="Threads">
            {[(false,"active","Active"),(true,"archived","Archived")].into_iter().map(|(on,arg,label)|view!{<button role="tab" class=format!("h-8 rounded-md px-3 text-sm touch:min-h-11 {}",if b.archived==on{"bg-secondary font-semibold text-foreground"}else{"text-subtle hover:text-foreground"}) aria-selected=(b.archived==on).to_string() data-live=format!("threads-tab:{arg}")>{label}</button>}).collect_view()}
            <span class="flex-1"></span>
            {if create{b.parent.map(|p|button("Create Thread",&format!("thread-new:{}",p.0),"primary")).into_any()}else{().into_any()}}
        </div>
        {if b.threads.is_empty(){view!{<p class="muted-copy py-6 text-center">{if b.busy{"Loading threads…"}else if b.archived{"No archived threads."}else{"No active threads. Start one from a message, or with Create Thread."}}</p>}.into_any()}else{().into_any()}}
        <ul class="m-0 flex max-h-[60vh] list-none flex-col gap-1 overflow-y-auto p-0">{b.threads.iter().map(|t|{
            let owner=a.model.users.get(&t.owner_id).map(name).unwrap_or_else(||"Someone".into());
            view!{<li><button class="thread-entry flex w-full items-center gap-3 rounded-md p-2 text-left hover:bg-accent touch:min-h-11" data-live=format!("thread-open:{}",t.id.0)>
                {icon_with(if t.private{"lock"}else{"message-square-text"},"icon size-5 flex-none text-muted-foreground")}
                <span class="flex min-w-0 flex-1 flex-col"><span class="truncate text-sm font-semibold">{t.name.clone()}</span><span class="truncate text-xs text-subtle">{format!("{owner} · {} · {}",count(t.message_count,"message","messages"),if b.archived{format!("archived {}",ago(t.archived_at.unwrap_or(t.last_activity_at)))}else{ago(t.last_activity_at)})}</span></span>
                {if joined(a,t.id){view!{<span class="rounded-sm bg-muted px-1.5 text-xs text-subtle">"Joined"</span>}.into_any()}else{().into_any()}}
            </button></li>}
        }).collect_view()}</ul>
        {if b.has_more{view!{<div class="mt-2 flex justify-center">{button(if b.busy{"Loading…"}else{"Load More"},"threads-more","secondary")}</div>}.into_any()}else{().into_any()}}
    </div>}.into_any()
}

fn members(a: &App, t: Id) -> AnyView {
    let info = thread(a, t);
    let me = a.model.me.as_ref().map(|m| m.user.id);
    let manage = manage(a, t);
    let owner = info.is_some_and(|i| Some(i.owner_id) == me);
    let private = info.is_some_and(|i| i.private);
    let can_add = info.is_some_and(|i| !i.locked || manage)
        && (if private {
            manage || owner || (info.is_some_and(|i| i.invitable) && joined(a, t))
        } else {
            rights(a, t).contains(Permissions::SEND_MESSAGES)
        });
    let can_remove = manage || (private && owner);
    let list = a.threads.members.as_ref().filter(|(x, ..)| *x == t);
    let present: Vec<Id> = list
        .map(|(_, l, _)| l.iter().map(|m| m.user.id).collect())
        .unwrap_or_default();
    let guild = info.map(|i| i.guild_id);
    let candidates: Vec<&Member> = a
        .model
        .members
        .values()
        .filter(|m| Some(m.guild_id) == guild && !present.contains(&m.user.id))
        .take(200)
        .collect();
    view!{<div class="thread-members"><h2>{format!("Members — {}",info.map_or(0,|i|i.member_count))}</h2>
        {if list.is_none(){view!{<p class="muted-copy">"Loading members…"</p>}.into_any()}else{().into_any()}}
        <ul class="m-0 flex max-h-[50vh] list-none flex-col gap-1 overflow-y-auto p-0">{list.map(|(_,l,_)|l.iter().map(|m|view!{<li class="flex items-center gap-2 rounded-md p-1">{avatar(&m.user,"small")}<span class="min-w-0 flex-1 truncate text-sm">{name(&m.user)}</span>{if can_remove&&Some(m.user.id)!=me{ib_plain("user-minus",&format!("Remove {}",name(&m.user)),&format!("thread-remove-member:{},{}",t.0,m.user.id.0))}else{().into_any()}}</li>}).collect_view())}</ul>
        {if list.is_some_and(|(_,_,more)|*more){view!{<div class="mt-2 flex justify-center">{button("Load More",&format!("thread-members-more:{}",t.0),"secondary")}</div>}.into_any()}else{().into_any()}}
        {if can_add&&!candidates.is_empty(){view!{<form data-live-form="thread-add-member" class="mt-4 flex items-end gap-2 phone:flex-col phone:items-stretch"><div class="field mb-0 flex-1"><label for="thread-member">"Add someone"</label><select id="thread-member">{candidates.into_iter().map(|m|view!{<option value=m.user.id.0.to_string()>{name(&m.user)}</option>}).collect_view()}</select></div><button type="submit" class="button primary">"Add"</button></form>}.into_any()}else{().into_any()}}
    </div>}.into_any()
}

fn new_post(a: &App) -> AnyView {
    let s = a.threads.forum.settings.clone().unwrap_or_default();
    let manage = a
        .threads
        .forum
        .channel
        .is_some_and(|c| rights(a, c).contains(Permissions::MANAGE_THREADS));
    view!{<form data-live-form="forum-post"><h2>"New Post"</h2>
        {if s.guidelines.trim().is_empty(){().into_any()}else{view!{<p class="forum-guidelines rounded-md bg-muted p-3 text-[13px] whitespace-pre-wrap text-subtle">{s.guidelines.clone()}</p>}.into_any()}}
        <div class="field"><label for="post-title">"Title"</label><input id="post-title" type="text" maxlength=MAX_THREAD_NAME_CHARS.to_string() required/></div>
        <div class="field"><label for="post-content">"Message"</label><textarea id="post-content" rows="5" maxlength=MAX_MESSAGE_CHARS.to_string() class="min-h-28 rounded-md border bg-input p-2"></textarea></div>
        {if s.tags.is_empty(){().into_any()}else{view!{<fieldset class="my-4 flex flex-wrap gap-2"><legend class="mb-2 text-[13px] font-medium text-subtle">{if s.require_tags{"Tags (choose at least one)"}else{"Tags (up to 5)"}}</legend>{s.tags.iter().map(|tag|view!{<label class="inline-flex items-center gap-1 rounded-full border px-2 py-1 text-sm touch:min-h-11"><input type="checkbox" id=format!("post-tag-{}",tag.id) disabled=tag.moderated&&!manage/>{format!("{}{}",tag.emoji.clone().map(|e|format!("{e} ")).unwrap_or_default(),tag.name)}</label>}).collect_view()}</fieldset>}.into_any()}}
        <div class="my-2">{a.attachments.iter().enumerate().map(|(i,f)|content::attachment_chip(a,i,f)).collect_view()}</div>
        {if a.uploading{view!{<p class="text-xs text-subtle" role="status">{a.upload_progress.clone()}</p>}.into_any()}else{().into_any()}}
        <div class="mt-2 flex flex-wrap items-center gap-2"><button type="button" class="button secondary" data-live="pick-file">{icon("paperclip")}"Add Files"</button><input type="file" id="live-file" multiple hidden/><button type="submit" class="button primary" disabled=a.uploading>"Post"</button></div>
    </form>}.into_any()
}

fn forum_settings(a: &App) -> AnyView {
    let s = a.threads.forum.settings.clone().unwrap_or_default();
    view!{<form data-live-form="forum-settings" class="forum-settings"><h2>"Forum Settings"</h2>
        <div class="field"><label for="forum-guidelines">"Post Guidelines"</label><textarea id="forum-guidelines" rows="4" maxlength=MAX_FORUM_GUIDELINES_CHARS.to_string() class="min-h-24 rounded-md border bg-input p-2">{s.guidelines.clone()}</textarea><small>"Shown to everyone writing a post."</small></div>
        <fieldset class="my-4"><legend class="mb-2 text-[13px] font-medium text-subtle">{format!("Tags ({} of {MAX_FORUM_TAGS})",s.tags.len())}</legend>
            {s.tags.iter().enumerate().map(|(i,t)|view!{<div class="forum-tag-row mb-2 flex flex-wrap items-center gap-2"><label class="sr-only" for=format!("tag-emoji-{i}")>"Emoji"</label><input id=format!("tag-emoji-{i}") class="h-9 w-14 rounded-md border bg-input px-2 text-center touch:min-h-11" value=t.emoji.clone().unwrap_or_default() maxlength="32" placeholder="🙂"/><label class="sr-only" for=format!("tag-name-{i}")>"Tag name"</label><input id=format!("tag-name-{i}") class="h-9 min-w-0 flex-1 rounded-md border bg-input px-2 touch:min-h-11" value=t.name.clone() maxlength=MAX_FORUM_TAG_CHARS.to_string()/><label class="inline-flex items-center gap-1 text-xs touch:min-h-11"><input type="checkbox" id=format!("tag-moderated-{i}") checked=t.moderated/>"Moderators only"</label><label class="inline-flex items-center gap-1 text-xs text-destructive touch:min-h-11"><input type="checkbox" id=format!("tag-remove-{i}")/>"Remove"</label></div>}).collect_view()}
            {if s.tags.len()<MAX_FORUM_TAGS{view!{<div class="forum-tag-row flex flex-wrap items-center gap-2"><label class="sr-only" for="tag-new-emoji">"New tag emoji"</label><input id="tag-new-emoji" class="h-9 w-14 rounded-md border bg-input px-2 text-center touch:min-h-11" maxlength="32" placeholder="🙂"/><label class="sr-only" for="tag-new">"New tag name"</label><input id="tag-new" class="h-9 min-w-0 flex-1 rounded-md border bg-input px-2 touch:min-h-11" maxlength=MAX_FORUM_TAG_CHARS.to_string() placeholder="Add a tag"/><label class="inline-flex items-center gap-1 text-xs touch:min-h-11"><input type="checkbox" id="tag-new-moderated"/>"Moderators only"</label></div>}.into_any()}else{().into_any()}}
        </fieldset>
        <label class="live-checkbox"><span class="mr-auto">"Require a tag on every post"</span><input type="checkbox" id="forum-require-tags" checked=s.require_tags/></label>
        <div class="field"><label for="forum-reaction">"Default Reaction"</label><input id="forum-reaction" type="text" maxlength="32" value=s.default_reaction.clone().unwrap_or_default() placeholder="👍"/></div>
        <div class="grid grid-cols-2 gap-3 phone:grid-cols-1">
            <div class="field"><label for="forum-sort">"Default Sort"</label><select id="forum-sort"><option value="activity" selected=s.sort==ForumSort::Activity>"Latest Activity"</option><option value="created" selected=s.sort==ForumSort::Created>"Creation Date"</option></select></div>
            <div class="field"><label for="forum-layout">"Default Layout"</label><select id="forum-layout"><option value="list" selected=s.layout==ForumLayout::List>"List"</option><option value="gallery" selected=s.layout==ForumLayout::Gallery>"Gallery"</option></select></div>
            <div class="field"><label for="forum-post-slowmode">"Post Slowmode (seconds)"</label><input id="forum-post-slowmode" type="number" min="0" max=MAX_FORUM_SLOWMODE_SECONDS.to_string() value=s.post_slowmode.to_string()/></div>
            <div class="field"><label for="forum-reply-slowmode">"Reply Slowmode (seconds)"</label><input id="forum-reply-slowmode" type="number" min="0" max=MAX_FORUM_SLOWMODE_SECONDS.to_string() value=s.reply_slowmode.to_string()/></div>
        </div>
        {archive_select(if s.default_auto_archive_minutes==0{4320}else{s.default_auto_archive_minutes})}
        {submit("Save Changes","primary")}
    </form>}.into_any()
}

fn follow(a: &App, source: Id) -> AnyView {
    let source_name = a
        .model
        .channels
        .get(&source)
        .map(|c| a.model.channel_name(c))
        .unwrap_or_default();
    let source_guild = a.model.channels.get(&source).and_then(|c| c.guild_id);
    // Text channels in servers where you have Manage Server.
    let targets: Vec<(&ChannelSummary, String)> = a
        .model
        .channels
        .values()
        .filter(|c| c.kind == ChannelKind::Text)
        .filter_map(|c| {
            let g = a.model.guilds.get(&c.guild_id?)?;
            let allowed = g.my_role == MemberRole::Owner
                || g.my_permissions
                    .intersects(Permissions::ADMINISTRATOR | Permissions::MANAGE_GUILD);
            allowed.then(|| (c, g.guild.name.clone()))
        })
        .collect();
    let manage = rights(a, source).contains(Permissions::MANAGE_CHANNELS);
    view!{<div><h2>{format!("Follow #{source_name}")}</h2>
        <p class="muted-copy">"Published announcements from this channel will appear in a channel of yours. Only the published messages are shared."</p>
        {if targets.is_empty(){view!{<p class="settings-empty">"You need Manage Server in a server to follow channels into it."</p>}.into_any()}else{view!{<form data-live-form="follow"><div class="field"><label for="follow-target">"Into"</label><select id="follow-target">{targets.into_iter().map(|(c,g)|view!{<option value=c.id.0.to_string() selected=c.guild_id!=source_guild>{format!("#{} — {g}",a.model.channel_name(c))}</option>}).collect_view()}</select></div>{submit("Follow","primary")}</form>}.into_any()}}
        {if manage{view!{<div class="mt-4">{button("See Followers",&format!("follows:{}",source.0),"link-button")}</div>}.into_any()}else{().into_any()}}
    </div>}.into_any()
}

fn follows(a: &App, ch: Id) -> AnyView {
    let list = a
        .threads
        .follows
        .as_ref()
        .filter(|(c, _)| *c == ch)
        .map(|(_, l)| l);
    let source = a
        .model
        .channels
        .get(&ch)
        .is_some_and(|c| c.kind == ChannelKind::Announcement);
    view!{<div><h2>{if source{"Followers"}else{"Followed Channels"}}</h2>
        {match list{
            None=>view!{<p class="muted-copy">"Loading…"</p>}.into_any(),
            Some(l) if l.is_empty()=>view!{<p class="muted-copy">{if source{"No servers follow this channel yet."}else{"This channel does not follow any announcement channels."}}</p>}.into_any(),
            Some(l)=>view!{<ul class="m-0 flex list-none flex-col gap-1 p-0">{l.iter().map(|f|view!{<li class="flex items-center gap-2 rounded-md p-2">{icon_with("megaphone","icon size-4 text-muted-foreground")}<span class="min-w-0 flex-1 truncate text-sm">{if source{format!("{} · #{}",f.target_guild_name,f.target_channel_name)}else{format!("{} · #{}",f.source_guild_name,f.source_channel_name)}}</span>{button("Remove",&format!("unfollow:{}",f.id.0),"danger-text")}</li>}).collect_view()}</ul>}.into_any(),
        }}
    </div>}.into_any()
}
