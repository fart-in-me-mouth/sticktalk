//! Pure rules for moving a server channel, shared by the drag and drop in the
//! channel list and the Move Channel page: where the channel lands and the
//! complete placement list `ReorderChannels` takes. Native tests below.
use std::collections::BTreeMap;
use sticktalk_proto::*;

/// How the sidebar orders siblings (`ui::channel_tree`): at every level the
/// channels come before the categories, each group by position.
fn key(c: &ChannelSummary) -> (bool, u16, Id) {
    (c.kind == ChannelKind::Category, c.position, c.id)
}

/// Every channel of `guild` with its placement after `moving` goes into
/// `parent` (`None`: the top level), just before its new sibling `before`, or,
/// with `before: None`, after the last sibling of its own sort. Because the
/// channels are listed before the categories, a channel always lands among the
/// channels and a category among the categories. Everything nested in `moving`
/// keeps its place inside it. Threads are not listed (the server lists them
/// under their channel, not as channels).
///
/// `None` when the move is not allowed (into itself or anything inside it, into
/// a forum, a parent or sibling that is not one of this server's channels, a
/// `before` of the other sort) or changes nothing.
pub fn moved(
    channels: &BTreeMap<Id, ChannelSummary>,
    guild: Id,
    moving: Id,
    parent: Option<Id>,
    before: Option<Id>,
) -> Option<Vec<ChannelPlacement>> {
    let listed = |c: &&ChannelSummary| c.guild_id == Some(guild) && c.kind != ChannelKind::Thread;
    let x = channels.get(&moving).filter(listed)?;
    if let Some(p) = parent {
        let p = channels.get(&p).filter(listed)?;
        // A forum holds posts, not channels.
        if p.kind == ChannelKind::Forum {
            return None;
        }
        // Not inside itself, however deep.
        let mut at = Some(p.id);
        for _ in 0..=channels.len() {
            match at {
                Some(v) if v == moving => return None,
                Some(v) => at = channels.get(&v).and_then(|c| c.parent_id),
                None => break,
            }
        }
    }
    let category = x.kind == ChannelKind::Category;
    let mut sets: BTreeMap<Option<Id>, Vec<&ChannelSummary>> = BTreeMap::new();
    for c in channels.values().filter(listed) {
        if c.id != moving {
            sets.entry(c.parent_id).or_default().push(c);
        }
    }
    for set in sets.values_mut() {
        set.sort_by_key(|c| key(c));
    }
    let target = sets.entry(parent).or_default();
    let at = match before {
        Some(b) => {
            let i = target.iter().position(|c| c.id == b)?;
            if (target[i].kind == ChannelKind::Category) != category {
                return None;
            }
            i
        }
        None if category => target.len(),
        None => target
            .iter()
            .filter(|c| c.kind != ChannelKind::Category)
            .count(),
    };
    target.insert(at, x);
    // The set it lands in and the one it leaves are numbered again from 0, in
    // the order shown; every other set keeps its positions.
    let mut out = Vec::with_capacity(channels.len());
    let mut changed = false;
    for (p, set) in sets {
        let renumber = p == parent || p == x.parent_id;
        for (i, c) in set.into_iter().enumerate() {
            let position = if renumber { i as u16 } else { c.position };
            let parent_id = if c.id == moving { parent } else { c.parent_id };
            changed |= position != c.position || parent_id != c.parent_id;
            out.push(ChannelPlacement {
                channel_id: c.id,
                parent_id,
                position,
            });
        }
    }
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(id: u64, kind: ChannelKind, parent: Option<u64>, position: u16) -> ChannelSummary {
        ChannelSummary {
            id: Id(id),
            kind,
            guild_id: Some(Id(1)),
            parent_id: parent.map(Id),
            name: Some(format!("c{id}")),
            position,
            recipient_ids: vec![],
            owner_id: None,
        }
    }
    fn list(v: Vec<ChannelSummary>) -> BTreeMap<Id, ChannelSummary> {
        v.into_iter().map(|c| (c.id, c)).collect()
    }
    /// Apply placements and list each parent's children in sidebar order.
    fn shown(
        channels: &BTreeMap<Id, ChannelSummary>,
        placements: &[ChannelPlacement],
    ) -> BTreeMap<Option<u64>, Vec<u64>> {
        let mut after = channels.clone();
        for p in placements {
            let c = after.get_mut(&p.channel_id).unwrap();
            c.parent_id = p.parent_id;
            c.position = p.position;
        }
        let mut sets: BTreeMap<Option<u64>, Vec<&ChannelSummary>> = BTreeMap::new();
        for c in after
            .values()
            .filter(|c| c.kind != ChannelKind::Thread && c.guild_id == Some(Id(1)))
        {
            sets.entry(c.parent_id.map(|p| p.0)).or_default().push(c);
        }
        sets.into_iter()
            .map(|(p, mut v)| {
                v.sort_by_key(|c| key(c));
                (p, v.into_iter().map(|c| c.id.0).collect())
            })
            .collect()
    }
    fn fixture() -> BTreeMap<Id, ChannelSummary> {
        use ChannelKind::*;
        list(vec![
            ch(10, Text, None, 0),
            ch(11, Voice, None, 1),
            ch(12, Text, None, 2),
            ch(20, Category, None, 3),
            ch(21, Text, Some(20), 0),
            ch(22, Voice, Some(20), 1),
            ch(30, Category, None, 4),
            ch(31, Forum, Some(30), 0),
            ch(40, Thread, Some(10), 0),
            ChannelSummary {
                guild_id: Some(Id(2)),
                ..ch(50, Text, None, 0)
            },
        ])
    }

    #[test]
    fn reorders_siblings_before_a_channel_or_after_the_last() {
        let c = fixture();
        let p = moved(&c, Id(1), Id(12), None, Some(Id(10))).unwrap();
        assert_eq!(shown(&c, &p)[&None], [12, 10, 11, 20, 30]);
        // Voice above text.
        let p = moved(&c, Id(1), Id(11), None, Some(Id(10))).unwrap();
        assert_eq!(shown(&c, &p)[&None], [11, 10, 12, 20, 30]);
        // The end of the channels is before the categories.
        let p = moved(&c, Id(1), Id(10), None, None).unwrap();
        assert_eq!(shown(&c, &p)[&None], [11, 12, 10, 20, 30]);
        // Categories go among the categories.
        let p = moved(&c, Id(1), Id(30), None, Some(Id(20))).unwrap();
        assert_eq!(shown(&c, &p)[&None], [10, 11, 12, 30, 20]);
        // Every channel of the server is listed, threads and other servers are not.
        let ids: Vec<u64> = p.iter().map(|p| p.channel_id.0).collect();
        assert_eq!(ids.len(), 8);
        assert!(!ids.contains(&40) && !ids.contains(&50));
    }

    #[test]
    fn nests_and_takes_back_out_with_what_is_inside() {
        let c = fixture();
        // Into a category, at the end of its channels.
        let p = moved(&c, Id(1), Id(10), Some(Id(20)), None).unwrap();
        let s = shown(&c, &p);
        assert_eq!(s[&None], [11, 12, 20, 30]);
        assert_eq!(s[&Some(20)], [21, 22, 10]);
        // Into a text channel; its thread stays where it is.
        let p = moved(&c, Id(1), Id(12), Some(Id(10)), None).unwrap();
        assert_eq!(shown(&c, &p)[&Some(10)], [12]);
        assert!(!p.iter().any(|p| p.channel_id == Id(40)));
        // A category inside a category takes its children along.
        let p = moved(&c, Id(1), Id(20), Some(Id(30)), None).unwrap();
        let s = shown(&c, &p);
        assert_eq!(s[&Some(30)], [31, 20]);
        assert_eq!(s[&Some(20)], [21, 22]);
        // Out to the top level, before a channel there.
        let p = moved(&c, Id(1), Id(21), None, Some(Id(11))).unwrap();
        let s = shown(&c, &p);
        assert_eq!(s[&None], [10, 21, 11, 12, 20, 30]);
        assert_eq!(s[&Some(20)], [22]);
        assert!(p.contains(&ChannelPlacement {
            channel_id: Id(22),
            parent_id: Some(Id(20)),
            position: 0
        }));
    }

    #[test]
    fn refuses_loops_forums_strangers_and_no_ops() {
        let c = fixture();
        assert_eq!(moved(&c, Id(1), Id(20), Some(Id(20)), None), None);
        assert_eq!(moved(&c, Id(1), Id(20), Some(Id(21)), None), None);
        assert_eq!(moved(&c, Id(1), Id(10), Some(Id(31)), None), None);
        assert_eq!(moved(&c, Id(1), Id(10), Some(Id(50)), None), None);
        assert_eq!(moved(&c, Id(1), Id(50), None, None), None);
        assert_eq!(moved(&c, Id(1), Id(40), None, None), None);
        assert_eq!(moved(&c, Id(1), Id(10), None, Some(Id(999))), None);
        // A channel cannot be put before a category, nor a category before a channel.
        assert_eq!(moved(&c, Id(1), Id(10), None, Some(Id(20))), None);
        assert_eq!(moved(&c, Id(1), Id(20), None, Some(Id(10))), None);
        // Where it already is.
        assert_eq!(moved(&c, Id(1), Id(11), None, Some(Id(12))), None);
        assert_eq!(moved(&c, Id(1), Id(12), None, None), None);
        assert_eq!(moved(&c, Id(1), Id(30), None, None), None);
    }

    #[test]
    fn gaps_in_positions_are_closed_where_it_lands_and_where_it_left() {
        use ChannelKind::*;
        let c = list(vec![
            ch(1, Text, None, 3),
            ch(2, Text, None, 7),
            ch(3, Category, None, 9),
            ch(4, Text, Some(3), 5),
        ]);
        let p = moved(&c, Id(1), Id(2), Some(Id(3)), None).unwrap();
        let at = |id| p.iter().find(|p| p.channel_id == Id(id)).unwrap().position;
        assert_eq!((at(1), at(3), at(4), at(2)), (0, 1, 0, 1));
    }
}
