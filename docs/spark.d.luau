-- Spark Engine - Luau type definitions for luau-lsp (autocomplete + type checking in VS Code / Zed / Neovim).
-- Generated for the API in docs/LUAU_API.md (in a game: docs/SPARK_API.md). Do not edit inside a game:
-- `spark docs .` overwrites it with the version that matches the engine.
--
-- VS Code: install "Luau Language Server" (JohnnyMorganz.luau-lsp); `spark new` writes .vscode/settings.json:
--   "luau-lsp.types.definitionFiles": ["docs/spark.d.luau"], "luau-lsp.platform.type": "standard"
-- Runtime check without an editor: `spark check .` (add --json for a machine-readable report).

-- ===================================================================== shared types

-- Keys accepted by input.down / pressed / released / repeated and sent by key_pressed / key_released
-- (physical US-layout positions). Aliases: shift ctrl alt super esc return - = [ ] \ ; ' , . / ` del ins pgup pgdn ...
type Key =
    "a" | "b" | "c" | "d" | "e" | "f" | "g" | "h" | "i" | "j" | "k" | "l" | "m"
    | "n" | "o" | "p" | "q" | "r" | "s" | "t" | "u" | "v" | "w" | "x" | "y" | "z"
    | "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
    | "space" | "enter" | "escape" | "tab" | "backspace" | "left" | "right" | "up" | "down"
    | "lshift" | "rshift" | "lctrl" | "rctrl" | "lalt" | "ralt" | "lsuper" | "rsuper"
    | "shift" | "ctrl" | "control" | "alt" | "super" | "meta" | "cmd" | "win" | "esc" | "return"
    | "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12"
    | "f13" | "f14" | "f15" | "f16" | "f17" | "f18" | "f19" | "f20" | "f21" | "f22" | "f23" | "f24"
    | "minus" | "equal" | "bracketleft" | "bracketright" | "backslash" | "semicolon" | "quote"
    | "comma" | "period" | "slash" | "backquote"
    | "-" | "=" | "[" | "]" | "\\" | ";" | "'" | "," | "." | "/" | "`"
    | "insert" | "delete" | "home" | "end" | "pageup" | "pagedown" | "del" | "ins" | "pgup" | "pgdn"
    | "capslock" | "numlock" | "scrolllock" | "printscreen" | "pause" | "menu"
    | "numpad0" | "numpad1" | "numpad2" | "numpad3" | "numpad4" | "numpad5" | "numpad6" | "numpad7" | "numpad8" | "numpad9"
    | "numpad_add" | "numpad_subtract" | "numpad_multiply" | "numpad_divide" | "numpad_decimal"

type MouseButton = "left" | "right" | "middle" | "back" | "forward"

type PadButton =
    "a" | "b" | "x" | "y" | "lb" | "rb" | "lt" | "rt" | "back" | "start" | "lstick" | "rstick"
    | "up" | "down" | "left" | "right"

type PadAxis = "left_x" | "left_y" | "right_x" | "right_y" | "lt" | "rt"

-- "red", "#ff8800", "#f80", "#rrggbbaa", 0xff8800, Color.rgb(...), vec3(r, g, b) or {r, g, b}
type ColorLike = string | number | Color | vector | { number }

type Ease =
    "linear" | "in" | "out" | "in_out" | "smooth" | "in_quad" | "out_quad" | "in_out_quad"
    | "in_cubic" | "out_cubic" | "in_out_cubic" | "in_sine" | "out_sine" | "in_out_sine"
    | "in_expo" | "out_expo" | "in_back" | "out_back" | "out_elastic" | "in_bounce" | "out_bounce"

type Align =
    "top_left" | "top" | "top_right" | "left" | "center" | "right" | "bottom_left" | "bottom" | "bottom_right"

type BodyType = "dynamic" | "static" | "kinematic"

type BodySpec = {
    type: BodyType?,
    shape: ("auto" | "box" | "sphere" | "capsule" | "cylinder")?,
    size: vector?,
    radius: number?,
    height: number?,
    mass: number?,
    friction: number?,
    bounciness: number?,
    sensor: boolean?,
    lock_rotation: boolean?,
    gravity_scale: number?,
    damping: number?,
    angular_damping: number?,
    ccd: boolean?,
    velocity: vector?,
    angular_velocity: vector?,
}

type LightSpec = {
    type: ("point" | "spot")?,
    color: ColorLike?,
    intensity: number?,
    range: number?,
    angle: number?,
    softness: number?,
    position: vector?,
    rotation: vector?,
    parent: Object?,
    name: string?,
}

type SpawnProps = {
    name: string?,
    position: vector?,
    rotation: vector?,
    scale: (number | vector)?,
    visible: boolean?,
    parent: Object?,
    color: ColorLike?,
    body: (BodyType | boolean | BodySpec)?,
    light: LightSpec?,
    shader: Shader?,
    data: { number }?,
}

type Hit = { object: Object, point: vector, normal: vector, distance: number }
type CollisionInfo = { speed: number, sensor: boolean }
type ListenerOptions = { priority: number?, owner: Object?, once: boolean? }
type TweenOptions = {
    ease: Ease?,
    delay: number?,
    loop: boolean?,
    yoyo: boolean?,
    unscaled: boolean?,
    done: (() -> ())?,
    owner: Object?,
}
type TextOptions = {
    size: number?,
    color: ColorLike?,
    align: Align?,
    font: Font?,
    width: number?,
    line_height: number?,
    shadow: (boolean | ColorLike)?,
}
type SoundOptions = {
    volume: number?,
    pitch: number?,
    pan: number?,
    loop: boolean?,
    fade_in: number?,
    bus: (string | Bus)?,
    at: (vector | Object)?,
    range: number?,
}
type MeshOptions = { tile: number?, cell: number? }
type PlayOptions = { loop: boolean?, speed: number?, fade: number?, restart: boolean? }

-- ===================================================================== classes

declare extern type Color with
    r: number
    g: number
    b: number
    a: number
    function lerp(self, other: ColorLike, t: number): Color
    function with_alpha(self, a: number): Color
end

declare extern type Mesh with
end

declare extern type Texture with
end

declare extern type Material with
    function tiling(self, x: number, y: number?): Material
    function with_color(self, color: ColorLike): Material
    function with_unlit(self, on: boolean?): Material
    function with_shader(self, shader: Shader?): Material
    function with_data(self, a: number, b: number?, c: number?, d: number?): Material
end

declare extern type Model with
    name: string
    animations: { string }
    size: vector
    function duration(self, clip: string): number
end

declare extern type Shader with
    name: string
    kind: "surface" | "post"
    params: { string }
    texture1: (Texture | string)?
    texture2: (Texture | string)?
    -- `shader.<param>` reads / writes a field of `struct Params` in the WGSL code (any name).
    [string]: any
    function set(self, values: { [string]: any }): ()
end

declare extern type Font with
end

declare extern type Timer with
    active: boolean
    paused: boolean
    remaining: number
    function cancel(self): ()
    function pause(self): ()
    function resume(self): ()
end

declare extern type Task with
    status: "running" | "waiting" | "done" | "cancelled" | "failed"
    alive: boolean
    function cancel(self): ()
end

declare extern type Listener with
    active: boolean
    function off(self): ()
end

declare extern type Tween with
    active: boolean
    finished: boolean
    function cancel(self): ()
end

declare extern type Sound with
    playing: boolean
    function stop(self, fade: number?): ()
    function set_volume(self, volume: number, fade: number?): ()
    function set_pitch(self, pitch: number, fade: number?): ()
    function set_pan(self, pan: number): ()
    function set_position(self, at: vector | Object): ()
end

declare extern type Bus with
    name: string
    volume: number
    paused: boolean
    function set_volume(self, volume: number, fade: number?): ()
    function pause(self, fade: number?): ()
    function resume(self, fade: number?): ()
    function stop(self, fade: number?): ()
end

declare extern type LightRef with
    type: "point" | "spot"
    color: Color
    intensity: number
    range: number
    angle: number
    softness: number
end

declare extern type Object with
    id: number
    name: string
    position: vector
    x: number
    y: number
    z: number
    rotation: vector
    scale: vector
    visible: boolean
    color: Color
    material: Material?
    mesh: Mesh?
    parent: Object?
    light: LightRef?
    shader: Shader?
    data: { number }
    -- physics (see "Physics")
    body: BodyType?
    velocity: vector
    angular_velocity: vector
    mass: number
    gravity_scale: number
    sensor: boolean

    function rotate_x(self, angle: number): ()
    function rotate_y(self, angle: number): ()
    function rotate_z(self, angle: number): ()
    function translate(self, delta: vector): ()
    function look_at(self, target: vector | Object): ()
    function place_on(self, other: Object): ()
    function set_parent(self, parent: Object?): ()
    function children(self): { Object }
    function world_position(self): vector
    function forward(self): vector
    function right(self): vector
    function up(self): vector
    function bounds(self): (vector?, vector?)
    function size(self): vector
    function distance_to(self, other: Object | vector): number
    function exists(self): boolean
    function destroy(self): ()
    function find(self, name: string): Object?
    function paint(self, material: Material | ColorLike): ()

    -- events and timers owned by the object
    function on(self, name: string, fn: (...any) -> any, opts: ListenerOptions?): Listener
    function once(self, name: string, fn: (...any) -> any, opts: ListenerOptions?): Listener
    function emit(self, name: string, ...: any): boolean
    function emit_later(self, name: string, ...: any): ()
    function after(self, seconds: number, fn: () -> ()): Timer
    function every(self, seconds: number, fn: (n: number) -> (), times: number?): Timer

    -- animation (glTF models)
    function play(self, clip: string, opts: PlayOptions?): ()
    function stop(self, fade: number?): ()
    function is_playing(self, clip: string?): boolean
    function animation(self): string?
    function animation_time(self): number
    function set_animation_speed(self, speed: number): ()
    function animations(self): { string }

    -- physics
    function add_body(self, spec: (BodyType | boolean | BodySpec)?): ()
    function remove_body(self): ()
    function impulse(self, v: vector): ()
    function force(self, v: vector): ()
    function torque(self, v: vector): ()
    function move(self, delta: vector): (vector, boolean)
end

declare extern type Camera with
    position: vector
    x: number
    y: number
    z: number
    rotation: vector
    fov: number
    near: number
    far: number
    function look_at(self, target: vector | Object): ()
    function translate(self, delta: vector): ()
    function forward(self): vector
    function right(self): vector
    function up(self): vector
end

-- ===================================================================== math

declare function vec3(x: number?, y: number?, z: number?): vector
declare function length(v: vector): number
declare function normalize(v: vector): vector
declare function dot(a: vector, b: vector): number
declare function cross(a: vector, b: vector): vector
declare function distance(a: vector, b: vector): number
declare function lerp<T>(a: T, b: T, t: number): T

-- ===================================================================== resources

declare Mesh: {
    cube: (size: number?) -> Mesh,
    box: (w: number?, h: number?, d: number?, opts: MeshOptions?) -> Mesh,
    plane: (w: number?, d: number?, opts: MeshOptions?) -> Mesh,
    quad: (w: number?, h: number?, opts: MeshOptions?) -> Mesh,
    sphere: (radius: number?, segments: number?) -> Mesh,
    cylinder: (radius: number?, height: number?, segments: number?) -> Mesh,
    triangles: (points: { vector }) -> Mesh,
}

declare Texture: {
    load: (path: string) -> Texture,
    checker: (a: ColorLike?, b: ColorLike?, cells: number?, size: number?) -> Texture,
    solid: (color: ColorLike) -> Texture,
}

declare Material: {
    color: (color: ColorLike) -> Material,
    unlit: (color: ColorLike) -> Material,
    texture: (texture: Texture | string, tint: ColorLike?) -> Material,
    checker: (a: ColorLike, b: ColorLike, repeats: number?) -> Material,
}

declare Color: {
    rgb: (r: number, g: number, b: number) -> Color,
    rgba: (r: number, g: number, b: number, a: number) -> Color,
    hex: (hex: string) -> Color,
    lerp: (a: ColorLike, b: ColorLike, t: number) -> Color,
    WHITE: Color, BLACK: Color, GRAY: Color, GREY: Color, RED: Color, GREEN: Color, BLUE: Color,
    YELLOW: Color, ORANGE: Color, PURPLE: Color, PINK: Color, BROWN: Color, SKY: Color,
}

declare Model: {
    load: (path: string) -> Model,
}

declare Shader: {
    load: (path: string) -> Shader,
    new: (code: string, name: string?) -> Shader,
}

declare Font: {
    load: (path: string) -> Font,
}

-- ===================================================================== objects and scene

declare function spawn(what: Mesh | Model | string, material: (Material | Texture | ColorLike | SpawnProps)?, props: SpawnProps?): Object
declare function group(props: SpawnProps?): Object
declare function light(spec: LightSpec): Object
declare function find(name: string): Object?
declare function find_all(name: string): { Object }
declare function destroy(obj: Object): ()
declare function dump_scene(): string

declare camera: Camera
declare function sun(direction: vector, color: ColorLike?, intensity: number?): ()
declare function ambient(color: ColorLike): ()
declare function background(color: ColorLike): ()
declare function fog(color: ColorLike?, near: number?, far: number?): ()

declare graphics: {
    height: number?,
    scale: number,
    upscale: "linear" | "nearest",
    filter: "linear" | "nearest",
    scene_width: number,
    scene_height: number,
    shader: Shader?,
    -- Shader, list of Shaders / { shader = s, size = "screen" | "scene" }, or nil
    post: any,
}

-- ===================================================================== scheduler, events, tweens

declare function after(seconds: number, fn: () -> (), owner: Object?): Timer
declare function every(seconds: number, fn: (n: number) -> (), opts: (number | { times: number?, owner: Object? })?): Timer
declare function next_frame(fn: () -> (), owner: Object?): Timer
declare function after_frames(n: number, fn: () -> (), owner: Object?): Timer
declare function task(fn: (...any) -> (), ...: any): Task
declare function wait(seconds: number?): number
declare function wait_frames(n: number?): ()
declare function wait_until(condition: () -> boolean, timeout: number?): boolean?
declare function wait_event(name: string, timeout: number?): ...any

declare function on(name: string, fn: (...any) -> any, opts: ListenerOptions?): Listener
declare function once(name: string, fn: (...any) -> any, opts: ListenerOptions?): Listener
declare function emit(name: string, ...: any): boolean
declare function emit_later(name: string, ...: any): ()
declare function off(listener: Listener | string): ()

declare function tween(target: any, goals: { [string]: any }, seconds: number, ease: (Ease | TweenOptions)?): Tween

-- ===================================================================== physics

declare physics: {
    gravity: vector,
    enabled: boolean,
}
declare function raycast(origin: vector, dir: vector, max: number?, opts: { ignore: Object? }?): Hit?
declare function pick(origin: vector, dir: vector, max: number?, opts: { ignore: Object? }?): Hit?

-- ===================================================================== sound

declare sound: {
    play: (path: string, opts: SoundOptions?) -> Sound,
    bus: (name: string) -> Bus,
    volume: number,
    stop_all: (fade: number?) -> (),
    enabled: () -> boolean,
}

-- ===================================================================== 2D drawing (inside render())

declare draw: {
    rect: (x: number, y: number, w: number, h: number, color: ColorLike?, opts: { align: Align?, line: number? }?) -> (),
    circle: (x: number, y: number, r: number, color: ColorLike?, opts: { line: number? }?) -> (),
    line: (x1: number, y1: number, x2: number, y2: number, color: ColorLike?, width: number?) -> (),
    polygon: (points: { number }, color: ColorLike?, opts: { line: number? }?) -> (),
    sprite: (texture: Texture | string, x: number, y: number, opts: {
        w: number?, h: number?, scale: number?, rotation: number?, color: ColorLike?, align: Align?,
        flip_x: boolean?, flip_y: boolean?, region: { number }?, grid: { number }?, frame: number?,
        filter: ("nearest" | "linear")?,
    }?) -> (),
    text: (text: string | number, x: number, y: number, opts: TextOptions?) -> (number, number),
    measure: (text: string | number, opts: { size: number?, font: Font?, width: number? }?) -> (number, number),
    push: () -> (),
    pop: () -> (),
    translate: (x: number, y: number) -> (),
    rotate: (radians: number) -> (),
    scale: (sx: number, sy: number?) -> (),
    layer: (name: ("ui" | "scene")?) -> string,
    filter: (mode: "nearest" | "linear") -> (),
    stats: () -> { [string]: number },
    spark_badge: (opts: { align: Align?, margin: number?, width: number?, alpha: number?, x: number?, y: number? }?) -> (boolean, boolean),
}

declare screen: {
    width: number,
    height: number,
    scale: number,
    pixel_width: number,
    pixel_height: number,
    mouse: () -> (number, number),
    center: () -> (number, number),
}

-- ===================================================================== input and time

declare input: {
    down: (key: Key) -> boolean,
    pressed: (key: Key) -> boolean,
    released: (key: Key) -> boolean,
    -- went down or auto-repeated this frame (key held): menus, text fields
    repeated: (key: Key) -> boolean,
    -- text typed this frame (keyboard layout and IME aware, no control characters); "" if none
    text: () -> string,
    mouse_down: (button: MouseButton?) -> boolean,
    mouse_pressed: (button: MouseButton?) -> boolean,
    mouse_released: (button: MouseButton?) -> boolean,
    mouse: () -> (number, number),
    mouse_delta: () -> (number, number),
    wheel: () -> number,
    lock_mouse: (on: boolean?) -> (),
    mouse_locked: () -> boolean,
    pad_down: (button: PadButton, pad: number?) -> boolean,
    pad_pressed: (button: PadButton, pad: number?) -> boolean,
    pad_released: (button: PadButton, pad: number?) -> boolean,
    pad_axis: (axis: PadAxis, pad: number?) -> number,
    pad_stick: (stick: "left" | "right", pad: number?) -> (number, number),
    pads: () -> number,
    pad_name: (pad: number?) -> string?,
    any_pressed: () -> boolean,
}

declare time: {
    dt: number,
    elapsed: number,
    frame: number,
    fps: number,
    unscaled_dt: number,
    unscaled_elapsed: number,
    scale: number,
    fixed_dt: number,
}

-- ===================================================================== saving, window, misc

declare save: {
    set: (key: string, value: any) -> (),
    get: <T>(key: string, default: T?) -> T,
    has: (key: string) -> boolean,
    delete: (key: string) -> (),
    clear: () -> (),
    keys: () -> { string },
    path: () -> string?,
}

declare window: {
    fullscreen: boolean,
    title: string?,
    width: number,
    height: number,
}

declare function quit(): ()

-- Callbacks the engine calls if the game defines them (globals in main.luau):
--   function start() end              once, after every (re)load; runs as a task (may wait)
--   function update(dt: number) end   every frame
--   function fixed_update(dt: number) end   constant 1/60 s steps (physics)
--   function late_update(dt: number) end    after animations
--   function render() end             2D drawing with draw.*
-- Built-in events (on(name, fn)): key_pressed(key), key_released(key), mouse_pressed(button, x, y),
--   mouse_released(button, x, y), wheel(amount), collision(a, b, info) / obj:on("collision", fn(other, info)).
