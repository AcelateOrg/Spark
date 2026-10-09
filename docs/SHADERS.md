# Spark Engine - shaders

Spark ships no looks, presets or effects. Out of the box you get one plain lit shader (sun + ambient + up to 32
point / spot lights + fog). Everything else - toon, PS1, pixel art, VHS, outlines, CRT - is a small WGSL file in your game.

You write **only your function**. The engine adds uniforms, bindings, entry points and helpers before compiling
(the full prelude is in `crates/spark-core/src/shaders/`). Two kinds:

| kind | you write | used by |
|---|---|---|
| surface | `fn fragment(f: Fragment) -> vec4<f32>` and/or `fn vertex(v: VertexInput) -> Fragment` | `graphics.shader`, `mat:with_shader(s)`, `obj.shader` |
| post | `fn post(p: PostInput) -> vec4<f32>` | `graphics.post = { s1, s2, ... }` |

A file containing `fn post(` is a post shader, anything else is a surface shader. Missing `vertex` / `fragment`
fall back to `default_vertex` / `default_fragment`.

## Params

Declare `struct Params { ... }` (f32, i32, u32, vec2/3/4<f32>; up to 256 bytes) and read `params.x` in WGSL.
From Luau every field is a property of the Shader, starting at 0:

```wgsl
struct Params {
    strength: f32,
    tint: vec4<f32>,
}
```
```lua
local s = Shader.load("shaders/tint.wgsl")
s.strength = 0.5
s.tint = "#ff8040"          -- colors are converted to linear
s:set({ strength = 1, tint = Color.rgb(1, 0, 0) })
s.texture1 = "noise.png"    -- optional extra textures: texture1, texture2
```

Params are shared by everything that uses the shader. Per-object values go through `material.data`
(`mat:with_data(a, b, c, d)`, `obj.data = {...}`) -> `object.data` in WGSL.

## Hot reload and errors

`Shader.load` files reload when saved. A compile error keeps the last working version and shows the error with the
line of **your** file on screen; fixing the file clears it. `Shader.new(code)` raises a normal Luau error.
Headless runs (`--headless --screenshot`) are the fastest way to check a shader.

## Available in every shader

```wgsl
frame.view_proj, frame.view, frame.proj, frame.inv_view_proj   // mat4x4<f32>
frame.camera_pos.xyz
frame.camera        // near, far, vertical fov (radians), aspect
frame.screen        // scene width, scene height, output width, output height (pixels)
frame.time          // seconds, dt, real seconds (ignores time.scale), frame number
frame.sun_dir.xyz, frame.sun_color.rgb, frame.ambient.rgb   // linear
frame.fog_color.rgb, frame.fog                              // fog: near, far, enabled
frame.light_count.x, frame.lights[i]                        // Light { position(xyz, w=range), color(rgb, w=1 spot), direction(xyz, w=cos outer), params(x=cos inner) }
params                                                      // your struct Params
texture1, texture2, linear_sampler, nearest_sampler
linear_to_srgb(rgb), srgb_to_linear(rgb)
```

All colors are linear; the engine converts to the screen at the end.

## Surface shaders

```wgsl
struct VertexInput { position: vec3<f32>, normal: vec3<f32>, uv: vec2<f32> }   // locations 0..2
struct Fragment {
    clip: vec4<f32>,       // @builtin(position)
    world_pos: vec3<f32>,
    normal: vec3<f32>,     // world space, not normalized
    uv: vec2<f32>,         // already multiplied by material tiling
    custom: vec4<f32>,     // free: pass anything from vertex to fragment
}
object.model, object.normal_matrix, object.color (linear), object.data, object.info (tiling x, tiling y, unlit 0/1)
base_texture, base_sampler                 // material texture (white if none), sampled with graphics.filter

default_vertex(v) -> Fragment
surface_color(uv) -> vec4<f32>             // object.color * base texture
lighting(world_pos, normal) -> vec3<f32>   // ambient + sun + lights
apply_fog(rgb, world_pos) -> vec3<f32>
default_fragment(f) -> vec4<f32>
```

Toon:

```wgsl
struct Params { bands: f32 }

fn fragment(f: Fragment) -> vec4<f32> {
    let base = surface_color(f.uv);
    let light = lighting(f.world_pos, f.normal);
    let l = max(max(light.r, light.g), light.b);
    let stepped = ceil(l * max(params.bands, 1.0)) / max(params.bands, 1.0);
    return vec4<f32>(apply_fog(base.rgb * light / max(l, 0.001) * stepped, f.world_pos), base.a);
}
```

PS1 vertex snapping (wobble):

```wgsl
struct Params { grid: f32 }   // e.g. 160

fn vertex(v: VertexInput) -> Fragment {
    var f = default_vertex(v);
    let g = vec2<f32>(params.grid * frame.camera.w, params.grid);
    f.clip = vec4<f32>(floor(f.clip.xy / f.clip.w * g) / g * f.clip.w, f.clip.zw);
    return f;
}
```

## Post shaders

Run full-screen, one after another; each pass reads the previous image.

```wgsl
struct PostInput { position: vec4<f32>, uv: vec2<f32> }   // uv 0..1, top-left = (0, 0)
input_color(uv) -> vec4<f32>   // previous image (3D scene + scene-layer 2D + earlier passes), sampled with graphics.upscale
input_size(), output_size()    // pixels
scene_depth(uv) -> f32         // raw depth 0..1 (1 = sky)
linear_depth(uv) -> f32        // meters from the camera
```

The UI layer (`draw.*` with layer "ui") is drawn after all passes, so effects never touch menus/HUD.
`{ shader = s, size = "scene" }` runs a pass at the internal resolution (`graphics.height`) - pixel-exact dither,
cheap blur; the default `"screen"` runs at window resolution (scanlines, CRT masks).

Grayscale:

```wgsl
struct Params { amount: f32 }

fn post(p: PostInput) -> vec4<f32> {
    let c = input_color(p.uv);
    let g = dot(c.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(mix(c.rgb, vec3<f32>(g), params.amount), 1.0);
}
```

Examples in the repo: `examples/demo/shaders/` (toon, pixel, outline)
(vignette, grain, scanlines, tape smear, chromatic aberration, color levels + dither).

## Rust

```rust
let s = world.assets.add_shader("gray", GRAY_WGSL)?;          // or world.assets.load_shader("shaders/gray.wgsl")?
world.assets.shader_mut(s).unwrap().set_param("amount", &[0.8])?;
world.render.height = Some(240);
world.render.post = vec![PostPass { shader: s, size: PassSize::Screen }];
let mat = Material::color(Color::RED).with_shader(Some(toon)).with_data([1.0, 0.0, 0.0, 0.0]);
```
