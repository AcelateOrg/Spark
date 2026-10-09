-- Physics + sound contract. Runs headless with the rapier backend and no audio backend.

local function expect(cond, msg)
    if not cond then
        error("FAIL: " .. msg, 2)
    end
end

local function near(a, b, eps)
    return math.abs(a - b) < (eps or 0.05)
end

local function check_landing()
    expect(physics.gravity.y < -9, "default gravity")
    local ground = spawn(Mesh.plane(40, 40), "gray", { name = "ground", body = "static" })
    expect(ground.body == "static", "body getter")
    local box = spawn(Mesh.cube(), "orange", { name = "box", position = vec3(0, 4, 0), body = "dynamic" })
    local hits, global_hits, speed = 0, 0, 0
    box:on("collision", function(other, info)
        if other == ground then
            hits += 1
            speed = math.max(speed, info.speed)
        end
    end)
    local off = on("collision", function(a, b, info)
        global_hits += 1
    end)
    wait(2)
    off:off()
    expect(near(box.y, 0.5), "box landed on the ground, y = " .. box.y)
    expect(hits >= 1, "obj:on('collision') fired")
    expect(global_hits >= 1, "global on('collision') fired")
    expect(speed > 3, "impact speed reported, got " .. speed)
    expect(box.velocity.y > -0.1, "box is at rest")

    local hit = raycast(vec3(0, 10, 0), vec3(0, -1, 0))
    expect(hit ~= nil and hit.object == box, "raycast hits the box")
    expect(near(hit.point.y, 1) and near(hit.distance, 9) and near(hit.normal.y, 1), "raycast point / distance / normal")
    local hit2 = raycast(vec3(0, 10, 0), vec3(0, -1, 0), 100, { ignore = box })
    expect(hit2 ~= nil and hit2.object == ground, "raycast ignore")
    expect(raycast(vec3(0, 10, 0), vec3(0, 1, 0), 100) == nil, "raycast miss")

    -- pick: rendered triangles, no colliders needed
    local p = pick(vec3(0, 10, 0), vec3(0, -1, 0))
    expect(p ~= nil and p.object == box, "pick hits the box")
    expect(near(p.point.y, 1, 0.05) and near(p.normal.y, 1) and near(p.distance, 9, 0.05), "pick point / normal / distance")
    expect(pick(vec3(0, 10, 0), vec3(0, -1, 0), 100, { ignore = box }).object == ground, "pick ignore")
    local deco = spawn(Mesh.cube(0.5), "red", { position = vec3(8, 1, 0) })
    local knob = spawn(Mesh.sphere(0.1), "white", { parent = deco, position = vec3(0, 0, 0.3) })
    local pk = pick(vec3(8, 1, 5), vec3(0, 0, -1))
    expect(pk ~= nil and pk.object == knob, "pick finds child mesh first")
    expect(pick(vec3(8, 1, 5), vec3(0, 0, -1), 100, { ignore = deco }) == nil, "pick ignore skips children")
    expect(pick(vec3(8, 1, 5), vec3(0, 0, -1), 4) == nil, "pick max distance")
    deco.visible = false
    expect(pick(vec3(8, 1, 5), vec3(0, 0, -1)) == nil, "pick skips hidden")
    deco:destroy()
    return ground, box
end

local function check_velocity_and_impulse()
    local ball = spawn(Mesh.sphere(0.5), "white", {
        position = vec3(5, 3, 0),
        body = { type = "dynamic", gravity_scale = 0 },
    })
    ball.velocity = vec3(0, 0, 2)
    wait(1)
    expect(near(ball.z, 2, 0.15), "velocity moves the ball, z = " .. ball.z)
    ball.velocity = vec3(0, 0, 0)
    wait_frames(1)
    ball:impulse(vec3(1, 0, 0))
    wait_frames(2)
    expect(ball.velocity.x > 0.1, "impulse changes velocity")
    -- Teleport: moving a body by hand moves the simulated body.
    ball.position = vec3(8, 3, 0)
    ball.velocity = vec3(0, 0, 0)
    wait_frames(2)
    expect(near(ball.x, 8, 0.1), "teleport sticks, x = " .. ball.x)
    ball:destroy()
end

local function check_sensor()
    local zone = spawn(Mesh.cube(2), "green", { position = vec3(-5, 2, 0), body = { type = "static", sensor = true } })
    expect(zone.sensor, "sensor getter")
    local entered, left = false, false
    zone:on("collision", function(other, info)
        expect(info.sensor, "sensor flag in info")
        entered = true
    end)
    zone:on("collision_end", function()
        left = true
    end)
    local faller = spawn(Mesh.cube(0.5), "red", { position = vec3(-5, 6, 0), body = { mass = 2, bounciness = 0 } })
    expect(faller.mass == 2, "mass getter")
    wait_until(function()
        return left
    end, 3)
    expect(entered and left, "sensor enter / leave")
end

local function check_kinematic_and_gravity()
    local platform = spawn(Mesh.box(2, 0.2, 2), "blue", { position = vec3(10, 1, 0), body = "kinematic" })
    local rider = spawn(Mesh.cube(0.5), "yellow", { position = vec3(10, 1.5, 0), body = "dynamic" })
    wait(0.5)
    for i = 1, 30 do
        platform.y += 0.02
        wait_frames(1)
    end
    expect(rider.y > 1.6, "kinematic platform lifts the rider, y = " .. rider.y)

    physics.gravity = vec3(0, 5, 0)
    wait(0.5)
    expect(rider.velocity.y > 0.5, "reversed gravity")
    physics.gravity = vec3(0, -9.81, 0)
    rider:remove_body()
    expect(rider.body == nil, "remove_body")
end

local function check_errors()
    expect(not pcall(function()
        spawn(Mesh.cube(), "red", { body = "flying" })
    end), "bad body type errors")
    expect(not pcall(function()
        spawn(Mesh.cube(), "red", { body = { shape = "donut" } })
    end), "bad shape errors")
    expect(not pcall(function()
        spawn(Mesh.cube(), "red", { body = { wings = 2 } })
    end), "unknown body key errors")
    local plain = spawn(Mesh.cube(), "red", { position = vec3(0, -50, 0) })
    expect(not pcall(function()
        plain:impulse(vec3(1, 0, 0))
    end), "impulse without a body errors")
    plain:add_body({ shape = "sphere", radius = 0.3 })
    expect(plain.body == "dynamic", "add_body")
    plain:destroy()
end

local function check_sound()
    expect(not pcall(function()
        sound.play("missing.wav")
    end), "missing sound file errors")
    expect(not pcall(function()
        sound.play("physics.luau", { loudness = 2 })
    end), "unknown play option errors")
    expect(sound.enabled() == false, "no audio backend in tests")
    expect(sound.volume == 1, "master volume default")
    sound.volume = 0.5
    expect(sound.volume == 0.5, "master volume set")
    -- Any existing file is accepted by the API; without a backend it never plays.
    local s = sound.play("physics.luau", { volume = 0.5, pitch = 1.2, loop = true })
    s:set_volume(0.2, 0.5)
    s:stop(0.1)
    expect(s.playing == false, "muted sound is not playing")
    sound.stop_all()
end

function start()
    check_landing()
    check_velocity_and_impulse()
    check_sensor()
    check_kinematic_and_gravity()
    check_errors()
    check_sound()
    print("physics contract ok")
    quit()
end
