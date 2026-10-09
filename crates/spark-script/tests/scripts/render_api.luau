-- Rendering API contract (no GPU): materials, camera helpers, graphics settings.

local function expect(cond, msg)
    if not cond then
        error("FAIL: " .. msg, 2)
    end
end

function start()
    -- Blend modes / double-sided / shadows on materials and objects.
    local glass = Material.color("white"):with_blend("alpha"):with_double_sided()
    expect(glass.blend == "alpha" and glass.double_sided, "material blend / double_sided")
    expect(Material.color("red"):with_cast_shadow(false).cast_shadow == false, "material cast_shadow")
    local o = spawn(Mesh.cube(), glass, { position = vec3(0, 1, 0) })
    expect(o.blend == "alpha", "object blend from material")
    o.blend = "additive"
    o.double_sided = false
    o.cast_shadow = false
    expect(o.blend == "additive" and not o.double_sided and not o.cast_shadow, "object setters")
    local leaf = spawn(Mesh.plane(1, 1), "green", { blend = "opaque", double_sided = true, cast_shadow = false })
    expect(leaf.double_sided and leaf.blend == "opaque", "spawn props")
    expect(not pcall(function() o.blend = "glow" end), "unknown blend mode is an error")

    -- Camera: screen <-> world.
    camera.position = vec3(0, 0, 10)
    camera:look_at(vec3(0, 0, 0))
    local x, y, depth = camera:world_to_screen(vec3(0, 0, 0))
    expect(x ~= nil and depth > 0 and depth < 1, "world_to_screen in front")
    local origin, dir = camera:screen_to_ray(x, y)
    expect(math.abs(dir.z + 1) < 1e-3 and origin.z < 10, "screen_to_ray through the center")
    expect(camera:world_to_screen(vec3(0, 0, 20)) == nil, "behind the camera = nil")
    camera.ortho = 12
    expect(camera.ortho == 12, "orthographic camera")
    local ox, oy = camera:world_to_screen(vec3(0, 0, 0))
    expect(math.abs(ox - x) < 0.5 and math.abs(oy - y) < 0.5, "ortho center")
    camera.ortho = nil
    expect(camera.ortho == nil, "back to perspective")

    -- Graphics settings.
    graphics.msaa = 1
    expect(graphics.msaa == 1, "msaa")
    graphics.msaa = true
    expect(graphics.msaa == 4, "msaa = true means 4x")
    graphics.shadows = false
    expect(graphics.shadows == false, "shadows off")
    graphics.shadow_distance = 60
    graphics.shadow_size = 1024
    expect(graphics.shadow_distance == 60 and graphics.shadow_size == 1024, "shadow settings")
    quit()
end
