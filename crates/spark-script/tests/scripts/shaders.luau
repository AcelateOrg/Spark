-- Shader / render contract. Runs headless without a GPU: compiles WGSL with naga, checks params and render settings.

local function expect(cond, msg)
    if not cond then
        error("FAIL: " .. msg, 2)
    end
end

local function fails(f, needle)
    local ok, err = pcall(f)
    if ok then
        return false
    end
    return needle == nil or string.find(tostring(err), needle, 1, true) ~= nil
end

local POST = [[
struct Params {
    strength: f32,
    tint: vec3<f32>,
    offset: vec2<f32>,
    enabled: f32,
}

fn post(p: PostInput) -> vec4<f32> {
    let c = input_color(p.uv + params.offset);
    let g = dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
    return vec4<f32>(mix(c.rgb, vec3<f32>(g) * params.tint, params.strength), 1.0);
}
]]

local SURFACE = [[
struct Params {
    bands: f32,
}

fn fragment(f: Fragment) -> vec4<f32> {
    let base = surface_color(f.uv);
    let l = lighting(f.world_pos, normalize(f.normal));
    let q = floor(l * params.bands) / params.bands;
    return vec4<f32>(apply_fog(base.rgb * q, f.world_pos), base.a);
}
]]

local function check_post()
    local s = Shader.new(POST, "gray")
    expect(s.name == "gray", "name")
    expect(s.kind == "post", "kind post, got " .. s.kind)
    local names = table.concat(s.params, ",")
    expect(names == "strength,tint,offset,enabled", "params in declaration order, got " .. names)
    expect(s.strength == 0, "params start at zero")
    s.strength = 0.75
    expect(math.abs(s.strength - 0.75) < 1e-6, "set/get f32")
    s.tint = Color.rgb(1, 1, 1)
    expect(math.abs(s.tint.x - 1) < 1e-6, "color -> vec3")
    s.offset = vec3(0.5, 0.25, 0)
    expect(s.offset.y == 0.25, "vector -> vec2")
    s.enabled = true
    expect(s.enabled == 1, "bool -> 1")
    s:set({ strength = 0.5, offset = { 1, 2 } })
    expect(s.strength == 0.5 and s.offset.x == 1 and s.offset.y == 2, "set{} with list table")
    expect(fails(function() s.nope = 1 end, "strength"), "unknown param lists params")
    expect(fails(function() return s.nope end, "nope"), "unknown param read errors")
    expect(fails(function() s.strength = "abc" end), "bad value type errors")
    expect(s.texture1 == nil, "texture1 nil by default")
    return s
end

local function check_surface()
    local s = Shader.new(SURFACE, "toon")
    expect(s.kind == "surface", "kind surface, got " .. s.kind)
    s.bands = 4
    local m = Material.color("white"):with_shader(s):with_data(1, 2, 3, 4)
    expect(m.shader ~= nil, "material keeps shader")
    expect(m.data.w == 4 or m.data[4] == 4, "material data")
    local o = spawn(Mesh.cube(), m)
    expect(o ~= nil, "spawn with shader material")
    local o2 = spawn(Mesh.cube(), "red", { shader = s, data = { 0.5 } })
    expect(o2.shader ~= nil, "spawn prop shader")
    o2.shader = nil
    expect(o2.shader == nil, "clear object shader")
    -- A plain shader without Params still works.
    local plain = Shader.new("fn fragment(f: Fragment) -> vec4<f32> { return surface_color(f.uv); }")
    expect(#plain.params == 0, "no params")
    expect(fails(function() plain.x = 1 end, "Params"), "hint to declare Params")
    return s
end

local function check_errors()
    expect(fails(function() Shader.new("fn post(p: PostInput) -> vec4<f32> { return oops; }", "bad") end, "oops"),
        "compile error mentions the bad identifier")
    expect(fails(function() Shader.load("shaders/missing.wgsl") end), "missing file errors")
end

local function check_render(post, surface)
    expect(graphics.height == nil, "native resolution by default")
    expect(graphics.upscale == "linear" or graphics.upscale == "nearest", "upscale readable")
    graphics.height = 180
    expect(graphics.height == 180, "height")
    expect(graphics.scene_height == 180, "scene_height follows height, got " .. graphics.scene_height)
    expect(graphics.scene_width == 320, "scene_width keeps 16:9, got " .. graphics.scene_width)
    graphics.height = nil
    expect(graphics.scene_height == 720, "native again")
    graphics.upscale = "nearest"
    graphics.filter = "nearest"
    expect(graphics.upscale == "nearest" and graphics.filter == "nearest", "filters")
    expect(fails(function() graphics.filter = "cubic" end, "cubic"), "bad filter errors")

    graphics.shader = surface
    expect(graphics.shader ~= nil, "default surface shader")
    expect(fails(function() graphics.shader = post end, "post shader"), "post shader rejected as surface")
    graphics.shader = nil

    graphics.post = post
    expect(#graphics.post == 1, "single post")
    graphics.post = { post, { shader = post, size = "scene" } }
    expect(#graphics.post == 2, "post chain")
    expect(fails(function() graphics.post = { surface } end, "surface shader"), "surface rejected as post")
    expect(fails(function() graphics.post = { { shader = post, size = "huge" } } end, "huge"), "bad size errors")
    graphics.post = nil
    expect(#graphics.post == 0, "post cleared")
end

function start()
    local post = check_post()
    local surface = check_surface()
    check_errors()
    check_render(post, surface)
    print("shaders contract ok")
    quit()
end
