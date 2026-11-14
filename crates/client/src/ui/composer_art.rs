//! Custom composer artwork over the native editor's identical shaped geometry.
//! The zero-ink token font reserves one 22px advance; the native editor still
//! owns all editing, selection, caret, IME, wrapping and scrolling behavior.
use super::*;
use crate::composer_emoji::{self, EmojiHighlighter};
use iced::advanced::{
    image::{self as advanced_image, Renderer as _},
    layout, mouse, overlay, renderer,
    text::{editor::Editor as _, highlighter::Highlighter as _, LineHeight},
    widget::{self as advanced_widget, Tree},
    Clipboard, Layout, Shell, Widget,
};
use std::cell::RefCell;
type Editor = iced_tiny_skia::graphics::text::Editor;
#[derive(Clone)]
pub(super) struct Picture {
    pub token: String,
    handle: image::Handle,
}
pub(super) fn pictures(app: &App) -> Vec<Picture> {
    use sticktalk_client_core::features3::{parts, Part};
    let mut pictures = Vec::new();
    for part in parts(&app.editor.text()) {
        let Part::Emoji(id, token) = part else {
            continue;
        };
        if pictures.iter().any(|p: &Picture| p.token == token) {
            continue;
        }
        if !crate::features3::entries(app).iter().any(|e| e.id == id) {
            continue;
        }
        if let Some(handle) = app
            .text_features
            .assets
            .get(&id)
            .and_then(|a| a.frames.as_ref())
            .and_then(|f| f.images.first())
            .map(|f| f.0.clone())
        {
            pictures.push(Picture {
                token: token.into(),
                handle,
            });
        }
        if pictures.len() >= 50 {
            break;
        }
    }
    pictures
}
pub(super) fn wrap<'a>(
    inner: Element<'a, Msg>,
    content: &'a text_editor::Content,
    pictures: Vec<Picture>,
) -> Element<'a, Msg> {
    Element::new(Artwork {
        inner,
        content,
        pictures,
    })
}
struct Artwork<'a> {
    inner: Element<'a, Msg>,
    content: &'a text_editor::Content,
    pictures: Vec<Picture>,
}
struct Geometry {
    editor: Editor,
    text: String,
}
impl Default for Geometry {
    fn default() -> Self {
        Self {
            editor: Editor::with_text(""),
            text: String::new(),
        }
    }
}
impl Widget<Msg, iced::Theme, iced::Renderer> for Artwork<'_> {
    fn tag(&self) -> advanced_widget::tree::Tag {
        advanced_widget::tree::Tag::of::<RefCell<Geometry>>()
    }
    fn state(&self) -> advanced_widget::tree::State {
        advanced_widget::tree::State::new(RefCell::new(Geometry::default()))
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.inner.as_widget())]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.children[0].diff(self.inner.as_widget());
    }
    fn size(&self) -> iced::Size<Length> {
        self.inner.as_widget().size()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let node = self
            .inner
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        // Keep this wrapper in the tree even before previews arrive, so an
        // asynchronous custom-emoji load never destroys native input focus.
        if self.pictures.is_empty() {
            return layout::Node::with_children(node.size(), vec![node]);
        }
        let mut g = tree.state.downcast_ref::<RefCell<Geometry>>().borrow_mut();
        let text = self.content.text();
        if g.text != text {
            g.editor = Editor::with_text(&text);
            g.text = text;
        }
        if g.editor.cursor() != self.content.cursor() {
            g.editor.move_to(self.content.cursor());
        }
        let settings = self.pictures.iter().map(|p| p.token.clone()).collect();
        let mut h = EmojiHighlighter::new(&settings);
        g.editor.update(
            iced::Size::new(
                (node.size().width - t::S8).max(0.),
                (node.size().height - t::S8).max(0.),
            ),
            t::regular(),
            t::BODY.into(),
            LineHeight::Absolute(t::COMPOSER_LINE.into()),
            widget::text::Wrapping::WordOrGlyph,
            &mut h,
        );
        g.editor
            .highlight(t::regular(), &mut EmojiHighlighter::new(&settings), |v| {
                composer_emoji::format(v, &iced::Theme::Dark)
            });
        layout::Node::with_children(node.size(), vec![node])
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn advanced_widget::Operation,
    ) {
        self.inner.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Msg>,
        viewport: &iced::Rectangle,
    ) {
        if self.pictures.is_empty() {
            self.inner.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout.children().next().unwrap(),
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
            return;
        }
        let mut messages = Vec::new();
        let mut child = Shell::new(&mut messages);
        self.inner.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            &mut child,
            viewport,
        );
        let geometry = tree.state.downcast_ref::<RefCell<Geometry>>();
        shell.merge(child, |message| {
            let mut g = geometry.borrow_mut();
            if let Msg::Editor(action) = &message {
                g.editor.perform(action.clone());
                g.text = (0..g.editor.line_count())
                    .filter_map(|i| g.editor.line(i))
                    .map(|l| l.text.into_owned())
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            message
        });
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        self.inner.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
        if self.pictures.is_empty() {
            return;
        }
        let bounds = layout.bounds().shrink(t::S4);
        let Some(clip) = bounds.intersection(viewport) else {
            return;
        };
        let g = tree.state.downcast_ref::<RefCell<Geometry>>().borrow();
        use iced::advanced::Renderer as _;
        renderer.with_layer(clip, |renderer| {
            for run in g.editor.buffer().layout_runs() {
                for picture in &self.pictures {
                    for (offset, _) in run.text.match_indices(&picture.token) {
                        if let Some(glyph) = run.glyphs.iter().find(|glyph| glyph.start == offset) {
                            let image_bounds = iced::Rectangle {
                                x: bounds.x + glyph.x,
                                // Same baseline/descender as the bundled native emoji face.
                                y: bounds.y + run.line_y - (t::EMOJI_INLINE - t::EMOJI_DESCENT),
                                width: t::EMOJI_INLINE,
                                height: t::EMOJI_INLINE,
                            };
                            renderer.draw_image(
                                advanced_image::Image::new(picture.handle.clone()),
                                image_bounds,
                                clip,
                            );
                        }
                    }
                }
            }
        });
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.inner.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &iced::Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'a, Msg, iced::Theme, iced::Renderer>> {
        self.inner.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::widget::{operation, Operation};
    use iced_runtime::user_interface::{Cache, UserInterface};
    #[test]
    fn preview_arrival_preserves_native_composer_focus_and_cursor() {
        crate::headless::load_fonts();
        let mut app = App::headless("rowfix-custom-composer", iced::Size::new(1280., 800.));
        app.editor = text_editor::Content::with_text("Hello <:stick_wave:800>");
        app.editor
            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
        let cursor = app.editor.cursor();
        let asset = app.text_features.assets.remove(&Id(800)).unwrap();
        assert!(pictures(&app).is_empty());
        #[cfg(feature = "wgpu")]
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            t::regular(),
            iced::Pixels(t::BODY),
        ));
        #[cfg(not(feature = "wgpu"))]
        let mut renderer = iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY));
        let mut ui = UserInterface::build(
            super::super::compose_editor(&app, String::new(), 160., false),
            iced::Size::new(240., 200.),
            Cache::new(),
            &mut renderer,
        );
        ui.operate(
            &renderer,
            &mut operation::focusable::focus::<()>("composer-editor".into()),
        );
        let cache = ui.into_cache();
        app.text_features.assets.insert(Id(800), asset);
        assert_eq!(pictures(&app).len(), 1);
        let mut ui = UserInterface::build(
            super::super::compose_editor(&app, String::new(), 160., false),
            iced::Size::new(240., 200.),
            cache,
            &mut renderer,
        );
        #[derive(Default)]
        struct Focused(bool);
        impl Operation for Focused {
            fn traverse(&mut self, f: &mut dyn FnMut(&mut dyn Operation)) {
                f(self);
            }
            fn focusable(
                &mut self,
                _: Option<&advanced_widget::Id>,
                _: iced::Rectangle,
                state: &mut dyn operation::Focusable,
            ) {
                self.0 |= state.is_focused();
            }
        }
        let mut focused = Focused::default();
        ui.operate(&renderer, &mut focused);
        assert!(focused.0, "preview load reset the editor's widget tree");
        assert_eq!(app.editor.cursor(), cursor);
    }
}
