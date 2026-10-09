// Retro look: fewer colors + ordered (Bayer) dithering. Run it at the scene resolution:
//   graphics.post = { { shader = pixel, size = "scene" } }   with graphics.height = 180
struct Params {
    levels: f32,  // color levels per channel, e.g. 16
}

fn post(p: PostInput) -> vec4<f32> {
    var bayer = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    let px = vec2<u32>(p.position.xy);
    let offset = bayer[(px.y % 4u) * 4u + (px.x % 4u)] / 16.0 - 0.5;
    let steps = max(params.levels, 2.0) - 1.0;
    let c = linear_to_srgb(input_color(p.uv).rgb);
    let q = floor(c * steps + vec3<f32>(0.5 + offset)) / steps;
    return vec4<f32>(srgb_to_linear(clamp(q, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}
