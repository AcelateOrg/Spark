-- Module contract: require paths, caching, folders with init.luau, errors and cycles.

local function expect(cond, msg)
    if not cond then
        error("FAIL: " .. msg, 2)
    end
end

local function fails(f, needle)
    local ok, err = pcall(f)
    if ok then
        return false
    end
    return needle == nil or string.find(tostring(err), needle, 1, true) ~= nil
end

local util = require("./src/util")
local hud = require("./src/ui/hud")
local counter = require("./src/counter")   -- src/counter/init.luau

function start()
    expect(util.add(2, 3) == 5, "module function")
    expect(require("./src/util") == util, "modules are cached")
    expect(require("@game/src/util") == util, "@game/ resolves to the same module")
    expect(require("./src/util.luau") == util, "explicit extension")
    expect(hud.label() == "score: 5", "../ and ./ inside modules, got " .. hud.label())
    counter.bump()
    counter.bump()
    expect(require("./src/counter").value == 2, "module state is shared")
    expect(require("./src/flag") == true, "a module without return gives true")
    expect(fails(function() require("util") end, "require(\"./util\")"), "bare name hint")
    expect(fails(function() require("./src/missing") end, "no such module"), "missing module")
    expect(fails(function() require("../outside") end, "above the game folder"), "cannot leave the game")
    expect(fails(function() require("./src/cycle_a") end, "require cycle: src/cycle_a.luau -> src/cycle_b.luau -> src/cycle_a.luau"), "cycle")
    expect(fails(function() require("./src/broken") end, "src/broken.luau:2"), "errors point at the module file")
    -- Engine globals work inside modules.
    expect(util.spawn_cube():exists(), "spawn from a module")
    print("modules contract ok")
    quit()
end
