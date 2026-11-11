-- PBR materials, emissive, normal maps and mesh LOD: properties, builders, argument errors.
local nm = Texture.solid("#8080ff")

local m = Material.pbr("#c0c0c0", 1, 0.2)
assert(m.pbr and m.metallic == 1, "Material.pbr sets the fields")
assert(math.abs(m.roughness - 0.2) < 1e-6)
assert(Material.pbr().metallic == 0 and Material.pbr().roughness == 0.5, "defaults: dielectric, roughness 0.5")
assert(Material.pbr("white", 5, -1).metallic == 1 and Material.pbr("white", 5, -1).roughness == 0, "values are clamped")
local c = Material.color("red")
assert(not c.pbr, "classic materials stay classic")
assert(c:with_roughness(0.5).pbr, "with_roughness turns PBR on")
assert(not c:with_emissive("#ff8000").pbr, "emissive works without PBR")
assert(c:with_normal_map(nm).normal_map ~= nil)
assert(c:with_normal_map(nm):with_normal_map(nil).normal_map == nil, "nil removes the normal map")

local high, low = Mesh.sphere(), Mesh.cube()
local o = spawn(high, "white", {
    pbr = true, metallic = 0.5, roughness = 0.3, emissive = "#202020", normal_map = nm,
    lod = { { 30, low } },
})
assert(o.pbr and o.metallic == 0.5 and o.normal_map ~= nil)
assert(o.emissive ~= nil)
assert(#o.lod == 1 and o.lod[1][1] == 30, "lod reads back")
o.roughness = 2
assert(o.roughness == 1, "property setter clamps")
o.normal_map = false
assert(o.normal_map == nil)
o.lod = {}
assert(#o.lod == 0)
o.pbr = false
assert(not o.pbr)

assert(not pcall(function() o.lod = { 30, low } end), "lod needs {distance, Mesh} pairs")
assert(not pcall(function() o.lod = { { -1, low } } end), "negative distance is rejected")
assert(not pcall(function() o.lod = { { 10, "cube" } } end), "lod needs a Mesh")
assert(not pcall(function() o.normal_map = 5 end), "normal_map needs a texture")
assert(not pcall(spawn, low, "white", { metalic = 1 }), "typos are reported")

function update()
    quit()
end
