// Vignette post-effect
// Darkens edges by fading toward a color based on distance from center.

struct Params {
    strength: f32,  // how strong the darkening is (0 = none, 1 = full)
    inner: f32,     // inner radius where falloff begins
    outer: f32,     // outer radius where falloff reaches max
    color: vec3f,   // color to fade toward at edges (usually black)
};

fn post(p: PostInput) -> vec4f {
    let color = input_color(p.uv);

    // Distance from screen center, scaled to [0, ~1.41]
    let d = length(p.uv - 0.5) * 2.0;

    // Vignette factor: 1 at center, fades to (1 - strength) at edges
    let v = 1.0 - smoothstep(params.inner, params.outer, d) * params.strength;

    // Blend from vignette color to input color by factor v
    let result = mix(params.color, color.rgb, v);

    return vec4f(result, color.a);
}
