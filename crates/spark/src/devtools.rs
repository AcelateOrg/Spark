//! `--devtools ADDR`: the runner's side of the live link to Spark Editor (see `spark-devtools`).
//!
//! Every frame the runner calls [`Devtools::begin_frame`] (applies editor commands, decides whether game
//! logic runs) and [`Devtools::end_frame`] (sends stats / scene snapshots, throttled). All socket I/O is
//! on background threads; a missing or crashed editor never affects the game.

use std::time::{Duration, Instant};

use spark_core::{VERSION, World};
use spark_devtools::{EditorCommand, GameMessage, Link, PROTOCOL_VERSION, SceneSnapshot, Stats};
use spark_render::FrameStats;

const STATS_EVERY: Duration = Duration::from_millis(100);
const MIN_SCENE_EVERY: Duration = Duration::from_millis(16);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct Devtools {
    link: Link<GameMessage, EditorCommand>,
    paused: bool,
    /// Frames of logic still to run while paused (`Step`).
    steps: u32,
    scene_every: Option<Duration>,
    last_scene: Option<Instant>,
    last_stats: Option<Instant>,
    last_error: Option<String>,
}

impl Devtools {
    /// Connects to the editor. `None` (with a warning) when it is not reachable: the game runs normally.
    pub fn connect(addr: &str, gpu: &str) -> Option<Self> {
        match Link::connect(addr, CONNECT_TIMEOUT) {
            Ok(link) => {
                link.send(GameMessage::Hello {
                    protocol: PROTOCOL_VERSION,
                    engine: VERSION.to_string(),
                    pid: std::process::id(),
                    gpu: gpu.to_string(),
                });
                log::info!("devtools: connected to {addr}");
                Some(Self { link, paused: false, steps: 0, scene_every: None, last_scene: None, last_stats: None, last_error: None })
            }
            Err(e) => {
                log::warn!("devtools: cannot connect to {addr}: {e}");
                None
            }
        }
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.link.send(GameMessage::Paused { paused });
        }
    }

    /// Applies the editor's commands. Returns whether game logic runs this frame.
    pub fn begin_frame(&mut self, world: &mut World) -> bool {
        let commands: Vec<EditorCommand> = self.link.drain().collect();
        for cmd in commands {
            match cmd {
                EditorCommand::Pause { paused } => {
                    self.steps = 0;
                    self.set_paused(paused);
                }
                EditorCommand::Step { frames } => {
                    self.set_paused(true);
                    self.steps = self.steps.saturating_add(frames.max(1));
                }
                EditorCommand::Reload => {
                    world.request_restart();
                    // The script host restarts at the start of a logic frame: run one even while paused.
                    if self.paused {
                        self.steps = self.steps.max(1);
                    }
                }
                EditorCommand::WatchScene { interval_ms } => {
                    self.scene_every = interval_ms.map(|ms| Duration::from_millis(ms as u64).max(MIN_SCENE_EVERY));
                    self.last_scene = None;
                }
                EditorCommand::Overlay { on } => world.debug.set_enabled(on),
                EditorCommand::Quit => world.quit(),
            }
        }
        if !self.paused {
            return true;
        }
        if self.steps > 0 {
            self.steps -= 1;
            return true;
        }
        false
    }

    /// Sends what changed: script errors at once, stats ~10x per second, the scene when watched.
    pub fn end_frame(&mut self, world: &World, render: FrameStats, error: Option<&str>) {
        if error != self.last_error.as_deref() {
            self.last_error = error.map(str::to_owned);
            self.link.send(GameMessage::ScriptError { message: self.last_error.clone() });
        }
        let now = Instant::now();
        let due = |last: Option<Instant>, every: Duration| last.is_none_or(|t| now - t >= every);
        if due(self.last_stats, STATS_EVERY) {
            self.last_stats = Some(now);
            let d = &world.debug;
            self.link.send(GameMessage::Stats(Stats {
                frame: world.time.frame,
                time: world.time.elapsed,
                fps: d.avg_fps(),
                frame_ms: d.avg_frame_ms(),
                min_frame_ms: d.min_frame_ms,
                max_frame_ms: d.max_frame_ms,
                objects: world.scene.len(),
                draw_calls: render.draw_calls,
                culled: render.culled,
                triangles: render.triangles,
                internal_size: render.internal_size,
                paused: self.paused,
            }));
        }
        if let Some(every) = self.scene_every {
            if due(self.last_scene, every) {
                self.last_scene = Some(now);
                match SceneSnapshot::from_engine_json(world.time.frame, &world.scene.dump_json()) {
                    Ok(scene) => {
                        self.link.send(GameMessage::Scene(scene));
                    }
                    Err(e) => log::warn!("devtools: {e}"),
                }
            }
        }
    }

    /// Best effort "the game is closing" (the editor also notices the process exit).
    pub fn bye(&self) {
        if self.link.send(GameMessage::Bye) {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}
