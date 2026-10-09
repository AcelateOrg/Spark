
struct PassInfo {
    input_size: vec4<f32>,   // xy = input image size in pixels
    output_size: vec4<f32>,  // xy = this pass's output size in pixels
};

@group(0) @binding(1) var input_texture: texture_2d<f32>;
@group(0) @binding(2) var input_sampler: sampler;
@group(0) @binding(3) var depth_texture: texture_depth_2d;
@group(0) @binding(4) var<uniform> pass_info: PassInfo;
@group(1) @binding(0) var<uniform> params: Params;
@group(1) @binding(1) var texture1: texture_2d<f32>;
@group(1) @binding(2) var texture2: texture_2d<f32>;
@group(1) @binding(3) var linear_sampler: sampler;
@group(1) @binding(4) var nearest_sampler: sampler;

struct PostInput {
    @builtin(position) position: vec4<f32>,  // output pixel position
    @location(0) uv: vec2<f32>,              // 0..1, top-left = (0, 0)
};

fn input_size() -> vec2<f32> {
    return pass_info.input_size.xy;
}

fn output_size() -> vec2<f32> {
    return pass_info.output_size.xy;
}

// The image so far (3D scene + 2D scene layer + earlier passes), linear rgb. Uses graphics.upscale filtering.
fn input_color(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(input_texture, input_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
}

// Raw depth buffer value at uv (0 = near plane, 1 = far plane / sky).
fn scene_depth(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth_texture));
    let p = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(depth_texture, p, 0);
}

// Distance from the camera plane in meters at uv.
fn linear_depth(uv: vec2<f32>) -> f32 {
    let near = frame.camera.x;
    let far = frame.camera.y;
    if (frame.proj[3][3] > 0.5) {  // orthographic camera: depth is linear
        return near + scene_depth(uv) * (far - near);
    }
    return far * near / (far - scene_depth(uv) * (far - near));
}

@vertex
fn spark_vs(@builtin(vertex_index) i: u32) -> PostInput {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: PostInput;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn spark_fs(p: PostInput) -> @location(0) vec4<f32> {
    return post(p);
}
