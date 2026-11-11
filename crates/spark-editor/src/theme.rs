//! The editor look: one dark theme, Spark orange accent, Noto Sans (Latin + Cyrillic) for the UI.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

use crate::syntax::Token;

pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x7a, 0x1a);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x8a, 0x48, 0x18);
pub const BG: Color32 = Color32::from_rgb(0x1e, 0x1f, 0x22);
pub const PANEL: Color32 = Color32::from_rgb(0x2b, 0x2d, 0x30);
pub const SURFACE: Color32 = Color32::from_rgb(0x25, 0x26, 0x29);
pub const BORDER: Color32 = Color32::from_rgb(0x39, 0x3b, 0x40);
pub const TEXT: Color32 = Color32::from_rgb(0xdf, 0xe1, 0xe5);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8c, 0x8f, 0x94);
pub const OK: Color32 = Color32::from_rgb(0x5f, 0xb8, 0x65);
pub const WARN: Color32 = Color32::from_rgb(0xe0, 0xb0, 0x40);
pub const ERROR: Color32 = Color32::from_rgb(0xf0, 0x5a, 0x5a);
pub const INFO: Color32 = Color32::from_rgb(0x5a, 0x9e, 0xf0);

/// Glyphs from egui's built-in emoji / icon fonts.
pub mod icon {
    pub const RUN: &str = "▶";
    pub const STOP: &str = "⏹";
    pub const PAUSE: &str = "⏸";
    pub const STEP: &str = "⏭";
    pub const RELOAD: &str = "⟳";
    pub const CHECK: &str = "✔";
    pub const BUILD: &str = "📦";
    pub const FOLDER: &str = "📁";
    pub const FOLDER_OPEN: &str = "📂";
    pub const FILE: &str = "🖹";
    pub const SCRIPT: &str = "📜";
    pub const IMAGE: &str = "🖼";
    pub const SOUND: &str = "🎵";
    pub const MODEL: &str = "🧊";
    pub const SHADER: &str = "✨";
    pub const SETTINGS: &str = "⚙";
    pub const SEARCH: &str = "🔍";
    pub const INFO: &str = "ℹ";
    pub const WARN: &str = "⚠";
    pub const ERROR: &str = "❌";
    pub const BUG: &str = "🐞";
    pub const SCENE: &str = "🌳";
    pub const VIEWPORT: &str = "🎮";
    pub const CONSOLE: &str = "🖵";
    pub const DOT: &str = "•";
    pub const MENU: &str = "⋯";
}

pub fn apply(ctx: &egui::Context, ui_scale: f32) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals());
    ctx.set_zoom_factor(ui_scale);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 24.0;
        s.interaction.tooltip_delay = 0.4;
    });
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    // The engine's font: complete Latin + Cyrillic coverage, matches the in-game look.
    fonts.font_data.insert(
        "noto".into(),
        Arc::new(FontData::from_static(include_bytes!("../../spark-core/assets/fonts/NotoSans-Regular.ttf"))),
    );
    if let Some(p) = fonts.families.get_mut(&FontFamily::Proportional) {
        p.insert(0, "noto".into());
    }
    // Monospace keeps egui's Hack first; Noto fills in Cyrillic and other missing glyphs.
    if let Some(m) = fonts.families.get_mut(&FontFamily::Monospace) {
        m.push("noto".into());
    }
    fonts
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    let r = CornerRadius::same(4);
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = SURFACE;
    v.code_bg_color = BG;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.hyperlink_color = INFO;
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.override_text_color = None;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = Color32::from_rgb(0x3a, 0x3c, 0x41);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x34, 0x36, 0x3a);
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = Color32::from_rgb(0x46, 0x48, 0x4e);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x43, 0x45, 0x4a);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x55, 0x58, 0x5e));
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.hovered.corner_radius = r;

    w.active.bg_fill = ACCENT_DIM;
    w.active.weak_bg_fill = ACCENT_DIM;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
    w.active.corner_radius = r;

    w.open.bg_fill = Color32::from_rgb(0x3a, 0x3c, 0x41);
    w.open.weak_bg_fill = Color32::from_rgb(0x3a, 0x3c, 0x41);
    w.open.corner_radius = r;
    v
}

/// The orange call-to-action button.
pub fn primary_button<'a>(text: impl Into<egui::WidgetText>) -> egui::Button<'a> {
    egui::Button::new(egui::RichText::from(text.into().text()).color(Color32::WHITE).strong()).fill(ACCENT_DIM).stroke(Stroke::new(1.0, ACCENT))
}

pub fn token_color(token: Token) -> Color32 {
    match token {
        Token::Text => Color32::from_rgb(0xbc, 0xbe, 0xc4),
        Token::Keyword => Color32::from_rgb(0xcf, 0x8e, 0x6d),
        Token::Builtin => Color32::from_rgb(0x56, 0xa8, 0xf5),
        Token::Function => Color32::from_rgb(0x56, 0xc1, 0xd6),
        Token::Number => Color32::from_rgb(0x2a, 0xac, 0xb8),
        Token::String => Color32::from_rgb(0x6a, 0xab, 0x73),
        Token::Comment => Color32::from_rgb(0x7a, 0x7e, 0x85),
        Token::Punct => Color32::from_rgb(0xa9, 0xb7, 0xc6),
        Token::Heading => ACCENT,
    }
}

/// A stable color for a project avatar (welcome screen), from its name.
pub fn avatar_color(name: &str) -> Color32 {
    const COLORS: [Color32; 8] = [
        Color32::from_rgb(0xd9, 0x6c, 0x2c),
        Color32::from_rgb(0x3d, 0x8b, 0xd9),
        Color32::from_rgb(0x5f, 0xa8, 0x55),
        Color32::from_rgb(0xa8, 0x5f, 0xc9),
        Color32::from_rgb(0xc9, 0x4f, 0x6d),
        Color32::from_rgb(0x2f, 0xa5, 0x9b),
        Color32::from_rgb(0xc9, 0xa2, 0x2f),
        Color32::from_rgb(0x6d, 0x7b, 0xd9),
    ];
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    COLORS[h as usize % COLORS.len()]
}

/// Two-letter project initials: "My Game" -> "MG", "metro" -> "ME".
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let s: String = match words.as_slice() {
        [] => "?".into(),
        [one] => one.chars().take(2).collect(),
        [a, b, ..] => a.chars().take(1).chain(b.chars().take(1)).collect(),
    };
    s.to_uppercase()
}

#[cfg(test)]
mod tests {
    #[test]
    fn initials() {
        assert_eq!(super::initials("My Game"), "MG");
        assert_eq!(super::initials("metro"), "ME");
        assert_eq!(super::initials("глубокая ночь"), "ГН");
        assert_eq!(super::initials(""), "?");
    }
}
