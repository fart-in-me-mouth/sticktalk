// Build with --target wasm32-unknown-unknown --profile release-small.
// Exports keep every public operation reachable for the incremental size probe.
use sticktalk_emoji::*;
#[no_mangle]
pub extern "C" fn emoji_probe(n: usize) -> usize {
    let e = all().nth(n % all().len()).unwrap();
    let mut usage = [Usage::new(e.shortcode().into(), 0)];
    usage[0].record(n as u64, 100);
    rank_frequently_used(&mut usage, n as u64, 100);
    TABLE_BYTES
        + categories().len()
        + by_category(e.category()).count()
        + search(e.name(), 8).len()
        + shortcode_to_emoji(e.shortcode()).unwrap().len()
        + emoji_to_shortcode(e.emoji()).unwrap().len()
        + apply_skin_tone(e.emoji(), SKIN_TONES[0]).map_or(0, str::len)
        + replace_shortcodes(":joy: :thumbsup:").len()
        + e.keywords().count()
        + e.version().0 as usize
        + usage[0].weight as usize
}
fn main() {}
