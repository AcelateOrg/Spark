// Ink outlines where the depth jumps (uses the depth buffer).
struct Params {
    color: vec4<f32>,  // rgb + strength
    thickness: f32,    // pixels
}

fn post(p: PostInput) -> vec4<f32> {
    let c = input_color(p.uv);
    let px = max(params.thickness, 1.0) / output_size();
    let d = linear_depth(p.uv);
    let dx = abs(linear_depth(p.uv + vec2<f32>(px.x, 0.0)) - linear_depth(p.uv - vec2<f32>(px.x, 0.0)));
    let dy = abs(linear_depth(p.uv + vec2<f32>(0.0, px.y)) - linear_depth(p.uv - vec2<f32>(0.0, px.y)));
    let edge = step(0.06 * d, max(dx, dy));
    return vec4<f32>(mix(c.rgb, params.color.rgb, edge * params.color.a), 1.0);
}
