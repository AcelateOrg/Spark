// Toon shading: the light falls into a few hard bands. Used as graphics.shader (every material).
struct Params {
    bands: f32,
}

fn fragment(f: Fragment) -> vec4<f32> {
    let base = surface_color(f.uv);
    if (base.a < 0.01) {
        discard;
    }
    if (object.info.z > 0.5) {  // unlit material
        return base;
    }
    let light = lighting(f.world_pos, f.normal);
    let l = max(max(light.r, light.g), light.b);
    let bands = max(params.bands, 1.0);
    let stepped = ceil(l * bands) / bands;
    let rgb = base.rgb * light / max(l, 0.001) * stepped;
    return vec4<f32>(apply_fog(rgb, f.world_pos), base.a);
}
