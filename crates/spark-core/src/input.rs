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
    // Punctuation (US layout positions).
    Minus => "minus", Equal => "equal", BracketLeft => "bracketleft", BracketRight => "bracketright",
    Backslash => "backslash", Semicolon => "semicolon", Quote => "quote", Comma => "comma",
    Period => "period", Slash => "slash", Backquote => "backquote",
    // Navigation / editing.
    Insert => "insert", Delete => "delete", Home => "home", End => "end", PageUp => "pageup", PageDown => "pagedown",
    CapsLock => "capslock", NumLock => "numlock", ScrollLock => "scrolllock", PrintScreen => "printscreen",
    Pause => "pause", Menu => "menu", LSuper => "lsuper", RSuper => "rsuper",
    // Numpad (numpad Enter is reported as "enter").
    Numpad0 => "numpad0", Numpad1 => "numpad1", Numpad2 => "numpad2", Numpad3 => "numpad3", Numpad4 => "numpad4",
    Numpad5 => "numpad5", Numpad6 => "numpad6", Numpad7 => "numpad7", Numpad8 => "numpad8", Numpad9 => "numpad9",
    NumpadAdd => "numpad_add", NumpadSubtract => "numpad_subtract", NumpadMultiply => "numpad_multiply",
    NumpadDivide => "numpad_divide", NumpadDecimal => "numpad_decimal",
    F13 => "f13", F14 => "f14", F15 => "f15", F16 => "f16", F17 => "f17", F18 => "f18",
    F19 => "f19", F20 => "f20", F21 => "f21", F22 => "f22", F23 => "f23", F24 => "f24",
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
            "super" | "meta" | "cmd" | "command" | "win" | "windows" => &[Key::LSuper, Key::RSuper],
            "lmeta" | "lcmd" | "lwin" => &[Key::LSuper],
            "rmeta" | "rcmd" | "rwin" => &[Key::RSuper],
            "-" => &[Key::Minus],
            "=" | "equals" => &[Key::Equal],
            "[" => &[Key::BracketLeft],
            "]" => &[Key::BracketRight],
            "\\" => &[Key::Backslash],
            ";" => &[Key::Semicolon],
            "'" | "apostrophe" => &[Key::Quote],
            "," => &[Key::Comma],
            "." => &[Key::Period],
            "/" => &[Key::Slash],
            "`" | "grave" | "backtick" | "tilde" => &[Key::Backquote],
            "del" => &[Key::Delete],
            "ins" => &[Key::Insert],
            "pgup" => &[Key::PageUp],
            "pgdn" => &[Key::PageDown],
            "prtsc" | "print" => &[Key::PrintScreen],
            "numpad_plus" => &[Key::NumpadAdd],
            "numpad_minus" => &[Key::NumpadSubtract],
            _ => &[],
        };
        if !alias.is_empty() {
            return Some(alias.to_vec());
        }
        Key::from_name(&lower).map(|k| vec![k])
    }

    /// Every key name and alias, for error messages and docs.
    pub fn names() -> String {
        let mut s: Vec<&str> = Key::ALL.iter().map(|k| k.name()).collect();
        s.extend(["shift", "ctrl", "alt", "super", "esc", "return"]);
        s.join(" ")
    }
}

/// Mouse button. `Back` / `Forward` are the side buttons (4 / 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

const MOUSE_BUTTONS: usize = 5;

impl MouseButton {
    pub const ALL: &'static [MouseButton] =
        &[MouseButton::Left, MouseButton::Right, MouseButton::Middle, MouseButton::Back, MouseButton::Forward];

    /// Script name: `left right middle back forward`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
            Self::Back => "back",
            Self::Forward => "forward",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "middle" => Some(Self::Middle),
            "back" | "x1" | "mouse4" => Some(Self::Back),
            "forward" | "x2" | "mouse5" => Some(Self::Forward),
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
    mouse_down: [bool; MOUSE_BUTTONS],
    mouse_pressed: [bool; MOUSE_BUTTONS],
    mouse_released: [bool; MOUSE_BUTTONS],
    has_mouse_position: bool,
    /// Cursor position in window pixels (0,0 = top-left).
    pub mouse_position: Vec2,
    /// Cursor movement since the previous frame, in pixels.
    pub mouse_delta: Vec2,
    /// Wheel movement this frame (positive = up).
    pub wheel: f32,
    /// Game wants the cursor hidden and captured (first-person look). `mouse_delta` then comes from raw motion.
    pub lock_mouse: bool,
    /// Text typed this frame (layout- and IME-aware, control characters removed). Cleared by `end_frame`.
    pub text: String,
    /// Keys that went down or auto-repeated (key held) this frame.
    repeated: HashSet<Key>,
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

    /// Went down or auto-repeated this frame (held keys repeat at the OS rate): menus, text fields.
    pub fn repeated(&self, key: Key) -> bool {
        self.repeated.contains(&key)
    }

    /// Text typed this frame.
    pub fn text(&self) -> &str {
        &self.text
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
            // A press while already down is an OS auto-repeat.
            self.repeated.insert(key);
        } else if self.down.remove(&key) {
            self.released.insert(key);
        }
    }

    /// Typed text (winit `KeyboardInput.text`, IME commit). Control characters (Enter, Backspace, ...) are dropped.
    pub fn text_event(&mut self, text: &str) {
        self.text.extend(text.chars().filter(|c| !c.is_control()));
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
        self.repeated.clear();
        self.text.clear();
        self.mouse_pressed = [false; MOUSE_BUTTONS];
        self.mouse_released = [false; MOUSE_BUTTONS];
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
        for &b in MouseButton::ALL {
            self.mouse_button_event(b, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_round_trip_and_aliases() {
        let mut seen = HashSet::new();
        for &k in Key::ALL {
            assert!(seen.insert(k.name()), "duplicate key name {}", k.name());
            assert_eq!(Key::from_name(k.name()), Some(k));
            assert_eq!(Key::resolve(k.name()), Some(vec![k]));
        }
        assert_eq!(Key::resolve("-"), Some(vec![Key::Minus]));
        assert_eq!(Key::resolve("`"), Some(vec![Key::Backquote]));
        assert_eq!(Key::resolve("\\"), Some(vec![Key::Backslash]));
        assert_eq!(Key::resolve("PgUp"), Some(vec![Key::PageUp]));
        assert_eq!(Key::resolve("cmd"), Some(vec![Key::LSuper, Key::RSuper]));
        assert_eq!(Key::resolve("numpad5"), Some(vec![Key::Numpad5]));
        assert_eq!(Key::resolve("f24"), Some(vec![Key::F24]));
        assert_eq!(Key::resolve("nope"), None);
        assert!(Key::names().contains("bracketleft") && Key::names().contains("super"));
    }

    #[test]
    fn side_mouse_buttons() {
        let mut i = Input::default();
        i.mouse_button_event(MouseButton::Back, true);
        assert!(i.mouse_pressed(MouseButton::Back) && i.any_pressed());
        i.end_frame();
        i.release_all();
        assert!(i.mouse_released(MouseButton::Back) && !i.mouse_down(MouseButton::Back));
        assert_eq!(MouseButton::from_name("mouse5"), Some(MouseButton::Forward));
        for &b in MouseButton::ALL {
            assert_eq!(MouseButton::from_name(b.name()), Some(b));
        }
    }

    #[test]
    fn text_and_repeat() {
        let mut i = Input::default();
        i.text_event("Hi");
        i.text_event("\r\u{8}");
        i.text_event("ё");
        assert_eq!(i.text(), "Hiё");
        i.key_event(Key::Backspace, true);
        assert!(i.pressed(Key::Backspace) && i.repeated(Key::Backspace));
        i.end_frame();
        assert_eq!(i.text(), "");
        assert!(!i.repeated(Key::Backspace));
        i.key_event(Key::Backspace, true); // OS auto-repeat
        assert!(!i.pressed(Key::Backspace) && i.repeated(Key::Backspace));
    }

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
