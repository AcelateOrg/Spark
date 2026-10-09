//! Gamepads through gilrs (XInput / DirectInput on Windows, evdev on Linux, IOKit on macOS).

use gilrs::{Axis, Button, EventType, Gilrs};
use spark_core::{Input, PadAxis, PadButton};

/// Polls connected gamepads into [`Input`]. Without a usable backend it does nothing.
pub struct Gamepads {
    gilrs: Option<Gilrs>,
}

impl Gamepads {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(g) => Some(g),
            Err(gilrs::Error::NotImplemented(g)) => Some(g),
            Err(e) => {
                log::warn!("gamepads disabled: {e}");
                None
            }
        };
        Self { gilrs }
    }

    /// Registers pads that were connected before the game started.
    pub fn init(&mut self, input: &mut Input) {
        let Some(g) = &self.gilrs else { return };
        for (id, pad) in g.gamepads() {
            log::info!("gamepad connected: {}", pad.name());
            input.pad_connected(usize::from(id), pad.name());
        }
    }

    /// Call once per frame, before the game update.
    pub fn poll(&mut self, input: &mut Input) {
        let Some(g) = &mut self.gilrs else { return };
        while let Some(ev) = g.next_event() {
            let id = usize::from(ev.id);
            match ev.event {
                EventType::Connected => {
                    let name = g.gamepad(ev.id).name().to_string();
                    log::info!("gamepad connected: {name}");
                    input.pad_connected(id, name);
                }
                EventType::Disconnected => {
                    log::info!("gamepad disconnected");
                    input.pad_disconnected(id);
                }
                EventType::ButtonPressed(b, _) => button(input, id, b, true),
                EventType::ButtonReleased(b, _) => button(input, id, b, false),
                EventType::ButtonChanged(Button::LeftTrigger2, v, _) => input.pad_axis_event(id, PadAxis::LT, v),
                EventType::ButtonChanged(Button::RightTrigger2, v, _) => input.pad_axis_event(id, PadAxis::RT, v),
                EventType::AxisChanged(a, v, _) => {
                    let axis = match a {
                        Axis::LeftStickX => PadAxis::LeftX,
                        Axis::LeftStickY => PadAxis::LeftY,
                        Axis::RightStickX => PadAxis::RightX,
                        Axis::RightStickY => PadAxis::RightY,
                        Axis::LeftZ => PadAxis::LT,
                        Axis::RightZ => PadAxis::RT,
                        _ => continue,
                    };
                    let v = if matches!(axis, PadAxis::LT | PadAxis::RT) { v.max(0.0) } else { v };
                    input.pad_axis_event(id, axis, v);
                }
                _ => {}
            }
        }
    }
}

impl Default for Gamepads {
    fn default() -> Self {
        Self::new()
    }
}

fn button(input: &mut Input, id: usize, b: Button, down: bool) {
    let pad = match b {
        Button::South => PadButton::A,
        Button::East => PadButton::B,
        Button::West => PadButton::X,
        Button::North => PadButton::Y,
        Button::LeftTrigger => PadButton::LB,
        Button::RightTrigger => PadButton::RB,
        Button::Select => PadButton::Back,
        Button::Start => PadButton::Start,
        Button::LeftThumb => PadButton::LStick,
        Button::RightThumb => PadButton::RStick,
        Button::DPadUp => PadButton::Up,
        Button::DPadDown => PadButton::Down,
        Button::DPadLeft => PadButton::Left,
        Button::DPadRight => PadButton::Right,
        // Triggers come as analog values (ButtonChanged) and become lt / rt buttons there.
        _ => return,
    };
    input.pad_button_event(id, pad, down);
}
