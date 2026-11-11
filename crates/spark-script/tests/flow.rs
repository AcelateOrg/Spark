//! Runs Luau contract scripts headless (no GPU): each must call quit() without errors.

use spark_core::{Game, World, run_frame};
use spark_script::ScriptGame;

fn run_script(name: &str, max_frames: u32) {
    run_script_at(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts").join(name), max_frames);
}

fn run_script_at(path: &std::path::Path, max_frames: u32) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let mut game = ScriptGame::new(path.to_path_buf()).hot_reload(false);
    let mut world = World::new();
    world.physics.set_backend(Box::new(spark_physics::RapierBackend::new()));
    game.start(&mut world);
    for _ in 0..max_frames {
        if world.quit_requested() || game.error().is_some() {
            break;
        }
        run_frame(&mut game, &mut world, 1.0 / 60.0);
    }
    if let Some(e) = game.error() {
        panic!("{name} failed:\n{e}");
    }
    assert!(world.quit_requested(), "{name} did not finish within {max_frames} frames");
}

#[test]
fn flow_contract() {
    run_script("flow.luau", 2000);
}

#[test]
fn physics_contract() {
    run_script("physics.luau", 2000);
}

#[test]
fn draw_contract() {
    run_script("draw.luau", 100);
}

#[test]
fn pbr_contract() {
    run_script("pbr.luau", 10);
}

/// Runs from a temp folder next to generated images (a valid 6:1 cubemap strip and a square one).
#[test]
fn sky_contract() {
    use spark_core::{Color, ImageData};
    let dir = std::env::temp_dir().join(format!("spark-sky-contract-{}", std::process::id()));
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/sky.luau");
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::copy(src, dir.join("sky.luau")).unwrap();
    ImageData::checker(12, 6, Color::WHITE, Color::BLACK).save_png(&dir.join("assets/not_a_strip.png")).unwrap();
    let strip = ImageData::new(12, 2, [40, 90, 200, 255].repeat(24));
    strip.save_png(&dir.join("assets/sky_strip.png")).unwrap();
    run_script_at(&dir.join("sky.luau"), 10);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn models_contract() {
    run_script("models.luau", 2000);
}

#[test]
fn shaders_contract() {
    run_script("shaders.luau", 100);
}

#[test]
fn modules_contract() {
    run_script("modules/main.luau", 100);
}

#[test]
fn render_api_contract() {
    run_script("render_api.luau", 10);
}

#[test]
fn infinite_loop_is_stopped() {
    for script in ["infinite_loop.luau", "pcall_loop.luau"] {
        stopped(script);
    }
}

fn stopped(script: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts").join(script);
    let mut game = ScriptGame::new(path).hot_reload(false).timeout(Some(std::time::Duration::from_millis(200)));
    let mut world = World::new();
    game.start(&mut world);
    let t = std::time::Instant::now();
    run_frame(&mut game, &mut world, 1.0 / 60.0);
    let e = game.error().expect("the loop must be stopped with an error");
    assert!(e.contains("infinite loop"), "{script}: {e}");
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn debug_contract() {
    run_script("debug.luau", 10);
}

#[test]
fn postfx_contract() {
    run_script("postfx.luau", 10);
}

#[test]
fn ui_contract() {
    run_script("ui.luau", 10);
}
