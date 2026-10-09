//! Spark branding: the startup splash ("POWERED BY SPARK" on a dark screen, shown by every
//! windowed Spark game before `Game::start`) and the small clickable badge for game menus that
//! opens [`SPARK_URL`].
//!
//! Pure data like the rest of the core: the badge only *requests* the link
//! ([`Branding::take_open_request`]); the app opens the browser and shows the hand cursor.

use glam::Vec2;

use crate::assets::{Assets, ImageData, TextureId};
use crate::canvas::{Align, SpriteParams};
use crate::color::Color;
use crate::input::MouseButton;
use crate::world::World;

/// Where the badge leads.
pub const SPARK_URL: &str = "https://acelate.com/spark";

const SPLASH_PNG: &[u8] = include_bytes!("../assets/branding/powered_by_spark_800.png");
const BADGE_PNG: &[u8] = include_bytes!("../assets/branding/powered_by_spark_320.png");

/// Splash background.
pub const SPLASH_BACKGROUND: Color = Color::rgb(0.03, 0.03, 0.04);

// Splash timeline, seconds.
const DELAY: f32 = 0.35;
const FADE_IN: f32 = 1.1;
const HOLD: f32 = 1.5;
const FADE_OUT: f32 = 0.8;
const TAIL: f32 = 0.25;

/// Logo width on the splash, in virtual pixels of a 720-tall screen.
/// (The PNGs carry a transparent border so repeat-sampling / mipmaps never bleed the opposite edge in.)
const SPLASH_WIDTH: f32 = 345.0;

/// Branding state stored in [`World::branding`].
#[derive(Debug, Default)]
pub struct Branding {
    /// (assets generation, splash texture, badge texture): re-created after `Assets::clear`.
    textures: Option<(u64, TextureId, TextureId)>,
    /// The cursor is over the badge drawn this frame (the app shows a hand cursor).
    pub badge_hovered: bool,
    open_requested: bool,
}

impl Branding {
    /// Called at the start of every frame.
    pub fn begin_frame(&mut self) {
        self.badge_hovered = false;
    }

    /// Asks the app to open [`SPARK_URL`] in the browser.
    pub fn request_open(&mut self) {
        self.open_requested = true;
    }

    /// True once after the badge was clicked (the app opens the link).
    pub fn take_open_request(&mut self) -> bool {
        std::mem::take(&mut self.open_requested)
    }

    fn textures(&mut self, assets: &mut Assets) -> (TextureId, TextureId) {
        let generation = assets.generation();
        if let Some((g, splash, badge)) = self.textures {
            if g == generation && assets.texture(splash).is_some() && assets.texture(badge).is_some() {
                return (splash, badge);
            }
        }
        let splash = assets.add_texture(decode(SPLASH_PNG));
        let badge = assets.add_texture(decode(BADGE_PNG));
        self.textures = Some((generation, splash, badge));
        (splash, badge)
    }
}

fn decode(bytes: &[u8]) -> ImageData {
    match image::load_from_memory(bytes) {
        Ok(img) => {
            let img = img.to_rgba8();
            ImageData::new(img.width(), img.height(), img.into_raw())
        }
        Err(e) => {
            log::error!("spark logo: {e}");
            ImageData::solid(Color::rgba(1.0, 1.0, 1.0, 0.0))
        }
    }
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The startup splash. Drive it with [`splash_frame`] until it returns `false`, then start the game.
#[derive(Clone, Debug, Default)]
pub struct Splash {
    t: f32,
}

impl Splash {
    pub fn new() -> Self {
        Self::default()
    }

    /// Total length in seconds (without skipping).
    pub const DURATION: f32 = DELAY + FADE_IN + HOLD + FADE_OUT + TAIL;

    pub fn finished(&self) -> bool {
        self.t >= Self::DURATION
    }

    /// Logo opacity at the current time.
    pub fn alpha(&self) -> f32 {
        let t = self.t;
        if t < DELAY {
            0.0
        } else if t < DELAY + FADE_IN {
            smoothstep((t - DELAY) / FADE_IN)
        } else if t < DELAY + FADE_IN + HOLD {
            1.0
        } else {
            1.0 - smoothstep((t - DELAY - FADE_IN - HOLD) / FADE_OUT)
        }
    }

    /// Advances the timeline. Any key / click skips the hold once the logo is fully visible.
    pub fn advance(&mut self, dt: f32, skip: bool) {
        self.t += dt.max(0.0);
        let shown = DELAY + FADE_IN;
        if skip && self.t >= shown {
            self.t = self.t.max(shown + HOLD);
        }
    }
}

/// One splash frame: clears the canvas, draws the logo, consumes input. Returns `false` when the
/// splash is over (nothing is drawn then) - start the game at that point.
pub fn splash_frame(world: &mut World, splash: &mut Splash, dt: f32) -> bool {
    let skip = world.input.any_pressed();
    splash.advance(dt, skip);
    let (w, h) = world.canvas.output_size();
    let internal = world.render.internal_size(w, h);
    world.canvas.begin_frame(internal);
    world.branding.begin_frame();
    world.input.end_frame();
    if splash.finished() {
        return false;
    }
    let (tex, _) = world.branding.textures(&mut world.assets);
    let size = world.assets.texture(tex).map(|t| (t.width, t.height)).unwrap_or((1, 1));
    let screen = world.canvas.size();
    let unit = screen.y / 720.0;
    world.canvas.rect(Vec2::ZERO, screen, SPLASH_BACKGROUND, Align::TOP_LEFT);
    let a = splash.alpha();
    if a > 0.0 {
        // A slow settle: the logo grows by 2% while it fades in and keeps drifting while it holds.
        let grow = 0.975 + 0.025 * smoothstep(splash.t / (DELAY + FADE_IN + HOLD));
        let width = (SPLASH_WIDTH * unit).min(screen.x * 0.6) * grow;
        let p = SpriteParams {
            size: Some(Vec2::new(width, width * size.1 as f32 / size.0 as f32)),
            color: Color::rgba(1.0, 1.0, 1.0, a),
            align: Align::CENTER,
            nearest: Some(false),
            ..Default::default()
        };
        world.canvas.sprite(tex, size, screen * 0.5, &p);
    }
    true
}

/// Options for [`badge`].
#[derive(Clone, Copy, Debug)]
pub struct BadgeParams {
    /// Anchor point in virtual pixels. Default: the screen corner given by `align`, inset by `margin`.
    pub pos: Option<Vec2>,
    /// Which point of the badge sits at `pos` (and which corner it goes to by default).
    pub align: Align,
    /// Width in virtual pixels of a 720-tall screen (scaled with `screen.height`).
    pub width: f32,
    /// Distance from the screen edges (same units as `width`).
    pub margin: f32,
    /// Opacity when not hovered (hovered = full).
    pub alpha: f32,
}

impl Default for BadgeParams {
    fn default() -> Self {
        Self { pos: None, align: Align { x: 1.0, y: 1.0 }, width: 140.0, margin: 18.0, alpha: 0.55 }
    }
}

/// Result of drawing the badge this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BadgeState {
    pub hovered: bool,
    /// Clicked this frame (the link was requested).
    pub clicked: bool,
}

/// Draws the "POWERED BY SPARK" badge (UI layer) and handles hover / click -> [`SPARK_URL`].
/// Hover and click only work while the mouse is not locked.
pub fn badge(world: &mut World, p: &BadgeParams) -> BadgeState {
    let (_, tex) = world.branding.textures(&mut world.assets);
    let size = world.assets.texture(tex).map(|t| (t.width, t.height)).unwrap_or((1, 1));
    let screen = world.canvas.size();
    let unit = screen.y / 720.0;
    let w = p.width.max(1.0) * unit;
    let h = w * size.1 as f32 / size.0 as f32;
    let margin = p.margin * unit;
    let pos = p.pos.unwrap_or_else(|| {
        Vec2::new(
            margin + (screen.x - 2.0 * margin) * p.align.x,
            margin + (screen.y - 2.0 * margin) * p.align.y,
        )
    });
    let top_left = pos - Vec2::new(w, h) * Vec2::new(p.align.x, p.align.y);
    let mouse = world.canvas.to_virtual(world.input.mouse_position);
    let pad = 6.0 * unit;
    let hovered = !world.input.lock_mouse
        && mouse.x >= top_left.x - pad
        && mouse.x <= top_left.x + w + pad
        && mouse.y >= top_left.y - pad
        && mouse.y <= top_left.y + h + pad;
    let clicked = hovered && world.input.mouse_pressed(MouseButton::Left);
    if hovered {
        world.branding.badge_hovered = true;
    }
    if clicked {
        world.branding.request_open();
    }
    let alpha = if hovered { 1.0 } else { p.alpha.clamp(0.0, 1.0) };
    let layer = world.canvas.layer();
    world.canvas.set_layer(crate::canvas::Layer::Ui);
    let sp = SpriteParams {
        size: Some(Vec2::new(w, h)),
        color: Color::rgba(1.0, 1.0, 1.0, alpha),
        align: Align::TOP_LEFT,
        nearest: Some(false),
        ..Default::default()
    };
    world.canvas.sprite(tex, size, top_left, &sp);
    world.canvas.set_layer(layer);
    BadgeState { hovered, clicked }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splash_timeline() {
        let mut s = Splash::new();
        assert_eq!(s.alpha(), 0.0);
        s.advance(DELAY + FADE_IN + 0.1, false);
        assert_eq!(s.alpha(), 1.0);
        s.advance(0.01, true);
        assert!(s.alpha() > 0.95 && s.alpha() <= 1.0, "skip jumps to the fade-out");
        s.advance(FADE_OUT + TAIL + 0.01, false);
        assert!(s.finished());
    }

    #[test]
    fn early_skip_is_ignored() {
        let mut s = Splash::new();
        s.advance(0.1, true);
        assert!(s.alpha() < 0.01);
        assert!(!s.finished());
    }

    #[test]
    fn splash_draws_until_done_and_badge_clicks() {
        let mut world = World::new();
        world.canvas.set_output_size(1280, 720);
        let mut s = Splash::new();
        let mut frames = 0;
        while splash_frame(&mut world, &mut s, 1.0 / 60.0) {
            frames += 1;
            assert!(!world.canvas.is_empty());
        }
        assert!(frames as f32 >= Splash::DURATION * 60.0 - 2.0);

        world.canvas.begin_frame((1280, 720));
        world.input.mouse_moved(Vec2::new(1280.0 - 40.0, 720.0 - 30.0));
        world.input.mouse_button_event(MouseButton::Left, true);
        let st = badge(&mut world, &BadgeParams::default());
        assert!(st.hovered && st.clicked);
        assert!(world.branding.take_open_request());
        assert!(!world.branding.take_open_request());
        world.input.lock_mouse = true;
        assert!(!badge(&mut world, &BadgeParams::default()).hovered);
    }
}
