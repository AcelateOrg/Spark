# Spark Engine - Luau API reference

This file is written for humans **and** AI assistants: it is the complete list of what a Spark script can do.
If something is not listed here, it does not exist.

## Running

```
spark new path/to/game        # creates a new game in the recommended layout (see "Game structure")
spark docs path/to/game       # (re)writes this reference into the game: docs/SPARK_API.md (+ AGENTS.md if missing)
spark path/to/game            # runs path/to/game/main.luau in a window, hot-reloads on save
spark game --headless --frames 120 --screenshot shot.png --dump-scene   # no window: check result as PNG + text
```

Flags: `--headless`, `--frames N`, `--screenshot PATH`, `--dump-scene`, `--size WxH`, `--fixed-dt S`,
`--no-vsync`, `--splash` / `--no-splash`, `--fullscreen`, `--windowed`, `--help`.
In a window F12 saves a screenshot to `screenshots/`, F11 or Alt+Enter toggles fullscreen.

### game.toml (optional, next to main.luau)

```toml
title = "My Game"        # window title, save folder name, build name (default: folder name)
width = 1280             # window size
height = 720
fullscreen = false       # start in borderless fullscreen
icon = "icon.png"        # window / taskbar icon
vsync = true
show_fps = false         # fps in the title bar (default: only when there is no title)
save_name = "MyGame"     # folder for save data (default: title)
build_name = "MyGame"    # exe / folder / zip name of `spark build` (default: title)
exclude = ["notes"]      # extra files / folders `spark build` leaves out
splash = false           # show the "POWERED BY SPARK" splash before start() (default: off)
```

Unknown keys are errors (typos do not pass silently).

### Sharing a game: spark build

```
spark build path/to/game [--out dist] [--loose] [--no-zip]
```

Default: **one executable**, `dist/<Name>.exe` (+ `dist/<Name>.zip`). It is spark without a console window
with every game file (scripts and modules, textures, sounds, models, shaders, fonts, game.toml) packed inside.
Nothing has to lie next to it; players just double-click it. Packed games don't hot-reload (the files can't change).
Files next to the exe are still found when the pack doesn't have them (e.g. mods).
`--loose`: the old layout - the exe plus the game files in a folder.

Left out: `tools/`, `dist/`, `screenshots/`, hidden files, `*.log`, `*.blend`, `*.psd`, ... and everything in
`exclude`. Executables use the static C runtime (no VC++ redistributable needed). Packing is not protection:
the files can be extracted from the exe.

### Errors

A script error pauses the game and shows a red error screen with the message and line (also written to
`error.log` next to `main.luau`); save the file to reload. Engine crashes write `spark-crash.log` next to the
executable and show a message box when there is no console.

Splash (opt-in: `splash = true` in game.toml or `--splash`): "POWERED BY SPARK" on a dark screen (~4 s; any key / click
after the logo has faded in skips the pause), then `start()`. `--no-splash` turns it off for one run.
In headless mode the first script error exits with code 1 and prints file:line + a stack trace.

## Conventions

- Y is up, right-handed, 1 unit = 1 meter. Ground is the XZ plane at y = 0.
- Objects and the camera look along their local **-Z**.
- Angles are **radians** (`math.rad(90)`), except `camera.fov` (degrees).
- Rotation as a vector is Euler `vec3(pitch, yaw, roll)` = rotation around X, Y, Z (applied Y, then X, then Z).
- Colors are sRGB 0..1. Anywhere a color is expected you can pass: `"red"`, `"#ff8800"`, `"#f80"`, `0xff8800`,
  `Color.rgb(1, 0.5, 0)`, `vec3(1, 0.5, 0)` or `{1, 0.5, 0}`. Names: white black gray grey red green blue yellow orange purple pink brown sky.
- Vectors are native Luau vectors: `+ - * /` work (`v * 2`, `a + b`), components `v.x v.y v.z` are read-only
  (create a new one instead: `obj.position = vec3(obj.x, 5, obj.z)` or `obj.y = 5`).

## Script structure

```lua
function start()        -- called once (and again after every hot reload); build the scene here
end

function update(dt)     -- called every frame; dt = seconds since last frame
end
```

On reload the whole scene is cleared and `start()` runs again. Locals at file level are reset too.

## Game structure (modules)

A small game can live in one `main.luau`. Anything bigger should be split into modules - one module per system.
`spark new mygame` creates this layout (a small working game):

```text
mygame/
  main.luau        entry point: requires the modules and wires start/update/render to them; keep it short
  game.toml        title, window, build name
  src/             game code: player.luau, level.luau, hud.luau, ui/menu.luau, ...
  assets/          textures/ sounds/ models/ shaders/ fonts/
  AGENTS.md        rules for AI coding agents working on the game
  docs/            this reference (SPARK_API.md) and the shader guide, for this engine version
```

```lua
-- src/player.luau
local Player = {}
Player.__index = Player

function Player.new(pos)
    return setmetatable({ body = spawn(Mesh.cube(), "red", { position = pos }) }, Player)
end

function Player:update(dt) ... end

return Player

-- main.luau
local Player = require("./src/player")
local player
function start() player = Player.new(vec3(0, 1, 0)) end
function update(dt) player:update(dt) end
```

- `require("./x")` - next to the calling file; `require("../x")` - one folder up; `require("@game/x")` - from the
  game folder (where main.luau is). `.luau` is added; `require("./src/ui")` also finds `src/ui/init.luau`.
  A bare name (`require("player")`) is an error with a hint.
- A module runs **once**; its return value is cached and shared by every `require` of the same file.
- Saving **any** required file reloads the whole game (like saving main.luau).
- Errors name the module file and line: `src/ui/hud.luau:42: ...`.
- Cycles (`a` requires `b` requires `a` while loading) are an error that shows the chain
  (`src/a.luau -> src/b.luau -> src/a.luau`): move the shared part into a third module or require inside a function.
- Asset paths (`"assets/textures/wall.png"`) are always relative to the game folder, not to the module.

Rules that keep games easy to change:

- One module = one system (player, world, enemies, ui/menu, audio). It returns a table; no globals.
- Pass shared state explicitly (`Hud.draw(player)`, `Enemy.new(world)`) or put it in a `src/state.luau` module.
- `main.luau` only defines the callbacks (`start`, `update`, `render`, ...) and calls into modules.
- If main.luau is longer than ~800 lines and requires nothing, the engine logs a one-time hint to split it.

Optional callbacks: `fixed_update(dt)` (constant 1/60 s steps, for physics / deterministic logic) and
`late_update(dt)` (after everything else, e.g. cameras that follow) and `render()` (2D drawing with `draw.*`,
see "2D drawing"; the name `draw` is reserved for the drawing table).

### Frame order (always exactly this)

1. input events: `key_pressed`, `key_released`, `mouse_pressed`, `mouse_released`, `wheel`
2. `fixed_update(dt)` 0..N times (N depends on frame time; max 8)
3. `update(dt)`
4. timers and tasks that are due (`after`, `every`, `wait`, `tween`, ...)
5. events sent with `emit_later`
6. model animations advance (bones posed, skinned meshes deformed)
7. `late_update(dt)` (bone objects already have this frame's pose: attach cameras / props here)
8. `render()` (2D draw calls; nothing is kept between frames)
9. GPU render: 3D scene -> 2D `scene` layer -> post effects -> 2D `ui` layer

`start()` runs as a task, so it may use `wait()`.

## Scheduler (timers and tasks)

All times are game seconds: they follow `time.scale` (0 = everything paused). Results do not depend on frame rate.

| Function | |
|---|---|
| `after(seconds, fn, owner?)` -> Timer | run `fn()` once |
| `every(seconds, fn, opts?)` -> Timer | run `fn(n)` repeatedly, `n` = 1, 2, 3...; opts: `{ times = 3, owner = obj }` or just a number of times |
| `next_frame(fn, owner?)`, `after_frames(n, fn, owner?)` -> Timer | frame based |
| `timer:cancel()`, `timer:pause()`, `timer:resume()`, `timer.active`, `timer.paused`, `timer.remaining` | |
| `task(fn, ...)` -> Task | runs `fn(...)` now as a coroutine until its first `wait*` |
| `wait(seconds)` | pauses the task, returns the real elapsed time |
| `wait_frames(n=1)` | |
| `wait_until(fn, timeout?)` | checks `fn()` every frame; returns `true`, or `nil` on timeout |
| `wait_event(name, timeout?)` | returns the event arguments (`true` if it had none), or `nil` on timeout |
| `t:cancel()`, `t.status` (`running waiting done cancelled failed`), `t.alive` | |

Rules:
- Timers due in the same frame run by due time, ties in the order they were created; frame timers after time timers.
- A timer created inside a timer/handler never runs in the same frame (no freezes with `after(0, ...)`).
- `every` catches up: if a frame took 0.5 s, `every(0.1)` fires 5 times in that frame.
- `owner` = an Object: when it is destroyed, the timer is cancelled automatically.
- `wait*` only work inside `task(fn)` or `start()`. Timer callbacks and event handlers are plain functions: start a `task` there if you need to wait.
- An error in a timer, handler or task stops the script and shows file:line (headless: exit code 1).

```lua
task(function()
    for i = 3, 1, -1 do
        print(i)
        wait(1)
    end
    emit("go")
end)
```

## Events

| Function | |
|---|---|
| `on(name, fn, opts?)` -> Listener | opts: `{ priority = 0, owner = obj, once = false }` |
| `once(name, fn, opts?)` -> Listener | removed after the first call |
| `emit(name, ...)` -> consumed | calls listeners **now**; returns `true` if one consumed it |
| `emit_later(name, ...)` | delivered in step 5 of this frame (events sent during delivery wait for the next frame) |
| `off(listener)`, `listener:off()`, `off(name)` (all listeners of an event), `listener.active` | |
| `obj:on(name, fn, opts?)`, `obj:once(...)`, `obj:emit(name, ...)`, `obj:emit_later(...)` | events of one object; listeners die with the object |
| `obj:after(seconds, fn)`, `obj:every(seconds, fn, times?)` | timers owned by the object |

Rules:
- Listeners run by `priority` (higher first), ties in registration order.
- A handler that returns `true` consumes the event: lower listeners are skipped.
- A listener added during an emit does not get that emit; one removed during an emit is skipped.
- `once` runs exactly once, even with nested emits.
- An emit nested more than 64 levels deep is an error (a handler re-emitting its own event).
- Built-in events: `key_pressed(key)`, `key_released(key)`, `mouse_pressed(button, x, y)`, `mouse_released(button, x, y)`, `wheel(amount)`.

```lua
local enemy = spawn(Mesh.cube(), "red")
enemy:on("damage", function(amount)
    enemy.scale *= 0.8
    if enemy.scale.x < 0.3 then
        emit("enemy_died", enemy.position)
        enemy:destroy()
    end
end)
on("key_pressed", function(key)
    if key == "space" and enemy:exists() then enemy:emit("damage", 10) end
end)
```

## Math

| Function | Result |
|---|---|
| `vec3(x, y, z)` / `vec3(s)` / `vec3()` | vector (missing components = 0, one argument = all three) |
| `length(v)`, `normalize(v)` (zero-safe), `dot(a, b)`, `cross(a, b)`, `distance(a, b)` | |
| `lerp(a, b, t)` | numbers or vectors |
| Luau built-ins | `math.sin`, `math.clamp`, `math.random`, `math.rad`, `math.pi`, ... |

## Meshes, textures, materials, colors

Primitives are centered at the origin (a cube of size 1 spans -0.5..0.5). Same parameters = same shared mesh, so calling `Mesh.cube()` in a loop is cheap.

| Function | Notes |
|---|---|
| `Mesh.cube(size=1)` | |
| `Mesh.box(w=1, h=1, d=1)` | |
| `Mesh.plane(w=10, d=w)` | flat on XZ, facing up |
| `Mesh.quad(w=1, h=w)` | vertical on XY, facing +Z |
| `Mesh.box/plane/quad(..., { tile = 2, cell = 1 })` | optional last arg: `tile` = meters per texture repeat (world-scale UVs, no stretching on long walls), `cell` = subdivide faces into cells of this size (better vertex lighting, less PS1 warping) |
| `Mesh.sphere(radius=0.5, segments=24)` | |
| `Mesh.cylinder(radius=0.5, height=1, segments=24)` | |
| `Mesh.triangles({p1, p2, p3, ...})` | counter-clockwise triangles, flat normals |
| `Texture.load("img.png")` | PNG/JPEG, path relative to the script folder, cached |
| `Texture.checker(a="white", b="gray", cells=8, size=64)` | |
| `Texture.solid(color)` | |
| `Material.color(c)` | lit solid color |
| `Material.unlit(c)` | ignores lighting |
| `Material.texture(tex_or_path, tint="white")` | |
| `Material.checker(a, b, repeats=1)` | 2x2 checker repeated `repeats` times across the surface |
| `mat:tiling(x, y=x)`, `mat:with_color(c)`, `mat:with_unlit(true)` | return a modified copy |
| `Color.rgb(r,g,b)`, `Color.rgba(r,g,b,a)`, `Color.hex("#ff8800")`, `Color.lerp(a,b,t)` | |
| `Color.RED`, `Color.SKY`, ... | uppercase names of the color list above |
| `color.r .g .b .a`, `color:lerp(c, t)`, `color:with_alpha(a)` | |

## Objects

```lua
local box = spawn(Mesh.cube(), "orange", { name = "box", position = vec3(0, 0.5, 0) })
local pivot = group({ name = "pivot" })     -- empty object to parent things to
```

- `spawn(mesh, material_or_color_or_texture = white, props?)` -> Object
- `group(props?)` -> empty Object
- props: `name, position, rotation, scale (number or vector), visible, parent, color, body` (see Physics)
- `find(name)` -> first Object with that name or `nil`; `find_all(name)` -> list; `destroy(obj)` (also destroys children)

Object fields (read/write): `name`, `position`, `x`, `y`, `z`, `rotation` (euler vector), `scale`, `visible`,
`color`, `material`, `mesh`, `parent` (Object or nil). Read-only: `id`.
Positions/rotations are relative to the parent.

Object methods:

| Method | |
|---|---|
| `obj:rotate_x(a)` / `rotate_y` / `rotate_z` | rotate around own axis (radians) |
| `obj:translate(v)` | position += v |
| `obj:look_at(target)` | target = world vector or Object; turns -Z towards it |
| `obj:place_on(other)` | moves obj vertically so its bottom rests on top of `other` |
| `obj:set_parent(p_or_nil)`, `obj:children()` | |
| `obj:world_position()`, `obj:forward()`, `obj:right()`, `obj:up()` | |
| `obj:bounds()` | world min, max vectors (nil, nil without mesh) |
| `obj:size()` | world-space size vector |
| `obj:distance_to(obj_or_vec)` | |
| `obj:exists()`, `obj:destroy()` | using a destroyed object raises an error |
| `a == b`, `tostring(obj)` | |

## 3D models and animation (glTF)

```lua
local hero = spawn(Model.load("models/hero.glb"), { position = vec3(0, 0, 0) })
hero:play("walk")                                   -- loops, cross-fades from the previous clip (0.2 s)
hero:play("attack", { loop = false, fade = 0.1 })
wait_until(function() return not hero:is_playing("attack") end)
spawn(Mesh.box(0.1, 0.1, 0.6), "gray", { parent = hero:find("hand.R") })   -- sword follows the hand bone
```

- Formats: `.glb` (recommended) and `.gltf` (+ .bin / images next to it). Export from Blender: File > Export > glTF 2.0,
  "+Y up", forward = -Z for Spark (models face -Z like every Spark object; `obj:look_at` turns -Z to the target).
- Supported: meshes (triangles), node hierarchy, base color factor + base color texture, `KHR_materials_unlit`,
  skins (up to 4 bones per vertex, CPU skinning), animations of translation / rotation / scale (step, linear, cubic).
  Not (yet): morph targets, PBR maps (normal / metal-rough are ignored), cameras / lights in the file.
- `Model.load(path)` -> Model (cached by path). `model.name`, `model.animations` (list of clip names), `model.size`
  (bind-pose size vector), `model:duration(name)`.
- `spawn(model_or_path, material?, props?)` -> root Object (empty, named after the file); every glTF node becomes a
  child object with the node's name, so bones can be found, moved and used as parents. `spawn("models/x.glb", props)`
  loads and spawns in one go. A `material` replaces every part's material.
- `obj:play(name, { loop = true, speed = 1, fade = 0.2, restart = false })` - starts / cross-fades to a clip.
  Playing the clip that is already playing only changes its options (use `restart = true` to start over).
  A finished one-shot clip (`loop = false`) holds its last frame.
- `obj:stop(fade = 0.2)`, `obj:is_playing(name?)` (finished one-shot = false), `obj:animation()` -> current clip name or nil,
  `obj:animation_time()` -> seconds, `obj:set_animation_speed(s)` (multiplies every clip; 0 freezes the pose), `obj:animations()` -> names
- `obj:find(name)` -> first descendant with that name (any depth) or nil
- `obj:paint(material_or_color)` - sets the material of the object and all its descendants that have a mesh
- Without playing anything a model shows the first frame of its first clip.

## Tweens

```lua
tween(door, { rotation = vec3(0, math.rad(90), 0) }, 0.8, "out_back")
tween(camera, { fov = 40 }, 0.3)
tween(lamp, { color = "#ff3010" }, 1, { ease = "in_out_sine", loop = true, yoyo = true })
local t = tween(ui, { alpha = 1 }, 0.5, { delay = 1, done = function() print("shown") end })
t:cancel()
```

- `tween(target, goals, seconds, ease_or_options)` -> handle `{ active, finished, cancel() }`. `target` is an Object, `camera`,
  a light ref or any table; goals are numbers, vectors or colors (fields are read when the tween starts, after `delay`).
- options: `ease` (default `"out_quad"`), `delay`, `loop`, `yoyo` (there and back), `unscaled` (ignores `time.scale`, for pause menus),
  `done` (called when finished, not when cancelled), `owner` (stop when this object is destroyed). Destroyed targets stop the tween.
- eases: `linear in out in_out smooth in_quad out_quad in_out_quad in_cubic out_cubic in_out_cubic in_sine out_sine in_out_sine
  in_expo out_expo in_back out_back out_elastic in_bounce out_bounce`
- `lerp(a, b, t)` works on numbers, vectors and colors.

## Camera, lights

- `camera.position`, `camera.x/y/z`, `camera.rotation` (euler), `camera.fov` (degrees, default 60), `camera.near`, `camera.far`
- `camera:look_at(vec_or_obj)`, `camera:translate(v)`, `camera:forward()`, `camera:right()`, `camera:up()`
- Default camera: position (0, 3, 8) looking at the origin.
- `sun(direction, color?, intensity?)` - direction the light travels, e.g. `vec3(-0.5, -1, -0.3)`
- `ambient(color)`, `background(color)`, `fog(color, near=10, far=60)`, `fog(nil)` to disable
- Point / spot lights (up to 32 nearest visible lights per frame; any object can carry one):
  - `light({ type = "point", color = "#ffd9a0", intensity = 1.5, range = 8, position = vec3(0, 3, 0) })` -> Object (invisible, only light)
  - `light({ type = "spot", angle = 30, softness = 0.3, ... })` - cone along the object's forward (-Z); aim with `rotation` or `obj:look_at(...)`
  - `spawn(mesh, mat, { light = { type = "point", ... } })` - glowing lamp mesh with its light
  - `obj.light` -> light ref (`.type .color .intensity .range .angle .softness`, read/write); `obj.light = {...}` / `obj.light = nil`
  - Flicker: change `obj.light.intensity` in `update`; flashlight: spot light that follows `camera.position/rotation`

## Render settings and shaders

Spark has no built-in looks or effects: the default is plain lit rendering, everything else is your own WGSL.
Full shader reference with examples: [SHADERS.md](SHADERS.md).

```lua
graphics.height = 240                -- render the 3D scene 240 px tall (width keeps the aspect), nil = native
graphics.upscale = "nearest"         -- how the small image is stretched to the window: "linear" (default) | "nearest"
graphics.filter = "nearest"          -- texture sampling: "linear" (default) | "nearest"

local toon = Shader.load("shaders/toon.wgsl")    -- surface shader: has `fn fragment(...)` and/or `fn vertex(...)`
toon.bands = 4                                   -- fields of `struct Params` in the shader
graphics.shader = toon                             -- every material without its own shader
spawn(Mesh.cube(), Material.color("red"):with_shader(toon):with_data(1, 0, 0, 0))

local vhs = Shader.load("shaders/vhs.wgsl")      -- post shader: has `fn post(p: PostInput) -> vec4<f32>`
vhs:set({ grain = 0.4, tint = "#ffd0a0" })
graphics.post = { vhs }                            -- chain of full-screen passes, in order
```

- `graphics.height` (nil or pixels), `graphics.scale` (0.05..2, multiplies the internal resolution), `graphics.upscale`, `graphics.filter`
- `graphics.scene_width`, `graphics.scene_height` -> current size of the 3D image in pixels (read-only)
- `graphics.shader` = surface Shader or nil (default lit shader)
- `graphics.post` = Shader, list of Shaders / `{ shader = s, size = "screen" | "scene" }`, or nil. `size = "scene"` runs the pass at
  the internal resolution (cheap, pixel-exact retro effects), `"screen"` (default) at window resolution.
- `Shader.load(path)` -> Shader (path relative to the game folder; hot-reloads on save, compile errors shown on screen),
  `Shader.new(code, name?)` -> Shader from a string. Compile errors are Luau errors with the WGSL line.
- `shader.<param>` read/write: number / bool (f32, i32, u32), vector (vec2 / vec3), vector / Color / `{x, y, z, w}` (vec4).
  Colors are converted to linear. `shader:set({ a = 1, b = vec3(...) })` sets many. Unknown names error with the list of params.
- `shader.texture1`, `shader.texture2` = Texture / image path / nil (bound as `texture1` / `texture2` in WGSL)
- `shader.name`, `shader.kind` (`"surface"` | `"post"`), `shader.params` -> list of param names
- Per material / object: `mat:with_shader(s)`, `mat:with_data(a, b, c, d)`; `obj.shader`, `obj.data` (read/write);
  spawn props `shader = s`, `data = {a, b, c, d}`. `data` arrives in WGSL as `object.data` (per-object values, e.g. a hit flash).

## Physics

rapier 3D, stepped in `fixed_update` (1/60). Y up, meters, default gravity `(0, -9.81, 0)`.

```lua
local ground = spawn(Mesh.plane(40), "gray", { body = "static" })
local box = spawn(Mesh.cube(), "orange", { position = vec3(0, 4, 0), body = "dynamic" })
box:on("collision", function(other, info) if info.speed > 3 then sound.play("sounds/hit.wav") end end)
```

- `body = "dynamic" | "static" | "kinematic" | true` (dynamic), or a table:
  `{ type, shape, size, radius, height, mass, friction, bounciness, sensor, lock_rotation, gravity_scale, damping, angular_damping, ccd, velocity, angular_velocity }`
- `shape`: `auto` (default, fitted to the mesh), `box` (`size`), `sphere` (`radius`), `capsule` (`radius`, `height`), `cylinder`
- dynamic = simulated; static = never moves; kinematic = moved by your code (`obj.position`, `obj.y += ...`), pushes dynamic bodies
- Object fields: `body` (type name or nil; set to a type/table/nil), `velocity`, `angular_velocity`, `mass`, `gravity_scale`, `sensor`
- Object methods: `obj:add_body(spec?)`, `obj:remove_body()`, `obj:impulse(v)` (instant), `obj:force(v)` (this step), `obj:torque(v)`
- Setting `obj.position` on a body teleports it.
- Character controller: `local moved, grounded = obj:move(delta)` moves an object with a (usually kinematic capsule) body by `delta`
  (world space, this frame - multiply by `dt`), sliding along walls, climbing steps up to 0.35 m and slopes up to 50 degrees,
  snapping down to the ground. `grounded` = standing on something afterwards. Gravity is up to you:
  ```lua
  hero:add_body({ type = "kinematic", shape = "capsule", radius = 0.3, height = 1.8 })
  function update(dt)
      vy = if grounded then (input.pressed("space") and 5 or -1) else vy - 9.81 * dt
      _, grounded = hero:move((wish * 4 + vec3(0, vy, 0)) * dt)
  end
  ```
- Events: `obj:on("collision", fn(other, info))`, `obj:on("collision_end", fn(other, info))`, global `on("collision", fn(a, b, info))`.
  `info.speed` = impact speed, `info.sensor` = true for sensor (trigger) contacts. Sensors detect overlap without pushing.
- `raycast(origin, dir, max=1000, { ignore = obj })` -> `nil` or `{ object, point, normal, distance }`
  (hits physics colliders only)
- `pick(origin, dir, max=1000, { ignore = obj })` -> same result, but tests the **rendered triangles**
  of every visible mesh object - no bodies needed. Use it for "what am I looking at" (interaction,
  mouse picking): `pick(camera.position, camera:forward(), 2.5)`. `ignore` skips an object and all its
  children. `hit.object` is the exact mesh hit - walk `obj.parent` up to find your interactive root.
- `physics.gravity` (vector, read/write), `physics.enabled` (pause simulation)
- Errors on unknown body types, shapes or keys, and on `impulse`/`velocity` for objects without a body.

## Sound

kira. Paths are relative to the game folder. wav, ogg, mp3, flac.

- `sound.play(path, { volume=1, pitch=1, pan=0, loop=false, fade_in=0, bus })` -> Sound
- 3D sound: `sound.play(path, { at = vec3(...) | object, range = 20, loop = true })` - volume falls to 0 at `range`, panned relative to the camera, follows the object every frame; `s:set_position(vec_or_obj)`
- `s:stop(fade=0)`, `s:set_volume(v, fade=0)`, `s:set_pitch(v, fade=0)`, `s:set_pan(-1..1)`, `s.playing`
- `sound.volume` (master, read/write), `sound.stop_all(fade=0)`, `sound.enabled()` (false when no audio device, e.g. tests; calls then do nothing)
- Missing file or unknown option -> error with the looked-up path.

Mixer buses: groups of sounds with their own volume / pause / stop (music, sfx, ui, voice...). A bus is created
by name on first use; sounds without `bus` go straight to the master output (`sound.volume` affects everything).

- `sound.bus(name)` -> Bus; `sound.play(path, { bus = "music" })` (name or Bus)
- `b.volume` (read/write, instant), `b:set_volume(v, fade=0)`, `b.paused` (read/write), `b:pause(fade=0)`,
  `b:resume(fade=0)`, `b:stop(fade=0)` (stops every sound on the bus), `b.name`

```lua
-- layered music: all layers loop in sync, the mix follows the action
local music = sound.bus("music")
local calm = sound.play("music/calm.ogg", { loop = true, bus = music })
local drums = sound.play("music/drums.ogg", { loop = true, volume = 0, bus = music })
function on_danger(on) drums:set_volume(if on then 1 else 0, 1.5) end
-- pause menu: game sounds stop, the menu keeps its own bus
sound.bus("sfx"):pause(0.2)
```

## 2D drawing (draw, screen, Font)

Immediate mode: call `draw.*` inside `render()` every frame. Coordinates are virtual pixels: origin top-left,
Y down, the screen is `screen.height` = 720 units tall (settable), width follows the window aspect
(`screen.width`). Everything scales with the window. Later calls draw on top.
Colors: `Color` or `"#rrggbb"` / `"#rrggbbaa"` strings; default white.

- `draw.rect(x, y, w, h, color?, { align="top_left", line=width })` (`line` = outline only)
- `draw.circle(x, y, r, color?, { line })`, `draw.line(x1, y1, x2, y2, color?, width=2)`
- `draw.polygon({x1, y1, x2, y2, ...}, color?, { line })` (convex fill; any shape with `line`)
- `draw.sprite(tex_or_path, x, y, { w, h, scale, rotation, color, align="center", flip_x, flip_y,
  region={x, y, w, h} (pixels), grid={cols, rows}, frame=1 (1-based, wraps; use for animation), filter="nearest"|"linear" })`
- `draw.text(text, x, y, { size=24, color, align="top_left", font, width (wrap), line_height, shadow=true|Color })` -> w, h
- `draw.measure(text, { size, font, width })` -> w, h
- align: `top_left top top_right left center right bottom_left bottom bottom_right`
- transform stack: `draw.push()`, `draw.pop()`, `draw.translate(x, y)`, `draw.rotate(radians)`, `draw.scale(sx, sy?)`
- `draw.layer("ui"|"scene")` -> previous. `ui` (default): crisp, over everything. `scene`: inside the 3D image,
  gets the look's pixelation/post effects.
- `draw.filter("nearest"|"linear")` default sprite filter; `draw.stats()` -> triangle/batch counts
- `draw.spark_badge({ align="bottom_right", margin=18, width=140, alpha=0.55, x, y })` -> hovered, clicked:
  small "POWERED BY SPARK" logo (ui layer) for the main menu, in the screen corner given by `align` (or at `x, y`).
  Brightens on hover (hand cursor); a click opens https://acelate.com/spark. Only reacts while the mouse is not
  locked. `width`/`margin` are in units of a 720-tall screen. Ignore your own click when `hovered` is true.
- `Font.load(path)` -> font for `{ font = f }` (ttf/otf). Built-in font: Noto Sans (Latin, Cyrillic, Greek).
- `screen.width`, `screen.height` (read/write), `screen.scale` (real px per virtual px),
  `screen.pixel_width`, `screen.pixel_height`, `screen.mouse()` -> x, y in virtual pixels, `screen.center()` -> x, y

```lua
function render()
    draw.text("Score: " .. score, 20, 20, { size = 32, shadow = true })
    draw.rect(screen.width / 2, screen.height - 40, 200, 16, "#5ad1ff", { align = "center" })
end
```

Full 2D game: `examples/starfall/main.luau`.

## Input and time

- `input.down(key)`, `input.pressed(key)` (this frame), `input.released(key)`
- keys: `"a"`..`"z"`, `"0"`..`"9"`, `space enter escape tab backspace left right up down shift ctrl alt lshift rshift lctrl rctrl lalt ralt f1..f12` (`esc`, `return` also work)
- `input.mouse_down(btn="left")`, `input.mouse_pressed(btn)`, `input.mouse_released(btn)`; buttons `left right middle`
- `input.mouse()` -> x, y in pixels; `input.mouse_delta()` -> dx, dy; `input.wheel()` -> number
- `input.lock_mouse(on=true)` - hide and capture the cursor for FPS mouse look (`mouse_delta` = raw motion); released automatically when the window loses focus, re-locked on focus. `input.mouse_locked()` -> bool. Typical: lock on click, unlock on Escape.
- Gamepads (XInput / DirectInput / SDL mappings, hot-plug). `pad` = 1-based pad number; omit it to accept any pad.
  - `input.pad_down(button, pad?)`, `input.pad_pressed(button, pad?)`, `input.pad_released(button, pad?)`
  - buttons (Xbox names; PlayStation: a = cross, b = circle, x = square, y = triangle):
    `a b x y lb rb lt rt back start lstick rstick up down left right` (d-pad = `up down left right`)
  - `input.pad_axis(axis, pad?)` -> number; axes `left_x left_y right_x right_y` (-1..1, up = +1, dead zone applied),
    `lt rt` (0..1). `input.pad_stick("left"|"right", pad?)` -> x, y
  - `input.pads()` -> number connected, `input.pad_name(pad=1)` -> string or nil
- `input.any_pressed()` -> any key, mouse button or pad button this frame ("press any key")
- `time.dt`, `time.elapsed` (seconds), `time.frame`, `time.fps`, `time.unscaled_dt`, `time.unscaled_elapsed`
- `time.scale` (read/write: 1 normal, 0.5 slow motion, 0 pause), `time.fixed_dt` (read/write, default 1/60)

## Saving

Small persistent key/value storage, saved instantly to `%APPDATA%/SparkGames/<title>/save.json`
(Linux / macOS: `~/.local/share/SparkGames/<title>/`). Headless runs keep it in memory only.

- `save.set(key, value)` - numbers, strings, booleans, vectors and (nested) tables of them; `nil` deletes
- `save.get(key, default)` -> value or `default`; `save.has(key)`, `save.delete(key)`, `save.clear()`, `save.keys()`, `save.path()`

```lua
local best = save.get("best_time", 0)
save.set("settings", { volume = 0.8, fov = 75 })
```

## Window

- `window.fullscreen` (read/write, borderless), `window.title` (read/write; nil = game title), `window.width`, `window.height` (pixels, read-only)

## Misc

- `print(...)` -> engine log `[script] ...`
- `dump_scene()` -> text with every object, position, rotation, bounds (same as `--dump-scene`)
- `quit()`

## Example

```lua
local player

function start()
    background("sky")
    fog("sky", 20, 60)
    local ground = spawn(Mesh.plane(50), Material.checker("#6fae4f", "#5d9a42", 25))
    player = spawn(Mesh.cube(), "orange", { name = "player" })
    player:place_on(ground)
end

function update(dt)
    local move = vec3(0, 0, 0)
    if input.down("w") then move += vec3(0, 0, -1) end
    if input.down("s") then move += vec3(0, 0, 1) end
    if input.down("a") then move += vec3(-1, 0, 0) end
    if input.down("d") then move += vec3(1, 0, 0) end
    player.position += normalize(move) * 5 * dt
    camera.position = player.position + vec3(0, 4, 7)
    camera:look_at(player)
end
```
