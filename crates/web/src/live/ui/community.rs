//! Community views: Rules Screening, Applications, Onboarding, Discovery,
//! Templates and Insights pages in server settings; the new-member flow,
//! Browse Channels and the Server Guide; the server directory (Discover);
//! applying to join; starting a server from a template.
use super::*;
pub(super) use crate::live::community::shown;
use crate::live::community::{steps, Step};
use std::fmt::Write as _;

pub(super) fn is_server_page(page: &str) -> bool {
    crate::live::community::admin_page(page)
}
/// The dialogs `dialog` draws (in this module's own WASM module: `ui::LazyViews`).
pub(super) fn owns(d: &str) -> bool {
    crate::live::community::owns_dialog(d) && d != "create"
}
fn state(a: &App) -> Option<&CommunityState> {
    a.guild.and_then(|g| a.community.states.get(&g))
}
/// A button that never submits its form.
fn tb(label: &str, action: &str, class: &str) -> AnyView {
    view!{<button type="button" class=format!("button {class}") data-live=action.to_owned()>{label.to_owned()}</button>}.into_any()
}
fn loading(title: &str) -> AnyView {
    view! {<h2>{title.to_owned()}</h2><p class="muted-copy" role="status">"Loading…"</p>}.into_any()
}
fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
fn channel_label(a: &App, ch: Id) -> String {
    a.model.channels.get(&ch).map_or_else(
        || "deleted channel".into(),
        |c| format!("#{}", a.model.channel_name(c)),
    )
}
/// Rules and descriptions are short Markdown: escaped text, never HTML.
fn rich(text: &str) -> AnyView {
    use sticktalk_client_core::markdown::{parse, Block, Inline, Target};
    let span = |v: &Inline| {
        let s = &v.style;
        let class = format!(
            "{}{}{}{}",
            if s.bold { "font-semibold " } else { "" },
            if s.italic { "italic " } else { "" },
            if s.strike {
                "line-through "
            } else if s.underline {
                "underline "
            } else {
                ""
            },
            if s.code {
                "rounded-sm bg-rail px-1 font-mono"
            } else {
                ""
            }
        );
        match &v.target {
            Some(Target::Auto(url) | Target::Masked(url)) if url.starts_with("https://") || url.starts_with("http://") => {
                view! {<a href=url.clone() target="_blank" rel="noopener noreferrer">{v.text.clone()}</a>}.into_any()
            }
            _ => view! {<span class=class>{v.text.clone()}</span>}.into_any(),
        }
    };
    let doc = parse(text);
    view!{<div class="markdown">{doc.blocks.iter().map(|b| match b {
        Block::Code { text, .. } => view!{<pre><code>{text.clone()}</code></pre>}.into_any(),
        Block::Header(_, v) | Block::Quote(v) | Block::List(v) | Block::Paragraph(v) => view!{<p>{v.iter().map(span).collect_view()}</p>}.into_any(),
    }).collect_view()}</div>}.into_any()
}
fn badge(text: &str, tone: &str) -> AnyView {
    view!{<span class=format!("inline-flex items-center rounded-sm border px-1.5 py-0.5 text-xs font-semibold {tone}")>{text.to_owned()}</span>}.into_any()
}
fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect()
}

// ---------------------------------------------------------------- shell hooks

/// The Discover entry at the bottom of the server rail.
pub(super) fn discover_button(_: &App) -> AnyView {
    ib("compass", "Discover", "dialog:discover")
}
/// Top of a server's channel list: Server Guide, Channels & Roles, Browse Channels.
pub(super) fn sidebar_rows(a: &App) -> AnyView {
    let Some(s) = state(a).filter(|_| !a.nav.favorites) else {
        return ().into_any();
    };
    let o = &s.onboarding;
    let row = |glyph: &str, label: &str, action: &str| {
        view!{<button class="community-row nav-row flex w-full items-center justify-start gap-2 rounded-md p-2 text-sm font-medium text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground touch:min-h-11 phone:text-base" data-live=action.to_owned()>{icon_with(glyph,"icon text-muted-foreground")}{label.to_owned()}</button>}.into_any()
    };
    let guide = o.enabled
        && (!o.tasks.is_empty() || !o.resources.is_empty() || !o.welcome.description.is_empty());
    let roles = o.enabled && !o.questions.is_empty();
    view! {
        {guide.then(|| row("list-checks", "Server Guide", "dialog:cm-guide"))}
        {roles.then(|| row("tag", "Channels & Roles", "cm:flow,revisit"))}
        {(!o.opt_in_channel_ids.is_empty()).then(|| row("layout-list", "Browse Channels", "dialog:cm-browse"))}
    }
    .into_any()
}
/// Entries in the server menu.
pub(super) fn menu_items(a: &App) -> AnyView {
    let Some(s) = state(a) else {
        return ().into_any();
    };
    let o = &s.onboarding;
    view! {
        {(o.enabled && !o.questions.is_empty()).then(|| view!{<ui-menu-item class="menu-item" value="cm:flow,revisit">{icon("tag")}"Channels & Roles"</ui-menu-item>})}
        {(!o.opt_in_channel_ids.is_empty()).then(|| view!{<ui-menu-item class="menu-item" value="dialog:cm-browse">{icon("layout-list")}"Browse Channels"</ui-menu-item>})}
        {(s.admission.screening && !s.admission.rules.is_empty()).then(|| view!{<ui-menu-item class="menu-item" value="dialog:cm-rules">{icon("shield-check")}"Server Rules"</ui-menu-item>})}
    }
    .into_any()
}
/// Above the composer: the member still has to accept the rules or answer.
pub(super) fn composer_notice(a: &App, c: Id) -> AnyView {
    let Some(s) = a
        .model
        .channels
        .get(&c)
        .and_then(|ch| ch.guild_id)
        .and_then(|g| a.community.states.get(&g))
    else {
        return ().into_any();
    };
    if !s.me.rules_pending && !s.me.questions_pending {
        return ().into_any();
    }
    let text = if s.me.rules_pending {
        "Accept this server’s rules to send messages, react, join voice and message its members."
    } else {
        "Answer this server’s questions for new members to start talking."
    };
    view!{<div class="screening-notice mb-2 flex flex-wrap items-center gap-3 rounded-lg border border-primary bg-card px-3 py-2 text-[13px]" role="status">{icon_with("shield-check","icon size-4 flex-none")}<span class="min-w-0 flex-1">{text}</span>{tb(if s.me.rules_pending{"Read the Rules"}else{"Continue"},"cm:flow","primary")}</div>}.into_any()
}
/// In the Join dialog: what the invite's server asks of new members.
pub(super) fn join_note(a: &App) -> AnyView {
    let Some((code, g)) = a
        .community
        .gate
        .as_ref()
        .filter(|(code, _)| *code == a.invite_wanted)
    else {
        return ().into_any();
    };
    if g.joined {
        return ().into_any();
    }
    let status = g.application.as_ref().map(|my| match my.status {
        ApplicationStatus::Pending => "Your application is waiting for review.",
        ApplicationStatus::Rejected => "Your last application was declined.",
        ApplicationStatus::Approved => "Your application was approved.",
        ApplicationStatus::Withdrawn => "You withdrew your application.",
    });
    view!{<div class="join-gate mt-2 mb-4 rounded-lg border bg-card p-3 text-sm" role="status">
        {if g.applications {view!{<p><strong class="font-semibold">"This server asks new members to apply."</strong>" A moderator reviews each application."</p>
            {status.map(|s| view!{<p class="mt-1 text-subtle">{s}</p>})}
            <div class="mt-3 flex flex-wrap gap-2">{if g.application.as_ref().is_some_and(|m|m.status==ApplicationStatus::Pending){tb("View Application","dialog:cm-mine","secondary")}else{tb("Apply to Join",&format!("dialog:cm-apply:invite:{code}"),"primary")}}</div>}.into_any()}
        else if g.screening {view!{<p>{format!("You’ll be asked to accept {} before you can talk.", plural(g.rules.len() as u32,"rule","rules"))}</p>}.into_any()}
        else {().into_any()}}
    </div>}.into_any()
}
/// In Create a Server: start from a starter template or a template link.
pub(super) fn create_extra(a: &App) -> AnyView {
    let glyph = |code: &str| match code {
        "builtin:friends" => "users",
        "builtin:gaming" => "headphones",
        "builtin:study" => "bookmark",
        "builtin:club" => "sparkles",
        _ => "layout-list",
    };
    view!{<section class="starter-templates mt-6 border-t pt-4"><h3 class="category mb-2">"Start from a template"</h3>
        <div class="grid gap-2 sm:grid-cols-2">{a.community.builtins.iter().map(|t| view!{<button type="button" class="starter-template flex items-start gap-3 rounded-lg border bg-card p-3 text-left hover:bg-accent touch:min-h-11" data-live=format!("dialog:template:{}",t.code)>{icon_with(glyph(&t.code),"icon mt-0.5 size-5 flex-none text-primary")}<span class="flex min-w-0 flex-col"><strong class="font-semibold">{t.name.clone()}</strong><small class="text-subtle">{t.description.clone()}</small></span></button>}).collect_view()}</div>
        {if a.community.builtins.is_empty(){view!{<p class="muted-copy" role="status">"Loading templates…"</p>}.into_any()}else{().into_any()}}
        <form data-live-form="cm-template-link" class="mt-4 flex flex-wrap items-end gap-2"><div class="field mb-0 min-w-48 flex-1"><label for="cm-template-code">"Template link or code"</label><input id="cm-template-code" type="text" maxlength="200"/></div><button type="button" class="button secondary" data-live="cm:template-link">"Preview"</button></form>
    </section>}.into_any()
}

// ---------------------------------------------------------------- settings

/// The Community group in server settings.
pub(super) fn nav(a: &App) -> Vec<(&'static str, &'static str, &'static str, bool)> {
    let manage = settings::can(a, Permissions::MANAGE_GUILD);
    vec![
        ("shield-check", "Rules Screening", "cm-screening", manage),
        ("file", "Applications", "cm-applications", manage),
        (
            "users",
            "Review Applications",
            "cm-queue",
            settings::can(a, Permissions::KICK_MEMBERS),
        ),
        ("sparkles", "Onboarding", "cm-onboarding", manage),
        ("compass", "Discovery", "cm-discovery", manage),
        ("layout-grid", "Templates", "cm-templates", manage),
        (
            "chart-column",
            "Insights",
            "cm-insights",
            settings::can_any(a, Permissions::VIEW_AUDIT_LOG | Permissions::MANAGE_GUILD),
        ),
    ]
}
pub(super) fn nav_group(a: &App) -> AnyView {
    let rows = nav(a);
    if !rows.iter().any(|r| r.3) {
        return ().into_any();
    }
    view!{<h3 class="category mx-2 mt-6 mb-2">"Community"</h3>{rows.into_iter().filter(|r|r.3).map(|(glyph,label,page,_)|settings_nav_row(glyph,label,&format!("dialog:{page}"),a.dialog==page)).collect_view()}}.into_any()
}

fn screening(a: &App) -> AnyView {
    let Some(s) = state(a) else {
        return loading("Rules Screening");
    };
    let allowed = settings::can(a, Permissions::MANAGE_GUILD);
    let adm = &s.admission;
    let slots = (adm.rules.len() + 1)
        .max(a.community.rule_slots)
        .clamp(1, MAX_RULES);
    view!{<form data-live-form="cm-screening"><h2>"Rules Screening"</h2>
        <p class="muted-copy mb-4">"New members accept your rules before they can send messages, react, join voice or message other members. Until then they can read the channels they’re allowed to see."</p>
        <fieldset disabled=!allowed>
        <label class="live-checkbox"><span>"Ask new members to accept the rules"</span><input id="cm-screening-on" type="checkbox" checked=adm.screening/></label>
        <label class="field"><span>"Introduction"</span><textarea id="cm-screening-description" rows="2" maxlength="300">{adm.description.clone()}</textarea></label>
        <div class="rule-list grid gap-2">{(0..slots).map(|i| view!{<label class="field mb-0"><span>{format!("Rule {}",i+1)}</span><textarea id=format!("cm-rule-{i}") rows="2" maxlength="300" placeholder=if i==0{"Be respectful. No harassment or hate speech."}else{""}>{adm.rules.get(i).cloned().unwrap_or_default()}</textarea></label>}).collect_view()}</div>
        <div class="mt-2">{if slots<MAX_RULES{tb("Add a Rule","cm:rule-add","secondary")}else{view!{<p class="muted-copy">"16 rules at most."</p>}.into_any()}}</div>
        <p class="muted-copy mt-2">"Rules may use short Markdown: **bold**, *italic*, `code` and links."</p>
        <label class="live-checkbox mt-4"><span>"Ask current members to accept the changed rules again"</span><input id="cm-reaccept" type="checkbox"/></label>
        </fieldset>
        <button type="submit" class="button primary" disabled=!allowed>"Save"</button>{saved_note(a,"cm-screening")}
        {(!allowed).then(|| view!{<p class="muted-copy">"You need Manage Server permission to change these."</p>})}
    </form>}.into_any()
}
fn applications(a: &App) -> AnyView {
    let Some(s) = state(a) else {
        return loading("Applications");
    };
    let allowed = settings::can(a, Permissions::MANAGE_GUILD);
    let adm = &s.admission;
    let slots = adm
        .questions
        .len()
        .max(1)
        .max(a.community.question_slots)
        .min(MAX_APPLICATION_QUESTIONS);
    view!{<form data-live-form="cm-applications"><h2>"Applications"</h2>
        <p class="muted-copy mb-4">"Instead of joining at once, people apply. They wait with no access until a moderator with Kick Members approves them. Your rules are part of the application when screening is on."</p>
        <fieldset disabled=!allowed>
        <label class="live-checkbox"><span>"People apply to join this server"</span><input id="cm-applications-on" type="checkbox" checked=adm.applications/></label>
        <h3 class="mt-4">"Questions"</h3>
        {(0..slots).map(|i| {let q=adm.questions.get(i).cloned().unwrap_or_default(); view!{<div class="application-question mb-3 grid gap-2 rounded-lg border p-3">
            <div class="flex flex-wrap gap-2"><label class="field mb-0 min-w-48 flex-1"><span>{format!("Question {}",i+1)}</span><input id=format!("cm-aq{i}-prompt") type="text" maxlength="150" value=q.prompt.clone()/></label>
            <label class="field mb-0 w-40"><span>"Answer"</span><select id=format!("cm-aq{i}-kind")><option value="short" selected=q.kind==QuestionKind::Short>"Short text"</option><option value="paragraph" selected=q.kind==QuestionKind::Paragraph>"Paragraph"</option><option value="choice" selected=q.kind==QuestionKind::Choice>"Single choice"</option></select></label></div>
            <label class="field mb-0"><span>"Choices (one per line, for single choice)"</span><textarea id=format!("cm-aq{i}-choices") rows="2">{q.choices.join("\n")}</textarea></label>
            <label class="live-checkbox"><span>"Required"</span><input id=format!("cm-aq{i}-required") type="checkbox" checked=q.required/></label>
        </div>}}).collect_view()}
        {if slots<MAX_APPLICATION_QUESTIONS{tb("Add a Question","cm:question-add","secondary")}else{().into_any()}}
        <p class="muted-copy mt-2">"Leave a question empty to remove it."</p>
        </fieldset>
        <div class="mt-4 flex flex-wrap gap-2"><button type="submit" class="button primary" disabled=!allowed>"Save"</button>{tb(&format!("Review Applications ({})",s.pending_applications),"dialog:cm-queue","secondary")}</div>{saved_note(a,"cm-applications")}
    </form>}.into_any()
}
fn queue(a: &App) -> AnyView {
    let Some((_, q)) = a
        .community
        .queue
        .as_ref()
        .filter(|(g, _)| Some(*g) == a.guild)
    else {
        return loading("Review Applications");
    };
    let card = |app: &Application| {
        let uid = app.user.id.0;
        view! {<article class="application-card mb-4 rounded-lg border bg-card p-4" data-applicant=uid.to_string()>
            <header class="flex flex-wrap items-center gap-3">{avatar(&app.user,"small")}<span class="flex min-w-0 flex-1 flex-col"><strong class="font-semibold wrap-anywhere">{name(&app.user)}</strong><small class="text-subtle">{format!("@{} · account created {} · applied {} · via {}",app.user.username,timestamp(app.account_created_at),timestamp(app.created_at),app.source.label())}</small></span></header>
            <dl class="mt-3 grid gap-2">{app.questions.iter().zip(app.answers.iter()).map(|(q,ans)| view!{<div><dt class="text-xs font-semibold text-subtle">{q.prompt.clone()}</dt><dd class="whitespace-pre-wrap wrap-anywhere">{if ans.is_empty(){"—".to_owned()}else{ans.clone()}}</dd></div>}).collect_view()}</dl>
            {match app.status {
                ApplicationStatus::Pending => view!{<div class="mt-3 grid gap-2"><label class="field mb-0"><span>"Reason (optional, shown to the applicant if declined)"</span><input id=format!("cm-reason-{uid}") type="text" maxlength="512"/></label>
                    <div class="flex flex-wrap gap-2 phone:flex-col">{tb("Approve",&format!("cm:review,{uid}"),"primary")}{tb(if app.interview_channel_id.is_some(){"Open Interview"}else{"Interview"},&format!("cm:interview,{uid}"),"secondary")}{tb("Decline",&format!("cm:reject,{uid}"),"danger-text")}</div></div>}.into_any(),
                status => view!{<p class="mt-2 text-sm">{badge(match status{ApplicationStatus::Approved=>"Approved",ApplicationStatus::Rejected=>"Declined",_=>"Withdrawn"},"")}{app.reason.clone().map(|r|view!{<span class="ml-2 text-subtle">{r}</span>})}</p>}.into_any(),
            }}
        </article>}
    };
    view!{<h2>"Review Applications"</h2><p class="muted-copy mb-4">{format!("{} waiting.", q.pending.len())}</p>
        {if q.pending.is_empty(){view!{<div class=SETTINGS_EMPTY>{icon("inbox")}<p>"No applications are waiting."</p></div>}.into_any()}else{q.pending.iter().map(card).collect_view().into_any()}}
        {(!q.decided.is_empty()).then(|| view!{<h3 class="mt-6 mb-2">"Recently decided"</h3>{q.decided.iter().map(card).collect_view()}})}
    }.into_any()
}

fn channel_options(a: &App, chosen: &[Id], none: bool) -> AnyView {
    view!{{none.then(|| view!{<option value="">"None"</option>})}{channel_tree(a).into_iter().filter(|(c,_)|c.kind!=ChannelKind::Category).map(|(c,depth)|view!{<option value=c.id.0.to_string() selected=chosen.contains(&c.id)>{format!("{}#{}","\u{2003}".repeat(depth.min(6)),a.model.channel_name(c))}</option>}).collect_view()}}.into_any()
}
fn onboarding_editor(a: &App) -> AnyView {
    let Some(s) = state(a) else {
        return loading("Onboarding");
    };
    let allowed = settings::can(a, Permissions::MANAGE_GUILD);
    let o = a
        .community
        .draft
        .clone()
        .unwrap_or_else(|| s.onboarding.clone());
    let mut roles: Vec<&Role> = a
        .roles
        .values()
        .filter(|r| Some(r.guild_id) == a.guild && r.id != r.guild_id)
        .collect();
    roles.sort_by_key(|r| std::cmp::Reverse(r.position));
    let role_options = |chosen: &[Id]| {
        roles.iter().map(|r| {let bad=!onboarding_grantable(r.permissions);view!{<option value=r.id.0.to_string() selected=chosen.contains(&r.id) disabled=bad>{if bad{format!("{} (has more than member rights)",r.name)}else{r.name.clone()}}</option>}}).collect_view()
    };
    let question = |q: &OnboardingQuestion| {
        let p = format!("cm-q{}", q.id);
        view! {<fieldset class="onboarding-question mb-4 rounded-lg border p-3" data-question=q.id.to_string()>
            <div class="flex items-end gap-2"><label class="field mb-0 flex-1"><span>"Question"</span><input id=format!("{p}-title") type="text" maxlength="100" value=q.title.clone() placeholder="What are you here for?"/></label>{tb("Remove",&format!("cm:q-remove,{}",q.id),"danger-text")}</div>
            <div class="mt-2 flex flex-wrap gap-x-6"><label class="live-checkbox"><span>"Before joining"</span><input id=format!("{p}-pre") type="checkbox" checked=q.pre_join/></label><label class="live-checkbox"><span>"Required"</span><input id=format!("{p}-required") type="checkbox" checked=q.required/></label><label class="live-checkbox"><span>"Several answers"</span><input id=format!("{p}-multi") type="checkbox" checked=q.multi/></label></div>
            {q.answers.iter().map(|an| {let ap=format!("{p}-a{}",an.id); view!{<div class="onboarding-answer mt-3 grid gap-2 rounded-md bg-card p-2 sm:grid-cols-[72px_1fr]">
                <label class="field mb-0"><span>"Emoji"</span><input id=format!("{ap}-emoji") type="text" maxlength="64" value=an.emoji.clone().unwrap_or_default()/></label>
                <label class="field mb-0"><span>"Answer"</span><input id=format!("{ap}-label") type="text" maxlength="50" value=an.label.clone()/></label>
                <label class="field mb-0 sm:col-span-2"><span>"Gives these roles"</span><select id=format!("{ap}-roles") multiple=true size="3">{role_options(&an.role_ids)}</select></label>
                <label class="field mb-0 sm:col-span-2"><span>"Shows these channels"</span><select id=format!("{ap}-channels") multiple=true size="3">{channel_options(a,&an.channel_ids,false)}</select></label>
                <div class="sm:col-span-2">{tb("Remove Answer",&format!("cm:a-remove,{},{}",q.id,an.id),"danger-text")}</div>
            </div>}}).collect_view()}
            <div class="mt-2">{tb("Add an Answer",&format!("cm:a-add,{}",q.id),"secondary")}</div>
        </fieldset>}
    };
    let slot = |i: usize, kind: &str| match kind {
        "hl" => {
            let h = o.welcome.highlights.get(i);
            view!{<div class="grid gap-2 sm:grid-cols-[1fr_72px_2fr]"><label class="field mb-0"><span>{format!("Highlight {}",i+1)}</span><select id=format!("cm-hl{i}-ch")>{channel_options(a,&h.map(|h|vec![h.channel_id]).unwrap_or_default(),true)}</select></label><label class="field mb-0"><span>"Emoji"</span><input id=format!("cm-hl{i}-emoji") type="text" maxlength="64" value=h.and_then(|h|h.emoji.clone()).unwrap_or_default()/></label><label class="field mb-0"><span>"Line of text"</span><input id=format!("cm-hl{i}-text") type="text" maxlength="60" value=h.map(|h|h.text.clone()).unwrap_or_default()/></label></div>}.into_any()
        }
        "task" => {
            let t = o.tasks.get(i);
            view!{<div class="grid gap-2 sm:grid-cols-[120px_1fr_2fr]"><label class="field mb-0"><span>{format!("To-do {}",i+1)}</span><select id=format!("cm-task{i}-kind")><option value="read" selected=t.is_none_or(|t|t.kind==GuideTaskKind::Read)>"Read"</option><option value="talk" selected=t.is_some_and(|t|t.kind==GuideTaskKind::Talk)>"Say hello"</option></select></label><label class="field mb-0"><span>"Channel"</span><select id=format!("cm-task{i}-ch")>{channel_options(a,&t.map(|t|vec![t.channel_id]).unwrap_or_default(),true)}</select></label><label class="field mb-0"><span>"Title"</span><input id=format!("cm-task{i}-title") type="text" maxlength="60" value=t.map(|t|t.title.clone()).unwrap_or_default()/></label></div>}.into_any()
        }
        _ => {
            let r = o.resources.get(i);
            view!{<div class="grid gap-2 sm:grid-cols-[1fr_1fr_2fr]"><label class="field mb-0"><span>{format!("Resource {}",i+1)}</span><select id=format!("cm-res{i}-ch")>{channel_options(a,&r.map(|r|vec![r.channel_id]).unwrap_or_default(),true)}</select></label><label class="field mb-0"><span>"Title"</span><input id=format!("cm-res{i}-title") type="text" maxlength="60" value=r.map(|r|r.title.clone()).unwrap_or_default()/></label><label class="field mb-0"><span>"Description"</span><input id=format!("cm-res{i}-desc") type="text" maxlength="120" value=r.map(|r|r.description.clone()).unwrap_or_default()/></label></div>}.into_any()
        }
    };
    view!{<form data-live-form="cm-onboarding" data-live-change="cm:preview"><h2>"Onboarding"</h2>
        <p class="muted-copy mb-4">"New members answer a few questions after they join. Each answer can give roles and show channels. Roles must be below your highest role and can’t have moderator or admin rights."</p>
        <fieldset disabled=!allowed>
        <label class="live-checkbox"><span>"Turn on onboarding"</span><input id="cm-ob-enabled" type="checkbox" checked=o.enabled/></label>
        <h3 class="mt-6">"Questions"</h3>
        {o.questions.iter().map(question).collect_view()}
        {if o.questions.len()<MAX_ONBOARDING_QUESTIONS{tb("Add a Question","cm:q-add","secondary")}else{().into_any()}}
        <h3 class="mt-6">"Default channels"</h3><p class="muted-copy">"Ticked channels are shown to everyone. Unticked ones are opt-in: members see them when an answer or Browse Channels adds them."</p>
        <div id="cm-defaults" class="grid gap-1 sm:grid-cols-2">{channel_tree(a).into_iter().filter(|(c,_)|c.kind!=ChannelKind::Category).map(|(c,_)|view!{<label class="live-checkbox"><span>{format!("#{}",a.model.channel_name(c))}</span><input id=format!("cm-default-{}",c.id.0) type="checkbox" checked=!o.opt_in_channel_ids.contains(&c.id)/></label>}).collect_view()}</div>
        <h3 class="mt-6">"Welcome screen"</h3>
        <label class="field"><span>"Server description"</span><textarea id="cm-welcome" rows="2" maxlength="300">{o.welcome.description.clone()}</textarea></label>
        <div class="grid gap-3">{(0..MAX_HIGHLIGHTS).map(|i|slot(i,"hl")).collect_view()}</div>
        <h3 class="mt-6">"Server guide"</h3><p class="muted-copy">"A short checklist for new members. “Say hello” is ticked when they send a message there."</p>
        <div class="grid gap-3">{(0..MAX_GUIDE_TASKS).map(|i|slot(i,"task")).collect_view()}</div>
        <h4 class="mt-4 text-sm font-semibold">"Resources"</h4>
        <div class="grid gap-3">{(0..MAX_GUIDE_RESOURCES).map(|i|slot(i,"res")).collect_view()}</div>
        </fieldset>
        <div class="mt-6 flex flex-wrap gap-2"><button type="submit" class="button primary" disabled=!allowed>"Save"</button>{tb("Update Preview","cm:preview","secondary")}</div>{saved_note(a,"cm-onboarding")}
    </form>
    <section class="onboarding-preview mt-8 border-t pt-6" aria-label="Preview"><h3>"Preview"</h3><p class="muted-copy mb-3">"What a new member sees, in order."</p>{preview_steps(a,&s.admission,&o)}</section>}.into_any()
}
/// The flow as cards, for the editor's preview.
fn preview_steps(a: &App, adm: &Admission, o: &Onboarding) -> AnyView {
    let card = |title: String, body: AnyView| view! {<div class="preview-step mb-3 rounded-lg border bg-card p-4"><h4 class="mb-2 text-sm font-semibold">{title}</h4>{body}</div>};
    let qcard = |q: &OnboardingQuestion| {
        card(format!("{}{}", if q.title.is_empty(){"Untitled question"}else{q.title.as_str()}, if q.required{" *"}else{""}), view!{<div class="flex flex-wrap gap-2">{q.answers.iter().map(|an|view!{<span class="answer-chip inline-flex items-center gap-1 rounded-full border px-3 py-1 text-sm">{an.emoji.clone().unwrap_or_default()}" "{if an.label.is_empty(){"Answer".to_owned()}else{an.label.clone()}}</span>}).collect_view()}</div>}.into_any())
    };
    if !o.enabled && !(adm.screening && !adm.rules.is_empty()) {
        return view!{<p class="muted-copy">"Onboarding and rules screening are off: new members go straight in."</p>}.into_any();
    }
    view!{
        {o.enabled.then(|| o.questions.iter().filter(|q|q.pre_join).map(qcard).collect_view())}
        {(adm.screening && !adm.rules.is_empty()).then(|| card("Rules".into(), view!{<ol class="list-decimal pl-5">{adm.rules.iter().map(|r|view!{<li>{rich(r)}</li>}).collect_view()}</ol>}.into_any()))}
        {o.enabled.then(|| o.questions.iter().filter(|q|!q.pre_join).map(qcard).collect_view())}
        {(o.enabled && (!o.welcome.description.is_empty() || !o.welcome.highlights.is_empty())).then(|| card("Welcome".into(), view!{<p class="mb-2">{o.welcome.description.clone()}</p>{o.welcome.highlights.iter().map(|h|view!{<p class="text-sm">{h.emoji.clone().unwrap_or_default()}" "<strong>{channel_label(a,h.channel_id)}</strong>" — "{h.text.clone()}</p>}).collect_view()}}.into_any()))}
        {(o.enabled && !o.tasks.is_empty()).then(|| card("Server guide".into(), view!{<ul>{o.tasks.iter().map(|t|view!{<li class="text-sm">{format!("☐ {} ({})",t.title,channel_label(a,t.channel_id))}</li>}).collect_view()}</ul>}.into_any()))}
    }.into_any()
}

fn discovery(a: &App) -> AnyView {
    let Some((_, d)) = a
        .community
        .listing
        .as_ref()
        .filter(|(g, _)| Some(*g) == a.guild)
    else {
        return loading("Discovery");
    };
    let allowed = settings::can(a, Permissions::MANAGE_GUILD);
    let ready = d.requirements.iter().all(|r| r.met);
    let draft = d
        .listing
        .as_ref()
        .map(|l| l.draft.clone())
        .unwrap_or(ListingDraft {
            language: "en".into(),
            ..Default::default()
        });
    let emoji: Vec<(Id, String)> = crate::live::expression::usable(a)
        .into_iter()
        .filter(|(g, _)| Some(*g) == a.guild)
        .map(|(_, e)| (e.id, e.name.clone()))
        .collect();
    view!{<h2>"Discovery"</h2><p class="muted-copy mb-4">"List this server in this StickTalk instance’s directory so people can find it and join, following your rules screening or applications."</p>
        {(!d.directory_enabled).then(|| view!{<p class="mb-4 rounded-lg border border-warning bg-card p-3" role="status">"The administrator of this StickTalk instance has turned the directory off."</p>})}
        {d.listing.as_ref().map(|l| view!{<p class="mb-4 flex flex-wrap gap-2" role="status">{badge("Listed","border-primary")}{l.featured.then(||badge("Featured by the administrator","border-primary"))}{l.hidden.then(||badge("Hidden by the administrator","border-warning"))}{l.delisted.then(||badge("Removed by the administrator","border-destructive"))}</p>})}
        <h3>"Requirements"</h3><ul class="listing-requirements mb-6 grid gap-2">{d.requirements.iter().map(|r|view!{<li class="flex items-center gap-2" data-met=r.met.to_string()>{icon_with(if r.met{"check"}else{"x"},if r.met{"icon size-4 text-primary"}else{"icon size-4 text-destructive"})}<span>{r.label.clone()}</span><span class="sr-only">{if r.met{"(done)"}else{"(not yet)"}}</span></li>}).collect_view()}</ul>
        <form data-live-form="cm-listing"><fieldset disabled=!allowed||!d.directory_enabled>
            <label class="field"><span>"Category"</span><select id="cm-listing-category">{DirectoryCategory::ALL.iter().map(|c|view!{<option value=c.name() selected=*c==draft.category>{c.label()}</option>}).collect_view()}</select></label>
            <label class="field"><span>"Description"</span><textarea id="cm-listing-description" rows="3" maxlength="300">{draft.description.clone()}</textarea></label>
            <div class="field"><label for="cm-listing-tags">"Tags (up to 5, separated by commas)"</label><input id="cm-listing-tags" type="text" maxlength="120" value=draft.tags.join(", ")/></div>
            <div class="field"><label for="cm-listing-language">"Language"</label><input id="cm-listing-language" type="text" maxlength="10" value=draft.language.clone()/></div>
            <label class="field"><span>"Featured emoji (up to 5)"</span><select id="cm-listing-emoji" multiple=true size="4">{emoji.iter().map(|(id,n)|view!{<option value=id.0.to_string() selected=draft.featured_emoji.contains(id)>{format!(":{n}:")}</option>}).collect_view()}</select></label>
            {emoji.is_empty().then(|| view!{<p class="muted-copy">"Add custom emoji to feature them here."</p>})}
        </fieldset>
        <div class="mt-4 flex flex-wrap gap-2"><button type="submit" class="button primary" disabled=!allowed||!ready||!d.directory_enabled>{if d.listing.is_some(){"Save Listing"}else{"List This Server"}}</button>{if d.listing.is_some()&&allowed{tb("Remove from Directory","cm:unlist","danger-text")}else{().into_any()}}</div>{saved_note(a,"cm-listing")}
        {(!ready).then(|| view!{<p class="muted-copy mt-2">"Meet every requirement to list this server."</p>})}
        </form>
    }.into_any()
}

fn templates(a: &App) -> AnyView {
    let list = a
        .community
        .templates
        .as_ref()
        .filter(|(g, _)| Some(*g) == a.guild)
        .map(|(_, l)| l);
    view!{<form data-live-form="cm-template-create"><h2>"Server Templates"</h2>
        <p class="muted-copy mb-4">"A template copies this server’s structure: channels and their nesting, roles and permissions, onboarding, AutoMod and settings. Never members, messages, invites or files. Anyone with its link can start a server from it."</p>
        <div class="field"><label for="cm-template-name">"Template name"</label><input id="cm-template-name" type="text" maxlength="100"/></div>
        <div class="field"><label for="cm-template-description">"Description"</label><input id="cm-template-description" type="text" maxlength="120"/></div>
        <button type="submit" class="button primary">"Create Template"</button>{saved_note(a,"cm-template-create")}</form>
        <div class="settings-divider"></div>
        {match list {
            None => view!{<p class="muted-copy" role="status">"Loading…"</p>}.into_any(),
            Some(l) if l.is_empty() => view!{<p class="muted-copy">"No templates yet."</p>}.into_any(),
            Some(l) => l.iter().map(|t| view!{<div class="template-row settings-list-row flex flex-wrap items-center gap-3 border-b py-4"><span class="settings-row-copy flex min-w-24 flex-1 flex-col gap-1 wrap-anywhere"><strong class="font-semibold">{t.name.clone()}{t.out_of_sync.then(||view!{<span class="ml-2">{badge("Out of sync","border-warning")}</span>})}</strong><small>{format!("{} · {} · last synced {}",plural(t.uses,"use","uses"),if t.description.is_empty(){"No description".to_owned()}else{t.description.clone()},timestamp(t.updated_at))}</small><code class="text-xs">{t.code.clone()}</code></span>
                <button type="button" class="button secondary" data-copy=format!("{}/?template={}",a.origin,t.code)>"Copy Link"</button>{tb("Sync",&format!("cm:template-sync,{}",t.code),"secondary")}{tb("Delete",&format!("cm:template-delete,{}",t.code),"danger-text")}</div>}).collect_view().into_any(),
        }}
    }.into_any()
}

// ---------------------------------------------------------------- insights

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn date(day: u32) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(f64::from(day) * 86_400_000.0));
    format!("{}/{}", d.get_utc_month() + 1, d.get_utc_date())
}
/// Rounded, readable top of a scale: 1, 2, 5 × 10^n.
fn nice(max: u32) -> u32 {
    let max = max.max(1);
    let mut step = 1;
    loop {
        for m in [1, 2, 5] {
            if step * m >= max {
                return step * m;
            }
        }
        step *= 10;
    }
}
/// A bar chart as inline SVG. Bars grow from a shared baseline, one shared
/// axis, 2px gaps; a hover target per column names every value. The SVG is
/// hidden from assistive technology, which reads the table after it instead.
fn bars(title: &str, labels: &[String], series: &[(&str, &str, Vec<u32>)]) -> AnyView {
    let (w, h, left, top, bottom) = (640.0, 180.0, 40.0, 10.0, 24.0);
    let n = labels.len().max(1);
    let max = nice(
        series
            .iter()
            .flat_map(|s| s.2.iter().copied())
            .max()
            .unwrap_or(0),
    );
    let plot_w = w - left - 8.0;
    let plot_h = h - top - bottom;
    let col = plot_w / n as f64;
    let k = series.len().max(1) as f64;
    let bar_w = ((col - 2.0) / k - 2.0).clamp(1.0, 24.0);
    let mut svg = String::new();
    let _ = write!(
        svg,
        r#"<svg viewBox="0 0 {w} {h}" class="h-auto w-full" aria-hidden="true" focusable="false">"#
    );
    let ticks: Vec<u32> = if max.is_multiple_of(2) {
        vec![0, max / 2, max]
    } else {
        vec![0, max]
    };
    for (i, v) in ticks.iter().enumerate() {
        let y = top + plot_h - plot_h * f64::from(*v) / f64::from(max);
        let _ = write!(
            svg,
            r#"<line x1="{left}" x2="{}" y1="{y:.1}" y2="{y:.1}" class="stroke-border" stroke-width="1"{} />"#,
            w - 8.0,
            if i == 0 {
                ""
            } else {
                r#" stroke-dasharray="2 3""#
            }
        );
        let _ = write!(
            svg,
            r#"<text x="{}" y="{:.1}" text-anchor="end" class="fill-muted-foreground" font-size="11">{v}</text>"#,
            left - 6.0,
            y + 4.0
        );
    }
    for (i, label) in labels.iter().enumerate() {
        let x0 = left + col * i as f64;
        for (j, (_, class, values)) in series.iter().enumerate() {
            let v = values.get(i).copied().unwrap_or(0);
            if v == 0 {
                continue;
            }
            let bh = (plot_h * f64::from(v) / f64::from(max)).max(1.0);
            let x = x0 + 1.0 + (col - (bar_w + 2.0) * k) / 2.0 + (bar_w + 2.0) * j as f64;
            let y = top + plot_h - bh;
            let r = 4.0_f64.min(bar_w / 2.0).min(bh);
            let base = top + plot_h;
            let _ = write!(
                svg,
                r#"<path class="{class}" d="M{x:.1},{base:.1}V{:.1}Q{x:.1},{y:.1} {:.1},{y:.1}H{:.1}Q{:.1},{y:.1} {:.1},{:.1}V{base:.1}Z" />"#,
                y + r,
                x + r,
                x + bar_w - r,
                x + bar_w,
                x + bar_w,
                y + r
            );
        }
        let tip = series
            .iter()
            .map(|(name, _, values)| format!("{name}: {}", values.get(i).copied().unwrap_or(0)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(
            svg,
            r#"<rect x="{x0:.1}" y="{top}" width="{col:.1}" height="{plot_h}" fill="transparent"><title>{}: {}</title></rect>"#,
            esc(label),
            esc(&tip)
        );
        if i == 0 || i + 1 == labels.len() || i == labels.len() / 2 {
            let anchor = if i == 0 {
                "start"
            } else if i + 1 == labels.len() {
                "end"
            } else {
                "middle"
            };
            let tx = if i == 0 {
                x0
            } else if i + 1 == labels.len() {
                x0 + col
            } else {
                x0 + col / 2.0
            };
            let _ = write!(
                svg,
                r#"<text x="{tx:.1}" y="{}" text-anchor="{anchor}" class="fill-muted-foreground" font-size="11">{}</text>"#,
                h - 6.0,
                esc(label)
            );
        }
    }
    svg.push_str("</svg>");
    let legend = (series.len() > 1).then(|| view!{<div class="chart-legend mb-1 flex flex-wrap gap-4 text-xs text-subtle">{series.iter().map(|(name,class,values)|view!{<span class="inline-flex items-center gap-1"><svg class="size-2.5" viewBox="0 0 10 10" aria-hidden="true"><rect width="10" height="10" rx="2" class=class.to_string()/></svg>{format!("{name} ({})",values.iter().sum::<u32>())}</span>}).collect_view()}</div>});
    view!{<figure class="insight-chart mb-6 rounded-lg border bg-card p-4"><figcaption class="mb-2 text-sm font-semibold">{title.to_owned()}</figcaption>{legend}<div inner_html=svg></div>
        <table class="sr-only"><caption>{title.to_owned()}</caption><thead><tr><th scope="col">"Period"</th>{series.iter().map(|s|view!{<th scope="col">{s.0.to_owned()}</th>}).collect_view()}</tr></thead><tbody>{labels.iter().enumerate().map(|(i,l)|view!{<tr><th scope="row">{l.clone()}</th>{series.iter().map(|s|view!{<td>{s.2.get(i).copied().unwrap_or(0).to_string()}</td>}).collect_view()}</tr>}).collect_view()}</tbody></table>
    </figure>}.into_any()
}
/// Horizontal bars for a ranked list, with the numbers written out.
fn ranking(title: &str, rows: Vec<(String, u32, String)>) -> AnyView {
    let max = rows.iter().map(|r| r.1).max().unwrap_or(0).max(1);
    view!{<figure class="insight-ranking mb-6 rounded-lg border bg-card p-4"><figcaption class="mb-3 text-sm font-semibold">{title.to_owned()}</figcaption>
        {if rows.is_empty(){view!{<p class="muted-copy">"Nothing yet."</p>}.into_any()}else{view!{<table class="w-full text-sm"><tbody>{rows.into_iter().map(|(label,v,note)|view!{<tr><th scope="row" class="w-2/5 py-1 pr-3 text-left font-normal wrap-anywhere">{label}</th><td class="py-1"><span class="block h-2 rounded-r bg-primary" style=format!("width:{:.0}%",100.0*f64::from(v)/f64::from(max)) aria-hidden="true"></span></td><td class="w-28 py-1 pl-3 text-right tabular-nums">{v.to_string()}<span class="ml-1 text-xs text-subtle">{note}</span></td></tr>}).collect_view()}</tbody></table>}.into_any()}}
    </figure>}.into_any()
}
fn tile(label: &str, value: String) -> AnyView {
    view!{<div class="stat-tile rounded-lg border bg-card p-3"><div class="text-xs text-subtle">{label.to_owned()}</div><div class="text-2xl font-semibold tabular-nums">{value}</div></div>}.into_any()
}
fn insights(a: &App) -> AnyView {
    let Some((_, i)) = a
        .community
        .insights
        .as_ref()
        .filter(|(g, _)| Some(*g) == a.guild)
    else {
        return loading("Insights");
    };
    let weekly = a.community.weekly;
    // Days, or weeks ending today.
    let buckets: Vec<(String, Vec<&InsightDay>)> = if weekly {
        let mut out: Vec<(String, Vec<&InsightDay>)> = vec![];
        for chunk in i.days.rchunks(7) {
            out.push((
                format!(
                    "{}–{}",
                    date(chunk[0].day),
                    date(chunk[chunk.len() - 1].day)
                ),
                chunk.iter().collect(),
            ));
        }
        out.reverse();
        out
    } else {
        i.days.iter().map(|d| (date(d.day), vec![d])).collect()
    };
    let labels: Vec<String> = buckets.iter().map(|b| b.0.clone()).collect();
    let sum = |f: fn(&InsightDay) -> u32| {
        buckets
            .iter()
            .map(|b| b.1.iter().map(|d| f(d)).sum())
            .collect::<Vec<u32>>()
    };
    let total = |f: fn(&InsightDay) -> u32| i.days.iter().map(f).sum::<u32>();
    let (joins, leaves) = (total(|d| d.joins), total(|d| d.leaves));
    let net = i64::from(joins) - i64::from(leaves);
    let retention: Vec<&RetentionCohort> = i.retention.iter().filter(|r| r.joined > 0).collect();
    let pct = |v: Option<u32>, n: u32| {
        v.map_or("—".to_owned(), |v| {
            format!("{}%", (100 * v).checked_div(n).unwrap_or(0))
        })
    };
    let funnel_first = i.funnel.first().map_or(0, |f| f.reached);
    view!{<h2>"Insights"</h2><p class="muted-copy mb-4">"Counts for this server, updated as things happen. Only totals are shown, never what one member did."</p>
        <div class="mb-4 flex flex-wrap items-end gap-3"><label class="field mb-0 w-40" data-live-change="cm:insights-days"><span>"Period"</span><select id="cm-insight-days">{[7u32,30,90,120].into_iter().map(|d|view!{<option value=d.to_string() selected=d==a.community.insight_days>{format!("Last {d} days")}</option>}).collect_view()}</select></label>
            <button type="button" class="button secondary" aria-pressed=weekly.to_string() data-live="cm:insights-weekly">{if weekly{"Show by Day"}else{"Show by Week"}}</button></div>
        <div class="mb-6 grid grid-cols-2 gap-3 sm:grid-cols-3">{tile("Members",i.member_count.to_string())}{tile("Joined",joins.to_string())}{tile("Left",leaves.to_string())}{tile("Net growth",format!("{}{net}",if net>0{"+"}else{""}))}{tile("Messages",total(|d|d.messages).to_string())}{tile("Voice minutes",total(|d|d.voice_minutes).to_string())}</div>
        {bars("Member growth",&labels,&[("Joined","fill-primary",sum(|d|d.joins)),("Left","fill-muted-foreground",sum(|d|d.leaves))])}
        {bars("Messages",&labels,&[("Messages","fill-primary",sum(|d|d.messages))])}
        {bars("Voice minutes",&labels,&[("Voice minutes","fill-primary",sum(|d|d.voice_minutes))])}
        {ranking("Where joins came from",i.sources.iter().map(|s|(match s.source{JoinSource::Invite=>format!("Invite {}",s.code),other=>other.label().to_owned()},s.joins,String::new())).collect())}
        {ranking("Most active channels",i.channels.iter().map(|c|(format!("#{}",c.name),c.messages,if c.voice_minutes>0{format!("+{} voice min",c.voice_minutes)}else{"messages".into()})).collect())}
        <figure class="insight-retention mb-6 rounded-lg border bg-card p-4"><figcaption class="mb-3 text-sm font-semibold">"New-member retention"</figcaption><p class="muted-copy mb-2">"Of the people who joined each day: still here and had talked after 1 and 7 days."</p>
            {if retention.is_empty(){view!{<p class="muted-copy">"No new members in this period."</p>}.into_any()}else{view!{<div class="overflow-x-auto"><table class="w-full text-sm tabular-nums"><thead><tr><th scope="col" class="text-left">"Joined"</th><th scope="col" class="text-right">"People"</th><th scope="col" class="text-right">"After 1 day"</th><th scope="col" class="text-right">"After 7 days"</th></tr></thead><tbody>{retention.iter().rev().take(30).map(|r|view!{<tr><th scope="row" class="text-left font-normal">{date(r.day)}</th><td class="text-right">{r.joined.to_string()}</td><td class="text-right">{pct(r.retained_1d,r.joined)}</td><td class="text-right">{pct(r.retained_7d,r.joined)}</td></tr>}).collect_view()}</tbody></table></div>}.into_any()}}
        </figure>
        {ranking("Onboarding: members reaching each step",i.funnel.iter().map(|f|(f.label.clone(),f.reached,if funnel_first>0&&f.reached<funnel_first{format!("−{}%",100-(100*f.reached)/funnel_first)}else{String::new()})).collect())}
    }.into_any()
}

// ---------------------------------------------------------------- member flows

fn answer_chips(q: &OnboardingQuestion, chosen: &[u32]) -> AnyView {
    view!{<div class="answer-grid grid gap-2 sm:grid-cols-2" role="group" aria-label=q.title.clone()>{q.answers.iter().map(|an|{let on=chosen.contains(&an.id);view!{<button type="button" class=format!("answer-choice flex min-h-12 items-center gap-3 rounded-lg border px-3 py-2 text-left {}",if on{"border-primary bg-secondary"}else{"bg-card hover:bg-accent"}) aria-pressed=on.to_string() data-live=format!("cm:pick,{},{}",q.id,an.id)>{an.emoji.clone().map(|e|view!{<span class="text-xl" aria-hidden="true">{e}</span>})}<span class="flex-1 font-medium">{an.label.clone()}</span>{on.then(||icon_with("check","icon size-4 text-primary"))}</button>}}).collect_view()}</div>}.into_any()
}
fn flow(a: &App) -> AnyView {
    let Some(f) = a.community.flow.as_ref() else {
        return view! {<p role="status">"Loading…"</p>}.into_any();
    };
    let Some(s) = a.community.states.get(&f.guild) else {
        return view! {<p role="status">"Loading…"</p>}.into_any();
    };
    let guild = a
        .model
        .guilds
        .get(&f.guild)
        .map_or(String::new(), |g| g.guild.name.clone());
    let o = &s.onboarding;
    if f.revisit {
        return view!{<h2>"Channels & Roles"</h2><p class="muted-copy mb-4">{format!("Change your answers to update your roles and channels in {guild}.")}</p>
            {o.questions.iter().map(|q|view!{<section class="mb-5"><h3 class="mb-2">{q.title.clone()}{q.multi.then(||view!{<small class="ml-2 font-normal text-subtle">"Choose any"</small>})}</h3>{answer_chips(q,f.answers.get(&q.id).map_or(&[][..],|v|v.as_slice()))}</section>}).collect_view()}
            <div class="flex flex-wrap gap-2">{tb("Save","cm:save-answers","primary")}{tb("Browse Channels","dialog:cm-browse","secondary")}</div>
        }.into_any();
    }
    let list = steps(s, f.rules);
    let Some(step) = list.get(f.step).copied() else {
        return view! {<h2>{format!("Welcome to {guild}")}</h2>{tb("Done","close","primary")}}
            .into_any();
    };
    let last = f.step + 1 == list.len();
    let progress = view! {<div class="mb-4"><p class="mb-1 text-xs text-subtle">{format!("Step {} of {}",f.step+1,list.len())}</p><div class="h-1 rounded-full bg-muted" role="progressbar" aria-valuemin="1" aria-valuemax=list.len().to_string() aria-valuenow=(f.step+1).to_string()><div class="h-1 rounded-full bg-primary" style=format!("width:{}%",100*(f.step+1)/list.len())></div></div></div>};
    let nav = |next: &str, skip: bool| view! {<div class="mt-6 flex flex-wrap gap-2 phone:flex-col-reverse">{(f.step>0).then(||tb("Back","cm:back","secondary"))}{skip.then(||tb("Skip","cm:skip","secondary"))}{tb(next,"cm:next","primary")}</div>};
    let body = match step {
        Step::Question(i) => {
            let q = &o.questions[i];
            view!{<h2>{q.title.clone()}</h2><p class="muted-copy mb-4">{if q.pre_join&&q.required{format!("{guild} asks this before you start talking.")}else if q.multi{"Choose all that apply.".to_owned()}else{"Choose one.".to_owned()}}</p>
                {answer_chips(q,f.answers.get(&q.id).map_or(&[][..],|v|v.as_slice()))}{nav(if last{"Finish"}else{"Next"},!q.required)}}.into_any()
        }
        Step::Rules => view!{<h2>{format!("Rules of {guild}")}</h2>{(!s.admission.description.is_empty()).then(||view!{<div class="mb-3">{rich(&s.admission.description)}</div>})}
            <ol class="rules-list mb-4 grid list-decimal gap-2 rounded-lg border bg-card py-3 pr-3 pl-8">{s.admission.rules.iter().map(|r|view!{<li>{rich(r)}</li>}).collect_view()}</ol>
            <label class="live-checkbox rounded-lg border px-3 py-2"><span class="font-medium">"I have read and agree to the rules"</span><input id="cm-agree" type="checkbox" checked=f.agreed data-live="cm:agree"/></label>
            {nav(if last{"Accept and Finish"}else{"Accept and Continue"},false)}}.into_any(),
        Step::Welcome => view!{<h2>{format!("Welcome to {guild}!")}</h2><div class="mb-4">{rich(&o.welcome.description)}</div>
            <div class="grid gap-2">{o.welcome.highlights.iter().map(|h|view!{<button type="button" class="welcome-highlight flex items-center gap-3 rounded-lg border bg-card p-3 text-left hover:bg-accent" data-live=format!("cm:open,{}",h.channel_id.0)><span class="text-xl" aria-hidden="true">{h.emoji.clone().unwrap_or_else(||"#".into())}</span><span class="flex min-w-0 flex-col"><strong class="font-semibold">{channel_label(a,h.channel_id)}</strong><small class="text-subtle">{h.text.clone()}</small></span></button>}).collect_view()}</div>
            {nav("Finish",false)}}.into_any(),
    };
    view! {{progress}{body}}.into_any()
}
fn browse(a: &App) -> AnyView {
    let Some(s) = state(a) else {
        return loading("Browse Channels");
    };
    let o = &s.onboarding;
    view!{<h2>"Browse Channels"</h2><p class="muted-copy mb-4">"Choose which optional channels appear in your channel list. Mentions in a hidden channel still notify you and show it."</p>
        <label class="live-checkbox mb-4 rounded-lg border px-3 py-2"><span class="flex flex-col"><strong class="font-semibold">"Show All Channels"</strong><small class="text-subtle">"See every channel you have access to."</small></span><input id="cm-show-all" type="checkbox" checked=s.me.show_all/></label>
        <div class="browse-list grid gap-1">{channel_tree(a).into_iter().map(|(c,depth)|{
            let label=a.model.channel_name(c);
            if c.kind==ChannelKind::Category{return view!{<h3 class="category mt-3" style=indent(depth,16)>{label}</h3>}.into_any()}
            let optional=o.opt_in_channel_ids.contains(&c.id);
            view!{<label class="live-checkbox" style=indent(depth,16)><span class="flex flex-col"><span>{format!("#{label}")}</span>{(!optional).then(||view!{<small class="text-subtle">"Everyone sees this"</small>})}</span><input id=format!("cm-browse-{}",c.id.0) type="checkbox" checked=!optional||s.me.opted_in.contains(&c.id) disabled=!optional/></label>}.into_any()
        }).collect_view()}</div>
        <div class="mt-4">{tb("Save","cm:browse-save","primary")}</div>
    }.into_any()
}
fn guide(a: &App) -> AnyView {
    let Some(s) = state(a) else {
        return loading("Server Guide");
    };
    let o = &s.onboarding;
    let guild = a
        .guild
        .and_then(|g| a.model.guilds.get(&g))
        .map_or(String::new(), |g| g.guild.name.clone());
    let done = o
        .tasks
        .iter()
        .filter(|t| s.me.guide_done.contains(&t.id))
        .count();
    view!{<h2>{format!("{guild} Guide")}</h2>{(!o.welcome.description.is_empty()).then(||view!{<div class="mb-4">{rich(&o.welcome.description)}</div>})}
        {(!o.tasks.is_empty()).then(||view!{<section class="mb-6"><h3 class="mb-1">"Getting started"</h3><p class="mb-2 text-xs text-subtle" role="status">{format!("{done} of {} done",o.tasks.len())}</p>
            <ul class="guide-tasks grid gap-2">{o.tasks.iter().map(|t|{let ok=s.me.guide_done.contains(&t.id);view!{<li class="flex items-center gap-3 rounded-lg border bg-card p-3" data-done=ok.to_string()>{icon_with(if ok{"check"}else{"circle"},if ok{"icon size-5 text-primary"}else{"icon size-5 text-muted-foreground"})}<span class="flex min-w-0 flex-1 flex-col"><span class=if ok{"line-through text-subtle"}else{"font-medium"}>{t.title.clone()}</span><small class="text-subtle">{format!("{} in {}",if t.kind==GuideTaskKind::Talk{"Send a message"}else{"Read"},channel_label(a,t.channel_id))}</small><span class="sr-only">{if ok{"Done"}else{"Not done yet"}}</span></span>{tb("Go",&format!("cm:open,{}",t.channel_id.0),"secondary")}</li>}}).collect_view()}</ul></section>})}
        {(!o.resources.is_empty()).then(||view!{<section class="mb-6"><h3 class="mb-2">"Resources"</h3><div class="grid gap-2 sm:grid-cols-2">{o.resources.iter().map(|r|view!{<button type="button" class="flex flex-col items-start rounded-lg border bg-card p-3 text-left hover:bg-accent" data-live=format!("cm:open,{}",r.channel_id.0)><strong class="font-semibold">{r.title.clone()}</strong><small class="text-subtle">{if r.description.is_empty(){channel_label(a,r.channel_id)}else{r.description.clone()}}</small></button>}).collect_view()}</div></section>})}
        <div class="flex flex-wrap gap-2">{(o.enabled&&!o.questions.is_empty()).then(||tb("Channels & Roles","cm:flow,revisit","secondary"))}{(!o.opt_in_channel_ids.is_empty()).then(||tb("Browse Channels","dialog:cm-browse","secondary"))}</div>
    }.into_any()
}
fn rules(a: &App) -> AnyView {
    let Some(s) = state(a) else {
        return loading("Server Rules");
    };
    view!{<h2>"Server Rules"</h2>{(!s.admission.description.is_empty()).then(||view!{<div class="mb-3">{rich(&s.admission.description)}</div>})}<ol class="grid list-decimal gap-2 pl-6">{s.admission.rules.iter().map(|r|view!{<li>{rich(r)}</li>}).collect_view()}</ol>
        {if s.me.rules_pending{view!{<div class="mt-4">{tb("Accept the Rules","cm:flow","primary")}</div>}.into_any()}else{view!{<p class="muted-copy mt-4">"You accepted these rules."</p>}.into_any()}}}.into_any()
}
fn apply(a: &App) -> AnyView {
    let gate = match crate::live::community::apply_via(a) {
        Some(JoinVia::Invite { code }) => a
            .community
            .gate
            .as_ref()
            .filter(|(c, _)| *c == code)
            .map(|(_, g)| g),
        Some(JoinVia::Discovery { guild_id }) => a
            .community
            .preview
            .as_ref()
            .map(|p| &p.gate)
            .filter(|g| g.guild_id == guild_id),
        None => None,
    };
    let Some(g) = gate else {
        return loading("Apply to Join");
    };
    view!{<form data-live-form="cm-apply"><h2>{format!("Apply to {}",g.name)}</h2><p class="muted-copy mb-4">"A moderator reviews your application. You can see its status in My Applications, and withdraw it there."</p>
        {g.questions.iter().enumerate().map(|(i,q)|{let id=format!("cm-answer-{i}");let label=format!("{}{}",q.prompt,if q.required{" (required)"}else{""});match q.kind{
            QuestionKind::Short=>view!{<div class="field"><label for=id.clone()>{label}</label><input id=id type="text" maxlength="150" required=q.required/></div>}.into_any(),
            QuestionKind::Paragraph=>view!{<div class="field"><label for=id.clone()>{label}</label><textarea id=id rows="4" maxlength="1000" required=q.required></textarea></div>}.into_any(),
            QuestionKind::Choice=>view!{<div class="field"><label for=id.clone()>{label}</label><select id=id>{(!q.required).then(||view!{<option value="">"No answer"</option>})}{q.choices.iter().map(|c|view!{<option value=c.clone()>{c.clone()}</option>}).collect_view()}</select></div>}.into_any(),
        }}).collect_view()}
        {g.screening.then(||view!{<h3 class="mt-4">"Rules"</h3>{(!g.screening_description.is_empty()).then(||rich(&g.screening_description))}<ol class="mb-3 grid list-decimal gap-1 pl-6">{g.rules.iter().map(|r|view!{<li>{rich(r)}</li>}).collect_view()}</ol><label class="live-checkbox rounded-lg border px-3 py-2"><span class="font-medium">"I have read and agree to the rules"</span><input id="cm-apply-rules" type="checkbox"/></label>})}
        <button type="submit" class="button primary mt-4">"Submit Application"</button>
    </form>}.into_any()
}
fn mine(a: &App) -> AnyView {
    view!{<h2>"My Applications"</h2>{if a.community.mine.is_empty(){view!{<p class="muted-copy">"You haven’t applied to any server."</p>}.into_any()}else{().into_any()}}
        {a.community.mine.iter().map(|m|view!{<div class="my-application settings-list-row flex flex-wrap items-center gap-3 border-b py-3"><span class="flex min-w-24 flex-1 flex-col gap-1"><strong class="font-semibold wrap-anywhere">{m.guild_name.clone()}</strong><small class="text-subtle">{format!("Applied {}",timestamp(m.created_at))}{m.reason.clone().map(|r|format!(" · {r}"))}</small></span>
            {badge(match m.status{ApplicationStatus::Pending=>"Waiting for review",ApplicationStatus::Approved=>"Approved",ApplicationStatus::Rejected=>"Declined",ApplicationStatus::Withdrawn=>"Withdrawn"},if m.status==ApplicationStatus::Approved{"border-primary"}else{""})}
            {m.interview_channel_id.filter(|_|m.status==ApplicationStatus::Pending).map(|c|tb("Open Interview",&format!("cm:open,{}",c.0),"secondary"))}
            {match m.status{ApplicationStatus::Pending=>tb("Withdraw",&format!("cm:withdraw,{}",m.guild_id.0),"danger-text"),ApplicationStatus::Approved=>tb("Open Server",&format!("cm:go,{}",m.guild_id.0),"secondary"),_=>().into_any()}}</div>}).collect_view()}
    }.into_any()
}

// ---------------------------------------------------------------- discover

fn entry_icon(e: &DirectoryEntry, size: &str) -> AnyView {
    match e.icon_id {
        Some(i) => view!{<img class=format!("{size} flex-none rounded-2xl object-cover") src=format!("/api/v1/files/{}/thumbnail",i.0) alt="" loading="lazy"/>}.into_any(),
        None => view!{<span class=format!("{size} grid flex-none place-items-center rounded-2xl bg-muted font-semibold") aria-hidden="true">{initials(&e.name)}</span>}.into_any(),
    }
}
const BANNERS: [(u32, u32); 10] = [
    (0x376f61, 0x1c3436),
    (0x487ea0, 0x29364e),
    (0x78609f, 0x37304b),
    (0xa05e79, 0x4b3041),
    (0xa67c50, 0x49382d),
    (0x408f8d, 0x253e4d),
    (0x646cb0, 0x323350),
    (0x965b9e, 0x49304d),
    (0xa19350, 0x4d4229),
    (0x687480, 0x323941),
];
fn banner(color: u8) -> String {
    let (start, end) = BANNERS[usize::from(color.min(9))];
    format!("background:linear-gradient(135deg,#{start:06x},#{end:06x})")
}
fn counts(e: &DirectoryEntry) -> String {
    format!(
        "{} online · {}",
        e.online_count,
        plural(e.member_count, "member", "members")
    )
}
fn card(e: &DirectoryEntry) -> AnyView {
    view!{<article class="directory-card flex flex-col overflow-hidden rounded-lg border bg-card" data-guild=e.guild_id.0.to_string()>
        <div class="h-16" style=banner(e.banner_color)></div>
        <div class="-mt-6 flex flex-1 flex-col gap-2 p-3">{entry_icon(e,"size-12 border-4 border-card")}
            <h4 class="flex items-center gap-2 font-semibold wrap-anywhere">{e.name.clone()}{e.featured.then(||icon_with("star","icon size-4 text-primary"))}</h4>
            <p class="line-clamp-3 text-sm text-subtle">{e.description.clone()}</p>
            <div class="flex flex-wrap gap-1">{e.tags.iter().map(|t|view!{<span class="rounded-full bg-muted px-2 py-0.5 text-xs">{t.clone()}</span>}).collect_view()}</div>
            <p class="mt-auto text-xs text-subtle">{counts(e)}</p>
            <button type="button" class="button secondary" data-live=format!("dialog:discover:{}",e.guild_id.0) aria-label=format!("View {}",e.name)>{if e.joined{"Joined · View"}else{"View"}}</button>
        </div></article>}.into_any()
}
fn grid(title: &str, list: &[DirectoryEntry]) -> AnyView {
    if list.is_empty() {
        return ().into_any();
    }
    view!{<section class="mb-8"><h3 class="mb-3">{title.to_owned()}</h3><div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">{list.iter().map(card).collect_view()}</div></section>}.into_any()
}
fn discover_nav(a: &App) -> AnyView {
    let counts: Vec<(DirectoryCategory, u32)> = a
        .community
        .directory
        .as_ref()
        .map(|d| d.categories.clone())
        .unwrap_or_default();
    view!{<h2 class="mx-2 text-xl">"Discover"</h2><h3 class="category mx-2 mt-6 mb-2">"Browse"</h3>
        {settings_nav_row("compass","Home","cm:category,",a.community.category.is_none()&&a.community.query.is_empty())}
        {counts.into_iter().map(|(c,n)|settings_nav_row("hash",&format!("{} ({n})",c.label()),&format!("cm:category,{}",c.name()),a.community.category==Some(c))).collect_view()}
        <h3 class="category mx-2 mt-6 mb-2">"You"</h3>{settings_nav_row("file","My Applications","dialog:cm-mine",false)}}.into_any()
}
fn discover(a: &App) -> AnyView {
    let body = match &a.community.directory {
        None => view!{<p class="muted-copy" role="status">"Loading…"</p>}.into_any(),
        Some(d) if !d.enabled => view!{<div class=SETTINGS_EMPTY>{icon("compass")}<p>"The server directory is turned off on this StickTalk instance."</p></div>}.into_any(),
        Some(d) => {
            let searching = !a.community.query.is_empty() || a.community.category.is_some();
            view!{
                <form data-live-form="cm-search" class="mb-4 flex gap-2" role="search"><label class="sr-only" for="cm-directory-q">"Search servers by name or tag"</label><input id="cm-directory-q" class="min-w-0 flex-1" type="search" maxlength="100" placeholder="Search servers by name or tag" value=a.community.query.clone()/><button type="submit" class="button primary">"Search"</button></form>
                <div class="mb-6 hidden flex-wrap gap-2 phone:flex">{tb("Home","cm:category,","secondary")}{d.categories.iter().filter(|(_,n)|*n>0).map(|(c,_)|tb(c.label(),&format!("cm:category,{}",c.name()),if a.community.category==Some(*c){"primary"}else{"secondary"})).collect_view()}{tb("My Applications","dialog:cm-mine","secondary")}</div>
                {if searching {view!{{grid(&a.community.category.map_or_else(||format!("Results for “{}”",a.community.query),|c|c.label().to_owned()),&d.results)}{d.results.is_empty().then(||view!{<p class="muted-copy">"No servers match. Try another word or category."</p>})}}.into_any()}
                else {view!{{grid("Featured",&d.featured)}{grid("Recently active",&d.active)}{d.active.is_empty().then(||view!{<div class=SETTINGS_EMPTY>{icon("compass")}<p>"No servers are listed yet. Owners can list theirs in Server Settings › Discovery."</p></div>})}}.into_any()}}
            }.into_any()
        }
    };
    settings_page(
        a,
        SettingsPage {
            root: false,
            label: "Discover",
            nav_label: "Directory categories",
            mobile_title: "Discover",
            back: ("Close", "close"),
            title: "Discover Servers",
        },
        discover_nav(a),
        body,
    )
}
fn preview(a: &App) -> AnyView {
    let Some(p) = a.community.preview.as_ref() else {
        return view! {<p role="status">"Loading…"</p>}.into_any();
    };
    let e = &p.entry;
    let g = &p.gate;
    let status = g
        .application
        .as_ref()
        .filter(|m| m.status == ApplicationStatus::Pending);
    let action = if e.joined {
        tb("Open Server", &format!("cm:go,{}", e.guild_id.0), "primary")
    } else if status.is_some() {
        tb("View Application", "dialog:cm-mine", "secondary")
    } else if g.applications {
        tb(
            "Apply to Join",
            &format!("dialog:cm-apply:discovery:{}", e.guild_id.0),
            "primary",
        )
    } else {
        tb(
            "Join Server",
            &format!("cm:join-discovered,{}", e.guild_id.0),
            "primary",
        )
    };
    view!{<div class="relative -mx-4 -mt-4 h-24 rounded-t-lg" style=banner(e.banner_color)>{super::identity::directory_banner(a,e.guild_id)}</div>
        <div class="-mt-8 flex items-end gap-3">{entry_icon(e,"size-16 border-4 border-popover")}<div class="min-w-0 pb-1"><h2 class="wrap-anywhere">{e.name.clone()}</h2><p class="text-xs text-subtle">{format!("{} · {} · {}",counts(e),e.category.label(),e.language)}</p></div></div>
        <p class="mt-3 wrap-anywhere">{e.description.clone()}</p>
        <div class="mt-2 flex flex-wrap gap-1">{e.tags.iter().map(|t|view!{<span class="rounded-full bg-muted px-2 py-0.5 text-xs">{t.clone()}</span>}).collect_view()}</div>
        {(!e.featured_emoji.is_empty()).then(||view!{<div class="mt-3 flex flex-wrap gap-2" aria-label="Featured emoji">{e.featured_emoji.iter().map(|m|view!{<img class="size-8" src=format!("/api/v1/emoji/{}",m.id.0) alt=format!(":{}:",m.name) title=format!(":{}:",m.name) loading="lazy"/>}).collect_view()}</div>})}
        {p.welcome.as_ref().map(|w|view!{<section class="mt-4 rounded-lg border bg-card p-3"><h3 class="mb-2 text-sm font-semibold">"Inside"</h3>{w.highlights.iter().map(|h|view!{<p class="text-sm">{h.emoji.clone().unwrap_or_default()}" "<strong>{format!("#{}",p.channel_names.iter().find(|(c,_)|*c==h.channel_id).map_or("",|(_,n)|n.as_str()))}</strong>" — "{h.text.clone()}</p>}).collect_view()}</section>})}
        <p class="mt-4 text-sm text-subtle">{if g.applications{"Joining takes an application that a moderator reviews.".to_owned()}else if g.screening{format!("New members accept {} before they can talk.",plural(g.rules.len() as u32,"rule","rules"))}else{"Anyone can join.".to_owned()}}</p>
        <div class="mt-4 flex flex-wrap gap-2">{action}{tb("Back to Discover","dialog:discover","secondary")}</div>
    }.into_any()
}
fn template_preview(a: &App) -> AnyView {
    let Some(t) = a.community.template.as_ref() else {
        return view! {<h2>"Server Template"</h2><p role="status">"Loading…"</p>}.into_any();
    };
    let s = &t.snapshot;
    let depth = |c: &TemplateChannel| {
        let mut d = 0;
        let mut at = c.parent;
        while let Some(p) = at {
            d += 1;
            at = s.channels.iter().find(|x| x.id == p).and_then(|x| x.parent);
            if d > 8 {
                break;
            }
        }
        d
    };
    let kind = |k: ChannelKind| match k {
        ChannelKind::Voice => "volume-2",
        ChannelKind::Category => "folder",
        ChannelKind::Announcement => "megaphone",
        ChannelKind::Forum => "messages-square",
        _ => "hash",
    };
    view!{<form data-live-form="cm-from-template"><h2>{t.name.clone()}</h2><p class="muted-copy">{t.description.clone()}</p>
        <p class="mt-2 text-sm text-subtle">{format!("{} · {}{}{}",plural(s.channels.len() as u32,"channel","channels"),plural(s.roles.len().saturating_sub(1) as u32,"role","roles"),if s.onboarding.as_ref().is_some_and(|o|o.enabled){" · onboarding"}else{""},if s.automod.is_empty(){String::new()}else{format!(" · {}",plural(s.automod.len() as u32,"AutoMod rule","AutoMod rules"))})}</p>
        <div class="template-structure my-4 grid gap-4 sm:grid-cols-2"><section class="rounded-lg border bg-card p-3"><h3 class="mb-2 text-sm font-semibold">"Channels"</h3><ul class="grid gap-1 text-sm">{s.channels.iter().map(|c|view!{<li class="flex items-center gap-2" style=indent(depth(c),16)>{icon_with(kind(c.kind),"icon size-4 text-muted-foreground")}{c.name.clone()}</li>}).collect_view()}</ul></section>
            <section class="rounded-lg border bg-card p-3"><h3 class="mb-2 text-sm font-semibold">"Roles"</h3><ul class="grid gap-1 text-sm">{s.roles.iter().rev().map(|r|view!{<li class="flex items-center gap-2"><span class="size-3 rounded-full" style=format!("background:#{:06x}",if r.color==0{0xa8a8b0}else{r.color})></span>{r.name.clone()}</li>}).collect_view()}</ul>
            {s.admission.as_ref().filter(|x|x.screening).map(|x|view!{<p class="mt-3 text-sm">{format!("Rules screening with {}.",plural(x.rules.len() as u32,"rule","rules"))}</p>})}</section></div>
        <div class="field"><label for="cm-server-name">"Server Name"</label><input id="cm-server-name" type="text" maxlength="64" value=t.name.clone()/></div>
        <p class="muted-copy">"Only the structure is copied. You’ll be the owner of the new server."</p>
        <div class="mt-4 flex flex-wrap gap-2"><button type="submit" class="button primary">"Create Server"</button>{tb("Back","dialog:create","secondary")}</div>
    </form>}.into_any()
}

/// Every dialog this module renders, with its frame.
pub(super) fn dialog(a: &App) -> Option<AnyView> {
    let d = a.dialog.as_str();
    Some(match d {
        "cm-screening" => settings::server_layout(a, screening(a)),
        "cm-applications" => settings::server_layout(a, applications(a)),
        "cm-queue" => settings::server_layout(a, queue(a)),
        "cm-onboarding" => settings::server_layout(a, onboarding_editor(a)),
        "cm-discovery" => settings::server_layout(a, discovery(a)),
        "cm-templates" => settings::server_layout(a, templates(a)),
        "cm-insights" => settings::server_layout(a, insights(a)),
        "cm-flow" => modal(a, "onboarding-flow", "Welcome", flow(a)),
        "cm-browse" => modal(a, "", "Browse Channels", browse(a)),
        "cm-guide" => modal(a, "", "Server Guide", guide(a)),
        "cm-rules" => modal(a, "", "Server Rules", rules(a)),
        "cm-mine" => modal(a, "", "My Applications", mine(a)),
        "discover" => discover(a),
        _ if d.starts_with("cm-apply:") => modal(a, "", "Apply to Join", apply(a)),
        _ if d.starts_with("discover:") => {
            modal(a, "directory-preview", "Server Preview", preview(a))
        }
        _ if d.starts_with("template:") => modal(a, "", "Server Template", template_preview(a)),
        _ => return None,
    })
}
