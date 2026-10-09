-- Contract test: save (memory only in tests), glTF models + animation, tween, character controller, window.
local FIGURE = "assets/figure.glb" -- skinned humanoid with idle / walk animations

function start()
    -- gamepads (none in tests) and mixer buses
    assert(input.pads() == 0 and input.pad_name() == nil)
    assert(not input.pad_down("a") and not input.pad_pressed("start", 2))
    assert(input.pad_axis("left_x") == 0)
    local sx, sy = input.pad_stick("right")
    assert(sx == 0 and sy == 0)
    assert(not pcall(input.pad_down, "triangle"))
    local music = sound.bus("music")
    assert(music.volume == 1 and not music.paused and music.name == "music")
    music:set_volume(0.3, 1)
    assert(math.abs(sound.bus("music").volume - 0.3) < 1e-6)
    music.paused = true
    assert(sound.bus("music").paused)
    music:resume()
    music:stop(0.5)
    assert(not pcall(sound.bus, ""))

    -- save
    assert(save.get("x", 5) == 5)
    save.set("x", { a = 1, list = { 1, 2, 3 }, v = vec3(1, 2, 3), s = "hi", b = true })
    local x = save.get("x")
    assert(x.a == 1 and #x.list == 3 and x.list[3] == 3 and x.v.y == 2 and x.s == "hi" and x.b == true, "save roundtrip")
    assert(save.has("x"))
    save.delete("x")
    assert(not save.has("x") and save.path() == nil)
    assert(not pcall(save.set, "f", function() end), "functions cannot be saved")

    -- models
    local m = Model.load(FIGURE)
    assert(#m.animations == 5, "clip count " .. #m.animations)
    assert(m:duration("walk") > 1)
    assert(m.size.y > 2.3, "model size")
    local f = spawn(m, "black", { position = vec3(1, 0, 0) })
    local head = f:find("head")
    assert(head, "head bone")
    local y0 = head:world_position().y
    assert(y0 > 2 and y0 < 2.3, "head y " .. y0)
    f:play("reach", { loop = false, fade = 0, speed = 4 })
    wait(0.6)
    assert(not f:is_playing("reach"), "one-shot finished")
    local hp = f:find("hand.R"):world_position()
    assert(hp.z < -0.5, "hand reached forward, z " .. hp.z)
    local lo = f:find("body"):bounds()
    assert(lo.z < -0.8, "skinned bounds follow the pose " .. lo.z)
    f:play("walk")
    wait(0.1)
    assert(f:animation() == "walk" and f:is_playing(), "walk playing")
    f:set_animation_speed(0)
    local t = f:animation_time()
    wait(0.1)
    assert(f:animation_time() == t, "frozen")
    assert(not pcall(function() f:play("dance") end), "unknown clip errors")
    f:paint("red")
    assert(f:find("body").color.r > 0.8)
    f:destroy()
    local g = spawn(FIGURE, { name = "g" })
    assert(g.name == "g" and g:find("foot.L"))

    -- tween
    local box = spawn(Mesh.cube(), "red")
    local done = false
    tween(box, { position = vec3(0, 2, 0), color = "#00ff00" }, 0.25, { ease = "linear", done = function() done = true end })
    wait(0.4)
    assert(done and math.abs(box.y - 2) < 1e-3, "tween y " .. box.y)
    assert(box.color.g > 0.99, "color tween")
    local h = tween(box, { x = 5 }, 1)
    wait(0.1)
    h:cancel()
    local bx = box.x
    wait(0.1)
    assert(box.x == bx and bx > 0, "cancel")
    assert(not pcall(tween, box, { x = 1 }, 1, "wobble"), "unknown ease errors")

    -- character controller
    spawn(Mesh.box(20, 1, 20), "gray", { position = vec3(0, -0.5, 0), body = "static" })
    spawn(Mesh.box(1, 3, 6), "gray", { position = vec3(3, 1.5, 0), body = "static" })
    local hero = group({ position = vec3(0, 1.5, 0), body = { type = "kinematic", shape = "capsule", radius = 0.3, height = 1.8 } })
    wait_frames(2)
    local grounded = false
    for _ = 1, 150 do
        local _, gr = hero:move(vec3(4, -5, 0) * time.dt)
        grounded = gr
        wait_frames(1)
    end
    assert(grounded, "grounded")
    assert(hero.x < 2.55 and hero.x > 1.9, "blocked by the wall, x " .. hero.x)
    assert(hero.y > 0.7 and hero.y < 1.4, "standing on the floor, y " .. hero.y)
    assert(not pcall(function() box:move(vec3(1, 0, 0)) end), "move needs a body")

    window.title = "x"
    assert(window.title == "x" and window.fullscreen == false)
    quit()
end
