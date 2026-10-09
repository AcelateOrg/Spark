//! `tween(target, { field = goal, ... }, seconds, ease_or_options)`: animates numbers, vectors and colors
//! of objects, the camera or plain tables. Written in Luau on top of tasks.

use mlua::Lua;

use crate::convert::LuaResult;

const PRELUDE: &str = r#"
local pi, sin, cos, sqrt = math.pi, math.sin, math.cos, math.sqrt
local function bounce_out(t)
    if t < 1 / 2.75 then return 7.5625 * t * t
    elseif t < 2 / 2.75 then t -= 1.5 / 2.75 return 7.5625 * t * t + 0.75
    elseif t < 2.5 / 2.75 then t -= 2.25 / 2.75 return 7.5625 * t * t + 0.9375
    else t -= 2.625 / 2.75 return 7.5625 * t * t + 0.984375 end
end
local EASE = {
    linear = function(t) return t end,
    in_quad = function(t) return t * t end,
    out_quad = function(t) return 1 - (1 - t) * (1 - t) end,
    in_out_quad = function(t) return if t < 0.5 then 2 * t * t else 1 - (-2 * t + 2) ^ 2 / 2 end,
    in_cubic = function(t) return t * t * t end,
    out_cubic = function(t) return 1 - (1 - t) ^ 3 end,
    in_out_cubic = function(t) return if t < 0.5 then 4 * t * t * t else 1 - (-2 * t + 2) ^ 3 / 2 end,
    in_sine = function(t) return 1 - cos(t * pi / 2) end,
    out_sine = function(t) return sin(t * pi / 2) end,
    in_out_sine = function(t) return -(cos(pi * t) - 1) / 2 end,
    in_expo = function(t) return if t == 0 then 0 else 2 ^ (10 * t - 10) end,
    out_expo = function(t) return if t == 1 then 1 else 1 - 2 ^ (-10 * t) end,
    in_back = function(t) return 2.70158 * t * t * t - 1.70158 * t * t end,
    out_back = function(t) t -= 1 return 1 + 2.70158 * t * t * t + 1.70158 * t * t end,
    out_elastic = function(t)
        if t == 0 or t == 1 then return t end
        return 2 ^ (-10 * t) * sin((t * 10 - 0.75) * (2 * pi / 3)) + 1
    end,
    out_bounce = bounce_out,
    in_bounce = function(t) return 1 - bounce_out(1 - t) end,
}
EASE["in"] = EASE.in_quad
EASE.out = EASE.out_quad
EASE.in_out = EASE.in_out_quad
EASE.smooth = function(t) return t * t * (3 - 2 * t) end

local ease_names = {}
for k in EASE do table.insert(ease_names, k) end
table.sort(ease_names)

local OPTS = { ease = true, delay = true, loop = true, yoyo = true, unscaled = true, done = true, owner = true }

local function alive(x)
    if type(x) ~= "userdata" then return true end
    local ok, exists = pcall(function() return x.exists end)
    if ok and type(exists) == "function" then return exists(x) end
    return true
end

function tween(target, goals, duration, opts)
    if type(target) ~= "table" and type(target) ~= "userdata" then
        error("tween: first argument must be an object, the camera or a table, got " .. typeof(target), 2)
    end
    if type(goals) ~= "table" then error("tween: second argument must be a table like { position = vec3(0, 2, 0) }", 2) end
    duration = tonumber(duration) or error("tween: third argument must be the duration in seconds", 2)
    if type(opts) == "string" then opts = { ease = opts } end
    opts = opts or {}
    for k in opts do
        if not OPTS[k] then error("tween: unknown option '" .. tostring(k) .. "' (allowed: ease, delay, loop, yoyo, unscaled, done, owner)", 2) end
    end
    local ease = EASE[opts.ease or "out_quad"]
    if not ease then error("tween: unknown ease '" .. tostring(opts.ease) .. "' (use one of: " .. table.concat(ease_names, ", ") .. ")", 2) end
    local h = { active = true, finished = false }
    function h:cancel() self.active = false end
    local function run()
        if opts.delay and opts.delay > 0 then wait(opts.delay) end
        local from = {}
        for k in goals do from[k] = target[k] end
        local t, forward = 0, true
        while h.active do
            if not alive(target) or (opts.owner and not alive(opts.owner)) then h.active = false break end
            t += if opts.unscaled then time.unscaled_dt else time.dt
            local k = if duration > 0 then math.min(1, t / duration) else 1
            local e = ease(if forward then k else 1 - k)
            for key, goal in goals do target[key] = lerp(from[key], goal, e) end
            if k >= 1 then
                if opts.yoyo and forward then forward, t = false, 0
                elseif opts.loop then forward, t = true, 0
                else break end
            end
            wait_frames(1)
        end
        if h.active then
            h.active, h.finished = false, true
            if opts.done then opts.done() end
        end
    end
    h.task = task(run)
    return h
end
"#;

pub(crate) fn install(lua: &Lua) -> LuaResult<()> {
    lua.load(PRELUDE).set_name("=spark.tween").exec()
}
