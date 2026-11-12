use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Untrusted OS/protocol input. The host still checks channel access/invite validity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Activation {
    Open,
    Invite(String),
    Message { channel_id: u64, message_id: u64 },
}
impl Activation {
    pub fn parse(link: &str) -> Result<Self> {
        if link.len() > 1024 {
            return Err(Error::Invalid("deep link too long"));
        }
        let url = url::Url::parse(link).map_err(|_| Error::Invalid("deep link URL"))?;
        if url.scheme() != "sticktalk"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::Invalid("deep link origin/components"));
        }
        let parts: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
        match (url.host_str(), parts.as_slice()) {
            (Some("open"), [""]) => Ok(Self::Open),
            (Some("invite"), [code])
                if !code.is_empty()
                    && code.len() <= 128
                    && code
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') =>
            {
                Ok(Self::Invite((*code).into()))
            }
            (Some("message"), [channel, message]) => {
                let id = |s: &str| {
                    s.parse::<u64>()
                        .ok()
                        .filter(|n| *n > 0 && *n <= i64::MAX as u64)
                        .ok_or(Error::Invalid("message ID"))
                };
                Ok(Self::Message {
                    channel_id: id(channel)?,
                    message_id: id(message)?,
                })
            }
            _ => Err(Error::Invalid("deep link route")),
        }
    }
    pub fn uri(&self) -> String {
        match self {
            Self::Open => "sticktalk://open".into(),
            Self::Invite(code) => format!("sticktalk://invite/{code}"),
            Self::Message {
                channel_id,
                message_id,
            } => format!("sticktalk://message/{channel_id}/{message_id}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deep_links_are_bounded_and_strict() {
        for s in [
            "sticktalk://open",
            "sticktalk://invite/abc-123_",
            "sticktalk://message/1/2",
        ] {
            assert_eq!(
                Activation::parse(s).unwrap(),
                Activation::parse(&Activation::parse(s).unwrap().uri()).unwrap()
            );
        }
        for s in [
            "https://invite/a",
            "sticktalk://invite/a%22",
            "sticktalk://message/0/2",
            "sticktalk://message/1/9223372036854775808",
            "sticktalk://open?cmd=quit",
            "sticktalk://invite/a/b",
            "sticktalk://user@open",
        ] {
            assert!(Activation::parse(s).is_err(), "{s}");
        }
    }
}
