//! Right-click the precise rich-text link/name under the pointer, preserving native wrapping.
use crate::{
    app::Msg,
    main_window::{mm, Entity, Message as Main, Popover},
};
use iced::{
    advanced::{
        layout, mouse, overlay, renderer,
        widget::{self, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
};
pub struct LinkContext<'a> {
    inner: Element<'a, Msg>,
}
pub fn wrap<'a>(inner: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    Element::new(LinkContext {
        inner: inner.into(),
    })
}
impl Widget<Msg, Theme, Renderer> for LinkContext<'_> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::stateless()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::None
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
        layout::Node::with_children(node.size(), vec![node])
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
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
        ) {
            let mut messages = Vec::new();
            let mut local = Shell::new(&mut messages);
            for e in [
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ] {
                self.inner.as_widget_mut().update(
                    &mut tree.children[0],
                    &e,
                    layout.children().next().unwrap(),
                    cursor,
                    renderer,
                    clipboard,
                    &mut local,
                    viewport,
                );
            }
            for message in messages {
                let entity = match message {
                    Msg::OpenLink(url) | Msg::Link(url) => Some(Entity::Link(url)),
                    Msg::Main(Main::Open(Popover::Profile(id)))
                    | Msg::Open(crate::app::Dialog::Profile(id)) => Some(Entity::User(id)),
                    _ => None,
                };
                if let Some(e) = entity {
                    shell.publish(mm(Main::Open(Popover::Context(e))));
                    shell.capture_event();
                    return;
                }
            }
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
