//! Community tools in the web app: rules screening, applications, onboarding,
//! Browse Channels, the server directory, templates and insights. Views are in
//! `ui/community.rs`. Nothing here polls: data is fetched when a server or a
//! page is opened and kept current by `Event::Community`.
use super::*;
use std::collections::BTreeSet;

/// The onboarding flow a member is walking through.
#[derive(Clone, Default)]
pub(super) struct Flow {
    pub guild: Id,
    pub step: usize,
    pub answers: BTreeMap<u32, Vec<u32>>,
    /// Opened again from the server menu: every question on one page.
    pub revisit: bool,
    pub agreed: bool,
    /// The rules step is part of this flow (fixed when it starts: accepting
    /// them must not change which step comes next).
    pub rules: bool,
}
#[derive(Default)]
pub(super) struct State {
    /// The viewer's community state per server (bounded by the servers joined).
    pub states: BTreeMap<Id, CommunityState>,
    loading: BTreeSet<Id>,
    /// What the invite in the Join field asks of new members.
    pub gate: Option<(String, JoinGate)>,
    pub queue: Option<(Id, ApplicationQueue)>,
    pub listing: Option<(Id, ListingAdmin)>,
    pub templates: Option<(Id, Vec<GuildTemplate>)>,
    pub builtins: Vec<GuildTemplate>,
    /// A template opened by link (or a starter one) for its preview.
    pub template: Option<GuildTemplate>,
    pub insights: Option<(Id, Insights)>,
    pub insight_days: u32,
    pub weekly: bool,
    pub directory: Option<DirectoryPage>,
    pub query: String,
    pub category: Option<DirectoryCategory>,
    pub preview: Option<DirectoryPreview>,
    pub mine: Vec<MyApplication>,
    /// The onboarding being edited in server settings.
    pub draft: Option<Onboarding>,
    /// How many rule boxes the Rules Screening page shows.
    pub rule_slots: usize,
    pub question_slots: usize,
    pub flow: Option<Flow>,
    /// Servers whose flow opened by itself this session.
    offered: BTreeSet<Id>,
    /// A `?template=` link waiting for the app to be ready.
    link: Option<String>,
}

fn fail(app: &Shared, e: String) {
    let mut a = app.borrow_mut();
    a.error = e;
    a.touch();
}
pub(super) fn cm(a: &mut App, cmd: CommunityCommand) {
    a.command(Command::Community(cmd));
}
fn element(id: &str) -> Option<web_sys::Element> {
    document().get_element_by_id(id)
}
pub(super) fn read(id: &str) -> String {
    element(id)
        .and_then(|e| {
            e.dyn_ref::<web_sys::HtmlInputElement>()
                .map(|i| i.value())
                .or_else(|| e.dyn_ref::<web_sys::HtmlSelectElement>().map(|s| s.value()))
                .or_else(|| {
                    e.dyn_ref::<web_sys::HtmlTextAreaElement>()
                        .map(|t| t.value())
                })
        })
        .unwrap_or_default()
}
fn exists(id: &str) -> bool {
    element(id).is_some()
}
pub(super) fn checked(id: &str) -> bool {
    element(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|i| i.checked())
}
/// The chosen options of a `<select multiple>`.
fn selected(field: &str) -> Vec<Id> {
    let Some(s) = element(field).and_then(|e| e.dyn_into::<web_sys::HtmlSelectElement>().ok())
    else {
        return vec![];
    };
    let options = s.selected_options();
    (0..options.length())
        .filter_map(|i| options.item(i))
        .filter_map(|o| o.get_attribute("value"))
        .filter_map(|v| id(&v))
        .collect()
}
fn emoji_of(raw: String) -> Option<String> {
    Some(raw.trim().to_owned()).filter(|v| !v.is_empty())
}

/// Every Ready: opt in to community events, and act on a `?template=` link.
pub(super) fn ready(app: Shared) {
    {
        let mut a = app.borrow_mut();
        cm(&mut a, CommunityCommand::Subscribe);
        a.community.states.clear();
        a.community.loading.clear();
        if a.community.link.is_none() {
            let search = window().location().search().unwrap_or_default();
            if let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) {
                if let Some(code) = params.get("template").filter(|c| c.len() <= 64) {
                    a.community.link = Some(code);
                    params.delete("template");
                    let rest = params.to_string().as_string().unwrap_or_default();
                    let path = window().location().pathname().unwrap_or_default();
                    let url = if rest.is_empty() {
                        path
                    } else {
                        format!("{path}?{rest}")
                    };
                    if let Ok(h) = window().history() {
                        let _ = h.replace_state_with_url(&JsValue::NULL, "", Some(&url));
                    }
                }
            }
        }
    }
    let (guild, link) = {
        let mut a = app.borrow_mut();
        (a.guild, a.community.link.take())
    };
    if let Some(code) = link {
        super::operations::action(app.clone(), &format!("dialog:template:{code}"));
    }
    if let Some(g) = guild {
        refresh(app, g, true);
    }
}
/// A server was opened: know its community state; offer the new-member flow.
pub(super) fn guild_opened(app: Shared, g: Id) {
    let known = app.borrow().community.states.contains_key(&g);
    if known {
        offer(&app, g);
    } else {
        refresh(app, g, true);
    }
}
pub(super) fn refresh(app: Shared, g: Id, offer_flow: bool) {
    let (http, epoch) = {
        let mut a = app.borrow_mut();
        if !a.community.loading.insert(g) {
            return;
        }
        (a.http(), a.epoch)
    };
    spawn_local(async move {
        let result = http
            .json::<CommunityState>(&format!("/guilds/{}/community", g.0))
            .await;
        {
            let mut a = app.borrow_mut();
            a.community.loading.remove(&g);
            if a.epoch != epoch || !a.model.guilds.contains_key(&g) {
                return;
            }
            let Ok(state) = result else { return };
            a.community.states.insert(g, state);
            a.touch();
        }
        if offer_flow {
            offer(&app, g);
        }
    });
}
/// What the flow shows: questions to ask and rules to accept.
pub(super) fn needs_flow(s: &CommunityState) -> bool {
    let o = &s.onboarding;
    s.me.rules_pending
        || s.me.questions_pending
        || (!s.me.onboarded
            && o.enabled
            && (!o.questions.is_empty()
                || !o.welcome.description.is_empty()
                || !o.welcome.highlights.is_empty()))
}
/// Open the flow by itself once per session, when the member still has to do it.
fn offer(app: &Shared, g: Id) {
    let open = {
        let mut a = app.borrow_mut();
        let due = a.guild == Some(g)
            && a.dialog.is_empty()
            && !a.community.offered.contains(&g)
            && a.community.states.get(&g).is_some_and(needs_flow);
        if due {
            a.community.offered.insert(g);
        }
        due
    };
    if open {
        start_flow(app, g, false);
    }
}
fn start_flow(app: &Shared, g: Id, revisit: bool) {
    {
        let mut a = app.borrow_mut();
        let answers = a
            .community
            .states
            .get(&g)
            .map(|s| {
                s.me.responses
                    .iter()
                    .map(|r| (r.question_id, r.answer_ids.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let rules = a
            .community
            .states
            .get(&g)
            .is_some_and(|s| s.me.rules_pending);
        a.community.flow = Some(Flow {
            guild: g,
            step: 0,
            answers,
            revisit,
            agreed: false,
            rules,
        });
    }
    super::operations::action(app.clone(), "dialog:cm-flow");
}

/// A channel was opened: a "read this channel" guide task is done.
pub(super) fn channel_opened(app: Shared, c: Id) {
    let mut a = app.borrow_mut();
    let Some(g) = a.model.channels.get(&c).and_then(|ch| ch.guild_id) else {
        return;
    };
    let Some(s) = a.community.states.get(&g) else {
        return;
    };
    let due: Vec<u32> = s
        .onboarding
        .tasks
        .iter()
        .filter(|t| {
            t.kind == GuideTaskKind::Read && t.channel_id == c && !s.me.guide_done.contains(&t.id)
        })
        .map(|t| t.id)
        .collect();
    for task_id in due {
        cm(
            &mut a,
            CommunityCommand::CompleteTask {
                guild_id: g,
                task_id,
            },
        );
    }
}

pub(super) fn event(a: &mut App, e: &CommunityEvent) {
    match e {
        CommunityEvent::Updated { guild_id } => {
            if a.community.states.contains_key(guild_id) {
                if let Some(app) = a.owner.upgrade() {
                    let g = *guild_id;
                    a.community.loading.remove(&g);
                    spawn_local(async move { refresh(app, g, false) });
                }
            }
        }
        CommunityEvent::Me(m) => {
            if let Some(s) = a.community.states.get_mut(&m.guild_id) {
                s.me = (**m).clone();
            }
        }
        CommunityEvent::Application(my) => {
            a.community.mine.retain(|v| v.guild_id != my.guild_id);
            a.community.mine.insert(0, (**my).clone());
            let text = match my.status {
                ApplicationStatus::Approved => format!(
                    "Your application to {} was approved. Welcome!",
                    my.guild_name
                ),
                ApplicationStatus::Rejected => {
                    format!("Your application to {} was declined.", my.guild_name)
                }
                _ => String::new(),
            };
            if !text.is_empty() {
                a.notice = text.clone();
                a.error = text;
            }
        }
        CommunityEvent::Queue { guild_id, pending } => {
            if let Some(s) = a.community.states.get_mut(guild_id) {
                s.pending_applications = *pending;
            }
            if a.community
                .queue
                .as_ref()
                .is_some_and(|(g, _)| g == guild_id)
            {
                if let Some(app) = a.owner.upgrade() {
                    let g = *guild_id;
                    spawn_local(async move { load_queue(app, g).await });
                }
            }
        }
    }
}

/// Settings pages and dialogs this module renders.
pub(super) fn admin_page(name: &str) -> bool {
    matches!(
        name,
        "cm-screening"
            | "cm-applications"
            | "cm-queue"
            | "cm-onboarding"
            | "cm-discovery"
            | "cm-templates"
            | "cm-insights"
    )
}
/// A dialog opened: fetch what its page shows.
pub(super) fn opened(app: Shared, name: &str) {
    let guild = app.borrow().guild;
    let name = name.to_owned();
    match name.as_str() {
        "discover" => load_directory(app),
        "create" => {
            if app.borrow().community.builtins.is_empty() {
                spawn_local(async move { load_builtins(app).await });
            }
        }
        "cm-mine" => spawn_local(async move { load_mine(app).await }),
        v if v.starts_with("discover:") => {
            if let Some(g) = id(v.trim_start_matches("discover:")) {
                app.borrow_mut().community.preview = None;
                spawn_local(async move { load_preview(app, g).await });
            }
        }
        v if v.starts_with("template:") => {
            let code = v.trim_start_matches("template:").to_owned();
            app.borrow_mut().community.template = None;
            spawn_local(async move { load_template(app, code).await });
        }
        _ => {
            let Some(g) = guild else { return };
            match name.as_str() {
                "cm-queue" => spawn_local(async move { load_queue(app, g).await }),
                "cm-discovery" => {
                    refresh_roles(app.clone(), g);
                    spawn_local(async move { load_listing(app, g).await });
                }
                "cm-templates" => spawn_local(async move { load_templates(app, g).await }),
                "cm-insights" => spawn_local(async move { load_insights(app, g).await }),
                "cm-onboarding" => {
                    {
                        let mut a = app.borrow_mut();
                        a.community.draft = None;
                        a.community.states.remove(&g);
                    }
                    refresh_roles(app.clone(), g);
                    refresh(app, g, false);
                }
                "cm-screening" | "cm-applications" => {
                    {
                        let mut a = app.borrow_mut();
                        a.community.rule_slots = 0;
                        a.community.question_slots = 0;
                        a.community.states.remove(&g);
                    }
                    refresh(app, g, false);
                }
                "cm-browse" | "cm-guide" | "cm-flow"
                    if !app.borrow().community.states.contains_key(&g) =>
                {
                    refresh(app, g, false);
                }
                _ => {}
            }
        }
    }
}
fn refresh_roles(app: Shared, g: Id) {
    let http = app.borrow().http();
    spawn_local(async move {
        if let Ok(p) = http
            .json::<Snapshot<Vec<Role>>>(&format!("/guilds/{}/roles", g.0))
            .await
        {
            let mut a = app.borrow_mut();
            if !a.model.snapshot_valid(&p.cursor) {
                return;
            }
            a.roles.retain(|_, r| r.guild_id != g);
            for r in p.data {
                a.roles.insert(r.id, r);
            }
            a.touch();
        }
    });
}
/// Fetch `path` for server `g` and keep it, if `g` is still the open server.
async fn load<T: serde::de::DeserializeOwned>(
    app: Shared,
    g: Id,
    path: String,
    keep: fn(&mut App, Id, T),
) {
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    let result = http.json::<T>(&path).await;
    let mut a = app.borrow_mut();
    if a.epoch != epoch || a.guild != Some(g) {
        return;
    }
    match result {
        Ok(v) => keep(&mut a, g, v),
        Err(e) => a.error = e,
    }
    a.touch();
}
async fn load_queue(app: Shared, g: Id) {
    load(
        app,
        g,
        format!("/guilds/{}/applications", g.0),
        |a, g, v| a.community.queue = Some((g, v)),
    )
    .await
}
async fn load_listing(app: Shared, g: Id) {
    load(app, g, format!("/guilds/{}/listing", g.0), |a, g, v| {
        a.community.listing = Some((g, v))
    })
    .await
}
async fn load_templates(app: Shared, g: Id) {
    load(app, g, format!("/guilds/{}/templates", g.0), |a, g, v| {
        a.community.templates = Some((g, v))
    })
    .await
}
async fn load_insights(app: Shared, g: Id) {
    let (http, days) = {
        let mut a = app.borrow_mut();
        if a.community.insight_days == 0 {
            a.community.insight_days = 30;
        }
        (a.http(), a.community.insight_days)
    };
    let result = http
        .json::<Insights>(&format!("/guilds/{}/insights?days={days}", g.0))
        .await;
    let mut a = app.borrow_mut();
    if a.guild != Some(g) {
        return;
    }
    match result {
        Ok(v) => a.community.insights = Some((g, v)),
        Err(e) => a.error = e,
    }
    a.touch();
}
async fn load_builtins(app: Shared) {
    let http = app.borrow().http();
    if let Ok(list) = http.json::<Vec<GuildTemplate>>("/templates").await {
        let mut a = app.borrow_mut();
        a.community.builtins = list;
        a.touch();
    }
}
async fn load_template(app: Shared, code: String) {
    let http = app.borrow().http();
    let path = format!("/templates/{}", js_sys::encode_uri_component(&code));
    match http.json::<GuildTemplate>(&path).await {
        Ok(t) => {
            let mut a = app.borrow_mut();
            if a.dialog == format!("template:{code}") {
                a.community.template = Some(t);
                a.touch();
            }
        }
        Err(e) => fail(&app, e),
    }
}
async fn load_mine(app: Shared) {
    let http = app.borrow().http();
    if let Ok(list) = http
        .json::<Vec<MyApplication>>("/users/@me/applications")
        .await
    {
        let mut a = app.borrow_mut();
        a.community.mine = list;
        a.touch();
    }
}
async fn load_preview(app: Shared, g: Id) {
    let http = app.borrow().http();
    match http
        .json::<DirectoryPreview>(&format!("/directory/{}", g.0))
        .await
    {
        Ok(p) => {
            let mut a = app.borrow_mut();
            if a.dialog == format!("discover:{}", g.0) {
                a.community.preview = Some(p);
                a.touch();
            }
        }
        Err(e) => fail(&app, e),
    }
}
fn load_directory(app: Shared) {
    let (http, path) = {
        let a = app.borrow();
        let mut path = format!(
            "/directory?q={}",
            js_sys::encode_uri_component(a.community.query.trim())
        );
        if let Some(c) = a.community.category {
            path.push_str(&format!("&category={}", c.name()));
        }
        (a.http(), path)
    };
    spawn_local(async move {
        match http.json::<DirectoryPage>(&path).await {
            Ok(p) => {
                let mut a = app.borrow_mut();
                a.community.directory = Some(p);
                a.touch();
            }
            Err(e) => fail(&app, e),
        }
    });
}
/// The Join field holds an invite: learn what its server asks of new members.
pub(super) fn invite_gate(app: Shared, code: String) {
    let http = app.borrow().http();
    app.borrow_mut().community.gate = None;
    spawn_local(async move {
        if let Ok(gate) = http
            .json::<JoinGate>(&format!("/invites/{code}/admission"))
            .await
        {
            let mut a = app.borrow_mut();
            if a.invite_wanted == code {
                a.community.gate = Some((code, gate));
                a.touch();
            }
        }
    });
}

/// The Browse Channels rule for the sidebar: a channel that is opt-in is shown
/// only to members who opted in (or show all), and always while it is open or
/// mentions you. Everything nested inside a hidden channel is hidden with it.
pub(super) fn shown<'a>(
    a: &App,
    rows: Vec<(&'a ChannelSummary, usize)>,
) -> Vec<(&'a ChannelSummary, usize)> {
    let Some(s) = a.guild.and_then(|g| a.community.states.get(&g)) else {
        return rows;
    };
    let opt_in = &s.onboarding.opt_in_channel_ids;
    // People who manage the channels always see all of them.
    let manager = a
        .guild
        .and_then(|g| a.model.guilds.get(&g))
        .is_some_and(|g| {
            g.my_role == MemberRole::Owner
                || g.my_permissions
                    .intersects(Permissions::ADMINISTRATOR | Permissions::MANAGE_CHANNELS)
        });
    if s.me.show_all || opt_in.is_empty() || manager {
        return rows;
    }
    let mut skip: Option<usize> = None;
    rows.into_iter()
        .filter(|(c, depth)| {
            if skip.is_some_and(|d| *depth > d) {
                return false;
            }
            skip = None;
            let mentioned = a
                .model
                .reads
                .get(&c.id)
                .is_some_and(|r| r.mention_count > 0);
            let hidden = opt_in.contains(&c.id)
                && !s.me.opted_in.contains(&c.id)
                && a.channel != Some(c.id)
                && !mentioned;
            if hidden {
                skip = Some(*depth);
            }
            !hidden
        })
        .collect()
}

/// `data-live="cm:<what>,<arg>"` actions.
pub(super) fn action(app: Shared, arg: &str) {
    let (what, rest) = arg.split_once(',').unwrap_or((arg, ""));
    let guild = app.borrow().guild;
    match what {
        "flow" => {
            if let Some(g) = guild {
                start_flow(&app, g, rest == "revisit");
            }
        }
        "pick" => {
            let mut parts = rest.split(',').filter_map(|v| v.parse::<u32>().ok());
            if let (Some(q), Some(answer)) = (parts.next(), parts.next()) {
                let mut a = app.borrow_mut();
                let multi = a
                    .community
                    .flow
                    .as_ref()
                    .and_then(|f| a.community.states.get(&f.guild))
                    .and_then(|s| s.onboarding.questions.iter().find(|x| x.id == q))
                    .is_some_and(|x| x.multi);
                if let Some(f) = a.community.flow.as_mut() {
                    let list = f.answers.entry(q).or_default();
                    if list.contains(&answer) {
                        list.retain(|v| *v != answer);
                    } else if multi {
                        list.push(answer);
                    } else {
                        *list = vec![answer];
                    }
                }
                a.touch();
            }
        }
        "agree" => {
            let mut a = app.borrow_mut();
            if let Some(f) = a.community.flow.as_mut() {
                f.agreed = checked("cm-agree");
            }
            a.touch();
        }
        "next" | "back" | "skip" => flow_step(&app, what),
        "save-answers" => {
            let mut a = app.borrow_mut();
            let Some(f) = a.community.flow.clone() else {
                return;
            };
            cm(
                &mut a,
                CommunityCommand::Respond {
                    guild_id: f.guild,
                    responses: responses(&f),
                    finish: true,
                },
            );
        }
        "open" => {
            if let Some(c) = id(rest) {
                {
                    let mut a = app.borrow_mut();
                    a.dialog.clear();
                    a.dialog_generation = a.dialog_generation.wrapping_add(1);
                }
                super::operations::load_channel(app, c, None);
            }
        }
        "browse-save" => {
            let Some(g) = guild else { return };
            let mut a = app.borrow_mut();
            let Some(s) = a.community.states.get(&g) else {
                return;
            };
            let opted: Vec<Id> = s
                .onboarding
                .opt_in_channel_ids
                .iter()
                .copied()
                .filter(|c| checked(&format!("cm-browse-{}", c.0)))
                .collect();
            cm(
                &mut a,
                CommunityCommand::SetChannels {
                    guild_id: g,
                    opted_in: opted,
                    show_all: checked("cm-show-all"),
                },
            );
        }
        // One more box than the page shows now.
        "rule-add" => {
            let mut a = app.borrow_mut();
            let rules = guild
                .and_then(|g| a.community.states.get(&g))
                .map_or(0, |s| s.admission.rules.len());
            let shown = (rules + 1).max(a.community.rule_slots).max(1);
            a.community.rule_slots = (shown + 1).min(MAX_RULES);
            a.touch();
        }
        "question-add" => {
            let mut a = app.borrow_mut();
            let asked = guild
                .and_then(|g| a.community.states.get(&g))
                .map_or(0, |s| s.admission.questions.len());
            let shown = asked.max(1).max(a.community.question_slots);
            a.community.question_slots = (shown + 1).min(MAX_APPLICATION_QUESTIONS);
            a.touch();
        }
        // Onboarding editor: change the draft's shape, keeping what is typed.
        "q-add" | "q-remove" | "a-add" | "a-remove" | "preview" => {
            let mut a = app.borrow_mut();
            let Some(mut o) = a.community.draft.take().or_else(|| {
                guild
                    .and_then(|g| a.community.states.get(&g))
                    .map(|s| s.onboarding.clone())
            }) else {
                return;
            };
            sync_draft(&a, &mut o);
            let nums: Vec<u32> = rest.split(',').filter_map(|v| v.parse().ok()).collect();
            match (what, nums.as_slice()) {
                ("q-add", _) if o.questions.len() < MAX_ONBOARDING_QUESTIONS => {
                    let next = o.questions.iter().map(|q| q.id).max().unwrap_or(0) + 1;
                    o.questions.push(OnboardingQuestion {
                        id: next,
                        title: String::new(),
                        multi: false,
                        required: false,
                        pre_join: false,
                        answers: vec![OnboardingAnswer {
                            id: 1,
                            ..Default::default()
                        }],
                    });
                }
                ("q-remove", [q]) => o.questions.retain(|x| x.id != *q),
                ("a-add", [q]) => {
                    if let Some(x) = o.questions.iter_mut().find(|x| x.id == *q) {
                        if x.answers.len() < MAX_ONBOARDING_ANSWERS {
                            let next = x.answers.iter().map(|v| v.id).max().unwrap_or(0) + 1;
                            x.answers.push(OnboardingAnswer {
                                id: next,
                                ..Default::default()
                            });
                        }
                    }
                }
                ("a-remove", [q, answer]) => {
                    if let Some(x) = o.questions.iter_mut().find(|x| x.id == *q) {
                        if x.answers.len() > 1 {
                            x.answers.retain(|v| v.id != *answer);
                        }
                    }
                }
                _ => {}
            }
            a.community.draft = Some(o);
            a.touch();
        }
        "review" | "reject" | "interview" => {
            let (Some(g), Some(v)) = (guild, id(rest)) else {
                return;
            };
            let mut a = app.borrow_mut();
            let cmd = match what {
                "interview" => CommunityCommand::Interview {
                    guild_id: g,
                    user_id: v,
                },
                _ => CommunityCommand::Review {
                    guild_id: g,
                    user_id: v,
                    approve: what == "review",
                    reason: Some(read(&format!("cm-reason-{}", v.0)).trim().to_owned())
                        .filter(|r| !r.is_empty()),
                },
            };
            cm(&mut a, cmd);
        }
        "withdraw" => {
            if let Some(g) = id(rest) {
                cm(
                    &mut app.borrow_mut(),
                    CommunityCommand::Withdraw { guild_id: g },
                );
            }
        }
        "unlist" => {
            if let Some(g) = guild {
                cm(
                    &mut app.borrow_mut(),
                    CommunityCommand::SetListing {
                        guild_id: g,
                        listing: None,
                    },
                );
            }
        }
        "template-sync" => cm(
            &mut app.borrow_mut(),
            CommunityCommand::SyncTemplate { code: rest.into() },
        ),
        "template-delete" => cm(
            &mut app.borrow_mut(),
            CommunityCommand::DeleteTemplate { code: rest.into() },
        ),
        "insights-days" => {
            let days = read("cm-insight-days").parse().unwrap_or(30);
            app.borrow_mut().community.insight_days = days;
            if let Some(g) = guild {
                spawn_local(async move { load_insights(app, g).await });
            }
        }
        "insights-weekly" => {
            let mut a = app.borrow_mut();
            a.community.weekly = !a.community.weekly;
            a.touch();
        }
        "category" => {
            {
                let mut a = app.borrow_mut();
                a.community.category = DirectoryCategory::parse(rest);
                a.community.query.clear();
            }
            load_directory(app);
        }
        "join-discovered" => {
            if let Some(g) = id(rest) {
                cm(
                    &mut app.borrow_mut(),
                    CommunityCommand::JoinDiscovered { guild_id: g },
                );
            }
        }
        "template-link" => submit(app, "cm-template-link"),
        "go" => {
            if let Some(g) = id(rest) {
                {
                    let mut a = app.borrow_mut();
                    a.dialog.clear();
                    a.dialog_generation = a.dialog_generation.wrapping_add(1);
                }
                super::operations::action(app, &format!("guild:{}", g.0));
            }
        }
        _ => {}
    }
}
fn responses(f: &Flow) -> Vec<OnboardingResponse> {
    f.answers
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(q, v)| OnboardingResponse {
            question_id: *q,
            answer_ids: v.clone(),
        })
        .collect()
}
/// The steps of the flow for server state `s`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Question(usize),
    Rules,
    Welcome,
}
pub(super) fn steps(s: &CommunityState, rules: bool) -> Vec<Step> {
    let o = &s.onboarding;
    let mut out = vec![];
    let asks = o.enabled;
    if asks {
        out.extend(
            o.questions
                .iter()
                .enumerate()
                .filter(|(_, q)| q.pre_join)
                .map(|(i, _)| Step::Question(i)),
        );
    }
    if rules {
        out.push(Step::Rules);
    }
    if asks {
        out.extend(
            o.questions
                .iter()
                .enumerate()
                .filter(|(_, q)| !q.pre_join)
                .map(|(i, _)| Step::Question(i)),
        );
        if !o.welcome.description.is_empty() || !o.welcome.highlights.is_empty() {
            out.push(Step::Welcome);
        }
    }
    out
}
fn flow_step(app: &Shared, what: &str) {
    let mut a = app.borrow_mut();
    let Some(mut f) = a.community.flow.clone() else {
        return;
    };
    let Some(s) = a.community.states.get(&f.guild).cloned() else {
        return;
    };
    let list = steps(&s, f.rules);
    let at = list.get(f.step).copied();
    if what == "back" {
        f.step = f.step.saturating_sub(1);
        a.community.flow = Some(f);
        a.touch();
        return;
    }
    match at {
        Some(Step::Question(i)) => {
            let q = &s.onboarding.questions[i];
            let answered = f.answers.get(&q.id).is_some_and(|v| !v.is_empty());
            if q.required && !answered && what != "skip" {
                a.error = "Choose an answer to continue.".into();
                a.touch();
                return;
            }
            // Leaving the last pre-join question: the server lifts the hold.
            let next_is_pre = matches!(list.get(f.step + 1), Some(Step::Question(j)) if s.onboarding.questions[*j].pre_join);
            if q.pre_join && !next_is_pre && s.me.questions_pending {
                cm(
                    &mut a,
                    CommunityCommand::Respond {
                        guild_id: f.guild,
                        responses: responses(&f),
                        finish: false,
                    },
                );
            }
        }
        Some(Step::Rules) if s.me.rules_pending => {
            if !f.agreed {
                a.error = "Tick “I have read and agree to the rules” to continue.".into();
                a.touch();
                return;
            }
            cm(
                &mut a,
                CommunityCommand::AcceptRules {
                    guild_id: f.guild,
                    rules_version: s.admission.rules_version,
                },
            );
        }
        _ => {}
    }
    if f.step + 1 >= list.len() {
        cm(
            &mut a,
            CommunityCommand::Respond {
                guild_id: f.guild,
                responses: if s.onboarding.enabled {
                    responses(&f)
                } else {
                    vec![]
                },
                finish: true,
            },
        );
        a.community.flow = None;
        a.dialog.clear();
        a.dialog_generation = a.dialog_generation.wrapping_add(1);
        a.touch();
        return;
    }
    f.step += 1;
    a.community.flow = Some(f);
    a.touch();
}

/// Copy what is typed in the onboarding editor into the draft.
fn sync_draft(a: &App, o: &mut Onboarding) {
    if !exists("cm-ob-enabled") {
        return;
    }
    o.enabled = checked("cm-ob-enabled");
    for q in &mut o.questions {
        let p = format!("cm-q{}", q.id);
        if !exists(&format!("{p}-title")) {
            continue;
        }
        q.title = read(&format!("{p}-title"));
        q.multi = checked(&format!("{p}-multi"));
        q.required = checked(&format!("{p}-required"));
        q.pre_join = checked(&format!("{p}-pre"));
        for an in &mut q.answers {
            let p = format!("{p}-a{}", an.id);
            if !exists(&format!("{p}-label")) {
                continue;
            }
            an.label = read(&format!("{p}-label"));
            an.emoji = emoji_of(read(&format!("{p}-emoji")));
            an.role_ids = selected(&format!("{p}-roles"));
            an.channel_ids = selected(&format!("{p}-channels"));
        }
    }
    let guild_channels: Vec<Id> = a
        .model
        .channels
        .values()
        .filter(|c| c.guild_id == a.guild && c.kind != ChannelKind::Thread)
        .map(|c| c.id)
        .collect();
    if exists("cm-defaults") {
        o.opt_in_channel_ids = guild_channels
            .iter()
            .copied()
            .filter(|c| {
                exists(&format!("cm-default-{}", c.0)) && !checked(&format!("cm-default-{}", c.0))
            })
            .collect();
    }
    o.welcome.description = read("cm-welcome");
    o.welcome.highlights = (0..MAX_HIGHLIGHTS)
        .filter_map(|i| {
            let ch = id(&read(&format!("cm-hl{i}-ch")))?;
            Some(Highlight {
                channel_id: ch,
                text: read(&format!("cm-hl{i}-text")),
                emoji: emoji_of(read(&format!("cm-hl{i}-emoji"))),
            })
        })
        .collect();
    let old = std::mem::take(&mut o.tasks);
    o.tasks = (0..MAX_GUIDE_TASKS)
        .filter_map(|i| {
            let ch = id(&read(&format!("cm-task{i}-ch")))?;
            Some(GuideTask {
                id: old.get(i).map_or(0, |t| t.id),
                kind: if read(&format!("cm-task{i}-kind")) == "talk" {
                    GuideTaskKind::Talk
                } else {
                    GuideTaskKind::Read
                },
                channel_id: ch,
                title: read(&format!("cm-task{i}-title")),
            })
        })
        .collect();
    o.resources = (0..MAX_GUIDE_RESOURCES)
        .filter_map(|i| {
            let ch = id(&read(&format!("cm-res{i}-ch")))?;
            Some(GuideResource {
                channel_id: ch,
                title: read(&format!("cm-res{i}-title")),
                description: read(&format!("cm-res{i}-desc")),
            })
        })
        .collect();
}

/// The questions on the Applications page.
fn read_questions() -> Vec<ApplicationQuestion> {
    (0..MAX_APPLICATION_QUESTIONS)
        .filter_map(|i| {
            let prompt = read(&format!("cm-aq{i}-prompt")).trim().to_owned();
            if prompt.is_empty() {
                return None;
            }
            let kind = match read(&format!("cm-aq{i}-kind")).as_str() {
                "paragraph" => QuestionKind::Paragraph,
                "choice" => QuestionKind::Choice,
                _ => QuestionKind::Short,
            };
            Some(ApplicationQuestion {
                kind,
                prompt,
                required: checked(&format!("cm-aq{i}-required")),
                choices: if kind == QuestionKind::Choice {
                    read(&format!("cm-aq{i}-choices"))
                        .lines()
                        .map(|l| l.trim().to_owned())
                        .filter(|l| !l.is_empty())
                        .collect()
                } else {
                    vec![]
                },
            })
        })
        .collect()
}

pub(super) fn submit(app: Shared, form: &str) {
    media_action(&format!("form-sent:{form}"));
    let mut a = app.borrow_mut();
    let guild = a.guild;
    let fail_form = |a: &mut App, text: &str| {
        a.error = text.into();
        media_action(&format!("form-failed:{form}"));
        a.touch();
    };
    match form {
        "cm-screening" | "cm-applications" => {
            let Some(g) = guild else { return };
            let Some(s) = a.community.states.get(&g).cloned() else {
                return;
            };
            let mut admission = s.admission.clone();
            if form == "cm-screening" {
                admission.screening = checked("cm-screening-on");
                admission.description = read("cm-screening-description");
                admission.rules = (0..MAX_RULES)
                    .map(|i| read(&format!("cm-rule-{i}")).trim().to_owned())
                    .filter(|r| !r.is_empty())
                    .collect();
            } else {
                admission.applications = checked("cm-applications-on");
                admission.questions = read_questions();
            }
            cm(
                &mut a,
                CommunityCommand::SetAdmission {
                    guild_id: g,
                    screening: admission.screening,
                    description: admission.description,
                    rules: admission.rules,
                    reaccept: form == "cm-screening" && checked("cm-reaccept"),
                    applications: admission.applications,
                    questions: admission.questions,
                },
            );
        }
        "cm-onboarding" => {
            let Some(g) = guild else { return };
            let Some(mut o) = a
                .community
                .draft
                .clone()
                .or_else(|| a.community.states.get(&g).map(|s| s.onboarding.clone()))
            else {
                return;
            };
            sync_draft(&a, &mut o);
            o.guild_id = g;
            a.community.draft = Some(o.clone());
            cm(&mut a, CommunityCommand::SetOnboarding(o));
        }
        "cm-apply" => {
            let Some(via) = apply_via(&a) else { return };
            let questions = match &via {
                JoinVia::Invite { .. } => {
                    a.community.gate.as_ref().map(|(_, g)| g.questions.clone())
                }
                JoinVia::Discovery { .. } => a
                    .community
                    .preview
                    .as_ref()
                    .map(|p| p.gate.questions.clone()),
            }
            .unwrap_or_default();
            let answers = (0..questions.len())
                .map(|i| read(&format!("cm-answer-{i}")).trim().to_owned())
                .collect();
            cm(
                &mut a,
                CommunityCommand::Apply {
                    via,
                    answers,
                    accept_rules: checked("cm-apply-rules"),
                },
            );
        }
        "cm-listing" => {
            let Some(g) = guild else { return };
            let description = read("cm-listing-description").trim().to_owned();
            if description.is_empty() {
                fail_form(&mut a, "Describe your server in a sentence or two.");
                return;
            }
            let featured = selected("cm-listing-emoji");
            cm(
                &mut a,
                CommunityCommand::SetListing {
                    guild_id: g,
                    listing: Some(ListingDraft {
                        category: DirectoryCategory::parse(&read("cm-listing-category"))
                            .unwrap_or_default(),
                        description,
                        tags: read("cm-listing-tags")
                            .split(',')
                            .map(|t| t.trim().to_owned())
                            .filter(|t| !t.is_empty())
                            .collect(),
                        language: read("cm-listing-language").trim().to_owned(),
                        featured_emoji: featured,
                    }),
                },
            );
        }
        "cm-template-create" => {
            let Some(g) = guild else { return };
            let name = read("cm-template-name").trim().to_owned();
            if name.is_empty() {
                fail_form(&mut a, "Give the template a name.");
                return;
            }
            cm(
                &mut a,
                CommunityCommand::CreateTemplate {
                    guild_id: g,
                    name,
                    description: read("cm-template-description").trim().to_owned(),
                },
            );
        }
        "cm-from-template" => {
            let Some(t) = a.community.template.clone() else {
                return;
            };
            let name = read("cm-server-name").trim().to_owned();
            if name.is_empty() {
                fail_form(&mut a, "Name your new server.");
                return;
            }
            let template = match t.code.strip_prefix("builtin:") {
                Some(key) => TemplateRef::Builtin(key.into()),
                None => TemplateRef::Code(t.code.clone()),
            };
            cm(
                &mut a,
                CommunityCommand::CreateFromTemplate { template, name },
            );
        }
        "cm-template-link" => {
            let raw = read("cm-template-code");
            let code = raw
                .rsplit(['=', '/'])
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            drop(a);
            if !code.is_empty() {
                super::operations::action(app, &format!("dialog:template:{code}"));
            }
        }
        // A search is not an edit: nothing is left unsaved.
        "cm-search" => {
            media_action("form-saved:cm-search");
            a.community.query = read("cm-directory-q");
            a.community.category = None;
            drop(a);
            load_directory(app);
        }
        _ => {}
    }
}
/// How the open application dialog reaches its server.
pub(super) fn apply_via(a: &App) -> Option<JoinVia> {
    let rest = a.dialog.strip_prefix("cm-apply:")?;
    match rest.split_once(':')? {
        ("invite", code) => Some(JoinVia::Invite { code: code.into() }),
        ("discovery", g) => id(g).map(|guild_id| JoinVia::Discovery { guild_id }),
        _ => None,
    }
}

pub(super) fn owns(op: &Command) -> bool {
    matches!(op, Command::Community(_))
}
/// A refused command of ours: the form that sent it is no longer waiting.
pub(super) fn failed(op: Option<&Command>) {
    let form = match op {
        Some(Command::Community(cmd)) => match cmd {
            CommunityCommand::SetAdmission {
                applications,
                questions,
                ..
            } => {
                if *applications || !questions.is_empty() {
                    "cm-applications"
                } else {
                    "cm-screening"
                }
            }
            CommunityCommand::SetOnboarding(_) => "cm-onboarding",
            CommunityCommand::Apply { .. } => "cm-apply",
            CommunityCommand::SetListing { .. } => "cm-listing",
            CommunityCommand::CreateTemplate { .. } => "cm-template-create",
            CommunityCommand::CreateFromTemplate { .. } => "cm-from-template",
            _ => return,
        },
        _ => return,
    };
    media_action(&format!("form-failed:{form}"));
}
fn saved(a: &mut App, form: &'static str, text: &str) {
    media_action(&format!("form-saved:{form}"));
    a.saved = Some((a.dialog_generation, form));
    if !text.is_empty() {
        a.notice = text.into();
        a.error = text.into();
    }
}
pub(super) fn answered(app: Shared, op: Command, result: CommandResult) {
    let Command::Community(cmd) = op else { return };
    let mut a = app.borrow_mut();
    a.error.clear();
    a.notice.clear();
    let guild = a.guild;
    match (&cmd, result) {
        (_, CommandResult::Guild(d)) => {
            let g = d.summary.guild.id;
            a.dialog.clear();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
            a.touch();
            drop(a);
            if app.borrow().model.guilds.contains_key(&g) {
                super::operations::action(app, &format!("guild:{}", g.0));
            }
            return;
        }
        (CommunityCommand::Interview { .. }, CommandResult::Channel(ch)) => {
            a.dialog.clear();
            a.touch();
            drop(a);
            super::operations::load_channel(app, ch.summary.id, None);
            return;
        }
        (
            CommunityCommand::SetAdmission { .. },
            CommandResult::Community(CommunityResult::Admission(adm)),
        ) => {
            if let Some(s) = a.community.states.get_mut(&adm.guild_id) {
                s.admission = *adm;
            }
            let form = if a.dialog == "cm-applications" {
                "cm-applications"
            } else {
                "cm-screening"
            };
            saved(&mut a, form, "");
        }
        (
            CommunityCommand::SetOnboarding(_),
            CommandResult::Community(CommunityResult::Onboarding(o)),
        ) => {
            if let Some(s) = a.community.states.get_mut(&o.guild_id) {
                s.onboarding = (*o).clone();
            }
            a.community.draft = Some(*o);
            saved(&mut a, "cm-onboarding", "");
        }
        (
            CommunityCommand::AcceptRules { .. }
            | CommunityCommand::Respond { .. }
            | CommunityCommand::SetChannels { .. }
            | CommunityCommand::CompleteTask { .. },
            CommandResult::Community(CommunityResult::Me(m)),
        ) => {
            let browse = matches!(cmd, CommunityCommand::SetChannels { .. });
            let revisit = matches!(cmd, CommunityCommand::Respond { finish: true, .. })
                && a.dialog == "cm-flow";
            if let Some(s) = a.community.states.get_mut(&m.guild_id) {
                s.me = *m;
            }
            if browse {
                a.notice = "Channel list updated.".into();
                a.error = a.notice.clone();
            }
            if revisit {
                a.dialog.clear();
                a.community.flow = None;
                a.notice = "Your answers were saved.".into();
                a.error = a.notice.clone();
            }
        }
        (
            CommunityCommand::Apply { .. },
            CommandResult::Community(CommunityResult::Application(my)),
        ) => {
            if let Some((_, g)) = a.community.gate.as_mut() {
                g.application = Some((*my).clone());
            }
            if let Some(p) = a.community.preview.as_mut() {
                p.gate.application = Some((*my).clone());
            }
            a.community.mine.retain(|v| v.guild_id != my.guild_id);
            a.community.mine.insert(0, *my);
            media_action("form-saved:cm-apply");
            a.dialog = "cm-mine".into();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
            a.notice = "Application sent. You’ll hear back here.".into();
            a.error = a.notice.clone();
        }
        (
            CommunityCommand::Withdraw { .. },
            CommandResult::Community(CommunityResult::Application(my)),
        ) => {
            a.community.mine.retain(|v| v.guild_id != my.guild_id);
            a.community.mine.insert(0, *my);
            a.notice = "Application withdrawn.".into();
            a.error = a.notice.clone();
        }
        (CommunityCommand::Review { approve, .. }, _) => {
            a.notice = if *approve {
                "Application approved."
            } else {
                "Application declined."
            }
            .into();
            a.error = a.notice.clone();
            a.touch();
            drop(a);
            if let Some(g) = guild {
                spawn_local(async move { load_queue(app, g).await });
            }
            return;
        }
        (
            CommunityCommand::SetListing { .. },
            CommandResult::Community(CommunityResult::Listing(l)),
        ) => {
            if let Some((_, admin)) = a.community.listing.as_mut() {
                admin.listing = l.map(|l| *l);
            }
            let removed = a
                .community
                .listing
                .as_ref()
                .is_some_and(|(_, x)| x.listing.is_none());
            saved(
                &mut a,
                "cm-listing",
                if removed {
                    "Removed from the directory."
                } else {
                    ""
                },
            );
        }
        (
            CommunityCommand::CreateTemplate { .. }
            | CommunityCommand::SyncTemplate { .. }
            | CommunityCommand::DeleteTemplate { .. },
            _,
        ) => {
            if matches!(cmd, CommunityCommand::CreateTemplate { .. }) {
                saved(
                    &mut a,
                    "cm-template-create",
                    "Template created. Copy its link to share it.",
                );
            } else if matches!(cmd, CommunityCommand::SyncTemplate { .. }) {
                a.notice = "Template synced.".into();
                a.error = a.notice.clone();
            }
            drop(a);
            if let Some(g) = guild {
                spawn_local(async move { load_templates(app, g).await });
            }
            return;
        }
        _ => {}
    }
    a.touch();
}
