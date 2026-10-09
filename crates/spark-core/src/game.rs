use crate::world::World;

/// A game. Implement this and pass it to `spark::App::run`.
///
/// Every frame runs in this exact order:
/// 1. input is collected
/// 2. `fixed_update` 0..N times (fixed step `world.time.fixed_dt`, for physics / deterministic logic),
///    each one followed by a physics step (collisions land in `world.physics.events`)
/// 3. `update` once (variable `dt`)
/// 4. animations advance (models spawned from glTF files are posed and skinned)
/// 5. `late_update` once (cameras, things that follow other things)
/// 6. `draw` once (2D drawing on `world.canvas`; the canvas is cleared at the start of every frame)
/// 7. queued sounds are sent to the audio backend
/// 8. render
///
/// All `dt` values are scaled by `world.time.scale`.
pub trait Game {
    /// Called once before the first frame. Build the scene here.
    fn start(&mut self, _world: &mut World) {}

    /// Called 0..N times per frame with a constant `dt` (default 1/60 s).
    fn fixed_update(&mut self, _world: &mut World, _dt: f32) {}

    /// Called every frame. `dt` = seconds since the previous frame.
    fn update(&mut self, _world: &mut World, _dt: f32) {}

    /// Called every frame after `update`.
    fn late_update(&mut self, _world: &mut World, _dt: f32) {}

    /// Called every frame after `late_update`: draw 2D shapes, sprites and text on `world.canvas`.
    fn draw(&mut self, _world: &mut World) {}
}

/// Runs one frame of game logic (steps 2-7). Used by every runner so they behave identically.
pub fn run_frame<G: Game + ?Sized>(game: &mut G, world: &mut World, real_dt: f32) {
    world.time.advance(real_dt);
    let (w, h) = world.canvas.output_size();
    let internal = world.render.internal_size(w, h);
    world.canvas.begin_frame(internal);
    world.branding.begin_frame();
    world.physics.events.clear();
    let steps = world.time.fixed_steps();
    let fixed = world.time.fixed_dt;
    for _ in 0..steps {
        game.fixed_update(world, fixed);
        crate::physics::step(world, fixed);
    }
    let dt = world.time.dt;
    game.update(world, dt);
    crate::animation::update(world, dt);
    game.late_update(world, dt);
    game.draw(world);
    world.input.end_frame();
    crate::audio::flush(world);
}
