local util = require("../util")
local config = require("./config")

local hud = {}

function hud.label(): string
    return config.prefix .. util.add(2, 3)
end

return hud
