//! Spark Engine - Window and input: maps winit events to [`spark_core::Input`].

pub use winit;

mod gamepad;
pub use gamepad::Gamepads;

use spark_core::{Input, Key, MouseButton, Vec2};
use winit::event::{ElementState, Ime, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

/// Feeds a window event into `input`. Returns true if it was an input event.
pub fn handle_input_event(input: &mut Input, event: &WindowEvent) -> bool {
    match event {
        WindowEvent::KeyboardInput { event, .. } => {
            let pressed = event.state == ElementState::Pressed;
            if let PhysicalKey::Code(code) = event.physical_key {
                if let Some(key) = map_key(code) {
                    input.key_event(key, pressed);
                }
            }
            if pressed {
                if let Some(text) = &event.text {
                    input.text_event(text);
                }
            }
            true
        }
        // Composed text from an input method (needs `window.set_ime_allowed(true)`).
        WindowEvent::Ime(Ime::Commit(text)) => {
            input.text_event(text);
            true
        }
        WindowEvent::MouseInput { state, button, .. } => {
            if let Some(b) = map_mouse_button(*button) {
                input.mouse_button_event(b, *state == ElementState::Pressed);
            }
            true
        }
        WindowEvent::CursorMoved { position, .. } => {
            input.mouse_moved(Vec2::new(position.x as f32, position.y as f32));
            true
        }
        WindowEvent::MouseWheel { delta, .. } => {
            input.wheel_event(match delta {
                MouseScrollDelta::LineDelta(_, y) => *y,
                MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
            });
            true
        }
        WindowEvent::Focused(false) => {
            input.release_all();
            true
        }
        _ => false,
    }
}

pub fn map_mouse_button(button: winit::event::MouseButton) -> Option<MouseButton> {
    match button {
        winit::event::MouseButton::Left => Some(MouseButton::Left),
        winit::event::MouseButton::Right => Some(MouseButton::Right),
        winit::event::MouseButton::Middle => Some(MouseButton::Middle),
        winit::event::MouseButton::Back => Some(MouseButton::Back),
        winit::event::MouseButton::Forward => Some(MouseButton::Forward),
        winit::event::MouseButton::Other(_) => None,
    }
}

pub fn map_key(code: KeyCode) -> Option<Key> {
    use KeyCode as C;
    Some(match code {
        C::KeyA => Key::A,
        C::KeyB => Key::B,
        C::KeyC => Key::C,
        C::KeyD => Key::D,
        C::KeyE => Key::E,
        C::KeyF => Key::F,
        C::KeyG => Key::G,
        C::KeyH => Key::H,
        C::KeyI => Key::I,
        C::KeyJ => Key::J,
        C::KeyK => Key::K,
        C::KeyL => Key::L,
        C::KeyM => Key::M,
        C::KeyN => Key::N,
        C::KeyO => Key::O,
        C::KeyP => Key::P,
        C::KeyQ => Key::Q,
        C::KeyR => Key::R,
        C::KeyS => Key::S,
        C::KeyT => Key::T,
        C::KeyU => Key::U,
        C::KeyV => Key::V,
        C::KeyW => Key::W,
        C::KeyX => Key::X,
        C::KeyY => Key::Y,
        C::KeyZ => Key::Z,
        C::Digit0 => Key::Num0,
        C::Digit1 => Key::Num1,
        C::Digit2 => Key::Num2,
        C::Digit3 => Key::Num3,
        C::Digit4 => Key::Num4,
        C::Digit5 => Key::Num5,
        C::Digit6 => Key::Num6,
        C::Digit7 => Key::Num7,
        C::Digit8 => Key::Num8,
        C::Digit9 => Key::Num9,
        C::Space => Key::Space,
        C::Enter | C::NumpadEnter => Key::Enter,
        C::Escape => Key::Escape,
        C::Tab => Key::Tab,
        C::Backspace => Key::Backspace,
        C::ArrowLeft => Key::Left,
        C::ArrowRight => Key::Right,
        C::ArrowUp => Key::Up,
        C::ArrowDown => Key::Down,
        C::ShiftLeft => Key::LShift,
        C::ShiftRight => Key::RShift,
        C::ControlLeft => Key::LCtrl,
        C::ControlRight => Key::RCtrl,
        C::AltLeft => Key::LAlt,
        C::AltRight => Key::RAlt,
        C::F1 => Key::F1,
        C::F2 => Key::F2,
        C::F3 => Key::F3,
        C::F4 => Key::F4,
        C::F5 => Key::F5,
        C::F6 => Key::F6,
        C::F7 => Key::F7,
        C::F8 => Key::F8,
        C::F9 => Key::F9,
        C::F10 => Key::F10,
        C::F11 => Key::F11,
        C::F12 => Key::F12,
        C::F13 => Key::F13,
        C::F14 => Key::F14,
        C::F15 => Key::F15,
        C::F16 => Key::F16,
        C::F17 => Key::F17,
        C::F18 => Key::F18,
        C::F19 => Key::F19,
        C::F20 => Key::F20,
        C::F21 => Key::F21,
        C::F22 => Key::F22,
        C::F23 => Key::F23,
        C::F24 => Key::F24,
        C::Minus => Key::Minus,
        C::Equal => Key::Equal,
        C::BracketLeft => Key::BracketLeft,
        C::BracketRight => Key::BracketRight,
        C::Backslash | C::IntlBackslash => Key::Backslash,
        C::Semicolon => Key::Semicolon,
        C::Quote => Key::Quote,
        C::Comma => Key::Comma,
        C::Period => Key::Period,
        C::Slash => Key::Slash,
        C::Backquote => Key::Backquote,
        C::Insert => Key::Insert,
        C::Delete => Key::Delete,
        C::Home => Key::Home,
        C::End => Key::End,
        C::PageUp => Key::PageUp,
        C::PageDown => Key::PageDown,
        C::CapsLock => Key::CapsLock,
        C::NumLock => Key::NumLock,
        C::ScrollLock => Key::ScrollLock,
        C::PrintScreen => Key::PrintScreen,
        C::Pause => Key::Pause,
        C::ContextMenu => Key::Menu,
        C::SuperLeft | C::Meta => Key::LSuper,
        C::SuperRight => Key::RSuper,
        C::Numpad0 => Key::Numpad0,
        C::Numpad1 => Key::Numpad1,
        C::Numpad2 => Key::Numpad2,
        C::Numpad3 => Key::Numpad3,
        C::Numpad4 => Key::Numpad4,
        C::Numpad5 => Key::Numpad5,
        C::Numpad6 => Key::Numpad6,
        C::Numpad7 => Key::Numpad7,
        C::Numpad8 => Key::Numpad8,
        C::Numpad9 => Key::Numpad9,
        C::NumpadAdd => Key::NumpadAdd,
        C::NumpadSubtract => Key::NumpadSubtract,
        C::NumpadMultiply | C::NumpadStar => Key::NumpadMultiply,
        C::NumpadDivide => Key::NumpadDivide,
        C::NumpadDecimal | C::NumpadComma => Key::NumpadDecimal,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_a_keycode() {
        use KeyCode as C;
        // Every Spark key must be reachable from some winit KeyCode.
        let codes = [
            C::KeyA, C::KeyB, C::KeyC, C::KeyD, C::KeyE, C::KeyF, C::KeyG, C::KeyH, C::KeyI, C::KeyJ, C::KeyK, C::KeyL, C::KeyM,
            C::KeyN, C::KeyO, C::KeyP, C::KeyQ, C::KeyR, C::KeyS, C::KeyT, C::KeyU, C::KeyV, C::KeyW, C::KeyX, C::KeyY, C::KeyZ,
            C::Digit0, C::Digit1, C::Digit2, C::Digit3, C::Digit4, C::Digit5, C::Digit6, C::Digit7, C::Digit8, C::Digit9,
            C::Space, C::Enter, C::Escape, C::Tab, C::Backspace, C::ArrowLeft, C::ArrowRight, C::ArrowUp, C::ArrowDown,
            C::ShiftLeft, C::ShiftRight, C::ControlLeft, C::ControlRight, C::AltLeft, C::AltRight,
            C::F1, C::F2, C::F3, C::F4, C::F5, C::F6, C::F7, C::F8, C::F9, C::F10, C::F11, C::F12,
            C::F13, C::F14, C::F15, C::F16, C::F17, C::F18, C::F19, C::F20, C::F21, C::F22, C::F23, C::F24,
            C::Minus, C::Equal, C::BracketLeft, C::BracketRight, C::Backslash, C::Semicolon, C::Quote, C::Comma,
            C::Period, C::Slash, C::Backquote, C::Insert, C::Delete, C::Home, C::End, C::PageUp, C::PageDown,
            C::CapsLock, C::NumLock, C::ScrollLock, C::PrintScreen, C::Pause, C::ContextMenu, C::SuperLeft, C::SuperRight,
            C::Numpad0, C::Numpad1, C::Numpad2, C::Numpad3, C::Numpad4, C::Numpad5, C::Numpad6, C::Numpad7, C::Numpad8,
            C::Numpad9, C::NumpadAdd, C::NumpadSubtract, C::NumpadMultiply, C::NumpadDivide, C::NumpadDecimal,
        ];
        let mapped: std::collections::HashSet<Key> = codes.iter().filter_map(|c| map_key(*c)).collect();
        for k in Key::ALL {
            assert!(mapped.contains(k), "no KeyCode maps to {k:?}");
        }
        assert_eq!(map_key(C::NumpadEnter), Some(Key::Enter), "numpad enter stays 'enter' (backward compatible)");
        assert_eq!(map_mouse_button(winit::event::MouseButton::Back), Some(MouseButton::Back));
    }

    #[test]
    fn ime_commit_is_text() {
        let mut input = Input::default();
        assert!(handle_input_event(&mut input, &WindowEvent::Ime(Ime::Commit("日本".into()))));
        assert_eq!(input.text(), "日本");
    }
}
