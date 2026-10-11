// FXAA (fast approximate antialiasing), the compact "console" variant by Timothy Lottes.
//
// Finds the local edge direction from the luma of the 4 diagonal neighbours and blends along it.
// Works on any input, smooths shader / alpha-test edges that MSAA misses. No parameters.

const FXAA_SPAN_MAX: f32 = 8.0;          // longest blur along an edge, in pixels
const FXAA_REDUCE_MIN: f32 = 1.0 / 128.0;
const FXAA_REDUCE_MUL: f32 = 1.0 / 8.0;

// Perceptual luma (sqrt approximates the sRGB curve of the linear input).
fn fxaa_luma(c: vec3<f32>) -> f32 {
    return dot(sqrt(max(c, vec3<f32>(0.0))), vec3<f32>(0.299, 0.587, 0.114));
}

fn post(p: PostInput) -> vec4<f32> {
    let texel = 1.0 / input_size();
    let center = input_color(p.uv);
    let l_nw = fxaa_luma(input_color(p.uv + vec2<f32>(-1.0, -1.0) * texel).rgb);
    let l_ne = fxaa_luma(input_color(p.uv + vec2<f32>(1.0, -1.0) * texel).rgb);
    let l_sw = fxaa_luma(input_color(p.uv + vec2<f32>(-1.0, 1.0) * texel).rgb);
    let l_se = fxaa_luma(input_color(p.uv + vec2<f32>(1.0, 1.0) * texel).rgb);
    let l_m = fxaa_luma(center.rgb);
    let l_min = min(l_m, min(min(l_nw, l_ne), min(l_sw, l_se)));
    let l_max = max(l_m, max(max(l_nw, l_ne), max(l_sw, l_se)));

    var dir = vec2<f32>(-((l_nw + l_ne) - (l_sw + l_se)), (l_nw + l_sw) - (l_ne + l_se));
    let reduce = max((l_nw + l_ne + l_sw + l_se) * 0.25 * FXAA_REDUCE_MUL, FXAA_REDUCE_MIN);
    let rcp_min = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * rcp_min, vec2<f32>(-FXAA_SPAN_MAX), vec2<f32>(FXAA_SPAN_MAX)) * texel;

    let a = 0.5 * (input_color(p.uv + dir * (1.0 / 3.0 - 0.5)).rgb + input_color(p.uv + dir * (2.0 / 3.0 - 0.5)).rgb);
    let b = a * 0.5 + 0.25 * (input_color(p.uv - dir * 0.5).rgb + input_color(p.uv + dir * 0.5).rgb);
    let l_b = fxaa_luma(b);
    // The wide sample left the local range: it crossed another edge, use the narrow one.
    let result = select(b, a, l_b < l_min || l_b > l_max);
    return vec4<f32>(result, center.a);
}
