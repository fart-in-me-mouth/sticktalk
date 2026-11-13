//! Focused stream input capture. Escape uses one deadline, never a polling loop.
use super::*;
use crate::voice_ui::Action;
use iced::widget::shader;
use iced::{
    keyboard::{
        self,
        key::{Code, Named, Physical},
    },
    mouse, Event, Rectangle,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct Capture {
    escape: Option<Instant>,
    released: bool,
    focused: bool,
    skip_click_release: bool,
    grant: Option<u64>,
}
pub struct Playback {
    pub frame: Arc<sticktalk_media::DisplayFrame>,
    pub controlling: Option<u64>,
}
impl shader::Program<Msg> for Playback {
    type State = Capture;
    type Primitive = sticktalk::voice::video::Frame;
    fn draw(&self, _: &Capture, _: mouse::Cursor, _: Rectangle) -> Self::Primitive {
        sticktalk::voice::video::Frame(self.frame.clone())
    }
    fn update(
        &self,
        state: &mut Capture,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<Msg>> {
        if state.grant != self.controlling {
            *state = Capture {
                grant: self.controlling,
                ..Default::default()
            };
        }
        capture_event(self.controlling.is_some(), state, event, bounds, cursor)
    }
}
fn capture_event(
    controlling: bool,
    state: &mut Capture,
    event: &Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
) -> Option<shader::Action<Msg>> {
    if !controlling {
        state.escape = None;
        state.released = false;
        state.focused = false;
        state.skip_click_release = false;
        return None;
    }
    if state.released {
        return None;
    }
    let release = |state: &mut Capture| {
        state.released = true;
        state.escape = None;
        shader::Action::publish(Msg::Call(Action::Revoke)).and_capture()
    };
    if let Event::Window(iced::window::Event::Unfocused) = event {
        return Some(release(state));
    }
    if !state.focused {
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) {
            if let Some(p) = cursor.position_in(bounds) {
                state.focused = true;
                state.skip_click_release = true;
                return Some(
                    shader::Action::publish(Msg::Call(Action::ViewerInput(ControlInput::Move {
                        x: (p.x / bounds.width * 65535.).clamp(0., 65535.) as u16,
                        y: (p.y / bounds.height * 65535.).clamp(0., 65535.) as u16,
                    })))
                    .and_capture(),
                );
            }
        }
        return None;
    }
    if state.skip_click_release
        && matches!(
            event,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        )
    {
        state.skip_click_release = false;
        return Some(shader::Action::capture());
    }
    if let Event::Window(iced::window::Event::RedrawRequested(now)) = event {
        if state
            .escape
            .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_secs(1))
        {
            return Some(release(state));
        }
    }
    let key_event = match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key, physical_key, ..
        }) => Some((key, physical_key, true)),
        Event::Keyboard(keyboard::Event::KeyReleased {
            key, physical_key, ..
        }) => Some((key, physical_key, false)),
        _ => None,
    };
    if let Some((key, physical, down)) = key_event {
        if *key == keyboard::Key::Named(Named::Escape) {
            if down {
                let at = *state.escape.get_or_insert_with(Instant::now);
                return Some(
                    shader::Action::request_redraw_at(at + Duration::from_secs(1)).and_capture(),
                );
            }
            state.escape = None;
            return Some(shader::Action::publish(Msg::Call(Action::ViewerEscapeTap)).and_capture());
        }
        if let Physical::Code(code) = physical {
            if let Some(usage) = usage(*code) {
                return Some(
                    shader::Action::publish(Msg::Call(Action::ViewerInput(ControlInput::Key {
                        usage,
                        down,
                    })))
                    .and_capture(),
                );
            }
        }
        return Some(shader::Action::capture());
    }
    if let Event::Mouse(event) = event {
        if matches!(event, mouse::Event::CursorLeft) {
            return Some(release(state));
        }
        if matches!(event, mouse::Event::CursorMoved { .. }) && cursor.position_in(bounds).is_none()
        {
            return Some(release(state));
        }
        if let Some(p) = cursor.position_in(bounds) {
            let input = match event {
                mouse::Event::CursorMoved { .. } => ControlInput::Move {
                    x: (p.x / bounds.width * 65535.).clamp(0., 65535.) as u16,
                    y: (p.y / bounds.height * 65535.).clamp(0., 65535.) as u16,
                },
                mouse::Event::ButtonPressed(b) | mouse::Event::ButtonReleased(b) => {
                    ControlInput::Button {
                        button: match b {
                            mouse::Button::Left => 0,
                            mouse::Button::Right => 1,
                            mouse::Button::Middle => 2,
                            mouse::Button::Back => 3,
                            mouse::Button::Forward => 4,
                            _ => return Some(shader::Action::capture()),
                        },
                        down: matches!(event, mouse::Event::ButtonPressed(_)),
                    }
                }
                mouse::Event::WheelScrolled { delta } => {
                    let (x, y) = match *delta {
                        mouse::ScrollDelta::Lines { x, y } => (x * 120., y * 120.),
                        mouse::ScrollDelta::Pixels { x, y } => (x, y),
                    };
                    ControlInput::Wheel {
                        x: x.clamp(-1200., 1200.) as i16,
                        y: y.clamp(-1200., 1200.) as i16,
                    }
                }
                _ => return None,
            };
            return Some(
                shader::Action::publish(Msg::Call(Action::ViewerInput(input))).and_capture(),
            );
        }
    }
    None
}

fn usage(code: Code) -> Option<u16> {
    // Explicit mapping, never casts platform scan-code enums onto the wire.
    Some(match code {
        Code::KeyA => 4,
        Code::KeyB => 5,
        Code::KeyC => 6,
        Code::KeyD => 7,
        Code::KeyE => 8,
        Code::KeyF => 9,
        Code::KeyG => 10,
        Code::KeyH => 11,
        Code::KeyI => 12,
        Code::KeyJ => 13,
        Code::KeyK => 14,
        Code::KeyL => 15,
        Code::KeyM => 16,
        Code::KeyN => 17,
        Code::KeyO => 18,
        Code::KeyP => 19,
        Code::KeyQ => 20,
        Code::KeyR => 21,
        Code::KeyS => 22,
        Code::KeyT => 23,
        Code::KeyU => 24,
        Code::KeyV => 25,
        Code::KeyW => 26,
        Code::KeyX => 27,
        Code::KeyY => 28,
        Code::KeyZ => 29,
        Code::Digit1 => 30,
        Code::Digit2 => 31,
        Code::Digit3 => 32,
        Code::Digit4 => 33,
        Code::Digit5 => 34,
        Code::Digit6 => 35,
        Code::Digit7 => 36,
        Code::Digit8 => 37,
        Code::Digit9 => 38,
        Code::Digit0 => 39,
        Code::Enter => 40,
        Code::Backspace => 42,
        Code::Tab => 43,
        Code::Space => 44,
        Code::Minus => 45,
        Code::Equal => 46,
        Code::BracketLeft => 47,
        Code::BracketRight => 48,
        Code::Backslash => 49,
        Code::Semicolon => 51,
        Code::Quote => 52,
        Code::Backquote => 53,
        Code::Comma => 54,
        Code::Period => 55,
        Code::Slash => 56,
        Code::CapsLock => 57,
        Code::F1 => 58,
        Code::F2 => 59,
        Code::F3 => 60,
        Code::F4 => 61,
        Code::F5 => 62,
        Code::F6 => 63,
        Code::F7 => 64,
        Code::F8 => 65,
        Code::F9 => 66,
        Code::F10 => 67,
        Code::F11 => 68,
        Code::F12 => 69,
        Code::Insert => 73,
        Code::Home => 74,
        Code::PageUp => 75,
        Code::Delete => 76,
        Code::End => 77,
        Code::PageDown => 78,
        Code::ArrowRight => 79,
        Code::ArrowLeft => 80,
        Code::ArrowDown => 81,
        Code::ArrowUp => 82,
        Code::ControlLeft => 0xe0,
        Code::ShiftLeft => 0xe1,
        Code::AltLeft => 0xe2,
        Code::ControlRight => 0xe4,
        Code::ShiftRight => 0xe5,
        Code::AltRight => 0xe6,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escape_deadline_is_one_second_and_focus_loss_releases_capture() {
        let now = Instant::now();
        let mut state = Capture {
            escape: Some(now),
            released: false,
            focused: true,
            ..Default::default()
        };
        let bounds = Rectangle::new(iced::Point::ORIGIN, iced::Size::new(1920., 1080.));
        let early = Event::Window(iced::window::Event::RedrawRequested(
            now + Duration::from_millis(999),
        ));
        assert!(
            capture_event(true, &mut state, &early, bounds, mouse::Cursor::Unavailable).is_none()
        );
        let deadline = Event::Window(iced::window::Event::RedrawRequested(
            now + Duration::from_secs(1),
        ));
        let (message, _, status) = capture_event(
            true,
            &mut state,
            &deadline,
            bounds,
            mouse::Cursor::Unavailable,
        )
        .unwrap()
        .into_inner();
        assert!(matches!(message, Some(Msg::Call(Action::Revoke))));
        assert_eq!(status, iced::event::Status::Captured);
        state.released = false;
        let (message, _, _) = capture_event(
            true,
            &mut state,
            &Event::Window(iced::window::Event::Unfocused),
            bounds,
            mouse::Cursor::Unavailable,
        )
        .unwrap()
        .into_inner();
        assert!(matches!(message, Some(Msg::Call(Action::Revoke))));
    }
    #[test]
    fn viewer_pointer_is_normalized_only_inside_the_video_rectangle() {
        let mut state = Capture {
            focused: true,
            ..Default::default()
        };
        let bounds = Rectangle::new(iced::Point::new(10., 20.), iced::Size::new(1920., 1080.));
        let p = iced::Point::new(970., 560.);
        let event = Event::Mouse(mouse::Event::CursorMoved { position: p });
        let (message, _, _) = capture_event(
            true,
            &mut state,
            &event,
            bounds,
            mouse::Cursor::Available(p),
        )
        .unwrap()
        .into_inner();
        assert!(matches!(
            message,
            Some(Msg::Call(Action::ViewerInput(ControlInput::Move {
                x: 32767,
                y: 32767
            })))
        ));
        let outside = iced::Point::new(0., 0.);
        let (message, _, _) = capture_event(
            true,
            &mut state,
            &event,
            bounds,
            mouse::Cursor::Available(outside),
        )
        .unwrap()
        .into_inner();
        assert!(matches!(message, Some(Msg::Call(Action::Revoke))));
        assert_eq!(usage(Code::KeyA), Some(4));
        assert_eq!(usage(Code::KeyZ), Some(29));
    }
    #[test]
    fn new_grant_requires_a_local_video_click_before_input_capture() {
        let mut state = Capture::default();
        let bounds = Rectangle::new(iced::Point::ORIGIN, iced::Size::new(1920., 1080.));
        let p = iced::Point::new(960., 540.);
        let cursor = mouse::Cursor::Available(p);
        assert!(capture_event(
            true,
            &mut state,
            &Event::Mouse(mouse::Event::CursorMoved { position: p }),
            bounds,
            cursor
        )
        .is_none());
        let (message, _, _) = capture_event(
            true,
            &mut state,
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            cursor,
        )
        .unwrap()
        .into_inner();
        assert!(matches!(
            message,
            Some(Msg::Call(Action::ViewerInput(ControlInput::Move { .. })))
        ));
        assert!(state.focused);
        let (message, _, _) = capture_event(
            true,
            &mut state,
            &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            bounds,
            cursor,
        )
        .unwrap()
        .into_inner();
        assert!(message.is_none());
    }
}
