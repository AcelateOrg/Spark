//! Render settings of a game: internal resolution, texture sampling, custom shaders, post passes.
//!
//! The engine has no looks of its own: no presets, no built-in effects. The default surface
//! shader is plain Lambert lighting + fog. Everything else (retro, toon, VHS, CRT, ...) is a WGSL
//! shader written by the game, see [`crate::shader`].

use crate::shader::ShaderId;

/// Texture / upscale sampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextureFilter {
    /// Sharp pixels.
    Nearest,
    /// Smooth.
    #[default]
    Linear,
}

impl TextureFilter {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "nearest" | "pixel" => Some(Self::Nearest),
            "linear" | "smooth" => Some(Self::Linear),
            _ => None,
        }
    }
}

/// Resolution a post pass renders at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PassSize {
    /// Window / screenshot resolution.
    #[default]
    Screen,
    /// Internal 3D resolution (see [`RenderSettings::height`] / [`RenderSettings::scale`]).
    Scene,
}

/// One full-screen pass of a post shader.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostPass {
    pub shader: ShaderId,
    pub size: PassSize,
}

/// How the frame is produced. Frame = 3D scene at the internal resolution -> 2D "scene" layer ->
/// post passes in order (or a plain upscale when there are none) -> 2D "ui" layer.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderSettings {
    /// Internal 3D height in pixels (e.g. 240). `None` = use `scale`.
    pub height: Option<u32>,
    /// Internal resolution relative to the window when `height` is `None` (1 = native).
    pub scale: f32,
    /// How the internal image is sampled when it is stretched to the window (and by post passes).
    pub upscale: TextureFilter,
    /// How mesh textures are sampled.
    pub filter: TextureFilter,
    /// Surface shader for every material that has none of its own. `None` = the engine default.
    pub shader: Option<ShaderId>,
    /// Post passes, run in order.
    pub post: Vec<PostPass>,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self { height: None, scale: 1.0, upscale: TextureFilter::Linear, filter: TextureFilter::Linear, shader: None, post: Vec::new() }
    }
}

impl RenderSettings {
    /// Internal render size for an output of `width` x `height`.
    pub fn internal_size(&self, width: u32, height: u32) -> (u32, u32) {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let (iw, ih) = match self.height {
            Some(ph) => {
                let ih = (ph as f32).min(h).max(1.0);
                (w * ih / h, ih)
            }
            None => {
                let s = self.scale.clamp(0.05, 2.0);
                (w * s, h * s)
            }
        };
        ((iw.round() as u32).max(1), (ih.round() as u32).max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_size() {
        let mut r = RenderSettings::default();
        assert_eq!(r.internal_size(1280, 720), (1280, 720));
        r.height = Some(240);
        assert_eq!(r.internal_size(1280, 720), (427, 240));
        r.height = None;
        r.scale = 0.5;
        assert_eq!(r.internal_size(1280, 720), (640, 360));
    }
}
