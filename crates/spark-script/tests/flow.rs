//! Runs Luau contract scripts headless (no GPU): each must call quit() without errors.

use spark_core::{Game, World, run_frame};
use spark_script::ScriptGame;

fn run_script(name: &str, max_frames: u32) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts").join(name);
    let mut game = ScriptGame::new(path).hot_reload(false);
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
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/infinite_loop.luau");
    let mut game = ScriptGame::new(path).hot_reload(false).timeout(Some(std::time::Duration::from_millis(200)));
    let mut world = World::new();
    game.start(&mut world);
    let t = std::time::Instant::now();
    run_frame(&mut game, &mut world, 1.0 / 60.0);
    let e = game.error().expect("the loop must be stopped with an error");
    assert!(e.contains("infinite loop"), "{e}");
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
}
