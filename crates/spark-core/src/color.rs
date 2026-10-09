/// RGBA color in sRGB space, components 0..1 (like CSS / three.js colors).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for Color {
    fn default() -> Self {
        Color::WHITE
    }
}

impl Color {
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    pub const GRAY: Color = Color::rgb(0.5, 0.5, 0.5);
    pub const RED: Color = Color::rgb(0.9, 0.2, 0.2);
    pub const GREEN: Color = Color::rgb(0.3, 0.8, 0.3);
    pub const BLUE: Color = Color::rgb(0.25, 0.45, 0.95);
    pub const YELLOW: Color = Color::rgb(1.0, 0.85, 0.2);
    pub const ORANGE: Color = Color::rgb(1.0, 0.55, 0.15);
    pub const PURPLE: Color = Color::rgb(0.6, 0.35, 0.9);
    pub const PINK: Color = Color::rgb(1.0, 0.5, 0.75);
    pub const BROWN: Color = Color::rgb(0.55, 0.36, 0.2);
    pub const SKY: Color = Color::rgb(0.53, 0.75, 0.95);

    /// Named colors available to scripts.
    pub const NAMED: &'static [(&'static str, Color)] = &[
        ("white", Color::WHITE),
        ("black", Color::BLACK),
        ("gray", Color::GRAY),
        ("grey", Color::GRAY),
        ("red", Color::RED),
        ("green", Color::GREEN),
        ("blue", Color::BLUE),
        ("yellow", Color::YELLOW),
        ("orange", Color::ORANGE),
        ("purple", Color::PURPLE),
        ("pink", Color::PINK),
        ("brown", Color::BROWN),
        ("sky", Color::SKY),
    ];

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// `Color::hex(0xff8800)`
    pub const fn hex(value: u32) -> Self {
        Self::rgb(
            ((value >> 16) & 0xff) as f32 / 255.0,
            ((value >> 8) & 0xff) as f32 / 255.0,
            (value & 0xff) as f32 / 255.0,
        )
    }

    /// Parses `"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"` or a color name like `"red"`.
    pub fn parse(text: &str) -> Option<Self> {
        let t = text.trim();
        if let Some(hex) = t.strip_prefix('#') {
            let v = u32::from_str_radix(hex, 16).ok()?;
            return match hex.len() {
                3 => {
                    let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
                    Some(Self::hex((r * 17) << 16 | (g * 17) << 8 | (b * 17)))
                }
                6 => Some(Self::hex(v)),
                8 => Some(Self::hex(v >> 8).with_alpha((v & 0xff) as f32 / 255.0)),
                _ => None,
            };
        }
        let lower = t.to_ascii_lowercase();
        Self::NAMED.iter().find(|(n, _)| *n == lower).map(|(_, c)| *c)
    }

    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    pub fn lerp(self, other: Color, t: f32) -> Self {
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    /// Linear-space RGBA, for shaders.
    pub fn to_linear(self) -> [f32; 4] {
        [srgb_to_linear(self.r), srgb_to_linear(self.g), srgb_to_linear(self.b), self.a]
    }

    /// sRGB bytes.
    pub fn to_rgba8(self) -> [u8; 4] {
        let f = |c: f32| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        [f(self.r), f(self.g), f(self.b), f(self.a)]
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
