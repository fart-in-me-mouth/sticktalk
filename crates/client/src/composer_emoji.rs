//! Native composer emoji shaping. Using a color bitmap face preserves the editor's
//! own caret, grapheme selection, wrapping, scrolling and IME geometry.
use iced::advanced::text::highlighter::{Format, Highlighter};
use iced::{Font, Theme};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub const CUSTOM_FONT: &[u8] = include_bytes!("../assets/fonts/CustomEmojiAdvance.ttf");
pub const FONT: &[u8] = include_bytes!("../assets/fonts/ComposerEmoji.ttf");
#[derive(Default)]
pub struct EmojiHighlighter {
    line: usize,
    custom: Vec<String>,
}
impl Highlighter for EmojiHighlighter {
    type Settings = Vec<String>;
    type Highlight = bool;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, bool)>;
    fn new(custom: &Vec<String>) -> Self {
        Self {
            custom: custom.clone(),
            ..Self::default()
        }
    }
    fn update(&mut self, custom: &Vec<String>) {
        self.line = 0;
        self.custom.clone_from(custom);
    }
    fn change_line(&mut self, line: usize) {
        self.line = self.line.min(line);
    }
    fn current_line(&self) -> usize {
        self.line
    }
    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        self.line += 1;
        let mut ranges = Vec::new();
        let mut offset = 0;
        for part in sticktalk_client_core::features3::parts(line) {
            use sticktalk_client_core::features3::Part;
            let text = match part {
                Part::Text(text) | Part::Emoji(_, text) => text,
            };
            if matches!(part, Part::Emoji(..)) && self.custom.iter().any(|c| c == text) {
                ranges.push((offset..offset + text.len(), true));
            } else {
                ranges.extend(
                    text.grapheme_indices(true)
                        .filter(|(_, g)| crate::ui::emoji_picker::has_artwork(g))
                        .map(|(i, g)| (offset + i..offset + i + g.len(), false)),
                );
            }
            offset += text.len();
        }
        ranges.into_iter()
    }
}
pub fn format(custom: &bool, _: &Theme) -> Format<Font> {
    Format {
        font: Some(Font::with_name(if *custom {
            "StickTalk Custom Advance"
        } else {
            "StickTalk Composer Emoji"
        })),
        color: None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_editor_highlights_whole_emoji_graphemes_only() {
        let mut h = EmojiHighlighter::default();
        let text = "Hello 👋🏽 ❤️‍🔥 🇺🇸 1️⃣!";
        let ranges: Vec<_> = h
            .highlight_line(text)
            .map(|(r, _)| text[r].to_string())
            .collect();
        assert_eq!(ranges, ["👋🏽", "❤️‍🔥", "🇺🇸", "1️⃣"]);
        assert!(h
            .highlight_line("plain text <:custom:123>")
            .next()
            .is_none());
        h.update(&vec!["<:custom:123>".into()]);
        let text = "Hi <:custom:123> 😄!";
        let ranges: Vec<_> = h
            .highlight_line(text)
            .map(|(r, custom)| (text[r].to_string(), custom))
            .collect();
        assert_eq!(
            ranges,
            [("<:custom:123>".into(), true), ("😄".into(), false)]
        );
    }
}
