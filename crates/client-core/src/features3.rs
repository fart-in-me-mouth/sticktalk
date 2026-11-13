//! Shared custom-emoji tokenization and completion. No runtime or renderer required.
use sticktalk_proto::*;
#[derive(Debug, PartialEq, Eq)]
pub enum Part<'a> {
    Text(&'a str),
    Emoji(Id, &'a str),
}
pub fn parts(mut text: &str) -> Vec<Part<'_>> {
    let mut out = Vec::new();
    while let Some(start) = text.find("<:") {
        let Some(end) = text[start..].find('>').map(|n| start + n + 1) else {
            break;
        };
        let token = &text[start..end];
        if let Some(id) = custom_emoji_id(token) {
            if start > 0 {
                out.push(Part::Text(&text[..start]));
            }
            out.push(Part::Emoji(id, token));
            text = &text[end..];
        } else {
            out.push(Part::Text(&text[..end]));
            text = &text[end..];
        }
    }
    if !text.is_empty() {
        out.push(Part::Text(text));
    }
    out
}
pub fn candidates<'a>(text: &str, caret: usize, emoji: &'a [GuildEmoji]) -> Vec<&'a GuildEmoji> {
    let Some((_, query)) = crate::emoji::query(text, caret) else {
        return Vec::new();
    };
    let query = query.to_ascii_lowercase();
    emoji
        .iter()
        .filter(|e| e.name.to_ascii_lowercase().contains(&query))
        .take(8)
        .collect()
}
pub fn prepare_send(text: &str, emoji: &[GuildEmoji]) -> String {
    // Replace server shortcodes before Unicode aliases. Existing custom tokens stay intact.
    let mut out = String::new();
    for part in parts(text) {
        match part {
            Part::Emoji(_, token) => out.push_str(token),
            Part::Text(text) => {
                let mut text = text.to_owned();
                for e in emoji {
                    text = text.replace(&format!(":{}:", e.name), &e.token());
                }
                for p in parts(&text) {
                    match p {
                        Part::Emoji(_, s) => out.push_str(s),
                        Part::Text(s) => out.push_str(&crate::emoji::prepare_send(s)),
                    }
                }
            }
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_completion_and_shortcode_collision_are_safe() {
        let e = GuildEmoji {
            id: Id(42),
            guild_id: Id(1),
            name: "stick_wave".into(),
            animated: true,
        };
        assert_eq!(
            candidates("你好 :sti", 11, std::slice::from_ref(&e)).len(),
            1
        );
        assert_eq!(
            prepare_send(":stick_wave: :smile:", std::slice::from_ref(&e)),
            "<:stick_wave:42> 😄"
        );
        let collision = GuildEmoji {
            name: "smile".into(),
            ..e.clone()
        };
        assert_eq!(prepare_send(":smile:", &[collision]), "<:smile:42>");
        assert_eq!(
            parts("hi <:stick_wave:42>!"),
            vec![
                Part::Text("hi "),
                Part::Emoji(Id(42), "<:stick_wave:42>"),
                Part::Text("!")
            ]
        );
        let parsed = crate::markdown::parse("<:stick_wave:42>");
        let crate::markdown::Block::Paragraph(v) = &parsed.blocks[0] else {
            panic!()
        };
        assert_eq!(v[0].text, e.token());
        let mut p = crate::launch::Projection::default();
        p.event(&Event::GuildEmojiUpdated {
            guild_id: Id(1),
            emoji: vec![e],
        });
        assert_eq!(p.emoji[&Id(1)].len(), 1);
        p.event(&Event::GuildRemoved {
            guild_id: Id(1),
            reason: RemovalReason::Left,
        });
        assert!(p.emoji.is_empty());
    }
}
