//! Offline, virtualized emoji surface. Only visible rows and two overscan rows are built.
use super::*;
use crate::features::{Event, Picker};
use iced::widget::column;
use sticktalk_emoji::{self as data, Emoji};
#[derive(Default)]
pub struct State {
    pub query: String,
    pub category: usize,
    pub tone: Option<char>,
    pub y: f32,
    pub hover: Option<Emoji>,
    pub usage: Vec<data::Usage>,
}
impl State {
    pub fn entries(&self) -> Vec<Emoji> {
        let entries: Vec<_> = if self.query.is_empty() {
            data::by_category(data::categories()[self.category]).collect()
        } else {
            data::search(&self.query, 2048)
        };
        entries
            .into_iter()
            .filter(|&e| has_artwork(&self.glyph(e)))
            .collect()
    }
    pub fn glyph(&self, e: Emoji) -> String {
        self.tone
            .and_then(|t| data::apply_skin_tone(e.emoji(), t))
            .unwrap_or(e.emoji())
            .into()
    }
    pub fn record(&mut self, glyph: &str) {
        let Some(code) = data::emoji_to_shortcode(glyph).or_else(|| {
            data::all()
                .find(|e| {
                    data::SKIN_TONES
                        .iter()
                        .any(|&t| data::apply_skin_tone(e.emoji(), t) == Some(glyph))
                })
                .map(|e| e.shortcode())
        }) else {
            return;
        };
        let now = crate::mock::now();
        if let Some(u) = self.usage.iter_mut().find(|u| u.shortcode == code) {
            u.record(now, 604_800_000);
        } else {
            let mut u = data::Usage::new(code.into(), now);
            u.record(now, 604_800_000);
            self.usage.push(u);
        }
        data::rank_frequently_used(&mut self.usage, now, 604_800_000);
        self.usage.truncate(32);
    }
}
const ART: &[u8] = include_bytes!("../../assets/emoji/twemoji.bin");
const INDEX: &[(&str, usize, usize)] = include!("../../assets/emoji/twemoji-index.rs");
fn art_key(value: &str) -> String {
    let exact = value
        .chars()
        .map(|c| format!("{:x}", c as u32))
        .collect::<Vec<_>>()
        .join("-");
    if INDEX.binary_search_by_key(&exact.as_str(), |v| v.0).is_ok() {
        exact
    } else {
        value
            .chars()
            .filter(|c| *c != '\u{fe0f}')
            .map(|c| format!("{:x}", c as u32))
            .collect::<Vec<_>>()
            .join("-")
    }
}
pub fn has_artwork(value: &str) -> bool {
    INDEX
        .binary_search_by_key(&art_key(value).as_str(), |v| v.0)
        .is_ok()
}
pub fn artwork(value: &str) -> Option<image::Handle> {
    thread_local! { static CACHE:std::cell::RefCell<std::collections::BTreeMap<String,image::Handle>>=const{std::cell::RefCell::new(std::collections::BTreeMap::new())}; }
    let key = art_key(value);
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(h) = cache.get(&key) {
            return Some(h.clone());
        }
        let i = INDEX.binary_search_by_key(&key.as_str(), |v| v.0).ok()?;
        let (_, start, len) = INDEX[i];
        let h = image::Handle::from_bytes(&ART[start..start + len]);
        if cache.len() >= 192 {
            cache.clear();
        }
        cache.insert(key, h.clone());
        Some(h)
    })
}
pub fn glyph<'a>(value: &str, size: f32) -> Element<'a, Msg> {
    if let Some(h) = artwork(value) {
        image(h).width(size).height(size).into()
    } else {
        space().width(0).height(0).into()
    }
}
fn cell(e: Emoji, s: &State) -> Element<'static, Msg> {
    let value = s.glyph(e);
    mouse_area(
        button(
            container(glyph(&value, t::S24))
                .center_x(t::AVATAR)
                .center_y(t::AVATAR),
        )
        .on_press(Msg::Emoji(value))
        .padding(0)
        .style(t::icon),
    )
    .on_enter(Msg::feature(Event::EmojiHover(e)))
    .into()
}
pub fn view(app: &App) -> Element<'_, Msg> {
    let s = &app.features.emoji;
    let body_height = (app.height - t::ATTACHMENT_CARD - t::S24 - t::S8 - t::S32 * 7.)
        .clamp(t::S32 * 6., t::S32 * 10. - t::S4);
    let mut tabs = column![].spacing(t::S4);
    for (i, (category, icon_name)) in data::categories()
        .iter()
        .zip([
            "smile",
            "sprout",
            "coffee",
            "trophy",
            "plane",
            "lightbulb",
            "heart",
            "flag",
        ])
        .enumerate()
    {
        tabs = tabs.push(
            tooltip(
                button(icon(
                    icon_name,
                    t::ICON,
                    if s.category == i {
                        t::ACCENT
                    } else {
                        t::TEXT_SECONDARY
                    },
                ))
                .on_press(Msg::feature(Event::EmojiCategory(i)))
                .padding(t::S8)
                .style(t::navigation(s.category == i)),
                container(hint(*category)).padding(t::S8).style(t::tooltip),
                tooltip::Position::Left,
            )
            .style(t::tooltip),
        );
    }
    let entries = s.entries();
    let count = entries.len().div_ceil(8);
    let row_h = t::AVATAR + t::S4;
    let first = (s.y / row_h) as usize;
    let start = first.saturating_sub(1).min(count);
    let end = (first + 8).min(count);
    let mut grid = column![space().height(start as f32 * row_h)].spacing(0);
    for r in start..end {
        let mut line = Row::new().spacing(t::S4);
        for &e in entries.iter().skip(r * 8).take(8) {
            line = line.push(cell(e, s));
        }
        grid = grid.push(container(line).height(row_h));
    }
    grid = grid.push(space().height((count - end) as f32 * row_h));
    if count == 0 {
        grid = grid.push(
            container(
                column![
                    icon("search", t::S32, t::TEXT_SECONDARY),
                    txt("No emoji found"),
                    hint("Try another word or shortcode.").color(t::TEXT_SECONDARY)
                ]
                .spacing(t::S12)
                .align_x(Center),
            )
            .center_x(Fill)
            .padding(t::S16),
        );
    }
    let mut frequent = Row::new().spacing(t::S4);
    let defaults = [
        "joy", "heart", "thumbsup", "fire", "tada", "eyes", "wave", "sparkles",
    ];
    let codes: Vec<_> = if s.usage.is_empty() {
        defaults.to_vec()
    } else {
        s.usage
            .iter()
            .take(8)
            .map(|u| u.shortcode.as_str())
            .collect()
    };
    for code in codes {
        if let Some(e) = data::search(code, 1)
            .first()
            .filter(|&&e| has_artwork(&s.glyph(e)))
        {
            frequent = frequent.push(cell(*e, s));
        }
    }
    let mut tones = Row::new().spacing(t::S4);
    for tone in std::iter::once(None).chain(data::SKIN_TONES.map(Some)) {
        let value = tone
            .and_then(|v| data::apply_skin_tone("👋", v))
            .unwrap_or("👋");
        tones = tones.push(
            button(glyph(value, t::ICON))
                .on_press(Msg::feature(Event::EmojiTone(tone)))
                .padding(t::S4)
                .style(t::navigation(s.tone == tone)),
        );
    }
    let preview = s.hover.or_else(|| entries.first().copied());
    let footer: Element<'_, Msg> = if let Some(e) = preview {
        row![
            glyph(&s.glyph(e), t::AVATAR),
            column![
                txt(e.name()).font(t::medium()),
                hint(format!(":{}:", e.shortcode()))
            ]
            .spacing(t::S4)
        ]
        .spacing(t::S12)
        .align_y(Center)
        .into()
    } else {
        hint("No emoji found").into()
    };
    container(
        column![
            row![
                txt("Emoji").font(t::semibold()),
                space().width(Fill),
                icon_button("x", "Close", Msg::feature(Event::Picker(Picker::None)))
            ]
            .align_y(Center),
            text_input("Search emoji", &s.query)
                .id("emoji-search")
                .on_input(|v| Msg::feature(Event::EmojiSearch(v)))
                .padding(t::S12)
                .font(t::regular())
                .size(t::BODY)
                .style(t::input),
            row![
                scroller(tabs).height(body_height),
                column![
                    section("FREQUENTLY USED"),
                    frequent,
                    section(if s.query.is_empty() {
                        data::categories()[s.category].to_uppercase()
                    } else {
                        "SEARCH RESULTS".into()
                    }),
                    scroller(grid)
                        .id("emoji-grid")
                        .height(body_height - t::S32 * 3.)
                        .on_scroll(|v| Msg::feature(Event::EmojiScroll(v.absolute_offset().y)))
                ]
                .spacing(t::S8)
            ]
            .spacing(t::S8),
            rule::horizontal(t::HAIRLINE).style(t::divider),
            row![hint("Skin tone"), space().width(Fill), tones].align_y(Center),
            rule::horizontal(t::HAIRLINE).style(t::divider),
            container(footer).height(t::HEADER)
        ]
        .spacing(t::S8),
    )
    .padding(t::S12)
    .width(t::PICKER_W)
    .style(t::popover)
    .into()
}
pub fn autocomplete(app: &App) -> Option<Element<'_, Msg>> {
    let entries = app.emoji_candidates();
    if entries.is_empty() {
        return None;
    }
    let mut rows = column![section("EMOJI MATCHING YOUR SHORTCODE")].spacing(t::S4);
    for (i, e) in entries.iter().enumerate() {
        rows = rows.push(
            button(
                row![
                    glyph(e.emoji(), t::S24),
                    txt(format!(":{}:", e.shortcode())),
                    hint(e.name())
                ]
                .spacing(t::S12)
                .align_y(Center),
            )
            .on_press(Msg::feature(Event::EmojiComplete(e.emoji().into())))
            .width(Fill)
            .height(t::AUTOCOMPLETE)
            .padding([t::S4, t::S8])
            .style(t::navigation(i == app.suggestion)),
        );
    }
    Some(
        container(
            scroller(rows)
                .id("composer-suggestions")
                .height(Length::Shrink),
        )
        .max_height((app.height * 0.4).min(t::S32 * 10.))
        .padding(t::S8)
        .style(t::popover)
        .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_art_index_is_sorted_and_common_sequences_render() {
        assert!(INDEX.windows(2).all(|w| w[0].0 < w[1].0));
        for glyph in ["❤️", "👍", "👋", "🙂", "😶", "❤️‍🔥", "👋🏽"] {
            assert!(artwork(glyph).is_some(), "{glyph}");
        }
    }
    #[test]
    fn every_picker_entry_has_art_even_with_a_selected_skin_tone() {
        for tone in std::iter::once(None).chain(data::SKIN_TONES.map(Some)) {
            for category in 0..8 {
                let s = State {
                    category,
                    tone,
                    ..Default::default()
                };
                assert!(s.entries().iter().all(|&e| has_artwork(&s.glyph(e))));
            }
        }
        assert!(!has_artwork("\u{10ffff}")); // Missing art never becomes a text label.
    }
    #[test]
    fn virtual_window_is_bounded() {
        let s = State::default();
        assert!(s.entries().len() > 100);
        assert_eq!(s.glyph(data::search("wave", 1)[0]), "👋");
    }
}
