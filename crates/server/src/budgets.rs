//! The per-account budgets that WebSocket commands draw on (the full table of
//! every budget is in `docs/features/integration.md`, "Rate-limit budgets").
//!
//! A command the person asked for (send, react, save a role, create a
//! channel, ...) draws on `cmd:{user}`. Two kinds of command the app sends by
//! itself have budgets of their own, because they come in bursts nobody
//! chose and used to starve the person's own actions:
//!
//! - **Session setup**: after every Ready (sign-in, reload, reconnect) the web
//!   app opts its stream in to each feature's events (eight `Subscribe`s) and
//!   names its device. That was 9 of the 20-command burst on every reload.
//! - **Read markers** (`MarkRead`): one when a message lands in front of you.
//!   The web app coalesces them (one a second at most per account), but a
//!   busy conversation and several open devices still add up.
//!
//! Both stay bounded: each is per account, cheap (a flag on the stream, or one
//! read-state row), and refused with `RateLimited` past its own limit.
use crate::{error::Result, state::Core};
use sticktalk_proto::*;

/// What a person does: 120 a minute, bursts of 20.
pub(crate) const ACTIONS: (f64, f64) = (120., 20.);
/// Messages (on top of `ACTIONS`): 60 a minute, bursts of 10.
pub(crate) const SENDS: (f64, f64) = (60., 10.);
/// Opting a stream in to events after a Ready: about six Readies back to
/// back, then one every five seconds.
pub(crate) const SESSION: (f64, f64) = (120., 60.);
/// Read markers.
pub(crate) const READS: (f64, f64) = (240., 60.);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Budget {
    Actions,
    Session,
    Reads,
}

/// Which budget a durable command draws on.
pub(crate) fn of(command: &Command) -> Budget {
    match command {
        Command::Content(ContentCommand::Subscribe)
        | Command::Threads(ThreadCommand::Subscribe)
        | Command::Expression(ExpressionCommand::Subscribe)
        | Command::Privacy(PrivacyCommand::Subscribe)
        | Command::Community(CommunityCommand::Subscribe)
        | Command::Schedule(ScheduleCommand::Subscribe)
        | Command::Appearance(AppearanceCommand::Subscribe)
        | Command::Launch(
            LaunchCommand::Moderation(ModerationCommand::Subscribe)
            | LaunchCommand::UpdateDevice { .. },
        ) => Budget::Session,
        Command::MarkRead { .. } => Budget::Reads,
        _ => Budget::Actions,
    }
}

/// Charge a durable command to its budget (and a message to `send:` too).
pub(crate) fn charge(core: &mut Core, u: Id, command: &Command) -> Result<()> {
    let (key, (per_minute, burst)) = match of(command) {
        Budget::Actions => ("cmd", ACTIONS),
        Budget::Session => ("session", SESSION),
        Budget::Reads => ("read", READS),
    };
    core.rate(format!("{key}:{}", u.0), per_minute, burst)?;
    if matches!(command, Command::SendMessage(_)) {
        core.rate(format!("send:{}", u.0), SENDS.0, SENDS.1)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn setup_and_read_markers_do_not_spend_the_actions_budget() {
        let (_dir, app) = crate::audit_tests::fixture().await;
        let mut c = app.0.core.lock().await;
        c.limits.clear();
        let c = &mut *c;
        let u = Id(7);
        // Three reloads back to back: 27 setup commands, then a busy channel's
        // read markers.
        for _ in 0..3 {
            for cmd in [
                Command::Content(ContentCommand::Subscribe),
                Command::Threads(ThreadCommand::Subscribe),
                Command::Expression(ExpressionCommand::Subscribe),
                Command::Privacy(PrivacyCommand::Subscribe),
                Command::Community(CommunityCommand::Subscribe),
                Command::Schedule(ScheduleCommand::Subscribe),
                Command::Appearance(AppearanceCommand::Subscribe),
                Command::Launch(LaunchCommand::Moderation(ModerationCommand::Subscribe)),
                Command::Launch(LaunchCommand::UpdateDevice {
                    device_name: "Chrome on Windows".into(),
                    platform: "web".into(),
                }),
            ] {
                assert_eq!(of(&cmd), Budget::Session);
                charge(c, u, &cmd).unwrap();
            }
        }
        for m in 0..40 {
            charge(
                c,
                u,
                &Command::MarkRead {
                    channel_id: Id(1),
                    message_id: Id(m),
                },
            )
            .unwrap();
        }
        // The person still has their whole burst for what they do.
        for _ in 0..ACTIONS.1 as usize {
            charge(c, u, &Command::LeaveGuild { guild_id: Id(2) }).unwrap();
        }
        let refused = charge(c, u, &Command::LeaveGuild { guild_id: Id(2) }).unwrap_err();
        assert_eq!(refused.0.code, ErrorCode::RateLimited);
    }

    #[tokio::test]
    async fn every_budget_still_refuses_a_flood() {
        let (_dir, app) = crate::audit_tests::fixture().await;
        let mut guard = app.0.core.lock().await;
        guard.limits.clear();
        let c = &mut *guard;
        let u = Id(8);
        let flood = |c: &mut Core, cmd: Command| {
            (0..1000)
                .map(|_| charge(c, u, &cmd))
                .take_while(|r| r.is_ok())
                .count()
        };
        assert_eq!(
            flood(c, Command::Privacy(PrivacyCommand::Subscribe)),
            SESSION.1 as usize
        );
        assert_eq!(
            flood(
                c,
                Command::MarkRead {
                    channel_id: Id(1),
                    message_id: Id(1)
                }
            ),
            READS.1 as usize
        );
    }
}
