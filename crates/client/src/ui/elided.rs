//! Single-line labels measured by the real font, not a guessed character count.
//! The shaped result lives in iced's widget tree; no work is scheduled while idle.
use crate::{app::Msg, theme as t};
use iced::{
    advanced::{
        layout, mouse, renderer,
        text::{self, paragraph::Plain},
        widget::{self, Tree},
        Layout, Widget,
    },
    Color, Element, Font, Length, Rectangle, Renderer, Size, Theme,
};
use unicode_segmentation::UnicodeSegmentation;

type Paragraph = <Renderer as text::Renderer>::Paragraph;

pub struct Elided {
    value: String,
    font: Font,
    size: f32,
    color: Option<Color>,
    width: Length,
    line_height: text::LineHeight,
}

pub fn label(value: impl Into<String>) -> Elided {
    Elided {
        value: value
            .into()
            .chars()
            .map(|c| {
                if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                    ' '
                } else {
                    c
                }
            })
            .collect(),
        font: t::regular(),
        size: t::BODY,
        color: None,
        width: Length::Shrink,
        line_height: text::LineHeight::default(),
    }
}

impl Elided {
    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }
    pub fn line_height(mut self, height: impl Into<text::LineHeight>) -> Self {
        self.line_height = height.into();
        self
    }
}

#[derive(Default)]
struct State {
    paragraph: Plain<Paragraph>,
    key: Option<(String, Font, u32, u32)>,
    display: String,
}

/// Binary search only at grapheme boundaries: never split combining marks or ZWJ emoji.
fn fit(value: &str, available: f32, mut measure: impl FnMut(&str) -> f32) -> String {
    if measure(value) <= available {
        return value.into();
    }
    if measure("…") > available {
        return String::new();
    }
    let boundaries: Vec<_> = value.grapheme_indices(true).map(|(i, _)| i).collect();
    let (mut low, mut high) = (0, boundaries.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        let end = boundaries.get(mid).copied().unwrap_or(value.len());
        if measure(&format!("{}…", &value[..end])) <= available {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    format!(
        "{}…",
        &value[..boundaries.get(low).copied().unwrap_or(value.len())]
    )
}

impl Widget<Msg, Theme, Renderer> for Elided {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }
    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        let width = limits.max().width.max(0.);
        let key = (
            self.value.clone(),
            self.font,
            self.size.to_bits(),
            width.to_bits(),
        );
        let format = iced::widget::text::Format {
            font: Some(self.font),
            size: Some(iced::Pixels(self.size)),
            line_height: self.line_height,
            wrapping: text::Wrapping::None,
            width: self.width,
            ..Default::default()
        };
        if state.key.as_ref() != Some(&key) {
            state.display = fit(&self.value, width, |s| {
                iced::widget::text::layout(
                    &mut state.paragraph,
                    renderer,
                    &layout::Limits::new(Size::ZERO, Size::INFINITE),
                    s,
                    iced::widget::text::Format {
                        width: Length::Shrink,
                        ..format
                    },
                );
                state.paragraph.min_width()
            });
            state.key = Some(key);
        }
        iced::widget::text::layout(
            &mut state.paragraph,
            renderer,
            limits,
            &state.display,
            format,
        )
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        if let Some(clip) = viewport.intersection(&layout.bounds()) {
            iced::widget::text::draw(
                renderer,
                defaults,
                layout.bounds(),
                state.paragraph.raw(),
                iced::widget::text::Style { color: self.color },
                &clip,
            );
        }
    }
    fn operate(
        &mut self,
        _: &mut Tree,
        layout: Layout<'_>,
        _: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        operation.text(None, layout.bounds(), &self.value);
    }
}

impl<'a> From<Elided> for Element<'a, Msg> {
    fn from(value: Elided) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fitting_preserves_graphemes_and_never_exceeds_available_width() {
        let measure = |s: &str| s.graphemes(true).count() as f32;
        for value in ["Wide names", "e\u{301}e\u{301}e\u{301}", "👩‍💻👩‍💻👩‍💻"]
        {
            for width in 0..16 {
                let result = fit(value, width as f32, measure);
                assert!(measure(&result) <= width as f32);
                if result.ends_with('…') {
                    let prefix = result.trim_end_matches('…');
                    assert!(value.starts_with(prefix));
                    assert!(value.grapheme_indices(true).any(|(i, _)| i == prefix.len()));
                }
            }
        }
        assert_eq!(
            fit("Wiii", 4., |s| s
                .chars()
                .map(|c| if c == 'W' { 4. } else { 1. })
                .sum()),
            "…"
        );
    }
}

#[cfg(test)]
mod qa_header_tests {
    use super::*;
    #[test]
    fn long_server_names_fit_one_line_without_splitting_graphemes() {
        crate::headless::load_fonts();
        #[cfg(feature = "wgpu")]
        let renderer = Renderer::Secondary(iced_tiny_skia::Renderer::new(
            t::regular(),
            iced::Pixels(t::BODY),
        ));
        #[cfg(not(feature = "wgpu"))]
        let renderer = iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY));
        for value in [
            "Real-data launch QA with a long server name",
            "WWWWWWWWWWWWWWWWWWWWWWWW",
            "iiiiiiiiiiiiiiiiiiiiiiiiiiii",
            "e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}",
            "First line\nSecond line\u{2028}Third line",
        ] {
            for width in [0., 32., 96., 144., 168.] {
                let mut widget = label(value)
                    .font(t::semibold())
                    .size(t::CHANNEL_NAME)
                    .width(Length::Fill);
                let mut tree = Tree::new(&widget as &dyn Widget<Msg, Theme, Renderer>);
                widget.layout(
                    &mut tree,
                    &renderer,
                    &layout::Limits::new(Size::ZERO, Size::new(width, 48.)),
                );
                let state = tree.state.downcast_ref::<State>();
                let fitted = &state.display;
                assert!(
                    state.paragraph.min_width() <= width + 0.01,
                    "{value:?} -> {fitted:?} exceeds {width}"
                );
                assert!(!fitted.contains(['\n', '\u{2028}']));
                if value.contains('\u{301}') && fitted.ends_with('…') {
                    assert!(!fitted[..fitted.len() - '…'.len_utf8()].ends_with('e'));
                }
            }
        }
    }
}
