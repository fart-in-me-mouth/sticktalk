//! Discord-style, bounded client-side markup. Parsed trees live in a bounded cache, not widgets.
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};
use sticktalk_proto::Id;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Auto(String),
    Masked(String),
    Mention(Id),
}
#[derive(Debug, Clone)]
pub struct Inline {
    pub text: String,
    pub style: Style,
    pub target: Option<Target>,
    pub spoiler: Option<usize>,
}
#[derive(Debug, Clone)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Header(u8, Vec<Inline>),
    Quote(Vec<Inline>),
    List(Vec<Inline>),
    Code { language: String, text: String },
}
#[derive(Debug, Clone)]
pub struct Document {
    pub blocks: Vec<Block>,
}
#[derive(Default)]
pub struct Cache {
    entries: RefCell<BTreeMap<Id, (String, Arc<Document>)>>,
}
impl Cache {
    pub fn get(&self, id: Id, source: &str) -> Arc<Document> {
        let mut entries = self.entries.borrow_mut();
        if let Some((s, d)) = entries.get(&id) {
            if s == source {
                return d.clone();
            }
        }
        // Same cap as the retained message projection; callers purge inaccessible/evicted IDs.
        if entries.len() >= crate::model::MESSAGE_LIMIT && !entries.contains_key(&id) {
            entries.pop_first();
        }
        let doc = Arc::new(parse(source));
        entries.insert(id, (source.to_owned(), doc.clone()));
        doc
    }
    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
    }
    pub fn retain(&self, model: &crate::model::Model) {
        self.entries
            .borrow_mut()
            .retain(|id, _| model.messages.values().any(|m| m.contains_key(id)));
    }
}
pub fn parse(source: &str) -> Document {
    let mut blocks = Vec::new();
    let mut lines = source.lines().peekable();
    let mut spoiler = 0;
    while let Some(line) = lines.next() {
        if let Some(language) = line.strip_prefix("```") {
            let mut code = String::new();
            for l in lines.by_ref() {
                if l.trim() == "```" {
                    break;
                }
                if !code.is_empty() {
                    code.push('\n');
                }
                code.push_str(l);
            }
            blocks.push(Block::Code {
                language: language.trim().chars().take(32).collect(),
                text: code,
            });
            continue;
        }
        let mut style = Style::default();
        let (kind, body) = if let Some(v) = line.strip_prefix("### ") {
            (3, v)
        } else if let Some(v) = line.strip_prefix("## ") {
            (2, v)
        } else if let Some(v) = line.strip_prefix("# ") {
            (1, v)
        } else if let Some(v) = line.strip_prefix("> ") {
            (4, v)
        } else if let Some(v) = line.strip_prefix("- ") {
            (5, v)
        } else {
            (0, line)
        };
        if (1..=3).contains(&kind) {
            style.bold = true;
        }
        let mut values = Vec::new();
        inline(body, style, None, &mut spoiler, 0, &mut values);
        blocks.push(match kind {
            1..=3 => Block::Header(kind, values),
            4 => Block::Quote(values),
            5 => Block::List(values),
            _ => Block::Paragraph(values),
        });
    }
    Document { blocks }
}
fn push(
    out: &mut Vec<Inline>,
    text: String,
    style: Style,
    target: Option<Target>,
    spoiler: Option<usize>,
) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut() {
        if last.style == style && last.target == target && last.spoiler == spoiler {
            last.text.push_str(&text);
            return;
        }
    }
    out.push(Inline {
        text,
        style,
        target,
        spoiler,
    });
}
fn closing(source: &str, delimiter: &str) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in source.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if source[i..].starts_with(delimiter) {
            return Some(i);
        }
    }
    None
}
fn safe_url(s: &str) -> bool {
    url::Url::parse(s).is_ok_and(|u| {
        matches!(u.scheme(), "https" | "http") && !u.host_str().unwrap_or_default().is_empty()
    })
}
fn inline(
    mut s: &str,
    style: Style,
    hidden: Option<usize>,
    next: &mut usize,
    depth: u8,
    out: &mut Vec<Inline>,
) {
    if depth >= 16 {
        push(out, s.into(), style, None, hidden);
        return;
    }
    while !s.is_empty() {
        if s.starts_with("<:") {
            if let Some(end) = s.find('>') {
                let token = &s[..=end];
                if sticktalk_proto::custom_emoji_id(token).is_some() {
                    push(out, token.into(), style, None, hidden);
                    s = &s[end + 1..];
                    continue;
                }
            }
        }
        if let Some(rest) = s.strip_prefix('\\') {
            if let Some(c) = rest.chars().next() {
                push(out, c.to_string(), style, None, hidden);
                s = &rest[c.len_utf8()..];
                continue;
            }
        }
        if let Some(rest) = s.strip_prefix('`') {
            if let Some(end) = rest.find('`') {
                push(
                    out,
                    rest[..end].into(),
                    Style {
                        code: true,
                        ..style
                    },
                    None,
                    hidden,
                );
                s = &rest[end + 1..];
                continue;
            }
        }
        if let Some(rest) = s.strip_prefix('[') {
            if let Some(end) = closing(rest, "](") {
                let after = &rest[end + 2..];
                // Balanced parentheses in URL; escaped delimiters remain literal.
                let mut level = 0;
                let mut finish = None;
                for (i, c) in after.char_indices() {
                    if c == '(' {
                        level += 1;
                    } else if c == ')' {
                        if level == 0 {
                            finish = Some(i);
                            break;
                        }
                        level -= 1;
                    }
                }
                if let Some(n) = finish {
                    let url = &after[..n];
                    if safe_url(url) {
                        let mut labels = Vec::new();
                        inline(&rest[..end], style, hidden, next, depth + 1, &mut labels);
                        for value in &mut labels {
                            value.target = Some(Target::Masked(url.into()));
                        }
                        out.extend(labels);
                        s = &after[n + 1..];
                        continue;
                    }
                }
            }
        }
        if let Some(rest) = s.strip_prefix("<@") {
            if let Some(end) = rest.find('>') {
                if let Ok(id) = rest[..end].parse::<u64>() {
                    push(
                        out,
                        format!("<@{id}>"),
                        style,
                        Some(Target::Mention(Id(id))),
                        hidden,
                    );
                    s = &rest[end + 1..];
                    continue;
                }
            }
        }
        // `<https://...>` is a link without a preview; the brackets are not shown.
        if let Some(rest) = s.strip_prefix('<') {
            if rest.starts_with("https://") || rest.starts_with("http://") {
                if let Some(end) = rest
                    .find('>')
                    .filter(|e| !rest[..*e].contains(char::is_whitespace))
                {
                    let url = &rest[..end];
                    if safe_url(url) {
                        push(
                            out,
                            url.into(),
                            style,
                            Some(Target::Auto(url.into())),
                            hidden,
                        );
                        s = &rest[end + 1..];
                        continue;
                    }
                }
            }
        }
        if s.starts_with("https://") || s.starts_with("http://") {
            let end = s.find(char::is_whitespace).unwrap_or(s.len());
            let url = s[..end].trim_end_matches(['.', ',', '!', ';', ')', ']']);
            if safe_url(url) {
                push(
                    out,
                    url.into(),
                    style,
                    Some(Target::Auto(url.into())),
                    hidden,
                );
                s = &s[url.len()..];
                continue;
            }
        }
        let mut matched = false;
        for delimiter in ["**", "__", "~~", "||", "*", "_"] {
            if let Some(rest) = s.strip_prefix(delimiter) {
                if let Some(end) = closing(rest, delimiter).filter(|n| *n > 0) {
                    let mut nested = style;
                    let mut spoiler = hidden;
                    match delimiter {
                        "**" => nested.bold = true,
                        "__" => nested.underline = true,
                        "~~" => nested.strike = true,
                        "||" => {
                            spoiler = Some(*next);
                            *next += 1;
                        }
                        _ => nested.italic = true,
                    }
                    inline(&rest[..end], nested, spoiler, next, depth + 1, out);
                    s = &rest[end + delimiter.len()..];
                    matched = true;
                    break;
                }
            }
        }
        if matched {
            continue;
        }
        let c = s.chars().next().unwrap();
        push(out, c.to_string(), style, None, hidden);
        s = &s[c.len_utf8()..];
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discord_subset_and_escapes() {
        let d=parse("# Header\n## Two\n### Three\n> quote\n- item\n```rust\nlet x = 1;\n```\n**bold** *italic* _also_ __underline__ ~~strike~~ `code` ||hidden|| [label](https://example.com) https://example.org <@1> \\*literal\\*");
        assert_eq!(d.blocks.len(), 7);
        let Block::Code { language, text } = &d.blocks[5] else {
            panic!()
        };
        assert_eq!(language, "rust");
        assert_eq!(text, "let x = 1;");
        let Block::Paragraph(v) = d.blocks.last().unwrap() else {
            panic!()
        };
        assert!(v.iter().any(|x| x.text == "bold" && x.style.bold));
        assert!(v.iter().any(|x| x.style.underline));
        assert!(v.iter().any(|x| x.style.strike));
        assert!(v.iter().any(|x| x.style.code));
        assert!(v.iter().any(|x| x.spoiler == Some(0)));
        assert!(v
            .iter()
            .any(|x| matches!(x.target, Some(Target::Masked(_)))));
        assert!(v.iter().any(|x| matches!(x.target, Some(Target::Auto(_)))));
        let quiet = parse("see <https://example.com/a> now").blocks;
        let Block::Paragraph(v) = &quiet[0] else {
            panic!()
        };
        assert!(v.iter().any(
            |x| x.text == "https://example.com/a" && matches!(x.target, Some(Target::Auto(_)))
        ));
        assert!(!v
            .iter()
            .any(|x| x.text.contains('<') || x.text.contains('>')));
        assert!(v.last().unwrap().text.contains("*literal*"));
    }
    #[test]
    fn cache_reuses_and_invalidates_edits() {
        let c = Cache::default();
        let a = c.get(Id(1), "**a**");
        assert!(Arc::ptr_eq(&a, &c.get(Id(1), "**a**")));
        assert!(!Arc::ptr_eq(&a, &c.get(Id(1), "b")));
    }
    #[test]
    fn unsafe_links_remain_text_and_nesting_is_bounded() {
        let d = parse("[bad](javascript:alert(1)) **nested _words_** \\__plain\\__");
        let Block::Paragraph(v) = &d.blocks[0] else {
            panic!()
        };
        assert!(v.iter().all(|v| v.target.is_none()));
        assert!(v.iter().any(|v| v.style.bold && v.style.italic));
        let _ = parse(&"[**||".repeat(1000));
    }
}
