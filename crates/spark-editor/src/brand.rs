//! The SPARK logo in the editor: the sparkle mark (sidebar, toolbar), the wordmark (About) and the window icon.
//!
//! `assets/` holds small copies made from the full-size logo (white on transparent, so they can be tinted).

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions, vec2};

const MARK_PNG: &[u8] = include_bytes!("../assets/spark-mark.png");
const LOGO_PNG: &[u8] = include_bytes!("../assets/spark-logo.png");
const ICON_PNG: &[u8] = include_bytes!("../assets/app-icon.png");

fn decode(png: &[u8]) -> Option<image::RgbaImage> {
    image::load_from_memory(png).map(|i| i.to_rgba8()).map_err(|e| log::warn!("logo: {e}")).ok()
}

/// Uploads once per egui context, then reuses the texture.
fn texture(ctx: &egui::Context, name: &'static str, png: &'static [u8]) -> Option<TextureHandle> {
    let id = egui::Id::new(("spark-brand", name));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(t);
    }
    let img = decode(png)?;
    let color = ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    let tex = ctx.load_texture(name, color, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}

/// The sparkle mark, `size` points square, tinted.
pub fn mark(ui: &mut egui::Ui, size: f32, tint: Color32) -> egui::Response {
    match texture(ui.ctx(), "spark-mark", MARK_PNG) {
        Some(t) => ui.add(egui::Image::new((t.id(), vec2(size, size))).tint(tint)),
        None => ui.label(egui::RichText::new("✦").size(size).color(tint)),
    }
}

/// The SPARK wordmark, `height` points high.
pub fn wordmark(ui: &mut egui::Ui, height: f32, tint: Color32) -> egui::Response {
    match texture(ui.ctx(), "spark-logo", LOGO_PNG) {
        Some(t) => {
            let [w, h] = t.size();
            ui.add(egui::Image::new((t.id(), vec2(height * w as f32 / h as f32, height))).tint(tint))
        }
        None => ui.label(egui::RichText::new("SPARK").size(height).strong().color(tint)),
    }
}

/// Window / taskbar icon.
pub fn window_icon() -> Option<egui::IconData> {
    let img = decode(ICON_PNG)?;
    Some(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() })
}
