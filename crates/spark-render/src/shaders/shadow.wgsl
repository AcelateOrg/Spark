// Sun shadow map: depth only. Reads the same per-object uniform windows as the scene pass.
struct Object {
    model: mat4x4<f32>,
    normal_matrix: mat4x4<f32>,
    color: vec4<f32>,
    data: vec4<f32>,
    info: vec4<f32>,
};

@group(0) @binding(0) var<uniform> light_view_proj: mat4x4<f32>;
@group(1) @binding(0) var<uniform> objects: array<Object, 64>;

@vertex
fn vs_main(@location(0) position: vec3<f32>, @builtin(instance_index) i: u32) -> @builtin(position) vec4<f32> {
    return light_view_proj * objects[i].model * vec4<f32>(position, 1.0);
}
