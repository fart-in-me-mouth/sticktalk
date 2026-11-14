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
    Element, Event, Length, Padding, Rectangle, Renderer, Size, Theme, Vector,
};
use std::{cell::Cell, rc::Rc};
thread_local! { static KEYBOARD_FOCUS: Cell<bool> = const { Cell::new(false) }; }
struct Focus {
    focused: bool,
    visible: bool,
    window_focused: bool,
}
impl Default for Focus {
    fn default() -> Self {
        Self {
            focused: false,
            visible: false,
            window_focused: true,
        }
    }
}
impl operation::Focusable for Focus {
    fn is_focused(&self) -> bool {
        self.focused
    }
    fn focus(&mut self) {
        self.focused = true;
        self.visible = KEYBOARD_FOCUS.with(Cell::get);
    }
    fn unfocus(&mut self) {
        self.focused = false;
        self.visible = false;
    }
}
pub struct Button<'a> {
    inner: native::Button<'a, Msg>,
    message: Option<Msg>,
    centered: bool,
    focus_visible: Rc<Cell<bool>>,
}
pub fn button<'a>(content: impl Into<Element<'a, Msg>>) -> Button<'a> {
    let focus_visible = Rc::new(Cell::new(false));
    let visible = focus_visible.clone();
    Button {
        inner: native::Button::new(content)
            .style(move |theme, status| t::button(theme, focus_status(status, visible.get()))),
        focus_visible,
        message: None,
        centered: false,
    }
}
// Pointer activation never creates a focus decoration. Keyboard traversal uses
// the widget's own hover fill instead of painting a second, unrelated outline.
fn focus_status(status: native::Status, visible: bool) -> native::Status {
    if visible && status == native::Status::Active {
        native::Status::Hovered
    } else {
        status
    }
}
/// Native, wrapping Tab order. iced's default traversal leaves an empty stop
/// after the final widget; modal scopes should wrap directly to their first item.
pub fn cycle(previous: bool) -> impl widget::Operation<()> {
    cycle_visible(previous, true)
}
fn cycle_visible(previous: bool, visible: bool) -> impl widget::Operation<()> {
    struct At {
        target: Option<usize>,
        current: usize,
        visible: bool,
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
                KEYBOARD_FOCUS.with(|flag| {
                    let previous = flag.replace(self.visible);
                    state.focus();
                    flag.set(previous);
                });
            } else {
                state.unfocus();
            }
            self.current += 1;
        }
    }
    fn next(count: operation::focusable::Count) -> At {
        at(count, false, true)
    }
    fn previous_item(count: operation::focusable::Count) -> At {
        at(count, true, true)
    }
    fn hidden(count: operation::focusable::Count) -> At {
        at(count, false, false)
    }
    fn at(count: operation::focusable::Count, previous: bool, visible: bool) -> At {
        At {
            target: (count.total > 0).then(|| match count.focused {
                Some(index) if previous => (index + count.total - 1) % count.total,
                Some(index) => (index + 1) % count.total,
                None if previous => count.total - 1,
                None => 0,
            }),
            current: 0,
            visible,
        }
    }
    let choose: fn(operation::focusable::Count) -> At = if !visible {
        hidden
    } else if previous {
        previous_item
    } else {
        next
    };
    operation::then(operation::focusable::count(), choose)
}
pub fn focus_dialog() -> iced::Task<Msg> {
    iced::widget::operation::focus("closed-composer").chain(
        widget::operate(operation::scope(
            "dialog-scope".into(),
            cycle_visible(false, false),
        ))
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
        let visible = self.focus_visible.clone();
        self.inner = self
            .inner
            .style(move |theme, status| style(theme, focus_status(status, visible.get())));
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
        match event {
            Event::Window(iced::window::Event::Unfocused) => focus.window_focused = false,
            Event::Window(iced::window::Event::Focused) => focus.window_focused = true,
            _ => {}
        }
        let cursor = if focus.window_focused {
            cursor
        } else {
            mouse::Cursor::Unavailable
        };

        if matches!(
            event,
            Event::Window(iced::window::Event::Unfocused) | Event::Mouse(mouse::Event::CursorLeft)
        ) {
            if matches!(event, Event::Window(iced::window::Event::Unfocused)) {
                focus.focused = false;
                focus.visible = false;
            }
            // iced's native button does not cancel a press on focus loss.
            // Keep child layout trees but drop the latched native press state.
            tree.children[0].state = self.inner.state();
            shell.request_redraw();
        }
        if let Event::Mouse(mouse::Event::ButtonPressed(_)) = event {
            let focused = self.message.is_some() && cursor.is_over(layout.bounds());
            if focus.focused != focused || focus.visible {
                focus.focused = focused;
                focus.visible = false;
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
        let focus = tree.state.downcast_ref::<Focus>();
        self.focus_visible
            .set(focus.window_focused && focus.focused && focus.visible && self.message.is_some());
        self.inner.draw(
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

#[cfg(test)]
mod focus_visible_tests {
    use super::*;
    use iced_runtime::user_interface::{Cache, UserInterface};
    fn renderer() -> Renderer {
        #[cfg(feature = "wgpu")]
        {
            Renderer::Secondary(iced_tiny_skia::Renderer::new(
                t::regular(),
                iced::Pixels(t::BODY),
            ))
        }
        #[cfg(not(feature = "wgpu"))]
        {
            iced_tiny_skia::Renderer::new(t::regular(), iced::Pixels(t::BODY))
        }
    }
    #[test]
    fn keyboard_focus_is_a_fill_and_mouse_or_blur_clears_it() {
        crate::headless::load_fonts();
        let mut renderer = renderer();
        let mut ui = UserInterface::build(
            button(iced::widget::text("Channel"))
                .on_press(Msg::Activity)
                .width(200.)
                .height(34.)
                .style(t::navigation(false)),
            Size::new(200., 34.),
            Cache::new(),
            &mut renderer,
        );
        let mut operation: Box<dyn widget::Operation<()>> = Box::new(cycle(false));
        loop {
            ui.operate(&renderer, operation.as_mut());
            match operation.finish() {
                operation::Outcome::Chain(next) => operation = next,
                _ => break,
            }
        }
        fn sample(
            ui: &mut UserInterface<'_, Msg, Theme, Renderer>,
            renderer: &mut Renderer,
        ) -> [u8; 3] {
            let mut messages = Vec::new();
            let _ = ui.update(
                &[Event::Window(iced::window::Event::RedrawRequested(
                    std::time::Instant::now(),
                ))],
                mouse::Cursor::Unavailable,
                renderer,
                &mut iced::advanced::graphics::core::clipboard::Null,
                &mut messages,
            );
            ui.draw(
                renderer,
                &Theme::Dark,
                &renderer::Style {
                    text_color: t::TEXT_PRIMARY,
                },
                mouse::Cursor::Unavailable,
            );
            #[cfg(feature = "wgpu")]
            let Renderer::Secondary(renderer) = renderer
            else {
                unreachable!()
            };
            let p = iced_tiny_skia::window::compositor::screenshot(
                renderer,
                &iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(200, 34), 1.),
                t::BG_SIDEBAR,
            );
            let i = (16 * 200 + 160) * 4;
            [p[i], p[i + 1], p[i + 2]]
        }
        assert_eq!(sample(&mut ui, &mut renderer), [0x23, 0x23, 0x27]);
        let mut messages = Vec::new();
        let _ = ui.update(
            &[
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ],
            mouse::Cursor::Available(iced::Point::new(80., 16.)),
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        let _ = ui.update(
            &[Event::Mouse(mouse::Event::CursorLeft)],
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        assert_eq!(sample(&mut ui, &mut renderer), [0x17, 0x17, 0x1a]);
        // Focus loss while held must also cancel iced's native pressed state.
        let _ = ui.update(
            &[Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))],
            mouse::Cursor::Available(iced::Point::new(80., 16.)),
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        let _ = ui.update(
            &[Event::Window(iced::window::Event::Unfocused)],
            mouse::Cursor::Available(iced::Point::new(80., 16.)),
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        assert_eq!(sample(&mut ui, &mut renderer), [0x17, 0x17, 0x1a]);
    }
    #[test]
    fn programmatic_focus_is_not_focus_visible() {
        let mut state = Focus::default();
        operation::Focusable::focus(&mut state);
        assert!(state.focused);
        assert!(!state.visible);
    }
}
