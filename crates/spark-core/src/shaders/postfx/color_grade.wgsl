// ─────────────────────────────────────────────────────────────────────────
// color_grade.wgsl — Color grading (exposure, CDL, contrast, saturation,
//                    temperature, tint)
//
// Pipeline: engine appends common.wgsl + post.wgsl before this file.
// Exposed helpers from prelude:
//   input_color(uv) -> vec4<f32>
//   scene_depth(uv) -> f32
//   linear_depth(uv) -> f32
//   input_size() -> vec2<f32>
//   output_size() -> vec2<f32>
//   linear_to_srgb(c), srgb_to_linear(c)
//   frame  (uniform)
//   PostInput { uv: vec2<f32> }
//
// Entry point: fn post(p: PostInput) -> vec4<f32>
// ─────────────────────────────────────────────────────────────────────────

struct Params {
    // Multiplicative exposure (1.0 = neutral).
    exposure: f32,
    // Contrast multiplier around a 0.5 pivot (1.0 = neutral).
    contrast: f32,
    // Saturation multiplier (1.0 = neutral, 0 = grayscale).
    saturation: f32,
    // Temperature shift: positive = warm (R up, B down), negative = cool.
    temperature: f32,
    // Tint shift: positive = green push, negative = magenta push.
    tint: f32,
    // ASC CDL offset ("lift") — added per channel, 0 = neutral.
    lift: vec3<f32>,
    // ASC CDL gamma — power per channel (must be > 0).
    gamma: vec3<f32>,
    // ASC CDL gain  — multiplicative per channel.
    gain: vec3<f32>,
}

// ── post ──────────────────────────────────────────────────────────────────
fn post(p: PostInput) -> vec4<f32> {
    let src = input_color(p.uv);
    var c = src.rgb;

    // ── (a) Exposure ──────────────────────────────────────────────────────
    // Simple linear multiply. Because we work in linear RGB this is
    // physically correct: doubling exposure ≈ +1 stop.
    c = c * params.exposure;

    // ── (b) Lift / Gamma / Gain (ASC CDL) ──────────────────────────────────
    // ASC CDL:  out = pow(max(in * gain + lift, 0), 1 / gamma)
    c = c * params.gain + params.lift;

    // Clamp negatives before the power to avoid NaN.
    c = max(c, vec3<f32>(0.0));

    // Per-channel gamma. Guard against division issues.
    let inv_gamma = vec3<f32>(1.0) / max(params.gamma, vec3<f32>(1e-5));
    c = pow(c, inv_gamma);

    // ── (c) Contrast around 0.5 pivot ─────────────────────────────────────
    c = (c - vec3<f32>(0.5)) * params.contrast + vec3<f32>(0.5);

    // ── (d) Saturation ────────────────────────────────────────────────────
    // Rec.709 luma weights. mix(luma, color, saturation) lets
    // saturation < 1 desaturate and > 1 oversaturate.
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = mix(vec3<f32>(luma), c, params.saturation);

    // ── (e) Temperature ───────────────────────────────────────────────────
    // Warm: shift red up, blue down. Cool: the reverse. Green is
    // left untouched so temperature stays on the blue↔amber axis.
    let temp = params.temperature;
    c.r = c.r + temp;
    c.b = c.b - temp;

    // ── (f) Tint ───────────────────────────────────────────────────────────
    // Tint operates on the green ↔ magenta axis. Positive tint
    // pushes green up and (R+B) down; negative pushes magenta.
    let tnt = params.tint;
    c.g = c.g + tnt;
    c.r = c.r - tnt * 0.5;
    c.b = c.b - tnt * 0.5;

    // Clamp to valid range to avoid clamping artefacts downstream.
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(65504.0));

    return vec4<f32>(c, src.a);
}
