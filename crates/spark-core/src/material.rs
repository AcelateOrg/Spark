use glam::Vec2;

use crate::assets::TextureId;
use crate::color::Color;
use crate::shader::ShaderId;

/// How a material is combined with what is behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlendMode {
    /// `Alpha` when `color.a < 1`, otherwise `Opaque`.
    #[default]
    Auto,
    /// Solid; pixels with alpha < 0.01 are cut out (foliage, fences).
    Opaque,
    /// See-through (glass, water, fading objects). Drawn after opaque objects, back to front.
    Alpha,
    /// Adds light (fire, lasers, glow). Drawn with the transparent objects.
    Additive,
}

impl BlendMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "opaque" => Some(Self::Opaque),
            "alpha" | "transparent" => Some(Self::Alpha),
            "additive" | "add" => Some(Self::Additive),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Opaque => "opaque",
            Self::Alpha => "alpha",
            Self::Additive => "additive",
        }
    }
}

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
    /// Blending with the background (see [`BlendMode`]).
    pub blend: BlendMode,
    /// Draw back faces too (leaves, flags, planes seen from below).
    pub double_sided: bool,
    /// Casts a shadow from the sun (`RenderSettings::shadows`).
    pub cast_shadow: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            color: Color::WHITE,
            texture: None,
            tiling: Vec2::ONE,
            unlit: false,
            shader: None,
            data: [0.0; 4],
            blend: BlendMode::Auto,
            double_sided: false,
            cast_shadow: true,
        }
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

    pub fn with_blend(mut self, blend: BlendMode) -> Self {
        self.blend = blend;
        self
    }

    pub fn with_double_sided(mut self, on: bool) -> Self {
        self.double_sided = on;
        self
    }

    /// The blend mode actually used (`Auto` resolved).
    pub fn effective_blend(&self) -> BlendMode {
        match self.blend {
            BlendMode::Auto if self.color.a < 0.999 => BlendMode::Alpha,
            BlendMode::Auto => BlendMode::Opaque,
            b => b,
        }
    }

    /// Drawn in the transparent pass (no depth writes, sorted back to front).
    pub fn is_transparent(&self) -> bool {
        matches!(self.effective_blend(), BlendMode::Alpha | BlendMode::Additive)
    }

    pub fn with_data(mut self, data: [f32; 4]) -> Self {
        self.data = data;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_blend_auto_alpha() {
        let m = Material::color(Color::rgba(1.0, 1.0, 1.0, 0.5));
        assert_eq!(m.effective_blend(), BlendMode::Alpha);
    }

    #[test]
    fn effective_blend_auto_opaque() {
        let m = Material::color(Color::WHITE);
        assert_eq!(m.effective_blend(), BlendMode::Opaque);
    }

    #[test]
    fn blend_parse_roundtrip() {
        for &mode in &[BlendMode::Auto, BlendMode::Opaque, BlendMode::Alpha, BlendMode::Additive] {
            assert_eq!(BlendMode::parse(mode.name()), Some(mode));
        }
    }

    #[test]
    fn material_builder_chain() {
        let m = Material::color(Color::RED)
            .with_blend(BlendMode::Additive)
            .with_double_sided(true)
            .with_tiling(2.0, 3.0);
        assert_eq!(m.blend, BlendMode::Additive);
        assert!(m.double_sided);
        assert_eq!(m.tiling, Vec2::new(2.0, 3.0));
    }
}
