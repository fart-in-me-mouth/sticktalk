//! Shared custom-emoji tokenization and completion. No runtime or renderer required.
use sticktalk_proto::*;
#[derive(Debug, PartialEq, Eq)]
pub enum Part<'a> {
    Text(&'a str),
    Emoji(Id, &'a str),
}
pub fn parts(mut text: &str) -> Vec<Part<'_>> {
    let mut out = Vec::new();
    while let Some(start) = text.find("<:") {
        let Some(end) = text[start..].find('>').map(|n| start + n + 1) else {
            break;
        };
        let token = &text[start..end];
        if let Some(id) = custom_emoji_id(token) {
            if start > 0 {
                out.push(Part::Text(&text[..start]));
            }
            out.push(Part::Emoji(id, token));
            text = &text[end..];
        } else {
            out.push(Part::Text(&text[..end]));
            text = &text[end..];
        }
    }
    if !text.is_empty() {
        out.push(Part::Text(text));
    }
    out
}
pub fn candidates<'a>(text: &str, caret: usize, emoji: &'a [GuildEmoji]) -> Vec<&'a GuildEmoji> {
    let Some((_, query)) = crate::emoji::query(text, caret) else {
        return Vec::new();
    };
    let query = query.to_ascii_lowercase();
    emoji
        .iter()
        .filter(|e| e.name.to_ascii_lowercase().contains(&query))
        .take(8)
        .collect()
}
pub fn prepare_send(text: &str, emoji: &[GuildEmoji]) -> String {
    // Replace server shortcodes before Unicode aliases. Existing custom tokens stay intact.
    let mut out = String::new();
    for part in parts(text) {
        match part {
            Part::Emoji(_, token) => out.push_str(token),
            Part::Text(text) => {
                let mut text = text.to_owned();
                for e in emoji {
                    text = text.replace(&format!(":{}:", e.name), &e.token());
                }
                for p in parts(&text) {
                    match p {
                        Part::Emoji(_, s) => out.push_str(s),
                        Part::Text(s) => out.push_str(&crate::emoji::prepare_send(s)),
                    }
                }
            }
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_completion_and_shortcode_collision_are_safe() {
        let e = GuildEmoji {
            id: Id(42),
            guild_id: Id(1),
            name: "stick_wave".into(),
            animated: true,
        };
        assert_eq!(
            candidates("你好 :sti", 11, std::slice::from_ref(&e)).len(),
            1
        );
        assert_eq!(
            prepare_send(":stick_wave: :smile:", std::slice::from_ref(&e)),
            "<:stick_wave:42> 😄"
        );
        let collision = GuildEmoji {
            name: "smile".into(),
            ..e.clone()
        };
        assert_eq!(prepare_send(":smile:", &[collision]), "<:smile:42>");
        assert_eq!(
            parts("hi <:stick_wave:42>!"),
            vec![
                Part::Text("hi "),
                Part::Emoji(Id(42), "<:stick_wave:42>"),
                Part::Text("!")
            ]
        );
        let parsed = crate::markdown::parse("<:stick_wave:42>");
        let crate::markdown::Block::Paragraph(v) = &parsed.blocks[0] else {
            panic!()
        };
        assert_eq!(v[0].text, e.token());
        let mut p = crate::launch::Projection::default();
        p.event(&Event::GuildEmojiUpdated {
            guild_id: Id(1),
            emoji: vec![e],
        });
        assert_eq!(p.emoji[&Id(1)].len(), 1);
        p.event(&Event::GuildRemoved {
            guild_id: Id(1),
            reason: RemovalReason::Left,
        });
        assert!(p.emoji.is_empty());
    }
}

/// A search surface keeps exactly one page and a bounded stack of keyset cursors.
#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    pub guild: Option<Id>,
    pub loading: bool,
    pub error: String,
    pub page: Option<MessageSearchPage>,
    pub serial: u64,
    pub cursors: Vec<Option<Id>>,
}
impl SearchState {
    pub fn start(&mut self, guild: Id, query: String, before: Option<Id>) -> u64 {
        self.serial = self.serial.wrapping_add(1);
        self.open = true;
        self.loading = true;
        self.error.clear();
        if self.guild != Some(guild) || self.query != query {
            self.cursors.clear();
            self.page = None;
        }
        self.guild = Some(guild);
        self.query = query;
        if self.cursors.last() != Some(&before) {
            self.cursors.push(before);
            if self.cursors.len() > 64 {
                self.cursors.remove(0);
            }
        }
        self.serial
    }
    pub fn apply(&mut self, serial: u64, result: Result<MessageSearchPage, String>) -> bool {
        if serial != self.serial {
            return false;
        }
        self.loading = false;
        match result {
            Ok(mut p) => {
                p.items.truncate(50);
                p.users.truncate(50);
                p.terms.truncate(32);
                self.page = Some(p);
            }
            Err(e) => self.error = e.chars().take(512).collect(),
        }
        true
    }
    pub fn close(&mut self) {
        self.serial = self.serial.wrapping_add(1);
        self.open = false;
        self.loading = false;
        self.page = None;
        self.error.clear();
    }
    pub fn purge_channel(&mut self, id: Id) {
        if let Some(p) = &mut self.page {
            p.items.retain(|m| m.channel_id != id);
        }
    }
}
/// Byte-safe Unicode case-fold highlighting; returns text fragments, never markup.
pub fn highlights<'a>(text: &'a str, terms: &[String]) -> Vec<(&'a str, bool)> {
    let mut folded = String::new();
    let mut map = Vec::new();
    for (i, c) in text.char_indices() {
        for lower in c.to_lowercase() {
            let start = folded.len();
            folded.push(lower);
            for _ in start..folded.len() {
                map.push(i);
            }
        }
    }
    map.push(text.len());
    let mut ranges = Vec::new();
    for term in terms.iter().take(32) {
        let term = term.to_lowercase();
        if term.is_empty() {
            continue;
        }
        for (start, _) in folded.match_indices(&term) {
            let end = start + term.len();
            let a = map[start];
            let b = if end >= folded.len() {
                text.len()
            } else {
                let pos = map[end];
                if pos == map[end - 1] {
                    text[pos..]
                        .char_indices()
                        .nth(1)
                        .map_or(text.len(), |(n, _)| pos + n)
                } else {
                    pos
                }
            };
            ranges.push((a, b));
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in ranges {
        if let Some(last) = merged.last_mut().filter(|v| a <= v.1) {
            last.1 = last.1.max(b);
        } else {
            merged.push((a, b));
        }
    }
    let mut out = Vec::new();
    let mut cursor = 0;
    for (a, b) in merged {
        if a > cursor {
            out.push((&text[cursor..a], false));
        }
        out.push((&text[a..b], true));
        cursor = b;
    }
    if cursor < text.len() {
        out.push((&text[cursor..], false));
    }
    out
}
#[cfg(test)]
mod search_tests {
    use super::*;
    #[test]
    fn stale_searches_do_not_replace_newer_results_and_highlights_are_utf8_safe() {
        let mut s = SearchState::default();
        let old = s.start(Id(1), "old".into(), None);
        let new = s.start(Id(1), "new".into(), None);
        assert!(!s.apply(old, Err("old error".into())));
        assert!(s.apply(new, Err("new error".into())));
        assert_eq!(s.error, "new error");
        assert_eq!(
            highlights("你好 WAVE café", &["wave".into(), "café".into()]),
            vec![
                ("你好 ", false),
                ("WAVE", true),
                (" ", false),
                ("café", true)
            ]
        );
        assert_eq!(
            highlights("İstanbul", &["i".into()]),
            vec![("İ", true), ("stanbul", false)]
        );
        for _ in 0..100 {
            s.start(Id(1), "new".into(), Some(Id(s.serial)));
        }
        assert!(s.cursors.len() <= 64);
        s.close();
        assert!(!s.apply(new, Err("late".into())));
    }
}

/// A monotonic-host clock for slowmode. Server timestamps never rely on the device's wall clock.
#[derive(Default, Debug, Clone)]
pub struct Cooldown {
    pub seconds: u32,
    pub bypassed: bool,
    until: u64,
    server_at: u64,
    local_at: u64,
}
impl Cooldown {
    pub fn receive(&mut self, status: &SlowmodeStatus, local_now: u64) {
        self.seconds = status.seconds;
        self.bypassed = status.bypassed;
        self.server_at = status.server_time;
        self.local_at = local_now;
        self.until = local_now
            .saturating_add(u64::from(status.retry_after_ms).min(u64::from(status.seconds) * 1000));
    }
    pub fn remaining(&self, local_now: u64) -> u64 {
        if self.seconds == 0 || self.bypassed {
            0
        } else {
            self.until.saturating_sub(local_now)
        }
    }
    pub fn observe_send(&mut self, created_at: u64, local_now: u64) {
        let server_now = self
            .server_at
            .saturating_add(local_now.saturating_sub(self.local_at));
        let remaining = created_at
            .saturating_add(u64::from(self.seconds) * 1000)
            .saturating_sub(server_now)
            .min(u64::from(self.seconds) * 1000);
        self.until = local_now.saturating_add(remaining);
    }
    pub fn rejected(&mut self, retry_ms: u32, local_now: u64) {
        self.until =
            local_now.saturating_add(u64::from(retry_ms).min(u64::from(self.seconds) * 1000));
    }
}
#[cfg(test)]
mod slowmode_tests {
    use super::*;
    #[test]
    fn cooldown_is_monotonic_handles_clock_skew_replay_and_bypass() {
        let mut c = Cooldown::default();
        let mut s = SlowmodeStatus {
            channel_id: Id(1),
            seconds: 5,
            bypassed: false,
            retry_after_ms: 3000,
            server_time: 1_000_000_000,
        };
        c.receive(&s, 10);
        assert_eq!(c.remaining(10), 3000);
        assert_eq!(c.remaining(3010), 0);
        c.observe_send(s.server_time + 5000, 5010);
        assert_eq!(c.remaining(5010), 5000);
        c.observe_send(s.server_time - 100000, 5010);
        assert_eq!(c.remaining(5010), 0);
        c.rejected(1000, 6000);
        assert_eq!(c.remaining(6500), 500);
        s.bypassed = true;
        c.receive(&s, 7000);
        assert_eq!(c.remaining(7000), 0);
        s.bypassed = false;
        s.seconds = 0;
        c.receive(&s, 8000);
        assert_eq!(c.remaining(8000), 0);
    }
}

/// Move one server atomically between folders, dropping groups emptied by the move.
pub fn move_server(folders: &mut Vec<ServerFolder>, guild: Id, destination: Option<u32>) -> bool {
    if destination.is_some_and(|id| !folders.iter().any(|f| f.id == id)) {
        return false;
    }
    for f in folders.iter_mut() {
        f.guild_ids.retain(|g| *g != guild);
    }
    if let Some(id) = destination {
        if let Some(f) = folders.iter_mut().find(|f| f.id == id) {
            f.guild_ids.push(guild);
        }
    }
    folders.retain(|f| !f.guild_ids.is_empty());
    true
}
#[cfg(test)]
mod folder_notification_tests {
    use super::*;
    #[test]
    fn moves_are_unique_folders_are_purged_on_leave_and_master_switch_survives_restart() {
        let mut f = vec![
            ServerFolder {
                id: 1,
                name: "Work".into(),
                color: 1,
                collapsed: true,
                guild_ids: vec![Id(10), Id(20)],
            },
            ServerFolder {
                id: 2,
                name: "Friends".into(),
                color: 2,
                collapsed: false,
                guild_ids: vec![Id(30)],
            },
        ];
        assert!(move_server(&mut f, Id(10), Some(2)));
        assert_eq!(f[0].guild_ids, vec![Id(20)]);
        assert_eq!(f[1].guild_ids, vec![Id(30), Id(10)]);
        assert!(!move_server(&mut f, Id(20), Some(99)));
        assert!(move_server(&mut f, Id(20), None));
        assert_eq!(f.len(), 1);
        let mut p = crate::launch::Projection::default();
        p.event(&Event::ServerFoldersUpdated(f));
        p.event(&Event::GuildRemoved {
            guild_id: Id(10),
            reason: RemovalReason::Left,
        });
        assert_eq!(p.server_folders[0].guild_ids, vec![Id(30)]);
        let mut prefs = crate::settings::Preferences::default();
        assert!(prefs.desktop_notifications);
        prefs.desktop_notifications = false;
        let saved = serde_json::to_vec(&prefs).unwrap();
        let restored: crate::settings::Preferences = serde_json::from_slice(&saved).unwrap();
        assert!(!restored.desktop_notifications);
        let old: crate::settings::Preferences = serde_json::from_str("{}").unwrap();
        assert!(old.desktop_notifications);
        let c = crate::notifications::Context {
            direct: true,
            mentioned: true,
            ..Default::default()
        };
        let override_all = NotificationSettings {
            guild_id: None,
            channel_id: Some(Id(1)),
            level: NotificationLevel::All,
            muted: false,
            mute_until: None,
            suppress_everyone: false,
            hide_muted_channels: false,
        };
        assert!(!crate::notifications::permits_on_device(
            std::slice::from_ref(&override_all),
            c,
            Some(NotificationLevel::All),
            false
        ));
        assert!(crate::notifications::permits_on_device(
            &[override_all],
            c,
            Some(NotificationLevel::All),
            true
        ));
    }
}
