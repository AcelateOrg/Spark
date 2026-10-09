-- Starfall: a tiny 2D game made only with draw.* (no image files).
-- A/D or arrows = move, hold left mouse = follow the mouse, R = restart, Esc = quit.
-- Gamepad: left stick / d-pad = move, A or Start = restart, Back = quit.

local player = { x = 640, y = 650, w = 130, speed = 760 }
local items = {}
local particles = {}
local sky = {}
local score, best, lives = 0, 0, 3
local state = "play" -- "play" | "over"
local shake = 0
local flash = 0
local spawner

local function spawn_item()
    local bomb = math.random() < 0.22
    table.insert(items, {
        x = math.random(40, math.floor(screen.width) - 40),
        y = -30,
        vy = math.random(200, 330) + score * 6,
        bomb = bomb,
        r = if bomb then 18 else 20,
        spin = math.random() * 6,
    })
end

local function burst(x, y, color, n)
    for _ = 1, n do
        local a = math.random() * math.pi * 2
        local s = math.random(120, 420)
        table.insert(particles, { x = x, y = y, vx = math.cos(a) * s, vy = math.sin(a) * s - 150, life = 0.8, color = color })
    end
end

local function reset()
    items, particles = {}, {}
    score, lives, state = 0, 3, "play"
    player.x = screen.width / 2
    if spawner then
        spawner:cancel()
    end
    spawner = every(0.45, spawn_item)
end

-- 5-pointed star as a fan: center first, outline closed.
local function star_points(x, y, r, rot)
    local pts = { x, y }
    for i = 0, 10 do
        local rr = if i % 2 == 0 then r else r * 0.45
        local a = rot + i * math.pi / 5 - math.pi / 2
        table.insert(pts, x + math.cos(a) * rr)
        table.insert(pts, y + math.sin(a) * rr)
    end
    return pts
end

function start()
    background("#120f24")
    for i = 1, 90 do
        sky[i] = { x = math.random() * 2000, y = math.random() * 720, s = math.random(1, 3) }
    end
    on("key_pressed", function(key)
        if key == "escape" then
            quit()
        elseif key == "r" and state == "over" then
            reset()
        end
    end)
    reset()
end

function update(dt)
    if input.pad_pressed("back") then quit() end
    if state == "over" and (input.pad_pressed("a") or input.pad_pressed("start")) then reset() end
    shake = math.max(0, shake - dt * 30)
    flash = math.max(0, flash - dt * 2)
    for i = #particles, 1, -1 do
        local p = particles[i]
        p.x += p.vx * dt
        p.y += p.vy * dt
        p.vy += 700 * dt
        p.life -= dt
        if p.life <= 0 then
            table.remove(particles, i)
        end
    end
    if state ~= "play" then
        return
    end

    local move = 0
    if input.down("a") or input.down("left") then move -= 1 end
    if input.down("d") or input.down("right") then move += 1 end
    if input.pad_down("left") then move -= 1 end
    if input.pad_down("right") then move += 1 end
    if move == 0 then move = input.pad_axis("left_x") end -- analog: 0 inside the dead zone
    if move ~= 0 then
        player.x += move * player.speed * dt
    elseif input.mouse_down("left") then
        local mx = screen.mouse()
        player.x += (mx - player.x) * math.min(1, dt * 14)
    end
    player.x = math.clamp(player.x, player.w / 2, screen.width - player.w / 2)

    for i = #items, 1, -1 do
        local it = items[i]
        it.y += it.vy * dt
        it.spin += dt * 2
        local caught = it.y > player.y - 24 and it.y < player.y + 12 and math.abs(it.x - player.x) < player.w / 2 + it.r * 0.6
        if caught then
            table.remove(items, i)
            if it.bomb then
                lives -= 1
                shake, flash = 14, 0.6
                burst(it.x, it.y, "#ff5a4a", 40)
                sound.play("sounds/hit.wav", { volume = 0.9, pitch = 0.6, bus = "sfx" })
                if lives <= 0 then
                    state = "over"
                    best = math.max(best, score)
                    spawner:cancel()
                end
            else
                score += 1
                burst(it.x, it.y, "#ffd84a", 18)
                sound.play("sounds/spawn.wav", { volume = 0.5, pitch = 1 + math.min(score, 30) * 0.03, bus = "sfx" })
            end
        elseif it.y > screen.height + 40 then
            table.remove(items, i)
        end
    end
end

function render()
    local W, H = screen.width, screen.height

    -- Parallax starfield.
    for _, s in sky do
        local x = (s.x - time.elapsed * 12 * s.s) % W
        draw.circle(x, s.y, s.s * 0.9, Color.rgba(1, 1, 1, 0.15 + 0.15 * s.s))
    end

    draw.push()
    if shake > 0 then
        draw.translate((math.random() - 0.5) * shake, (math.random() - 0.5) * shake)
    end

    for _, it in items do
        if it.bomb then
            draw.circle(it.x, it.y, it.r, "#2d2b3d")
            draw.circle(it.x, it.y, it.r, "#ff5a4a", { line = 3 })
            draw.line(it.x, it.y - it.r, it.x + 6, it.y - it.r - 10, "#c8a060", 3)
            if math.floor(time.elapsed * 8) % 2 == 0 then
                draw.circle(it.x + 6, it.y - it.r - 11, 4, "#ffcc33")
            end
        else
            draw.circle(it.x, it.y, it.r * 1.4, Color.rgba(1, 0.85, 0.3, 0.12))
            draw.polygon(star_points(it.x, it.y, it.r, it.spin), "#ffd84a")
        end
    end

    -- Player: glowing paddle with round ends.
    local px, py, pw = player.x, player.y, player.w
    draw.rect(px, py, pw + 24, 34, Color.rgba(0.35, 0.8, 1, 0.15), { align = "center" })
    draw.rect(px, py, pw - 16, 16, "#5ad1ff", { align = "center" })
    draw.circle(px - pw / 2 + 8, py, 8, "#5ad1ff")
    draw.circle(px + pw / 2 - 8, py, 8, "#5ad1ff")

    for _, p in particles do
        draw.circle(p.x, p.y, 3 + p.life * 3, Color.hex(p.color):with_alpha(math.clamp(p.life * 1.5, 0, 1)))
    end
    draw.pop()

    if flash > 0 then
        draw.rect(0, 0, W, H, Color.rgba(1, 0.2, 0.15, flash * 0.35))
    end

    -- HUD
    draw.text("Score: " .. score, 24, 18, { size = 38, shadow = true })
    draw.text("Best: " .. best, 26, 64, { size = 22, color = "#a8a4c8" })
    for i = 1, 3 do
        local x = W - 36 - (i - 1) * 42
        draw.circle(x, 40, 15, if i <= lives then "#ff4d6a" else "#3a3450")
    end
    draw.text("A/D or arrows - move  ·  hold LMB - follow the mouse  ·  Esc - quit", W / 2, H - 14,
        { size = 18, align = "bottom", color = "#7d7899" })

    if state == "over" then
        draw.rect(0, 0, W, H, Color.rgba(0.03, 0.02, 0.08, 0.7))
        draw.text("GAME OVER", W / 2, H / 2 - 30, { size = 76, align = "center", color = "#ff5a6a", shadow = true })
        draw.text("Score: " .. score .. "    Best: " .. best, W / 2, H / 2 + 40, { size = 30, align = "center" })
        draw.text("R - restart", W / 2, H / 2 + 90, { size = 24, align = "center", color = "#a8a4c8" })
    end
end
