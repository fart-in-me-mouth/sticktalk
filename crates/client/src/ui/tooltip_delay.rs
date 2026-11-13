//! Delayed native tooltip with a shared adjacent-target grace window.
//! The only deadline is iced's one-shot tooltip redraw; there is no ticker.
use crate::app::Msg;
use iced::{
    advanced::{
        layout, mouse, overlay, renderer,
        widget::{self, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    widget::tooltip,
    Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
thread_local! {static WARM:Cell<Option<Instant>>=const{Cell::new(None)};}
#[derive(Default)]
struct Hover {
    since: Option<Instant>,
}
pub struct Tooltip<'a> {
    inner: Option<tooltip::Tooltip<'a, Msg>>,
}
pub fn warm<'a>(inner: tooltip::Tooltip<'a, Msg>) -> Element<'a, Msg> {
    Element::new(Tooltip { inner: Some(inner) })
}
impl Widget<Msg, Theme, Renderer> for Tooltip<'_> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<Hover>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(Hover::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(
            self.inner.as_ref().unwrap() as &dyn Widget<Msg, Theme, Renderer>
        )]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.children[0].diff(self.inner.as_ref().unwrap() as &dyn Widget<Msg, Theme, Renderer>);
    }
    fn size(&self) -> Size<Length> {
        self.inner.as_ref().unwrap().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.inner.as_ref().unwrap().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let node = self
            .inner
            .as_mut()
            .unwrap()
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
        self.inner.as_mut().unwrap().operate(
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
        let now = if let Event::Window(iced::window::Event::RedrawRequested(now)) = event {
            *now
        } else {
            Instant::now()
        };
        let hover = tree.state.downcast_mut::<Hover>();
        let warmed = WARM.with(|v| {
            v.get()
                .is_some_and(|t| now.saturating_duration_since(t) < Duration::from_millis(250))
        });
        if cursor.is_over(layout.bounds()) {
            let since = *hover.since.get_or_insert(now);
            if warmed || now.saturating_duration_since(since) >= Duration::from_millis(400) {
                WARM.with(|v| v.set(Some(now)));
            }
        } else if let Some(since) = hover.since.take() {
            if warmed || now.saturating_duration_since(since) >= Duration::from_millis(400) {
                WARM.with(|v| v.set(Some(now)));
            }
        }
        self.inner = self.inner.take().map(|v| {
            v.delay(if warmed {
                Duration::ZERO
            } else {
                Duration::from_millis(400)
            })
        });
        self.inner.as_mut().unwrap().update(
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
        self.inner.as_ref().unwrap().draw(
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
        self.inner.as_ref().unwrap().mouse_interaction(
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
        self.inner.as_mut().unwrap().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}
