use super::controller::Controller;
use crate::app::Msg;
use iced::{
    advanced::{
        layout, mouse, overlay, renderer,
        widget::{self, Operation, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    keyboard::{self, key::Named},
    widget::scrollable,
    Element, Event, Length, Rectangle, Size, Vector,
};
use std::{cell::Cell, rc::Rc, time::Instant};
pub struct SmoothScrollable<'a> {
    inner: scrollable::Scrollable<'a, Msg>,
    horizontal: bool,
    viewport: Rc<Cell<Option<scrollable::Viewport>>>,
    correction: Rc<Cell<Vector>>,
    callback: Option<Box<dyn Fn(scrollable::Viewport) -> Msg + 'a>>,
}
#[derive(Default)]
struct State {
    x: Controller,
    y: Controller,
    offset: Vector,
    pending: Option<scrollable::Viewport>,
    last_frame: Option<Instant>,
}
impl State {
    fn cancel(&mut self) {
        self.x.cancel();
        self.y.cancel();
    }
}
#[derive(Default)]
struct Geometry {
    offset: Vector,
    bounds: Rectangle,
    content: Rectangle,
    set: Option<Vector>,
}
impl Operation for Geometry {
    fn traverse(&mut self, _: &mut dyn FnMut(&mut dyn Operation)) {}
    // Deliberately inspect only the outer native scrollable, not nested code blocks.
    fn scrollable(
        &mut self,
        _: Option<&widget::Id>,
        bounds: Rectangle,
        content: Rectangle,
        offset: Vector,
        state: &mut dyn widget::operation::Scrollable,
    ) {
        self.offset = offset;
        self.bounds = bounds;
        self.content = content;
        if let Some(v) = self.set {
            state.scroll_to(widget::operation::scrollable::AbsoluteOffset {
                x: Some(v.x),
                y: Some(v.y),
            });
        }
    }
}
impl<'a> SmoothScrollable<'a> {
    pub fn new(body: impl Into<Element<'a, Msg>>) -> Self {
        let viewport = Rc::new(Cell::new(None));
        let correction = Rc::new(Cell::new(Vector::ZERO));
        let capture = viewport.clone();
        let body = Element::new(super::subpixel::Subpixel {
            content: body.into(),
            correction: correction.clone(),
        });
        Self {
            inner: scrollable::Scrollable::new(body).on_scroll(move |v| {
                capture.set(Some(v));
                Msg::ScrollInternal
            }),
            horizontal: false,
            viewport,
            correction,
            callback: None,
        }
    }
    pub fn direction(mut self, direction: scrollable::Direction) -> Self {
        self.inner = self.inner.direction(direction);
        self
    }
    pub fn style(
        mut self,
        style: impl Fn(&iced::Theme, scrollable::Status) -> scrollable::Style + 'a,
    ) -> Self {
        self.inner = self.inner.style(style);
        self
    }
    fn sync(&self, tree: &mut Tree) {
        if let Some(v) = self.viewport.take() {
            tree.state.downcast_mut::<State>().pending = Some(v);
            let v = v.absolute_offset();
            tree.state.downcast_mut::<State>().offset = Vector::new(v.x, v.y);
        }
        let v = tree.state.downcast_ref::<State>().offset;
        self.correction
            .set(Vector::new(v.x.round() - v.x, v.y.round() - v.y));
    }
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.inner = self.inner.height(height);
        self
    }
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.inner = self.inner.id(id);
        self
    }
    pub fn on_scroll(mut self, f: impl Fn(scrollable::Viewport) -> Msg + 'a) -> Self {
        self.callback = Some(Box::new(f));
        self
    } // Same delegate signature as Widget::update; filter bookkeeping notifications
      // so settings/sidebar animation translates native content without app rebuilds.
    #[allow(clippy::too_many_arguments)]
    fn forward_update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Msg>,
        viewport: &Rectangle,
    ) {
        let mut messages = Vec::new();
        let mut local = Shell::new(&mut messages);
        self.inner.update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            &mut local,
            viewport,
        );
        merge_metadata(shell, local);
        for message in messages {
            if !matches!(message, Msg::ScrollInternal) {
                shell.publish(message);
            }
        }
    }
    fn geometry(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        set: Option<Vector>,
    ) -> Geometry {
        let mut op = Geometry {
            set,
            ..Geometry::default()
        };
        self.inner
            .operate(&mut tree.children[0], layout, renderer, &mut op);
        op
    }
}
impl<'a> From<SmoothScrollable<'a>> for Element<'a, Msg> {
    fn from(value: SmoothScrollable<'a>) -> Self {
        Self::new(value)
    }
}
impl Widget<Msg, iced::Theme, iced::Renderer> for SmoothScrollable<'_> {
    fn size(&self) -> Size<Length> {
        self.inner.size()
    }
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(
            &self.inner as &dyn Widget<Msg, iced::Theme, iced::Renderer>,
        )]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.inner as &dyn Widget<Msg, iced::Theme, iced::Renderer>]);
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.inner.layout(&mut tree.children[0], renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let v = tree.state.downcast_ref::<State>().offset;
        self.correction
            .set(Vector::new(v.x.round() - v.x, v.y.round() - v.y));
        self.inner.draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.inner
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.inner
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Msg, iced::Theme, iced::Renderer>> {
        self.inner.overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Msg>,
        viewport: &Rectangle,
    ) {
        if std::env::var_os("STICKTALK_SCROLL_BASELINE").is_some()
            && matches!(
                event,
                Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Lines { .. }
                })
            )
        {
            self.forward_update(
                tree, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
            self.sync(tree);
            return;
        }
        let mut before = self.geometry(tree, layout, renderer, None);
        let exact = tree.state.downcast_ref::<State>().offset;
        if before.offset == Vector::new(exact.x.round(), exact.y.round()) {
            before.offset = exact;
        } else {
            tree.state.downcast_mut::<State>().offset = before.offset;
        }
        if let Event::Window(iced::window::Event::RedrawRequested(now)) = event {
            let previous_redraw = shell.redraw_request();
            tree.state.downcast_mut::<State>().last_frame = Some(*now);
            let state = tree.state.downcast_mut::<State>();
            let delta = Vector::new(state.x.advance(*now), state.y.advance(*now));
            if delta != Vector::ZERO {
                self.geometry(tree, layout, renderer, Some(before.offset + delta));
            }
            let state = tree.state.downcast_mut::<State>();
            let active = state.x.active() || state.y.active();
            self.forward_update(
                tree, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
            self.sync(tree);
            if let Some(v) = tree.state.downcast_mut::<State>().pending.take() {
                if let Some(f) = &self.callback {
                    shell.publish(f(v));
                }
            }
            if active || delta != Vector::ZERO {
                let next = if active {
                    super::pacing::next(*now)
                } else {
                    iced::window::RedrawRequest::Wait
                };
                Shell::replace_redraw_request(shell, previous_redraw.min(next));
            }
            return;
        }
        let wheel = matches!(
            event,
            Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { .. }
            })
        );
        if matches!(
            event,
            Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Pixels { .. }
            }) | Event::Mouse(mouse::Event::ButtonPressed(_))
                | Event::Touch(_)
        ) {
            tree.state.downcast_mut::<State>().cancel();
        }
        // Native dispatch first gives nested scrollers and text inputs first refusal.
        // If this scrollable moved, undo ONLY its jump and defer its callback to redraw.
        let previous_redraw = shell.redraw_request();
        let mut messages = Vec::new();
        let mut local = Shell::new(&mut messages);
        self.inner.update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            &mut local,
            viewport,
        );
        let captured = local.is_event_captured();
        let mut after = self.geometry(tree, layout, renderer, None);
        if let Some(v) = self.viewport.get() {
            let v = v.absolute_offset();
            after.offset = Vector::new(v.x, v.y);
        }
        if wheel && after.offset != before.offset {
            self.geometry(tree, layout, renderer, Some(before.offset));
            // Wheel events publish only viewport notifications; suppress the provisional jump.
            drop(local);
            messages.clear();
            self.viewport.set(None);
            tree.state.downcast_mut::<State>().offset = before.offset;
            let Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x, y },
            }) = event
            else {
                unreachable!()
            };
            let (x, y) = if self.horizontal {
                (if *x == 0. { *y } else { *x }, 0.)
            } else {
                (*x, *y)
            };
            let state = tree.state.downcast_mut::<State>();
            for (c, offset, max, raw) in [
                (
                    &mut state.x,
                    before.offset.x,
                    (before.content.width - before.bounds.width).max(0.),
                    -x * 60.,
                ),
                (
                    &mut state.y,
                    before.offset.y,
                    (before.content.height - before.bounds.height).max(0.),
                    -y * 60.,
                ),
            ] {
                let pending = c.remaining();
                let amount = (offset + pending + raw).clamp(0., max) - offset - pending;
                c.push(amount, Instant::now());
            }
            shell.capture_event();
            let request = super::pacing::input_request(state.last_frame, Instant::now());
            Shell::replace_redraw_request(shell, previous_redraw.min(request));
            return;
        }
        merge_metadata(shell, local);
        for message in messages {
            if !matches!(message, Msg::ScrollInternal) {
                shell.publish(message);
            }
        }
        self.sync(tree);
        if after.offset != before.offset
            && matches!(
                event,
                Event::Mouse(mouse::Event::WheelScrolled { .. })
                    | Event::Mouse(mouse::Event::CursorMoved { .. })
                    | Event::Touch(_)
            )
        {
            let request = super::pacing::input_request(
                tree.state.downcast_ref::<State>().last_frame,
                Instant::now(),
            );
            Shell::replace_redraw_request(shell, previous_redraw.min(request));
        }
        if !captured && cursor.is_over(layout.bounds()) {
            if let Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(key),
                ..
            }) = event
            {
                let max = (before.content.height - before.bounds.height).max(0.);
                let state = tree.state.downcast_mut::<State>();
                match key {
                    Named::PageUp | Named::PageDown => {
                        let direction = if *key == Named::PageUp { -1. } else { 1. };
                        let pending = state.y.remaining();
                        let amount =
                            (before.offset.y + pending + direction * before.bounds.height * 0.9)
                                .clamp(0., max)
                                - before.offset.y
                                - pending;
                        state.y.push(amount, Instant::now());
                        shell.request_redraw();
                        shell.capture_event();
                    }
                    Named::Home | Named::End => {
                        state.cancel();
                        let y = if *key == Named::Home { 0. } else { max };
                        self.geometry(
                            tree,
                            layout,
                            renderer,
                            Some(Vector::new(before.offset.x, y)),
                        );
                        shell.request_redraw();
                        shell.capture_event();
                    }
                    _ => {}
                }
            }
        }
    }
}
fn merge_metadata(shell: &mut Shell<'_, Msg>, local: Shell<'_, Msg>) {
    if local.is_event_captured() {
        shell.capture_event();
    }
    if local.is_layout_invalid() {
        shell.invalidate_layout();
    }
    if local.are_widgets_invalid() {
        shell.invalidate_widgets();
    }
    shell.request_redraw_at(local.redraw_request());
    shell.request_input_method(local.input_method());
}
#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::widget::operation::{self, scrollable::AbsoluteOffset};
    use iced_runtime::user_interface::{Cache, UserInterface};
    use std::time::Duration;
    fn renderer() -> iced::Renderer {
        #[cfg(feature = "wgpu")]
        {
            iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
                crate::theme::regular(),
                iced::Pixels(15.),
            ))
        }
        #[cfg(not(feature = "wgpu"))]
        {
            iced_tiny_skia::Renderer::new(crate::theme::regular(), iced::Pixels(15.))
        }
    }
    #[derive(Default)]
    struct Offset(f32);
    impl Operation for Offset {
        fn traverse(&mut self, f: &mut dyn FnMut(&mut dyn Operation)) {
            f(self);
        }
        fn scrollable(
            &mut self,
            id: Option<&widget::Id>,
            _: Rectangle,
            _: Rectangle,
            v: Vector,
            _: &mut dyn widget::operation::Scrollable,
        ) {
            if id == Some(&widget::Id::new("test")) {
                self.0 = v.y;
            }
        }
    }
    fn position(
        ui: &mut UserInterface<'_, Msg, iced::Theme, iced::Renderer>,
        renderer: &iced::Renderer,
    ) -> f32 {
        let mut op = Offset::default();
        ui.operate(renderer, &mut op);
        op.0
    }
    fn dispatch(
        ui: &mut UserInterface<'_, Msg, iced::Theme, iced::Renderer>,
        renderer: &mut iced::Renderer,
        event: Event,
    ) -> iced_runtime::user_interface::State {
        ui.update(
            &[event],
            mouse::Cursor::Available(iced::Point::new(100., 100.)),
            renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut vec![],
        )
        .0
    }
    #[test]
    fn wheel_eases_pixels_cancel_tail_and_idle_stops_requesting_frames() {
        let body = iced::widget::space().width(300.).height(3000.);
        let exact = Rc::new(Cell::new(0.));
        let capture = exact.clone();
        let scroll = SmoothScrollable::new(body)
            .on_scroll(move |v| {
                capture.set(v.absolute_offset().y);
                Msg::Scroll(v)
            })
            .id("test")
            .height(200.);
        let mut renderer = renderer();
        let mut ui =
            UserInterface::build(scroll, Size::new(300., 200.), Cache::new(), &mut renderer);
        ui.operate(
            &renderer,
            &mut operation::scrollable::scroll_to::<()>(
                "test".into(),
                AbsoluteOffset {
                    x: None,
                    y: Some(100.),
                },
            ),
        );
        let now = Instant::now();
        dispatch(
            &mut ui,
            &mut renderer,
            Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0., y: -1. },
            }),
        );
        assert_eq!(position(&mut ui, &renderer), 100., "wheel must not jump");
        dispatch(
            &mut ui,
            &mut renderer,
            Event::Window(iced::window::Event::RedrawRequested(
                now + Duration::from_millis(70),
            )),
        );
        let mid = exact.get();
        assert!(mid > 140. && mid < 160., "ease midpoint: {mid}");
        dispatch(
            &mut ui,
            &mut renderer,
            Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Pixels { x: 0., y: -0.25 },
            }),
        );
        dispatch(
            &mut ui,
            &mut renderer,
            Event::Window(iced::window::Event::RedrawRequested(
                now + Duration::from_millis(80),
            )),
        );
        assert!(
            (exact.get() - mid - 0.25).abs() < 0.001,
            "pixel after {mid}: {}",
            position(&mut ui, &renderer)
        );
        dispatch(
            &mut ui,
            &mut renderer,
            Event::Window(iced::window::Event::RedrawRequested(
                now + Duration::from_secs(1),
            )),
        );
        dispatch(
            &mut ui,
            &mut renderer,
            Event::Window(iced::window::Event::RedrawRequested(
                now + Duration::from_millis(80),
            )),
        );
        assert!(
            (exact.get() - mid - 0.25).abs() < 0.001,
            "OS pixel momentum must not inherit wheel animation"
        );
        let state = dispatch(
            &mut ui,
            &mut renderer,
            Event::Window(iced::window::Event::RedrawRequested(
                now + Duration::from_secs(2),
            )),
        );
        assert!(
            matches!(
                state,
                iced_runtime::user_interface::State::Updated {
                    redraw_request: iced::window::RedrawRequest::Wait,
                    ..
                }
            ),
            "idle must not poll: {state:?}"
        );
    }
    #[test]
    fn wheel_tail_survives_tree_rebuild_and_anchor_operation() {
        let mut renderer = renderer();
        let mut cache = Cache::new();
        let now = Instant::now();
        for frame in 0..3 {
            let scroll = SmoothScrollable::new(iced::widget::space().width(300.).height(3000.))
                .id("test")
                .height(200.);
            let mut ui = UserInterface::build(scroll, Size::new(300., 200.), cache, &mut renderer);
            if frame == 0 {
                dispatch(
                    &mut ui,
                    &mut renderer,
                    Event::Mouse(mouse::Event::WheelScrolled {
                        delta: mouse::ScrollDelta::Lines { x: 0., y: -1. },
                    }),
                );
            }
            if frame == 1 {
                ui.operate(
                    &renderer,
                    &mut operation::scrollable::scroll_to::<()>(
                        "test".into(),
                        AbsoluteOffset {
                            x: None,
                            y: Some(500.),
                        },
                    ),
                );
            }
            dispatch(
                &mut ui,
                &mut renderer,
                Event::Window(iced::window::Event::RedrawRequested(
                    now + Duration::from_millis(frame * 80),
                )),
            );
            if frame == 2 {
                assert!(
                    (position(&mut ui, &renderer) - 560.).abs() < 0.1,
                    "prepend anchor and remaining wheel distance preserved"
                );
            }
            cache = ui.into_cache();
        }
    }
    #[test]
    fn high_refresh_frames_do_not_lose_fractional_distance() {
        let exact = Rc::new(Cell::new(0.));
        let capture = exact.clone();
        let scroll = SmoothScrollable::new(iced::widget::space().width(300.).height(3000.))
            .height(200.)
            .on_scroll(move |v| {
                capture.set(v.absolute_offset().y);
                Msg::Scroll(v)
            });
        let mut renderer = renderer();
        let mut ui =
            UserInterface::build(scroll, Size::new(300., 200.), Cache::new(), &mut renderer);
        dispatch(
            &mut ui,
            &mut renderer,
            Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0., y: -1. },
            }),
        );
        let now = Instant::now();
        for n in 1..=40 {
            dispatch(
                &mut ui,
                &mut renderer,
                Event::Window(iced::window::Event::RedrawRequested(
                    now + Duration::from_secs_f64(n as f64 / 240.),
                )),
            );
        }
        assert!(
            (exact.get() - 60.).abs() < 0.001,
            "fractional frame deltas were rounded away: {}",
            exact.get()
        );
    }
    #[test]
    fn bookkeeping_callbacks_do_not_rebuild_settings_on_each_frame() {
        let scroll =
            SmoothScrollable::new(iced::widget::space().width(300.).height(3000.)).height(200.);
        let mut renderer = renderer();
        let mut ui =
            UserInterface::build(scroll, Size::new(300., 200.), Cache::new(), &mut renderer);
        let mut messages = vec![];
        let now = Instant::now();
        let _ = ui.update(
            &[
                Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Lines { x: 0., y: -1. },
                }),
                Event::Window(iced::window::Event::RedrawRequested(
                    now + Duration::from_millis(70),
                )),
            ],
            mouse::Cursor::Available(iced::Point::new(100., 100.)),
            &mut renderer,
            &mut iced::advanced::graphics::core::clipboard::Null,
            &mut messages,
        );
        assert!(
            messages.is_empty(),
            "internal native viewport tracking must stay inside the widget"
        );
    }
    fn key_event(key: Named) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(key),
            modified_key: keyboard::Key::Named(key),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        })
    }
    #[test]
    fn page_keys_animate_and_home_end_seek_directly() {
        let scroll = SmoothScrollable::new(iced::widget::space().width(300.).height(3000.))
            .id("test")
            .height(200.);
        let mut renderer = renderer();
        let mut ui =
            UserInterface::build(scroll, Size::new(300., 200.), Cache::new(), &mut renderer);
        ui.operate(
            &renderer,
            &mut operation::scrollable::scroll_to::<()>(
                "test".into(),
                AbsoluteOffset {
                    x: None,
                    y: Some(500.),
                },
            ),
        );
        dispatch(&mut ui, &mut renderer, key_event(Named::PageUp));
        assert_eq!(position(&mut ui, &renderer), 500.);
        dispatch(
            &mut ui,
            &mut renderer,
            Event::Window(iced::window::Event::RedrawRequested(
                Instant::now() + Duration::from_millis(200),
            )),
        );
        assert_eq!(position(&mut ui, &renderer), 320.);
        dispatch(&mut ui, &mut renderer, key_event(Named::Home));
        assert_eq!(position(&mut ui, &renderer), 0.);
        dispatch(&mut ui, &mut renderer, key_event(Named::End));
        assert_eq!(position(&mut ui, &renderer), 2800.);
    }
}
