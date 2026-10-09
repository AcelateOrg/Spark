//! Immediate-mode 2D drawing: shapes, sprites and text on top of (or inside) the 3D scene.
//!
//! Coordinates are *virtual pixels*: origin top-left, X right, Y down (like CSS / canvas / Love2D).
//! The screen is [`Canvas::virtual_height`] units tall (default 720) and as wide as the window
//! aspect requires, so a game looks the same at any resolution. Everything drawn is cleared at the
//! start of the next frame.
//!
//! Two layers: [`Layer::Ui`] (default) is drawn crisp at full resolution after the post passes;
//! [`Layer::Scene`] is drawn into the 3D image at the internal resolution, before the post passes
//! (use it for pixel-art 2D games).

use bytemuck::{Pod, Zeroable};
use glam::{Affine2, Vec2};

use crate::assets::TextureId;
use crate::color::Color;
use crate::text::{FontId, Fonts, MAX_GLYPH_PX, wrap_lines};

/// Default virtual screen height.
pub const DEFAULT_VIRTUAL_HEIGHT: f32 = 720.0;

/// One 2D vertex. `pos` in virtual pixels, `color` in linear RGBA.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Vertex2D {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// What a batch samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasTexture {
    /// Plain color (shapes).
    White,
    Texture(TextureId),
    /// The glyph atlas of [`Fonts`].
    Glyphs,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layer {
    /// Inside the 3D image: internal resolution, goes through the post passes.
    Scene,
    /// On top of everything at full resolution (HUD, menus, text).
    #[default]
    Ui,
}

impl Layer {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "ui" | "hud" | "overlay" => Some(Self::Ui),
            "scene" | "world" | "game" => Some(Self::Scene),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Scene => "scene",
            Self::Ui => "ui",
        }
    }
}

/// Consecutive triangles that share a texture and filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Batch {
    pub texture: CanvasTexture,
    pub nearest: bool,
    /// First vertex and vertex count (triangle list) in [`LayerData::vertices`].
    pub start: u32,
    pub count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct LayerData {
    pub vertices: Vec<Vertex2D>,
    pub batches: Vec<Batch>,
}

impl LayerData {
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}

/// Which point of a shape / sprite / text block sits at the given position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Align {
    /// 0 = left, 0.5 = center, 1 = right.
    pub x: f32,
    /// 0 = top, 0.5 = center, 1 = bottom.
    pub y: f32,
}

impl Align {
    pub const TOP_LEFT: Align = Align { x: 0.0, y: 0.0 };
    pub const CENTER: Align = Align { x: 0.5, y: 0.5 };
    pub const NAMES: &'static [&'static str] =
        &["topleft", "top", "topright", "left", "center", "right", "bottomleft", "bottom", "bottomright"];

    /// `"topleft"`, `"top"`, `"center"`, `"bottomright"`, ... (`-`, `_` and spaces are ignored).
    pub fn parse(name: &str) -> Option<Self> {
        let n: String = name.chars().filter(|c| !matches!(c, '-' | '_' | ' ')).collect::<String>().to_ascii_lowercase();
        let (x, y) = match n.as_str() {
            "topleft" | "lefttop" => (0.0, 0.0),
            "top" | "topcenter" => (0.5, 0.0),
            "topright" | "righttop" => (1.0, 0.0),
            "left" | "centerleft" | "leftcenter" => (0.0, 0.5),
            "center" | "middle" => (0.5, 0.5),
            "right" | "centerright" | "rightcenter" => (1.0, 0.5),
            "bottomleft" | "leftbottom" => (0.0, 1.0),
            "bottom" | "bottomcenter" => (0.5, 1.0),
            "bottomright" | "rightbottom" => (1.0, 1.0),
            _ => return None,
        };
        Some(Align { x, y })
    }
}

/// Options for [`Canvas::sprite`].
#[derive(Clone, Copy, Debug)]
pub struct SpriteParams {
    /// Drawn size in virtual pixels. Default: the region size in texture pixels.
    pub size: Option<Vec2>,
    pub scale: Vec2,
    /// Radians, clockwise on screen, around the aligned point.
    pub rotation: f32,
    pub color: Color,
    pub align: Align,
    pub flip_x: bool,
    pub flip_y: bool,
    /// Part of the texture in pixels: x, y, w, h. Default: whole texture.
    pub region: Option<[f32; 4]>,
    /// Override the canvas default filter.
    pub nearest: Option<bool>,
}

impl Default for SpriteParams {
    fn default() -> Self {
        Self {
            size: None,
            scale: Vec2::ONE,
            rotation: 0.0,
            color: Color::WHITE,
            align: Align::CENTER,
            flip_x: false,
            flip_y: false,
            region: None,
            nearest: None,
        }
    }
}

/// Options for [`Canvas::text`].
#[derive(Clone, Copy, Debug)]
pub struct TextParams {
    /// Font size in virtual pixels.
    pub size: f32,
    pub color: Color,
    /// Which point of the text block sits at the position; lines are aligned the same way horizontally.
    pub align: Align,
    pub font: FontId,
    /// Wrap lines to this width (virtual pixels).
    pub width: Option<f32>,
    /// Line spacing multiplier.
    pub line_height: f32,
}

impl Default for TextParams {
    fn default() -> Self {
        Self { size: 24.0, color: Color::WHITE, align: Align::TOP_LEFT, font: FontId::DEFAULT, width: None, line_height: 1.0 }
    }
}

/// The 2D drawing surface of a [`World`](crate::World).
#[derive(Debug)]
pub struct Canvas {
    /// Height of the screen in virtual pixels (width follows the aspect ratio). `<= 0` = real pixels.
    pub virtual_height: f32,
    /// Default sprite filter: nearest (pixel art) or linear (smooth).
    pub nearest: bool,
    pub fonts: Fonts,
    output: (u32, u32),
    internal: (u32, u32),
    layer: Layer,
    transform: Affine2,
    stack: Vec<Affine2>,
    layers: [LayerData; 2],
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            virtual_height: DEFAULT_VIRTUAL_HEIGHT,
            nearest: false,
            fonts: Fonts::new(),
            output: (1280, 720),
            internal: (1280, 720),
            layer: Layer::Ui,
            transform: Affine2::IDENTITY,
            stack: Vec::new(),
            layers: Default::default(),
        }
    }
}

impl Canvas {
    pub fn new() -> Self {
        Self::default()
    }

    /// Window / screenshot size in real pixels (set by the app every frame).
    pub fn set_output_size(&mut self, width: u32, height: u32) {
        self.output = (width.max(1), height.max(1));
    }

    pub fn output_size(&self) -> (u32, u32) {
        self.output
    }

    /// Clears all drawing. `internal` = size of the 3D image (internal resolution) for [`Layer::Scene`] text.
    pub fn begin_frame(&mut self, internal: (u32, u32)) {
        self.internal = (internal.0.max(1), internal.1.max(1));
        for l in &mut self.layers {
            l.vertices.clear();
            l.batches.clear();
        }
        self.transform = Affine2::IDENTITY;
        self.stack.clear();
        self.layer = Layer::Ui;
        self.fonts.begin_frame();
    }

    /// Settings back to defaults (hot reload). Keeps loaded fonts.
    pub fn reset(&mut self) {
        self.virtual_height = DEFAULT_VIRTUAL_HEIGHT;
        self.nearest = false;
        let internal = self.internal;
        self.begin_frame(internal);
    }

    fn height(&self) -> f32 {
        if self.virtual_height > 0.0 { self.virtual_height } else { self.output.1 as f32 }
    }

    /// Screen size in virtual pixels.
    pub fn size(&self) -> Vec2 {
        let h = self.height();
        Vec2::new(h * self.output.0 as f32 / self.output.1 as f32, h)
    }

    /// Real pixels per virtual pixel.
    pub fn scale(&self) -> f32 {
        self.output.1 as f32 / self.height()
    }

    /// Window pixel position (e.g. the mouse) -> virtual pixels.
    pub fn to_virtual(&self, pixels: Vec2) -> Vec2 {
        pixels / self.scale()
    }

    pub fn layer(&self) -> Layer {
        self.layer
    }

    pub fn set_layer(&mut self, layer: Layer) {
        self.layer = layer;
    }

    pub fn layer_data(&self, layer: Layer) -> &LayerData {
        &self.layers[layer as usize]
    }

    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(LayerData::is_empty)
    }

    // ---- transform stack ------------------------------------------------------------------

    pub fn transform(&self) -> Affine2 {
        self.transform
    }

    pub fn push(&mut self) {
        self.stack.push(self.transform);
    }

    /// Restores the transform saved by the matching [`Canvas::push`]. Returns false if there was none.
    pub fn pop(&mut self) -> bool {
        match self.stack.pop() {
            Some(t) => {
                self.transform = t;
                true
            }
            None => false,
        }
    }

    pub fn stack_depth(&self) -> usize {
        self.stack.len()
    }

    pub fn translate(&mut self, offset: Vec2) {
        self.transform = self.transform * Affine2::from_translation(offset);
    }

    /// Radians, clockwise on screen.
    pub fn rotate(&mut self, angle: f32) {
        self.transform = self.transform * Affine2::from_angle(angle);
    }

    pub fn scale_by(&mut self, s: Vec2) {
        self.transform = self.transform * Affine2::from_scale(s);
    }

    // ---- primitives -----------------------------------------------------------------------

    fn emit(&mut self, texture: CanvasTexture, nearest: bool, verts: &[Vertex2D]) {
        if verts.is_empty() {
            return;
        }
        let layer = &mut self.layers[self.layer as usize];
        let start = layer.vertices.len() as u32;
        layer.vertices.extend_from_slice(verts);
        match layer.batches.last_mut() {
            Some(b) if b.texture == texture && b.nearest == nearest => b.count += verts.len() as u32,
            _ => layer.batches.push(Batch { texture, nearest, start, count: verts.len() as u32 }),
        }
    }

    fn vertex(&self, p: Vec2, uv: Vec2, color: [f32; 4]) -> Vertex2D {
        Vertex2D { pos: self.transform.transform_point2(p).to_array(), uv: uv.to_array(), color }
    }

    /// A textured quad; corners in order top-left, top-right, bottom-right, bottom-left.
    pub fn quad(&mut self, p: [Vec2; 4], uv: [Vec2; 4], color: Color, texture: CanvasTexture, nearest: bool) {
        let c = color.to_linear();
        let v: [Vertex2D; 4] = std::array::from_fn(|i| self.vertex(p[i], uv[i], c));
        self.emit(texture, nearest, &[v[0], v[1], v[2], v[0], v[2], v[3]]);
    }

    /// Untextured triangles (every 3 points = one triangle).
    pub fn triangles(&mut self, points: &[Vec2], color: Color) {
        let c = color.to_linear();
        let verts: Vec<Vertex2D> = points[..points.len() / 3 * 3].iter().map(|p| self.vertex(*p, Vec2::ZERO, c)).collect();
        self.emit(CanvasTexture::White, false, &verts);
    }

    fn aligned(pos: Vec2, size: Vec2, align: Align) -> Vec2 {
        pos - size * Vec2::new(align.x, align.y)
    }

    /// Filled rectangle.
    pub fn rect(&mut self, pos: Vec2, size: Vec2, color: Color, align: Align) {
        let tl = Self::aligned(pos, size, align);
        let br = tl + size;
        let z = Vec2::ZERO;
        self.quad([tl, Vec2::new(br.x, tl.y), br, Vec2::new(tl.x, br.y)], [z; 4], color, CanvasTexture::White, false);
    }

    /// Rectangle outline, `thickness` drawn inside the rectangle.
    pub fn rect_line(&mut self, pos: Vec2, size: Vec2, thickness: f32, color: Color, align: Align) {
        let tl = Self::aligned(pos, size, align);
        let t = thickness.min(size.x / 2.0).min(size.y / 2.0).max(0.0);
        let a = Align::TOP_LEFT;
        self.rect(tl, Vec2::new(size.x, t), color, a);
        self.rect(Vec2::new(tl.x, tl.y + size.y - t), Vec2::new(size.x, t), color, a);
        self.rect(Vec2::new(tl.x, tl.y + t), Vec2::new(t, size.y - 2.0 * t), color, a);
        self.rect(Vec2::new(tl.x + size.x - t, tl.y + t), Vec2::new(t, size.y - 2.0 * t), color, a);
    }

    fn segments(&self, radius: f32) -> usize {
        let px = radius * self.scale() * self.transform.matrix2.determinant().abs().sqrt();
        (px * 0.75).clamp(12.0, 128.0) as usize
    }

    pub fn circle(&mut self, center: Vec2, radius: f32, color: Color) {
        let n = self.segments(radius);
        let mut pts = Vec::with_capacity(n * 3);
        for i in 0..n {
            let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
            let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
            pts.extend([center, center + Vec2::from_angle(a0) * radius, center + Vec2::from_angle(a1) * radius]);
        }
        self.triangles(&pts, color);
    }

    /// Circle outline, `thickness` drawn inside the radius.
    pub fn circle_line(&mut self, center: Vec2, radius: f32, thickness: f32, color: Color) {
        let n = self.segments(radius);
        let inner = (radius - thickness).max(0.0);
        let mut pts = Vec::with_capacity(n * 6);
        for i in 0..n {
            let d0 = Vec2::from_angle(i as f32 / n as f32 * std::f32::consts::TAU);
            let d1 = Vec2::from_angle((i + 1) as f32 / n as f32 * std::f32::consts::TAU);
            let (o0, o1, i0, i1) = (center + d0 * radius, center + d1 * radius, center + d0 * inner, center + d1 * inner);
            pts.extend([o0, o1, i1, o0, i1, i0]);
        }
        self.triangles(&pts, color);
    }

    /// Filled convex polygon (points in order, either winding).
    pub fn polygon(&mut self, points: &[Vec2], color: Color) {
        if points.len() < 3 {
            return;
        }
        let mut tris = Vec::with_capacity((points.len() - 2) * 3);
        for i in 1..points.len() - 1 {
            tris.extend([points[0], points[i], points[i + 1]]);
        }
        self.triangles(&tris, color);
    }

    /// Closed outline through the points.
    pub fn polygon_line(&mut self, points: &[Vec2], width: f32, color: Color) {
        for i in 0..points.len() {
            self.line(points[i], points[(i + 1) % points.len()], width, color);
        }
    }

    pub fn line(&mut self, a: Vec2, b: Vec2, width: f32, color: Color) {
        let d = b - a;
        if d.length_squared() < 1e-12 {
            return;
        }
        let n = d.perp().normalize() * (width * 0.5);
        self.triangles(&[a + n, b + n, b - n, a + n, b - n, a - n], color);
    }

    /// Draws a texture. `texture_size` = full texture size in pixels.
    pub fn sprite(&mut self, texture: TextureId, texture_size: (u32, u32), pos: Vec2, p: &SpriteParams) {
        let (tw, th) = (texture_size.0.max(1) as f32, texture_size.1.max(1) as f32);
        let [rx, ry, rw, rh] = p.region.unwrap_or([0.0, 0.0, tw, th]);
        let size = p.size.unwrap_or(Vec2::new(rw, rh)) * p.scale;
        let tl = -size * Vec2::new(p.align.x, p.align.y);
        let corners = [tl, tl + Vec2::new(size.x, 0.0), tl + size, tl + Vec2::new(0.0, size.y)];
        let rot = Affine2::from_angle_translation(p.rotation, pos);
        let corners = corners.map(|c| rot.transform_point2(c));
        let (mut u0, mut u1) = (rx / tw, (rx + rw) / tw);
        let (mut v0, mut v1) = (ry / th, (ry + rh) / th);
        if p.flip_x {
            std::mem::swap(&mut u0, &mut u1);
        }
        if p.flip_y {
            std::mem::swap(&mut v0, &mut v1);
        }
        let uv = [Vec2::new(u0, v0), Vec2::new(u1, v0), Vec2::new(u1, v1), Vec2::new(u0, v1)];
        let nearest = p.nearest.unwrap_or(self.nearest);
        self.quad(corners, uv, p.color, CanvasTexture::Texture(texture), nearest);
    }

    /// Raster pixel size + raster pixels per virtual pixel for text at `size` on the current layer.
    fn text_raster(&self, size: f32) -> (u16, f32) {
        let target_h = match self.layer {
            Layer::Ui => self.output.1,
            Layer::Scene => self.internal.1,
        } as f32;
        let zoom = self.transform.matrix2.determinant().abs().sqrt().max(1e-3);
        let px = (size * target_h / self.height() * zoom).round().clamp(1.0, MAX_GLYPH_PX as f32);
        (px as u16, px / size.max(1e-3))
    }

    fn line_width(&mut self, line: &str, font: FontId, px: u16, k: f32) -> f32 {
        let mut w = 0.0;
        let mut prev = None;
        for ch in line.chars() {
            if let Some(p) = prev {
                w += self.fonts.kern(font, p, ch, px);
            }
            w += self.fonts.glyph(font, ch, px).advance;
            prev = Some(ch);
        }
        w / k
    }

    fn layout(&mut self, text: &str, p: &TextParams) -> (Vec<(String, f32)>, Vec2, u16, f32) {
        let size = p.size.max(1.0);
        let (px, k) = self.text_raster(size);
        let lines = wrap_lines(text, p.width, |s| self.line_width(s, p.font, px, k));
        let lines: Vec<(String, f32)> = lines.into_iter().map(|l| {
            let w = self.line_width(&l, p.font, px, k);
            (l, w)
        }).collect();
        let m = self.fonts.line_metrics(p.font, px);
        let line_h = m.line_height / k * p.line_height;
        let width = lines.iter().map(|(_, w)| *w).fold(0.0, f32::max);
        let height = m.line_height / k + line_h * (lines.len().saturating_sub(1)) as f32;
        (lines, Vec2::new(width, height), px, k)
    }

    /// Size of a text block in virtual pixels (same rules as [`Canvas::text`]).
    pub fn measure_text(&mut self, text: &str, p: &TextParams) -> Vec2 {
        self.layout(text, p).1
    }

    /// Draws text and returns the size of the block in virtual pixels.
    pub fn text(&mut self, text: &str, pos: Vec2, p: &TextParams) -> Vec2 {
        let (lines, block, px, k) = self.layout(text, p);
        let m = self.fonts.line_metrics(p.font, px);
        let line_h = m.line_height / k * p.line_height;
        let top_left = Self::aligned(pos, block, p.align);
        let mut quads = Vec::new();
        for (i, (line, w)) in lines.iter().enumerate() {
            let mut pen = Vec2::new(top_left.x + (block.x - w) * p.align.x, top_left.y + i as f32 * line_h + m.ascent / k);
            let mut prev = None;
            for ch in line.chars() {
                if let Some(pc) = prev {
                    pen.x += self.fonts.kern(p.font, pc, ch, px) / k;
                }
                let g = self.fonts.glyph(p.font, ch, px);
                if g.size.x > 0.0 {
                    let tl = pen + g.offset / k;
                    let sz = g.size / k;
                    quads.push((tl, sz, g.uv_min, g.uv_max));
                }
                pen.x += g.advance / k;
                prev = Some(ch);
            }
        }
        for (tl, sz, a, b) in quads {
            let br = tl + sz;
            self.quad(
                [tl, Vec2::new(br.x, tl.y), br, Vec2::new(tl.x, br.y)],
                [a, Vec2::new(b.x, a.y), b, Vec2::new(a.x, b.y)],
                p.color,
                CanvasTexture::Glyphs,
                false,
            );
        }
        block
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Canvas {
        let mut c = Canvas::new();
        c.set_output_size(1920, 1080);
        c.begin_frame((1920, 1080));
        c
    }

    #[test]
    fn virtual_size_follows_aspect() {
        let c = canvas();
        assert_eq!(c.size(), Vec2::new(1280.0, 720.0));
        assert_eq!(c.scale(), 1.5);
        assert_eq!(c.to_virtual(Vec2::new(960.0, 540.0)), Vec2::new(640.0, 360.0));
    }

    #[test]
    fn batching_and_layers() {
        let mut c = canvas();
        c.rect(Vec2::ZERO, Vec2::splat(10.0), Color::RED, Align::TOP_LEFT);
        c.circle(Vec2::splat(50.0), 5.0, Color::BLUE);
        let ui = c.layer_data(Layer::Ui);
        assert_eq!(ui.batches.len(), 1, "shapes share one batch");
        assert_eq!(ui.vertices[0].pos, [0.0, 0.0]);
        assert_eq!(ui.vertices[2].pos, [10.0, 10.0]);
        c.set_layer(Layer::Scene);
        c.sprite(TextureId(3), (16, 16), Vec2::splat(100.0), &SpriteParams::default());
        let scene = c.layer_data(Layer::Scene);
        assert_eq!(scene.batches[0].texture, CanvasTexture::Texture(TextureId(3)));
        // Centered by default: 16x16 around (100, 100).
        assert_eq!(scene.vertices[0].pos, [92.0, 92.0]);
        c.begin_frame((1920, 1080));
        assert!(c.is_empty());
        assert_eq!(c.layer(), Layer::Ui);
    }

    #[test]
    fn transforms() {
        let mut c = canvas();
        c.push();
        c.translate(Vec2::new(100.0, 0.0));
        c.scale_by(Vec2::splat(2.0));
        c.rect(Vec2::ZERO, Vec2::splat(10.0), Color::RED, Align::TOP_LEFT);
        assert!(c.pop());
        assert!(!c.pop());
        let v = &c.layer_data(Layer::Ui).vertices;
        assert_eq!(v[2].pos, [120.0, 20.0]);
    }

    #[test]
    fn text_alignment() {
        let mut c = canvas();
        let p = TextParams { size: 32.0, align: Align::CENTER, ..Default::default() };
        let size = c.measure_text("Hello", &p);
        assert!(size.x > 40.0 && size.y >= 32.0, "{size}");
        let drawn = c.text("Hello", Vec2::new(640.0, 360.0), &p);
        assert_eq!(size, drawn);
        let v = &c.layer_data(Layer::Ui).vertices;
        let minx = v.iter().map(|v| v.pos[0]).fold(f32::MAX, f32::min);
        let maxx = v.iter().map(|v| v.pos[0]).fold(f32::MIN, f32::max);
        assert!(((minx + maxx) / 2.0 - 640.0).abs() < 4.0, "centered: {minx}..{maxx}");
        let two = c.measure_text("a\nb", &TextParams::default());
        let one = c.measure_text("a", &TextParams::default());
        assert!(two.y > one.y * 1.5);
        let wrapped = c.measure_text("word word word word", &TextParams { width: Some(80.0), ..Default::default() });
        assert!(wrapped.x <= 80.0 && wrapped.y > one.y * 1.5);
    }

    #[test]
    fn align_names() {
        for n in Align::NAMES {
            assert!(Align::parse(n).is_some(), "{n}");
        }
        assert_eq!(Align::parse("Bottom-Right"), Some(Align { x: 1.0, y: 1.0 }));
        assert_eq!(Align::parse("nope"), None);
    }
}
