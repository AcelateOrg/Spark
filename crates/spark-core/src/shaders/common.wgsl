
// ================= Spark prelude (added by the engine after your code) =================
// Everything below is available in every Spark shader. See docs/SHADERS.md.

struct Light {
    position: vec4<f32>,   // xyz = world position, w = range
    color: vec4<f32>,      // rgb = linear color * intensity, w = 1 for spot lights
    direction: vec4<f32>,  // xyz = direction, w = cos(outer half angle)
    params: vec4<f32>,     // x = cos(inner half angle)
};

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,  // xyz
    camera: vec4<f32>,      // near, far, vertical fov (radians), aspect
    screen: vec4<f32>,      // scene width, scene height, output width, output height (pixels)
    time: vec4<f32>,        // seconds (game time), dt, seconds (real time, ignores time.scale), frame number
    sun_dir: vec4<f32>,     // xyz = direction the light travels
    sun_color: vec4<f32>,   // rgb = linear color * intensity
    ambient: vec4<f32>,     // rgb, linear
    fog_color: vec4<f32>,   // rgb, linear
    fog: vec4<f32>,         // near, far, enabled (0 / 1)
    light_count: vec4<u32>, // x = number of entries in `lights`
    lights: array<Light, 32>,
    shadow_view_proj: mat4x4<f32>, // world -> sun shadow map clip space
    shadow: vec4<f32>,      // enabled (0 / 1), 1 / map size, normal offset (m), shadow distance (m)
};

@group(0) @binding(0) var<uniform> frame: Frame;

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}
