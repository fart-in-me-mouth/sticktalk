//! Composer emoji completion. Byte positions are explicit so IME/caret edits remain safe.
use sticktalk_emoji::Emoji;
pub fn query(text: &str, caret: usize) -> Option<(usize, &str)> {
    let prefix = text.get(..caret)?;
    let start = prefix.rfind(':')?;
    if start > 0 && !prefix[..start].ends_with(char::is_whitespace) {
        return None;
    }
    let query = &prefix[start + 1..];
    (query.len() >= 2
        && query.len() <= 64
        && query
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'+'))
    .then_some((start, query))
}
pub fn candidates(text: &str, caret: usize) -> Vec<Emoji> {
    query(text, caret).map_or_else(Vec::new, |(_, q)| sticktalk_emoji::search(q, 8))
}
pub fn complete(text: &str, caret: usize, glyph: &str) -> Option<String> {
    let (start, _) = query(text, caret)?;
    Some(format!("{}{glyph} {}", &text[..start], &text[caret..]))
}
pub fn prepare_send(text: &str) -> String {
    sticktalk_emoji::replace_shortcodes(text).into_owned()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trigger_and_caret_are_utf8_safe() {
        for value in [":", ":s", "https://sm", "a:sm", ":sm ", ":sm:"] {
            assert!(query(value, value.len()).is_none(), "{value}");
        }
        let s = "你好 :smi next";
        let caret = s.find(" next").unwrap();
        assert!(!candidates(s, caret).is_empty());
        assert_eq!(complete(s, caret, "😄").unwrap(), "你好 😄  next");
        assert!(query(s, 1).is_none());
    }
    #[test]
    fn replacement_preserves_unknown_and_unicode() {
        assert_eq!(
            prepare_send("你好 :smile: :not_real: :+1:"),
            "你好 😄 :not_real: 👍"
        );
        assert_eq!(
            prepare_send("https://example.com :unfinished"),
            "https://example.com :unfinished"
        );
    }
}
