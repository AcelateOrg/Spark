// Chromatic aberration post-effect
// Offsets RGB channels to simulate lens color fringing.
// radial 0 = directional (horizontal shift), 1 = radial (increases toward edges).

struct Params {
    amount: f32,  // aberration magnitude in pixels
    radial: u32,  // 0 = directional, 1 = radial
};

fn post(p: PostInput) -> vec4f {
    let texel = 1.0 / input_size();

    var offset: vec2f;
    if params.radial == 1u {
        // Radial: offset direction points away from center, magnitude
        // scales with distance from center.
        let dir = p.uv - 0.5;
        offset = dir * 2.0 * params.amount * texel; // `amount` pixels at the screen edge
    } else {
        // Directional: uniform horizontal shift.
        offset = vec2f(params.amount * texel.x, 0.0);
    }

    // Sample each channel at a different UV offset
    let r = input_color(p.uv - offset).r;
    let g = input_color(p.uv).g;
    let b = input_color(p.uv + offset).b;

    return vec4f(r, g, b, input_color(p.uv).a);
}
