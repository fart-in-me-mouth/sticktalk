use crate::{activation::Activation, Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotifyLevel {
    All,
    #[default]
    Mentions,
    Nothing,
}

/// A channel override wins over its server setting. Store this in host preferences.
#[derive(Clone, Copy, Debug, Default)]
pub struct NotificationPolicy {
    pub server: NotifyLevel,
    pub channel: Option<NotifyLevel>,
    pub muted: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct NotificationContext {
    pub direct_message: bool,
    pub mention: bool,
    pub window_focused: bool,
    pub viewing_channel: bool,
    pub do_not_disturb: bool,
}
impl NotificationPolicy {
    pub fn permits(&self, c: NotificationContext) -> bool {
        if self.muted || c.do_not_disturb || (c.window_focused && c.viewing_channel) {
            return false;
        }
        match self.channel.unwrap_or(self.server) {
            NotifyLevel::All => true,
            NotifyLevel::Mentions => c.direct_message || c.mention,
            NotifyLevel::Nothing => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MessageToast {
    pub sender: String,
    pub server_channel: String,
    pub preview: String,
    /// Cached local image file URI. No network/avatar decoding work in this layer.
    pub avatar_uri: Option<String>,
    pub channel_id: u64,
    pub message_id: u64,
}
fn xml(s: &str) -> String {
    s.chars()
        .filter(|c| {
            (*c >= ' ' && !matches!(c, '\u{fffe}' | '\u{ffff}')) || matches!(c, '\n' | '\r' | '\t')
        })
        .fold(String::new(), |mut out, c| {
            out.push_str(match c {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '"' => "&quot;",
                '\'' => "&apos;",
                _ => {
                    out.push(c);
                    return out;
                }
            });
            out
        })
}
impl MessageToast {
    pub fn xml(&self) -> Result<String> {
        if self.sender.len() > 512 || self.server_channel.len() > 1024 || self.preview.len() > 4096
        {
            return Err(Error::Invalid("toast text length"));
        }
        let link = Activation::Message {
            channel_id: self.channel_id,
            message_id: self.message_id,
        }
        .uri();
        Activation::parse(&link)?;
        let avatar = match &self.avatar_uri {
            Some(uri) => {
                let parsed = url::Url::parse(uri).map_err(|_| Error::Invalid("avatar file URI"))?;
                if parsed.scheme() != "file"
                    || parsed.host_str().is_some_and(|h| h != "localhost")
                    || uri.len() > 2048
                {
                    return Err(Error::Invalid("avatar must be a local file URI"));
                }
                format!(
                    "<image placement=\"appLogoOverride\" hint-crop=\"circle\" src=\"{}\"/>",
                    xml(uri)
                )
            }
            None => String::new(),
        };
        // Protocol activation works even after the app exits, without a COM server.
        // Inline reply requires a COM activator and is intentionally not advertised.
        Ok(format!("<toast activationType=\"protocol\" launch=\"{link}\"><visual><binding template=\"ToastGeneric\">{avatar}<text>{}</text><text>{}</text><text>{}</text></binding></visual><audio silent=\"true\"/></toast>", xml(&self.sender), xml(&self.server_channel), xml(&self.preview)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_respects_overrides_focus_dm_and_dnd() {
        let mut p = NotificationPolicy::default();
        let mut c = NotificationContext {
            direct_message: true,
            ..Default::default()
        };
        assert!(p.permits(c));
        c.window_focused = true;
        c.viewing_channel = true;
        assert!(!p.permits(c));
        c.window_focused = false;
        p.channel = Some(NotifyLevel::Nothing);
        assert!(!p.permits(c));
        p.channel = Some(NotifyLevel::All);
        assert!(p.permits(c));
        c.do_not_disturb = true;
        assert!(!p.permits(c));
        c.do_not_disturb = false;
        p.muted = true;
        assert!(!p.permits(c));
    }
    #[test]
    fn toast_escapes_text_and_contains_activation() {
        let mut t = MessageToast {
            sender: "<Ada & Bob>".into(),
            server_channel: "Friends / #general".into(),
            preview: "\0\"hello\"".into(),
            avatar_uri: Some("file:///C:/cache/a.png".into()),
            channel_id: 1,
            message_id: 2,
        };
        let xml = t.xml().unwrap();
        assert!(xml.contains("&lt;Ada &amp; Bob&gt;"));
        assert!(!xml.contains('\0'));
        assert!(xml.contains("sticktalk://message/1/2"));
        assert!(xml.contains("hint-crop=\"circle\""));
        t.avatar_uri = Some("https://example.com/a.png".into());
        assert!(t.xml().is_err());
    }
}
