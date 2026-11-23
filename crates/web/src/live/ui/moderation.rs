//! Moderation views: Safety Setup, AutoMod, Audit Log and Prune pages in server
//! settings, the channel permission editor, member sanctions, and the notices
//! in the chat (raid alert, age gate, timeout, verification and slowmode).
use super::*;
use crate::live::moderation::{applies, clock, cooldown, radio_name, CHANNEL_PERMISSIONS};

/// Dialog pages this module renders.
pub(super) fn owns(page: &str) -> bool {
    matches!(page, "audit-log" | "automod" | "safety" | "prune")
        || [
            "automod-rule:",
            "channel-perms:",
            "mod-timeout:",
            "mod-kick:",
            "mod-ban:",
        ]
        .iter()
        .any(|p| page.starts_with(p))
}
/// The ones shown inside server settings (the rest are dialogs).
pub(super) fn is_server_page(page: &str) -> bool {
    owns(page) && !page.starts_with("mod-")
}
pub(super) fn page(a: &App, page: &str) -> AnyView {
    match page {
        "audit-log" => audit_log(a),
        "automod" => automod(a),
        "safety" => safety(a),
        "prune" => prune(a),
        p if p.starts_with("automod-rule:") => {
            rule_editor(a, p.trim_start_matches("automod-rule:"))
        }
        p if p.starts_with("channel-perms:") => channel_permissions(a, p),
        p => sanction(a, p),
    }
}
fn guild_state(a: &App) -> Option<&ModerationState> {
    a.guild.and_then(|g| a.moderation.states.get(&g))
}
fn moderator(a: &App) -> bool {
    settings::can_any(
        a,
        Permissions::MANAGE_GUILD
            | Permissions::KICK_MEMBERS
            | Permissions::BAN_MEMBERS
            | Permissions::MODERATE_MEMBERS,
    )
}
/// A form with its own submit label, for actions rather than saves.
fn action_form(
    title: &str,
    kind: &str,
    label: &str,
    class: &str,
    fields: AnyView,
    back: &str,
) -> AnyView {
    view!{<form data-live-form=kind.to_owned()><h2>{title.to_owned()}</h2>{fields}<div class="mt-4 flex flex-wrap gap-2 phone:flex-col"><button type="submit" class=format!("button {class}")>{label.to_owned()}</button>{if back.is_empty(){().into_any()}else{view!{<button type="button" class="button secondary" data-live=back.to_owned()>"Cancel"</button>}.into_any()}}</div></form>}.into_any()
}
fn duration(seconds: u64) -> String {
    let plural = |n: u64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    match seconds {
        s if s >= 86_400 && s % 86_400 == 0 => plural(s / 86_400, "day"),
        s if s >= 3_600 && s % 3_600 == 0 => plural(s / 3_600, "hour"),
        s if s >= 60 && s % 60 == 0 => plural(s / 60, "minute"),
        s => plural(s, "second"),
    }
}
fn notice_row(icon_name: &str, text: AnyView, tone: &str) -> AnyView {
    view!{<div class=format!("moderation-notice mb-2 flex items-start gap-3 rounded-lg border px-3 py-2 text-[13px] [&>.icon]:mt-0.5 [&>.icon]:size-4 [&>.icon]:flex-none {tone}") role="status">{icon(icon_name)}<div class="min-w-0 flex-1 wrap-anywhere">{text}</div></div>}.into_any()
}

// ---------------------------------------------------------------- chat notices

/// Above the composer: why the viewer cannot talk here, or when they can again.
pub(super) fn composer_notice(a: &App, c: Id) -> AnyView {
    let t = Browser.unix_ms();
    let Some(g) = a.model.channels.get(&c).and_then(|ch| ch.guild_id) else {
        return ().into_any();
    };
    if let Some(timeout) = a
        .moderation
        .states
        .get(&g)
        .and_then(|s| s.timeout.as_ref())
        .filter(|x| x.until > t)
    {
        return notice_row(
            "mic-off",
            view!{<strong class="font-semibold">{format!("You’re timed out until {}.",timestamp(timeout.until))}</strong>" You can read, but not send messages, react or join voice."{timeout.reason.clone().map(|r|view!{<span class="mt-1 block text-subtle">{format!("Reason: {r}")}</span>})}}.into_any(),
            "timeout-notice border-warning bg-card",
        );
    }
    if let Some(until) = a
        .moderation
        .states
        .get(&g)
        .and_then(|s| s.send_blocked_until)
        .filter(|u| *u > t)
    {
        return notice_row(
            "shield-check",
            view!{{format!("This server lets new members send messages after a short wait. You can send messages at {}.",short_time(until))}}.into_any(),
            "verification-notice bg-card",
        );
    }
    let Some((s, _)) = a
        .moderation
        .slowmode
        .as_ref()
        .filter(|(s, _)| s.channel_id == c && s.seconds > 0)
    else {
        return ().into_any();
    };
    let every = duration(u64::from(s.seconds));
    if s.bypassed {
        return view!{<p class="slowmode-notice mb-1 text-xs text-subtle">{format!("Slowmode is on (one message every {every}). You aren’t limited.")}</p>}.into_any();
    }
    match cooldown(a, c) {
        Some(left) => {
            crate::live::moderation::tick(a);
            view!{<p class="slowmode-notice mb-1 text-xs text-subtle" role="status">{format!("Slowmode is on: one message every {every}. You can send again in ")}<span id="slowmode-countdown" class="font-semibold tabular-nums text-foreground">{clock(left)}</span>"."</p>}.into_any()
        }
        None => view!{<p class="slowmode-notice mb-1 text-xs text-subtle">{format!("Slowmode is on: one message every {every}.")}</p>}.into_any(),
    }
}
/// For moderators: a burst of joins was detected in this server.
pub(super) fn raid_banner(a: &App) -> AnyView {
    let Some(raid) = guild_state(a).and_then(|s| s.safety.raid.as_ref()) else {
        return ().into_any();
    };
    if !moderator(a) {
        return ().into_any();
    }
    view!{<div class="raid-banner flex flex-wrap items-center gap-3 border-b border-warning bg-card px-4 py-2 text-sm phone:px-2" role="alert"><span class="text-warning">{icon("shield")}</span><span class="min-w-0 flex-1 wrap-anywhere"><strong class="font-semibold">"Unusual join activity. "</strong>{format!("{} members joined within {} seconds.",raid.joins,raid.window_seconds)}</span>{button("Security Actions","dialog:safety","secondary")}{button("Dismiss","mod:dismiss-raid","link-button")}</div>}.into_any()
}
/// An age-restricted channel this viewer has not confirmed: its content stays hidden.
pub(super) fn age_gate(a: &App, c: Id) -> Option<AnyView> {
    if !crate::live::moderation::gated(a, c) {
        return None;
    }
    let name = a
        .model
        .channels
        .get(&c)
        .map(|ch| a.model.channel_name(ch))
        .unwrap_or_default();
    Some(view!{<div class="age-gate flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center phone:p-4" role="region" aria-label="Age-restricted channel"><span class="inline-flex rounded-full bg-muted p-4 [&>.icon]:size-8">{icon("lock")}</span><h2>{format!("#{name} is age-restricted")}</h2><p class="max-w-[440px] text-subtle">"This channel may contain content for adults only. Confirm you are 18 or older to see it. You will only be asked once for this channel."</p><div class="flex flex-wrap justify-center gap-3 phone:w-full phone:flex-col">{button("I’m 18 or Older",&format!("mod:confirm-age,{}",c.0),"primary")}{button("Go Back","home","secondary")}</div></div>}.into_any())
}

// ---------------------------------------------------------------- member actions

/// Kick, ban and time out, in a member's profile, for those allowed to.
pub(super) fn member_actions(a: &App, u: &UserSummary) -> AnyView {
    let Some(g) = a.guild.and_then(|g| a.model.guilds.get(&g)) else {
        return ().into_any();
    };
    let me = a.model.me.as_ref().map(|m| m.user.id);
    let Some(member) = a
        .model
        .members
        .get(&u.id)
        .filter(|m| m.guild_id == g.guild.id)
    else {
        return ().into_any();
    };
    if Some(u.id) == me || member.role == MemberRole::Owner {
        return ().into_any();
    }
    let timed_out = member.timeout_until.is_some_and(|t| t > Browser.unix_ms());
    let id = u.id.0;
    let mut actions = vec![];
    if settings::can(a, Permissions::MODERATE_MEMBERS) {
        actions.push(if timed_out {
            button(
                "Remove Timeout",
                &format!("mod:remove-timeout,{id}"),
                "secondary",
            )
        } else {
            button("Time Out", &format!("dialog:mod-timeout:{id}"), "secondary")
        });
    }
    if settings::can(a, Permissions::KICK_MEMBERS) {
        actions.push(button(
            "Kick",
            &format!("dialog:mod-kick:{id}"),
            "danger-text",
        ));
    }
    if settings::can(a, Permissions::BAN_MEMBERS) {
        actions.push(button(
            "Ban",
            &format!("dialog:mod-ban:{id}"),
            "danger-text",
        ));
    }
    if actions.is_empty() {
        return ().into_any();
    }
    view!{<section class="member-moderation mt-4 border-t pt-4" aria-label="Moderation"><h3 class="category mb-2">{format!("Moderate in {}",g.guild.name)}</h3>{if timed_out{view!{<p class="muted-copy mb-2">{format!("Timed out until {}.",timestamp(member.timeout_until.unwrap_or_default()))}</p>}.into_any()}else{().into_any()}}<div class="flex flex-wrap gap-2">{actions}</div></section>}.into_any()
}
fn sanction(a: &App, page: &str) -> AnyView {
    let (kind, user) = page.split_once(':').map_or(("", None), |(k, v)| (k, id(v)));
    let Some(u) = user.and_then(|u| {
        a.model
            .users
            .get(&u)
            .or_else(|| a.model.members.get(&u).map(|m| &m.user))
    }) else {
        return view! {<h2>"Member"</h2><p class="muted-copy">"That member is no longer here."</p>}
            .into_any();
    };
    let who = name(u);
    let hidden = view! {<input id="mod-user" type="hidden" value=u.id.0.to_string()/>}.into_any();
    let reason = view!{<label class="field"><span>"Reason (optional)"</span><textarea id="mod-reason" rows="2" maxlength="512" aria-describedby="mod-reason-help"></textarea><small id="mod-reason-help">"Shown in the audit log."</small></label>}.into_any();
    match kind {
        "mod-timeout" => action_form(&format!("Time Out {who}"),"mod-timeout","Time Out","primary",view!{
            <p class="muted-copy">"They can still read, but can’t send messages, react, or join and speak in voice until the timeout ends."</p>{hidden}
            <fieldset class="my-4"><legend class="mb-2 text-[13px] font-medium text-subtle">"Duration"</legend><div class="timeout-durations grid grid-cols-2 gap-2 phone:grid-cols-1">
                {TIMEOUT_PRESETS.iter().enumerate().map(|(i,s)|view!{<label class="live-checkbox rounded-lg border px-3"><span>{duration(u64::from(*s))}</span><input type="radio" name="mod-duration-choice" value=s.to_string() checked=i==2 data-duration=s.to_string()/></label>}).collect_view()}
                <label class="live-checkbox rounded-lg border px-3"><span>"Custom"</span><input type="radio" name="mod-duration-choice" value="custom" data-duration="custom"/></label>
            </div></fieldset>
            <div class="custom-duration flex gap-2"><label class="field flex-1"><span>"Custom length"</span><input id="mod-custom" type="number" min="1" max="40320" value="2" inputmode="numeric"/></label><label class="field flex-1"><span>"Unit"</span><select id="mod-unit"><option value="60">"Minutes"</option><option value="3600" selected>"Hours"</option><option value="86400">"Days"</option></select></label></div>
            <p class="muted-copy">"Up to 28 days."</p>{reason}
        }.into_any(),"close"),
        "mod-kick" => action_form(&format!("Kick {who}"),"mod-kick","Kick","danger",view!{<p>"They are removed from the server and can rejoin with a new invite."</p>{hidden}{reason}}.into_any(),"close"),
        _ => action_form(&format!("Ban {who}"),"mod-ban","Ban","danger",view!{<p>"They are removed and can’t rejoin until the ban is revoked."</p>{hidden}{reason}}.into_any(),"close"),
    }
}

// ---------------------------------------------------------------- safety setup

pub(super) fn safety(a: &App) -> AnyView {
    let allowed = settings::can(a, Permissions::MANAGE_GUILD);
    let Some(s) = guild_state(a) else {
        return view! {<h2>"Safety Setup"</h2><p class="muted-copy" role="status">"Loading…"</p>}
            .into_any();
    };
    let level = s.safety.verification_level;
    let t = Browser.unix_ms();
    let pause = |kind: &str, label: &str, what: &str, until: Option<u64>| {
        let form = format!("mod-pause-{kind}");
        view!{<section class=format!("pause-{kind} mt-6 border-t pt-6")>{match until.filter(|u|*u>t){
            Some(u)=>view!{<h3>{label.to_owned()}</h3><p class="mb-3 text-warning" role="status">{format!("{what} are paused until {}.",timestamp(u))}</p>{button(&format!("Resume {what}"),&format!("mod:resume,{kind}"),"secondary")}}.into_any(),
            None=>view!{<form data-live-form=form.clone()><h3>{label.to_owned()}</h3><p class="muted-copy">{match kind{"invites"=>"Stop new members joining with any invite link. Existing members are not affected.",_=>"Stop direct messages between members of this server who are not friends."}}</p><fieldset disabled=!allowed class="flex flex-wrap items-end gap-2"><label class="field mb-0 min-w-40 flex-1"><span>"Pause for"</span><select id=format!("{form}-for")>{[(3600,"1 hour"),(7200,"2 hours"),(21600,"6 hours"),(43200,"12 hours"),(86400,"24 hours")].into_iter().map(|(v,l)|view!{<option value=v.to_string()>{l}</option>}).collect_view()}</select></label><button type="submit" class="button secondary">{format!("Pause {what}")}</button></fieldset></form>}.into_any(),
        }}</section>}.into_any()
    };
    view!{<h2>"Safety Setup"</h2><p class="muted-copy mb-6">"Protect this server from spam and raids. These settings apply on the server, for everyone."</p>
        {s.safety.raid.as_ref().map(|r|view!{<div class="raid-alert mb-6 rounded-lg border border-warning bg-card p-4" role="alert"><strong class="font-semibold">"Unusual join activity"</strong><p class="mt-1">{format!("{} members joined within {} seconds, at {}. Consider pausing invites.",r.joins,r.window_seconds,short_time(r.detected_at))}</p><div class="mt-3">{button("Dismiss Alert","mod:dismiss-raid","secondary")}</div></div>})}
        <form data-live-form="mod-verification"><h3>"Verification Level"</h3><p class="muted-copy">"Members must meet this before they can send messages or direct-message other members. Members with a role are not affected."</p><fieldset disabled=!allowed class="verification-levels my-3 grid gap-2">
            {[(VerificationLevel::None,"none","None","Unrestricted."),(VerificationLevel::AccountAge,"account_age","Low","Their StickTalk account must be older than 5 minutes."),(VerificationLevel::MemberAge,"member_age","Medium","Their account must be older than 5 minutes and they must have been a member for 10 minutes.")].into_iter().map(|(l,v,title,help)|view!{<label class="live-checkbox items-start rounded-lg border px-3 py-2"><span class="flex flex-col gap-1"><strong class="font-semibold">{title}</strong><small class="text-subtle">{help}</small></span><input type="radio" name="mod-level-choice" value=v checked=l==level data-level=v/></label>}).collect_view()}
        </fieldset><p class="muted-copy">"StickTalk does not collect email addresses or phone numbers, so there are no email or phone levels."</p><button type="submit" class="button primary" disabled=!allowed>"Save"</button>{saved_note(a,"mod-verification")}</form>
        {pause("invites","Pause Invites","Invites",s.safety.invites_paused_until)}
        {pause("dms","Pause Direct Messages","DMs",s.safety.dms_paused_until)}
        {if allowed{().into_any()}else{view!{<p class="muted-copy mt-6">"You need Manage Server permission to change these."</p>}.into_any()}}
    }.into_any()
}

// ---------------------------------------------------------------- automod

fn trigger_text(r: &AutoModRule) -> String {
    match &r.trigger {
        AutoModTrigger::Keyword { keywords, .. } => format!(
            "{} keyword{}",
            keywords.len(),
            if keywords.len() == 1 { "" } else { "s" }
        ),
        AutoModTrigger::MentionSpam { limit } => {
            format!("More than {limit} mentions in one message")
        }
        AutoModTrigger::Spam { duplicates, links } => {
            let mut v = vec![];
            if *duplicates > 0 {
                v.push(format!("the same message {duplicates} times in 30 seconds"));
            }
            if *links > 0 {
                v.push(format!("more than {links} links"));
            }
            format!("Spam: {}", v.join(" or "))
        }
    }
}
fn actions_text(a: &App, r: &AutoModRule) -> String {
    let mut v = vec![];
    if r.actions.block {
        v.push("block".to_string());
    }
    if let Some(c) = r.actions.alert_channel_id {
        v.push(format!(
            "alert #{}",
            a.model
                .channels
                .get(&c)
                .map(|ch| a.model.channel_name(ch))
                .unwrap_or_else(|| "deleted channel".into())
        ));
    }
    if r.actions.timeout_seconds > 0 {
        v.push(format!(
            "time out {}",
            duration(u64::from(r.actions.timeout_seconds))
        ));
    }
    v.join(", ")
}
pub(super) fn automod(a: &App) -> AnyView {
    let allowed = settings::can(a, Permissions::MANAGE_GUILD);
    let rules = a
        .moderation
        .rules
        .as_ref()
        .filter(|(g, _)| Some(*g) == a.guild)
        .map(|(_, r)| r);
    let count = rules.map_or(0, |r| r.len());
    let has =
        |f: fn(&AutoModTrigger) -> bool| rules.is_some_and(|r| r.iter().any(|x| f(&x.trigger)));
    let full = count >= MAX_AUTOMOD_RULES;
    view!{<h2>"AutoMod"</h2><p class="muted-copy mb-6">"Rules run on the server before a message is posted, on sends and edits. Members with Manage Server, and the roles and channels you exempt, are not checked."</p>
        {if !allowed{view!{<p class="muted-copy">"You need Manage Server permission to manage AutoMod."</p>}.into_any()}else{().into_any()}}
        {match rules{None if allowed=>view!{<p class="muted-copy" role="status">"Loading…"</p>}.into_any(),None=>().into_any(),Some(rules)=>view!{
            {if rules.is_empty(){view!{<div class="settings-empty flex flex-col items-start gap-4 pt-2 pb-4 [&>.icon]:size-8 [&>.icon]:text-subtle">{icon("shield")}<p>"No rules yet. Start with a keyword list, or block mention and link spam."</p></div>}.into_any()}else{().into_any()}}
            <div class="automod-rules mb-6">{rules.iter().map(|r|view!{<div class="automod-rule settings-list-row flex flex-wrap items-center gap-3 border-b py-4" data-rule=r.id.0.to_string()><span class="settings-row-copy flex min-w-40 flex-1 flex-col gap-1 wrap-anywhere"><strong class="font-semibold">{r.name.clone()}{if r.enabled{().into_any()}else{view!{<span class="ml-2 rounded-sm bg-muted px-1.5 py-0.5 text-xs font-semibold text-subtle">"Off"</span>}.into_any()}}</strong><small>{format!("{} · {}",trigger_text(r),actions_text(a,r))}</small></span>
                {button(if r.enabled{"Turn Off"}else{"Turn On"},&format!("mod:rule-toggle,{}",r.id.0),"secondary")}<button class="icon-button" aria-label=format!("Edit {}",r.name) data-live=format!("dialog:automod-rule:{}",r.id.0)>{icon("settings")}</button></div>}).collect_view()}</div>
        }.into_any()}}
        {if allowed{view!{<div class="flex flex-wrap gap-2 phone:flex-col">
            <button class="button primary" data-live="dialog:automod-rule:new-keyword" disabled=full>"Add Keyword Rule"</button>
            <button class="button secondary" data-live="dialog:automod-rule:new-mention" disabled=full||has(|t|matches!(t,AutoModTrigger::MentionSpam{..}))>"Add Mention Spam Rule"</button>
            <button class="button secondary" data-live="dialog:automod-rule:new-spam" disabled=full||has(|t|matches!(t,AutoModTrigger::Spam{..}))>"Add Spam Rule"</button>
        </div><p class="muted-copy mt-3">{format!("{count} of {MAX_AUTOMOD_RULES} rules. One mention spam rule and one spam rule per server.")}</p>}.into_any()}else{().into_any()}}
    }.into_any()
}
fn rule_editor(a: &App, which: &str) -> AnyView {
    let allowed = settings::can(a, Permissions::MANAGE_GUILD);
    let existing = id(which).and_then(|r| {
        a.moderation
            .rules
            .as_ref()
            .and_then(|(_, rules)| rules.iter().find(|x| x.id == r))
    });
    if id(which).is_some() && existing.is_none() {
        return view! {<h2>"AutoMod Rule"</h2><p class="muted-copy" role="status">"Loading…"</p>}
            .into_any();
    }
    let g = a.guild.unwrap_or_default();
    let fresh = |trigger| AutoModRule {
        id: Id(0),
        guild_id: g,
        name: String::new(),
        enabled: true,
        trigger,
        actions: AutoModActions {
            block: true,
            ..Default::default()
        },
        exempt_roles: vec![],
        exempt_channels: vec![],
    };
    let rule = existing.cloned().unwrap_or_else(|| match which {
        "new-mention" => {
            let mut r = fresh(AutoModTrigger::MentionSpam { limit: 5 });
            r.name = "Block mention spam".into();
            r
        }
        "new-spam" => {
            let mut r = fresh(AutoModTrigger::Spam {
                duplicates: 3,
                links: 5,
            });
            r.name = "Block spam".into();
            r
        }
        _ => {
            let mut r = fresh(AutoModTrigger::Keyword {
                keywords: vec![],
                allow: vec![],
            });
            r.name = "Blocked words".into();
            r
        }
    });
    let (kind, body) = match &rule.trigger {
        AutoModTrigger::Keyword { keywords, allow } => ("keyword", view!{
            <label class="field"><span>"Keywords"</span><textarea id="rule-keywords" rows="5" aria-describedby="rule-keywords-help">{keywords.join("\n")}</textarea><small id="rule-keywords-help">"One per line or separated by commas, up to 1,000. Whole words match by default; put * at the start or end to match words ending or starting with it (cat* matches catfish)."</small></label>
            <label class="field"><span>"Allowed words"</span><textarea id="rule-allow" rows="2" aria-describedby="rule-allow-help">{allow.join("\n")}</textarea><small id="rule-allow-help">"Words that never trigger this rule, even if a keyword matches them. Up to 100."</small></label>
        }.into_any()),
        AutoModTrigger::MentionSpam { limit } => ("mention_spam", view!{<label class="field"><span>"Most unique mentions allowed per message"</span><input id="rule-limit" type="number" min="1" max="50" value=limit.to_string() inputmode="numeric"/></label>}.into_any()),
        AutoModTrigger::Spam { duplicates, links } => ("spam", view!{
            <label class="field"><span>"Block the same message sent this many times within 30 seconds"</span><input id="rule-duplicates" type="number" min="0" max="10" value=duplicates.to_string() inputmode="numeric" aria-describedby="rule-duplicates-help"/><small id="rule-duplicates-help">"2 to 10, or 0 to allow repeats."</small></label>
            <label class="field"><span>"Most links allowed per message"</span><input id="rule-links" type="number" min="0" max="20" value=links.to_string() inputmode="numeric" aria-describedby="rule-links-help"/><small id="rule-links-help">"Up to 20, or 0 for no limit."</small></label>
        }.into_any()),
    };
    let text_channels: Vec<_> = a
        .model
        .channels
        .values()
        .filter(|c| c.guild_id == a.guild && c.kind == ChannelKind::Text)
        .collect();
    let roles: Vec<_> = a
        .roles
        .values()
        .filter(|r| Some(r.guild_id) == a.guild && r.id != r.guild_id)
        .collect();
    let all_channels: Vec<_> = a
        .model
        .channels
        .values()
        .filter(|c| c.guild_id == a.guild)
        .collect();
    let r = &rule;
    view!{<form data-live-form="mod-rule"><h2>{if existing.is_some(){"Edit AutoMod Rule"}else{"New AutoMod Rule"}}</h2><fieldset disabled=!allowed>
        <input id="rule-id" type="hidden" value=existing.map_or(String::new(),|r|r.id.0.to_string())/><input id="rule-kind" type="hidden" value=kind/>
        <div class="field"><label for="rule-name">"Rule name"</label><input id="rule-name" type="text" maxlength="100" value=r.name.clone()/></div>
        {body}
        <h3 class="category mt-6 mb-2">"Actions"</h3>
        <label class="live-checkbox"><span>"Block the message"</span><input id="rule-block" type="checkbox" checked=r.actions.block/></label>
        <label class="field"><span>"Message shown to the author (optional)"</span><input id="rule-message" type="text" maxlength="150" value=r.actions.message.clone().unwrap_or_default() placeholder="This server’s AutoMod blocked your message"/></label>
        <label class="field"><span>"Send an alert to"</span><select id="rule-alert"><option value="">"No alert"</option>{text_channels.iter().map(|c|view!{<option value=c.id.0.to_string() selected=r.actions.alert_channel_id==Some(c.id)>{format!("#{}",a.model.channel_name(c))}</option>}).collect_view()}</select></label>
        <label class="field"><span>"Time the member out"</span><select id="rule-timeout"><option value="0">"No timeout"</option>{TIMEOUT_PRESETS.iter().map(|s|view!{<option value=s.to_string() selected=r.actions.timeout_seconds==*s>{duration(u64::from(*s))}</option>}).collect_view()}</select></label>
        <h3 class="category mt-6 mb-2">"Exempt roles"</h3>
        {if roles.is_empty(){view!{<p class="muted-copy">"This server has no roles to exempt yet."</p>}.into_any()}else{roles.iter().map(|x|view!{<label class="live-checkbox"><span>{x.name.clone()}</span><input id=format!("rule-role-{}",x.id.0) type="checkbox" checked=r.exempt_roles.contains(&x.id)/></label>}).collect_view().into_any()}}
        <h3 class="category mt-6 mb-2">"Exempt channels"</h3><p class="muted-copy">"Everything inside an exempt channel is exempt too."</p>
        <div class="max-h-60 overflow-y-auto">{all_channels.iter().map(|c|view!{<label class="live-checkbox"><span>{a.model.channel_name(c)}</span><input id=format!("rule-channel-{}",c.id.0) type="checkbox" checked=r.exempt_channels.contains(&c.id)/></label>}).collect_view()}</div>
    </fieldset><div class="mt-4 flex flex-wrap gap-2 phone:flex-col"><button type="submit" class="button primary" disabled=!allowed>{if existing.is_some(){"Save Rule"}else{"Create Rule"}}</button><button type="button" class="button secondary" data-live="dialog:automod">"Cancel"</button>{existing.filter(|_|allowed).map(|r|view!{<button type="button" class="button danger-text" data-live=format!("mod:rule-delete,{}",r.id.0)>"Delete Rule"</button>})}</div></form>}.into_any()
}

// ---------------------------------------------------------------- audit log

fn when(ms: u64) -> String {
    timestamp(ms)
}
pub(super) fn audit_log(a: &App) -> AnyView {
    if !settings::can(a, Permissions::VIEW_AUDIT_LOG) {
        return view!{<h2>"Audit Log"</h2><p class="muted-copy">"You need View Audit Log permission to see this."</p>}.into_any();
    }
    let Some(v) = a
        .moderation
        .audit
        .as_ref()
        .filter(|v| Some(v.guild) == a.guild)
    else {
        return view! {<h2>"Audit Log"</h2><p class="muted-copy" role="status">"Loading…"</p>}
            .into_any();
    };
    let who = |u: Id| {
        v.users
            .get(&u)
            .or_else(|| a.model.users.get(&u))
            .map_or_else(|| "Someone".to_string(), name)
    };
    let mut actors: Vec<(Id, String)> = v.users.values().map(|u| (u.id, name(u))).collect();
    for m in a
        .model
        .members
        .values()
        .filter(|m| Some(m.guild_id) == a.guild)
    {
        if !actors.iter().any(|(id, _)| *id == m.user.id) {
            actors.push((m.user.id, name(&m.user)));
        }
    }
    actors.sort_by_key(|x| x.1.to_lowercase());
    view!{<h2>"Audit Log"</h2><p class="muted-copy mb-4">"Who changed what in this server, newest first. The latest 5,000 actions are kept."</p>
        <div class="audit-filters mb-4 flex flex-wrap items-end gap-2 phone:flex-col phone:items-stretch">
            <label class="field mb-0 min-w-44 flex-1"><span>"Filter by user"</span><select id="audit-actor"><option value="">"All users"</option>{actors.into_iter().map(|(id,n)|view!{<option value=id.0.to_string() selected=v.actor==Some(id)>{n}</option>}).collect_view()}</select></label>
            <label class="field mb-0 min-w-44 flex-1"><span>"Filter by action"</span><select id="audit-action"><option value="">"All actions"</option>{AuditAction::ALL.iter().map(|x|view!{<option value=x.name() selected=v.action==Some(*x)>{x.label()}</option>}).collect_view()}</select></label>
            {button("Apply","mod:audit-filter","secondary")}
        </div>
        {if v.items.is_empty()&&!v.busy{view!{<div class="settings-empty flex flex-col items-start gap-4 pt-2 pb-4 [&>.icon]:size-8 [&>.icon]:text-subtle">{icon("shield")}<p>"Nothing has been logged yet."</p></div>}.into_any()}else{().into_any()}}
        <ol class="audit-entries" aria-label="Audit log entries">{v.items.iter().map(|e|{
            let actor=v.users.get(&e.actor_id).cloned().unwrap_or(UserSummary{id:e.actor_id,username:"unknown".into(),..Default::default()});
            let target=match (e.target_kind,e.target_id){(AuditTarget::User,Some(t))=>Some(format!("@{}",v.users.get(&t).map_or_else(||e.target_name.clone().unwrap_or_default(),|u|u.username.clone()))),_=>e.target_name.clone()};
            view!{<li class="audit-entry flex gap-3 border-b py-3" data-action=e.action.name()>{avatar(&actor,"small")}<div class="min-w-0 flex-1 wrap-anywhere"><p><strong class="font-semibold">{who(e.actor_id)}</strong>{format!(" {}",e.action.label().to_lowercase().trim_start_matches("automod ").to_owned())}{target.map(|t|view!{<span>": "<strong class="font-semibold">{t}</strong></span>})}</p>
                {if e.changes.is_empty(){().into_any()}else{view!{<ul class="mt-1 text-[13px] text-subtle">{e.changes.iter().map(|c|view!{<li><span class="font-medium text-foreground">{c.field.clone()}</span>{match (&c.before,&c.after){(Some(b),Some(x))=>format!(": {b} → {x}"),(None,Some(x))=>format!(": {x}"),(Some(b),None)=>format!(": {b} (removed)"),(None,None)=>String::new()}}</li>}).collect_view()}</ul>}.into_any()}}
                {e.reason.clone().map(|r|view!{<p class="mt-1 text-[13px]"><span class="text-subtle">"Reason: "</span>{r}</p>})}
                <small class="text-xs text-subtle">{when(e.created_at)}</small></div></li>}
        }).collect_view()}</ol>
        {if v.busy{view!{<p class="muted-copy" role="status">"Loading…"</p>}.into_any()}else if v.more{button("Load More","mod:audit-more","secondary")}else{().into_any()}}
    }.into_any()
}

// ---------------------------------------------------------------- prune

pub(super) fn prune(a: &App) -> AnyView {
    if !settings::can(a, Permissions::KICK_MEMBERS) {
        return view!{<h2>"Prune Members"</h2><p class="muted-copy">"You need Kick Members permission to prune."</p>}.into_any();
    }
    let roles: Vec<_> = a
        .roles
        .values()
        .filter(|r| Some(r.guild_id) == a.guild && r.id != r.guild_id)
        .collect();
    let preview = a.moderation.prune.as_ref();
    action_form("Prune Members","mod-prune","Prune","danger",view!{
        <p class="muted-copy">"Remove members who have not been on StickTalk for a while. They can rejoin with a new invite. Members you can’t kick, and anyone online now, are never removed."</p>
        <label class="field"><span>"Inactive for"</span><select id="prune-days">{PRUNE_DAYS.iter().map(|d|view!{<option value=d.to_string() selected=preview.map_or(*d==7,|p|p.days==*d)>{if *d==1{"1 day".to_string()}else{format!("{d} days")}}</option>}).collect_view()}</select></label>
        <h3 class="category mt-4 mb-2">"Include members with these roles"</h3><p class="muted-copy">"With none chosen, only members without any role are pruned."</p>
        {roles.into_iter().map(|r|view!{<label class="live-checkbox"><span>{r.name.clone()}</span><input id=format!("prune-role-{}",r.id.0) type="checkbox"/></label>}).collect_view()}
        <div class="my-4 flex flex-wrap items-center gap-3"><p class="prune-count m-0 flex-1 font-semibold" role="status">{match preview{Some(p)=>format!("{} member{} would be removed.",p.count,if p.count==1{""}else{"s"}),None=>"Counting…".into()}}</p>{button("Update Count","mod:prune-preview","secondary")}</div>
        <label class="field"><span>"Reason (optional)"</span><textarea id="mod-reason" rows="2" maxlength="512"></textarea></label>
    }.into_any(),"")
}

// ---------------------------------------------------------------- channel settings

/// Slowmode, age restriction and the link to permissions, in the channel editor.
pub(super) fn channel_safety(a: &App, c: &ChannelSummary) -> AnyView {
    let allowed = settings::can(a, Permissions::MANAGE_CHANNELS);
    let restricted = a
        .moderation
        .states
        .get(&c.guild_id.unwrap_or_default())
        .is_some_and(|s| s.age_restricted.contains(&c.id));
    let seconds = a
        .moderation
        .slowmode
        .as_ref()
        .filter(|(s, _)| s.channel_id == c.id)
        .map_or(0, |(s, _)| s.seconds);
    let text = c.kind == ChannelKind::Text;
    view!{<div class="settings-divider"></div>
        {if matches!(c.kind,ChannelKind::Text|ChannelKind::Voice){view!{<form data-live-form="mod-channel"><h2>"Slowmode & Safety"</h2><fieldset disabled=!allowed><input id="mod-channel" type="hidden" value=c.id.0.to_string()/>
            {if text{view!{<label class="field"><span>"Slowmode"</span><select id="mod-slowmode" aria-describedby="slowmode-help">{SLOWMODE_CHOICES.iter().map(|(s,l)|view!{<option value=s.to_string() selected=*s==seconds>{*l}</option>}).collect_view()}</select><small id="slowmode-help">"How long members wait between messages. Members who can manage messages or this channel are not limited."</small></label>}.into_any()}else{().into_any()}}
            <label class="live-checkbox"><span class="flex flex-col gap-1"><span>"Age-restricted channel"</span><small class="text-subtle">"Members must confirm they are 18 or older before they see its content."</small></span><input id="mod-age" type="checkbox" checked=restricted/></label>
        </fieldset><button type="submit" class="button primary" disabled=!allowed>"Save"</button>{saved_note(a,"mod-channel")}</form>}.into_any()}else{().into_any()}}
        {if settings::can(a,Permissions::MANAGE_ROLES){view!{<div class="settings-divider"></div><h2>"Permissions"</h2><p class="muted-copy">"Choose who can see and use this channel: allow or deny each permission for any role or member."</p>{button("Edit Permissions",&format!("dialog:channel-perms:{}",c.id.0),"secondary")}}.into_any()}else{().into_any()}}
    }.into_any()
}
fn target_name(a: &App, o: &PermissionOverwrite) -> String {
    match o.kind {
        OverwriteKind::Role if Some(o.target_id) == a.guild => "@everyone".into(),
        OverwriteKind::Role => a
            .roles
            .get(&o.target_id)
            .map_or_else(|| "Deleted role".into(), |r| r.name.clone()),
        OverwriteKind::Member => a
            .model
            .members
            .get(&o.target_id)
            .map(|m| m.nickname.clone().unwrap_or_else(|| name(&m.user)))
            .or_else(|| a.model.users.get(&o.target_id).map(name))
            .unwrap_or_else(|| "Member".into()),
    }
}
fn channel_permissions(a: &App, page: &str) -> AnyView {
    let Some(c) = id(page.trim_start_matches("channel-perms:"))
        .and_then(|c| a.model.channels.get(&c))
        .filter(|c| c.guild_id == a.guild)
    else {
        return ().into_any();
    };
    let allowed = settings::can(a, Permissions::MANAGE_ROLES);
    let Some((_, list)) = a
        .moderation
        .overwrites
        .as_ref()
        .filter(|(ch, _)| *ch == c.id)
    else {
        return view!{<h2>"Channel Permissions"</h2><p class="muted-copy" role="status">"Loading…"</p>}.into_any();
    };
    let everyone = a.guild.unwrap_or_default();
    let private = list.iter().any(|o| {
        o.kind == OverwriteKind::Role
            && o.target_id == everyone
            && o.deny.contains(Permissions::VIEW_CHANNEL)
    });
    let summary = a.moderation.access.as_ref().filter(|x| x.channel_id == c.id).map(|x| {
        let roles: Vec<String> = x.roles.iter().filter_map(|r| a.roles.get(r)).map(|r| r.name.clone()).collect();
        if x.everyone {
            format!("Everyone in the server can see this channel ({} of {} members).", x.viewers, x.members)
        } else if roles.is_empty() {
            format!("{} of {} members can see this channel: the owner, administrators and members allowed individually.", x.viewers, x.members)
        } else {
            format!("{} of {} members can see this channel. Roles that can: {}.", x.viewers, x.members, roles.join(", "))
        }
    });
    let mut sorted: Vec<&PermissionOverwrite> = list.iter().collect();
    sorted.sort_by_key(|o| {
        (
            o.kind == OverwriteKind::Member,
            o.target_id != everyone,
            std::cmp::Reverse(a.roles.get(&o.target_id).map_or(0, |r| r.position)),
        )
    });
    let unused_roles: Vec<_> = a
        .roles
        .values()
        .filter(|r| {
            Some(r.guild_id) == a.guild
                && !list
                    .iter()
                    .any(|o| o.kind == OverwriteKind::Role && o.target_id == r.id)
        })
        .collect();
    let unused_members: Vec<_> = a
        .model
        .members
        .values()
        .filter(|m| {
            Some(m.guild_id) == a.guild
                && !list
                    .iter()
                    .any(|o| o.kind == OverwriteKind::Member && o.target_id == m.user.id)
        })
        .collect();
    let parent = c.parent_id.and_then(|p| a.model.channels.get(&p));
    let label = a.model.channel_name(c);
    // Overwrites flow into nested channels, so offer what applies to this
    // channel or to anything inside it.
    let mut kinds = vec![c.kind];
    let mut frontier = vec![c.id];
    while let Some(p) = frontier.pop() {
        for ch in a.model.channels.values().filter(|x| x.parent_id == Some(p)) {
            if !kinds.contains(&ch.kind) {
                kinds.push(ch.kind);
            }
            frontier.push(ch.id);
        }
    }
    let groups: Vec<(&str, Vec<(&str, Permissions)>)> = CHANNEL_PERMISSIONS
        .iter()
        .map(|(group, bits)| {
            let bits = bits
                .iter()
                .copied()
                .filter(|(_, bit)| kinds.iter().any(|k| applies(*bit, *k)))
                .collect::<Vec<_>>();
            (*group, bits)
        })
        .filter(|(_, bits)| !bits.is_empty())
        .collect();
    view!{<h2>{format!("Permissions for {label}")}</h2>
        {summary.map(|s|view!{<p class="channel-access mb-4 rounded-lg bg-card p-3" role="status">{icon("users")}" "{s}</p>})}
        <fieldset disabled=!allowed>
        <div class="settings-list-row flex flex-wrap items-center gap-3 border-b py-3"><span class="settings-row-copy flex min-w-40 flex-1 flex-col gap-1"><strong class="font-semibold">"Private channel"</strong><small>"Only the roles and members you allow below can see it."</small></span>{button(if private{"Make Public"}else{"Make Private"},if private{"mod:ow-private,off"}else{"mod:ow-private,on"},"secondary")}</div>
        {parent.map(|p|view!{<div class="settings-list-row flex flex-wrap items-center gap-3 border-b py-3"><span class="settings-row-copy flex min-w-40 flex-1 flex-col gap-1"><strong class="font-semibold">{format!("Sync with {}",a.model.channel_name(p))}</strong><small>"This channel always follows the permissions of everything it is inside. Syncing removes its own changes so it matches exactly."</small></span>{button("Sync Now","mod:ow-sync","secondary")}</div>})}
        <div class="mt-4 flex flex-wrap items-end gap-2 phone:flex-col phone:items-stretch">
            <label class="field mb-0 min-w-40 flex-1"><span>"Add a role"</span><select id="ow-add-role">{unused_roles.iter().map(|r|view!{<option value=r.id.0.to_string()>{if r.id==everyone{"@everyone".to_string()}else{r.name.clone()}}</option>}).collect_view()}</select></label>{button("Add Role","mod:ow-add,role:","secondary")}
            <label class="field mb-0 min-w-40 flex-1"><span>"Add a member"</span><select id="ow-add-member">{unused_members.iter().map(|m|view!{<option value=m.user.id.0.to_string()>{name(&m.user)}</option>}).collect_view()}</select></label>{button("Add Member","mod:ow-add,member:","secondary")}
        </div>
        {if sorted.is_empty(){view!{<p class="muted-copy mt-6">"No changes yet: this channel uses the server’s role permissions (and those of everything it is inside)."</p>}.into_any()}else{().into_any()}}
        {sorted.into_iter().map(|o|{let who=target_name(a,o);let kind=if o.kind==OverwriteKind::Member{"member"}else{"role"};view!{<section class="overwrite mt-6 rounded-lg border p-4 phone:p-3" data-overwrite=format!("{kind}:{}",o.target_id.0)>
            <div class="mb-2 flex items-center gap-2"><strong class="flex-1 font-semibold wrap-anywhere">{who.clone()}</strong><small class="text-subtle">{if kind=="member"{"Member"}else{"Role"}}</small>{ib_plain("x",&format!("Remove {who}"),&format!("mod:ow-remove,{kind}:{}",o.target_id.0))}</div>
            {groups.iter().map(|(group,bits)|view!{<h3 class="category mt-3 mb-1">{*group}</h3>{bits.iter().map(|(label,bit)|{let state=if o.allow.contains(*bit){"allow"}else if o.deny.contains(*bit){"deny"}else{"inherit"};let n=radio_name(o,*bit);view!{<div class="tri-state flex items-center gap-2 border-b py-1.5" role="radiogroup" aria-label=format!("{label} for {who}")><span class="min-w-0 flex-1 text-sm">{*label}</span>
                {[("deny","x","Deny"),("inherit","minus","Inherit"),("allow","check","Allow")].into_iter().map(|(v,glyph,title)|view!{<label class=format!("tri-option grid size-9 cursor-pointer place-items-center rounded-md border has-checked:border-transparent touch:size-11 [&>.icon]:size-4 {}",match v{"allow"=>"has-checked:bg-primary has-checked:text-primary-foreground","deny"=>"has-checked:bg-destructive has-checked:text-primary-foreground",_=>"has-checked:bg-secondary"}) title=title><input class="sr-only" type="radio" name=n.clone() value=v checked=state==v aria-label=title/>{icon(glyph)}</label>}).collect_view()}
            </div>}}).collect_view()}}).collect_view()}
        </section>}}).collect_view()}
        </fieldset>
        <div class="sticky bottom-0 mt-6 flex flex-wrap gap-2 border-t bg-background py-3 phone:flex-col"><button class="button primary" data-live="mod:ow-save" disabled=!allowed>"Save Permissions"</button>{button("Back to Channel",&format!("dialog:channel-edit:{}",c.id.0),"secondary")}</div>
    }.into_any()
}
