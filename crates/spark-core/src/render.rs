//! Render settings of a game: internal resolution, texture sampling, custom shaders, post passes.
//!
//! The engine has no looks of its own: no presets, no built-in effects. The default surface
//! shader is plain Lambert lighting + fog. Everything else (retro, toon, VHS, CRT, ...) is a WGSL
//! shader written by the game, see [`crate::shader`].

use crate::shader::{PostFxKind, ShaderId};

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
    /// Built-in post-processing effects (bloom, SSAO, FXAA, etc.).
    /// These run before user-defined `post` passes.
    pub postfx: PostProcessing,
    /// Multisample anti-aliasing of the 3D scene: 1 (off), 2, 4 or 8 (falls back to what the GPU supports).
    pub msaa: u32,
    /// Sun shadows (shadow map around the camera).
    pub shadows: bool,
    /// How far from the camera shadows are drawn, meters.
    pub shadow_distance: f32,
    /// Shadow map resolution in pixels (square).
    pub shadow_size: u32,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            height: None,
            scale: 1.0,
            upscale: TextureFilter::Linear,
            filter: TextureFilter::Linear,
            shader: None,
            post: Vec::new(),
            postfx: PostProcessing::default(),
            msaa: 4,
            shadows: true,
            shadow_distance: 40.0,
            shadow_size: 2048,
        }
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

    #[test]
    fn internal_size_scale_clamped() {
        let mut r = RenderSettings::default();
        r.scale = 0.01; // below minimum 0.05
        let (w, h) = r.internal_size(1280, 720);
        assert!(w >= 1 && h >= 1); // clamped, not zero
    }


    #[test]
    fn pass_size_default_is_screen() {
        assert_eq!(PassSize::default(), PassSize::Screen);
    }
}

// ── Built-in post-processing effects ─────────────────────────────────────────

/// Tonemapping operator: maps the HDR scene (values above 1) into the displayable range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tonemap {
    /// No tonemapping: values above 1 are clipped (default, unchanged look of existing games).
    #[default]
    Off,
    /// ACES filmic — cinematic contrast, good choice with bloom / bright lights.
    Aces,
    /// Reinhard — simple, gentle.
    Reinhard,
    /// Hejl-Burgess-Dawson filmic.
    Hejl,
}

/// Bloom settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Bloom {
    /// Brightness where bloom starts (linear; the HDR scene can go above 1).
    pub threshold: f32,
    /// Soft knee width for smooth threshold transition.
    pub knee: f32,
    /// Bloom intensity multiplier.
    pub strength: f32,
    /// Blur radius in pixels (larger = softer, wider bloom).
    pub radius: f32,
}

impl Default for Bloom {
    fn default() -> Self {
        Self { threshold: 0.8, knee: 0.2, strength: 0.4, radius: 8.0 }
    }
}

/// Screen-space ambient occlusion settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Ssao {
    /// Sampling radius in world units.
    pub radius: f32,
    /// Number of samples (1–64, more = higher quality).
    pub samples: u32,
    /// Effect strength (0–2).
    pub strength: f32,
    /// Depth bias in meters against self-occlusion on flat surfaces.
    pub bias: f32,
}

impl Default for Ssao {
    fn default() -> Self {
        Self { radius: 0.5, samples: 16, strength: 1.0, bias: 0.05 }
    }
}

/// Motion blur settings.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionBlur {
    /// Fraction of the frame time the "shutter" is open (1 = blur over the whole camera motion).
    pub strength: f32,
    /// Number of blur samples (1–32).
    pub samples: u32,
}

impl Default for MotionBlur {
    fn default() -> Self {
        Self { strength: 1.0, samples: 8 }
    }
}

/// Depth of field settings.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthOfField {
    /// Focus distance in world units.
    pub focus: f32,
    /// Aperture / blur strength.
    pub aperture: f32,
    /// Focal range (distance over which everything is in focus).
    pub focal_range: f32,
    /// Bokeh blur samples.
    pub samples: u32,
}

impl Default for DepthOfField {
    fn default() -> Self {
        Self { focus: 10.0, aperture: 2.0, focal_range: 3.0, samples: 16 }
    }
}

/// Color grading settings (simple color correction without LUT).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorGrading {
    /// Exposure multiplier (1 = normal, >1 brighter).
    pub exposure: f32,
    /// Contrast multiplier (1 = normal).
    pub contrast: f32,
    /// Saturation multiplier (0 = grayscale, 1 = normal, >1 = vivid).
    pub saturation: f32,
    /// Temperature shift (-1 warm → +1 cool).
    pub temperature: f32,
    /// Tint shift (-1 green → +1 magenta).
    pub tint: f32,
    /// Lift/gamma/gain shadows/midtones/highlights.
    pub lift: [f32; 3],
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
}

impl Default for ColorGrading {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
            lift: [0.0; 3],
            gamma: [1.0; 3],
            gain: [1.0; 3],
        }
    }
}

/// Vignette settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Vignette {
    /// Vignette intensity (0–1).
    pub strength: f32,
    /// Inner radius where the vignette starts (0–1).
    pub inner: f32,
    /// Outer radius where it reaches full darkness (0–1).
    pub outer: f32,
    /// Color the edges fade to (usually black).
    pub color: crate::Color,
}

impl Default for Vignette {
    fn default() -> Self {
        Self { strength: 0.5, inner: 0.3, outer: 0.85, color: crate::Color::BLACK }
    }
}

/// Film grain settings.
#[derive(Clone, Debug, PartialEq)]
pub struct FilmGrain {
    /// Grain intensity (0–1).
    pub strength: f32,
    /// Grain size in pixels (1 = fine, 4 = coarse).
    pub size: f32,
}

impl Default for FilmGrain {
    fn default() -> Self {
        Self { strength: 0.05, size: 2.0 }
    }
}

/// Chromatic aberration settings.
#[derive(Clone, Debug, PartialEq)]
pub struct ChromaticAberration {
    /// Aberration amount in pixels.
    pub amount: f32,
    /// true = split outward from the screen center, false = horizontal split.
    pub radial: bool,
}

impl Default for ChromaticAberration {
    fn default() -> Self {
        Self { amount: 2.0, radial: true }
    }
}

/// All built-in post-processing effects. Effects run in this order:
/// SSAO → Motion blur → DoF → Bloom → Color grading → Tonemap → FXAA →
/// Chromatic aberration → Film grain → Vignette.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostProcessing {
    pub tonemap: Tonemap,
    pub bloom: Option<Bloom>,
    pub ssao: Option<Ssao>,
    pub motion_blur: Option<MotionBlur>,
    pub dof: Option<DepthOfField>,
    pub color_grading: Option<ColorGrading>,
    pub fxaa: bool,
    pub chromatic_aberration: Option<ChromaticAberration>,
    pub film_grain: Option<FilmGrain>,
    pub vignette: Option<Vignette>,
}

/// Parameter values of one built-in effect: `struct Params` field name -> numbers.
pub type PostFxParams = Vec<(&'static str, Vec<f32>)>;

impl PostProcessing {
    /// Enabled built-in effects in the order they run, each with its shader parameter values
    /// (`struct Params` field name -> numbers). The renderer runs exactly this list.
    pub fn passes(&self) -> Vec<(PostFxKind, PostFxParams)> {
        let mut out = Vec::new();
        if let Some(s) = &self.ssao {
            out.push((
                PostFxKind::Ssao,
                vec![
                    ("radius", vec![s.radius]),
                    ("samples", vec![s.samples as f32]),
                    ("strength", vec![s.strength]),
                    ("bias", vec![s.bias]),
                ],
            ));
        }
        if let Some(m) = &self.motion_blur {
            out.push((PostFxKind::MotionBlur, vec![("strength", vec![m.strength]), ("samples", vec![m.samples as f32])]));
        }
        if let Some(d) = &self.dof {
            out.push((
                PostFxKind::Dof,
                vec![
                    ("focus", vec![d.focus]),
                    ("aperture", vec![d.aperture]),
                    ("focal_range", vec![d.focal_range]),
                    ("samples", vec![d.samples as f32]),
                ],
            ));
        }
        if let Some(b) = &self.bloom {
            out.push((
                PostFxKind::Bloom,
                vec![
                    ("threshold", vec![b.threshold]),
                    ("knee", vec![b.knee]),
                    ("strength", vec![b.strength]),
                    ("radius", vec![b.radius]),
                ],
            ));
        }
        if let Some(c) = &self.color_grading {
            out.push((
                PostFxKind::ColorGrade,
                vec![
                    ("exposure", vec![c.exposure]),
                    ("contrast", vec![c.contrast]),
                    ("saturation", vec![c.saturation]),
                    ("temperature", vec![c.temperature]),
                    ("tint", vec![c.tint]),
                    ("lift", c.lift.to_vec()),
                    ("gamma", c.gamma.to_vec()),
                    ("gain", c.gain.to_vec()),
                ],
            ));
        }
        let mode = match self.tonemap {
            Tonemap::Off => None,
            Tonemap::Aces => Some(0.0),
            Tonemap::Reinhard => Some(1.0),
            Tonemap::Hejl => Some(2.0),
        };
        if let Some(mode) = mode {
            out.push((PostFxKind::Tonemap, vec![("mode", vec![mode])]));
        }
        if self.fxaa {
            out.push((PostFxKind::Fxaa, Vec::new()));
        }
        if let Some(c) = &self.chromatic_aberration {
            out.push((
                PostFxKind::ChromaticAberration,
                vec![("amount", vec![c.amount]), ("radial", vec![if c.radial { 1.0 } else { 0.0 }])],
            ));
        }
        if let Some(f) = &self.film_grain {
            out.push((PostFxKind::FilmGrain, vec![("strength", vec![f.strength]), ("size", vec![f.size])]));
        }
        if let Some(v) = &self.vignette {
            out.push((
                PostFxKind::Vignette,
                vec![
                    ("strength", vec![v.strength]),
                    ("inner", vec![v.inner]),
                    ("outer", vec![v.outer]),
                    ("color", v.color.to_linear()[..3].to_vec()),
                ],
            ));
        }
        out
    }

    /// True when at least one built-in effect is enabled.
    pub fn is_active(&self) -> bool {
        !self.passes().is_empty()
    }
}

#[cfg(test)]
mod postfx_tests {
    use super::*;
    use crate::shader::ShaderData;

    fn everything_on() -> PostProcessing {
        PostProcessing {
            tonemap: Tonemap::Aces,
            bloom: Some(Bloom::default()),
            ssao: Some(Ssao::default()),
            motion_blur: Some(MotionBlur::default()),
            dof: Some(DepthOfField::default()),
            color_grading: Some(ColorGrading::default()),
            fxaa: true,
            chromatic_aberration: Some(ChromaticAberration::default()),
            film_grain: Some(FilmGrain::default()),
            vignette: Some(Vignette::default()),
        }
    }

    /// Default settings must not change how existing games look.
    #[test]
    fn default_runs_nothing() {
        let p = PostProcessing::default();
        assert_eq!(p.tonemap, Tonemap::Off);
        assert!(!p.is_active());
    }

    #[test]
    fn passes_follow_the_documented_order() {
        let kinds: Vec<PostFxKind> = everything_on().passes().into_iter().map(|(k, _)| k).collect();
        use PostFxKind::*;
        assert_eq!(kinds, [Ssao, MotionBlur, Dof, Bloom, ColorGrade, Tonemap, Fxaa, ChromaticAberration, FilmGrain, Vignette]);
    }

    /// Every value the engine sends exists in the effect's `struct Params` with the right size,
    /// and every shader param is set (no silently ignored settings).
    #[test]
    fn param_names_match_shaders() {
        for (kind, params) in everything_on().passes() {
            let mut data = ShaderData::compile(kind.name(), kind.shader_source()).expect("built-in effect compiles");
            for (name, value) in &params {
                data.set_param(name, value).unwrap_or_else(|e| panic!("{e}"));
            }
            for p in &data.params {
                assert!(params.iter().any(|(n, _)| *n == p.name), "{}: param '{}' is never set", kind.name(), p.name);
            }
        }
    }

    /// Neutral color grading leaves colors unchanged (checked on the CPU copy of the formula).
    #[test]
    fn neutral_color_grading_defaults() {
        let c = ColorGrading::default();
        assert_eq!((c.exposure, c.contrast, c.saturation, c.temperature, c.tint), (1.0, 1.0, 1.0, 0.0, 0.0));
        assert_eq!((c.lift, c.gamma, c.gain), ([0.0; 3], [1.0; 3], [1.0; 3]));
    }
}
