// Film grain post-effect
// Adds procedural pseudo-random noise to simulate analog film grain.

struct Params {
    strength: f32,  // intensity of the grain (typically 0.01–0.1)
    size: f32,      // grain cell size in pixels (larger = coarser grain)
};

// Integer-free hash producing pseudo-random noise in [0, 1).
fn hash21(uv: vec2f) -> f32 {
    var p = vec2f(dot(uv, vec2f(127.1, 311.7)), dot(uv, vec2f(269.5, 183.3)));
    p = fract(p * 18.5453);
    p = p + dot(p, p.yx + 19.19);
    return fract((p.x + p.y) * p.x);
}

fn post(p: PostInput) -> vec4f {
    let color = input_color(p.uv);

    // Scale UVs by grain cell size and animate over time
    let grain_uv = p.uv * input_size() / max(params.size, 0.001) + vec2f(frame.time.x);
    let h = hash21(grain_uv);

    // Center noise around zero and apply strength
    let noise = (h - 0.5) * params.strength;

    return vec4f(color.rgb + noise, color.a);
}
