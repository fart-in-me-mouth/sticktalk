//! Messages from bots and webhooks: checking embeds and components, which
//! mentions notify anyone, posting through the ordinary send path, and the
//! images the server fetches for embeds and webhook avatars.
use super::*;
use std::collections::VecDeque;

fn chars(s: &str) -> usize {
    s.chars().count()
}
fn plain_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && chars(s) <= max && !s.chars().any(|c| c.is_control() && c != '\n')
}
/// An `http(s)` URL without credentials, for links in embeds and buttons.
pub(crate) fn web_url(s: &str) -> bool {
    s.len() <= 2048
        && reqwest::Url::parse(s).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
        })
}

/// Text AutoMod screens: the message, its embeds and its component labels.
pub(crate) fn screen_text(content: &str, embeds: &[EmbedDraft], rows: &[ActionRow]) -> String {
    let mut t = content.to_owned();
    let mut add = |s: &str| {
        if t.len() < 64 * 1024 {
            t.push('\n');
            t.push_str(s);
        }
    };
    for e in embeds {
        for s in [&e.title, &e.description].into_iter().flatten() {
            add(s);
        }
        if let Some(a) = &e.author {
            add(&a.name);
        }
        if let Some(f) = &e.footer {
            add(&f.text);
        }
        for f in &e.fields {
            add(&f.name);
            add(&f.value);
        }
    }
    for r in rows {
        for c in &r.components {
            match c {
                Component::Button(b) => {
                    if let Some(l) = &b.label {
                        add(l);
                    }
                }
                Component::Select(s) => {
                    if let Some(p) = &s.placeholder {
                        add(p);
                    }
                    for o in &s.options {
                        add(&o.label);
                    }
                }
            }
        }
    }
    t
}

fn custom_id_ok(s: &str) -> bool {
    (1..=MAX_CUSTOM_ID_CHARS).contains(&chars(s)) && !s.chars().any(char::is_control)
}
fn emoji_ok(e: &Option<String>) -> bool {
    e.as_ref()
        .is_none_or(|e| (1..=64).contains(&e.len()) && !e.chars().any(char::is_control))
}

pub(crate) fn check_components(rows: &[ActionRow]) -> Result<()> {
    let bad = |m: &str| Err(invalid("components", m.to_owned()));
    if rows.len() > MAX_ACTION_ROWS {
        return bad("Up to 5 rows of components.");
    }
    let mut ids = HashSet::new();
    for row in rows {
        let selects = row
            .components
            .iter()
            .filter(|c| matches!(c, Component::Select(_)))
            .count();
        if row.components.is_empty()
            || (selects > 0 && row.components.len() != 1)
            || row.components.len() > MAX_ROW_BUTTONS
        {
            return bad("A row holds 1-5 buttons, or one select menu.");
        }
        for c in &row.components {
            match c {
                Component::Button(b) => {
                    if b.label.is_none() && b.emoji.is_none()
                        || b.label
                            .as_ref()
                            .is_some_and(|l| !plain_ok(l, MAX_BUTTON_LABEL_CHARS))
                        || !emoji_ok(&b.emoji)
                    {
                        return bad("A button needs a label (up to 80 characters) or an emoji.");
                    }
                    if b.style == ButtonStyle::Link {
                        if b.custom_id.is_some() || !b.url.as_deref().is_some_and(web_url) {
                            return bad("A link button has an http(s) URL and no custom_id.");
                        }
                    } else {
                        let Some(cid) = &b.custom_id else {
                            return bad("A button needs a custom_id.");
                        };
                        if b.url.is_some() || !custom_id_ok(cid) || !ids.insert(cid.clone()) {
                            return bad("custom_id: 1-100 characters, unique in the message.");
                        }
                    }
                }
                Component::Select(s) => {
                    if !custom_id_ok(&s.custom_id) || !ids.insert(s.custom_id.clone()) {
                        return bad("custom_id: 1-100 characters, unique in the message.");
                    }
                    if s.placeholder.as_ref().is_some_and(|p| !plain_ok(p, 150)) {
                        return bad("A placeholder is up to 150 characters.");
                    }
                    let options = s.options.len();
                    let max = if s.kind == SelectKind::String {
                        options
                    } else {
                        MAX_SELECT_OPTIONS
                    };
                    if s.kind == SelectKind::String && !(1..=MAX_SELECT_OPTIONS).contains(&options)
                        || s.kind != SelectKind::String && options != 0
                        || s.min_values > s.max_values
                        || s.max_values == 0
                        || usize::from(s.max_values) > max
                    {
                        return bad("A select has 1-25 options (text selects), and 1 ≤ min ≤ max ≤ options.");
                    }
                    let mut values = HashSet::new();
                    for o in &s.options {
                        if !plain_ok(&o.label, 100)
                            || !(1..=100).contains(&chars(&o.value))
                            || !values.insert(o.value.clone())
                            || o.description.as_ref().is_some_and(|d| !plain_ok(d, 100))
                            || !emoji_ok(&o.emoji)
                        {
                            return bad(
                                "Select options: labels and unique values of up to 100 characters.",
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Where a fetched image goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    Avatar,
    Image(usize),
    Thumbnail(usize),
}
impl Slot {
    fn number(self) -> i64 {
        match self {
            Slot::Avatar => 0,
            Slot::Image(i) => 1 + 2 * i as i64,
            Slot::Thumbnail(i) => 2 + 2 * i as i64,
        }
    }
}
pub(crate) struct ImageJob {
    pub message: Id,
    pub channel: Id,
    pub slot: Slot,
    pub url: String,
}
/// Fetches waiting for the timer task. Bounded: extra requests are dropped.
#[derive(Default)]
pub(crate) struct ImageQueue(VecDeque<ImageJob>);
impl ImageQueue {
    pub(crate) fn take(&mut self) -> Vec<ImageJob> {
        self.0.drain(..).collect()
    }
    fn push(&mut self, job: ImageJob) -> bool {
        if self.0.len() >= 256 {
            return false;
        }
        self.0.push_back(job);
        true
    }
}

fn timestamp(s: &str) -> Result<UnixMillis> {
    let t: jiff::Timestamp = s.parse().map_err(|_| {
        invalid(
            "embeds",
            "timestamp is an RFC 3339 time, e.g. 2026-11-23T17:00:00Z.",
        )
    })?;
    u64::try_from(t.as_millisecond()).map_err(|_| invalid("embeds", "timestamp is out of range."))
}

/// Embeds as viewers get them (images come later), and the images to fetch.
/// Images to fetch for a message: where each goes, and its URL.
pub(crate) type Fetches = Vec<(Slot, String)>;

pub(crate) fn check_embeds(drafts: &[EmbedDraft]) -> Result<(Vec<RichEmbed>, Fetches)> {
    let bad = |m: &str| Err(invalid("embeds", m.to_owned()));
    if drafts.len() > MAX_EMBEDS {
        return bad("Up to 10 embeds.");
    }
    let mut total = 0;
    let mut out = vec![];
    let mut images = vec![];
    for (i, d) in drafts.iter().enumerate() {
        let text = |s: &Option<String>, max: usize| -> std::result::Result<Option<String>, ()> {
            match s.as_deref().map(str::trim) {
                None | Some("") => Ok(None),
                Some(s) if chars(s) <= max && !s.chars().any(|c| c.is_control() && c != '\n') => {
                    Ok(Some(s.to_owned()))
                }
                _ => Err(()),
            }
        };
        let (Ok(title), Ok(description)) = (
            text(&d.title, MAX_EMBED_TITLE_CHARS),
            text(&d.description, MAX_EMBED_DESCRIPTION_CHARS),
        ) else {
            return bad("Embed title up to 256 and description up to 4096 characters.");
        };
        if d.url.as_deref().is_some_and(|u| !web_url(u))
            || d.author
                .as_ref()
                .is_some_and(|a| a.url.as_deref().is_some_and(|u| !web_url(u)))
        {
            return bad("Embed links are http(s) URLs.");
        }
        if d.color.is_some_and(|c| c > 0xFF_FFFF) {
            return bad("color is 0xRRGGBB.");
        }
        if d.fields.len() > MAX_EMBED_FIELDS {
            return bad("Up to 25 fields per embed.");
        }
        for f in &d.fields {
            if !plain_ok(&f.name, MAX_EMBED_FIELD_NAME_CHARS)
                || !plain_ok(&f.value, MAX_EMBED_FIELD_VALUE_CHARS)
            {
                return bad("Field names up to 256 and values up to 1024 characters, not empty.");
            }
            total += chars(&f.name) + chars(&f.value);
        }
        if d.author
            .as_ref()
            .is_some_and(|a| !plain_ok(&a.name, MAX_EMBED_AUTHOR_CHARS))
            || d.footer
                .as_ref()
                .is_some_and(|f| !plain_ok(&f.text, MAX_EMBED_FOOTER_CHARS))
        {
            return bad("Author up to 256 and footer up to 2048 characters.");
        }
        total += title.as_deref().map_or(0, chars)
            + description.as_deref().map_or(0, chars)
            + d.author.as_ref().map_or(0, |a| chars(&a.name))
            + d.footer.as_ref().map_or(0, |f| chars(&f.text));
        for (slot, u) in [
            (Slot::Image(i), &d.image),
            (Slot::Thumbnail(i), &d.thumbnail),
        ] {
            if let Some(u) = u {
                if !web_url(&u.url) {
                    return bad("Embed images are http(s) URLs.");
                }
                images.push((slot, u.url.clone()));
            }
        }
        let embed = RichEmbed {
            title,
            description,
            url: d.url.clone(),
            color: d.color,
            fields: d.fields.clone(),
            author: d.author.clone(),
            footer: d.footer.clone(),
            image: None,
            thumbnail: None,
            timestamp: d.timestamp.as_deref().map(timestamp).transpose()?,
        };
        if embed == RichEmbed::default() && d.image.is_none() && d.thumbnail.is_none() {
            return bad("An embed needs something to show.");
        }
        out.push(embed);
    }
    if total > MAX_EMBED_TOTAL_CHARS {
        return bad("All embeds together hold up to 6000 characters.");
    }
    Ok((out, images))
}

/// Which mentions in `content` may notify: returns the text with the others
/// made inert, and the people to notify.
pub(crate) async fn mentions(
    c: &mut SqliteConnection,
    author: Id,
    channel: &Channel,
    content: &str,
    allowed: Option<&AllowedMentions>,
) -> Result<(String, Vec<Id>)> {
    let everything = AllowedMentions {
        parse: vec![
            MentionParse::Users,
            MentionParse::Roles,
            MentionParse::Everyone,
        ],
        ..Default::default()
    };
    let a = allowed.unwrap_or(&everything);
    let users_all = a.parse.contains(&MentionParse::Users);
    let roles_all = a.parse.contains(&MentionParse::Roles);
    let everyone = a.parse.contains(&MentionParse::Everyone);
    let names: HashMap<Id, String> = match channel.summary.guild_id {
        Some(g) => crate::permissions::roles(c, g)
            .await?
            .into_iter()
            .map(|r| (r.id, r.name))
            .collect(),
        None => HashMap::new(),
    };
    let mut text = String::with_capacity(content.len());
    let mut users = vec![];
    let mut rest = content;
    while let Some(at) = rest.find('@') {
        let (before, after) = rest.split_at(at);
        let token = before.ends_with('<');
        text.push_str(before);
        let tail = &after[1..];
        if token {
            let role = tail.starts_with('&');
            let digits: String = tail
                .trim_start_matches('&')
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            let close = tail.trim_start_matches('&')[digits.len()..].starts_with('>');
            let value = digits
                .parse::<u64>()
                .ok()
                .filter(|v| *v > 0 && *v <= i64::MAX as u64)
                .map(Id);
            if let (Some(v), true) = (value, close) {
                let len = 1 + usize::from(role) + digits.len() + 1;
                if role {
                    if roles_all || a.roles.contains(&v) {
                        text.push_str(&after[..len]);
                    } else {
                        // `<` was written already; replace the token with a plain name.
                        text.pop();
                        text.push_str("@\u{200B}");
                        let name = names.get(&v).map_or("role", String::as_str);
                        text.push_str(&name.trim_start_matches('@').replace('@', "@\u{200B}"));
                    }
                } else {
                    text.push_str(&after[..len]);
                    if (users_all || a.users.contains(&v))
                        && v != author
                        && !users.contains(&v)
                        && users.len() < MAX_MESSAGE_MENTIONS
                        && access(c, v, channel.summary.id, Permissions::VIEW_CHANNEL)
                            .await
                            .is_ok()
                    {
                        users.push(v);
                    }
                }
                rest = &after[len..];
                continue;
            }
            text.push('@');
            rest = tail;
            continue;
        }
        let word = tail.starts_with("everyone") || tail.starts_with("here");
        let keep = if word { everyone } else { roles_all };
        text.push('@');
        if !keep {
            text.push('\u{200B}');
        }
        rest = tail;
    }
    text.push_str(rest);
    Ok((text, users))
}

/// Posts through the ordinary send path (permissions, slowmode, mentions,
/// read states, threads, fan-out), then adds the app's part.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn post(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    author: Id,
    draft: MessageDraft,
    app: AppMessage,
    images: Vec<(Slot, String)>,
    out: &mut Deliveries,
) -> Result<Message> {
    let empty_app = app.embeds.is_empty() && app.components.is_empty();
    require(
        !draft.content.trim().is_empty() || !draft.attachment_ids.is_empty() || !empty_app,
        ErrorCode::InvalidRequest,
    )?;
    let channel_id = draft.channel_id;
    let result = rich_scope(Box::pin(crate::commands::mutate(
        c,
        core,
        key,
        author,
        Command::SendMessage(draft),
        out,
    )))
    .await?;
    let CommandResult::Message(mut m) = result else {
        return Err(Error::new(ErrorCode::Internal));
    };
    attach(c, core, &mut m, app, images, channel_id, out).await?;
    Ok(m)
}

/// Stores `app` with `m` and adds it to the events already queued for it.
pub(crate) async fn attach(
    c: &mut SqliteConnection,
    core: &mut Core,
    m: &mut Message,
    app: AppMessage,
    images: Vec<(Slot, String)>,
    channel_id: Id,
    out: &mut Deliveries,
) -> Result<()> {
    store(c, m.id, &app).await?;
    for (_, e) in out.iter_mut() {
        match e {
            Event::MessageCreated { message, .. } | Event::MessageUpdated(message)
                if message.id == m.id =>
            {
                message.extra.app = Some(Box::new(app.clone()));
            }
            _ => {}
        }
    }
    m.extra.app = Some(Box::new(app));
    queue(core, m.author_id, m.id, channel_id, images);
    Ok(())
}

pub(crate) async fn store(c: &mut SqliteConnection, m: Id, app: &AppMessage) -> Result<()> {
    let data = serde_json::to_string(app).map_err(|_| Error::new(ErrorCode::Internal))?;
    sqlx::query("INSERT INTO app_messages(message_id,app_id,webhook_id,data) VALUES(?,?,?,?) ON CONFLICT(message_id) DO UPDATE SET data=excluded.data")
        .bind(id(m)?)
        .bind(opt(app.app_id)?)
        .bind(opt(app.webhook_id)?)
        .bind(data)
        .execute(&mut *c)
        .await?;
    Ok(())
}

fn queue(core: &mut Core, author: Id, m: Id, ch: Id, images: Vec<(Slot, String)>) {
    let mut any = false;
    for (slot, url) in images.into_iter().take(MAX_FETCHED_IMAGES) {
        // Thirty images a minute per author, bursting to twelve.
        if core
            .rate(format!("app-images:{}", author.0), 30., 12.)
            .is_err()
        {
            break;
        }
        any |= core.apps.images.push(ImageJob {
            message: m,
            channel: ch,
            slot,
            url,
        });
    }
    if any {
        core.apps.wake.notify_one();
    }
}

/// A bot's message: checks the draft, then posts it.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_draft(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    bot: Id,
    app: Id,
    channel_id: Id,
    draft: AppMessageDraft,
    interaction: Option<InteractionMeta>,
    out: &mut Deliveries,
) -> Result<Message> {
    require(
        !draft.ephemeral || interaction.is_some(),
        ErrorCode::InvalidRequest,
    )?;
    let channel = channel(c, channel_id).await?;
    require(channel.summary.guild_id.is_some(), ErrorCode::Forbidden)?;
    check_components(&draft.components)?;
    let (embeds, images) = check_embeds(&draft.embeds)?;
    let (content, mention_ids) = mentions(
        c,
        bot,
        &channel,
        &draft.content,
        draft.allowed_mentions.as_ref(),
    )
    .await?;
    post(
        c,
        core,
        key,
        bot,
        MessageDraft {
            channel_id,
            content,
            reply_to: draft.reply_to,
            attachment_ids: vec![],
            mention_ids,
        },
        AppMessage {
            app_id: Some(app),
            embeds,
            components: draft.components,
            interaction,
            ..Default::default()
        },
        images,
        out,
    )
    .await
}

/// Each viewer's copy of an edited message.
pub(crate) async fn updated(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    ch: Id,
    m: Id,
) -> Result<()> {
    for v in audience(c, ch).await? {
        if let Ok(msg) = message(c, v, ch, m, false).await {
            out.push((vec![v], Event::MessageUpdated(msg)));
        }
    }
    Ok(())
}

/// Replaces the text and the app's part of one of the author's messages.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn replace(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    author: Id,
    ch: Id,
    m: Id,
    content: String,
    mention_ids: Vec<Id>,
    app: AppMessage,
    images: Vec<(Slot, String)>,
    out: &mut Deliveries,
) -> Result<Message> {
    let old = message(c, author, ch, m, false).await?;
    require(old.author_id == author, ErrorCode::Forbidden)?;
    require(
        !content.trim().is_empty()
            || !old.attachments.is_empty()
            || !app.embeds.is_empty()
            || !app.components.is_empty(),
        ErrorCode::InvalidRequest,
    )?;
    // New embeds bring new images; a webhook's avatar stays.
    sqlx::query("DELETE FROM app_images WHERE message_id=? AND slot>0")
        .bind(id(m)?)
        .execute(&mut *c)
        .await?;
    store(c, m, &app).await?;
    let before = out.len();
    if old.content != content || old.mention_ids != mention_ids {
        rich_scope(Box::pin(crate::commands::mutate(
            c,
            core,
            key,
            author,
            Command::EditMessage(MessageEdit {
                channel_id: ch,
                message_id: m,
                content,
                mention_ids,
            }),
            out,
        )))
        .await?;
    }
    if out.len() == before {
        updated(c, out, ch, m).await?;
    }
    queue(core, author, m, ch, images);
    message(c, author, ch, m, false).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn edit_draft(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    bot: Id,
    app: Id,
    ch: Id,
    m: Id,
    draft: AppMessageDraft,
    out: &mut Deliveries,
) -> Result<Message> {
    let channel = channel(c, ch).await?;
    check_components(&draft.components)?;
    let (embeds, images) = check_embeds(&draft.embeds)?;
    let (content, mention_ids) = mentions(
        c,
        bot,
        &channel,
        &draft.content,
        draft.allowed_mentions.as_ref(),
    )
    .await?;
    let old = message(c, bot, ch, m, false).await?;
    let interaction = old.extra.app.and_then(|a| a.interaction);
    replace(
        c,
        core,
        key,
        bot,
        ch,
        m,
        content,
        mention_ids,
        AppMessage {
            app_id: Some(app),
            embeds,
            components: draft.components,
            interaction,
            ..Default::default()
        },
        images,
        out,
    )
    .await
}

/// Re-encodes a fetched image to fit `max` pixels.
fn fit(bytes: &[u8], max: u32) -> Option<(Vec<u8>, &'static str, u16, u16)> {
    use image::ImageEncoder as _;
    let img = image::load_from_memory(bytes).ok()?;
    let img = img.thumbnail(max, max);
    let mut out = vec![];
    let kind = if img.color().has_alpha() {
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(
                img.to_rgba8().as_raw(),
                img.width(),
                img.height(),
                image::ExtendedColorType::Rgba8,
            )
            .ok()?;
        "image/png"
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82)
            .encode_image(&img.to_rgb8())
            .ok()?;
        "image/jpeg"
    };
    Some((out, kind, img.width() as u16, img.height() as u16))
}

/// The timer task: fetch, store, and tell everyone who can see the message.
pub(crate) async fn fetch_images(app: &App, jobs: Vec<ImageJob>) {
    let mut done = vec![];
    for job in jobs {
        if app.0.shutdown.is_cancelled() {
            return;
        }
        let Some((bytes, kind, w, h)) = app.0.content.fetch_image(&job.url).await else {
            continue;
        };
        let fitted = match job.slot {
            Slot::Avatar => tokio::task::spawn_blocking(move || fit(&bytes, 128))
                .await
                .ok()
                .flatten(),
            Slot::Thumbnail(_) => tokio::task::spawn_blocking(move || fit(&bytes, 160))
                .await
                .ok()
                .flatten(),
            Slot::Image(_) => Some((bytes, kind, w, h)),
        };
        if let Some(image) = fitted {
            done.push((job, image));
        }
    }
    if done.is_empty() {
        return;
    }
    let mut core = app.0.core.lock().await;
    let Ok(mut c) = app.0.pool.acquire().await else {
        return;
    };
    let mut out: Deliveries = vec![];
    let mut touched: Vec<(Id, Id)> = vec![];
    for (job, (bytes, kind, w, h)) in done {
        let data: Option<String> = sqlx::query_scalar("SELECT a.data FROM app_messages a JOIN messages m ON m.id=a.message_id WHERE a.message_id=? AND m.channel_id=? AND m.deleted_at IS NULL")
            .bind(job.message.0 as i64)
            .bind(job.channel.0 as i64)
            .fetch_optional(&mut *c)
            .await
            .ok()
            .flatten();
        let Some(mut app_msg) = data.and_then(|d| serde_json::from_str::<AppMessage>(&d).ok())
        else {
            continue;
        };
        let path = format!(
            "/api/v1/channels/{}/messages/{}/app-images/{}",
            job.channel.0,
            job.message.0,
            job.slot.number()
        );
        let image = EmbedImage {
            url: path.clone(),
            width: w,
            height: h,
        };
        match job.slot {
            Slot::Avatar => app_msg.avatar = Some(path),
            Slot::Image(i) => match app_msg.embeds.get_mut(i) {
                Some(e) => e.image = Some(image),
                None => continue,
            },
            Slot::Thumbnail(i) => match app_msg.embeds.get_mut(i) {
                Some(e) => e.thumbnail = Some(image),
                None => continue,
            },
        }
        let saved = async {
            sqlx::query("INSERT INTO app_images(message_id,slot,content_type,width,height,bytes) VALUES(?,?,?,?,?,?) ON CONFLICT(message_id,slot) DO UPDATE SET content_type=excluded.content_type,width=excluded.width,height=excluded.height,bytes=excluded.bytes")
                .bind(job.message.0 as i64)
                .bind(job.slot.number())
                .bind(kind)
                .bind(i64::from(w))
                .bind(i64::from(h))
                .bind(bytes)
                .execute(&mut *c)
                .await?;
            store(&mut c, job.message, &app_msg).await
        }
        .await;
        if saved.is_ok() && !touched.contains(&(job.channel, job.message)) {
            touched.push((job.channel, job.message));
        }
    }
    for (ch, m) in touched {
        let _ = updated(&mut c, &mut out, ch, m).await;
    }
    for (users, e) in out {
        let _ = core.emit(&users, e);
    }
}

/// `GET /api/v1/channels/{c}/messages/{m}/app-images/{slot}`.
pub(crate) async fn image(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
    slot: i64,
) -> Result<(String, Vec<u8>)> {
    text(c, u, ch, Permissions::READ_HISTORY).await?;
    let r = sqlx::query("SELECT i.content_type,i.bytes FROM app_images i JOIN messages m ON m.id=i.message_id WHERE i.message_id=? AND m.channel_id=? AND m.deleted_at IS NULL AND i.slot=?")
        .bind(id(m)?)
        .bind(id(ch)?)
        .bind(slot)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
    Ok((r.get("content_type"), r.get("bytes")))
}

#[cfg(test)]
mod unit {
    use super::*;
    #[test]
    fn embeds_and_components_are_bounded() {
        let ok = EmbedDraft {
            title: Some("Build passed".into()),
            image: Some(EmbedUrl {
                url: "https://example.com/a.png".into(),
            }),
            timestamp: Some("2026-11-23T17:00:00Z".into()),
            ..Default::default()
        };
        let (e, images) = check_embeds(std::slice::from_ref(&ok)).unwrap();
        assert_eq!(
            images,
            vec![(Slot::Image(0), "https://example.com/a.png".into())]
        );
        assert!(e[0].timestamp.is_some());
        for bad in [
            EmbedDraft::default(),
            EmbedDraft {
                url: Some("javascript:alert(1)".into()),
                ..ok.clone()
            },
            EmbedDraft {
                image: Some(EmbedUrl {
                    url: "file:///etc/passwd".into(),
                }),
                ..ok.clone()
            },
            EmbedDraft {
                color: Some(0x1_000_000),
                ..ok.clone()
            },
            EmbedDraft {
                description: Some("x".repeat(4097)),
                ..ok.clone()
            },
        ] {
            assert!(check_embeds(&[bad]).is_err());
        }
        assert!(check_embeds(&vec![ok.clone(); 11]).is_err());
        let button = |id: &str| {
            Component::Button(Button {
                style: ButtonStyle::Primary,
                label: Some("Go".into()),
                emoji: None,
                custom_id: Some(id.into()),
                url: None,
                disabled: false,
            })
        };
        assert!(check_components(&[ActionRow {
            components: vec![button("a"), button("b")]
        }])
        .is_ok());
        assert!(check_components(&[ActionRow {
            components: vec![button("a"), button("a")]
        }])
        .is_err());
        assert!(check_components(&[ActionRow {
            components: (0..6).map(|i| button(&i.to_string())).collect()
        }])
        .is_err());
        let link = Component::Button(Button {
            style: ButtonStyle::Link,
            label: Some("Docs".into()),
            emoji: None,
            custom_id: None,
            url: Some("javascript:alert(1)".into()),
            disabled: false,
        });
        assert!(check_components(&[ActionRow {
            components: vec![link]
        }])
        .is_err());
        let text = screen_text(
            "hi",
            &[ok],
            &[ActionRow {
                components: vec![button("x")],
            }],
        );
        assert!(text.contains("Build passed") && text.contains("Go"));
    }
}
