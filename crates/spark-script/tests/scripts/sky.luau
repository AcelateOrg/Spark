-- sky(): gradient / cubemap sky and sky lighting; argument errors. Run by flow.rs from a temp folder
-- with generated images in assets/.
sky({ top = "sky", bottom = "#303030" })
sky({ top = Color.rgb(0.2, 0.4, 0.9), bottom = "black", lighting = 1, blend = 0.5 })
sky(true)
sky(nil)
sky(false)
sky({ cubemap = false })
sky({ cubemap = "assets/sky_strip.png", lighting = 0.8 })
sky({ cubemap = "assets/sky_strip.png" }) -- cached by path

assert(not pcall(sky, { tpo = "red" }), "typos are reported")
assert(not pcall(sky, { top = true }), "colors are checked")
assert(not pcall(sky, { cubemap = 1 }), "cubemap needs a path")
assert(not pcall(sky, { cubemap = "missing_sky.png" }), "missing files are errors")
local ok, err = pcall(sky, { cubemap = "assets/not_a_strip.png" })
assert(not ok and string.find(tostring(err), "6"), "a cubemap must be a 6:1 strip: " .. tostring(err))
assert(not pcall(sky, 42), "sky needs a table")

function update()
    quit()
end
