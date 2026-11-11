// ─────────────────────────────────────────────────────────────────────────
// bloom.wgsl — Single-pass bloom approximation
//
// Pipeline: engine appends common.wgsl + post.wgsl before this file.
// Exposed helpers from prelude:
//   input_color(uv) -> vec4<f32>
//   scene_depth(uv) -> f32
//   linear_depth(uv) -> f32
//   input_size() -> vec2<f32>
//   output_size() -> vec2<f32>
//   linear_to_srgb(c), srgb_to_linear(c)
//   frame  (uniform: frame.time.w, frame.time, etc.)
//   PostInput { uv: vec2<f32> }
//
// Entry point: fn post(p: PostInput) -> vec4<f32>
// ─────────────────────────────────────────────────────────────────────────

struct Params {
    // Luminance threshold above which a pixel contributes to bloom.
    threshold: f32,
    // Soft knee width — the transition zone where a pixel ramps from
    // no contribution to full contribution. Smaller = sharper cutoff.
    knee: f32,
    // Blended bloom intensity multiplied into the final composite.
    strength: f32,
    // Blur radius in source pixels along the X axis. The kernel is
    // scaled uniformly by radius / input_size().x so the visual spread
    // is resolution-independent.
    radius: f32,
}

// ── Hash / noise ──────────────────────────────────────────────────────────
// Cheap hash -> [0, 1). Used to jitter spiral sample positions and break
// the deterministic look of a fixed kernel.

fn hash12(p: vec2<f32>) -> f32 {
    var v = p;
    v = fract(v * vec2<f32>(123.34, 345.45));
    v += dot(v, v + 34.345);
    return fract(v.x * v.y);
}

// ── Bright-pass extraction ────────────────────────────────────────────────
// Returns only the portion of the color that exceeds the threshold,
// with a smooth soft-knee ramp to avoid harsh banding.

fn bright_pass(c: vec3<f32>, threshold: f32, knee: f32) -> vec3<f32> {
    let luma = max(max(c.r, c.g), c.b);
    let soft = smoothstep(threshold, threshold + knee, luma);
    return c * soft;
}

// ── post ──────────────────────────────────────────────────────────────────
fn post(p: PostInput) -> vec4<f32> {
    let src = input_color(p.uv);
    let isz = input_size();
    let texel = vec2<f32>(1.0) / isz;

    // (a) Input color — work in linear space for physically correct add.
    let color = src.rgb;

    // (b) Bright pass with soft knee.
    let bright = bright_pass(color, params.threshold, params.knee);

    // (c) Multi-tap Gaussian blur on bright areas.
    //
    // We use 13 taps arranged in a spiral pattern. Each tap is jittered
    // by a hash to reduce visible banding / pattern artifacts. The
    // overall scale is driven by radius / input_size().x so the bloom
    // spread is consistent across render resolutions.
    //
    // Gaussian weights are precomputed from a radial falloff and
    // applied per-tap — cheap but visually pleasing for a single pass.

    let scale = vec2<f32>(params.radius) / isz; // pixels -> uv per axis (round on any aspect)
    var bloom = vec3<f32>(0.0);
    var weight_sum = 0.0;

    // 13 sample offsets in a spiral, normalised direction + distance.
    // Golden-angle stepping for good coverage of the disc.
    let tap_count = 13u;
    let golden_angle = 2.39996323;

    for (var i = 0u; i < tap_count; i = i + 1u) {
        // Angle and normalised radial distance for this tap.
        let angle = f32(i) * golden_angle;
        let dist_norm = (f32(i) + 0.5) / f32(tap_count);
        // Quadratic falloff so closer taps cluster near the center.
        let dist = dist_norm * dist_norm;

        // Jitter to break patterns — per-pixel hash seeded by tap index
        // and frame counter for temporal variation.
        let jitter = hash12(p.uv * isz + vec2<f32>(f32(i), frame.time.w)) * 0.5;
        let r = (dist + jitter * 0.15) * scale;

        let offset = vec2<f32>(cos(angle), sin(angle)) * r;
        let suv = p.uv + offset;

        // Gaussian weight based on radial distance.
        let w = exp(-(dist * dist) * 4.0);

        let sample_bright = bright_pass(input_color(suv).rgb, params.threshold, params.knee);
        bloom = bloom + sample_bright * w;
        weight_sum = weight_sum + w;
    }

    // Normalise the accumulated bloom.
    bloom = bloom / max(weight_sum, 1e-5);

    // (d) Composite: original + bloom * strength.
    let result = color + bloom * params.strength;

    return vec4<f32>(result, src.a);
}
