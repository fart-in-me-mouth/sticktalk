//! Pure navigation rules shared by the live client and its native tests: how
//! the server rail is laid out and rearranged, how Favorites are reordered, how
//! search terms are highlighted and when reminder presets fall due.
use sticktalk_proto::*;

/// One place on the rail, with its folder's servers when it is a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    Guild(Id),
    Folder(ServerFolder),
}
/// Something being moved on the rail, or the thing it is dropped on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Guild(Id),
    Folder(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Before,
    After,
    Into,
}
impl Item {
    /// `g<id>` or `f<id>`, as the rail's drag handles name them.
    pub fn parse(s: &str) -> Option<Self> {
        let n = s.get(1..)?.parse::<u64>().ok().filter(|n| *n > 0)?;
        match s.as_bytes().first()? {
            b'g' if n <= i64::MAX as u64 => Some(Self::Guild(Id(n))),
            b'f' => u32::try_from(n).ok().map(Self::Folder),
            _ => None,
        }
    }
}
impl Place {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "before" => Some(Self::Before),
            "after" => Some(Self::After),
            "into" => Some(Self::Into),
            _ => None,
        }
    }
}
pub const FOLDER_COLORS: [u32; 6] = [0x3ddc97, 0x5b8def, 0xf2b84b, 0xf25f5c, 0xb07cf2, 0x8a8a94];

/// The rail as shown: the saved order, then any server it does not mention
/// (in `guilds` order), with folders holding only servers still joined.
pub fn layout(s: &NavSettings, guilds: &[Id]) -> Vec<Slot> {
    let mut out = vec![];
    let mut placed = std::collections::BTreeSet::new();
    let folders: Vec<ServerFolder> = s
        .folders
        .iter()
        .map(|f| ServerFolder {
            guild_ids: f
                .guild_ids
                .iter()
                .copied()
                .filter(|g| guilds.contains(g))
                .collect(),
            ..f.clone()
        })
        .filter(|f| !f.guild_ids.is_empty())
        .collect();
    let mut used_folders = std::collections::BTreeSet::new();
    for item in &s.rail {
        match item {
            RailItem::Guild(g) if guilds.contains(g) && !placed.contains(g) => {
                if folders.iter().any(|f| f.guild_ids.contains(g)) {
                    continue;
                }
                placed.insert(*g);
                out.push(Slot::Guild(*g));
            }
            RailItem::Folder(id) => {
                if let Some(f) = folders.iter().find(|f| f.id == *id) {
                    if used_folders.insert(*id) {
                        placed.extend(f.guild_ids.iter().copied());
                        out.push(Slot::Folder(f.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    for f in &folders {
        if used_folders.insert(f.id) {
            placed.extend(f.guild_ids.iter().copied());
            out.push(Slot::Folder(f.clone()));
        }
    }
    for g in guilds {
        if placed.insert(*g) {
            out.push(Slot::Guild(*g));
        }
    }
    out
}
pub fn to_rail(layout: &[Slot]) -> SetRail {
    SetRail {
        folders: layout
            .iter()
            .filter_map(|s| match s {
                Slot::Folder(f) => Some(f.clone()),
                Slot::Guild(_) => None,
            })
            .collect(),
        order: layout
            .iter()
            .map(|s| match s {
                Slot::Guild(g) => RailItem::Guild(*g),
                Slot::Folder(f) => RailItem::Folder(f.id),
            })
            .collect(),
    }
}
/// Where a server is: its slot index and, inside a folder, its index there.
fn find_guild(layout: &[Slot], g: Id) -> Option<(usize, Option<usize>)> {
    layout.iter().enumerate().find_map(|(i, s)| match s {
        Slot::Guild(v) if *v == g => Some((i, None)),
        Slot::Folder(f) => f
            .guild_ids
            .iter()
            .position(|v| *v == g)
            .map(|j| (i, Some(j))),
        _ => None,
    })
}
fn find_folder(layout: &[Slot], id: u32) -> Option<usize> {
    layout
        .iter()
        .position(|s| matches!(s, Slot::Folder(f) if f.id == id))
}
/// Take a server out of wherever it is; a folder left empty goes away.
fn take_guild(layout: &mut Vec<Slot>, g: Id) -> bool {
    let Some((i, inner)) = find_guild(layout, g) else {
        return false;
    };
    match inner {
        None => {
            layout.remove(i);
        }
        Some(j) => {
            if let Slot::Folder(f) = &mut layout[i] {
                f.guild_ids.remove(j);
                if f.guild_ids.is_empty() {
                    layout.remove(i);
                }
            }
        }
    }
    true
}
fn new_folder_id(layout: &[Slot]) -> u32 {
    layout
        .iter()
        .filter_map(|s| match s {
            Slot::Folder(f) => Some(f.id),
            Slot::Guild(_) => None,
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .max(1)
}
fn folder_count(layout: &[Slot]) -> usize {
    layout
        .iter()
        .filter(|s| matches!(s, Slot::Folder(_)))
        .count()
}
/// The rail after dropping `src` on `dst`. None when nothing would change or
/// the move is not allowed (a folder cannot go inside another).
pub fn drop(layout: &[Slot], src: Item, dst: Item, place: Place) -> Option<Vec<Slot>> {
    if src == dst {
        return None;
    }
    let mut out = layout.to_vec();
    match src {
        Item::Guild(g) => {
            find_guild(&out, g)?;
            match dst {
                Item::Guild(h) => {
                    let (_, inner) = find_guild(&out, h)?;
                    take_guild(&mut out, g);
                    let (i, inner_now) = find_guild(&out, h)?;
                    match (inner.is_some(), place) {
                        (false, Place::Into) => {
                            if folder_count(&out) >= MAX_GUILDS_PER_USER {
                                return None;
                            }
                            let id = new_folder_id(&out);
                            out[i] = Slot::Folder(ServerFolder {
                                id,
                                name: "New Folder".into(),
                                color: FOLDER_COLORS[(id as usize) % FOLDER_COLORS.len()],
                                collapsed: false,
                                guild_ids: vec![h, g],
                            });
                        }
                        (false, place) => {
                            let at = if place == Place::After { i + 1 } else { i };
                            out.insert(at, Slot::Guild(g));
                        }
                        (true, place) => {
                            if let Slot::Folder(f) = &mut out[i] {
                                let j = inner_now.unwrap_or(0);
                                let at = if place == Place::Before { j } else { j + 1 };
                                f.guild_ids.insert(at.min(f.guild_ids.len()), g);
                            }
                        }
                    }
                }
                Item::Folder(id) => {
                    find_folder(&out, id)?;
                    take_guild(&mut out, g);
                    let i = find_folder(&out, id)?;
                    match place {
                        Place::Into => {
                            if let Slot::Folder(f) = &mut out[i] {
                                f.guild_ids.push(g);
                            }
                        }
                        Place::Before => out.insert(i, Slot::Guild(g)),
                        Place::After => out.insert(i + 1, Slot::Guild(g)),
                    }
                }
            }
        }
        Item::Folder(id) => {
            let from = find_folder(&out, id)?;
            // A folder goes beside whatever holds the target, never inside it.
            let target = match dst {
                Item::Guild(h) => find_guild(&out, h)?.0,
                Item::Folder(other) => find_folder(&out, other)?,
            };
            if target == from {
                return None;
            }
            let slot = out.remove(from);
            let target = if target > from { target - 1 } else { target };
            let at = if place == Place::After {
                target + 1
            } else {
                target
            };
            out.insert(at.min(out.len()), slot);
        }
    }
    (out != layout).then_some(out)
}
/// Move a server or folder one step earlier (-1) or later (+1). A server in a
/// folder moves within it.
pub fn step(layout: &[Slot], item: Item, delta: i32) -> Option<Vec<Slot>> {
    let mut out = layout.to_vec();
    let swap = |len: usize, i: usize| -> Option<usize> {
        let j = i as i64 + i64::from(delta);
        (0..len as i64).contains(&j).then_some(j as usize)
    };
    match item {
        Item::Guild(g) => match find_guild(&out, g)? {
            (i, None) => {
                let j = swap(out.len(), i)?;
                out.swap(i, j);
            }
            (i, Some(k)) => {
                if let Slot::Folder(f) = &mut out[i] {
                    let j = swap(f.guild_ids.len(), k)?;
                    f.guild_ids.swap(k, j);
                }
            }
        },
        Item::Folder(id) => {
            let i = find_folder(&out, id)?;
            let j = swap(out.len(), i)?;
            out.swap(i, j);
        }
    }
    Some(out)
}
/// Put a server in a folder (None: a new folder of its own, where it is now).
pub fn into_folder(layout: &[Slot], g: Id, folder: Option<u32>) -> Option<Vec<Slot>> {
    let mut out = layout.to_vec();
    match folder {
        Some(id) => {
            if find_guild(&out, g).is_some_and(|(i, inner)| {
                inner.is_some() && matches!(&out[i], Slot::Folder(f) if f.id == id)
            }) {
                return None;
            }
            find_folder(&out, id)?;
            take_guild(&mut out, g);
            let i = find_folder(&out, id)?;
            if let Slot::Folder(f) = &mut out[i] {
                f.guild_ids.push(g);
            }
        }
        None => {
            if folder_count(&out) >= MAX_GUILDS_PER_USER {
                return None;
            }
            let (i, _) = find_guild(&out, g)?;
            let id = new_folder_id(&out);
            take_guild(&mut out, g);
            let folder = Slot::Folder(ServerFolder {
                id,
                name: "New Folder".into(),
                color: FOLDER_COLORS[(id as usize) % FOLDER_COLORS.len()],
                collapsed: false,
                guild_ids: vec![g],
            });
            out.insert(i.min(out.len()), folder);
        }
    }
    Some(out)
}
/// Take a server out of its folder and put it right after that folder.
pub fn out_of_folder(layout: &[Slot], g: Id) -> Option<Vec<Slot>> {
    let mut out = layout.to_vec();
    let (i, Some(_)) = find_guild(&out, g)? else {
        return None;
    };
    let emptied = matches!(&out[i], Slot::Folder(f) if f.guild_ids.len() == 1);
    take_guild(&mut out, g);
    out.insert(if emptied { i } else { i + 1 }, Slot::Guild(g));
    Some(out)
}
/// Dissolve a folder, leaving its servers in its place.
pub fn ungroup(layout: &[Slot], id: u32) -> Option<Vec<Slot>> {
    let mut out = layout.to_vec();
    let i = find_folder(&out, id)?;
    let Slot::Folder(f) = out.remove(i) else {
        return None;
    };
    for (k, g) in f.guild_ids.into_iter().enumerate() {
        out.insert(i + k, Slot::Guild(g));
    }
    Some(out)
}
pub fn edit_folder(
    layout: &[Slot],
    id: u32,
    f: impl FnOnce(&mut ServerFolder),
) -> Option<Vec<Slot>> {
    let mut out = layout.to_vec();
    let i = find_folder(&out, id)?;
    if let Slot::Folder(folder) = &mut out[i] {
        f(folder);
    }
    Some(out)
}

/// Favorites after moving one entry a step within its own category.
pub fn favorite_step(
    list: &[FavoriteEntry],
    channel: Id,
    delta: i32,
) -> Option<Vec<FavoriteEntry>> {
    let i = list.iter().position(|f| f.channel_id == channel)?;
    let category = list[i].category_id;
    let mut peers: Vec<usize> = list
        .iter()
        .enumerate()
        .filter(|(_, f)| f.category_id == category)
        .map(|(i, _)| i)
        .collect();
    let at = peers.iter().position(|p| *p == i)?;
    let to = at as i64 + i64::from(delta);
    if !(0..peers.len() as i64).contains(&to) {
        return None;
    }
    let mut out = list.to_vec();
    out.swap(i, peers[to as usize]);
    peers.clear();
    Some(out)
}

/// Up to `max` characters of a message on one line, spoilers hidden.
pub fn excerpt(text: &str, max: usize) -> String {
    let parts = text.split("||").collect::<Vec<_>>();
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i % 2 == 1 && i + 1 < parts.len() {
            out.push_str("(spoiler)");
        } else {
            if i % 2 == 1 {
                out.push_str("||");
            }
            out.push_str(part);
        }
    }
    let line = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > max {
        line.chars().take(max).chain(std::iter::once('…')).collect()
    } else {
        line
    }
}
/// Text split into (part, matched) pieces for each search term, ignoring case.
pub fn highlight(text: &str, terms: &[String]) -> Vec<(String, bool)> {
    let lower = text.to_lowercase();
    // Lowercasing can change byte lengths; only highlight when it does not.
    if lower.len() != text.len() || terms.is_empty() {
        return vec![(text.to_owned(), false)];
    }
    let terms: Vec<String> = terms
        .iter()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty() && t.len() <= 200)
        .collect();
    let mut out = vec![];
    let mut at = 0;
    let mut plain = 0;
    while at < text.len() {
        let hit = terms
            .iter()
            .filter(|t| lower[at..].starts_with(t.as_str()))
            .map(|t| t.len())
            .max();
        match hit {
            Some(n) if text.is_char_boundary(at + n) => {
                if plain < at {
                    out.push((text[plain..at].to_owned(), false));
                }
                out.push((text[at..at + n].to_owned(), true));
                at += n;
                plain = at;
            }
            _ => {
                at += text[at..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    if plain < text.len() {
        out.push((text[plain..].to_owned(), false));
    }
    out
}

/// Reminder presets relative to `now` (local wall-clock pieces supplied by the
/// caller): minutes from now, or "tomorrow at 9" computed from `minutes_today`.
pub fn reminder_at(now: u64, preset: &str, minutes_today: u64) -> Option<u64> {
    let min = 60_000;
    match preset {
        "20m" => Some(now + 20 * min),
        "1h" => Some(now + 60 * min),
        "3h" => Some(now + 180 * min),
        "tomorrow" => Some(now + (24 * 60 - minutes_today.min(24 * 60) + 9 * 60) * min),
        "week" => Some(now + 7 * 24 * 60 * min),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn folder(id: u32, guilds: &[u64]) -> ServerFolder {
        ServerFolder {
            id,
            name: "F".into(),
            color: 0,
            collapsed: false,
            guild_ids: guilds.iter().map(|g| Id(*g)).collect(),
        }
    }
    fn names(l: &[Slot]) -> String {
        l.iter()
            .map(|s| match s {
                Slot::Guild(g) => g.0.to_string(),
                Slot::Folder(f) => format!(
                    "[{}]",
                    f.guild_ids
                        .iter()
                        .map(|g| g.0.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    #[test]
    fn layout_follows_saved_order_then_appends_new_servers_and_drops_left_ones() {
        let s = NavSettings {
            folders: vec![folder(1, &[3, 9])],
            rail: vec![
                RailItem::Guild(Id(2)),
                RailItem::Folder(1),
                RailItem::Guild(Id(8)),
            ],
            ..Default::default()
        };
        let l = layout(&s, &[1, 2, 3, 4].map(Id));
        assert_eq!(names(&l), "2 [3] 1 4");
        assert_eq!(to_rail(&l).folders[0].guild_ids, [Id(3)]);
    }
    #[test]
    fn dropping_creates_fills_reorders_and_empties_folders() {
        let l = layout(&NavSettings::default(), &[1, 2, 3, 4].map(Id));
        let l = drop(&l, Item::Guild(Id(3)), Item::Guild(Id(1)), Place::Into).unwrap();
        assert_eq!(names(&l), "[1,3] 2 4");
        let l = drop(&l, Item::Guild(Id(4)), Item::Guild(Id(1)), Place::Before).unwrap();
        assert_eq!(names(&l), "[4,1,3] 2");
        let l = drop(&l, Item::Guild(Id(2)), Item::Folder(1), Place::Before).unwrap();
        assert_eq!(names(&l), "2 [4,1,3]");
        let l = drop(&l, Item::Guild(Id(1)), Item::Guild(Id(2)), Place::After).unwrap();
        assert_eq!(names(&l), "2 1 [4,3]");
        let l = drop(&l, Item::Folder(1), Item::Guild(Id(2)), Place::Before).unwrap();
        assert_eq!(names(&l), "[4,3] 2 1");
        assert!(drop(&l, Item::Folder(1), Item::Guild(Id(4)), Place::Into).is_none());
        let l = drop(&l, Item::Guild(Id(4)), Item::Guild(Id(1)), Place::After).unwrap();
        let l = drop(&l, Item::Guild(Id(3)), Item::Guild(Id(1)), Place::After).unwrap();
        assert_eq!(names(&l), "2 1 3 4", "the empty folder is gone");
        assert!(drop(&l, Item::Guild(Id(3)), Item::Guild(Id(3)), Place::Into).is_none());
    }
    #[test]
    fn menu_paths_step_group_and_ungroup() {
        let l = layout(&NavSettings::default(), &[1, 2, 3].map(Id));
        let l = into_folder(&l, Id(2), None).unwrap();
        assert_eq!(names(&l), "1 [2] 3");
        let id = match &l[1] {
            Slot::Folder(f) => f.id,
            _ => unreachable!(),
        };
        let l = into_folder(&l, Id(3), Some(id)).unwrap();
        assert_eq!(names(&l), "1 [2,3]");
        let l = step(&l, Item::Guild(Id(3)), -1).unwrap();
        assert_eq!(names(&l), "1 [3,2]");
        let l = step(&l, Item::Folder(id), -1).unwrap();
        assert_eq!(names(&l), "[3,2] 1");
        assert!(step(&l, Item::Folder(id), -1).is_none());
        let l = out_of_folder(&l, Id(3)).unwrap();
        assert_eq!(names(&l), "[2] 3 1");
        let l = ungroup(&l, id).unwrap();
        assert_eq!(names(&l), "2 3 1");
        let l = edit_folder(&into_folder(&l, Id(1), None).unwrap(), 1, |f| {
            f.name = "Work".into()
        })
        .unwrap();
        assert!(matches!(&l[2], Slot::Folder(f) if f.name == "Work"));
    }
    #[test]
    fn rail_handles_parse_strictly() {
        assert_eq!(Item::parse("g12"), Some(Item::Guild(Id(12))));
        assert_eq!(Item::parse("f3"), Some(Item::Folder(3)));
        for bad in ["", "g", "g0", "x1", "f99999999999", "g-1"] {
            assert_eq!(Item::parse(bad), None, "{bad}");
        }
        assert_eq!(Place::parse("into"), Some(Place::Into));
        assert_eq!(Place::parse("in"), None);
    }
    #[test]
    fn favorites_step_within_their_category() {
        let e = |c: u64, k: Option<u32>| FavoriteEntry {
            channel_id: Id(c),
            category_id: k,
        };
        let list = vec![e(1, None), e(2, Some(1)), e(3, None), e(4, Some(1))];
        let moved = favorite_step(&list, Id(3), -1).unwrap();
        assert_eq!(
            moved.iter().map(|f| f.channel_id.0).collect::<Vec<_>>(),
            [3, 2, 1, 4]
        );
        assert!(favorite_step(&list, Id(4), 1).is_none());
    }
    #[test]
    fn highlight_marks_terms_case_insensitively_and_safely() {
        let terms = vec!["launch".to_string()];
        assert_eq!(
            highlight("Launch the launcher", &terms),
            vec![
                ("Launch".into(), true),
                (" the ".into(), false),
                ("launch".into(), true),
                ("er".into(), false)
            ]
        );
        assert_eq!(
            highlight("İstanbul", &terms),
            vec![("İstanbul".into(), false)]
        );
        assert_eq!(highlight("plain", &[]), vec![("plain".into(), false)]);
    }
    #[test]
    fn excerpts_hide_spoilers_and_stay_short() {
        assert_eq!(excerpt("a ||secret|| b", 50), "a (spoiler) b");
        assert_eq!(excerpt("abcdef", 3), "abc…");
        assert_eq!(
            excerpt(
                "line
one", 50
            ),
            "line one"
        );
    }
    #[test]
    fn reminder_presets() {
        assert_eq!(reminder_at(0, "1h", 0), Some(3_600_000));
        // At 22:00, tomorrow at 9 is 11 hours away.
        assert_eq!(reminder_at(0, "tomorrow", 22 * 60), Some(11 * 3_600_000));
        assert_eq!(reminder_at(0, "never", 0), None);
    }
}
