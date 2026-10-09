//! Fonts and the glyph atlas used by 2D text. Pure CPU data: the renderer uploads the atlas when it changes.
//!
//! The built-in font is Noto Sans (Latin, Cyrillic, Greek; SIL Open Font License, see assets/fonts/OFL.txt).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use glam::Vec2;

use crate::assets::ImageData;

const DEFAULT_FONT: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
const ATLAS_SIZE: u32 = 1024;
const PAD: u32 = 1;
/// Glyphs bigger than this (in raster pixels) are drawn at this size and scaled up.
pub const MAX_GLYPH_PX: u16 = 256;

/// Handle to a loaded font. `FontId::DEFAULT` is the built-in font.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontId(pub u32);

impl FontId {
    pub const DEFAULT: FontId = FontId(0);
}

/// A rasterized glyph in the atlas. Distances are in raster pixels.
#[derive(Clone, Copy, Debug, Default)]
pub struct Glyph {
    /// Top-left of the bitmap relative to the pen position on the baseline (Y down).
    pub offset: Vec2,
    /// Bitmap size (zero for spaces).
    pub size: Vec2,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    /// How far the pen moves after this glyph.
    pub advance: f32,
}

/// Vertical metrics of a font at a pixel size (raster pixels).
#[derive(Clone, Copy, Debug)]
pub struct LineMetrics {
    pub ascent: f32,
    pub line_height: f32,
}

/// All fonts + one shared glyph atlas (RGBA: white with coverage in alpha).
pub struct Fonts {
    fonts: Vec<fontdue::Font>,
    paths: HashMap<PathBuf, FontId>,
    cache: HashMap<(u32, char, u16), Glyph>,
    atlas: ImageData,
    version: u64,
    cursor: (u32, u32),
    row_height: u32,
    full: bool,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts").field("fonts", &self.fonts.len()).field("glyphs", &self.cache.len()).finish()
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    pub fn new() -> Self {
        let default = fontdue::Font::from_bytes(DEFAULT_FONT, fontdue::FontSettings::default()).expect("built-in font");
        Self {
            fonts: vec![default],
            paths: HashMap::new(),
            cache: HashMap::new(),
            atlas: ImageData::new(ATLAS_SIZE, ATLAS_SIZE, vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize]),
            version: 1,
            cursor: (PAD, PAD),
            row_height: 0,
            full: false,
        }
    }

    /// Loads a .ttf / .otf file once; later calls with the same path return the same id.
    pub fn load(&mut self, path: &Path) -> Result<FontId, String> {
        if let Some(&id) = self.paths.get(path) {
            return Ok(id);
        }
        let bytes = crate::vfs::read(path).map_err(|e| format!("cannot read font: {e}"))?;
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|e| format!("'{}' is not a valid .ttf/.otf font: {e}", path.display()))?;
        self.fonts.push(font);
        let id = FontId(self.fonts.len() as u32 - 1);
        self.paths.insert(path.to_path_buf(), id);
        Ok(id)
    }

    pub fn contains(&self, font: FontId) -> bool {
        (font.0 as usize) < self.fonts.len()
    }

    fn font(&self, font: FontId) -> &fontdue::Font {
        self.fonts.get(font.0 as usize).unwrap_or(&self.fonts[0])
    }

    pub fn line_metrics(&self, font: FontId, px: u16) -> LineMetrics {
        let px = px as f32;
        match self.font(font).horizontal_line_metrics(px) {
            Some(m) => LineMetrics { ascent: m.ascent, line_height: m.new_line_size },
            None => LineMetrics { ascent: px * 0.8, line_height: px * 1.2 },
        }
    }

    pub fn kern(&self, font: FontId, left: char, right: char, px: u16) -> f32 {
        self.font(font).horizontal_kern(left, right, px as f32).unwrap_or(0.0)
    }

    /// The glyph for `ch` at `px` pixels, rasterized into the atlas on first use.
    pub fn glyph(&mut self, font: FontId, ch: char, px: u16) -> Glyph {
        let font = if self.contains(font) { font } else { FontId::DEFAULT };
        let key = (font.0, ch, px);
        if let Some(g) = self.cache.get(&key) {
            return *g;
        }
        let (m, bitmap) = self.font(font).rasterize(ch, px as f32);
        let mut glyph = Glyph {
            offset: Vec2::new(m.xmin as f32, -(m.ymin as f32 + m.height as f32)),
            size: Vec2::new(m.width as f32, m.height as f32),
            advance: m.advance_width,
            ..Default::default()
        };
        if m.width > 0 && m.height > 0 {
            match self.pack(m.width as u32, m.height as u32) {
                Some((x, y)) => {
                    for row in 0..m.height {
                        for col in 0..m.width {
                            let i = (((y + row as u32) * ATLAS_SIZE + x + col as u32) * 4) as usize;
                            self.atlas.pixels[i..i + 4].copy_from_slice(&[255, 255, 255, bitmap[row * m.width + col]]);
                        }
                    }
                    let s = ATLAS_SIZE as f32;
                    glyph.uv_min = Vec2::new(x as f32 / s, y as f32 / s);
                    glyph.uv_max = Vec2::new((x + m.width as u32) as f32 / s, (y + m.height as u32) as f32 / s);
                    self.version += 1;
                }
                // Atlas is full: skip drawing this glyph now; the atlas is rebuilt next frame.
                None => {
                    self.full = true;
                    glyph.size = Vec2::ZERO;
                    return glyph;
                }
            }
        }
        self.cache.insert(key, glyph);
        glyph
    }

    fn pack(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w + 2 * PAD > ATLAS_SIZE || h + 2 * PAD > ATLAS_SIZE {
            return None;
        }
        if self.cursor.0 + w + PAD > ATLAS_SIZE {
            self.cursor = (PAD, self.cursor.1 + self.row_height + PAD);
            self.row_height = 0;
        }
        if self.cursor.1 + h + PAD > ATLAS_SIZE {
            return None;
        }
        let at = self.cursor;
        self.cursor.0 += w + PAD;
        self.row_height = self.row_height.max(h);
        Some(at)
    }

    /// Call once per frame: rebuilds the atlas if it overflowed last frame.
    pub fn begin_frame(&mut self) {
        if self.full {
            self.full = false;
            self.cache.clear();
            self.atlas.pixels.fill(0);
            self.cursor = (PAD, PAD);
            self.row_height = 0;
            self.version += 1;
            log::debug!("glyph atlas was full and has been rebuilt");
        }
    }

    /// RGBA atlas image (white, coverage in alpha). Not sRGB-encoded.
    pub fn atlas(&self) -> &ImageData {
        &self.atlas
    }

    /// Changes whenever the atlas pixels change.
    pub fn version(&self) -> u64 {
        self.version
    }
}

/// Splits `text` into lines at `\n` and, if `max_width` is set, wraps words so no line is wider.
/// `width_of` measures a string.
pub fn wrap_lines(text: &str, max_width: Option<f32>, mut width_of: impl FnMut(&str) -> f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let Some(max) = max_width.filter(|w| *w > 0.0) else {
            out.push(para.to_string());
            continue;
        };
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if width_of(&candidate) <= max || line.is_empty() && width_of(word) <= max {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            // A single word wider than the line: break it by characters.
            if width_of(word) > max {
                let mut part = String::new();
                for ch in word.chars() {
                    part.push(ch);
                    if width_of(&part) > max && part.chars().count() > 1 {
                        part.pop();
                        out.push(std::mem::take(&mut part));
                        part.push(ch);
                    }
                }
                line = part;
            } else {
                line = word.to_string();
            }
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_are_cached_and_packed() {
        let mut f = Fonts::new();
        let v0 = f.version();
        let a = f.glyph(FontId::DEFAULT, 'A', 32);
        assert!(a.size.x > 0.0 && a.advance > 0.0);
        assert!(f.version() > v0);
        let v1 = f.version();
        f.glyph(FontId::DEFAULT, 'A', 32);
        assert_eq!(f.version(), v1, "second lookup comes from the cache");
        let space = f.glyph(FontId::DEFAULT, ' ', 32);
        assert_eq!(space.size, Vec2::ZERO);
        assert!(space.advance > 0.0);
        let ya = f.glyph(FontId::DEFAULT, 'Я', 32);
        assert!(ya.size.x > 0.0, "Cyrillic is in the built-in font");
    }

    #[test]
    fn wrapping() {
        let w = |s: &str| s.chars().count() as f32;
        assert_eq!(wrap_lines("a b\nc", None, w), vec!["a b", "c"]);
        assert_eq!(wrap_lines("one two three", Some(7.0), w), vec!["one two", "three"]);
        assert_eq!(wrap_lines("abcdefgh", Some(3.0), w), vec!["abc", "def", "gh"]);
    }
}
