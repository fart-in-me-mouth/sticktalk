//! Rich message content: polls, forwarded snapshots, link previews, attachment
//! descriptions/spoilers, voice messages and scheduled messages.
//!
//! Everything here is additive. New fields on `Message` and `Attachment` are
//! omitted when empty, so older peers see exactly the old wire shape. New events
//! travel inside `Event::Content`, which the server sends only to connections
//! that opted in with `ContentCommand::Subscribe`. See `docs/features/content.md`.
use crate::*;
use serde::{Deserialize, Serialize};

pub const MIN_POLL_ANSWERS: usize = 2;
pub const MAX_POLL_ANSWERS: usize = 10;
pub const MAX_POLL_QUESTION_CHARS: usize = 300;
pub const MAX_POLL_ANSWER_CHARS: usize = 55;
/// The poll lengths a creator can choose, in hours (1 hour to 2 weeks).
pub const POLL_DURATION_HOURS: &[u32] = &[1, 4, 8, 24, 72, 168, 336];
pub const MAX_FORWARD_DESTINATIONS: usize = 5;
pub const MAX_ATTACHMENT_DESCRIPTION_CHARS: usize = 1024;
/// Voice messages are at most 20 minutes long.
pub const MAX_VOICE_MESSAGE_MS: u32 = 20 * 60 * 1000;
/// Waveform samples (0..=255 amplitude each) stored with a voice message.
pub const MAX_VOICE_WAVEFORM_SAMPLES: usize = 256;
pub const MAX_SCHEDULED_PER_USER: usize = 50;
/// A message can be scheduled up to 30 days ahead.
pub const MAX_SCHEDULE_AHEAD_MS: u64 = 30 * 86_400_000;
pub const MAX_LINK_EMBEDS: usize = 5;

/// Optional rich content carried by a message. Empty for ordinary messages.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MessageExtra {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<Box<Poll>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarded: Option<Box<ForwardedMessage>>,
    /// Link preview cards, filled in by the server shortly after sending.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embeds: Vec<LinkEmbed>,
    /// Sent from the GIF picker: the provider ID (`klipy:...`), so anyone can
    /// add the GIF to their favorites. The message text is its media URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gif: Option<String>,
    /// Stickers sent with the message (`expression.rs`), at most three.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stickers: Vec<crate::StickerItem>,
    /// Super Reactions, kept apart from `Message::reactions` so older apps
    /// never see them (`expression.rs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub super_reactions: Vec<ReactionSummary>,
}
impl MessageExtra {
    pub fn is_empty(&self) -> bool {
        self.poll.is_none()
            && self.forwarded.is_none()
            && self.embeds.is_empty()
            && self.gif.is_none()
            && self.stickers.is_empty()
            && self.super_reactions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollAnswer {
    pub text: String,
    /// One Unicode emoji or a `<:name:id>` custom emoji token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    pub votes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poll {
    pub question: String,
    pub answers: Vec<PollAnswer>,
    /// Voters may pick more than one answer.
    pub multiple: bool,
    pub expires_at: UnixMillis,
    /// Ended, early or at its deadline. Closed polls take no votes.
    pub closed: bool,
    /// Distinct people who voted.
    pub voters: u32,
    /// The receiving person's own answers (indexes). Empty when they have not
    /// voted, and in broadcasts that are not about that person's vote.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub my_votes: Vec<u8>,
}
impl Poll {
    /// Answers with the most votes once the poll has closed; none if nobody voted.
    pub fn winners(&self) -> Vec<usize> {
        let top = self.answers.iter().map(|a| a.votes).max().unwrap_or(0);
        if !self.closed || top == 0 {
            return vec![];
        }
        (0..self.answers.len())
            .filter(|i| self.answers[*i].votes == top)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollAnswerDraft {
    pub text: String,
    #[serde(default)]
    pub emoji: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollDraft {
    pub channel_id: ChannelId,
    pub question: String,
    pub answers: Vec<PollAnswerDraft>,
    pub multiple: bool,
    /// One of `POLL_DURATION_HOURS`.
    pub duration_hours: u32,
}

/// A copy of a message as it was when forwarded. Never updated afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardedMessage {
    /// Mentions are already written out as "@name"; markdown is preserved.
    pub content: String,
    pub attachments: Vec<Attachment>,
    pub created_at: UnixMillis,
    /// Where it came from: present only for people who can read the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ForwardSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardSource {
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<GuildId>,
}

/// A website preview card. Text is plain (never HTML). The image is served by
/// this server, so viewers never contact the linked site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkEmbed {
    /// The link as written in the message.
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<EmbedImage>,
    /// The page offers a video player; open the link to watch it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub video: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedImage {
    /// Same-origin path, e.g. `/api/v1/channels/1/messages/2/embeds/0/image`.
    pub url: String,
    pub width: u16,
    pub height: u16,
}

/// Author-supplied attachment details. Stored with the upload.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AttachmentMeta {
    /// Alt text, up to `MAX_ATTACHMENT_DESCRIPTION_CHARS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Hidden behind a "Spoiler" cover until clicked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub spoiler: bool,
    /// Set for voice messages: the recording's length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
    /// Set for voice messages: amplitudes computed once while recording.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waveform: Vec<u8>,
}
impl AttachmentMeta {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
    pub fn voice(&self) -> bool {
        self.duration_ms.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentMetaPatch {
    pub attachment_id: AttachmentId,
    pub meta: AttachmentMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledMessage {
    pub id: Id,
    pub channel_id: ChannelId,
    pub content: String,
    pub mention_ids: Vec<UserId>,
    pub send_at: UnixMillis,
    pub created_at: UnixMillis,
    /// Set when delivery was refused (for example, you can no longer send
    /// there). A failed message stays listed until it is rescheduled or cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ContentCommand {
    /// Opt this connection in to `Event::Content`. Sent after every Ready.
    Subscribe,
    /// `SendMessage` plus attachment details (descriptions, spoilers, voice).
    SendMessage {
        draft: MessageDraft,
        #[serde(default)]
        attachments: Vec<AttachmentMetaPatch>,
    },
    CreatePoll(PollDraft),
    /// A GIF from the picker: `url` is its send rendition on the provider's media host.
    SendGif {
        channel_id: ChannelId,
        gif_id: String,
        url: String,
    },
    /// Replace your vote. An empty list removes it.
    VotePoll {
        channel_id: ChannelId,
        message_id: MessageId,
        answers: Vec<u8>,
    },
    /// The author, or someone with Manage Messages, ends the poll now.
    EndPoll {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    Forward {
        channel_id: ChannelId,
        message_id: MessageId,
        destinations: Vec<ChannelId>,
        comment: String,
    },
    /// The author, or someone with Manage Messages, removes link previews.
    RemoveEmbeds {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    ScheduleMessage {
        channel_id: ChannelId,
        content: String,
        mention_ids: Vec<UserId>,
        send_at: UnixMillis,
    },
    UpdateScheduled {
        id: Id,
        content: String,
        mention_ids: Vec<UserId>,
        send_at: UnixMillis,
    },
    CancelScheduled {
        id: Id,
    },
    SendScheduledNow {
        id: Id,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ContentResult {
    Ack,
    Poll {
        channel_id: ChannelId,
        message_id: MessageId,
        poll: Poll,
    },
    /// One forwarded copy per destination.
    Forwarded(Vec<Message>),
    Scheduled(ScheduledMessage),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ContentEvent {
    /// New tallies. `voter` names whose vote changed and what it now is.
    PollUpdated {
        channel_id: ChannelId,
        message_id: MessageId,
        poll: Poll,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        voter: Option<PollVoter>,
    },
    EmbedsUpdated {
        channel_id: ChannelId,
        message_id: MessageId,
        embeds: Vec<LinkEmbed>,
    },
    /// Only to the author's own sessions.
    ScheduledUpserted(ScheduledMessage),
    /// Only to the author's own sessions: sent (`sent` is the new message) or cancelled.
    ScheduledRemoved {
        id: Id,
        channel_id: ChannelId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sent: Option<MessageId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollVoter {
    pub user_id: UserId,
    pub answers: Vec<u8>,
}

/// `GET /api/v1/channels/{c}/messages/{m}/poll/answers/{i}/voters?after=`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollVotersPage {
    pub users: Vec<UserSummary>,
    pub has_more: bool,
}

/// `GET /api/v1/users/@me/scheduled-messages`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledMessagesPage {
    pub items: Vec<ScheduledMessage>,
}

/// Every `http(s)://` link in message text that should get a preview, in
/// order and without duplicates. Links wrapped in `<...>`, inside code, or
/// inside a spoiler are skipped, as in Discord.
pub fn preview_links(content: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let (mut fence, mut tick, mut spoiler) = (false, false, false);
    let mut rest = content;
    while let Some(c) = rest.chars().next() {
        if let Some(tail) = rest.strip_prefix("```") {
            fence = !fence;
            rest = tail;
            continue;
        }
        if let Some(tail) = rest.strip_prefix("||") {
            spoiler = !spoiler;
            rest = tail;
            continue;
        }
        if c == '`' && !fence {
            tick = !tick;
        } else if c == '\n' {
            tick = false;
        } else if !fence && !tick && !spoiler {
            if let Some(tail) = rest.strip_prefix('<') {
                if tail.starts_with("http://") || tail.starts_with("https://") {
                    // A suppressed link: skip it entirely.
                    let end = tail
                        .find(|c: char| c == '>' || c.is_whitespace())
                        .unwrap_or(tail.len());
                    rest = &tail[end..];
                    continue;
                }
            }
            if rest.starts_with("https://") || rest.starts_with("http://") {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let url = rest[..end].trim_end_matches(['.', ',', '!', ';', ')', ']', '>', '"']);
                if url.len() > 10 && url.len() <= 2048 && !out.iter().any(|u| u == url) {
                    out.push(url.to_owned());
                }
                rest = &rest[end..];
                if out.len() >= MAX_LINK_EMBEDS {
                    break;
                }
                continue;
            }
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_links_skip_suppressed_code_and_spoilers() {
        let text = "see https://a.example/x. and <https://b.example/y> `https://c.example` ||https://d.example|| ```\nhttps://e.example\n``` https://a.example/x https://f.example/(1)";
        assert_eq!(
            preview_links(text),
            vec!["https://a.example/x", "https://f.example/(1"]
        );
        let many = (0..9)
            .map(|i| format!("https://site{i}.example/"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(preview_links(&many).len(), MAX_LINK_EMBEDS);
    }
    #[test]
    fn empty_extras_are_not_on_the_wire() {
        assert!(MessageExtra::default().is_empty());
        assert!(AttachmentMeta::default().is_empty());
        let poll = Poll {
            question: "q".into(),
            answers: vec![
                PollAnswer {
                    text: "a".into(),
                    emoji: None,
                    votes: 2,
                },
                PollAnswer {
                    text: "b".into(),
                    emoji: None,
                    votes: 2,
                },
            ],
            multiple: false,
            expires_at: 1,
            closed: true,
            voters: 4,
            my_votes: vec![],
        };
        assert_eq!(poll.winners(), vec![0, 1]);
        let json = serde_json::to_string(&poll).unwrap();
        assert!(!json.contains("my_votes"));
    }
}
