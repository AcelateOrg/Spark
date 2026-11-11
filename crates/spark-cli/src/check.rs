//! `spark check path/to/game [--frames N] [--json]`: finds script errors without a window or GPU.
//!
//! 1. Compiles every `.luau` file of the game (syntax errors in modules that are not required yet).
//! 2. Runs `main.luau` headless (physics on, no audio, no renderer) for N frames (default 60):
//!    `start()`, `update`, `fixed_update`, `render`, timers, tasks and every `require`d module.
//!
//! Prints `file:line: message` per error (or a JSON report with `--json`); exit code 1 on any error.

use std::path::{Path, PathBuf};

use engine::script::ScriptGame;
use engine::{Game, World, run_frame};

use crate::manifest::Manifest;

const USAGE: &str = "usage: spark check path/to/game [--frames N] [--json]";
/// Folders whose `.luau` files are not part of the game.
const SKIP_DIRS: &[&str] = &["dist", "target", "node_modules", "screenshots"];

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Report {
    pub errors: Vec<Problem>,
    pub frames: u32,
    pub objects: usize,
    /// `.luau` files compiled.
    pub files: usize,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn to_json(&self) -> String {
        let errors: Vec<String> = self
            .errors
            .iter()
            .map(|e| {
                let line = e.line.map(|l| l.to_string()).unwrap_or_else(|| "null".into());
                format!("{{\"file\":{},\"line\":{line},\"message\":{}}}", json_str(&e.file), json_str(&e.message))
            })
            .collect();
        format!(
            "{{\"ok\":{},\"errors\":[{}],\"frames\":{},\"objects\":{},\"files\":{}}}",
            self.ok(),
            errors.join(","),
            self.frames,
            self.objects,
            self.files
        )
    }
}

pub fn run(args: &[String]) -> i32 {
    let mut game: Option<PathBuf> = None;
    let mut frames = 60u32;
    let mut json = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json = true,
            "--frames" => match it.next().and_then(|n| n.parse().ok()) {
                Some(n) => frames = n,
                None => {
                    eprintln!("spark check: --frames needs a number, e.g. --frames 120\n{USAGE}");
                    return 2;
                }
            },
            "--help" | "-h" => {
                println!("{USAGE}");
                return 0;
            }
            _ if game.is_none() && !a.starts_with("--") => game = Some(PathBuf::from(a)),
            _ => {
                eprintln!("spark check: unexpected argument '{a}'\n{USAGE}");
                return 2;
            }
        }
    }
    let game = game.unwrap_or_else(|| PathBuf::from("."));
    let report = check(&game, frames);
    if json {
        println!("{}", report.to_json());
    } else {
        for e in &report.errors {
            match e.line {
                Some(l) => eprintln!("{}:{l}: {}", e.file, e.message),
                None => eprintln!("{}: {}", e.file, e.message),
            }
        }
        if report.ok() {
            println!("spark check: ok ({} files, {} frames, {} objects)", report.files, report.frames, report.objects);
        } else {
            eprintln!("spark check: {} error(s)", report.errors.len());
        }
    }
    if report.ok() { 0 } else { 1 }
}

/// Checks the game at `path` (folder or its main.luau).
pub fn check(path: &Path, frames: u32) -> Report {
    let mut report = Report::default();
    let main = if path.is_dir() { path.join("main.luau") } else { path.to_path_buf() };
    let Some(dir) = main.parent().map(|d| if d.as_os_str().is_empty() { Path::new(".") } else { d }).and_then(|d| d.canonicalize().ok()) else {
        report.errors.push(problem(&main.display().to_string(), None, "game folder not found"));
        return report;
    };
    if !main.is_file() {
        let msg = format!("no main.luau in '{}' (create a game with: spark new {})", dir.display(), path.display());
        report.errors.push(problem("main.luau", None, &msg));
        return report;
    }
    let main = dir.join(main.file_name().unwrap_or_default());
    if let Err(e) = Manifest::load(&dir) {
        report.errors.push(problem("game.toml", None, &e));
    }

    // 1. Syntax of every module, required or not.
    let mut files = Vec::new();
    luau_files(&dir, &dir, &mut files);
    report.files = files.len();
    let lua = mlua::Lua::new();
    for rel in &files {
        let name = rel.to_string_lossy().replace('\\', "/");
        let source = match std::fs::read_to_string(dir.join(rel)) {
            Ok(s) => s,
            Err(e) => {
                report.errors.push(problem(&name, None, &format!("cannot read: {e}")));
                continue;
            }
        };
        if let Err(e) = lua.load(source.as_str()).set_name(format!("@{name}")).into_function() {
            report.errors.push(parse_error(&e.to_string(), &name));
        }
    }
    if !report.ok() {
        // Running would only repeat the syntax error.
        return report;
    }

    // 2. Run headless.
    let error_log = dir.join("error.log");
    let had_error_log = error_log.exists();
    let mut game = ScriptGame::new(main.clone()).hot_reload(false);
    let mut world = World::new();
    world.physics.set_backend(Box::new(engine::physics::RapierBackend::new()));
    game.start(&mut world);
    while report.frames < frames && game.error().is_none() && !world.quit_requested() {
        run_frame(&mut game, &mut world, 1.0 / 60.0);
        report.frames += 1;
    }
    report.objects = world.scene.len();
    if let Some(e) = game.error() {
        let main_name = main.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        report.errors.push(parse_error(e, &main_name));
    }
    if !had_error_log {
        let _ = std::fs::remove_file(&error_log);
    }
    report
}

fn problem(file: &str, line: Option<u32>, message: &str) -> Problem {
    Problem { file: file.into(), line, message: message.trim().into() }
}

/// Splits `src/a.luau:12: message` (anywhere in a Luau error / traceback) into file, line, message.
/// The first `<file>.luau:<line>:` location wins; without one the error is attributed to `fallback`.
pub fn parse_error(text: &str, fallback: &str) -> Problem {
    let text = text.trim();
    let mut search = 0;
    while let Some(pos) = text[search..].find(".luau") {
        let at = search + pos;
        // `file.luau:12:` or `[string "file.luau"]:12:`
        let after = text[at + 5..].trim_start_matches(['"', '\'', ']']);
        let Some(after) = after.strip_prefix(':') else {
            search = at + 5;
            continue;
        };
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            let start = text[..at]
                .char_indices()
                .rev()
                .find(|(_, c)| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | '[' | '@' | '<'))
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            let file = text[start..at + 5].replace('\\', "/");
            let line = digits.parse().ok();
            let rest = after[digits.len()..].trim_start_matches(':').trim();
            // Keep any context in front of the location ("in start(): ...") and the rest (stack trace, hint).
            let prefix = text[..start].trim().trim_end_matches(['[', '"', '\'', '(', '@', '<']).trim();
            let message = if prefix.is_empty() || prefix.ends_with("error:") { rest.to_string() } else { format!("{prefix} {rest}") };
            return Problem { file, line, message: message.trim().into() };
        }
        search = at + 5;
    }
    problem(fallback, None, text)
}

/// `.luau` files below `dir` (relative to `root`), sorted, without hidden / build folders.
fn luau_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                luau_files(root, &path, out);
            }
        } else if name.ends_with(".luau") && !name.ends_with(".d.luau") {
            out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(files: &[(&str, &str)]) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("spark-check-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        for (rel, text) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dir
    }

    #[test]
    fn parses_locations() {
        let p = parse_error("src/player.luau:12: attempt to index nil with 'x'\nstack traceback:\n...", "main.luau");
        assert_eq!((p.file.as_str(), p.line), ("src/player.luau", Some(12)));
        assert!(p.message.starts_with("attempt to index nil"), "{}", p.message);
        let p = parse_error("runtime error: [string \"main.luau\"]:3: boom", "main.luau");
        assert_eq!((p.file.as_str(), p.line), ("main.luau", Some(3)));
        let p = parse_error("something without a location", "main.luau");
        assert_eq!((p.file.as_str(), p.line), ("main.luau", None));
        let p = parse_error(r"C:\games\x\src\a.luau:7: bad", "main.luau");
        assert_eq!((p.file.as_str(), p.line), ("C:/games/x/src/a.luau", Some(7)));
    }

    #[test]
    fn ok_game_runs_frames() {
        let dir = game(&[
            ("main.luau", "local M = require(\"./src/m\")\nfunction start() M.build() end\nfunction update(dt) local _ = input.text() .. tostring(input.repeated(\"minus\")) end\n"),
            ("src/m.luau", "local M = {}\nfunction M.build() spawn(Mesh.cube(), \"red\") end\nreturn M\n"),
        ]);
        let r = check(&dir, 10);
        assert!(r.ok(), "{:?}", r.errors);
        assert_eq!((r.frames, r.objects, r.files), (10, 1, 2));
        assert!(r.to_json().starts_with("{\"ok\":true,\"errors\":[],\"frames\":10,\"objects\":1"), "{}", r.to_json());
        assert!(!dir.join("error.log").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_syntax_error_in_unrequired_module() {
        let dir = game(&[("main.luau", "function start() end\n"), ("src/broken.luau", "local x = \n\nlocal = 3\n")]);
        let r = check(&dir, 10);
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        assert_eq!(r.errors[0].file, "src/broken.luau");
        assert!(r.errors[0].line.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_runtime_error_in_module() {
        let dir = game(&[
            ("main.luau", "local M = require(\"./src/m\")\nfunction update(dt)\n    if time.frame > 3 then M.fail() end\nend\n"),
            ("src/m.luau", "local M = {}\nfunction M.fail()\n    input.down(\"nokey\")\nend\nreturn M\n"),
        ]);
        let r = check(&dir, 60);
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        let e = &r.errors[0];
        assert_eq!((e.file.as_str(), e.line), ("src/m.luau", Some(3)), "{e:?}");
        assert!(e.message.contains("unknown key 'nokey'"), "{}", e.message);
        assert!(r.frames < 60);
        let json = r.to_json();
        assert!(json.starts_with("{\"ok\":false,\"errors\":[{\"file\":\"src/m.luau\",\"line\":3,"), "{json}");
        assert!(!dir.join("error.log").exists(), "check leaves no error.log behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_input_reaches_scripts() {
        let dir = game(&[("main.luau", "typed = \"\"\nfunction update(dt) typed ..= input.text() if typed == \"hi\" then quit() end end\n")]);
        let mut game = ScriptGame::new(dir.join("main.luau")).hot_reload(false);
        let mut world = World::new();
        game.start(&mut world);
        world.input.text_event("hi\r");
        run_frame(&mut game, &mut world, 1.0 / 60.0);
        assert!(game.error().is_none(), "{:?}", game.error());
        assert!(world.quit_requested());
        assert_eq!(world.input.text, "", "cleared after the frame");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_main() {
        let dir = game(&[("readme.txt", "x")]);
        let r = check(&dir, 1);
        assert!(!r.ok() && r.errors[0].message.contains("no main.luau"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
