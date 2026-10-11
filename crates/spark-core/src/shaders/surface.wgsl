
struct Object {
    model: mat4x4<f32>,
    normal_matrix: mat4x4<f32>,
    color: vec4<f32>,  // material color, linear
    data: vec4<f32>,   // material.data: free for your shader
    info: vec4<f32>,   // tiling x, tiling y, unlit (0 / 1), pbr (0 / 1)
    pbr: vec4<f32>,    // metallic, roughness, has normal map (0 / 1), -
    emissive: vec4<f32>, // rgb, linear
};

// Per-object data of every drawn object (instanced draws). `object` = the one being drawn.
@group(1) @binding(0) var<uniform> spark_objects: array<Object, 64>;
var<private> object: Object;
// Sun shadow map (see `sun_shadow`).
@group(0) @binding(1) var spark_shadow_map: texture_depth_2d;
@group(0) @binding(2) var spark_shadow_sampler: sampler_comparison;
@group(2) @binding(0) var base_texture: texture_2d<f32>;
@group(2) @binding(1) var base_sampler: sampler;
// Material normal map (a flat 1x1 map when the material has none). See `surface_normal`.
@group(2) @binding(2) var normal_texture: texture_2d<f32>;
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
    var light = ambient_light(n);
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

// ─── Normal maps ───────────────────────────────────────────────

// Shading normal: the geometric normal tilted by the material's tangent-space normal map
// (OpenGL convention, green = +V). Tangents come from screen-space derivatives (no tangent
// attribute needed); materials without a normal map sample a flat 1x1 map, so the result is the
// geometric normal. Call it in uniform control flow (it uses derivatives).
fn surface_normal(world_pos: vec3<f32>, normal: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let n = normalize(normal);
    // Normal maps hold data, not colors: undo the sRGB decode of the color texture format.
    let tn = linear_to_srgb(textureSample(normal_texture, base_sampler, uv).rgb) * 2.0 - 1.0;
    let dp1 = dpdx(world_pos);
    let dp2 = dpdy(world_pos);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let scale = max(dot(t, t), dot(b, b));
    let inv = inverseSqrt(max(scale, 1e-30));
    let mapped = normalize(t * inv * tn.x + b * inv * tn.y + n * tn.z);
    // No normal map, or degenerate UVs (no tangent frame): keep the geometric normal.
    return select(mapped, n, object.pbr.z < 0.5 || scale < 1e-20);
}

// ─── PBR (metallic / roughness, Cook-Torrance GGX) ─────────────

const PI: f32 = 3.14159265359;

// Split-sum environment BRDF, analytic fit (Karis, "Physically Based Shading on Mobile").
fn env_brdf_approx(roughness: f32, n_dot_v: f32) -> vec2<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    return vec2<f32>(-1.04, 1.04) * a004 + r.zw;
}

// Reflected light for one light direction `l` with irradiance `radiance` (light color * attenuation).
// Light colors use the same convention as the classic shader: a white light at normal incidence
// lights a white diffuse surface to its full color (the BRDF is scaled by PI).
fn pbr_direct(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>, base: vec3<f32>, metallic: f32, roughness: f32) -> vec3<f32> {
    let n_dot_l = max(dot(n, l), 0.0);
    if (n_dot_l <= 0.0) {
        return vec3<f32>(0.0);
    }
    let n_dot_v = max(dot(n, v), 1e-4);
    let h = normalize(v + l);
    let n_dot_h = max(dot(n, h), 0.0);
    let alpha = roughness * roughness;
    let a2 = alpha * alpha;
    let d_denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * d_denom * d_denom);
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let g = (n_dot_v / (n_dot_v * (1.0 - k) + k)) * (n_dot_l / (n_dot_l * (1.0 - k) + k));
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let f = f0 + (1.0 - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);
    let specular = d * g * f / (4.0 * n_dot_v * n_dot_l);
    let diffuse = (1.0 - f) * (1.0 - metallic) * base;
    return (diffuse + specular * PI) * radiance * n_dot_l;
}

// Ambient light on a PBR surface: diffuse plus the environment reflection (the sky with sky
// lighting on, else the ambient color), so metals are not black without lights.
fn pbr_ambient(n: vec3<f32>, v: vec3<f32>, base: vec3<f32>, metallic: f32, roughness: f32) -> vec3<f32> {
    let n_dot_v = max(dot(n, v), 1e-4);
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let env = env_brdf_approx(roughness, n_dot_v);
    let specular = f0 * env.x + env.y;
    let diffuse = ambient_light(n) * (1.0 - specular) * (1.0 - metallic) * base;
    return diffuse + ambient_reflection(reflect(-v, n), roughness) * specular;
}

// Full PBR lighting of a surface point: ambient + sun (with shadows) + point / spot lights.
fn pbr_lighting(world_pos: vec3<f32>, normal: vec3<f32>, base: vec3<f32>, metallic: f32, roughness: f32) -> vec3<f32> {
    let n = normalize(normal);
    let v = normalize(frame.camera_pos.xyz - world_pos);
    let r = clamp(roughness, 0.045, 1.0); // a perfect mirror has no highlight for point lights
    let m = clamp(metallic, 0.0, 1.0);
    var rgb = pbr_ambient(n, v, base, m, r);
    let sun_l = normalize(-frame.sun_dir.xyz);
    if (dot(n, sun_l) > 0.0) {
        rgb = rgb + pbr_direct(n, v, sun_l, frame.sun_color.rgb * sun_shadow(world_pos, n), base, m, r);
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
        rgb = rgb + pbr_direct(n, v, ldir, li.color.rgb * att, base, m, r);
    }
    return rgb;
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
    let n = surface_normal(f.world_pos, f.normal, f.uv);
    var rgb = base.rgb;
    if (object.info.z < 0.5) {
        if (object.info.w > 0.5) {
            rgb = pbr_lighting(f.world_pos, n, base.rgb, object.pbr.x, object.pbr.y);
        } else {
            rgb = rgb * lighting(f.world_pos, n);
        }
    }
    rgb = rgb + object.emissive.rgb;
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
