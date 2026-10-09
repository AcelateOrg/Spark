-- Spark demo: run with  spark examples/demo   (or cargo run -p spark-cli -- examples/demo)
-- WASD + mouse drag (right button) = fly camera, Q/E = down/up, 1-4 = look (our own shaders in shaders/), Space = slow motion,
-- left click = shoot a ball, F = drop crates.
-- Save this file while the game runs: it reloads instantly.

local cubes = {}
local yaw, pitch = 0, -0.35
-- Looks are not built into the engine: these are this game's own WGSL shaders.
local toon = Shader.load("shaders/toon.wgsl")
toon.bands = 3
local pixel = Shader.load("shaders/pixel.wgsl")
pixel.levels = 12
local outline = Shader.load("shaders/outline.wgsl")
outline.color = Color.rgb(0.05, 0.04, 0.08)
outline.thickness = 1.5

local function set_look(n)
    graphics.height = nil
    graphics.upscale = "linear"
    graphics.filter = "linear"
    graphics.shader = nil
    graphics.post = nil
    if n == 2 then
        graphics.shader = toon
        graphics.post = { outline }
    elseif n == 3 then
        graphics.height = 180
        graphics.upscale = "nearest"
        graphics.filter = "nearest"
        graphics.post = { { shader = pixel, size = "scene" } }
    elseif n == 4 then
        graphics.scale = 0.5
    end
    if n ~= 4 then graphics.scale = 1 end
end
local LOOKS = { "plain", "toon + outlines", "pixel 180p + dither", "half resolution" }
local crate_colors = { "#c8873a", "#b5762e", "#d89a4a" }

-- Physics + sound: impacts play a pop, louder for harder hits (max one per object every 0.1 s).
local last_hit = {}
local function add_hit_sound(obj)
    obj:on("collision", function(other, info)
        local now = time.elapsed
        if info.speed > 1 and now - (last_hit[obj] or -1) > 0.1 then
            last_hit[obj] = now
            sound.play("sounds/hit.wav", { volume = math.min(info.speed / 8, 1), pitch = 0.8 + math.random() * 0.4 })
        end
    end)
end

local function drop_crates(n)
    for i = 1, n do
        local crate = spawn(Mesh.cube(0.7), crate_colors[i % 3 + 1], {
            position = vec3(math.random() * 4 - 2, 6 + i * 0.9, math.random() * 4 - 2),
            rotation = vec3(math.random(), math.random(), math.random()),
            body = { mass = 2, bounciness = 0.2 },
        })
        add_hit_sound(crate)
        after(30, function()
            crate:destroy()
        end, crate)
    end
end

local function shoot()
    local ball = spawn(Mesh.sphere(0.25, 12), "white", {
        position = camera.position + camera:forward(),
        body = { shape = "sphere", radius = 0.25, mass = 3, bounciness = 0.5, ccd = true },
    })
    ball.velocity = camera:forward() * 25
    add_hit_sound(ball)
    sound.play("sounds/spawn.wav", { volume = 0.4 })
    after(10, function()
        ball:destroy()
    end, ball)
end

function start()
    background("sky")
    sun(vec3(-0.5, -1, -0.3), "white", 1.1)
    ambient("#5a6070")
    fog("sky", 15, 45)

    local ground = spawn(Mesh.plane(40), Material.checker("#6fae4f", "#5d9a42", 20), { name = "ground", body = "static" })

    local colors = { "red", "orange", "yellow", "green", "blue", "purple" }
    for i, c in colors do
        local angle = i / #colors * math.pi * 2
        local cube = spawn(Mesh.cube(1), c, {
            name = "cube" .. i,
            position = vec3(math.cos(angle) * 4, 0, math.sin(angle) * 4),
        })
        cube:place_on(ground)
        cube.body = "kinematic" -- moved by code, pushes crates around
        table.insert(cubes, cube)
    end

    local tower = spawn(Mesh.cylinder(0.6, 3), "#d8d0c0", { name = "tower" })
    tower:place_on(ground)
    tower.body = "static"
    local ball = spawn(Mesh.sphere(0.8), "pink", { name = "ball", body = "kinematic" })
    ball:place_on(tower)

    camera.position = vec3(0, 4, 11)
    camera.rotation = vec3(pitch, yaw, 0)

    -- Events: keys arrive as events, no polling needed.
    on("key_pressed", function(key)
        local n = tonumber(key)
        if n and LOOKS[n] then
            set_look(n)
            print("look:", LOOKS[n])
        elseif key == "f" then
            drop_crates(8)
        elseif key == "space" then
            time.scale = if time.scale == 1 then 0.25 else 1
        elseif key == "escape" then
            quit()
        end
    end)

    on("mouse_pressed", function(button)
        if button == "left" then
            shoot()
        end
    end)

    drop_crates(10)

    -- Scheduler: the ball blinks every second (timer dies with the ball)...
    every(1, function(n)
        ball.color = if n % 2 == 0 then "pink" else "white"
    end, ball)

    -- ...and the cubes hop one after another, forever, as a task.
    task(function()
        while true do
            for _, cube in cubes do
                cube.y += 0.6
                wait(0.12)
                cube.y -= 0.6
            end
            wait(0.8)
        end
    end)
end

function update(dt)
    for i, cube in cubes do
        cube:rotate_y(dt * (0.5 + i * 0.2))
    end
    local ball = find("ball")
    if ball then
        ball.y = 3.8 + math.sin(time.elapsed * 2) * 0.3
    end
end

-- Camera after everything else has moved. Uses unscaled time so slow motion does not slow the camera.
function late_update()
    local dt = time.unscaled_dt
    if input.mouse_down("right") then
        local dx, dy = input.mouse_delta()
        yaw -= dx * 0.004
        pitch = math.clamp(pitch - dy * 0.004, -1.5, 1.5)
    end
    camera.rotation = vec3(pitch, yaw, 0)
    local move = vec3(0, 0, 0)
    if input.down("w") then move += camera:forward() end
    if input.down("s") then move -= camera:forward() end
    if input.down("d") then move += camera:right() end
    if input.down("a") then move -= camera:right() end
    if input.down("e") then move += vec3(0, 1, 0) end
    if input.down("q") then move -= vec3(0, 1, 0) end
    local speed = if input.down("shift") then 12 else 5
    camera.position += normalize(move) * speed * dt
end
