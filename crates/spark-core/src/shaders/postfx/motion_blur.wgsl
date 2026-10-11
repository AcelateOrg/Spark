// Camera motion blur.
//
// Rebuilds each pixel's world position from depth, projects it with last frame's camera
// (`frame.prev_view_proj`) and blurs along the screen-space motion. Moving or turning the camera
// blurs, a still camera gives a sharp image (objects moving in front of a still camera are not
// blurred: there is no per-object velocity buffer).

struct Params {
    strength: f32,  // 1 = blur over the whole frame time (360° shutter), 0.5 = half
    samples: u32,   // samples along the motion (1..32)
};

const MAX_BLUR_PIXELS: f32 = 32.0;

fn post(p: PostInput) -> vec4<f32> {
    let input = input_color(p.uv);
    let ndc = vec4<f32>(p.uv.x * 2.0 - 1.0, 1.0 - p.uv.y * 2.0, scene_depth(p.uv), 1.0);
    let world = frame.inv_view_proj * ndc;
    let prev = frame.prev_view_proj * vec4<f32>(world.xyz / world.w, 1.0);
    if (prev.w <= 0.0) {
        return input; // behind last frame's camera
    }
    let prev_uv = vec2<f32>(prev.x / prev.w * 0.5 + 0.5, 0.5 - prev.y / prev.w * 0.5);

    let size = input_size();
    var velocity = (p.uv - prev_uv) * params.strength;
    let pixels = length(velocity * size);
    if (pixels < 0.5) {
        return input;
    }
    if (pixels > MAX_BLUR_PIXELS) {
        velocity = velocity * (MAX_BLUR_PIXELS / pixels); // camera cuts / teleports
    }

    let count = clamp(params.samples, 1u, 32u);
    var color = vec3<f32>(0.0);
    for (var i = 0u; i < count; i = i + 1u) {
        let t = (f32(i) + 0.5) / f32(count) - 0.5; // -0.5 .. 0.5 around the pixel
        color = color + input_color(p.uv - velocity * t).rgb;
    }
    return vec4<f32>(color / f32(count), input.a);
}
