struct Params {
    focus: f32,
    aperture: f32,
    focal_range: f32,
    samples: f32,
}

// --- Hash function for jittering the spiral samples ---
fn hash21(p: vec2<f32>) -> vec2<f32> {
    let h = vec2<f32>(
        dot(p, vec2<f32>(127.1, 311.7)),
        dot(p, vec2<f32>(269.5, 183.3)),
    );
    return fract(sin(h) * 43758.5453);
}

// --- Golden spiral (Fibonacci) disk sampling ---
// Returns a 2D point in a unit disk using the golden angle for even distribution.
fn golden_spiral(i: f32, n: f32) -> vec2<f32> {
    let golden_angle = 2.39996323; // pi * (3 - sqrt(5))
    let r = sqrt((i + 0.5) / n);
    let theta = i * golden_angle;
    return vec2<f32>(r * cos(theta), r * sin(theta));
}

// --- Depth of field main entry point ---
fn post(p: PostInput) -> vec4<f32> {
    let uv = p.uv;
    let input = input_color(uv);

    // (a) Get depth at pixel
    let d = linear_depth(uv);

    // (b) Compute Circle of Confusion
    // coc measures how out-of-focus this pixel is, normalized to [0, 1]
    var coc = abs(d - params.focus) / params.focal_range * params.aperture;
    coc = clamp(coc, 0.0, 1.0);

    // (c) In-focus early out — skip expensive blur for sharp pixels
    if (coc < 0.01) {
        return input;
    }

    // (d) Spiral bokeh blur — sample in a golden spiral pattern, weighted by coc
    let sample_count = clamp(params.samples, 1.0, 64.0);
    let n = u32(sample_count);

    // Blur radius in UV space, scaled by coc and output resolution
    let out_size = output_size();
    let max_radius_pixels = 40.0; // max bokeh radius in pixels
    let radius = coc * max_radius_pixels / out_size; // per axis: round bokeh on any aspect

    // Per-pixel jitter to reduce banding artifacts
    let jitter = (hash21(uv * out_size) - 0.5);

    var blurred = vec3<f32>(0.0);
    var total_weight = 0.0;

    for (var i: u32 = 0u; i < n; i = i + 1u) {
        let fi = f32(i);

        // Golden spiral offset in a disk
        let spiral = golden_spiral(fi + jitter.x, sample_count);
        let offset = spiral * radius;
        let sample_uv = uv + offset;

        // Clamp to valid UV range
        let clamped_uv = clamp(sample_uv, vec2<f32>(0.0), vec2<f32>(1.0));

        // Weight: center samples weighted more heavily (Gaussian-ish falloff)
        let dist_from_center = length(spiral);
        let weight = 1.0 - dist_from_center * 0.5;

        blurred = blurred + input_color(clamped_uv).rgb * weight;
        total_weight = total_weight + weight;
    }

    blurred = blurred / max(total_weight, 0.0001);

    // (e) Blend between the original sharp color and the blurred color by coc
    let result = mix(input.rgb, blurred, coc);

    return vec4<f32>(result, input.a);
}
