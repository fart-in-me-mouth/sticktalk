//! Server-synced notification precedence. Suppression never changes unread counts.
use sticktalk_proto::*;
#[derive(Clone, Copy, Default)]
pub struct Context {
    pub guild: Option<Id>,
    pub channel: Option<Id>,
    pub direct: bool,
    pub mentioned: bool,
    pub everyone_only: bool,
    pub focused: bool,
    pub viewing: bool,
    pub dnd: bool,
    pub own: bool,
    pub blocked: bool,
    pub now: u64,
}
pub fn permits(settings: &[NotificationSettings], c: Context) -> bool {
    if c.own || c.blocked || c.dnd || c.focused && c.viewing {
        return false;
    }
    let server = settings
        .iter()
        .find(|s| s.channel_id.is_none() && s.guild_id == c.guild);
    let channel = settings
        .iter()
        .find(|s| s.channel_id.is_some() && s.channel_id == c.channel);
    let muted =
        |s: &NotificationSettings| s.muted && s.mute_until.is_none_or(|until| until > c.now);
    if server.is_some_and(muted) || channel.is_some_and(muted) {
        return false;
    }
    let chosen = channel.or(server);
    let mention = c.mentioned && !(c.everyone_only && chosen.is_some_and(|s| s.suppress_everyone));
    match chosen.map(|s| s.level).unwrap_or(if c.direct {
        NotificationLevel::All
    } else {
        NotificationLevel::Mentions
    }) {
        NotificationLevel::All => !c.everyone_only || mention,
        NotificationLevel::Mentions => mention,
        NotificationLevel::Nothing => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn setting(channel: Option<Id>, level: NotificationLevel) -> NotificationSettings {
        NotificationSettings {
            guild_id: Some(Id(1)),
            channel_id: channel,
            level,
            muted: false,
            mute_until: None,
            suppress_everyone: false,
            hide_muted_channels: false,
        }
    }
    #[test]
    fn server_channel_expiry_and_focus() {
        let c = Context {
            guild: Some(Id(1)),
            channel: Some(Id(2)),
            mentioned: true,
            now: 100,
            ..Default::default()
        };
        assert!(permits(&[], c));
        let mut s = vec![
            setting(None, NotificationLevel::All),
            setting(Some(Id(2)), NotificationLevel::Nothing),
        ];
        assert!(!permits(&s, c));
        s[1].level = NotificationLevel::Mentions;
        assert!(permits(&s, c));
        s[0].muted = true;
        s[0].mute_until = Some(101);
        assert!(!permits(&s, c));
        s[0].mute_until = Some(100);
        assert!(permits(&s, c));
        for c in [
            Context {
                focused: true,
                viewing: true,
                ..c
            },
            Context { dnd: true, ..c },
            Context { own: true, ..c },
            Context { blocked: true, ..c },
        ] {
            assert!(!permits(&s, c));
        }
        s[1].suppress_everyone = true;
        assert!(!permits(
            &s,
            Context {
                everyone_only: true,
                ..c
            }
        ));
        assert!(permits(&s, c));
    }
    #[test]
    fn dm_nothing_and_mentions_are_not_all() {
        let c = Context {
            direct: true,
            channel: Some(Id(2)),
            ..Default::default()
        };
        assert!(permits(&[], c));
        let mut s = setting(Some(Id(2)), NotificationLevel::Nothing);
        s.guild_id = None;
        assert!(!permits(&[s.clone()], c));
        s.level = NotificationLevel::Mentions;
        assert!(!permits(&[s.clone()], c));
        assert!(permits(
            &[s],
            Context {
                mentioned: true,
                ..c
            }
        ));
    }
}
