//! `spark test path/to/game [NAME...] [--update] [--json] [--max-time S]`: scenario tests.
//!
//! Every `tests/**/*.luau` file is one test. For each, the game starts fresh (headless, physics on,
//! no audio / GPU / save file, fixed 1/60 s steps) and the test file runs as a task next to it with
//! the `test` global: it presses keys, waits, checks state and takes snapshots of the world.
//!
//! A test fails on a failed check, on ANY script error of the game, on timeout, or when a
//! snapshot differs from `tests/snapshots/<test>.<name>.snap` (shown as a diff). New snapshots are
//! written on the first run; `--update` rewrites changed ones (after an intended change).

use std::path::{Path, PathBuf};
use std::time::Instant;

use engine::script::testing::{self, FAIL_PREFIX};
use engine::script::{ScriptGame, TaskStatus};
use engine::{Game, World, run_frame};

use crate::check::{Problem, parse_error};

const USAGE: &str = "usage: spark test path/to/game [NAME...] [--update] [--json] [--max-time SECONDS]";
const DT: f32 = 1.0 / 60.0;
const SNAP_DIR: &str = "snapshots";

#[derive(Debug)]
pub struct TestResult {
    /// `tests/bridge.luau`
    pub name: String,
    pub ok: bool,
    pub error: Option<Problem>,
    /// Last `test.step(...)` when it failed.
    pub step: Option<String>,
    /// Snapshot diffs: `(snapshot name, lines)`.
    pub diffs: Vec<(String, Vec<String>)>,
    /// Snapshots written for the first time / updated.
    pub written: Vec<String>,
    pub game_seconds: f32,
    pub real_seconds: f32,
}

pub struct Options {
    pub filters: Vec<String>,
    pub update: bool,
    pub max_time: f32,
}

pub fn run(args: &[String]) -> i32 {
    let mut game: Option<PathBuf> = None;
    let mut opts = Options { filters: Vec::new(), update: false, max_time: 300.0 };
    let mut json = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json = true,
            "--update" | "-u" => opts.update = true,
            "--max-time" => match it.next().and_then(|n| n.parse().ok()) {
                Some(n) => opts.max_time = n,
                None => {
                    eprintln!("spark test: --max-time needs seconds of game time, e.g. --max-time 600\n{USAGE}");
                    return 2;
                }
            },
            "--help" | "-h" => {
                println!("{USAGE}\n\n{HELP}");
                return 0;
            }
            _ if a.starts_with("--") => {
                eprintln!("spark test: unexpected argument '{a}'\n{USAGE}");
                return 2;
            }
            _ if game.is_none() && Path::new(a).join("main.luau").is_file() => game = Some(PathBuf::from(a)),
            _ => opts.filters.push(a.clone()),
        }
    }
    let game = game.unwrap_or_else(|| PathBuf::from("."));
    let Some(dir) = game.canonicalize().ok().filter(|d| d.join("main.luau").is_file()) else {
        eprintln!("spark test: no main.luau in '{}'\n{USAGE}", game.display());
        return 2;
    };
    let files = test_files(&dir);
    if files.is_empty() {
        eprintln!("spark test: no tests in {} (create tests/<name>.luau)\n\n{HELP}", dir.join("tests").display());
        return 2;
    }
    let selected: Vec<PathBuf> = files
        .into_iter()
        .filter(|f| opts.filters.is_empty() || opts.filters.iter().any(|q| rel(&dir, f).contains(q.as_str())))
        .collect();
    if selected.is_empty() {
        eprintln!("spark test: no test matches {:?}", opts.filters);
        return 2;
    }
    let results: Vec<TestResult> = selected.iter().map(|f| run_test(&dir, f, &opts)).collect();
    let failed = results.iter().filter(|r| !r.ok).count();
    if json {
        println!("{}", to_json(&results));
    } else {
        for r in &results {
            print_result(r);
        }
        println!("spark test: {} passed, {failed} failed", results.len() - failed);
    }
    if failed == 0 { 0 } else { 1 }
}

const HELP: &str = "\
A test is a .luau file in tests/. It runs next to the game like a player would:
  test.step(\"talk to anna\")          name of the current step (shown on failure)
  test.tap(\"e\") test.hold(\"w\", 1.5)  press keys (also test.press / test.release)
  test.click(x, y) test.type(\"hi\")   mouse and text
  test.wait(2) test.frames(10)        let the game run
  test.wait_until(fn, 5, \"door open\") wait for a condition (fails after 5 s)
  test.find(\"door\")                   wait for an object and return it
  test.expect(cond, msg) test.eq(a, b, msg) test.near(a, b, eps, msg) test.fail(msg)
  test.snapshot(\"after_door\", { flags = my_flags })   compare the world with tests/snapshots/
  Game globals are visible: call your own checkpoint helpers (e.g. goto_chapter(3)).
  Any script error of the game fails the test.";

fn rel(dir: &Path, f: &Path) -> String {
    f.strip_prefix(dir).unwrap_or(f).to_string_lossy().replace('\\', "/")
}

/// `tests/**/*.luau`, sorted, without `tests/snapshots` and files starting with `_` (helpers).
fn test_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(d) else { return };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.starts_with('.') || name.starts_with('_') {
                continue;
            }
            if p.is_dir() {
                if name != SNAP_DIR {
                    walk(&p, out);
                }
            } else if name.ends_with(".luau") && !name.ends_with(".d.luau") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(&dir.join("tests"), &mut out);
    out
}

/// `tests/ch3/bridge.luau` + `after` -> `tests/snapshots/ch3.bridge.after.snap`.
fn snapshot_path(dir: &Path, test: &str, snap: &str) -> PathBuf {
    let stem = test.trim_start_matches("tests/").trim_end_matches(".luau").replace('/', ".");
    let safe: String = snap.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    dir.join("tests").join(SNAP_DIR).join(format!("{stem}.{safe}.snap"))
}

pub fn run_test(dir: &Path, file: &Path, opts: &Options) -> TestResult {
    let name = rel(dir, file);
    let started = Instant::now();
    let error_log = dir.join("error.log");
    let had_error_log = error_log.exists();
    let mut game = ScriptGame::new(dir.join("main.luau")).hot_reload(false).test(file, name.clone());
    let mut world = World::new();
    world.physics.set_backend(Box::new(engine::physics::RapierBackend::new()));
    game.start(&mut world);
    let max_frames = (opts.max_time / DT).ceil() as u64;
    let mut timeout = false;
    let mut quit_early = false;
    loop {
        if game.error().is_some() || matches!(game.test_status(), Some(TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled)) {
            break;
        }
        if world.quit_requested() {
            quit_early = true;
            break;
        }
        if world.time.frame >= max_frames {
            timeout = true;
            break;
        }
        for i in game.take_test_input() {
            i.apply(&mut world);
        }
        run_frame(&mut game, &mut world, DT);
    }
    if !had_error_log {
        let _ = std::fs::remove_file(&error_log);
    }
    let mut r = TestResult {
        name: name.clone(),
        ok: false,
        error: None,
        step: game.test_step(),
        diffs: Vec::new(),
        written: Vec::new(),
        game_seconds: world.time.frame as f32 * DT,
        real_seconds: started.elapsed().as_secs_f32(),
    };
    if let Some(e) = game.error() {
        let mut p = parse_error(e, &name);
        let failed_check = e.contains(FAIL_PREFIX);
        p.message = short_trace(&p.message.replacen(FAIL_PREFIX, "", 1), failed_check);
        if !failed_check {
            p.message = format!("game error: {}", p.message);
        }
        r.error = Some(p);
        return r;
    }
    if timeout {
        r.error = Some(Problem {
            file: name,
            line: None,
            message: format!("the test did not finish within {} s of game time (stuck? raise --max-time)", opts.max_time),
        });
        return r;
    }
    if quit_early && game.test_status() != Some(TaskStatus::Done) {
        r.error = Some(Problem { file: name, line: None, message: "the game called quit() before the test finished".into() });
        return r;
    }
    if game.test_status() == Some(TaskStatus::Cancelled) {
        r.error = Some(Problem { file: name, line: None, message: "the test task was cancelled".into() });
        return r;
    }
    // Snapshots.
    for (snap, text) in game.test_snapshots() {
        let path = snapshot_path(dir, &name, &snap);
        match std::fs::read_to_string(&path) {
            Ok(old) => {
                let changes = testing::diff(&old, &text);
                if changes.is_empty() {
                    continue;
                }
                if opts.update {
                    if let Err(e) = std::fs::write(&path, &text) {
                        r.error = Some(Problem { file: rel(dir, &path), line: None, message: format!("cannot write: {e}") });
                        return r;
                    }
                    r.written.push(snap);
                } else {
                    r.diffs.push((snap, changes.iter().map(|c| c.to_string()).collect()));
                }
            }
            Err(_) => {
                let written = path.parent().map(std::fs::create_dir_all).transpose().and_then(|_| std::fs::write(&path, &text));
                if let Err(e) = written {
                    r.error = Some(Problem { file: rel(dir, &path), line: None, message: format!("cannot write: {e}") });
                    return r;
                }
                r.written.push(snap);
            }
        }
    }
    r.ok = r.diffs.is_empty();
    r
}

/// Failed checks: no traceback (the location says it all). Game errors: only the game's own frames.
fn short_trace(msg: &str, failed_check: bool) -> String {
    let Some((head, trace)) = msg.split_once("stack traceback:") else { return msg.trim().to_string() };
    let head = head.trim();
    if failed_check {
        return head.to_string();
    }
    let frames: Vec<&str> = trace
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("[C]") && !l.starts_with("test:") && !l.starts_with("[string"))
        .take(6)
        .collect();
    if frames.is_empty() { head.to_string() } else { format!("{head}\nstack: {}", frames.join(" <- ")) }
}

fn print_result(r: &TestResult) {
    let time = format!("{:.1} s game, {:.1} s real", r.game_seconds, r.real_seconds);
    if r.ok {
        println!("PASS {} ({time})", r.name);
    } else {
        println!("FAIL {} ({time})", r.name);
    }
    if let Some(e) = &r.error {
        match e.line {
            Some(l) => println!("  {}:{l}: {}", e.file, indent(&e.message)),
            None => println!("  {}: {}", e.file, indent(&e.message)),
        }
    }
    if !r.ok {
        if let Some(s) = &r.step {
            println!("  at step: {s}");
        }
    }
    for (snap, lines) in &r.diffs {
        println!("  snapshot '{snap}' changed ({} lines; if intended: spark test --update):", lines.len());
        for l in lines.iter().take(40) {
            println!("    {l}");
        }
        if lines.len() > 40 {
            println!("    ... {} more", lines.len() - 40);
        }
    }
    for s in &r.written {
        println!("  snapshot '{s}' written");
    }
}

fn indent(s: &str) -> String {
    // Keep the first lines of a traceback, indented under the test.
    let lines: Vec<&str> = s.lines().take(8).collect();
    lines.join("\n    ")
}

fn js(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn to_json(results: &[TestResult]) -> String {
    let items: Vec<String> = results
        .iter()
        .map(|r| {
            let error = r.error.as_ref().map_or("null".into(), |e| {
                format!(
                    "{{\"file\":{},\"line\":{},\"message\":{}}}",
                    js(&e.file),
                    e.line.map_or("null".into(), |l| l.to_string()),
                    js(&e.message)
                )
            });
            let diffs: Vec<String> = r
                .diffs
                .iter()
                .map(|(n, l)| format!("{{\"snapshot\":{},\"changes\":[{}]}}", js(n), l.iter().map(|x| js(x)).collect::<Vec<_>>().join(",")))
                .collect();
            format!(
                "{{\"test\":{},\"ok\":{},\"error\":{error},\"step\":{},\"diffs\":[{}],\"game_seconds\":{:.2}}}",
                js(&r.name),
                r.ok,
                r.step.as_deref().map_or("null".into(), js),
                diffs.join(","),
                r.game_seconds
            )
        })
        .collect();
    let failed = results.iter().filter(|r| !r.ok).count();
    format!("{{\"ok\":{},\"passed\":{},\"failed\":{failed},\"tests\":[{}]}}", failed == 0, results.len() - failed, items.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(files: &[(&str, &str)]) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("spark-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        for (rel, text) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dir.canonicalize().unwrap()
    }

    fn opts() -> Options {
        Options { filters: Vec::new(), update: false, max_time: 30.0 }
    }

    const DOOR_GAME: &str = r#"
opened = false
function start()
    spawn(Mesh.cube(), "gray", { name = "door", position = vec3(0, 1, 0) })
end
function update(dt)
    if input.pressed("e") then
        opened = true
        find("door").position = vec3(0, 3, 0)
    end
end
"#;

    #[test]
    fn passing_test_with_input_and_snapshot() {
        let dir = game(&[
            ("main.luau", DOOR_GAME),
            (
                "tests/door.luau",
                "test.step('open')\nlocal d = test.find('door')\ntest.eq(opened, false)\ntest.tap('e')\ntest.eq(opened, true, 'door opened')\ntest.near(d.position, vec3(0, 3, 0))\ntest.snapshot('open', { opened = opened })\n",
            ),
        ]);
        let file = dir.join("tests/door.luau");
        let r = run_test(&dir, &file, &opts());
        assert!(r.ok, "{r:?}");
        assert_eq!(r.written, vec!["open".to_string()]);
        let snap = std::fs::read_to_string(dir.join("tests/snapshots/door.open.snap")).unwrap();
        assert!(snap.contains("obj \"door\" = pos (0.00, 3.00, 0.00)"), "{snap}");
        assert!(snap.contains("state.opened = true"), "{snap}");
        // Second run: same snapshot, nothing written.
        let r = run_test(&dir, &file, &opts());
        assert!(r.ok && r.written.is_empty(), "{r:?}");
        // The game changes: the snapshot diff fails the test.
        std::fs::write(dir.join("main.luau"), DOOR_GAME.replace("vec3(0, 3, 0)", "vec3(0, 5, 0)")).unwrap();
        std::fs::write(&file, "test.find('door') test.tap('e') test.snapshot('open', { opened = opened })\n").unwrap();
        let r = run_test(&dir, &file, &opts());
        assert!(!r.ok);
        assert_eq!(r.diffs.len(), 1, "{r:?}");
        assert!(r.diffs[0].1[0] == "~ obj \"door\": pos (0.00, 3.00, 0.00) -> (0.00, 5.00, 0.00)", "{:?}", r.diffs);
        // --update accepts it.
        let r = run_test(&dir, &file, &Options { update: true, ..opts() });
        assert!(r.ok && r.written == vec!["open".to_string()], "{r:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_check_has_location_and_step() {
        let dir = game(&[("main.luau", DOOR_GAME), ("tests/door.luau", "test.step('check')\ntest.wait(0.1)\ntest.eq(opened, true, 'door opened')\n")]);
        let r = run_test(&dir, &dir.join("tests/door.luau"), &opts());
        assert!(!r.ok);
        let e = r.error.unwrap();
        assert_eq!((e.file.as_str(), e.line), ("tests/door.luau", Some(3)), "{e:?}");
        assert!(e.message.starts_with("door opened: expected true, got false"), "{}", e.message);
        assert_eq!(r.step.as_deref(), Some("check"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn game_error_fails_the_test() {
        let dir = game(&[
            ("main.luau", "function update(dt) if input.pressed('e') then local x = nil; x.y = 1 end end\n"),
            ("tests/t.luau", "test.tap('e')\ntest.wait(1)\n"),
        ]);
        let r = run_test(&dir, &dir.join("tests/t.luau"), &opts());
        let e = r.error.unwrap();
        assert!(e.message.starts_with("game error:"), "{e:?}");
        assert_eq!((e.file.as_str(), e.line), ("main.luau", Some(1)), "{e:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wait_until_times_out() {
        let dir = game(&[("main.luau", DOOR_GAME), ("tests/t.luau", "test.wait_until(function() return opened end, 0.5, 'door to open')\n")]);
        let r = run_test(&dir, &dir.join("tests/t.luau"), &opts());
        let e = r.error.unwrap();
        assert!(e.message.contains("timed out after 0.5 s waiting for door to open"), "{e:?}");
        assert!(r.game_seconds < 1.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_test_files() {
        let dir = game(&[("main.luau", ""), ("tests/a.luau", ""), ("tests/ch1/b.luau", ""), ("tests/_helpers.luau", ""), ("tests/snapshots/x.luau", "")]);
        let names: Vec<String> = test_files(&dir).iter().map(|f| rel(&dir, f)).collect();
        assert_eq!(names, vec!["tests/a.luau", "tests/ch1/b.luau"]);
        assert_eq!(snapshot_path(&dir, "tests/ch1/b.luau", "after door"), dir.join("tests/snapshots/ch1.b.after_door.snap"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
