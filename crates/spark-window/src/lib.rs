//! Spark Engine - Window and input: maps winit events to [`spark_core::Input`].

pub use winit;

mod gamepad;
pub use gamepad::Gamepads;

use spark_core::{Input, Key, MouseButton, Vec2};
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

/// Feeds a window event into `input`. Returns true if it was an input event.
pub fn handle_input_event(input: &mut Input, event: &WindowEvent) -> bool {
    match event {
        WindowEvent::KeyboardInput { event, .. } => {
            if let PhysicalKey::Code(code) = event.physical_key {
                if let Some(key) = map_key(code) {
                    input.key_event(key, event.state == ElementState::Pressed);
                }
            }
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
        _ => None,
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
        _ => return None,
    })
}
