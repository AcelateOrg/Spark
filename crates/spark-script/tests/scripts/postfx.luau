-- Built-in post effects on `graphics`: nil/false = off, true = defaults, table = settings.
local function fails(f, needle)
    local ok, err = pcall(f)
    assert(not ok, "expected an error")
    if needle then
        assert(string.find(tostring(err), needle, 1, true), tostring(err))
    end
end

function start()
    -- everything off by default, tonemapping included (no change to existing games)
    assert(graphics.tonemap == "off")
    assert(graphics.bloom == nil and graphics.vignette == nil and graphics.fxaa == false)

    graphics.tonemap = "aces"
    assert(graphics.tonemap == "aces")
    fails(function() graphics.tonemap = "acse" end, "unknown 'acse'")

    graphics.bloom = true
    local b = graphics.bloom
    assert(type(b) == "table" and b.strength > 0 and b.threshold > 0, "true gives defaults")

    graphics.bloom = { strength = 2 }
    assert(graphics.bloom.strength == 2, "key set")
    assert(graphics.bloom.threshold == b.threshold, "missing keys keep defaults")

    graphics.ssao = { samples = 1000.4 }
    assert(graphics.ssao.samples == 64, "sample counts are clamped")

    graphics.color_grading = { gain = vec3(1, 0.9, 0.8), lift = { 0, 0, 0.05 } }
    local cg = graphics.color_grading
    assert(cg.gain.y > 0.89 and cg.gain.y < 0.91 and cg.lift.z > 0.04, "vec3 settings")
    assert(cg.exposure == 1 and cg.saturation == 1, "neutral defaults")

    graphics.vignette = { color = Color.RED, strength = 0.7 }
    assert(graphics.vignette.color == Color.RED)

    graphics.chromatic_aberration = { radial = false }
    assert(graphics.chromatic_aberration.radial == false)

    graphics.fxaa = true
    assert(graphics.fxaa == true)

    -- mistakes are errors instead of silently ignored settings
    fails(function() graphics.bloom = { strenght = 1 } end, "unknown setting 'strenght'")
    fails(function() graphics.bloom = { strength = "big" } end, "graphics.bloom.strength")
    fails(function() graphics.dof = 5 end, "expected a table")
    fails(function() graphics.vignette = { color = true } end, "graphics.vignette.color")

    graphics.bloom = false
    assert(graphics.bloom == nil)
    graphics.vignette = nil
    assert(graphics.vignette == nil)
    quit()
end
