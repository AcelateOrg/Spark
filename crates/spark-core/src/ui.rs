//! Immediate-mode UI: panels, buttons, labels, images, sliders, checkboxes and progress bars.
//!
//! Widgets are drawn through the [`Canvas`] on [`Layer::Ui`] in virtual pixels (origin top-left,
//! Y down) and return their interaction result right away:
//!
//! ```ignore
//! if world.ui.button(&mut world.canvas, 1, Rect::new(10.0, 10.0, 200.0, 40.0), "Start") {
//!     // clicked this frame
//! }
//! ```
//!
//! The frame runner calls [`Ui::begin`] before `update` and [`Ui::end`] after `draw`, so widgets
//! work in both. Interactive widgets need an `id` that stays the same from frame to frame (a press
//! and the release that completes a click happen in different frames).

use glam::{Affine2, Vec2};

use crate::assets::TextureId;
use crate::canvas::{Align, Canvas, Layer, SpriteParams, TextParams};
use crate::color::Color;
use crate::input::{Input, MouseButton};

/// Rectangle in virtual pixels (origin top-left, Y down).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// True if `p` is inside the rectangle (edges included).
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }

    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    fn pos(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    fn size(&self) -> Vec2 {
        Vec2::new(self.w, self.h)
    }
}

/// Colors of the built-in widgets.
#[derive(Clone, Debug, PartialEq)]
pub struct UiStyle {
    pub panel: Color,
    pub widget: Color,
    pub widget_hover: Color,
    pub widget_pressed: Color,
    pub border: Color,
    pub track: Color,
    pub fill: Color,
    pub fill_hover: Color,
    pub check: Color,
    pub text: Color,
    /// Largest text size of button labels (smaller buttons get smaller text).
    pub button_text_size: f32,
}

impl Default for UiStyle {
    fn default() -> Self {
        Self {
            panel: Color::rgba(0.1, 0.1, 0.12, 0.9),
            widget: Color::rgb(0.25, 0.27, 0.33),
            widget_hover: Color::rgb(0.36, 0.38, 0.43),
            widget_pressed: Color::rgb(0.18, 0.19, 0.23),
            border: Color::rgb(0.12, 0.12, 0.15),
            track: Color::rgb(0.15, 0.15, 0.18),
            fill: Color::rgb(0.25, 0.45, 0.85),
            fill_hover: Color::rgb(0.35, 0.55, 0.95),
            check: Color::rgb(0.35, 0.8, 0.45),
            text: Color::WHITE,
            button_text_size: 18.0,
        }
    }
}

/// Immediate-mode UI state, kept across frames in [`crate::World::ui`].
#[derive(Clone, Debug, Default)]
pub struct Ui {
    /// Widget under the mouse cursor this frame.
    pub hot: Option<u64>,
    /// Widget being pressed or dragged (left mouse button held since the press on it).
    pub active: Option<u64>,
    /// Widget colors.
    pub style: UiStyle,
    /// Mouse position in virtual pixels.
    pub mouse_pos: Vec2,
    /// Left mouse button held.
    pub mouse_down: bool,
    /// Left mouse button pressed this frame.
    pub mouse_pressed: bool,
    /// Canvas size in virtual pixels.
    pub screen_size: Vec2,
    /// The cursor is over a widget or panel drawn this frame / the previous frame.
    over: bool,
    over_prev: bool,
}

/// Puts the canvas on the UI layer without a transform for the widget and restores it afterwards,
/// so widgets never change where the game's own `draw` calls go.
fn on_ui_layer<R>(canvas: &mut Canvas, f: impl FnOnce(&mut Canvas) -> R) -> R {
    let (layer, transform) = (canvas.layer(), canvas.transform());
    canvas.set_layer(Layer::Ui);
    canvas.set_transform(Affine2::IDENTITY);
    let r = f(canvas);
    canvas.set_layer(layer);
    canvas.set_transform(transform);
    r
}

impl Ui {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the mouse state for this frame. Called by the frame runner before `update`.
    pub fn begin(&mut self, canvas: &Canvas, input: &Input) {
        self.screen_size = canvas.size();
        self.mouse_pos = canvas.to_virtual(input.mouse_position);
        self.mouse_down = input.mouse_down(MouseButton::Left);
        self.mouse_pressed = input.mouse_pressed(MouseButton::Left);
        self.hot = None;
        self.over_prev = self.over;
        self.over = false;
    }

    /// Ends the frame. Called by the frame runner after `draw`. Releases the active widget when
    /// the mouse button is up, even if that widget was not drawn this frame.
    pub fn end(&mut self) {
        if !self.mouse_down {
            self.active = None;
        }
        self.mouse_pressed = false;
    }

    /// True when the cursor is over UI drawn this frame or the previous one: lets the game ignore
    /// clicks meant for the UI (e.g. `if input.mouse_pressed("left") and not ui.hovered() then shoot() end`).
    pub fn hovered(&self) -> bool {
        self.over || self.over_prev
    }

    fn hover(&mut self, id: Option<u64>, rect: Rect) -> bool {
        let hovered = rect.contains(self.mouse_pos);
        if hovered {
            self.over = true;
            if id.is_some() {
                self.hot = id;
            }
        }
        hovered
    }

    /// Press on the widget makes it active; returns true on the release that ends that press
    /// while the cursor is still over the widget (a click).
    fn click(&mut self, id: u64, hovered: bool) -> bool {
        if self.mouse_pressed && hovered && self.active.is_none() {
            self.active = Some(id);
            false
        } else if !self.mouse_down && self.active == Some(id) {
            self.active = None;
            hovered
        } else {
            false
        }
    }

    // ---- widgets ------------------------------------------------------------------------------

    /// Filled rectangle with an optional border `(thickness, color)`. Returns true when hovered.
    pub fn panel(&mut self, canvas: &mut Canvas, rect: Rect, bg: Color, border: Option<(f32, Color)>) -> bool {
        on_ui_layer(canvas, |c| {
            c.rect(rect.pos(), rect.size(), bg, Align::TOP_LEFT);
            if let Some((thickness, color)) = border {
                c.rect_line(rect.pos(), rect.size(), thickness, color, Align::TOP_LEFT);
            }
        });
        self.hover(None, rect)
    }

    /// Button with a centered label. Returns true when clicked (pressed and released over it).
    pub fn button(&mut self, canvas: &mut Canvas, id: u64, rect: Rect, text: &str) -> bool {
        let hovered = self.hover(Some(id), rect);
        let clicked = self.click(id, hovered);
        let s = &self.style;
        let color = if self.active == Some(id) {
            s.widget_pressed
        } else if hovered {
            s.widget_hover
        } else {
            s.widget
        };
        let text_params = TextParams {
            size: s.button_text_size.min(rect.h * 0.6).max(1.0),
            color: s.text,
            align: Align::CENTER,
            ..Default::default()
        };
        let border = s.border;
        on_ui_layer(canvas, |c| {
            c.rect(rect.pos(), rect.size(), color, Align::TOP_LEFT);
            c.rect_line(rect.pos(), rect.size(), 1.0, border, Align::TOP_LEFT);
            c.text(text, rect.center(), &text_params);
        });
        clicked
    }

    /// Text with its top-left corner at `pos`.
    pub fn label(&mut self, canvas: &mut Canvas, pos: Vec2, text: &str, size: f32, color: Color) {
        let p = TextParams { size, color, align: Align::TOP_LEFT, ..Default::default() };
        on_ui_layer(canvas, |c| {
            c.text(text, pos, &p);
        });
    }

    /// Texture stretched over `rect`.
    pub fn image(&mut self, canvas: &mut Canvas, rect: Rect, texture: TextureId) {
        // With no region the texture size only scales `region`, so (1, 1) gives the full 0..1 UVs.
        let params = SpriteParams { size: Some(rect.size()), align: Align::TOP_LEFT, ..Default::default() };
        on_ui_layer(canvas, |c| c.sprite(texture, (1, 1), rect.pos(), &params));
    }

    /// Horizontal slider for `value` in `min..=max`. Returns `Some(new value)` while it is dragged
    /// to a different value, `None` otherwise.
    pub fn slider(&mut self, canvas: &mut Canvas, id: u64, rect: Rect, value: f32, min: f32, max: f32) -> Option<f32> {
        let hovered = self.hover(Some(id), rect);
        if self.mouse_pressed && hovered && self.active.is_none() {
            self.active = Some(id);
        }
        let dragging = self.active == Some(id) && self.mouse_down;
        let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
        let mut shown = value.clamp(lo, hi);
        let mut changed = None;
        if dragging {
            let t = if rect.w > 0.0 { ((self.mouse_pos.x - rect.x) / rect.w).clamp(0.0, 1.0) } else { 0.0 };
            let v = min + t * (max - min);
            if v != value {
                changed = Some(v);
            }
            shown = v;
        }
        if !self.mouse_down && self.active == Some(id) {
            self.active = None;
        }

        let s = &self.style;
        let t = if hi > lo { (shown - min) / (max - min) } else { 0.0 };
        let fill_w = rect.w * t.clamp(0.0, 1.0);
        let fill = if dragging || hovered { s.fill_hover } else { s.fill };
        let (track, border, knob) = (s.track, s.border, s.text);
        on_ui_layer(canvas, |c| {
            c.rect(rect.pos(), rect.size(), track, Align::TOP_LEFT);
            c.rect(rect.pos(), Vec2::new(fill_w, rect.h), fill, Align::TOP_LEFT);
            c.rect_line(rect.pos(), rect.size(), 1.0, border, Align::TOP_LEFT);
            c.circle(Vec2::new(rect.x + fill_w, rect.center().y), rect.h * 0.6, knob);
        });
        changed
    }

    /// Checkbox. Returns `Some(new state)` when clicked, `None` otherwise.
    pub fn checkbox(&mut self, canvas: &mut Canvas, id: u64, rect: Rect, checked: bool) -> Option<bool> {
        let hovered = self.hover(Some(id), rect);
        let toggled = self.click(id, hovered);
        let shown = checked != toggled;
        let s = &self.style;
        let (bg, border, check) = (if hovered { s.widget_hover } else { s.widget }, s.border, s.check);
        on_ui_layer(canvas, |c| {
            c.rect(rect.pos(), rect.size(), bg, Align::TOP_LEFT);
            c.rect_line(rect.pos(), rect.size(), 1.0, border, Align::TOP_LEFT);
            if shown {
                let inset = (rect.w.min(rect.h) * 0.2).max(1.0);
                let size = (rect.size() - Vec2::splat(inset * 2.0)).max(Vec2::ZERO);
                c.rect(rect.pos() + Vec2::splat(inset), size, check, Align::TOP_LEFT);
            }
        });
        toggled.then_some(shown)
    }

    /// Progress bar, `progress` 0..1 (clamped).
    pub fn progress(&mut self, canvas: &mut Canvas, rect: Rect, progress: f32, color: Color) {
        let fill = rect.w * progress.clamp(0.0, 1.0);
        let (track, border) = (self.style.track, self.style.border);
        on_ui_layer(canvas, |c| {
            c.rect(rect.pos(), rect.size(), track, Align::TOP_LEFT);
            c.rect(rect.pos(), Vec2::new(fill, rect.h), color, Align::TOP_LEFT);
            c.rect_line(rect.pos(), rect.size(), 1.0, border, Align::TOP_LEFT);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1920x1080 window: scale 1.5, virtual 1280x720.
    fn canvas() -> Canvas {
        let mut c = Canvas::new();
        c.set_output_size(1920, 1080);
        c.begin_frame((1920, 1080));
        c
    }

    /// Window pixels for a virtual position (inverse of `Canvas::to_virtual`).
    fn win(c: &Canvas, virtual_pos: Vec2) -> Vec2 {
        virtual_pos * c.scale()
    }

    /// One frame: mouse at `at` (virtual), left button `down`.
    fn frame(ui: &mut Ui, c: &Canvas, input: &mut Input, at: Vec2, down: bool) {
        input.end_frame();
        input.mouse_moved(win(c, at));
        if input.mouse_down(MouseButton::Left) != down {
            input.mouse_button_event(MouseButton::Left, down);
        }
        ui.end();
        ui.begin(c, input);
    }

    const BTN: Rect = Rect { x: 100.0, y: 100.0, w: 200.0, h: 40.0 };

    #[test]
    fn rect_contains_and_center() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert!(r.contains(Vec2::new(10.0, 20.0)));
        assert!(r.contains(Vec2::new(110.0, 70.0)));
        assert!(!r.contains(Vec2::new(9.0, 20.0)));
        assert!(!r.contains(Vec2::new(111.0, 70.0)));
        assert_eq!(r.center(), Vec2::new(60.0, 45.0));
    }

    #[test]
    fn begin_converts_mouse_to_virtual() {
        let c = canvas();
        let mut input = Input::default();
        input.mouse_moved(Vec2::new(960.0, 540.0));
        let mut ui = Ui::new();
        ui.begin(&c, &input);
        assert_eq!(ui.screen_size, Vec2::new(1280.0, 720.0));
        assert_eq!(ui.mouse_pos, Vec2::new(640.0, 360.0));
    }

    #[test]
    fn button_clicks_on_release_over_it() {
        let mut c = canvas();
        let (mut ui, mut input) = (Ui::new(), Input::default());
        frame(&mut ui, &c, &mut input, BTN.center(), true);
        assert!(!ui.button(&mut c, 1, BTN, "Go"), "press alone is no click");
        assert_eq!(ui.active, Some(1));
        frame(&mut ui, &c, &mut input, BTN.center(), true);
        assert!(!ui.button(&mut c, 1, BTN, "Go"), "holding is no click");
        frame(&mut ui, &c, &mut input, BTN.center(), false);
        assert!(ui.button(&mut c, 1, BTN, "Go"));
        assert_eq!(ui.active, None);
        frame(&mut ui, &c, &mut input, BTN.center(), false);
        assert!(!ui.button(&mut c, 1, BTN, "Go"), "one click per press");
    }

    #[test]
    fn button_no_click_when_released_outside() {
        let mut c = canvas();
        let (mut ui, mut input) = (Ui::new(), Input::default());
        frame(&mut ui, &c, &mut input, BTN.center(), true);
        ui.button(&mut c, 1, BTN, "Go");
        frame(&mut ui, &c, &mut input, Vec2::new(10.0, 10.0), false);
        assert!(!ui.button(&mut c, 1, BTN, "Go"));
        assert_eq!(ui.active, None);
    }

    #[test]
    fn press_outside_then_drag_in_is_no_click() {
        let mut c = canvas();
        let (mut ui, mut input) = (Ui::new(), Input::default());
        frame(&mut ui, &c, &mut input, Vec2::new(10.0, 10.0), true);
        ui.button(&mut c, 1, BTN, "Go");
        frame(&mut ui, &c, &mut input, BTN.center(), true);
        ui.button(&mut c, 1, BTN, "Go");
        frame(&mut ui, &c, &mut input, BTN.center(), false);
        assert!(!ui.button(&mut c, 1, BTN, "Go"));
    }

    #[test]
    fn checkbox_toggles_once() {
        let mut c = canvas();
        let (mut ui, mut input) = (Ui::new(), Input::default());
        let r = Rect::new(50.0, 50.0, 20.0, 20.0);
        frame(&mut ui, &c, &mut input, r.center(), true);
        assert_eq!(ui.checkbox(&mut c, 7, r, false), None);
        frame(&mut ui, &c, &mut input, r.center(), false);
        assert_eq!(ui.checkbox(&mut c, 7, r, false), Some(true));
        frame(&mut ui, &c, &mut input, r.center(), false);
        assert_eq!(ui.checkbox(&mut c, 7, r, true), None);
    }

    #[test]
    fn slider_follows_drag_and_releases() {
        let mut c = canvas();
        let (mut ui, mut input) = (Ui::new(), Input::default());
        let r = Rect::new(100.0, 100.0, 200.0, 20.0);
        frame(&mut ui, &c, &mut input, Vec2::new(150.0, 110.0), true);
        let v = ui.slider(&mut c, 2, r, 0.0, 0.0, 100.0).expect("press moves the value");
        assert!((v - 25.0).abs() < 0.01, "{v}");
        // dragging outside the track clamps
        frame(&mut ui, &c, &mut input, Vec2::new(900.0, 400.0), true);
        assert_eq!(ui.slider(&mut c, 2, r, v, 0.0, 100.0), Some(100.0));
        frame(&mut ui, &c, &mut input, Vec2::new(900.0, 400.0), true);
        assert_eq!(ui.slider(&mut c, 2, r, 100.0, 0.0, 100.0), None, "unchanged value is not reported");
        frame(&mut ui, &c, &mut input, Vec2::new(150.0, 110.0), false);
        assert_eq!(ui.slider(&mut c, 2, r, 100.0, 0.0, 100.0), None);
        assert_eq!(ui.active, None);
    }

    #[test]
    fn slider_with_zero_width_or_range_is_safe() {
        let mut c = canvas();
        let (mut ui, mut input) = (Ui::new(), Input::default());
        frame(&mut ui, &c, &mut input, Vec2::new(10.0, 10.0), true);
        let v = ui.slider(&mut c, 3, Rect::new(10.0, 0.0, 0.0, 20.0), 5.0, 0.0, 10.0);
        assert!(v.is_none_or(|v| v.is_finite()));
        let v = ui.slider(&mut c, 4, Rect::new(0.0, 0.0, 100.0, 20.0), 1.0, 1.0, 1.0);
        assert!(v.is_none_or(|v| v.is_finite()));
    }

    #[test]
    fn first_widget_pressed_owns_the_mouse() {
        let mut c = canvas();
        let (mut ui, mut input) = (Ui::new(), Input::default());
        frame(&mut ui, &c, &mut input, BTN.center(), true);
        ui.button(&mut c, 1, BTN, "A");
        ui.button(&mut c, 2, BTN, "B"); // overlapping
        assert_eq!(ui.active, Some(1));
    }

    #[test]
    fn active_is_released_even_if_widget_disappears() {
        let mut c = canvas();
        let (mut ui, mut input) = (Ui::new(), Input::default());
        frame(&mut ui, &c, &mut input, BTN.center(), true);
        ui.button(&mut c, 1, BTN, "Go");
        frame(&mut ui, &c, &mut input, BTN.center(), false);
        frame(&mut ui, &c, &mut input, BTN.center(), false);
        assert_eq!(ui.active, None);
    }

    #[test]
    fn hovered_covers_panels_and_lasts_into_next_update() {
        let mut c = canvas();
        let (mut ui, mut input) = (Ui::new(), Input::default());
        frame(&mut ui, &c, &mut input, Vec2::new(20.0, 20.0), false);
        assert!(ui.panel(&mut c, Rect::new(0.0, 0.0, 100.0, 100.0), Color::BLACK, None));
        assert!(ui.hovered());
        frame(&mut ui, &c, &mut input, Vec2::new(20.0, 20.0), false);
        assert!(ui.hovered(), "update() of the next frame still sees the UI under the cursor");
        frame(&mut ui, &c, &mut input, Vec2::new(20.0, 20.0), false);
        assert!(!ui.hovered(), "nothing drawn for a frame");
    }

    #[test]
    fn widgets_keep_the_game_layer_and_transform() {
        let mut c = canvas();
        let mut ui = Ui::new();
        c.set_layer(Layer::Scene);
        c.translate(Vec2::new(50.0, 0.0));
        let t = c.transform();
        ui.progress(&mut c, Rect::new(0.0, 0.0, 100.0, 10.0), 0.5, Color::GREEN);
        ui.label(&mut c, Vec2::ZERO, "hi", 16.0, Color::WHITE);
        assert_eq!(c.layer(), Layer::Scene);
        assert_eq!(c.transform(), t);
        assert!(!c.layer_data(Layer::Ui).vertices.is_empty(), "widgets drawn on the UI layer");
        assert!(c.layer_data(Layer::Scene).vertices.is_empty());
    }

    #[test]
    fn non_interactive_widgets_do_not_take_the_mouse() {
        let mut c = canvas();
        let (mut ui, mut input) = (Ui::new(), Input::default());
        frame(&mut ui, &c, &mut input, Vec2::new(5.0, 5.0), true);
        ui.progress(&mut c, Rect::new(0.0, 0.0, 200.0, 20.0), 0.5, Color::GREEN);
        ui.label(&mut c, Vec2::ZERO, "Hello", 24.0, Color::WHITE);
        assert_eq!((ui.hot, ui.active), (None, None));
    }
}
