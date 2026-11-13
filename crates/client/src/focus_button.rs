//! Native button focus/keyboard support layered over iced's pointer button.
use crate::{app::Msg, theme as t};
use iced::{
    advanced::{
        layout, mouse, overlay, renderer,
        widget::{self, operation, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    keyboard::{self, key::Named},
    widget::button as native,
    Border, Element, Event, Length, Padding, Rectangle, Renderer, Size, Theme, Vector,
};
#[derive(Default)]
struct Focus {
    focused: bool,
}
impl operation::Focusable for Focus {
    fn is_focused(&self) -> bool {
        self.focused
    }
    fn focus(&mut self) {
        self.focused = true;
    }
    fn unfocus(&mut self) {
        self.focused = false;
    }
}
pub struct Button<'a> {
    inner: native::Button<'a, Msg>,
    message: Option<Msg>,
    centered: bool,
}
pub fn button<'a>(content: impl Into<Element<'a, Msg>>) -> Button<'a> {
    Button {
        inner: native::Button::new(content),
        message: None,
        centered: false,
    }
}
/// Native, wrapping Tab order. iced's default traversal leaves an empty stop
/// after the final widget; modal scopes should wrap directly to their first item.
pub fn cycle(previous: bool) -> impl widget::Operation<()> {
    struct At {
        target: Option<usize>,
        current: usize,
    }
    impl widget::Operation<()> for At {
        fn traverse(&mut self, f: &mut dyn FnMut(&mut dyn widget::Operation<()>)) {
            f(self);
        }
        fn focusable(
            &mut self,
            _: Option<&widget::Id>,
            _: Rectangle,
            state: &mut dyn operation::Focusable,
        ) {
            if self.target == Some(self.current) {
                state.focus();
            } else {
                state.unfocus();
            }
            self.current += 1;
        }
    }
    fn next(count: operation::focusable::Count) -> At {
        at(count, false)
    }
    fn previous_item(count: operation::focusable::Count) -> At {
        at(count, true)
    }
    fn at(count: operation::focusable::Count, previous: bool) -> At {
        At {
            target: (count.total > 0).then(|| match count.focused {
                Some(index) if previous => (index + count.total - 1) % count.total,
                Some(index) => (index + 1) % count.total,
                None if previous => count.total - 1,
                None => 0,
            }),
            current: 0,
        }
    }
    let choose: fn(operation::focusable::Count) -> At = if previous { previous_item } else { next };
    operation::then(operation::focusable::count(), choose)
}
pub fn focus_dialog() -> iced::Task<Msg> {
    iced::widget::operation::focus("closed-composer").chain(
        widget::operate(operation::scope("dialog-scope".into(), cycle(false)))
            .map(|_: ()| Msg::Activity),
    )
}
impl<'a> Button<'a> {
    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }
    pub fn on_press(mut self, message: Msg) -> Self {
        self.message = Some(message.clone());
        self.inner = self.inner.on_press(message);
        self
    }
    pub fn on_press_maybe(mut self, message: Option<Msg>) -> Self {
        self.message = message.clone();
        self.inner = self.inner.on_press_maybe(message);
        self
    }
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.inner = self.inner.padding(padding);
        self
    }
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.inner = self.inner.height(height);
        self
    }
    pub fn style(mut self, style: impl Fn(&Theme, native::Status) -> native::Style + 'a) -> Self {
        self.inner = self.inner.style(style);
        self
    }
}
impl Widget<Msg, Theme, Renderer> for Button<'_> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<Focus>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(Focus::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.inner as &dyn Widget<Msg, Theme, Renderer>)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.children[0].diff(&self.inner as &dyn Widget<Msg, Theme, Renderer>);
    }
    fn size(&self) -> Size<Length> {
        self.inner.size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.inner.size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let mut child = self.inner.layout(&mut tree.children[0], renderer, limits);
        if self.centered {
            let bounds = child.size();
            let nodes = child
                .children()
                .iter()
                .cloned()
                .map(|content| {
                    let size = content.size();
                    content.move_to(iced::Point::new(
                        (bounds.width - size.width) / 2.,
                        (bounds.height - size.height) / 2.,
                    ))
                })
                .collect();
            child = layout::Node::with_children(bounds, nodes);
        }
        layout::Node::with_children(child.size(), vec![child])
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        if self.message.is_some() {
            operation.focusable(None, layout.bounds(), tree.state.downcast_mut::<Focus>());
        }
        self.inner.operate(
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
        let focus = tree.state.downcast_mut::<Focus>();
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let focused = self.message.is_some() && cursor.is_over(layout.bounds());
            if focus.focused != focused {
                focus.focused = focused;
                shell.request_redraw();
            }
        }
        if focus.focused {
            if let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event {
                if matches!(key, keyboard::Key::Named(Named::Enter))
                    || matches!(key,keyboard::Key::Character(v)if v.as_str()==" ")
                {
                    if let Some(message) = self.message.clone() {
                        shell.publish(message);
                        shell.capture_event();
                        shell.request_redraw();
                        return;
                    }
                }
            }
        }
        self.inner.update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
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
        self.inner.draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
        if tree.state.downcast_ref::<Focus>().focused && self.message.is_some() {
            use iced::advanced::Renderer as _;
            renderer.fill_quad(
                renderer::Quad {
                    bounds: layout.bounds(),
                    border: Border {
                        color: t::ACCENT,
                        width: t::FOCUS,
                        radius: t::S8.into(),
                    },
                    ..Default::default()
                },
                t::TRANSPARENT,
            );
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.inner.mouse_interaction(
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
        self.inner.overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}
impl<'a> From<Button<'a>> for Element<'a, Msg> {
    fn from(value: Button<'a>) -> Self {
        Element::new(value)
    }
}
