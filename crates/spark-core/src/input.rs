use std::collections::HashSet;

use glam::Vec2;

macro_rules! keys {
    ($($variant:ident => $name:literal),* $(,)?) => {
        /// Keyboard key (physical position, US layout names).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Key { $($variant),* }

        impl Key {
            pub const ALL: &'static [Key] = &[$(Key::$variant),*];

            /// Script name, e.g. `"w"`, `"space"`, `"left"`.
            pub fn name(self) -> &'static str {
                match self { $(Key::$variant => $name),* }
            }

            pub fn from_name(name: &str) -> Option<Key> {
                match name { $($name => Some(Key::$variant),)* _ => None }
            }
        }
    };
}

keys! {
    A => "a", B => "b", C => "c", D => "d", E => "e", F => "f", G => "g", H => "h", I => "i",
    J => "j", K => "k", L => "l", M => "m", N => "n", O => "o", P => "p", Q => "q", R => "r",
    S => "s", T => "t", U => "u", V => "v", W => "w", X => "x", Y => "y", Z => "z",
    Num0 => "0", Num1 => "1", Num2 => "2", Num3 => "3", Num4 => "4",
    Num5 => "5", Num6 => "6", Num7 => "7", Num8 => "8", Num9 => "9",
    Space => "space", Enter => "enter", Escape => "escape", Tab => "tab", Backspace => "backspace",
    Left => "left", Right => "right", Up => "up", Down => "down",
    LShift => "lshift", RShift => "rshift", LCtrl => "lctrl", RCtrl => "rctrl", LAlt => "lalt", RAlt => "ralt",
    F1 => "f1", F2 => "f2", F3 => "f3", F4 => "f4", F5 => "f5", F6 => "f6",
    F7 => "f7", F8 => "f8", F9 => "f9", F10 => "f10", F11 => "f11", F12 => "f12",
}

impl Key {
    /// Keys matched by a script name, including aliases like `"shift"` (either shift key).
    pub fn resolve(name: &str) -> Option<Vec<Key>> {
        let lower = name.trim().to_ascii_lowercase();
        let alias: &[Key] = match lower.as_str() {
            "shift" => &[Key::LShift, Key::RShift],
            "ctrl" | "control" => &[Key::LCtrl, Key::RCtrl],
            "alt" => &[Key::LAlt, Key::RAlt],
            "esc" => &[Key::Escape],
            "return" => &[Key::Enter],
            _ => &[],
        };
        if !alias.is_empty() {
            return Some(alias.to_vec());
        }
        Key::from_name(&lower).map(|k| vec![k])
    }
}

/// Mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "middle" => Some(Self::Middle),
            _ => None,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

macro_rules! named {
    ($(#[$m:meta])* $ty:ident { $($variant:ident => $name:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $ty { $($variant),* }

        impl $ty {
            pub const ALL: &'static [$ty] = &[$($ty::$variant),*];
            pub const NAMES: &'static str = concat!($($name, " "),*);

            pub fn name(self) -> &'static str {
                match self { $($ty::$variant => $name),* }
            }

            pub fn from_name(name: &str) -> Option<Self> {
                match name.trim().to_ascii_lowercase().as_str() { $($name => Some($ty::$variant),)* _ => None }
            }

            fn index(self) -> usize {
                self as usize
            }
        }
    };
}

named! {
    /// Gamepad button, Xbox layout names (on PlayStation: a = cross, b = circle, x = square, y = triangle).
    PadButton {
        A => "a", B => "b", X => "x", Y => "y",
        LB => "lb", RB => "rb", LT => "lt", RT => "rt",
        Back => "back", Start => "start", LStick => "lstick", RStick => "rstick",
        Up => "up", Down => "down", Left => "left", Right => "right",
    }
}

named! {
    /// Gamepad axis. Sticks: -1..1, Y up = +1. Triggers: 0..1.
    PadAxis {
        LeftX => "left_x", LeftY => "left_y", RightX => "right_x", RightY => "right_y", LT => "lt", RT => "rt",
    }
}

const PAD_BUTTONS: usize = 16;
const PAD_AXES: usize = 6;
/// Stick values below this are reported as 0.
pub const STICK_DEADZONE: f32 = 0.15;

/// One connected gamepad.
#[derive(Clone, Debug, Default)]
pub struct Gamepad {
    /// Backend id (stable while connected).
    pub id: usize,
    pub name: String,
    down: [bool; PAD_BUTTONS],
    pressed: [bool; PAD_BUTTONS],
    released: [bool; PAD_BUTTONS],
    axes: [f32; PAD_AXES],
}

impl Gamepad {
    pub fn down(&self, b: PadButton) -> bool {
        self.down[b.index()]
    }
    pub fn pressed(&self, b: PadButton) -> bool {
        self.pressed[b.index()]
    }
    pub fn released(&self, b: PadButton) -> bool {
        self.released[b.index()]
    }
    /// Axis value with the stick dead zone applied.
    pub fn axis(&self, a: PadAxis) -> f32 {
        let v = self.axes[a.index()];
        match a {
            PadAxis::LT | PadAxis::RT => v.clamp(0.0, 1.0),
            _ if v.abs() < STICK_DEADZONE => 0.0,
            _ => (v.signum() * (v.abs() - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).clamp(-1.0, 1.0),
        }
    }
    fn button(&mut self, b: PadButton, is_down: bool) {
        let i = b.index();
        if is_down && !self.down[i] {
            self.pressed[i] = true;
        } else if !is_down && self.down[i] {
            self.released[i] = true;
        }
        self.down[i] = is_down;
    }
}

/// Keyboard and mouse state for the current frame.
#[derive(Clone, Debug, Default)]
pub struct Input {
    /// Connected gamepads in connection order (pad 1 = `pads[0]`).
    pub pads: Vec<Gamepad>,
    down: HashSet<Key>,
    pressed: HashSet<Key>,
    released: HashSet<Key>,
    mouse_down: [bool; 3],
    mouse_pressed: [bool; 3],
    mouse_released: [bool; 3],
    has_mouse_position: bool,
    /// Cursor position in window pixels (0,0 = top-left).
    pub mouse_position: Vec2,
    /// Cursor movement since the previous frame, in pixels.
    pub mouse_delta: Vec2,
    /// Wheel movement this frame (positive = up).
    pub wheel: f32,
    /// Game wants the cursor hidden and captured (first-person look). `mouse_delta` then comes from raw motion.
    pub lock_mouse: bool,
}

impl Input {
    /// Held down right now.
    pub fn down(&self, key: Key) -> bool {
        self.down.contains(&key)
    }

    /// Went down this frame.
    pub fn pressed(&self, key: Key) -> bool {
        self.pressed.contains(&key)
    }

    /// Went up this frame.
    pub fn released(&self, key: Key) -> bool {
        self.released.contains(&key)
    }

    pub fn mouse_down(&self, button: MouseButton) -> bool {
        self.mouse_down[button.index()]
    }

    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.mouse_pressed[button.index()]
    }

    pub fn mouse_released(&self, button: MouseButton) -> bool {
        self.mouse_released[button.index()]
    }

    /// Any key, mouse button or gamepad button went down this frame.
    pub fn any_pressed(&self) -> bool {
        !self.pressed.is_empty()
            || self.mouse_pressed.iter().any(|b| *b)
            || self.pads.iter().any(|p| p.pressed.iter().any(|b| *b))
    }

    /// Gamepad by 1-based number; `None` = not connected.
    pub fn pad(&self, n: usize) -> Option<&Gamepad> {
        n.checked_sub(1).and_then(|i| self.pads.get(i))
    }

    // ---- Gamepads (fed by the window backend) ----

    pub fn pad_connected(&mut self, id: usize, name: impl Into<String>) {
        if !self.pads.iter().any(|p| p.id == id) {
            self.pads.push(Gamepad { id, name: name.into(), ..Default::default() });
        }
    }

    pub fn pad_disconnected(&mut self, id: usize) {
        self.pads.retain(|p| p.id != id);
    }

    pub fn pad_button_event(&mut self, id: usize, button: PadButton, is_down: bool) {
        if let Some(p) = self.pads.iter_mut().find(|p| p.id == id) {
            p.button(button, is_down);
        }
    }

    /// Raw axis value: sticks -1..1 (Y up = +1), triggers 0..1. Triggers also act as `lt` / `rt` buttons.
    pub fn pad_axis_event(&mut self, id: usize, axis: PadAxis, value: f32) {
        if let Some(p) = self.pads.iter_mut().find(|p| p.id == id) {
            p.axes[axis.index()] = value;
            match axis {
                PadAxis::LT => p.button(PadButton::LT, value > 0.5),
                PadAxis::RT => p.button(PadButton::RT, value > 0.5),
                _ => {}
            }
        }
    }

    // ---- Feeding events (window backend / tests) ----

    pub fn key_event(&mut self, key: Key, is_down: bool) {
        if is_down {
            if self.down.insert(key) {
                self.pressed.insert(key);
            }
        } else if self.down.remove(&key) {
            self.released.insert(key);
        }
    }

    pub fn mouse_button_event(&mut self, button: MouseButton, is_down: bool) {
        let i = button.index();
        if is_down && !self.mouse_down[i] {
            self.mouse_pressed[i] = true;
        } else if !is_down && self.mouse_down[i] {
            self.mouse_released[i] = true;
        }
        self.mouse_down[i] = is_down;
    }

    /// Raw mouse movement (not limited by the window edge). Used while `lock_mouse` is on.
    pub fn mouse_motion(&mut self, delta: Vec2) {
        if self.lock_mouse {
            self.mouse_delta += delta;
        }
    }

    pub fn mouse_moved(&mut self, position: Vec2) {
        if self.has_mouse_position && !self.lock_mouse {
            self.mouse_delta += position - self.mouse_position;
        }
        self.mouse_position = position;
        self.has_mouse_position = true;
    }

    pub fn wheel_event(&mut self, amount: f32) {
        self.wheel += amount;
    }

    /// Call after each update: clears per-frame state.
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
        self.mouse_pressed = [false; 3];
        self.mouse_released = [false; 3];
        for p in &mut self.pads {
            p.pressed = [false; PAD_BUTTONS];
            p.released = [false; PAD_BUTTONS];
        }
        self.mouse_delta = Vec2::ZERO;
        self.wheel = 0.0;
    }

    /// Releases everything (e.g. window lost focus).
    pub fn release_all(&mut self) {
        let keys: Vec<Key> = self.down.iter().copied().collect();
        for k in keys {
            self.key_event(k, false);
        }
        for b in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            self.mouse_button_event(b, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamepad_buttons_axes_and_triggers() {
        let mut i = Input::default();
        i.pad_connected(7, "Test pad");
        i.pad_button_event(7, PadButton::A, true);
        i.pad_axis_event(7, PadAxis::LeftX, 0.1);
        i.pad_axis_event(7, PadAxis::LeftY, -1.0);
        i.pad_axis_event(7, PadAxis::RT, 0.9);
        let p = i.pad(1).unwrap();
        assert!(p.pressed(PadButton::A) && p.down(PadButton::A));
        assert_eq!(p.axis(PadAxis::LeftX), 0.0, "inside the dead zone");
        assert_eq!(p.axis(PadAxis::LeftY), -1.0);
        assert!(p.pressed(PadButton::RT));
        assert!(i.any_pressed());
        i.end_frame();
        let p = i.pad(1).unwrap();
        assert!(!p.pressed(PadButton::A) && p.down(PadButton::A));
        assert!(i.pad(2).is_none());
        i.pad_disconnected(7);
        assert!(i.pad(1).is_none());
        assert_eq!(PadButton::from_name("LB"), Some(PadButton::LB));
        assert_eq!(PadAxis::from_name("right_y"), Some(PadAxis::RightY));
    }
}
