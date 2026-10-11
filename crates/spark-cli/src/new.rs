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
//!   docs/            the full Spark API reference + spark.d.luau type definitions, matching this engine version
//!   .luaurc, .vscode/settings.json   luau-lsp setup: autocomplete and type checking of the Spark API
//! ```

use std::path::PathBuf;

use spark_project::template::{write_docs, write_template};

/// `spark docs path/to/game`
pub fn run_docs(args: &[String]) -> i32 {
    let [path] = args else {
        eprintln!(
            "usage: spark docs path/to/game   (writes docs/SPARK_API.md, docs/SPARK_SHADERS.md, docs/spark.d.luau; AGENTS.md, .luaurc, .vscode/settings.json if missing)"
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_complete() {
        let dir = std::env::temp_dir().join(format!("spark-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_template(&dir, "Test \"Game\"").unwrap();
        for f in ["main.luau", "game.toml", "src/player.luau", "src/level.luau", "src/hud.luau", "assets/textures/.gitkeep", "AGENTS.md", "docs/SPARK_API.md", "docs/SPARK_SHADERS.md", "docs/spark.d.luau", ".luaurc", ".vscode/settings.json"] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        let toml = std::fs::read_to_string(dir.join("game.toml")).unwrap();
        let m = crate::manifest::Manifest::parse(&toml).expect(&toml);
        assert!(m.version_warning(engine::VERSION).is_none(), "{toml}");
        // The template game runs without errors.
        let mut game = engine::script::ScriptGame::new(dir.join("main.luau")).hot_reload(false);
        let mut world = engine::World::new();
        engine::Game::start(&mut game, &mut world);
        for _ in 0..30 {
            engine::run_frame(&mut game, &mut world, 1.0 / 60.0);
        }
        assert!(game.error().is_none(), "{:?}", game.error());
        assert!(world.scene.len() > 5, "level spawned");
        // `spark check` agrees.
        let report = crate::check::check(&dir, 30);
        assert!(report.ok(), "{:?}", report.errors);
        // User edits of the editor setup survive `spark docs`.
        std::fs::write(dir.join(".luaurc"), "{}").unwrap();
        write_docs(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(".luaurc")).unwrap(), "{}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn definitions_cover_every_input_name() {
        let defs = include_str!("../../../docs/spark.d.luau");
        for k in engine::Key::ALL {
            assert!(defs.contains(&format!("\"{}\"", k.name())), "docs/spark.d.luau: Key type misses \"{}\"", k.name());
        }
        for b in engine::MouseButton::ALL {
            assert!(defs.contains(&format!("\"{}\"", b.name())), "docs/spark.d.luau misses mouse button \"{}\"", b.name());
        }
        for b in engine::PadButton::ALL {
            assert!(defs.contains(&format!("\"{}\"", b.name())), "docs/spark.d.luau misses pad button \"{}\"", b.name());
        }
        // Every global table of the API reference is declared.
        for g in ["input", "time", "camera", "graphics", "physics", "sound", "draw", "screen", "save", "window", "Mesh", "Texture", "Material", "Color", "Model", "Shader", "Font"] {
            assert!(defs.contains(&format!("declare {g}:")), "docs/spark.d.luau misses `declare {g}:`");
        }
        let api = include_str!("../../../docs/LUAU_API.md");
        for f in ["input.text()", "input.repeated(", "spark check"] {
            assert!(api.contains(f), "docs/LUAU_API.md does not document {f}");
        }
    }
}
