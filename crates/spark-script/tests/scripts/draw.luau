-- 2D drawing contract. Runs headless without a GPU: checks the API, layout math and errors.

local function expect(cond, msg)
    if not cond then
        error("FAIL: " .. msg, 2)
    end
end

local function near(a, b, eps)
    return math.abs(a - b) < (eps or 0.5)
end

local function fails(f, needle)
    local ok, err = pcall(f)
    if ok then
        return false
    end
    return needle == nil or string.find(tostring(err), needle, 1, true) ~= nil
end

local rendered = 0

function render()
    rendered += 1
    draw.rect(10, 10, 100, 50, "red")
    draw.text("render", 0, 0)
end

local function check_screen()
    -- Default output in tests is 1280x720 and the virtual screen is 720 tall.
    expect(screen.height == 720, "default virtual height")
    expect(near(screen.width, 1280), "width follows aspect, got " .. screen.width)
    expect(screen.scale == 1, "scale 1 at 720p")
    local cx, cy = screen.center()
    expect(near(cx, 640) and near(cy, 360), "screen.center()")
    screen.height = 360
    expect(near(screen.width, 640) and screen.scale == 2, "screen.height changes the virtual size")
    screen.height = 720
    local mx, my = screen.mouse()
    expect(type(mx) == "number" and type(my) == "number", "screen.mouse()")
    expect(fails(function() screen.width = 5 end, "aspect"), "screen.width is read-only")
    expect(fails(function() screen.height = 0 end), "screen.height must be > 0")
    expect(fails(function() return screen.dpi end, "does not exist"), "unknown screen field")
end

local function check_shapes()
    local before = draw.stats().ui_triangles
    draw.rect(0, 0, 10, 10, "blue")
    draw.rect(0, 0, 10, 10, "#ff8800", { line = 2, align = "center" })
    draw.circle(100, 100, 20, Color.rgba(1, 1, 1, 0.5))
    draw.circle(100, 100, 20, nil, { line = 3 })
    draw.line(0, 0, 50, 50, "white", 4)
    local s = draw.stats()
    expect(s.ui_triangles > before + 10, "shapes add triangles")
    expect(s.ui_batches == 1, "shapes share one batch, got " .. s.ui_batches)
    expect(fails(function() draw.rect(0, 0, 1, 1, "red", { allign = "center" }) end, "allowed: align, line"), "unknown option lists allowed keys")
    expect(fails(function() draw.rect(0, 0, 1, 1, "red", { align = "middle-ish" }) end, "topleft"), "bad align lists names")
    expect(fails(function() draw.rect(0, 0, 1, 1, "redd") end, "unknown color"), "bad color")
end

local function check_text()
    local w, h = draw.measure("Hello", { size = 32 })
    expect(w > 40 and h >= 32, "measure gives a size")
    local w2 = draw.measure("Hello Hello", { size = 32 })
    expect(w2 > w * 1.8, "longer text is wider")
    local _, h2 = draw.measure("a\nb", { size = 32 })
    expect(h2 > h * 1.5, "newline adds a line")
    local ww, wh = draw.measure("word word word word word", { size = 24, width = 100 })
    expect(ww <= 100 and wh > 50, "wrapping to width")
    local dw, dh = draw.text("Hello", 640, 360, { size = 32, align = "center", color = "yellow", shadow = true })
    expect(near(dw, w) and near(dh, h), "draw.text returns the same size as measure")
    local nw = draw.text(12345, 0, 0)
    expect(nw > 0, "numbers are accepted as text")
    local cw = draw.measure("Привет, мир!")
    expect(cw > 50, "Cyrillic text has width")
    expect(fails(function() draw.text("x", 0, 0, { size = 0 }) end, "size"), "size must be > 0")
    expect(fails(function() draw.text({}, 0, 0) end, "string or a number"), "text type")
    expect(fails(function() Font.load("missing.ttf") end, "cannot read font"), "missing font file")
    expect(fails(function() draw.text("x", 0, 0, { font = "Arial" }) end, "Font.load"), "font must be loaded")
end

local function check_sprites()
    local tex = Texture.checker("white", "black", 4, 64)
    local before = draw.stats()
    draw.sprite(tex, 100, 100)
    draw.sprite(tex, 100, 100, { w = 32, scale = 2, rotation = 0.5, flip_x = true, color = "red", align = "bottom" })
    draw.sprite(tex, 100, 100, { grid = { 4, 2 }, frame = 11, filter = "nearest" })
    draw.sprite(tex, 100, 100, { region = { 0, 0, 16, 16 } })
    local s = draw.stats()
    expect(s.ui_triangles == before.ui_triangles + 8, "4 sprites = 8 triangles")
    expect(fails(function() draw.sprite("missing.png", 0, 0) end, "draw.sprite"), "missing image errors")
    expect(fails(function() draw.sprite(tex, 0, 0, { grid = { 0, 1 } }) end, "columns"), "bad grid")
    expect(fails(function() draw.sprite(tex, 0, 0, { sise = 3 }) end, "allowed"), "unknown sprite option")
end

local function check_transforms_and_layers()
    draw.push()
    draw.translate(100, 50)
    draw.rotate(0.3)
    draw.scale(2)
    draw.rect(0, 0, 10, 10)
    draw.pop()
    expect(fails(function() draw.pop() end, "without a matching"), "unbalanced pop")
    expect(draw.layer() == "ui", "default layer is ui")
    local old = draw.layer("scene")
    expect(old == "ui" and draw.layer() == "scene", "switch to the scene layer")
    draw.rect(0, 0, 10, 10, "green")
    expect(draw.stats().scene_triangles == 2, "scene layer gets its own triangles")
    draw.layer("ui")
    expect(fails(function() draw.layer("sky") end, "ui"), "unknown layer")
    draw.filter("nearest")
    draw.filter("linear")
    expect(fails(function() draw.filter("blurry") end), "unknown filter")
end

function start()
    check_screen()
    check_shapes()
    check_text()
    check_sprites()
    check_transforms_and_layers()
    -- The canvas is cleared every frame, and render() runs every frame.
    wait_frames(1)
    expect(draw.stats().ui_triangles == 0 and draw.stats().scene_triangles == 0, "canvas cleared at frame start")
    expect(draw.layer() == "ui", "layer resets every frame")
    wait_frames(2)
    expect(rendered >= 2, "render() is called every frame, got " .. rendered)
    print("draw contract ok")
    quit()
end
