<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/spark-logo-white.png">
    <img src="docs/assets/spark-logo-dark.png" alt="SPARK" width="520">
  </picture>
</p>

<p align="center">
  <b>A modular, code-first game engine built from scratch for making games with AI.</b><br>
  Rust + wgpu core &middot; game code in Luau &middot; native executables, no browser.
</p>

<p align="center">
  <a href="https://github.com/AcelateOrg/Spark/actions/workflows/ci.yml"><img src="https://github.com/AcelateOrg/Spark/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MPL--2.0-blue" alt="License: MPL-2.0"></a>
</p>

<p align="center">
  English &middot; <a href="README.ru.md">Русский</a>
</p>

---

## Why

Modern models can already generate working games: low-poly 3D, sprites, procedural sound, game logic.
But almost always they do it in HTML + JS / Three.js, inside a browser, and the browser is a bad home for games:

- **Performance.** Getting Three.js or Pixi to run without micro-stutters on any hardware is a fight you lose.
- **No native builds.** The browser is a sandbox: no real `.exe`, no control over memory, no direct path to the GPU.
- **Distribution.** An HTML game is awkward to ship to Steam or itch.io, and it renders differently in every browser.

**Spark** gives the AI a native, fast runtime instead, driven by **Luau** - a small, well-known language that
models read and write very well. The goal: your AI agent builds a complete project on its own - from assets to
gameplay - and you get a native window and a single executable to share. Writing games by hand is, of course,
also allowed.

Spark does not try to compete with Unity, Unreal or Godot. It has a different job: be the best target for AI-written games.

## Built for AI agents

- **Small, imperative API with one way to do things.** The whole Luau API fits in one file:
  [docs/LUAU_API.md](docs/LUAU_API.md). Give it to your model.
- **Every project carries its own docs.** `spark new` writes `AGENTS.md` and the API reference of the exact engine
  version into the game folder, so any coding agent (Claude Code, Codex, Cursor, ...) knows the rules without setup.
- **The agent can see what it made.** Headless runs render frames to PNG and dump the scene as text
  (`--headless --frames N --screenshot shot.png --dump-scene`) - no window, no human needed.
- **Errors that say how to fix them.** Unknown keys, options and names fail loudly with `file:line`, the allowed
  values and a hint. A headless run with an error exits with code 1.
- **Hot reload.** Save any `.luau` file or shader and the game restarts instantly.
- **Code-first.** No editor, no scene files, no hidden state: the scene, logic, UI and assets are all code.

## Features

| Module | What you get |
|---|---|
| **Render** | wgpu (Vulkan / DirectX 12 / Metal / OpenGL fallback). 3D meshes and primitives, glTF 2.0 models with skins and animations, sun / point / spot lights, fog, your own WGSL surface and full-screen post shaders with hot reload ([docs/SHADERS.md](docs/SHADERS.md)), low internal resolution for weak PCs |
| **2D** | immediate-mode drawing: sprites and sprite sheets, text with any TTF/OTF font, shapes, transforms, UI and in-scene layers |
| **Script** | Luau runtime with hot reload, modules (`require`), timers, coroutine tasks with `wait`, tweens |
| **Input** | keyboard, mouse (with FPS mouse lock), gamepads with hot-plug |
| **Audio** | wav / ogg / mp3 / flac, 3D positional sound, fades, mixer buses (music / sfx / ui ...), layered music |
| **Events** | events with priorities, one-shot listeners, per-object events, deferred events, collision events |
| **Physics** | rapier3d: dynamic / kinematic / static bodies, character controller, raycasts, collision events, picking against rendered meshes |
| **Ship** | `spark build` packs the game into **one executable** (+ zip). Save files, fullscreen, window icon, splash |

## Install (Windows)

In PowerShell:

```powershell
irm https://raw.githubusercontent.com/AcelateOrg/Spark/main/install.ps1 | iex
```

This downloads the latest release into `%LOCALAPPDATA%\Spark\bin` and adds it to your `PATH` (no admin rights).
Open a new terminal, then:

```
spark new mygame          # a working game in the recommended layout
spark mygame              # run it; edit any file and it reloads
spark build mygame        # dist/mygame.exe - one file with everything inside
spark update              # later: install the newest Spark
```

Or download `spark-windows-x64.zip` from [Releases](https://github.com/AcelateOrg/Spark/releases) and put
`spark.exe` anywhere you like. The exe is not code-signed yet, so Windows SmartScreen may say "Unknown publisher"
and some antivirus programs may be suspicious of fresh unsigned builds.

Prebuilt binaries are Windows x64 only for now; on Linux and macOS build from source.

### Build from source

Requires [Rust](https://rustup.rs) 1.85+.

```
git clone https://github.com/AcelateOrg/Spark.git
cd Spark
cargo build --release
target/release/spark new mygame
```

## Quick start

Minimal game (`main.luau`):

```lua
local cube

function start()
    spawn(Mesh.plane(20), "green")
    cube = spawn(Mesh.cube(), "orange", { position = vec3(0, 0.5, 0) })
end

function update(dt)
    local speed = if input.down("space") or input.pad_down("a") then 4 else 1
    cube:rotate_y(dt * speed)
end
```

Examples in this repo:

```
target/release/spark examples/demo        # 3D: physics, custom shaders, sound
target/release/spark examples/starfall    # 2D game made only with draw.* (keyboard, mouse or gamepad)
cargo run -p hello                        # the same idea in pure Rust
```

## Working with an AI agent

1. `spark new mygame` and open the folder in your agent.
2. Describe the game. The agent reads `AGENTS.md` and `docs/SPARK_API.md` and writes the code.
3. It checks itself with `spark . --headless --frames 120 --screenshot shot.png` and looks at the picture.
4. You play with `spark .` - every save reloads the game.
5. `spark build .` gives you one `.exe` to send to friends or upload to itch.io.

After updating the engine, `spark docs mygame` refreshes the API reference inside the game.

## Command line

```
spark path/to/game [flags]           run a game (folder with main.luau)
spark new path/to/game               new game: main.luau, src/, assets/, game.toml, AGENTS.md, docs/
spark docs path/to/game              refresh docs/ and AGENTS.md
spark build path/to/game [--out DIR] [--loose] [--no-zip]
spark update [VERSION]               install the latest (or given) release over this spark.exe
spark --version
```

`spark = "0.1"` in a game's `game.toml` records the engine version it was made for; a different minor version
(before 1.0) prints a warning with what to do.

Flags for every game: `--headless`, `--frames N`, `--screenshot PATH`, `--dump-scene`, `--size WxH`, `--fixed-dt S`,
`--no-vsync`, `--splash` / `--no-splash`, `--fullscreen`, `--windowed`, `--help`.
In a window: F12 = screenshot, F11 / Alt+Enter = fullscreen. Environment: `RUST_LOG=debug`, `WGPU_BACKEND=dx12|vulkan|gl`.

## Architecture

| Crate | Role |
|---|---|
| `spark` | facade: `App`, prelude, re-exports - use this one from Rust |
| `spark-core` | world data without GPU: scene, assets, materials, input, time, events, scheduler, audio queue, virtual file system |
| `spark-render` | wgpu renderer: surface / post shaders, 2D canvas, windowed and headless, screenshots |
| `spark-window` | winit window and input mapping, gamepads (gilrs) |
| `spark-audio` | kira audio backend |
| `spark-physics` | rapier3d backend |
| `spark-script` | the Luau API, modules and hot reload (`ScriptGame`) |
| `spark-cli` | the `spark` executable: run, new, docs, build, update |

Conventions: Y-up, right-handed, 1 unit = 1 meter, objects look along -Z, radians (FOV in degrees), sRGB colors.

## Status

Early version (0.1). Windows 10/11 x64 is the main and tested platform; the code is cross-platform, but Linux and
macOS builds are not tested yet. The API can still change between versions - `spark docs` keeps a game's docs in sync.

## Releases (maintainers)

Bump `version` in the root `Cargo.toml`, commit, then push a tag with the same version:

```
git tag v0.1.0
git push origin v0.1.0
```

The [release workflow](.github/workflows/release.yml) tests, builds `spark-windows-x64.zip` and publishes the
GitHub release that `install.ps1` and `spark update` download.

## License

[Mozilla Public License 2.0](LICENSE). You can use Spark in any project, including closed-source and commercial
games; changes to the engine's own files must stay open under the MPL. Your game code and assets remain yours.

Bundled font: Noto Sans, [SIL Open Font License](crates/spark-core/assets/fonts/OFL.txt).

---

<p align="center">Developed by <b>Acelate</b>, specially for AI &middot; <a href="https://acelate.com/spark">acelate.com/spark</a></p>
