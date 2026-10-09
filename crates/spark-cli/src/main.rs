//! `spark path/to/game [flags]` - runs a Luau game (a folder means `folder/main.luau`).
//! Without a path: `main.luau` in the current folder, else next to the executable (shipped games:
//! put the exe, `main.luau` and the assets in one folder and double-click the exe).
//!
//! `spark new path/to/game` - a new game with the recommended folder layout (+ AGENTS.md and the API docs).
//! `spark docs path/to/game` - refresh the API docs inside a game after an engine update.
//! `spark update [version]` - replace this spark executable with a GitHub release (Windows, Linux, macOS).
//! `spark check path/to/game [--frames N] [--json]` - find script errors headless, without a window or GPU.
//!
//! `spark build path/to/game` - a player build: ONE executable (this binary, no console window) with
//! every game file packed inside, plus a .zip. When such an executable starts it runs its packed game.
#![cfg_attr(all(feature = "gui", windows), windows_subsystem = "windows")]

mod check;
mod dist;
mod manifest;
mod new;
mod update;

use std::path::{Path, PathBuf};

use engine::script::{ScriptGame, default_save_path};
use engine::{App, HELP, ImageData};

use manifest::Manifest;

const USAGE: &str = "Usage:
  spark path/to/game [flags]              run a game (folder with main.luau, or the .luau file)
  spark new path/to/game                  create a new game with the recommended layout
  spark docs path/to/game                 write the API docs + AGENTS.md (for AI agents) into a game
  spark check path/to/game [--frames N]   find script errors without a window or GPU (default 60 frames)
                           [--json]       machine-readable report: {ok, errors:[{file,line,message}], frames, objects}
  spark update [version]                  download the latest (or given) Spark release from GitHub
  spark --version
  spark build path/to/game [--out DIR]    make a player build: one executable with everything inside (+ .zip / .tar.gz)
                           [--loose]      exe + game files in a folder instead
                           [--no-zip]
                           [--app]        macOS: also wrap it in a <Name>.app bundle";

fn main() {
    // A player build: the game is packed inside this executable.
    if let Some(pack) = engine::vfs::Pack::embedded() {
        run_packed(pack);
        return;
    }
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match raw.first().map(String::as_str) {
        Some("build") => std::process::exit(dist::run(&raw[1..])),
        Some("new") => std::process::exit(new::run(&raw[1..])),
        Some("docs") => std::process::exit(new::run_docs(&raw[1..])),
        Some("check") => std::process::exit(check::run(&raw[1..])),
        Some("update") => std::process::exit(update::run(&raw[1..])),
        Some("--version" | "-V") => {
            println!("spark {}", engine::VERSION);
            return;
        }
        _ => {}
    }
    let app = App::new("Spark");
    let arg = app.args().rest.first().cloned();
    let mut path = match arg {
        Some(a) => PathBuf::from(a),
        None => {
            let here = PathBuf::from("main.luau");
            let beside_exe = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("main.luau")));
            match beside_exe {
                Some(p) if !here.is_file() && p.is_file() => p,
                _ => here,
            }
        }
    };
    if path.is_dir() {
        path = path.join("main.luau");
    }
    if !path.is_file() {
        eprintln!("spark: script not found: {}\n{USAGE}\n\n{HELP}", path.display());
        std::process::exit(2);
    }
    let dir = path.canonicalize().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    let path = dir.join(path.file_name().unwrap_or_default());
    run(app, path, dir, false);
}

/// Runs the game packed in this executable (no hot reload: the files can't change).
fn run_packed(pack: engine::vfs::Pack) {
    let exe = std::env::current_exe().ok();
    let dir = exe.as_ref().and_then(|e| e.parent()).map(Path::to_path_buf).unwrap_or_default();
    let dir = dir.canonicalize().unwrap_or(dir);
    if !pack.contains("main.luau") {
        eprintln!("spark: the game pack in this executable has no main.luau (rebuild it with spark build)");
        std::process::exit(2);
    }
    engine::vfs::mount(&dir, pack);
    let app = App::new("Spark");
    run(app, dir.join("main.luau"), dir, true);
}

fn run(app: App, path: PathBuf, dir: PathBuf, packed: bool) {
    let manifest = match Manifest::load(&dir) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("spark: {e}");
            std::process::exit(2);
        }
    };
    if !packed {
        if let Some(w) = manifest.version_warning(engine::VERSION) {
            eprintln!("spark: warning: {w}");
        }
    }
    let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Spark".into());
    let title = manifest.title.clone().unwrap_or(folder);
    let headless = app.args().headless;
    let mut app = app.title(title.clone()).size(manifest.width, manifest.height).vsync(manifest.vsync);
    app = app.fullscreen(manifest.fullscreen).splash(manifest.splash).fps_in_title(manifest.show_fps.unwrap_or(manifest.title.is_none()));
    if let Some(icon) = &manifest.icon {
        match ImageData::load(&dir.join(icon)) {
            Ok(img) => app = app.icon(img),
            Err(e) => eprintln!("spark: game.toml icon: {e}"),
        }
    }
    let save = if headless { None } else { default_save_path(manifest.save_name.as_deref().unwrap_or(&title)) };
    app.run(ScriptGame::new(path).strict(headless).hot_reload(!headless && !packed).save_path(save));
}
