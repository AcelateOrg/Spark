//! `spark new path/to/game`: a small working game in the recommended layout.
//! `spark docs path/to/game`: (re)writes the engine docs + AGENTS.md into an existing game.
//!
//! ```text
//! mygame/
//!   main.luau        entry point: wires the modules together, stays short
//!   game.toml        title, window, build name
//!   src/             game code, one module per system (require("./src/player"))
//!   assets/          textures/ sounds/ models/ shaders/ fonts/
//!   AGENTS.md        instructions for AI coding agents
//!   docs/            the full Spark API reference, matching this engine version
//! ```

use std::path::{Path, PathBuf};

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

## Layout

- `main.luau` - entry point. Only wires modules together in `start / update / fixed_update / late_update / render`.
- `src/` - one module per system: `local Player = require("./src/player")`, each file returns a table.
  Shared state lives in its own module (e.g. `src/state.luau`). Keep files under ~800 lines.
- `assets/` - textures, sounds, models (glTF), shaders, fonts. Paths are relative to the game folder.
- `game.toml` - title, window size, icon, build name.

## Check your work (no window needed)

```
spark . --headless --frames 120 --screenshot shot.png   # render a frame, then look at shot.png
spark . --headless --frames 60 --dump-scene            # every object: position, rotation, bounds
```

A script error stops a headless run with exit code 1 and prints `file:line`, the message and a hint.
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
];

const ASSET_DIRS: &[&str] = &["textures", "sounds", "models", "shaders", "fonts"];

/// `spark docs path/to/game`
pub fn run_docs(args: &[String]) -> i32 {
    let [path] = args else {
        eprintln!("usage: spark docs path/to/game   (writes docs/SPARK_API.md, docs/SPARK_SHADERS.md, AGENTS.md)");
        return 1;
    };
    let dir = PathBuf::from(path);
    if !dir.join("main.luau").is_file() {
        eprintln!("spark docs: '{}' has no main.luau", dir.display());
        return 1;
    }
    match write_docs(&dir) {
        Ok(()) => {
            println!("spark docs: updated {}", dir.join("docs").display());
            0
        }
        Err(e) => {
            eprintln!("spark docs: {e}");
            1
        }
    }
}

fn write_file(p: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create '{}': {e}", parent.display()))?;
    }
    std::fs::write(p, text).map_err(|e| format!("cannot write '{}': {e}", p.display()))
}

/// Engine docs are always overwritten (they belong to the engine); AGENTS.md only if missing (the user may edit it).
fn write_docs(dir: &Path) -> Result<(), String> {
    for (rel, text) in DOCS {
        write_file(&dir.join(rel), text)?;
    }
    let agents = dir.join("AGENTS.md");
    if !agents.exists() {
        write_file(&agents, AGENTS)?;
    }
    Ok(())
}

pub fn run(args: &[String]) -> i32 {
    match create(args) {
        Ok(dir) => {
            println!("spark new: created {}", dir.display());
            println!("  run it:   spark {}", dir.display());
            println!("  ship it:  spark build {}", dir.display());
            0
        }
        Err(e) => {
            eprintln!("spark new: {e}");
            1
        }
    }
}

fn create(args: &[String]) -> Result<PathBuf, String> {
    let [path] = args else { return Err("usage: spark new path/to/game".into()) };
    let dir = PathBuf::from(path);
    if dir.join("main.luau").exists() {
        return Err(format!("'{}' already has a main.luau", dir.display()));
    }
    let title = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty() && n != ".")
        .unwrap_or_else(|| "My Game".into());
    write_template(&dir, &title)?;
    Ok(dir)
}

fn write_template(dir: &Path, title: &str) -> Result<(), String> {
    let t = title.replace('"', "'");
    let files: [(&str, String); 6] = [
        ("main.luau", MAIN.replace("{TITLE}", &t)),
        ("game.toml", TOML.replace("{TITLE}", &t)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_complete() {
        let dir = std::env::temp_dir().join(format!("spark-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_template(&dir, "Test \"Game\"").unwrap();
        for f in ["main.luau", "game.toml", "src/player.luau", "src/level.luau", "src/hud.luau", "assets/textures/.gitkeep", "AGENTS.md", "docs/SPARK_API.md", "docs/SPARK_SHADERS.md"] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        let toml = std::fs::read_to_string(dir.join("game.toml")).unwrap();
        assert!(crate::manifest::Manifest::parse(&toml).is_ok(), "{toml}");
        // The template game runs without errors.
        let mut game = engine::script::ScriptGame::new(dir.join("main.luau")).hot_reload(false);
        let mut world = engine::World::new();
        engine::Game::start(&mut game, &mut world);
        for _ in 0..30 {
            engine::run_frame(&mut game, &mut world, 1.0 / 60.0);
        }
        assert!(game.error().is_none(), "{:?}", game.error());
        assert!(world.scene.len() > 5, "level spawned");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
