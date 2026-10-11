// Screen-space ambient occlusion (hemisphere sampling in view space).
//
// Darkens creases and contact areas using only the depth buffer: for every pixel the view-space
// position is rebuilt from depth, a normal is derived from neighbouring depths, and `samples`
// points in the normal-oriented hemisphere of `radius` meters are tested against the depth buffer.
// Runs on the lit image (multiplies it), so it also darkens direct light slightly.

struct Params {
    radius: f32,    // sampling radius in meters
    samples: u32,   // samples per pixel (1..64)
    strength: f32,  // 0 = off, 1 = full occlusion
    bias: f32,      // depth bias in meters against self-occlusion on flat surfaces
};

// View-space position of the surface at uv (camera at origin, looking down -Z).
fn view_pos_at(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, scene_depth(uv), 1.0);
    let w = frame.inv_view_proj * ndc;
    return (frame.view * vec4<f32>(w.xyz / w.w, 1.0)).xyz;
}

fn ssao_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn post(p: PostInput) -> vec4<f32> {
    let input = input_color(p.uv);
    if (scene_depth(p.uv) >= 1.0) {
        return input; // sky / nothing drawn
    }
    let pos = view_pos_at(p.uv);
    let texel = 1.0 / vec2<f32>(textureDimensions(depth_texture));

    // Normal from the neighbour on the side with the smaller depth step (no halos at edges).
    let pr = view_pos_at(p.uv + vec2<f32>(texel.x, 0.0));
    let pl = view_pos_at(p.uv - vec2<f32>(texel.x, 0.0));
    let pd = view_pos_at(p.uv + vec2<f32>(0.0, texel.y));
    let pu = view_pos_at(p.uv - vec2<f32>(0.0, texel.y));
    let dx = select(pr - pos, pos - pl, abs(pl.z - pos.z) < abs(pr.z - pos.z));
    let dy = select(pd - pos, pos - pu, abs(pu.z - pos.z) < abs(pd.z - pos.z));
    var n = normalize(cross(dx, dy));
    if (dot(n, pos) > 0.0) {
        n = -n; // face the camera
    }

    // Per-pixel random rotation of the sample kernel around the normal.
    let angle = ssao_hash(p.position.xy) * 6.28318530718;
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.99);
    let t0 = normalize(cross(up, n));
    let b0 = cross(n, t0);
    let t = t0 * cos(angle) + b0 * sin(angle);
    let b = cross(n, t);

    let count = clamp(params.samples, 1u, 64u);
    var occlusion = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        // Fibonacci hemisphere direction; more samples close to the pixel.
        let fi = f32(i);
        let z = 1.0 - (fi + 0.5) / f32(count);
        let r = sqrt(max(0.0, 1.0 - z * z));
        let phi = fi * 2.39996323;
        let dir = t * (r * cos(phi)) + b * (r * sin(phi)) + n * z;
        let k = (fi + 1.0) / f32(count);
        let sample_pos = pos + dir * params.radius * mix(0.1, 1.0, k * k);

        let clip = frame.proj * vec4<f32>(sample_pos, 1.0);
        let suv = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
        if (any(suv < vec2<f32>(0.0)) || any(suv > vec2<f32>(1.0))) {
            continue;
        }
        let scene_z = view_pos_at(suv).z;
        // Ignore occluders far outside the radius (e.g. a wall far in front of the pixel).
        let range = smoothstep(0.0, 1.0, params.radius / max(abs(pos.z - scene_z), 1e-4));
        occlusion = occlusion + select(0.0, 1.0, scene_z >= sample_pos.z + params.bias) * range;
    }
    let ao = clamp(1.0 - occlusion / f32(count) * params.strength, 0.0, 1.0);
    return vec4<f32>(input.rgb * ao, input.a);
}
