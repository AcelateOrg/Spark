
// ─── Skybox pass ───────────────────────────────────────────────
// Full-screen triangle drawn first in the scene pass (no depth write): every pixel not covered by
// an object shows the sky in its view direction.

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn sky_vs(@builtin(vertex_index) i: u32) -> SkyOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    let ndc = uv * 2.0 - 1.0;
    var o: SkyOut;
    o.clip = vec4<f32>(ndc, 1.0, 1.0);
    o.ndc = ndc;
    return o;
}

@fragment
fn sky_fs(o: SkyOut) -> @location(0) vec4<f32> {
    let far = frame.inv_view_proj * vec4<f32>(o.ndc, 1.0, 1.0);
    let near = frame.inv_view_proj * vec4<f32>(o.ndc, 0.0, 1.0);
    let dir = far.xyz / far.w - near.xyz / near.w;
    return vec4<f32>(sky_color(dir, 0.0), 1.0);
}
