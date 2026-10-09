//! Spark Engine - Luau scripting. A game can be one `main.luau` file:
//!
//! ```lua
//! local cube
//! function start()
//!     cube = spawn(Mesh.cube(), "orange", { position = vec3(0, 0.5, 0) })
//! end
//! function update(dt)
//!     cube:rotate_y(dt)
//! end
//! ```
//!
//! Bigger games split into modules (`require("./src/player")`, see `modules.rs`).
//! The game reloads automatically when any of its files is saved.

mod api;
mod audio;
mod convert;
mod draw;
mod flow;
mod light;
mod model;
mod modules;
mod physics;
mod runtime;
mod save;
mod shader;
mod tween;
mod types;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use mlua::{ErrorContext, Function, Lua, MultiValue, Value};
use spark_core::{Align, Color, Game, Layer, TextParams, Vec2, World};

pub use save::default_save_path;

use convert::Shared;
use runtime::SharedRuntime;

/// Runs a Luau script as a [`Game`].
///
/// Frame order: input events -> (collision events -> `fixed_update(dt)` -> physics step) xN ->
/// collision events -> `update(dt)` -> timers & tasks -> `emit_later` events -> `late_update(dt)` -> `render()` (2D drawing)
/// -> render. `start()` runs once as a task (may `wait`).
pub struct ScriptGame {
    path: PathBuf,
    shared: Rc<RefCell<World>>,
    lua: Option<Lua>,
    modified: Option<SystemTime>,
    last_check: Instant,
    error: Option<String>,
    strict: bool,
    hot_reload: bool,
    /// Last `world.time.frame` for which the frame start (timers advance + input events) ran.
    frame_started: u64,
    /// File behind the `save` global (`None` = memory only).
    save_path: Option<PathBuf>,
    /// Last seen modification time of every `.wgsl` file loaded by the script (hot reload).
    shader_mtimes: HashMap<PathBuf, Option<SystemTime>>,
    /// Compile error of a hot-reloaded shader (the old version keeps running).
    shader_error: Option<String>,
    /// Files loaded with `require` by the current run (hot reload).
    modules: Option<modules::SharedModules>,
    /// Max time one callback may run before it is stopped with an error (infinite loop guard).
    timeout: std::time::Duration,
    /// Deadline of the callback running now (checked by the Luau interrupt).
    deadline: Rc<std::cell::Cell<Option<Instant>>>,
}

impl ScriptGame {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            shared: Rc::new(RefCell::new(World::new())),
            lua: None,
            modified: None,
            last_check: Instant::now(),
            error: None,
            strict: false,
            hot_reload: true,
            frame_started: 0,
            save_path: None,
            shader_mtimes: HashMap::new(),
            shader_error: None,
            modules: None,
            timeout: std::time::Duration::from_secs(5),
            deadline: Rc::default(),
        }
    }

    /// Max run time of one script callback (default 5 s). A `while true do end` then stops with
    /// a clear error instead of freezing the game. `None` = no limit.
    pub fn timeout(mut self, limit: Option<std::time::Duration>) -> Self {
        self.timeout = limit.unwrap_or(std::time::Duration::MAX);
        self
    }

    /// Exit the process with code 1 on the first script error (for headless / CI / AI runs).
    pub fn strict(mut self, on: bool) -> Self {
        self.strict = on;
        self
    }

    /// Reload the script when the file changes (default on).
    pub fn hot_reload(mut self, on: bool) -> Self {
        self.hot_reload = on;
        self
    }

    /// Where `save.set(...)` stores data (see [`default_save_path`]). Default: memory only.
    pub fn save_path(mut self, path: Option<PathBuf>) -> Self {
        self.save_path = path;
        self
    }

    /// Last script error, if the script is paused because of one.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn fail(&mut self, msg: String) {
        log::error!("script error: {msg}");
        if self.strict {
            std::process::exit(1);
        }
        log::warn!("script paused - fix the error and save {} to reload", self.path.display());
        // Players' builds have no console: keep a copy next to the script (best effort).
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::write(dir.join("error.log"), format!("{msg}\n"));
        }
        self.error = Some(msg);
    }

    /// Runs `f` with the real world visible to Lua. Errors pause the script (or exit in strict mode).
    fn run(&mut self, world: &mut World, f: impl FnOnce(&Lua) -> mlua::Result<()>) {
        if self.error.is_some() {
            return;
        }
        let Some(lua) = self.lua.clone() else { return };
        std::mem::swap(world, &mut self.shared.borrow_mut());
        self.deadline.set(Instant::now().checked_add(self.timeout));
        let result = f(&lua);
        self.deadline.set(None);
        std::mem::swap(world, &mut self.shared.borrow_mut());
        if let Err(e) = result {
            self.fail(e.to_string());
        }
    }

    fn drop_lua(&mut self) {
        if let Some(old) = self.lua.take() {
            if let Err(e) = save::flush(&old) {
                log::error!("{e}");
            }
            // Release every function / coroutine reference before the state goes away.
            if let Some(rt) = old.remove_app_data::<SharedRuntime>() {
                rt.borrow_mut().clear();
            }
        }
    }

    fn load(&mut self, world: &mut World) {
        self.drop_lua();
        self.error = None;
        self.modified = modified(&self.path);
        self.frame_started = world.time.frame;
        world.reset();
        let root = self.path.parent().map(Path::to_path_buf).unwrap_or_default();
        world.assets.root = root.clone();
        let source = match spark_core::vfs::read_string(&self.path) {
            Ok(s) => s,
            Err(e) => return self.fail(e),
        };
        let modules = modules::new(root);
        self.modules = Some(modules.clone());
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "main.luau".into());
        let lua = Lua::new();
        let deadline = self.deadline.clone();
        let limit = self.timeout;
        lua.set_interrupt(move |_| {
            if deadline.get().is_some_and(|d| Instant::now() > d) {
                // The deadline stays set: a `pcall` around the loop cannot swallow the error, every
                // later check fails too until the callback returns (`run` then clears it).
                return Err(mlua::Error::runtime(format!(
                    "script ran for more than {:.1} s without returning (infinite loop?). \
                     Long work: split it over frames with wait() / task.spawn",
                    limit.as_secs_f32()
                )));
            }
            Ok(mlua::VmState::Continue)
        });
        lua.set_app_data(Shared(self.shared.clone()));
        lua.set_app_data::<SharedRuntime>(Rc::default());
        lua.set_app_data(save::SavePath(self.save_path.clone()));
        self.lua = Some(lua);
        self.run(world, |lua| {
            api::install(lua)?;
            modules::install(lua, &lua.globals(), modules)?;
            lua.load(source.as_str()).set_name(format!("@{name}")).exec()?;
            if matches!(lua.globals().get::<Value>("draw")?, Value::Function(_)) {
                return Err(mlua::Error::runtime(
                    "'draw' is Spark's 2D drawing library (draw.rect, draw.text, ...). \
                     Name your drawing callback 'function render()' instead of 'function draw()'",
                ));
            }
            if let Some(start) = lua.globals().get::<Option<Function>>("start")? {
                runtime::spawn_task(lua, start, MultiValue::new()).map_err(|e| e.context("in start()"))?;
            }
            Ok(())
        });
        if self.error.is_none() {
            log::info!("script loaded: {}", self.path.display());
            let lines = source.lines().count();
            let split = self.modules.as_ref().is_some_and(|m| !m.borrow().files.is_empty());
            if lines > BIG_MAIN && !split && !BIG_HINT.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::info!(
                    "hint: {name} has {lines} lines. Split the game into modules, e.g. src/player.luau with \
                     `return Player` and `local Player = require(\"./src/player\")` (docs/LUAU_API.md, \"Game structure\")"
                );
            }
        }
    }

    /// Hot reload: the script (restarts the game) or single shaders (the game keeps running).
    fn poll_files(&mut self, world: &mut World) {
        if !self.hot_reload || self.last_check.elapsed() < Duration::from_millis(300) {
            return;
        }
        self.last_check = Instant::now();
        let m = modified(&self.path);
        let changed_module = self.modules.as_ref().and_then(|ms| {
            ms.borrow().files.iter().find(|(p, t)| t.is_some() && modified(p) != *t).map(|(p, _)| p.clone())
        });
        if (m.is_some() && m != self.modified) || changed_module.is_some() {
            log::info!("reloading {}", changed_module.as_deref().unwrap_or(&self.path).display());
            self.shader_mtimes.clear();
            self.shader_error = None;
            self.load(world);
            return;
        }
        let files: Vec<(PathBuf, spark_core::ShaderId)> =
            world.assets.shader_files().map(|(p, id)| (p.to_path_buf(), id)).collect();
        for (path, id) in files {
            let m = modified(&path);
            match self.shader_mtimes.get(&path) {
                None => {
                    self.shader_mtimes.insert(path, m);
                }
                Some(old) if *old != m => {
                    self.shader_mtimes.insert(path.clone(), m);
                    let result = spark_core::vfs::read_string(&path).and_then(|code| {
                        world.assets.shader_mut(id).ok_or_else(|| "shader was unloaded".to_string())?.recompile(&code)
                    });
                    match result {
                        Ok(()) => {
                            log::info!("shader reloaded: {}", path.display());
                            self.shader_error = None;
                        }
                        Err(e) => {
                            log::error!("{e}");
                            self.shader_error = Some(e);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Step 1 of the frame, exactly once per frame (whichever callback comes first).
    fn ensure_frame_started(&mut self, world: &mut World) {
        if self.frame_started == world.time.frame {
            return;
        }
        self.poll_files(world);
        self.frame_started = world.time.frame;
        let dt = world.time.dt as f64;
        self.run(world, |lua| runtime::begin_frame(lua, dt));
    }
}

/// Red error screen shown while the script is paused (players see what went wrong without a console).
fn draw_error(world: &mut World, heading: &str, msg: &str, hint_text: &str) {
    let c = &mut world.canvas;
    let prev = c.layer();
    c.set_layer(Layer::Ui);
    let size = c.size();
    let pad = (size.y * 0.05).max(16.0);
    c.rect(Vec2::ZERO, size, Color::rgba(0.07, 0.01, 0.01, 0.92), Align::TOP_LEFT);
    let title = TextParams { size: 30.0, color: Color::rgb(1.0, 0.38, 0.32), ..Default::default() };
    c.text(heading, Vec2::new(pad, pad), &title);
    let mut text: String = msg.chars().take(2500).collect();
    if text.len() < msg.len() {
        text.push_str(" ...");
    }
    let body = TextParams { size: 17.0, width: Some(size.x - pad * 2.0), line_height: 1.15, ..Default::default() };
    c.text(&text, Vec2::new(pad, pad + 48.0), &body);
    let hint = TextParams {
        size: 15.0,
        color: Color::rgb(0.7, 0.66, 0.64),
        align: Align { x: 0.0, y: 1.0 },
        width: Some(size.x - pad * 2.0),
        ..Default::default()
    };
    c.text(hint_text, Vec2::new(pad, size.y - pad), &hint);
    c.set_layer(prev);
}

fn modified(path: &Path) -> Option<SystemTime> {
    spark_core::vfs::modified(path)
}

/// `main.luau` longer than this (and no modules) gets a one-time hint to split it.
const BIG_MAIN: usize = 800;
static BIG_HINT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn call_global(lua: &Lua, name: &str, dt: f32) -> mlua::Result<()> {
    if let Some(f) = lua.globals().get::<Option<Function>>(name)? {
        f.call::<()>(dt)?;
    }
    Ok(())
}

impl Game for ScriptGame {
    fn start(&mut self, world: &mut World) {
        self.load(world);
    }

    fn fixed_update(&mut self, world: &mut World, dt: f32) {
        self.ensure_frame_started(world);
        self.run(world, |lua| {
            physics::dispatch_collisions(lua)?;
            call_global(lua, "fixed_update", dt)
        });
    }

    fn update(&mut self, world: &mut World, dt: f32) {
        self.ensure_frame_started(world);
        self.run(world, |lua| {
            physics::dispatch_collisions(lua)?;
            call_global(lua, "update", dt)?;
            runtime::run_timers(lua)?;
            runtime::flush_deferred(lua)
        });
    }

    fn late_update(&mut self, world: &mut World, dt: f32) {
        self.run(world, |lua| call_global(lua, "late_update", dt));
    }

    fn draw(&mut self, world: &mut World) {
        if let Some(msg) = &self.error {
            let file = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let hint = format!("Fix the error and save the file ({file} or a module): the game reloads automatically. (A copy is in error.log.)");
            draw_error(world, "Script error", msg, &hint);
            return;
        }
        self.run(world, |lua| {
            if let Some(f) = lua.globals().get::<Option<Function>>("render")? {
                f.call::<()>(()).map_err(|e| e.context("in render()"))?;
            }
            save::flush(lua)
        });
        if let Some(msg) = &self.shader_error {
            draw_error(world, "Shader error", msg, "Fix the shader and save it: it reloads automatically (the previous version keeps running).");
        }
    }
}

impl Drop for ScriptGame {
    fn drop(&mut self) {
        self.drop_lua();
    }
}
