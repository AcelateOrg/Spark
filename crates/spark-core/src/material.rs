use glam::Vec2;

use crate::assets::TextureId;
use crate::color::Color;
use crate::shader::ShaderId;

/// How a mesh looks. Lit by the sun + ambient unless `unlit`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Base color (multiplied with the texture).
    pub color: Color,
    pub texture: Option<TextureId>,
    /// Texture repeats across the surface (e.g. `(4, 4)` on a ground plane).
    pub tiling: Vec2,
    /// Ignore lighting: shows the exact color (UI, glowing things, sprites).
    pub unlit: bool,
    /// Surface shader (`None` = [`crate::RenderSettings::shader`], else the engine default).
    pub shader: Option<ShaderId>,
    /// Free per-material numbers for custom shaders (`object.data` in WGSL).
    pub data: [f32; 4],
}

impl Default for Material {
    fn default() -> Self {
        Self { color: Color::WHITE, texture: None, tiling: Vec2::ONE, unlit: false, shader: None, data: [0.0; 4] }
    }
}

impl Material {
    /// Lit, solid color.
    pub fn color(color: Color) -> Self {
        Self { color, ..Default::default() }
    }

    /// Unlit, solid color.
    pub fn unlit(color: Color) -> Self {
        Self { color, unlit: true, ..Default::default() }
    }

    /// Lit, textured.
    pub fn textured(texture: TextureId) -> Self {
        Self { texture: Some(texture), ..Default::default() }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_texture(mut self, texture: TextureId) -> Self {
        self.texture = Some(texture);
        self
    }

    pub fn with_tiling(mut self, x: f32, y: f32) -> Self {
        self.tiling = Vec2::new(x, y);
        self
    }

    pub fn with_unlit(mut self, unlit: bool) -> Self {
        self.unlit = unlit;
        self
    }

    pub fn with_shader(mut self, shader: Option<ShaderId>) -> Self {
        self.shader = shader;
        self
    }

    pub fn with_data(mut self, data: [f32; 4]) -> Self {
        self.data = data;
        self
    }
}
