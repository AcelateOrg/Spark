// Tonemapping post-effect
// Applies one of three tonemapping operators to the HDR input color.
// mode 0 = ACES filmic, 1 = Reinhard, 2 = Hejl.

struct Params {
    mode: u32,
};

// ACES filmic tonemap approximation.
fn tonemap_aces(c: vec3f) -> vec3f {
    let num = c * (2.51 * c + 0.03);
    let den = c * (2.43 * c + 0.59) + 0.14;
    return clamp(num / den, vec3f(0.0), vec3f(1.0));
}

// Reinhard tonemap — simple, tends to desaturate highlights.
fn tonemap_reinhard(c: vec3f) -> vec3f {
    return c / (1.0 + c);
}

// Hejl 2015 tonemap — preserves saturation well in highlights.
fn tonemap_hejl(c: vec3f) -> vec3f {
    let x = max(vec3f(0.0), c - 0.004);
    return (x * (6.2 * x + 0.5)) / (x * (6.2 * x + 1.7) + 0.06);
}

fn post(p: PostInput) -> vec4f {
    let color = input_color(p.uv);
    var mapped: vec3f;

    switch params.mode {
        case 0u: { // ACES
            mapped = tonemap_aces(color.rgb);
        }
        case 1u: { // Reinhard
            mapped = tonemap_reinhard(color.rgb);
        }
        case 2u: { // Hejl
            mapped = tonemap_hejl(color.rgb);
        }
        default: {
            mapped = tonemap_aces(color.rgb);
        }
    }

    return vec4f(mapped, color.a);
}
