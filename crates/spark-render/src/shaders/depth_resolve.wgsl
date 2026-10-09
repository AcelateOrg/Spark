// Copies sample 0 of the multisampled scene depth into the single-sample depth read by post shaders.
@group(0) @binding(0) var depth_ms: texture_depth_multisampled_2d;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    return textureLoad(depth_ms, vec2<i32>(p.xy), 0);
}
