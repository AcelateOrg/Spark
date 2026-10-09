
struct Object {
    model: mat4x4<f32>,
    normal_matrix: mat4x4<f32>,
    color: vec4<f32>,  // material color, linear
    data: vec4<f32>,   // material.data: free for your shader
    info: vec4<f32>,   // tiling x, tiling y, unlit (0 / 1), -
};

// Per-object data of every drawn object (instanced draws). `object` = the one being drawn.
@group(1) @binding(0) var<uniform> spark_objects: array<Object, 64>;
var<private> object: Object;
// Sun shadow map (see `sun_shadow`).
@group(0) @binding(1) var spark_shadow_map: texture_depth_2d;
@group(0) @binding(2) var spark_shadow_sampler: sampler_comparison;
@group(2) @binding(0) var base_texture: texture_2d<f32>;
@group(2) @binding(1) var base_sampler: sampler;
@group(3) @binding(0) var<uniform> params: Params;
@group(3) @binding(1) var texture1: texture_2d<f32>;
@group(3) @binding(2) var texture2: texture_2d<f32>;
@group(3) @binding(3) var linear_sampler: sampler;
@group(3) @binding(4) var nearest_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

// Vertex shader output = fragment shader input.
struct Fragment {
    @builtin(position) clip: vec4<f32>,   // vertex: clip position; fragment: pixel position
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,       // world space, not normalized
    @location(2) uv: vec2<f32>,           // already multiplied by the material tiling
    @location(3) custom: vec4<f32>,       // free for your shader (0 by default)
    @location(4) @interpolate(flat) spark_instance: u32,  // engine-managed, leave as is
};

fn default_vertex(v: VertexInput) -> Fragment {
    var f: Fragment;
    let world = object.model * vec4<f32>(v.position, 1.0);
    f.clip = frame.view_proj * world;
    f.world_pos = world.xyz;
    f.normal = (object.normal_matrix * vec4<f32>(v.normal, 0.0)).xyz;
    f.uv = v.uv * object.info.xy;
    f.custom = vec4<f32>(0.0);
    return f;
}

// Material color * texture.
fn surface_color(uv: vec2<f32>) -> vec4<f32> {
    return object.color * textureSample(base_texture, base_sampler, uv);
}

// How much sun reaches a point: 1 = lit, 0 = in shadow (3x3 PCF, fades out at the shadow distance).
fn sun_shadow(world_pos: vec3<f32>, normal: vec3<f32>) -> f32 {
    if (frame.shadow.x < 0.5) {
        return 1.0;
    }
    let n = normalize(normal);
    let p = frame.shadow_view_proj * vec4<f32>(world_pos + n * frame.shadow.z, 1.0);
    let c = p.xyz / p.w;
    let uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    if (uv.x <= 0.0 || uv.y <= 0.0 || uv.x >= 1.0 || uv.y >= 1.0 || c.z >= 1.0) {
        return 1.0;
    }
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y)) * frame.shadow.y;
            sum = sum + textureSampleCompareLevel(spark_shadow_map, spark_shadow_sampler, uv + o, c.z - 0.0003);
        }
    }
    let fade = smoothstep(frame.shadow.w * 0.8, frame.shadow.w, distance(world_pos, frame.camera_pos.xyz));
    return mix(sum / 9.0, 1.0, fade);
}

// Light arriving at a point: ambient + sun (with shadows) + point / spot lights (linear rgb).
fn lighting(world_pos: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let n = normalize(normal);
    let sun = max(dot(n, normalize(-frame.sun_dir.xyz)), 0.0);
    var light = frame.ambient.rgb;
    if (sun > 0.0) {
        light = light + frame.sun_color.rgb * sun * sun_shadow(world_pos, n);
    }
    for (var i = 0u; i < frame.light_count.x; i = i + 1u) {
        let li = frame.lights[i];
        let to_light = li.position.xyz - world_pos;
        let dist = length(to_light);
        let range = li.position.w;
        if (dist >= range) {
            continue;
        }
        let ldir = to_light / max(dist, 0.0001);
        let x = dist / range;
        let window = clamp(1.0 - x * x, 0.0, 1.0);
        var att = window * window / (1.0 + 4.0 * x * x);
        if (li.color.w > 0.5) {
            att = att * smoothstep(li.direction.w, li.params.x, dot(-ldir, li.direction.xyz));
        }
        let lambert = clamp((dot(n, ldir) + 0.15) / 1.15, 0.0, 1.0);
        light = light + li.color.rgb * att * lambert;
    }
    return light;
}

fn apply_fog(rgb: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    if (frame.fog.z < 0.5) {
        return rgb;
    }
    let d = distance(world_pos, frame.camera_pos.xyz);
    return mix(rgb, frame.fog_color.rgb, clamp((d - frame.fog.x) / max(frame.fog.y - frame.fog.x, 0.001), 0.0, 1.0));
}

fn default_fragment(f: Fragment) -> vec4<f32> {
    let base = surface_color(f.uv);
    if (base.a < 0.01) {
        discard;
    }
    var rgb = base.rgb;
    if (object.info.z < 0.5) {
        rgb = rgb * lighting(f.world_pos, f.normal);
    }
    return vec4<f32>(apply_fog(rgb, f.world_pos), base.a);
}

@vertex
fn spark_vs(v: VertexInput, @builtin(instance_index) spark_ii: u32) -> Fragment {
    object = spark_objects[spark_ii];
    var f = SPARK_VERTEX(v);
    f.spark_instance = spark_ii;
    return f;
}

@fragment
fn spark_fs(f: Fragment) -> @location(0) vec4<f32> {
    object = spark_objects[f.spark_instance];
    return SPARK_FRAGMENT(f);
}
