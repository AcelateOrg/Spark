// ─── Sky ───────────────────────────────────────────────────────

// Sky cubemap (a 1x1 white cube when the scene has none) and its trilinear sampler.
@group(0) @binding(3) var spark_sky_cube: texture_cube<f32>;
@group(0) @binding(4) var spark_sky_sampler: sampler;

// Sky color (linear) seen in world direction `dir`: the top / bottom gradient with the cubemap
// blended over it. `blur` 0 = sharp, 1 = the blurriest cubemap mip (rough reflections, ambient).
// Black when the scene has no sky.
fn sky_color(dir: vec3<f32>, blur: f32) -> vec3<f32> {
    let d = normalize(dir);
    var c = mix(frame.sky_bottom.rgb, frame.sky_top.rgb, clamp(d.y * 0.5 + 0.5, 0.0, 1.0));
    if (frame.sky.y > 0.0) {
        let lod = clamp(blur, 0.0, 1.0) * max(frame.sky.w - 1.0, 0.0);
        c = mix(c, textureSampleLevel(spark_sky_cube, spark_sky_sampler, d, lod).rgb, frame.sky.y);
    }
    return c * frame.sky.x;
}

// Ambient (indirect) light arriving at a surface with normal `n`: the sky when sky lighting is on,
// otherwise the scene's flat ambient color.
fn ambient_light(n: vec3<f32>) -> vec3<f32> {
    if (frame.sky.z > 0.0) {
        return sky_color(n, 1.0) * frame.sky.z;
    }
    return frame.ambient.rgb;
}

// Light reflected towards the camera by a mirror-like surface (`r` = reflection direction).
fn ambient_reflection(r: vec3<f32>, roughness: f32) -> vec3<f32> {
    if (frame.sky.z > 0.0) {
        return sky_color(r, sqrt(clamp(roughness, 0.0, 1.0))) * frame.sky.z;
    }
    return frame.ambient.rgb;
}
