//! Apps in the web app: the Developer section, the install consent screen,
//! Server Settings › Integrations, channel webhooks, the `/` command picker,
//! interactions (ephemeral answers, forms) and app messages. Views are in
//! `ui/apps.rs` and load on first use. Nothing here polls: data is fetched
//! when a page opens (or a picker needs it) and kept current by `Event::Apps`.
//!
//! Lazily loaded (docs/features/perf.md): `state` (its events) and `run`
//! (pages, forms, actions and the answers to the commands those send) are
//! split modules. The composer's paths are small modules of their own, so
//! typing `/` or sending `/shrug` does not wait for the pages: the picker
//! (`compose_lazy`, and its view `ui::apps::load_picker`), sending a command
//! (`run_slash`) and a channel's command list (`load_catalog`). The core keeps
//! only the state struct and the checks that decide whether to hand over.
use super::features::{self, Call, Feature, Reply};
use super::*;
use std::collections::BTreeSet;

/// Commands the app itself handles, listed in the picker before apps'.
pub(super) const BUILTINS: &[(&str, &str, &str)] = &[
    ("shrug", "[message]", "Appends ¯\\_(ツ)_/¯ to your message."),
    (
        "tableflip",
        "[message]",
        "Appends (╯°□°)╯︵ ┻━┻ to your message.",
    ),
    (
        "unflip",
        "[message]",
        "Appends ┬─┬ ノ( ゜-゜ノ) to your message.",
    ),
    ("me", "<action>", "Sends your message as an action."),
    (
        "spoiler",
        "<message>",
        "Hides your message until it is clicked.",
    ),
    (
        "nick",
        "[nickname]",
        "Changes your nickname in this server.",
    ),
    (
        "tts",
        "<message>",
        "Reads your message aloud to the people viewing the channel.",
    ),
];
const MAX_CATALOGS: usize = 8;
const MAX_EPHEMERAL: usize = 50;

/// One row of the `/` picker.
#[derive(Clone, PartialEq)]
pub(super) struct Row {
    pub group: String,
    pub label: String,
    pub detail: String,
    pub insert: String,
}
#[derive(Clone, Default)]
pub(super) struct Picker {
    pub rows: Vec<Row>,
    /// The command's signature and what the option being typed wants.
    pub hint: String,
    /// The text the rows were computed for (`complete:` request).
    pub request: String,
    /// Suggestions from the bot for the option being typed.
    pub auto: Option<(u32, String, Vec<Choice>)>,
}

#[derive(Default)]
pub(super) struct State {
    pub views: Option<super::ui::apps::Views>,
    loading: std::cell::Cell<bool>,
    pub developer: Option<DeveloperState>,
    /// A bot token just created or reset: shown once.
    pub token: Option<(Id, String)>,
    pub install: Option<Result<InstallPreview, String>>,
    install_link: Option<Id>,
    pub integrations: Option<(Id, GuildIntegrations)>,
    pub webhooks: Option<(Id, Vec<Webhook>)>,
    /// A webhook URL just created or rotated: shown once.
    pub secret: Option<(Id, String)>,
    pub catalogs: Vec<CommandCatalog>,
    wanted: BTreeSet<Id>,
    pub picker: Option<Picker>,
    pub ephemeral: Vec<EphemeralMessage>,
    /// A form a bot opened: (interaction, app, channel, form).
    pub modal: Option<(Id, Id, Id, Modal)>,
    request: u32,
    /// Send the next message as typed (it starts with `/` but is no command).
    bypass: bool,
    /// What an uploaded avatar is for.
    icon_for: Option<(String, Id)>,
}

fn fail(app: &Shared, e: String) {
    let mut a = app.borrow_mut();
    a.error = e;
    a.touch();
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
fn checked(id: &str) -> bool {
    element(id)
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|i| i.checked())
}
fn selected(field: &str) -> Vec<String> {
    let Some(s) = element(field).and_then(|e| e.dyn_into::<web_sys::HtmlSelectElement>().ok())
    else {
        return vec![];
    };
    let options = s.selected_options();
    (0..options.length())
        .filter_map(|i| options.item(i))
        .filter_map(|o| o.get_attribute("value"))
        .collect()
}
fn cmd(a: &mut App, c: AppCommand) {
    a.command(Command::Apps(c));
}

/// Loads the views the first time something needs them.
pub(super) fn views(a: &App) -> Option<super::ui::apps::Views> {
    if a.apps.views.is_none() && !a.apps.loading.replace(true) {
        let owner = a.owner.clone();
        spawn_local(async move {
            let v = super::ui::apps::load_views().await;
            if let Some(app) = owner.upgrade() {
                app.borrow_mut().apps.views = Some(v);
                app.borrow().touch();
            }
        });
    }
    a.apps.views
}

/// Every Ready: opt in to app events, and act on an `?app=` install link.
pub(super) fn ready(app: Shared) {
    let link = {
        let mut a = app.borrow_mut();
        cmd(&mut a, AppCommand::Subscribe);
        a.apps.catalogs.clear();
        a.apps.ephemeral.clear();
        if a.apps.install_link.is_none() {
            a.apps.install_link = link_from_location();
        }
        a.apps.install_link.take()
    };
    if let Some(id) = link {
        app.borrow_mut().apps.install_link = Some(id);
        super::operations::action(app, "dialog:app-install");
    }
}
fn link_from_location() -> Option<Id> {
    let search = window().location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    let value = params.get("app")?;
    params.delete("app");
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
    id(&value)
}

fn event(a: &mut App, e: AppEvent) {
    match e {
        AppEvent::Interaction(_) => {}
        AppEvent::Ephemeral(m) => {
            let list = &mut a.apps.ephemeral;
            match list.iter_mut().find(|x| x.id == m.id) {
                Some(x) => *x = *m,
                None => {
                    if list.len() >= MAX_EPHEMERAL {
                        list.remove(0);
                    }
                    list.push(*m);
                }
            }
        }
        AppEvent::EphemeralRemoved { id, .. } => a.apps.ephemeral.retain(|m| m.id != id),
        AppEvent::Modal {
            interaction_id,
            app_id,
            channel_id,
            modal,
        } => {
            a.apps.modal = Some((interaction_id, app_id, channel_id, modal));
            a.dialog = "apps-modal".into();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
        }
        AppEvent::Autocomplete { request, choices } => {
            if let Some(p) = a.apps.picker.as_mut() {
                if let Some((r, name, list)) = p.auto.as_mut() {
                    if *r == request && !name.is_empty() {
                        *list = choices;
                        let request = p.request.clone();
                        if let Some(app) = a.owner.upgrade() {
                            spawn_local(compose_lazy(app, request, false));
                        }
                    }
                }
            }
        }
        AppEvent::Failed {
            interaction_id,
            message,
            ..
        } => {
            a.apps
                .ephemeral
                .retain(|m| m.interaction_id != interaction_id);
            a.error = message;
        }
        AppEvent::CommandsUpdated { guild_id } => {
            let channels = &a.model.channels;
            a.apps.catalogs.retain(|c| {
                channels
                    .get(&c.channel_id)
                    .is_some_and(|ch| ch.guild_id != Some(guild_id))
            });
        }
        AppEvent::IntegrationsUpdated { guild_id } => {
            if a.apps
                .integrations
                .as_ref()
                .is_some_and(|(g, _)| *g == guild_id)
                || a.apps.webhooks.is_some()
            {
                if let Some(app) = a.owner.upgrade() {
                    let dialog = a.dialog.clone();
                    features::defer(app, Feature::Apps, Call::Other("refresh", dialog));
                }
            }
        }
        AppEvent::AppsUpdated => {
            if a.apps.developer.is_some() {
                if let Some(app) = a.owner.upgrade() {
                    features::defer(
                        app,
                        Feature::Apps,
                        Call::Other("refresh", "developer".into()),
                    );
                }
            }
        }
    }
    a.touch();
}

/// User settings pages (Developer).
pub(super) fn is_user_page(page: &str) -> bool {
    page == "developer" || page.starts_with("developer-")
}
pub(super) fn is_server_page(page: &str) -> bool {
    page == "server-integrations"
        || page.starts_with("integration-")
        || page.starts_with("channel-webhooks:")
        || page.starts_with("webhook-delete:")
}
pub(super) fn owns_page(page: &str) -> bool {
    is_user_page(page)
        || is_server_page(page)
        || matches!(page, "app-install" | "apps-modal")
        || page.starts_with("apps-target:")
}
pub(super) fn title(page: &str) -> Option<&'static str> {
    is_user_page(page).then_some("Developer")
}

/// Dialogs whose data this feature fetches when they open (`run`).
pub(super) fn owns_dialog(name: &str) -> bool {
    owns_page(name)
}
async fn refresh(app: Shared, name: String) {
    let (http, guild, epoch) = {
        let a = app.borrow();
        (a.http(), a.guild, a.epoch)
    };
    let current = |app: &Shared| app.borrow().epoch == epoch;
    if is_user_page(&name) {
        match http.json::<DeveloperState>("/applications/@me").await {
            Ok(d) if current(&app) => {
                let mut a = app.borrow_mut();
                a.apps.developer = Some(d);
                a.touch();
            }
            Ok(_) => {}
            Err(e) => fail(&app, e),
        }
        return;
    }
    if name == "app-install" {
        let Some(link) = app.borrow().apps.install_link else {
            return;
        };
        let r = http
            .json::<InstallPreview>(&format!("/applications/{}/install", link.0))
            .await;
        if current(&app) {
            let mut a = app.borrow_mut();
            a.apps.install = Some(r.map_err(|_| {
                "This app can't be added: the link is wrong, or the app is private or turned off."
                    .to_owned()
            }));
            a.touch();
        }
        return;
    }
    if let Some(ch) = name.strip_prefix("channel-webhooks:").and_then(id) {
        match http
            .json::<Vec<Webhook>>(&format!("/channels/{}/webhooks", ch.0))
            .await
        {
            Ok(list) if current(&app) => {
                let mut a = app.borrow_mut();
                a.apps.webhooks = Some((ch, list));
                a.touch();
            }
            Ok(_) => {}
            Err(e) => fail(&app, e),
        }
        return;
    }
    if name.starts_with("apps-target:") {
        let ch = app.borrow().channel;
        if let Some(ch) = ch {
            catalog(app.clone(), ch, true).await;
        }
        return;
    }
    let Some(g) = guild else { return };
    if name == "server-integrations"
        || name.starts_with("integration-")
        || name.starts_with("webhook-delete:")
    {
        match http
            .json::<GuildIntegrations>(&format!("/guilds/{}/integrations", g.0))
            .await
        {
            Ok(v) if current(&app) => {
                let mut a = app.borrow_mut();
                a.apps.integrations = Some((g, v));
                a.touch();
            }
            Ok(_) => {}
            Err(e) => fail(&app, e),
        }
    }
}

/// The picker's commands for a channel, fetched once and kept until they change.
async fn catalog(app: Shared, ch: Id, force: bool) -> Option<CommandCatalog> {
    if !force {
        if let Some(c) = app
            .borrow()
            .apps
            .catalogs
            .iter()
            .find(|c| c.channel_id == ch)
        {
            return Some(c.clone());
        }
    }
    if !app.borrow_mut().apps.wanted.insert(ch) {
        return None;
    }
    let (http, epoch) = {
        let a = app.borrow();
        (a.http(), a.epoch)
    };
    let r = http
        .json::<CommandCatalog>(&format!("/channels/{}/commands", ch.0))
        .await;
    let mut a = app.borrow_mut();
    a.apps.wanted.remove(&ch);
    if a.epoch != epoch {
        return None;
    }
    let c = r.ok()?;
    a.apps.catalogs.retain(|x| x.channel_id != ch);
    if a.apps.catalogs.len() >= MAX_CATALOGS {
        a.apps.catalogs.remove(0);
    }
    a.apps.catalogs.push(c.clone());
    a.touch();
    Some(c)
}

/// Opening a channel in a server with bots: learn its commands (for the
/// Apps item on messages and members). Servers without bots cost nothing.
pub(super) fn channel_opened(app: Shared, ch: Id) {
    let wanted = {
        let a = app.borrow();
        a.guild.is_some()
            && a.model.members.values().any(|m| m.user.style.bot)
            && !a.apps.catalogs.iter().any(|c| c.channel_id == ch)
    };
    if wanted {
        spawn_local(load_catalog(app, ch));
    }
}
#[lazy]
async fn load_catalog(app: Shared, ch: Id) {
    catalog(app, ch, false).await;
}

// ---------------------------------------------------------------- the picker

/// `complete:/|<text before the cursor>`: the picker loads on the first `/`.
pub(super) fn compose(app: Shared, request: &str) {
    spawn_local(compose_lazy(app, request.to_owned(), true));
}
pub(super) fn close_picker(a: &mut App) {
    a.apps.picker = None;
}

/// Splits a command line into words; `"…"` keeps spaces in a value.
fn words(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
fn fuzzy(hay: &str, needle: &str) -> Option<u32> {
    let hay = hay.to_lowercase();
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return Some(1);
    }
    if hay.starts_with(&needle) {
        return Some(3);
    }
    if hay.contains(&needle) {
        return Some(2);
    }
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c)).then_some(1)
}
fn kind_hint(o: &CommandOption) -> String {
    let t = match o.kind {
        OptionKind::String => "text".to_owned(),
        OptionKind::Integer => match (o.min_value, o.max_value) {
            (Some(a), Some(b)) => format!("{}–{}", a.0, b.0),
            _ => "number".into(),
        },
        OptionKind::Number => "decimal".into(),
        OptionKind::Boolean => "true/false".into(),
        OptionKind::User => "member".into(),
        OptionKind::Channel => "channel".into(),
        OptionKind::Role => "role".into(),
        OptionKind::Mentionable => "member or role".into(),
        OptionKind::Attachment => "file".into(),
        OptionKind::SubCommand | OptionKind::SubCommandGroup => "subcommand".into(),
    };
    format!("{}{}:<{t}>", o.name, if o.required { "*" } else { "" })
}
/// The command's options after its subcommand words, and how many words those took.
fn leaf<'a>(
    cmd: &'a ApplicationCommand,
    rest: &[String],
) -> (Vec<String>, &'a [CommandOption], usize) {
    let mut path = vec![];
    let mut opts: &[CommandOption] = &cmd.options;
    let mut used = 0;
    while opts
        .iter()
        .any(|o| matches!(o.kind, OptionKind::SubCommand | OptionKind::SubCommandGroup))
    {
        let Some(word) = rest.get(used) else { break };
        let Some(sub) = opts.iter().find(|o| &o.name == word) else {
            break;
        };
        path.push(sub.name.clone());
        opts = &sub.options;
        used += 1;
    }
    (path, opts, used)
}

pub(super) fn find_command<'a>(
    c: &'a CommandCatalog,
    name: &str,
) -> Option<(&'a CatalogApp, &'a ApplicationCommand)> {
    c.apps.iter().find_map(|app| {
        app.commands
            .iter()
            .find(|cmd| cmd.kind == CommandKind::ChatInput && cmd.name == name)
            .map(|cmd| (app, cmd))
    })
}

#[lazy]
async fn compose_lazy(app: Shared, request: String, ask: bool) {
    let Some(before) = request.strip_prefix("/|").map(str::to_owned) else {
        return;
    };
    let Some(ch) = app.borrow().channel else {
        return;
    };
    let in_guild = app.borrow().guild.is_some();
    {
        let mut a = app.borrow_mut();
        let keep_auto = a.apps.picker.as_ref().and_then(|p| p.auto.clone());
        let p = a.apps.picker.get_or_insert_with(Picker::default);
        p.request = request.clone();
        p.auto = keep_auto;
    }
    let catalog = if in_guild {
        match catalog(app.clone(), ch, false).await {
            Some(c) => Some(c),
            None => app
                .borrow()
                .apps
                .catalogs
                .iter()
                .find(|c| c.channel_id == ch)
                .cloned(),
        }
    } else {
        None
    };
    let mut a = app.borrow_mut();
    if a.apps.picker.as_ref().is_none_or(|p| p.request != request) {
        return;
    }
    let ended = before.ends_with(char::is_whitespace);
    let all = words(&before);
    let first = all.first().cloned().unwrap_or_default();
    let name = first.trim_start_matches('/').to_owned();
    let mut rows = vec![];
    let mut hint = String::new();
    let mut auto_request = None;
    if all.len() <= 1 && !ended {
        // Choosing the command.
        let mut scored: Vec<(u32, Row)> = vec![];
        for (n, args, about) in BUILTINS {
            if *n == "nick" && !in_guild {
                continue;
            }
            if let Some(s) = fuzzy(n, &name) {
                scored.push((
                    s,
                    Row {
                        group: "Built-in".into(),
                        label: format!("/{n} {args}"),
                        detail: (*about).into(),
                        insert: format!("/{n}"),
                    },
                ));
            }
        }
        if let Some(c) = &catalog {
            for capp in &c.apps {
                for command in capp
                    .commands
                    .iter()
                    .filter(|c| c.kind == CommandKind::ChatInput)
                {
                    if let Some(s) = fuzzy(&command.name, &name) {
                        scored.push((
                            s,
                            Row {
                                group: capp.name.clone(),
                                label: format!("/{}", command.name),
                                detail: command.description.clone(),
                                insert: format!("/{}", command.name),
                            },
                        ));
                    }
                }
            }
        }
        // Best matches first, keeping each group together.
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        let mut groups: Vec<String> = vec![];
        for (_, r) in &scored {
            if !groups.contains(&r.group) {
                groups.push(r.group.clone());
            }
        }
        for g in groups {
            rows.extend(
                scored
                    .iter()
                    .filter(|(_, r)| r.group == g)
                    .map(|(_, r)| r.clone())
                    .take(25),
            );
        }
        rows.truncate(25);
        if rows.is_empty() {
            hint = "No commands match.".into();
        }
    } else if let Some((_, args, about)) = BUILTINS.iter().find(|b| b.0 == name) {
        hint = format!("/{name} {args} — {about}");
    } else if let Some((capp, command)) = catalog.as_ref().and_then(|c| find_command(c, &name)) {
        let rest: Vec<String> = all[1..].to_vec();
        let typing = if ended {
            String::new()
        } else {
            rest.last().cloned().unwrap_or_default()
        };
        let done = if ended {
            rest.len()
        } else {
            rest.len().saturating_sub(1)
        };
        let (path, opts, used) = leaf(command, &rest[..done.min(rest.len())]);
        let wants_sub = opts
            .iter()
            .any(|o| matches!(o.kind, OptionKind::SubCommand | OptionKind::SubCommandGroup));
        let signature: Vec<String> = opts.iter().map(kind_hint).collect();
        hint = format!(
            "/{}{}{} {} — {}",
            command.name,
            if path.is_empty() { "" } else { " " },
            path.join(" "),
            if wants_sub {
                opts.iter()
                    .map(|o| o.name.clone())
                    .collect::<Vec<_>>()
                    .join(" | ")
            } else {
                signature.join(" ")
            },
            capp.name
        );
        if wants_sub {
            for o in opts.iter().filter(|o| fuzzy(&o.name, &typing).is_some()) {
                rows.push(Row {
                    group: capp.name.clone(),
                    label: o.name.clone(),
                    detail: o.description.clone(),
                    insert: o.name.clone(),
                });
            }
        } else {
            let given: Vec<String> = rest[used..done.min(rest.len())]
                .iter()
                .filter_map(|w| w.split_once(':').map(|(n, _)| n.to_owned()))
                .collect();
            match typing.split_once(':') {
                Some((opt, value)) => {
                    if let Some(o) = opts.iter().find(|o| o.name == opt) {
                        hint = format!("{}: {}", kind_hint(o), o.description);
                        let row = |label: String, value: String| Row {
                            group: capp.name.clone(),
                            label,
                            detail: String::new(),
                            insert: format!(
                                "{}:{}",
                                o.name,
                                if value.contains(' ') {
                                    format!("\"{value}\"")
                                } else {
                                    value
                                }
                            ),
                        };
                        if o.autocomplete {
                            if let Some((_, n, choices)) =
                                a.apps.picker.as_ref().and_then(|p| p.auto.as_ref())
                            {
                                if *n == o.name {
                                    rows.extend(
                                        choices
                                            .iter()
                                            .map(|c| row(c.name.clone(), choice_text(&c.value))),
                                    );
                                }
                            }
                            auto_request = Some((
                                o.clone(),
                                value.trim_matches('"').to_owned(),
                                rest[used..done.min(rest.len())].to_vec(),
                                path.clone(),
                                command.id,
                            ));
                        } else if !o.choices.is_empty() {
                            rows.extend(
                                o.choices
                                    .iter()
                                    .filter(|c| fuzzy(&c.name, value).is_some())
                                    .map(|c| row(c.name.clone(), choice_text(&c.value))),
                            );
                        } else {
                            match o.kind {
                                OptionKind::Boolean => rows.extend(
                                    ["true", "false"]
                                        .iter()
                                        .filter(|v| v.starts_with(value))
                                        .map(|v| row((*v).into(), (*v).into())),
                                ),
                                OptionKind::User | OptionKind::Mentionable => {
                                    let q = value.trim_start_matches('@');
                                    for m in a
                                        .model
                                        .members
                                        .values()
                                        .filter(|m| Some(m.guild_id) == a.guild)
                                        .filter(|m| fuzzy(&m.user.username, q).is_some())
                                        .take(8)
                                    {
                                        rows.push(row(
                                            m.user
                                                .display_name
                                                .clone()
                                                .unwrap_or_else(|| m.user.username.clone()),
                                            format!("@{}", m.user.username),
                                        ));
                                    }
                                    if o.kind == OptionKind::Mentionable {
                                        for r in a
                                            .roles
                                            .values()
                                            .filter(|r| {
                                                Some(r.guild_id) == a.guild
                                                    && fuzzy(&r.name, q).is_some()
                                            })
                                            .take(4)
                                        {
                                            rows.push(row(
                                                format!("@{}", r.name),
                                                format!("@{}", r.name),
                                            ));
                                        }
                                    }
                                }
                                OptionKind::Role => {
                                    let q = value.trim_start_matches('@');
                                    for r in a
                                        .roles
                                        .values()
                                        .filter(|r| {
                                            Some(r.guild_id) == a.guild
                                                && fuzzy(&r.name, q).is_some()
                                        })
                                        .take(8)
                                    {
                                        rows.push(row(
                                            format!("@{}", r.name),
                                            format!("@{}", r.name),
                                        ));
                                    }
                                }
                                OptionKind::Channel => {
                                    let q = value.trim_start_matches('#');
                                    for c in a
                                        .model
                                        .channels
                                        .values()
                                        .filter(|c| {
                                            c.guild_id == a.guild
                                                && c.name
                                                    .as_deref()
                                                    .is_some_and(|n| fuzzy(n, q).is_some())
                                        })
                                        .take(8)
                                    {
                                        let n = c.name.clone().unwrap_or_default();
                                        rows.push(row(format!("#{n}"), format!("#{n}")));
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else {
                        hint = format!("{} has no option “{opt}”.", command.name);
                    }
                }
                None => {
                    for o in opts.iter().filter(|o| !given.contains(&o.name)) {
                        let wanted = if typing.is_empty() {
                            o.required
                        } else {
                            fuzzy(&o.name, &typing).is_some()
                        };
                        if wanted {
                            rows.push(Row {
                                group: capp.name.clone(),
                                label: kind_hint(o),
                                detail: o.description.clone(),
                                insert: format!("{}:", o.name),
                            });
                        }
                    }
                }
            }
        }
    } else if !name.is_empty() && catalog.is_none() && in_guild {
        hint = "Loading commands…".into();
    }
    if let Some(p) = a.apps.picker.as_mut() {
        p.rows = rows;
        p.hint = hint;
    }
    // Suggestions from the bot for the option being typed (at most one
    // request per change; answers older than the newest are ignored).
    if let (true, Some((o, value, given, path, command_id))) = (ask, auto_request) {
        a.apps.request = a.apps.request.wrapping_add(1);
        let request = a.apps.request;
        let mut options = parse_options(&a, &o.name, &given).unwrap_or_default();
        let value = match o.kind {
            OptionKind::Integer => value
                .parse()
                .map(OptionValue::Integer)
                .unwrap_or(OptionValue::String(value)),
            OptionKind::Number => value
                .parse()
                .map(|n| OptionValue::Number(Number(n)))
                .unwrap_or(OptionValue::String(value)),
            _ => OptionValue::String(value),
        };
        options.push(InvokedOption {
            name: o.name.clone(),
            value: Some(value),
            options: vec![],
            focused: true,
        });
        let options = nest(path, options);
        if let Some(p) = a.apps.picker.as_mut() {
            let keep = p
                .auto
                .as_ref()
                .filter(|(_, n, _)| *n == o.name)
                .map(|(_, _, c)| c.clone())
                .unwrap_or_default();
            p.auto = Some((request, o.name.clone(), keep));
        }
        cmd(
            &mut a,
            AppCommand::Autocomplete {
                invocation: Invocation {
                    channel_id: ch,
                    command_id,
                    options,
                    target_id: None,
                },
                request,
            },
        );
    }
    a.touch();
}
fn choice_text(v: &OptionValue) -> String {
    match v {
        OptionValue::String(s) => s.clone(),
        OptionValue::Integer(i) => i.to_string(),
        OptionValue::Number(n) => n.0.to_string(),
        OptionValue::Boolean(b) => b.to_string(),
        OptionValue::User(i)
        | OptionValue::Channel(i)
        | OptionValue::Role(i)
        | OptionValue::Mentionable(i)
        | OptionValue::Attachment(i) => i.0.to_string(),
    }
}
/// Wraps leaf options in their subcommand path.
fn nest(path: Vec<String>, leaf: Vec<InvokedOption>) -> Vec<InvokedOption> {
    path.into_iter().rev().fold(leaf, |inner, name| {
        vec![InvokedOption {
            name,
            value: None,
            options: inner,
            focused: false,
        }]
    })
}
/// `name:value` words as typed options (all but `skip`), leniently.
fn parse_options(a: &App, skip: &str, given: &[String]) -> Option<Vec<InvokedOption>> {
    Some(
        given
            .iter()
            .filter_map(|w| w.split_once(':'))
            .filter(|(n, _)| *n != skip)
            .map(|(n, v)| InvokedOption {
                name: n.to_owned(),
                value: Some(OptionValue::String(v.to_owned())),
                options: vec![],
                focused: false,
            })
            .filter(|_| a.ready)
            .collect(),
    )
}

/// One typed value as the option's type (the server checks it again).
fn value_of(a: &App, o: &CommandOption, raw: &str) -> Result<OptionValue, String> {
    let bad = |what: &str| Err(format!("{}: {what}", o.name));
    let member = |s: &str| -> Option<Id> {
        let s = s.trim_start_matches('@');
        if let Some(n) = s
            .strip_prefix("<")
            .and_then(|s| s.strip_suffix('>'))
            .and_then(|s| s.trim_start_matches('@').parse::<u64>().ok())
        {
            return Some(Id(n));
        }
        a.model
            .members
            .values()
            .filter(|m| Some(m.guild_id) == a.guild)
            .find(|m| m.user.username.eq_ignore_ascii_case(s))
            .map(|m| m.user.id)
    };
    let role = |s: &str| -> Option<Id> {
        let s = s.trim_start_matches('@');
        if let Some(n) = s
            .strip_prefix("<&")
            .and_then(|s| s.strip_suffix('>'))
            .and_then(|s| s.parse::<u64>().ok())
        {
            return Some(Id(n));
        }
        a.roles
            .values()
            .find(|r| Some(r.guild_id) == a.guild && r.name.eq_ignore_ascii_case(s))
            .map(|r| r.id)
    };
    let value = match o.kind {
        OptionKind::String => OptionValue::String(raw.to_owned()),
        OptionKind::Integer => match raw.parse() {
            Ok(i) => OptionValue::Integer(i),
            Err(_) => return bad("enter a whole number."),
        },
        OptionKind::Number => match raw.parse::<f64>() {
            Ok(n) if n.is_finite() => OptionValue::Number(Number(n)),
            _ => return bad("enter a number."),
        },
        OptionKind::Boolean => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" => OptionValue::Boolean(true),
            "false" | "no" | "off" => OptionValue::Boolean(false),
            _ => return bad("enter true or false."),
        },
        OptionKind::User => match member(raw) {
            Some(u) => OptionValue::User(u),
            None => return bad("choose someone in this server, like @name."),
        },
        OptionKind::Role => match role(raw) {
            Some(r) => OptionValue::Role(r),
            None => return bad("choose a role, like @name."),
        },
        OptionKind::Mentionable => match member(raw).or_else(|| role(raw)) {
            Some(v) => OptionValue::Mentionable(v),
            None => return bad("choose a member or a role."),
        },
        OptionKind::Channel => {
            let n = raw.trim_start_matches('#');
            match a.model.channels.values().find(|c| {
                c.guild_id == a.guild
                    && c.name.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(n))
            }) {
                Some(c) => OptionValue::Channel(c.id),
                None => return bad("choose a channel, like #general."),
            }
        }
        OptionKind::Attachment => match a.attachments.first() {
            Some(f) => OptionValue::Attachment(f.id),
            None => return bad("attach the file with + first."),
        },
        OptionKind::SubCommand | OptionKind::SubCommandGroup => return bad("choose a subcommand."),
    };
    if !o.choices.is_empty()
        && !o
            .choices
            .iter()
            .any(|c| c.value == value || c.name.eq_ignore_ascii_case(raw))
    {
        return bad("pick one of the suggestions.");
    }
    // A choice typed by its name stands for its value.
    Ok(o.choices
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(raw))
        .map_or(value, |c| c.value.clone()))
}

/// A whole command line as an invocation, or what is wrong with it.
pub(super) fn parse(
    a: &App,
    catalog: &CommandCatalog,
    ch: Id,
    text: &str,
) -> Option<Result<Invocation, String>> {
    let all = words(text);
    let name = all.first()?.trim_start_matches('/').to_owned();
    let (_, command) = find_command(catalog, &name)?;
    Some((|| {
        let rest = &all[1..];
        let (path, opts, used) = leaf(command, rest);
        if opts
            .iter()
            .any(|o| matches!(o.kind, OptionKind::SubCommand | OptionKind::SubCommandGroup))
        {
            return Err(format!(
                "/{} needs one of: {}.",
                command.name,
                opts.iter()
                    .map(|o| o.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let mut options = vec![];
        let mut text_rest = vec![];
        for w in &rest[used..] {
            match w.split_once(':') {
                Some((n, v)) if opts.iter().any(|o| o.name == n) => {
                    let o = opts.iter().find(|o| o.name == n).expect("found");
                    if options.iter().any(|x: &InvokedOption| x.name == n) {
                        return Err(format!("{n} is given twice."));
                    }
                    options.push(InvokedOption {
                        name: n.to_owned(),
                        value: Some(value_of(a, o, v)?),
                        options: vec![],
                        focused: false,
                    });
                }
                _ => text_rest.push(w.clone()),
            }
        }
        // Text after the command fills its first text option, if it has one.
        if !text_rest.is_empty() {
            match opts
                .iter()
                .find(|o| o.kind == OptionKind::String && !options.iter().any(|x| x.name == o.name))
            {
                Some(o) => options.push(InvokedOption {
                    name: o.name.clone(),
                    value: Some(value_of(a, o, &text_rest.join(" "))?),
                    options: vec![],
                    focused: false,
                }),
                None => {
                    return Err(format!(
                        "Write options as name:value, like {}.",
                        opts.first().map_or("name:value".into(), kind_hint)
                    ))
                }
            }
        }
        if let Some(missing) = opts
            .iter()
            .find(|o| o.required && !options.iter().any(|x| x.name == o.name))
        {
            return Err(format!("/{} needs {}.", command.name, kind_hint(missing)));
        }
        Ok(Invocation {
            channel_id: ch,
            command_id: command.id,
            options: nest(path, options),
            target_id: None,
        })
    })())
}

/// The composer sent text starting with `/`: run a built-in or an app's
/// command instead of posting it. False when it is ordinary text.
pub(super) fn slash(app: &Shared, content: &str) -> bool {
    let mut a = app.borrow_mut();
    if std::mem::take(&mut a.apps.bypass) {
        return false;
    }
    if !content.starts_with('/') || content.starts_with("/tts ") || content.starts_with("//") {
        return false;
    }
    a.apps.picker = None;
    drop(a);
    spawn_local(run_slash(app.clone(), content.to_owned()));
    true
}
fn sent(a: &App) {
    if let Some(c) = a.channel {
        media_action(&format!("sent:{}", c.0));
    }
}
#[lazy]
async fn run_slash(app: Shared, content: String) {
    let name = content
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_owned();
    let rest = content.trim_start()[name.len() + 1..].trim().to_owned();
    let builtin = match name.as_str() {
        // `\\` is a backslash and `\_` an underscore in message Markdown.
        "shrug" => Some(format!("{rest} ¯\\\\\\_(ツ)_/¯").trim_start().to_owned()),
        "tableflip" => Some(format!("{rest} (╯°□°)╯︵ ┻━┻").trim_start().to_owned()),
        "unflip" => Some(format!("{rest} ┬─┬ ノ( ゜-゜ノ)").trim_start().to_owned()),
        "me" if !rest.is_empty() => Some(format!("_{rest}_")),
        "spoiler" if !rest.is_empty() => Some(format!("||{rest}||")),
        _ => None,
    };
    if let Some(text) = builtin {
        app.borrow_mut().apps.bypass = true;
        super::operations::action(app, &format!("send:{text}"));
        return;
    }
    if name == "nick" {
        let mut a = app.borrow_mut();
        let (Some(g), Some(me)) = (a.guild, a.model.me.as_ref().map(|m| m.user.id)) else {
            return;
        };
        a.command(Command::Launch(LaunchCommand::SetNickname {
            guild_id: g,
            user_id: me,
            nickname: Some(rest).filter(|s| !s.is_empty()),
        }));
        sent(&a);
        return;
    }
    let (ch, in_guild) = {
        let a = app.borrow();
        (a.channel, a.guild.is_some())
    };
    let Some(ch) = ch else { return };
    let catalog = if in_guild {
        catalog(app.clone(), ch, false).await
    } else {
        None
    };
    let mut a = app.borrow_mut();
    let parsed = catalog.as_ref().and_then(|c| parse(&a, c, ch, &content));
    match parsed {
        Some(Ok(inv)) => {
            cmd(&mut a, AppCommand::Invoke(inv));
            sent(&a);
            a.touch();
        }
        Some(Err(problem)) => {
            a.error = problem;
            a.touch();
        }
        None => {
            // Not a command: send it as written.
            a.apps.bypass = true;
            drop(a);
            super::operations::action(app, &format!("send:{content}"));
        }
    }
}

// ------------------------------------------------------- actions and forms

fn parts(arg: &str) -> Vec<&str> {
    arg.split(',').collect()
}
/// `apps:<what>,<arg>,…` from a button.
async fn act(app: Shared, arg: String) {
    let p = parts(&arg);
    let n = |i: usize| p.get(i).and_then(|v| id(v));
    if p.first() == Some(&"icon") {
        // icon,<app|webhook>,<id>: upload the chosen file as an avatar.
        let target = (
            p.get(1).unwrap_or(&"").to_string(),
            n(2).unwrap_or_default(),
        );
        let file = element(&format!("apps-icon-{}", p.get(2).unwrap_or(&"")))
            .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
            .and_then(|e| e.files())
            .and_then(|f| f.get(0));
        if let Some(file) = file {
            let epoch = {
                let mut a = app.borrow_mut();
                a.apps.icon_for = Some(target);
                a.epoch
            };
            super::operations::upload(app, UploadPurpose::Avatar, file, None, None, epoch).await;
        }
        return;
    }
    let mut a = app.borrow_mut();
    match p.first().copied().unwrap_or_default() {
        "reset-token" => {
            if let Some(app_id) = n(1) {
                cmd(&mut a, AppCommand::ResetToken { app_id });
            }
        }
        "token-hide" => a.apps.token = None,
        "secret-hide" => a.apps.secret = None,
        "rotate" => {
            if let Some(webhook_id) = n(1) {
                cmd(&mut a, AppCommand::RotateWebhook { webhook_id });
            }
        }
        "dismiss" => {
            if let Some(m) = n(1) {
                a.apps.ephemeral.retain(|e| e.id != m);
            }
        }
        "press" => {
            if let (Some(channel_id), Some(message_id)) = (n(1), n(2)) {
                let custom_id = p[3..].join(",");
                cmd(
                    &mut a,
                    AppCommand::Component {
                        channel_id,
                        message_id,
                        custom_id,
                        values: vec![],
                    },
                );
            }
        }
        "target" => {
            // A user or message command from a menu.
            if let (Some(command_id), Some(target_id), Some(channel_id)) = (n(1), n(2), a.channel) {
                cmd(
                    &mut a,
                    AppCommand::Invoke(Invocation {
                        channel_id,
                        command_id,
                        options: vec![],
                        target_id: Some(target_id),
                    }),
                );
                a.dialog.clear();
                a.dialog_generation = a.dialog_generation.wrapping_add(1);
            }
        }
        "rule-remove" => {
            // rule-remove,<target id>,<kind>,<id>
            if let (Some((g, list)), Some(target), Some(rid)) =
                (a.apps.integrations.clone(), n(1), n(3))
            {
                let kind = if p.get(2) == Some(&"channel") {
                    TargetKind::Channel
                } else {
                    TargetKind::Role
                };
                let targets = rules_of(&list, target)
                    .into_iter()
                    .filter(|t| !(t.kind == kind && t.id == rid))
                    .collect();
                cmd(
                    &mut a,
                    AppCommand::SetCommandPermissions {
                        guild_id: g,
                        id: target,
                        targets,
                    },
                );
            }
        }
        "icon-clear" => {
            if let (Some(d), Some(app_id)) = (a.apps.developer.clone(), n(1)) {
                if let Some(info) = d.apps.iter().find(|x| x.id == app_id) {
                    let draft = AppDraft {
                        name: info.name.clone(),
                        description: info.description.clone(),
                        icon: Patch::Clear,
                        public: info.public,
                        intents: info.intents,
                        permissions: info.permissions,
                    };
                    cmd(&mut a, AppCommand::UpdateApp { app_id, draft });
                }
            }
        }
        _ => {}
    }
    a.touch();
}
fn rules_of(list: &GuildIntegrations, target: Id) -> Vec<CommandTarget> {
    list.apps
        .iter()
        .flat_map(|i| i.overrides.iter())
        .find(|o| o.id == target)
        .map(|o| o.targets.clone())
        .unwrap_or_default()
}

/// The avatar upload finished: it is an app's icon or a webhook's avatar
/// (the form that started it is this feature's, so `run` is loaded).
pub(super) fn uploaded(a: &mut App, purpose: UploadPurpose, file: &Attachment) -> bool {
    if purpose != UploadPurpose::Avatar || a.apps.icon_for.is_none() {
        return false;
    }
    if let Some(app) = a.owner.upgrade() {
        features::defer(
            app,
            Feature::Apps,
            Call::Other("uploaded", file.id.0.to_string()),
        );
    }
    true
}
fn icon_uploaded(a: &mut App, file: Id) {
    let Some((kind, target)) = a.apps.icon_for.take() else {
        return;
    };
    match kind.as_str() {
        "app" => {
            if let Some(info) = a
                .apps
                .developer
                .as_ref()
                .and_then(|d| d.apps.iter().find(|x| x.id == target))
                .cloned()
            {
                let draft = AppDraft {
                    name: info.name,
                    description: info.description,
                    icon: Patch::Set(file),
                    public: info.public,
                    intents: info.intents,
                    permissions: info.permissions,
                };
                cmd(
                    a,
                    AppCommand::UpdateApp {
                        app_id: target,
                        draft,
                    },
                );
            }
        }
        "webhook" => cmd(
            a,
            AppCommand::UpdateWebhook {
                webhook_id: target,
                name: None,
                avatar: Patch::Set(file),
                channel_id: None,
            },
        ),
        _ => {}
    }
}

pub(super) fn is_form(form: &str) -> bool {
    form.starts_with("apps-")
}
fn bits(prefix: &str, all: impl Iterator<Item = u64>) -> u64 {
    all.filter(|b| checked(&format!("{prefix}{b}")))
        .fold(0, |acc, b| acc | b)
}
fn permission_bits() -> Permissions {
    Permissions::from_bits_truncate(bits(
        "apps-perm-",
        super::ui::apps::PERMISSIONS.iter().map(|p| p.1),
    ))
}
async fn submit_lazy(app: Shared, form: String) {
    let mut a = app.borrow_mut();
    let sent_form = |form: &str| media_action(&format!("form-sent:{form}"));
    match form.as_str() {
        "apps-new" => {
            let name = read("apps-name");
            cmd(
                &mut a,
                AppCommand::CreateApp(AppDraft {
                    name,
                    ..Default::default()
                }),
            );
            sent_form(&form);
        }
        "apps-edit" => {
            let Some(app_id) = id(&read("apps-id")) else {
                return;
            };
            let draft = AppDraft {
                name: read("apps-name"),
                description: read("apps-description"),
                icon: Patch::Keep,
                public: checked("apps-public"),
                intents: Intents::from_bits_truncate(bits(
                    "apps-intent-",
                    super::ui::apps::INTENTS.iter().map(|i| i.2),
                ) as u32),
                permissions: permission_bits(),
            };
            cmd(&mut a, AppCommand::UpdateApp { app_id, draft });
            sent_form(&form);
        }
        "apps-delete" => {
            if let Some(app_id) = id(&read("apps-id")) {
                cmd(&mut a, AppCommand::DeleteApp { app_id });
                sent_form(&form);
            }
        }
        "apps-install" => {
            let (Some(app_id), Some(guild_id)) = (id(&read("apps-id")), id(&read("apps-guild")))
            else {
                a.error = "Choose a server.".into();
                a.touch();
                return;
            };
            let permissions = permission_bits();
            cmd(
                &mut a,
                AppCommand::Install(InstallRequest {
                    app_id,
                    guild_id,
                    permissions,
                    message_content: checked("apps-content"),
                    confirm_administrator: checked("apps-admin"),
                }),
            );
            sent_form(&form);
        }
        "apps-remove" => {
            if let (Some(app_id), Some(guild_id)) = (id(&read("apps-id")), a.guild) {
                cmd(&mut a, AppCommand::Uninstall { guild_id, app_id });
                sent_form(&form);
            }
        }
        "apps-rule" => {
            let (Some((g, list)), Some(target)) =
                (a.apps.integrations.clone(), id(&read("apps-target")))
            else {
                return;
            };
            let (kind, value) = read("apps-rule-on")
                .split_once(':')
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .unwrap_or_default();
            let Some(rid) = id(&value) else {
                a.error = "Choose a role or a channel.".into();
                a.touch();
                return;
            };
            let kind = if kind == "channel" {
                TargetKind::Channel
            } else {
                TargetKind::Role
            };
            let mut targets: Vec<CommandTarget> = rules_of(&list, target)
                .into_iter()
                .filter(|t| !(t.kind == kind && t.id == rid))
                .collect();
            targets.push(CommandTarget {
                kind,
                id: rid,
                allow: read("apps-rule-allow") == "allow",
            });
            cmd(
                &mut a,
                AppCommand::SetCommandPermissions {
                    guild_id: g,
                    id: target,
                    targets,
                },
            );
            sent_form(&form);
        }
        "apps-webhook-new" => {
            let Some(channel_id) = id(&read("apps-channel")) else {
                return;
            };
            cmd(
                &mut a,
                AppCommand::CreateWebhook {
                    channel_id,
                    name: read("apps-webhook-name"),
                    avatar_id: None,
                },
            );
            sent_form(&form);
        }
        f if f.starts_with("apps-webhook-save:") => {
            let Some(webhook_id) = id(f.trim_start_matches("apps-webhook-save:")) else {
                return;
            };
            let name = read(&format!("apps-webhook-name-{}", webhook_id.0));
            cmd(
                &mut a,
                AppCommand::UpdateWebhook {
                    webhook_id,
                    name: Some(name),
                    avatar: Patch::Keep,
                    channel_id: None,
                },
            );
            sent_form(f);
        }
        "apps-webhook-delete" => {
            let Some(webhook_id) = id(&read("apps-webhook-id")) else {
                return;
            };
            cmd(&mut a, AppCommand::DeleteWebhook { webhook_id });
            sent_form(&form);
        }
        "apps-modal" => {
            let Some((interaction_id, _, _, modal)) = a.apps.modal.clone() else {
                return;
            };
            let values = modal
                .inputs
                .iter()
                .enumerate()
                .map(|(i, input)| ModalValue {
                    custom_id: input.custom_id.clone(),
                    value: read(&format!("apps-input-{i}")),
                })
                .collect();
            cmd(
                &mut a,
                AppCommand::SubmitModal {
                    interaction_id,
                    values,
                },
            );
            a.apps.modal = None;
            a.dialog.clear();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
        }
        f if f.starts_with("apps-select:") => {
            // apps-select:<channel>:<message>:<row>; the select's custom_id is in a hidden field.
            let ids: Vec<&str> = f.split(':').collect();
            if let (Some(channel_id), Some(message_id), Some(row)) = (
                ids.get(1).and_then(|v| id(v)),
                ids.get(2).and_then(|v| id(v)),
                ids.get(3),
            ) {
                let custom_id = read(&format!("apps-select-id-{}-{row}", message_id.0));
                let values: Vec<String> = selected(&format!("apps-select-{}-{row}", message_id.0))
                    .into_iter()
                    .filter(|v| !v.is_empty())
                    .collect();
                cmd(
                    &mut a,
                    AppCommand::Component {
                        channel_id,
                        message_id,
                        custom_id,
                        values,
                    },
                );
            }
        }
        _ => {}
    }
    a.touch();
}

pub(super) fn owns(op: &Command) -> bool {
    // All of them: the generic answer path would close the open page.
    matches!(op, Command::Apps(_))
}
/// Answers that change nothing the person sees: the core drops them, so
/// they load nothing (the subscription after every Ready, autocomplete).
pub(super) fn housekeeping(op: &Command) -> bool {
    matches!(
        op,
        Command::Apps(AppCommand::Subscribe | AppCommand::Autocomplete { .. })
    )
}
fn answered(app: Shared, op: Command, result: AppResult) {
    // Housekeeping answers change nothing the person sees.
    if matches!(
        op,
        Command::Apps(AppCommand::Subscribe | AppCommand::Autocomplete { .. })
    ) {
        return;
    }
    let mut a = app.borrow_mut();
    a.error.clear();
    let Command::Apps(op) = op else { return };
    match (op, result) {
        (
            AppCommand::CreateApp(_) | AppCommand::ResetToken { .. },
            AppResult::Token { app: info, token },
        ) => {
            a.apps.token = Some((info.id, token));
            if let Some(d) = a.apps.developer.as_mut() {
                d.apps.retain(|x| x.id != info.id);
                d.apps.push(*info.clone());
            }
            a.dialog = format!("developer-app:{}", info.id.0);
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
        }
        (AppCommand::UpdateApp { .. }, AppResult::App(info)) => {
            if let Some(d) = a.apps.developer.as_mut() {
                if let Some(x) = d.apps.iter_mut().find(|x| x.id == info.id) {
                    *x = *info;
                }
            }
            a.notice = "Saved.".into();
            a.error = a.notice.clone();
        }
        (AppCommand::DeleteApp { app_id }, _) => {
            if let Some(d) = a.apps.developer.as_mut() {
                d.apps.retain(|x| x.id != app_id);
            }
            a.dialog = "developer".into();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
        }
        (AppCommand::Install(r), _) => {
            a.apps.install = None;
            let name = a
                .model
                .guilds
                .get(&r.guild_id)
                .map(|g| g.guild.name.clone())
                .unwrap_or_default();
            a.notice = format!("Added to {name}.");
            a.error = a.notice.clone();
            a.dialog.clear();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
        }
        (AppCommand::Uninstall { .. }, _) => {
            a.dialog = "server-integrations".into();
            a.dialog_generation = a.dialog_generation.wrapping_add(1);
        }
        (
            AppCommand::CreateWebhook { .. } | AppCommand::RotateWebhook { .. },
            AppResult::WebhookSecret(s),
        ) => {
            a.apps.secret = Some((s.webhook.id, s.url.clone()));
            if let Some((ch, list)) = a.apps.webhooks.as_mut() {
                if *ch == s.webhook.channel_id {
                    list.retain(|w| w.id != s.webhook.id);
                    list.push(s.webhook.clone());
                }
            }
        }
        (AppCommand::UpdateWebhook { .. }, AppResult::Webhook(w)) => {
            if let Some((_, list)) = a.apps.webhooks.as_mut() {
                if let Some(x) = list.iter_mut().find(|x| x.id == w.id) {
                    *x = *w;
                }
            }
        }
        (AppCommand::DeleteWebhook { webhook_id }, _) => {
            if let Some((_, list)) = a.apps.webhooks.as_mut() {
                list.retain(|w| w.id != webhook_id);
            }
            if a.dialog.starts_with("webhook-delete:") {
                let ch = a.apps.webhooks.as_ref().map(|(c, _)| *c);
                a.dialog = ch.map_or("server-integrations".into(), |c| {
                    format!("channel-webhooks:{}", c.0)
                });
                a.dialog_generation = a.dialog_generation.wrapping_add(1);
            }
        }
        _ => {}
    }
    a.touch();
}

/// Everything a person does with apps: pages, forms, actions, the `/` picker,
/// running a command, and the answers to the commands these send (they were
/// sent from here, so this module is already loaded).
#[leptos::wasm_split::wasm_split(
    apps,
    wasm_split_path = ::leptos::wasm_split,
    fallible,
    preload(prefetch)
)]
pub(super) fn run(app: Shared, call: features::Call) -> features::Outcome {
    match call {
        Call::Action(_, arg) => spawn_local(act(app, arg)),
        Call::Submit(form) => spawn_local(submit_lazy(app, form)),
        Call::Opened(name) | Call::Other("refresh", name) => spawn_local(refresh(app, name)),
        Call::Other("uploaded", file) => {
            if let Some(file) = id(&file) {
                icon_uploaded(&mut app.borrow_mut(), file);
            }
        }
        Call::Answer(Some(op), reply) => {
            let result = match reply {
                // A plain acknowledgement from the core's decoder.
                Reply::Core(_) => AppResult::Ack,
                Reply::Raw(raw) => match crate::inbound::feature::app_result(&raw.data) {
                    Ok(r) => r,
                    Err(()) => return Ok(false),
                },
            };
            answered(app, op, result);
        }
        _ => {}
    }
    Ok(true)
}

/// Apps' events: ephemeral answers, forms, autocomplete, catalogs changing.
#[leptos::wasm_split::wasm_split(apps_state, wasm_split_path = ::leptos::wasm_split, fallible)]
pub(super) fn state(app: Shared, call: features::Call) -> features::Outcome {
    if let Call::Event(raw) = call {
        let Ok(e) = crate::inbound::feature::app_event(&raw.data) else {
            return Ok(false);
        };
        event(&mut app.borrow_mut(), e);
    }
    Ok(true)
}
