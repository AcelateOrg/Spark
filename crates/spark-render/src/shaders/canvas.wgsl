// Spark 2D canvas: textured, vertex-colored triangles in virtual pixels (origin top-left, Y down).

struct Canvas {
    scale_offset: vec4<f32>,  // virtual px -> clip space: pos * xy + zw
};

@group(0) @binding(0) var<uniform> canvas: Canvas;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(v.pos * canvas.scale_offset.xy + canvas.scale_offset.zw, 0.0, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, tex_sampler, in.uv) * in.color;
}
