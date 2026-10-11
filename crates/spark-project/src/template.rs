//! New-game template (`spark new`, Spark Editor "New project") and the engine docs shipped into
//! every game (`spark docs`).
//!
//! ```text
//! mygame/
//!   main.luau        entry point: wires the modules together, stays short
//!   game.toml        title, window, build name
//!   src/             game code, one module per system (require("./src/player"))
//!   assets/          textures/ sounds/ models/ shaders/ fonts/
//!   AGENTS.md        instructions for AI coding agents
//!   docs/            the full Spark API reference + spark.d.luau type definitions, matching this engine version
//!   .luaurc, .vscode/settings.json   luau-lsp setup: autocomplete and type checking of the Spark API
//! ```

use std::path::Path;

const MAIN: &str = r##"-- {TITLE}: entry point. Keep this file short - it only wires the modules in src/ together.
-- Run:   spark .          (saving any file reloads the game instantly)
-- Ship:  spark build .    (one .exe with everything inside)

local Level = require("./src/level")
local Player = require("./src/player")
local Hud = require("./src/hud")

local player

function start()
    Level.build()
    player = Player.new(vec3(0, 0.5, 0))
end

function update(dt)
    player:update(dt)
end

function render()
    Hud.draw(player)
end
"##;

const LEVEL: &str = r##"-- The world: light, ground and a few props.

local Level = {}

local PROPS = 8

function Level.build()
    background("#9cc8f0")
    sun(vec3(-0.5, -1, -0.3))
    ambient("#4a5068")
    spawn(Mesh.plane(30), "#5a8f4e")
    for i = 1, PROPS do
        local a = i / PROPS * math.pi * 2
        spawn(Mesh.cube(), "#c8873a", { position = vec3(math.cos(a) * 6, 0.5, math.sin(a) * 6) })
    end
end

return Level
"##;

const PLAYER: &str = r##"-- The player: WASD to move, the camera follows.

local Player = {}
Player.__index = Player

local SPEED = 5
local CAMERA_OFFSET = vec3(0, 6, 9)

export type Player = typeof(setmetatable({} :: { body: any, walked: number }, Player))

function Player.new(position: vector): Player
    local self = setmetatable({}, Player)
    self.body = spawn(Mesh.cube(), "#e04848", { position = position, scale = vec3(0.6, 1, 0.6) })
    self.walked = 0
    return self
end

function Player.update(self: Player, dt: number)
    local move = vec3(0, 0, 0)
    if input.down("w") then move += vec3(0, 0, -1) end
    if input.down("s") then move += vec3(0, 0, 1) end
    if input.down("a") then move += vec3(-1, 0, 0) end
    if input.down("d") then move += vec3(1, 0, 0) end
    -- gamepad left stick (0 inside the dead zone, up = +1)
    move += vec3(input.pad_axis("left_x"), 0, -input.pad_axis("left_y"))
    if length(move) > 0 then
        local step = (if length(move) > 1 then normalize(move) else move) * SPEED * dt
        self.body.position += step
        self.walked += length(step)
    end
    camera.position = self.body.position + CAMERA_OFFSET
    camera:look_at(self.body)
end

return Player
"##;

const HUD: &str = r##"-- 2D overlay, drawn every frame from render().

local Hud = {}

function Hud.draw(player)
    draw.text(string.format("walked: %.1f m", player.walked), 20, 20, { size = 28, shadow = true })
    draw.text("WASD / left stick - move", 20, 56, { size = 18, color = "#ffffffaa" })
end

return Hud
"##;

const TOML: &str = r##"# Spark game manifest (all keys optional, see docs/LUAU_API.md).
title = "{TITLE}"
spark = "{SPARK}"         # engine version the game is made for
width = 1280
height = 720
# fullscreen = false
# icon = "assets/icon.png"
# splash = false          # "Powered by Spark" intro
# build_name = "MyGame"   # exe name of `spark build`
exclude = ["AGENTS.md", "docs"]  # not packed into the build
"##;

const GITIGNORE: &str = "dist/\nscreenshots/\nerror.log\n";

const AGENTS: &str = r##"# AGENTS.md - working on this game

This is a game for **Spark Engine**: a native runtime (Rust + wgpu) where all game code is **Luau**.
There is no editor - the scene, logic, UI and assets are all described in code.

## Read first

- `docs/SPARK_API.md` - the complete Luau API of the engine version this game was made with.
  Use only what is listed there: unknown functions, options and keys raise errors that name the allowed ones.
- `docs/SPARK_SHADERS.md` - custom WGSL surface and post-process shaders.
- `docs/spark.d.luau` - type definitions of the same API (luau-lsp). Editors with luau-lsp show wrong
  names / arguments as you type (`.vscode/settings.json` and `.luaurc` are set up for it).

## Layout

- `main.luau` - entry point. Only wires modules together in `start / update / fixed_update / late_update / render`.
- `src/` - one module per system: `local Player = require("./src/player")`, each file returns a table.
  Shared state lives in its own module (e.g. `src/state.luau`). Keep files under ~800 lines.
- `assets/` - textures, sounds, models (glTF), shaders, fonts. Paths are relative to the game folder.
- `game.toml` - title, window size, icon, build name, `spark` = the engine version the game is made for.
  After `spark update` to a new minor version, run `spark docs .`, adapt the code to the new API, then raise `spark`.

## Check your work (no window needed)

```
spark check .                    # compiles every .luau file and runs 60 frames headless (no GPU): file:line: message
spark check . --frames 300 --json   # {"ok":false,"errors":[{"file":"src/player.luau","line":12,"message":"..."}],...}
spark . --headless --frames 120 --screenshot shot.png   # render a frame, then look at shot.png
spark . --headless --frames 60 --dump-scene            # every object: position, rotation, bounds
```

`spark check` and a headless run exit with code 1 on the first script error and print `file:line`, the message and a hint.
Fix every error before moving on. `print(...)` goes to the log.

## Run and ship

```
spark .            # window, the game reloads on every save
spark build .      # dist/<Name>.exe - one file with everything packed inside
```

## Conventions

Y-up, right-handed, 1 unit = 1 meter, objects look along -Z, angles in radians (camera FOV in degrees),
colors as `"#rrggbb"` strings or `Color`.
"##;

/// Engine docs shipped into every game, so the AI agent always has the API of this exact version.
const DOCS: &[(&str, &str)] = &[
    ("docs/SPARK_API.md", include_str!("../../../docs/LUAU_API.md")),
    ("docs/SPARK_SHADERS.md", include_str!("../../../docs/SHADERS.md")),
    ("docs/spark.d.luau", include_str!("../../../docs/spark.d.luau")),
];

/// Editor setup for luau-lsp, written only when missing (the user may change them).
const EDITOR: &[(&str, &str)] = &[(".luaurc", LUAURC), (".vscode/settings.json", VSCODE)];

/// `require("@game/src/x")` resolves from the game folder, like the engine does. Engine callbacks
/// (`function start()`, `update`, ...) are called by name, so "unused function" warnings are off.
const LUAURC: &str = r#"{
    "languageMode": "nonstrict",
    "lint": {
        "FunctionUnused": false
    },
    "aliases": {
        "game": "."
    }
}
"#;

/// VS Code + "Luau Language Server" (JohnnyMorganz.luau-lsp): the Spark API from docs/spark.d.luau.
const VSCODE: &str = r#"{
    "luau-lsp.platform.type": "standard",
    "luau-lsp.types.definitionFiles": {
        "@spark": "docs/spark.d.luau"
    },
    "luau-lsp.sourcemap.enabled": false,
    "luau-lsp.require.mode": "relativeToFile",
    "files.associations": {
        "*.luau": "luau"
    }
}
"#;

const ASSET_DIRS: &[&str] = &["textures", "sounds", "models", "shaders", "fonts"];

fn write_file(p: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create '{}': {e}", parent.display()))?;
    }
    std::fs::write(p, text).map_err(|e| format!("cannot write '{}': {e}", p.display()))
}

/// Engine docs are always overwritten (they belong to the engine); AGENTS.md and the editor setup only if
/// missing (the user may edit them).
pub fn write_docs(dir: &Path) -> Result<(), String> {
    for (rel, text) in DOCS {
        write_file(&dir.join(rel), text)?;
    }
    for (rel, text) in [("AGENTS.md", AGENTS)].iter().chain(EDITOR) {
        let p = dir.join(rel);
        if !p.exists() {
            write_file(&p, text)?;
        }
    }
    Ok(())
}

/// "0.1" for engine 0.1.3: the API version a new game targets.
pub fn engine_minor() -> String {
    spark_core::VERSION.split('.').take(2).collect::<Vec<_>>().join(".")
}

pub fn write_template(dir: &Path, title: &str) -> Result<(), String> {
    let t = title.replace('"', "'");
    let files: [(&str, String); 6] = [
        ("main.luau", MAIN.replace("{TITLE}", &t)),
        ("game.toml", TOML.replace("{TITLE}", &t).replace("{SPARK}", &engine_minor())),
        ("src/level.luau", LEVEL.into()),
        ("src/player.luau", PLAYER.into()),
        ("src/hud.luau", HUD.into()),
        (".gitignore", GITIGNORE.into()),
    ];
    for (rel, text) in files {
        let p = dir.join(rel);
        if !p.exists() {
            write_file(&p, &text)?;
        }
    }
    write_docs(dir)?;
    for d in ASSET_DIRS {
        let p = dir.join("assets").join(d);
        std::fs::create_dir_all(&p).map_err(|e| format!("cannot create '{}': {e}", p.display()))?;
        let keep = p.join(".gitkeep");
        if !keep.exists() {
            let _ = std::fs::write(keep, "");
        }
    }
    Ok(())
}
