//! AutoMod: per-server rules checked on the server before a message is stored.
//! Rules are compiled once and cached per server until they change.
use super::*;

/// A keyword or allow-list entry, compiled: lowercase words, and whether the
/// first word may be preceded (`*word`) and the last followed (`word*`) by
/// more letters.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pattern {
    words: Vec<String>,
    open_start: bool,
    open_end: bool,
}
impl Pattern {
    fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim().to_lowercase();
        let open_start = s.starts_with('*');
        let open_end = s.len() > 1 && s.ends_with('*');
        let core = s.trim_matches('*');
        let words: Vec<String> = words(core).map(str::to_owned).collect();
        (!words.is_empty()).then_some(Self {
            words,
            open_start,
            open_end,
        })
    }
    /// Whether this pattern matches the words starting at `at`.
    fn matches_at(&self, text: &[&str], at: usize) -> bool {
        let n = self.words.len();
        if at + n > text.len() {
            return false;
        }
        self.words.iter().enumerate().all(|(i, w)| {
            let t = text[at + i];
            let first = i == 0;
            let last = i + 1 == n;
            match (first && self.open_start, last && self.open_end) {
                (true, true) => t.contains(w.as_str()),
                (true, false) => t.ends_with(w.as_str()),
                (false, true) => t.starts_with(w.as_str()),
                (false, false) => t == w,
            }
        })
    }
}
/// Words are runs of letters and digits; everything else separates them.
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
}
/// A keyword is 1 to 60 characters with at least one letter or digit, and `*`
/// only at its start or end.
pub(crate) fn valid_keyword(raw: &str) -> bool {
    let s = raw.trim();
    let inner = s.trim_matches('*');
    !s.is_empty()
        && s.chars().count() <= MAX_KEYWORD_CHARS
        && !s.chars().any(char::is_control)
        && !inner.contains('*')
        && inner.chars().any(char::is_alphanumeric)
}
/// The first keyword match in `content` that no allow-list entry covers.
pub(crate) fn keyword_match(
    keywords: &[Pattern],
    allow: &[Pattern],
    content: &str,
) -> Option<String> {
    let lower = content.to_lowercase();
    let text: Vec<&str> = words(&lower).collect();
    for at in 0..text.len() {
        for k in keywords {
            if k.matches_at(&text, at) {
                let n = k.words.len();
                let allowed = allow
                    .iter()
                    .any(|a| a.words.len() == n && a.matches_at(&text, at));
                if !allowed {
                    return Some(text[at..at + n].join(" "));
                }
            }
        }
    }
    None
}
/// Unique user and role mentions, from the explicit list and the markup.
pub(crate) fn mention_count(content: &str, explicit: &[Id]) -> usize {
    let mut users: HashSet<u64> = explicit.iter().map(|v| v.0).collect();
    let mut roles: HashSet<u64> = HashSet::new();
    let mut rest = content;
    while let Some(i) = rest.find("<@") {
        rest = &rest[i + 2..];
        let role = rest.starts_with('&');
        let digits = rest.trim_start_matches('&');
        let n = digits.bytes().take_while(u8::is_ascii_digit).count();
        if n > 0 && digits[n..].starts_with('>') {
            if let Ok(v) = digits[..n].parse::<u64>() {
                if role {
                    roles.insert(v);
                } else {
                    users.insert(v);
                }
            }
        }
    }
    users.len() + roles.len()
}
pub(crate) fn link_count(content: &str) -> usize {
    let lower = content.to_lowercase();
    lower.matches("http://").count() + lower.matches("https://").count() + {
        // Bare www. links not already counted as part of a URL.
        lower
            .match_indices("www.")
            .filter(|(i, _)| !lower[..*i].ends_with("://"))
            .count()
    }
}
/// A stable fingerprint of a message for the repeated-message check.
pub(crate) fn fingerprint(content: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    content.trim().to_lowercase().hash(&mut h);
    h.finish()
}

pub(crate) struct Compiled {
    pub rule: AutoModRule,
    keywords: Vec<Pattern>,
    allow: Vec<Pattern>,
}
impl Compiled {
    fn new(rule: AutoModRule) -> Self {
        let (keywords, allow) = match &rule.trigger {
            AutoModTrigger::Keyword { keywords, allow } => (
                keywords.iter().filter_map(|k| Pattern::parse(k)).collect(),
                allow.iter().filter_map(|k| Pattern::parse(k)).collect(),
            ),
            _ => (vec![], vec![]),
        };
        Self {
            rule,
            keywords,
            allow,
        }
    }
}
/// Why a rule fired, for the author and the alert.
pub(crate) struct Hit {
    pub rule: AutoModRule,
    pub why: String,
}
pub(crate) struct Draft<'a> {
    pub content: &'a str,
    pub mentions: &'a [Id],
    /// Previous identical messages by this author in this server, within 30 seconds.
    pub repeats: usize,
}
pub(crate) fn check(rules: &[Compiled], roles: &[Id], lineage: &[Id], d: &Draft) -> Vec<Hit> {
    let mut hits = vec![];
    for c in rules.iter().filter(|c| c.rule.enabled) {
        let r = &c.rule;
        if r.exempt_roles.iter().any(|x| roles.contains(x))
            || r.exempt_channels.iter().any(|x| lineage.contains(x))
        {
            continue;
        }
        let why = match &r.trigger {
            AutoModTrigger::Keyword { .. } => keyword_match(&c.keywords, &c.allow, d.content)
                .map(|w| format!("it contains “{w}”")),
            AutoModTrigger::MentionSpam { limit } => {
                let n = mention_count(d.content, d.mentions);
                (n > usize::from(*limit)).then(|| format!("it mentions {n} people or roles"))
            }
            AutoModTrigger::Spam { duplicates, links } => {
                let n = link_count(d.content);
                if *links > 0 && n > usize::from(*links) {
                    Some(format!("it has {n} links"))
                } else if *duplicates > 0 && d.repeats + 1 >= usize::from(*duplicates) {
                    Some("it repeats the same message".into())
                } else {
                    None
                }
            }
        };
        if let Some(why) = why {
            hits.push(Hit {
                rule: r.clone(),
                why,
            });
        }
    }
    hits
}

/// The server's enabled and disabled rules, compiled (cached).
pub(crate) async fn rules(
    rt: &mut Runtime,
    c: &mut SqliteConnection,
    g: Id,
) -> Result<std::sync::Arc<Vec<Compiled>>> {
    if let Some(v) = rt.rules.get(&g) {
        return Ok(v.clone());
    }
    let list = load(c, g).await?;
    let compiled = std::sync::Arc::new(list.into_iter().map(Compiled::new).collect::<Vec<_>>());
    if rt.rules.len() >= 512 {
        rt.rules.clear();
    }
    rt.rules.insert(g, compiled.clone());
    Ok(compiled)
}
pub(crate) async fn load(c: &mut SqliteConnection, g: Id) -> Result<Vec<AutoModRule>> {
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT rule FROM automod_rules WHERE guild_id=? ORDER BY id")
            .bind(id(g)?)
            .fetch_all(&mut *c)
            .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| serde_json::from_str(&r).ok())
        .collect())
}

/// Validate and normalize a rule for server `g`. Bounds every list.
pub(crate) async fn validate(
    c: &mut SqliteConnection,
    g: Id,
    mut r: AutoModRule,
) -> Result<AutoModRule> {
    require(r.guild_id == g, ErrorCode::InvalidRequest)?;
    r.name = name(&r.name, 100)?;
    match &mut r.trigger {
        AutoModTrigger::Keyword { keywords, allow } => {
            for list in [&mut *keywords, &mut *allow] {
                for k in list.iter_mut() {
                    *k = k.trim().to_lowercase();
                }
                list.sort();
                list.dedup();
            }
            require(
                !keywords.is_empty()
                    && keywords.len() <= MAX_AUTOMOD_KEYWORDS
                    && allow.len() <= MAX_AUTOMOD_ALLOW
                    && keywords
                        .iter()
                        .chain(allow.iter())
                        .all(|k| valid_keyword(k)),
                ErrorCode::InvalidRequest,
            )?;
        }
        AutoModTrigger::MentionSpam { limit } => {
            require((1..=50).contains(limit), ErrorCode::InvalidRequest)?;
        }
        AutoModTrigger::Spam { duplicates, links } => {
            require(
                (*duplicates == 0 || (2..=10).contains(duplicates))
                    && *links <= 20
                    && (*duplicates > 0 || *links > 0),
                ErrorCode::InvalidRequest,
            )?;
        }
    }
    let a = &mut r.actions;
    a.message = a
        .message
        .take()
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty());
    if let Some(m) = &a.message {
        require(
            m.chars().count() <= MAX_AUTOMOD_MESSAGE_CHARS && !m.chars().any(char::is_control),
            ErrorCode::InvalidRequest,
        )?;
    }
    require(
        a.timeout_seconds <= MAX_TIMEOUT_SECONDS
            && (a.block || a.alert_channel_id.is_some() || a.timeout_seconds > 0),
        ErrorCode::InvalidRequest,
    )?;
    if let Some(ch) = a.alert_channel_id {
        let ch = db::channel(c, ch)
            .await
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        require(
            ch.summary.guild_id == Some(g) && ch.summary.kind == ChannelKind::Text,
            ErrorCode::InvalidRequest,
        )?;
    }
    unique(&r.exempt_roles, MAX_AUTOMOD_EXEMPT_ROLES)?;
    unique(&r.exempt_channels, MAX_AUTOMOD_EXEMPT_CHANNELS)?;
    let roles = acl::roles(c, g).await?;
    for x in &r.exempt_roles {
        require(roles.iter().any(|v| v.id == *x), ErrorCode::InvalidRequest)?;
    }
    for x in &r.exempt_channels {
        let ch = db::channel(c, *x)
            .await
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        require(ch.summary.guild_id == Some(g), ErrorCode::InvalidRequest)?;
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(v: &[&str]) -> Vec<Pattern> {
        v.iter().filter_map(|k| Pattern::parse(k)).collect()
    }
    #[test]
    fn keywords_match_words_with_wildcards_and_allow_list() {
        let k = p(&["cat", "dog*", "*fish", "*bad*", "free money"]);
        let a = p(&["dogma", "concatenate"]);
        let m = |s: &str| keyword_match(&k, &a, s);
        assert_eq!(m("I have a CAT."), Some("cat".into()));
        assert_eq!(m("category"), None, "plain keywords are whole words");
        assert_eq!(m("doggo time"), Some("doggo".into()));
        assert_eq!(m("dogma"), None, "allow list wins");
        assert_eq!(m("catfish swordfish"), Some("catfish".into()));
        assert_eq!(m("fishy"), None);
        assert_eq!(m("unbadly"), Some("unbadly".into()));
        assert_eq!(m("get FREE   money now"), Some("free money".into()));
        assert_eq!(m("free time, money later"), None);
        assert_eq!(m("concatenate"), None);
        assert_eq!(m(""), None);
    }
    #[test]
    fn keyword_validation_bounds_wildcards() {
        assert!(valid_keyword("cat"));
        assert!(valid_keyword("*cat*"));
        assert!(valid_keyword("two words"));
        assert!(!valid_keyword("c*t"));
        assert!(!valid_keyword("***"));
        assert!(!valid_keyword(""));
        assert!(!valid_keyword(&"a".repeat(61)));
    }
    #[test]
    fn mentions_and_links_are_counted_uniquely() {
        assert_eq!(mention_count("<@1> <@1> <@&5> <@2", &[Id(1), Id(3)]), 3);
        assert_eq!(
            link_count("https://a.b http://c.d www.e.f https://www.g.h"),
            4
        );
        assert_eq!(fingerprint(" Hello "), fingerprint("hello"));
    }
    #[test]
    fn exemptions_and_spam_rules() {
        let rule = |trigger| AutoModRule {
            id: Id(1),
            guild_id: Id(10),
            name: "r".into(),
            enabled: true,
            trigger,
            actions: AutoModActions {
                block: true,
                ..Default::default()
            },
            exempt_roles: vec![Id(30)],
            exempt_channels: vec![Id(20)],
        };
        let rules = vec![
            Compiled::new(rule(AutoModTrigger::MentionSpam { limit: 2 })),
            Compiled::new(rule(AutoModTrigger::Spam {
                duplicates: 3,
                links: 1,
            })),
        ];
        let d = |content, repeats| Draft {
            content,
            mentions: &[],
            repeats,
        };
        assert_eq!(
            check(&rules, &[], &[Id(11)], &d("<@1><@2><@3>", 0)).len(),
            1
        );
        assert!(check(&rules, &[Id(30)], &[Id(11)], &d("<@1><@2><@3>", 0)).is_empty());
        assert!(check(&rules, &[], &[Id(20), Id(21)], &d("<@1><@2><@3>", 0)).is_empty());
        assert!(check(&rules, &[], &[Id(11)], &d("same", 1)).is_empty());
        assert_eq!(check(&rules, &[], &[Id(11)], &d("same", 2)).len(), 1);
        assert_eq!(
            check(&rules, &[], &[Id(11)], &d("http://a http://b", 0)).len(),
            1
        );
    }
}
