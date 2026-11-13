//! Event-driven layout/focus observation; no subscriptions or polling.
use crate::{
    app::Msg,
    main_window::{mm, Message as Main},
};
use iced::{
    advanced::{
        layout, mouse, overlay, renderer,
        widget::{self, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
};
pub struct Observer<'a> {
    inner: Element<'a, Msg>,
    focus: bool,
    shift: Vector,
    clamp: bool,
    avoid: Option<Rectangle>,
}
#[derive(Default)]
struct State {
    focused: bool,
}
#[derive(Default)]
struct Focus(bool);
impl widget::Operation for Focus {
    fn traverse(&mut self, f: &mut dyn FnMut(&mut dyn widget::Operation)) {
        f(self);
    }
    fn focusable(
        &mut self,
        _: Option<&widget::Id>,
        _: Rectangle,
        state: &mut dyn widget::operation::Focusable,
    ) {
        self.0 |= state.is_focused();
    }
}
pub fn anchor<'a>(inner: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    Element::new(Observer {
        inner: inner.into(),
        focus: false,
        shift: Vector::ZERO,
        clamp: false,
        avoid: None,
    })
}
pub fn focus<'a>(inner: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    Element::new(Observer {
        inner: inner.into(),
        focus: true,
        shift: Vector::ZERO,
        clamp: false,
        avoid: None,
    })
}
pub fn shift<'a>(inner: impl Into<Element<'a, Msg>>, shift: Vector) -> Element<'a, Msg> {
    Element::new(Observer {
        inner: inner.into(),
        focus: false,
        shift,
        clamp: false,
        avoid: None,
    })
}
pub fn popup<'a>(
    inner: impl Into<Element<'a, Msg>>,
    position: iced::Point,
    avoid: Option<Rectangle>,
) -> Element<'a, Msg> {
    Element::new(Observer {
        inner: inner.into(),
        focus: false,
        shift: Vector::new(position.x, position.y),
        clamp: true,
        avoid,
    })
}
impl Widget<Msg, Theme, Renderer> for Observer<'_> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.inner.as_widget())]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.children[0].diff(self.inner.as_widget());
    }
    fn size(&self) -> Size<Length> {
        self.inner.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.inner.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let node = self
            .inner
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        let position = iced::Point::new(self.shift.x, self.shift.y);
        let position = if self.clamp {
            crate::main_window::place_popover(position, node.size(), limits.max(), self.avoid)
        } else {
            position
        };
        layout::Node::with_children(node.size(), vec![node.move_to(position)])
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
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
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Msg>,
        viewport: &Rectangle,
    ) {
        if !self.focus
            && !self.clamp
            && self.shift == Vector::ZERO
            && matches!(event, Event::Mouse(mouse::Event::ButtonPressed(_)))
            && cursor.is_over(layout.bounds())
        {
            shell.publish(mm(Main::Anchor(layout.bounds())));
        }
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
        if self.focus {
            let mut focus = Focus::default();
            self.inner.as_widget_mut().operate(
                &mut tree.children[0],
                layout.children().next().unwrap(),
                renderer,
                &mut focus,
            );
            let state = tree.state.downcast_mut::<State>();
            if state.focused != focus.0 {
                state.focused = focus.0;
                shell.publish(mm(Main::SearchFocus(focus.0)));
            }
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
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
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
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
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Msg, Theme, Renderer>> {
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
    use iced::{
        advanced::widget::operation,
        widget::{container, mouse_area, space, text_input},
        Point,
    };
    use iced_runtime::user_interface::{Cache, UserInterface};
    fn renderer() -> Renderer {
        #[cfg(feature = "wgpu")]
        {
            iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
                crate::theme::regular(),
                iced::Pixels(crate::theme::BODY),
            ))
        }
        #[cfg(not(feature = "wgpu"))]
        {
            iced_tiny_skia::Renderer::new(crate::theme::regular(), iced::Pixels(crate::theme::BODY))
        }
    }
    #[test]
    fn keyboard_focus_is_observed_without_polling() {
        let mut renderer = renderer();
        let input = focus(
            text_input("Search", "")
                .id("header-search")
                .on_input(|s| mm(Main::Search(s)))
                .style(crate::theme::input),
        );
        let mut ui = UserInterface::build(input, Size::new(240., 32.), Cache::new(), &mut renderer);
        ui.operate(
            &renderer,
            &mut operation::focusable::focus::<()>("header-search".into()),
        );
        let mut messages = Vec::new();
        let _ = ui.update(
            &[Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            ))],
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        assert!(messages
            .iter()
            .any(|m| matches!(m, Msg::Main(Main::SearchFocus(true)))));
        messages.clear();
        let _ = ui.update(
            &[Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            ))],
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        assert!(!messages
            .iter()
            .any(|m| matches!(m, Msg::Main(Main::SearchFocus(_)))));
    }
    #[test]
    fn context_anchor_precedes_the_open_message() {
        let mut renderer = renderer();
        let row = anchor(
            mouse_area(container(space()).width(200.).height(42.)).on_right_press(mm(Main::Open(
                crate::main_window::Popover::Context(crate::main_window::Entity::User(
                    sticktalk_proto::Id(2),
                )),
            ))),
        );
        let mut ui = UserInterface::build(row, Size::new(240., 80.), Cache::new(), &mut renderer);
        let mut messages = Vec::new();
        let _ = ui.update(
            &[Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Right,
            ))],
            mouse::Cursor::Available(Point::new(20., 20.)),
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        assert!(
            matches!(messages.first(), Some(Msg::Main(Main::Anchor(r))) if r.width == 200. && r.height == 42.)
        );
        assert!(matches!(messages.get(1), Some(Msg::Main(Main::Open(_)))));
    }
}
