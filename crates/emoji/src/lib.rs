//! Offline Unicode Emoji 18.0 data. Picker entries exclude skin-tone variants;
//! those are available through [`apply_skin_tone`]. No runtime dependencies.
use std::borrow::Cow;
const POOL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pool.bin"));
const RECORDS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/records.bin"));
const TONES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tones.bin"));
pub const UNICODE_VERSION: &str = "18.0";
pub const TABLE_BYTES: usize = POOL.len() + RECORDS.len() + TONES.len();
pub const SKIN_TONES: [char; 5] = [
    '\u{1f3fb}',
    '\u{1f3fc}',
    '\u{1f3fd}',
    '\u{1f3fe}',
    '\u{1f3ff}',
];
const CATEGORIES: [&str; 8] = [
    "Smileys & People",
    "Nature",
    "Food",
    "Activities",
    "Travel",
    "Objects",
    "Symbols",
    "Flags",
];
pub fn categories() -> &'static [&'static str; 8] {
    &CATEGORIES
}
fn offset(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}
fn string(at: usize) -> &'static str {
    let tail = &POOL[at..];
    std::str::from_utf8(&tail[..tail.iter().position(|&b| b == 0).unwrap()]).unwrap()
}
/// A compact, copyable picker entry. Persist its canonical shortcode, not its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Emoji(u16);
impl Emoji {
    fn record(self) -> &'static [u8] {
        &RECORDS[self.0 as usize * 17..][..17]
    }
    pub fn emoji(self) -> &'static str {
        string(offset(self.record(), 0))
    }
    pub fn name(self) -> &'static str {
        string(offset(self.record(), 4))
    }
    /// English name words are the keyword source (no separate CLDR annotations).
    pub fn keywords(self) -> impl Iterator<Item = &'static str> {
        self.name()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
    }
    /// Bare aliases; wrap in colons for display or sending.
    pub fn shortcodes(self) -> impl Iterator<Item = &'static str> {
        string(offset(self.record(), 8)).split('|')
    }
    pub fn shortcode(self) -> &'static str {
        self.shortcodes().next().unwrap()
    }
    pub fn category(self) -> &'static str {
        CATEGORIES[self.record()[14] as usize]
    }
    pub fn version(self) -> (u8, u8) {
        (self.record()[15], self.record()[16])
    }
    fn tone_index(self) -> usize {
        u16::from_le_bytes(self.record()[12..14].try_into().unwrap()) as usize
    }
    pub fn supports_skin_tone(self) -> bool {
        self.tone_index() != 0
    }
}
pub fn all() -> impl ExactSizeIterator<Item = Emoji> {
    (0..(RECORDS.len() / 17) as u16).map(Emoji)
}
pub fn by_category(category: &str) -> impl Iterator<Item = Emoji> + '_ {
    all().filter(move |e| e.category() == category)
}
/// Exact alias, alias prefix, name prefix, then keyword prefix; stable Unicode order ties.
/// ASCII case-insensitive, whitespace-separated AND terms. Allocates at most `limit` entries.
pub fn search(query: &str, limit: usize) -> Vec<Emoji> {
    let query = query.trim().trim_matches(':');
    let mut best: Vec<(u8, Emoji)> = Vec::new();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    for e in all() {
        let mut score = 0;
        let mut matches = true;
        for term in query.split_whitespace() {
            let prefix = |s: &str| {
                s.get(..term.len())
                    .is_some_and(|p| p.eq_ignore_ascii_case(term))
            };
            let rank = if e.shortcodes().any(|s| s.eq_ignore_ascii_case(term)) {
                0
            } else if e.shortcodes().any(prefix) {
                1
            } else if prefix(e.name()) {
                2
            } else if e.keywords().any(prefix) {
                3
            } else {
                matches = false;
                break;
            };
            score = score.max(rank);
        }
        if matches {
            let at = best.partition_point(|(s, _)| *s <= score);
            if at < limit {
                best.insert(at, (score, e));
                if best.len() > limit {
                    best.pop();
                }
            }
        }
    }
    best.into_iter().map(|(_, e)| e).collect()
}
pub fn shortcode_to_emoji(shortcode: &str) -> Option<&'static str> {
    let code = shortcode
        .strip_prefix(':')
        .and_then(|s| s.strip_suffix(':'))
        .unwrap_or(shortcode);
    all()
        .find(|e| e.shortcodes().any(|s| s == code))
        .map(Emoji::emoji)
}
pub fn emoji_to_shortcode(glyph: &str) -> Option<&'static str> {
    all().find(|e| e.emoji() == glyph).map(Emoji::shortcode)
}
/// Select an official same-tone RGI sequence; unsupported input/modifiers return None.
/// Mixed-tone couples are intentionally not synthesized.
pub fn apply_skin_tone(glyph: &str, modifier: char) -> Option<&'static str> {
    let tone = SKIN_TONES.iter().position(|&c| c == modifier)?;
    let e = all().find(|e| e.emoji() == glyph)?;
    let index = e.tone_index().checked_sub(1)?;
    let result = string(offset(TONES, index * 20 + tone * 4));
    (!result.is_empty()).then_some(result)
}
/// Unknown aliases and unmatched colons are preserved. Returns borrowed text if unchanged.
pub fn replace_shortcodes(text: &str) -> Cow<'_, str> {
    let mut output: Option<String> = None;
    let mut copied = 0;
    let mut scan = 0;
    while let Some(start) = text[scan..].find(':').map(|i| i + scan) {
        let Some(end) = text[start + 1..].find(':').map(|i| i + start + 1) else {
            break;
        };
        if let Some(glyph) = shortcode_to_emoji(&text[start..=end]) {
            let out = output.get_or_insert_with(|| String::with_capacity(text.len()));
            out.push_str(&text[copied..start]);
            out.push_str(glyph);
            copied = end + 1;
            scan = end + 1;
        } else {
            scan = start + 1;
        }
    }
    match output {
        Some(mut s) => {
            s.push_str(&text[copied..]);
            Cow::Owned(s)
        }
        None => Cow::Borrowed(text),
    }
}
/// Persist these fields using the client's storage format. Time units are caller-defined.
#[derive(Clone, Debug, PartialEq)]
pub struct Usage {
    pub shortcode: String,
    pub weight: f64,
    pub updated_at: u64,
}
impl Usage {
    pub fn new(shortcode: String, now: u64) -> Self {
        Self {
            shortcode,
            weight: 0.0,
            updated_at: now,
        }
    }
    /// Clock rollback freezes decay. Invalid persisted weights contribute zero.
    pub fn score(&self, now: u64, half_life: u64) -> f64 {
        if !self.weight.is_finite() || self.weight < 0.0 {
            return 0.0;
        }
        self.weight
            * 2.0f64.powf(-(now.saturating_sub(self.updated_at) as f64) / half_life.max(1) as f64)
    }
    pub fn record(&mut self, now: u64, half_life: u64) {
        self.weight = self.score(now, half_life) + 1.0;
        self.updated_at = self.updated_at.max(now);
    }
}
/// Sort caller-owned, persisted counters in place. No timer or storage backend required.
pub fn rank_frequently_used(entries: &mut [Usage], now: u64, half_life: u64) {
    entries.sort_unstable_by(|a, b| {
        b.score(now, half_life)
            .total_cmp(&a.score(now, half_life))
            .then_with(|| a.shortcode.cmp(&b.shortcode))
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coverage() {
        assert!(all().len() > 1900);
        assert_eq!(categories().len(), 8);
        for category in categories() {
            assert!(by_category(category).next().is_some());
        }
        for e in all() {
            assert!(!e.name().is_empty());
            assert!(e.keywords().next().is_some());
            assert!(e.version().0 <= 18);
            assert_eq!(emoji_to_shortcode(e.emoji()), Some(e.shortcode()));
            for code in e.shortcodes() {
                assert_eq!(shortcode_to_emoji(code), Some(e.emoji()));
            }
        }
        assert!(all().any(|e| e.version().0 == 18));
    }
    #[test]
    fn aliases_and_replacement() {
        assert_eq!(shortcode_to_emoji(":joy:"), Some("😂"));
        assert_eq!(shortcode_to_emoji("+1"), shortcode_to_emoji("thumbsup"));
        assert_eq!(
            replace_shortcodes("a:joy::joy: :unknown: :"),
            "a😂😂 :unknown: :"
        );
        assert_eq!(replace_shortcodes(":unknown:joy:"), ":unknown😂");
        assert!(matches!(
            replace_shortcodes("é unknown :not_known:"),
            Cow::Borrowed(_)
        ));
    }
    #[test]
    fn searching() {
        assert_eq!(search(":JOY:", 1)[0].emoji(), "😂");
        assert_eq!(search("thumb", 1).len(), 1);
        assert!(search("", 10).is_empty());
        assert!(search("joy", 0).is_empty());
        assert!(search("not_a_real_emoji", 10).is_empty());
        assert!(!search("face tears", 10).is_empty());
    }
    #[test]
    fn tones() {
        for (i, tone) in SKIN_TONES.iter().enumerate() {
            let s = apply_skin_tone("👍", *tone).unwrap();
            assert!(s.contains(*tone));
            assert_eq!(s.chars().count(), 2);
            assert_eq!(i, (*tone as u32 - 0x1f3fb) as usize);
        }
        assert!(apply_skin_tone("😂", SKIN_TONES[0]).is_none());
        assert!(apply_skin_tone("👍", 'x').is_none());
        let doctor = shortcode_to_emoji("female-doctor").unwrap();
        assert!(apply_skin_tone(doctor, SKIN_TONES[0])
            .unwrap()
            .contains('\u{200d}'));
    }
    #[test]
    fn generated_tones_are_official() {
        let source = include_str!("../data/emoji-test.txt");
        let official: std::collections::HashSet<String> = source
            .lines()
            .filter_map(|line| {
                let (codes, rest) = line.split_once(';')?;
                rest.trim_start().starts_with("fully-qualified").then(|| {
                    codes
                        .split_whitespace()
                        .map(|s| char::from_u32(u32::from_str_radix(s, 16).unwrap()).unwrap())
                        .collect()
                })
            })
            .collect();
        const { assert!(TABLE_BYTES <= 150_000) };
        for e in all() {
            assert!(official.contains(e.emoji()));
            if e.supports_skin_tone() {
                for modifier in SKIN_TONES {
                    let toned = apply_skin_tone(e.emoji(), modifier).unwrap();
                    assert!(official.contains(toned));
                    assert!(toned.contains(modifier));
                }
            }
        }
    }
    #[test]
    fn decay() {
        let mut u = Usage::new("joy".into(), 0);
        u.record(0, 10);
        assert_eq!(u.score(10, 10), 0.5);
        u.record(10, 10);
        assert_eq!(u.weight, 1.5);
        assert_eq!(u.score(0, 10), 1.5);
        let mut entries = vec![Usage::new("z".into(), 0), u];
        rank_frequently_used(&mut entries, 10, 10);
        assert_eq!(entries[0].shortcode, "joy");
        entries[0].weight = f64::NAN;
        assert_eq!(entries[0].score(10, 0), 0.0);
    }
}
