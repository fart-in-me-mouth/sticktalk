use crate::model::ConnectionState;
use sticktalk_proto::*;
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct Config {
    pub server: String,
    pub token: String,
    pub mock: bool,
    pub generation: u64,
    pub scenario: String,
}
#[derive(Clone)]
pub struct Session {
    pub config: Config,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session(<redacted>)")
    }
}
#[derive(Debug, Clone)]
pub enum Request<R = String> {
    Launch {
        read: crate::launch::Read,
        generation: u64,
    },
    /// Ephemeral: never queued/replayed as a durable command.
    Voice(VoicePayload),
    Settings {
        query: crate::settings::Query,
        generation: u64,
    },
    Command(CommandRequest),
    Applied(ResumeCursor),
    View,
    History {
        seek: Option<u64>,
        channel: Id,
        before: Option<Id>,
        generation: u64,
    },
    NewerHistory {
        seek: u64,
        channel: Id,
        after: Id,
        generation: u64,
    },
    AroundHistory {
        channel: Id,
        anchor: HistoryAnchor,
        seek: u64,
        generation: u64,
    },
    Channel {
        channel: Id,
        generation: u64,
    },
    Members {
        guild: Id,
        after: Option<Id>,
        generation: u64,
    },
    Invites {
        after: Option<Id>,
        guild: Id,
        generation: u64,
    },
    Bans {
        after: Option<Id>,
        guild: Id,
        generation: u64,
    },
    Upload {
        form_revision: u64,
        path: R,
        purpose: UploadPurpose,
        channel: Option<Id>,
        generation: u64,
    },
    Users {
        ids: Vec<Id>,
        generation: u64,
    },
    Message {
        channel: Id,
        message: Id,
        generation: u64,
    },
    DiscardUpload(Id),
    SaveOriginal {
        attachment: Id,
        path: R,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryAnchor {
    Message(Id),
    Timestamp(UnixMillis),
}
impl HistoryAnchor {
    /// A message ID or an ISO calendar date at midnight UTC, bounded to Snowflake time.
    pub fn parse(value: &str) -> Result<Self, String> {
        let invalid = || "Enter a message ID or a date as YYYY-MM-DD.".to_string();
        if let Ok(id) = value.parse::<u64>() {
            return if (1..=i64::MAX as u64).contains(&id) {
                Ok(Self::Message(Id(id)))
            } else {
                Err(invalid())
            };
        }
        let parts: Vec<_> = value.split('-').collect();
        if parts.len() != 3
            || !parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
            || parts[0].len() != 4
            || parts[1].len() != 2
            || parts[2].len() != 2
        {
            return Err(invalid());
        }
        let year: u64 = parts[0].parse().map_err(|_| invalid())?;
        let month: usize = parts[1].parse().map_err(|_| invalid())?;
        let day: u64 = parts[2].parse().map_err(|_| invalid())?;
        if !(2025..=2094).contains(&year) || !(1..=12).contains(&month) {
            return Err(invalid());
        }
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let months = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        if day == 0 || day > months[month - 1] {
            return Err(invalid());
        }
        let leaps = |y: u64| (y - 1) / 4 - (y - 1) / 100 + (y - 1) / 400;
        let days = 365 * (year - 1970) + leaps(year) - leaps(1970)
            + months[..month - 1].iter().sum::<u64>()
            + day
            - 1;
        let time = days * 86_400_000;
        if time - SNOWFLAKE_EPOCH_MS > (i64::MAX as u64 >> 22) {
            return Err(invalid());
        }
        Ok(Self::Timestamp(time))
    }

    pub fn query(self) -> String {
        match self {
            Self::Message(id) => format!("around={}", id.0),
            Self::Timestamp(time) => format!("around_timestamp={time}"),
        }
    }
}
#[derive(Debug, Clone)]
pub enum Data<H = ()> {
    Launch {
        data: crate::launch::SnapshotData,
        generation: u64,
    },
    Settings {
        snapshot: Snapshot<crate::settings::SnapshotData>,
        generation: u64,
    },
    Connected(H),
    Rejected {
        nonce: Nonce,
        reason: String,
    },
    Uncertain(Vec<Nonce>),
    State(ConnectionState),
    Frame(ServerMsg),
    History {
        seek: Option<u64>,
        channel: Id,
        page: Page<Message>,
        generation: u64,
    },
    NewerHistory {
        seek: u64,
        channel: Id,
        page: Page<Message>,
        generation: u64,
    },
    AroundHistory {
        channel: Id,
        anchor: HistoryAnchor,
        seek: u64,
        page: Page<Message>,
        generation: u64,
    },
    Channel {
        snapshot: Snapshot<Channel>,
        generation: u64,
    },
    Members {
        guild: Id,
        after: Option<Id>,
        page: Page<Member>,
        generation: u64,
    },
    Invites {
        guild: Id,
        after: Option<Id>,
        page: Page<Invite>,
        generation: u64,
    },
    Bans {
        guild: Id,
        after: Option<Id>,
        page: Page<Ban>,
        generation: u64,
    },
    Uploaded {
        form_revision: u64,
        response: UploadResponse,
        purpose: UploadPurpose,
        channel: Option<Id>,
        generation: u64,
    },
    Error(String),
    Saved,
    Users {
        snapshot: Snapshot<Vec<UserSummary>>,
        generation: u64,
    },
    Message {
        snapshot: Snapshot<Message>,
        generation: u64,
    },
}
#[derive(Clone)]
pub enum AccountAction {
    Read,
    Link(String),
    Unlink,
    Password {
        current: Option<String>,
        new: String,
    },
}
impl std::fmt::Debug for AccountAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccountAction(<redacted>)")
    }
}
pub fn decode(bytes: &[u8]) -> Result<ServerMsg, String> {
    if bytes.len() > MAX_WS_FRAME_BYTES as usize {
        return Err("Server frame exceeds 512 KiB".into());
    }
    let mut de = rmp_serde::Deserializer::new(std::io::Cursor::new(bytes));
    de.set_max_depth(32);
    let v = serde::Deserialize::deserialize(&mut de)
        .map_err(|_| "Invalid server protocol frame".to_string())?;
    if de.position() != bytes.len() as u64 {
        return Err("Trailing bytes in server frame".into());
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jump_targets_validate_dates_and_ids_without_a_timezone_or_clock() {
        assert_eq!(
            HistoryAnchor::parse("2025-01-01"),
            Ok(HistoryAnchor::Timestamp(SNOWFLAKE_EPOCH_MS))
        );
        assert_eq!(
            HistoryAnchor::parse("2028-03-01"),
            Ok(HistoryAnchor::Timestamp(
                SNOWFLAKE_EPOCH_MS + 1155 * 86_400_000
            ))
        );
        assert_eq!(
            HistoryAnchor::parse("42"),
            Ok(HistoryAnchor::Message(Id(42)))
        );
        for value in [
            "0",
            "9223372036854775808",
            "2024-12-31",
            "2026-02-29",
            "2026-13-01",
            "2026-01-00",
            "2026-1-01",
            "2095-01-01",
        ] {
            assert!(HistoryAnchor::parse(value).is_err(), "{value}");
        }
    }
    #[test]
    fn rejects_trailing_and_large_frames() {
        let msg = ServerMsg::HeartbeatAck(HeartbeatAck {
            nonce: 1,
            server_time: 0,
        });
        let mut bytes = rmp_serde::to_vec_named(&msg).unwrap();
        assert!(decode(&bytes).is_ok());
        bytes.push(0);
        assert!(decode(&bytes).is_err());
        assert!(decode(&vec![0; 524289]).is_err());
    }
}
