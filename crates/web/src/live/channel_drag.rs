//! Dragging channels to a new place in a server's channel list
//! (`docs/features/channel-drag.md`). The gesture and its drop marker are in
//! `public/nav.js`; this turns a drop into `ReorderChannels`, shows the new
//! order at once and puts the list back if the server refuses.
use super::*;

thread_local! {
    /// Moves waiting for the server's answer: what was sent, and the same
    /// channels as they were before.
    static WAITING: RefCell<Vec<(Vec<ChannelPlacement>, Vec<ChannelPlacement>)>> =
        const { RefCell::new(Vec::new()) };
}
/// More moves than this waiting at once only lose the oldest one's undo.
const MAX_WAITING: usize = 16;

/// What a manager's row carries for `nav.js`: `<id>,<parent id or 0>,<kind>`,
/// kind `c` for a category, `f` for a forum (which holds posts, not channels)
/// and `t` for any other channel. The parent is the one the row is shown under.
pub(super) fn handle(a: &App, c: &ChannelSummary) -> String {
    let parent = c
        .parent_id
        .filter(|p| a.model.channels.contains_key(p))
        .map_or(0, |p| p.0);
    let kind = match c.kind {
        ChannelKind::Category => 'c',
        ChannelKind::Forum => 'f',
        _ => 't',
    };
    format!("{},{parent},{kind}", c.id.0)
}

/// The same check the Move Channel page makes (`ui::settings::can`); the server
/// also checks the overwrites of each channel moved and of where it goes.
fn manages(a: &App) -> bool {
    a.guild
        .and_then(|g| a.model.guilds.get(&g))
        .is_some_and(|g| {
            g.my_role == MemberRole::Owner
                || g.my_permissions
                    .intersects(Permissions::ADMINISTRATOR | Permissions::MANAGE_CHANNELS)
        })
}

fn apply(channels: &mut BTreeMap<Id, ChannelSummary>, placements: &[ChannelPlacement]) {
    for p in placements {
        if let Some(c) = channels.get_mut(&p.channel_id) {
            c.parent_id = p.parent_id;
            c.position = p.position;
        }
    }
}

/// `chan-drop:<moving>,<parent or 0>,<before or 0>` from `nav.js` (an empty
/// argument only loads this module, when a drag starts).
#[lazy]
pub(super) fn drop_channel(app: Shared, arg: String, epoch: u64) {
    let mut a = app.borrow_mut();
    if a.epoch != epoch || !manages(&a) {
        return;
    }
    let Some(guild) = a.guild else { return };
    let mut parts = arg.split(',');
    let (Some(moving), Some(parent), Some(before)) = (
        parts.next().and_then(id),
        parts.next().map(id),
        parts.next().map(id),
    ) else {
        return;
    };
    let Some(placements) =
        crate::channel_order::moved(&a.model.channels, guild, moving, parent, before)
    else {
        return;
    };
    let before: Vec<_> = placements
        .iter()
        .filter_map(|p| a.model.channels.get(&p.channel_id))
        .map(|c| ChannelPlacement {
            channel_id: c.id,
            parent_id: c.parent_id,
            position: c.position,
        })
        .collect();
    let queued = a.pending.len();
    a.command(Command::ReorderChannels {
        guild_id: guild,
        placements: placements.clone(),
    });
    // Not sent (reconnecting, or too much waiting): nothing changes.
    if a.pending.len() == queued {
        return;
    }
    apply(&mut a.model.channels, &placements);
    WAITING.with_borrow_mut(|w| {
        if w.len() >= MAX_WAITING {
            w.remove(0);
        }
        w.push((placements, before));
    });
    a.touch();
}

/// The server answered a `ReorderChannels`. When it refused one sent by a
/// drag, every channel that move changed and nothing has moved since goes
/// back where it was. Returns the message to show, for a refusal.
pub(super) fn settled(
    a: &mut App,
    operation: Option<&Command>,
    refused: Option<ErrorCode>,
) -> Option<String> {
    let Some(Command::ReorderChannels { placements, .. }) = operation else {
        return None;
    };
    let undo = WAITING.with_borrow_mut(|w| {
        let i = w.iter().position(|(sent, _)| sent == placements)?;
        Some(w.remove(i).1)
    });
    let code = refused?;
    if let Some(undo) = undo {
        let untouched: Vec<_> = undo
            .into_iter()
            .filter(|old| {
                let sent = placements.iter().find(|p| p.channel_id == old.channel_id);
                a.model.channels.get(&old.channel_id).is_some_and(|c| {
                    sent.is_some_and(|p| p.parent_id == c.parent_id && p.position == c.position)
                })
            })
            .collect();
        apply(&mut a.model.channels, &untouched);
    }
    Some(
        match code {
            ErrorCode::Forbidden => {
                "You don’t have permission to move that channel there, so it’s back where it was."
            }
            ErrorCode::RateLimited => "Too many moves at once. Wait a moment and try again.",
            _ => "That channel couldn’t be moved there, so it’s back where it was.",
        }
        .into(),
    )
}
