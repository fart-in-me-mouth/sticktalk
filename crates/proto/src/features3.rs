//! Additive text feature DTOs. Custom emoji tokens are `<:name:id>`.
use crate::*;
use serde::{Deserialize, Serialize};
pub const MAX_GUILD_EMOJI: usize = 50;
pub const MAX_EMOJI_BYTES: usize = 256 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildEmoji {
    pub id: Id,
    pub guild_id: GuildId,
    pub name: String,
    pub animated: bool,
}
impl GuildEmoji {
    pub fn token(&self) -> String {
        format!("<:{}:{}>", self.name, self.id.0)
    }
}
/// Names are stable ASCII identifiers; case-insensitively unique in each server.
pub fn valid_emoji_name(name: &str) -> bool {
    (2..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
/// Parse a complete custom token without accepting URLs, markup, or arbitrary IDs.
pub fn custom_emoji_id(token: &str) -> Option<Id> {
    let inner = token.strip_prefix("<:")?.strip_suffix('>')?;
    let (name, id) = inner.split_once(':')?;
    if !valid_emoji_name(name) {
        return None;
    }
    let id = id.parse::<u64>().ok()?;
    (id > 0 && id <= i64::MAX as u64).then_some(Id(id))
}
