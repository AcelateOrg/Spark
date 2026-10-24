//! The game session: starts `spark <game> --devtools ADDR`, talks to it over the devtools link, and the two
//! panels that show what happens inside the game (Debug: state, controls, frame times; Scene: objects).
//!
//! Also runs `spark check --json` and `spark build` for the project.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Stroke, pos2, vec2};
use spark_devtools::{EditorCommand, GameMessage, Link, ObjectInfo, SceneSnapshot, Server, Stats};

use super::Action;
use crate::i18n::{t, tf};
use crate::process::{CheckReport, Job, JobKind, JobState, Level, LogLine, editor_line, find_location};
use crate::theme::{self, icon};

/// Frame times kept for the graph.
const HISTORY: usize = 300;
/// How often the game sends the scene while the editor is connected.
const SCENE_INTERVAL_MS: u32 = 250;
/// `Quit` was sent: kill the game if it is still alive after this long.
const QUIT_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameState {
    Stopped,
    /// Process started, devtools link not connected yet.
    Starting,
    Running,
    Paused,
    /// `Quit` sent, waiting for the process to exit.
    Stopping,
}

/// Something the workspace reacts to.
pub enum SessionEvent {
    CheckDone(Option<CheckReport>),
    ScriptError(Option<String>),
    GameExited,
    BuildDone(bool),
}

struct Hello {
    engine: String,
    gpu: String,
    pid: u32,
}

pub struct GameSession {
    root: PathBuf,
    run: Option<Job>,
    check: Option<Job>,
    build: Option<Job>,
    server: Option<Server>,
    link: Option<Link<EditorCommand, GameMessage>>,
    hello: Option<Hello>,
    stats: Option<Stats>,
    frame_ms: VecDeque<f32>,
    scene: Option<SceneSnapshot>,
    paused: bool,
    overlay: bool,
    script_error: Option<String>,
    quit_sent: Option<Instant>,
    /// Start again once the current game exited (Restart).
    restart: Option<PathBuf>,
    selected: Option<u32>,
    scene_filter: String,
}

impl GameSession {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            run: None,
            check: None,
            build: None,
            server: None,
            link: None,
            hello: None,
            stats: None,
            frame_ms: VecDeque::new(),
            scene: None,
            paused: false,
            overlay: false,
            script_error: None,
            quit_sent: None,
            restart: None,
            selected: None,
            scene_filter: String::new(),
        }
    }

    pub fn state(&self) -> GameState {
        match &self.run {
            Some(job) if job.is_running() => {
                if self.quit_sent.is_some() || self.restart.is_some() {
                    GameState::Stopping
                } else if self.link.is_none() {
                    GameState::Starting
                } else if self.paused {
                    GameState::Paused
                } else {
                    GameState::Running
                }
            }
            _ => GameState::Stopped,
        }
    }

    pub fn is_running(&self) -> bool {
        self.state() != GameState::Stopped
    }

    pub fn is_connected(&self) -> bool {
        self.link.is_some()
    }

    pub fn is_checking(&self) -> bool {
        self.check.as_ref().is_some_and(Job::is_running)
    }

    pub fn is_building(&self) -> bool {
        self.build.as_ref().is_some_and(Job::is_running)
    }

    /// Anything that needs regular repaints (output, stats).
    pub fn is_busy(&self) -> bool {
        self.is_running() || self.is_checking() || self.is_building()
    }

    pub fn fps(&self) -> Option<f32> {
        self.stats.as_ref().filter(|_| self.link.is_some()).map(|s| s.fps)
    }

    /// Starts the game (restarts it when it is already running).
    pub fn run(&mut self, spark: &Path, clock: &Instant) -> Result<Vec<LogLine>, String> {
        if self.is_running() {
            self.stop();
            self.restart = Some(spark.to_path_buf());
            return Ok(vec![editor_line(JobKind::Run, Level::Info, t("game.restarting"), clock)]);
        }
        let server = Server::bind_local().map_err(|e| format!("devtools: {e}"))?;
        let args = vec![self.root.display().to_string(), spark_devtools::FLAG.to_string(), server.addr().to_string()];
        let job = Job::spawn(JobKind::Run, spark, &args, &self.root)?;
        let line = editor_line(JobKind::Run, Level::Info, tf("game.started", &[("pid", &job.pid())]), clock);
        self.reset_live_state();
        self.server = Some(server);
        self.run = Some(job);
        Ok(vec![line])
    }

    fn reset_live_state(&mut self) {
        self.link = None;
        self.server = None;
        self.hello = None;
        self.stats = None;
        self.frame_ms.clear();
        self.scene = None;
        self.paused = false;
        self.script_error = None;
        self.quit_sent = None;
    }

    /// Asks the game to quit (killed after [`QUIT_GRACE`] if it does not).
    pub fn stop(&mut self) {
        self.restart = None;
        let Some(job) = self.run.as_mut().filter(|j| j.is_running()) else { return };
        match &self.link {
            Some(link) if link.send(EditorCommand::Quit) => self.quit_sent = Some(Instant::now()),
            _ => job.kill(),
        }
    }

    fn send(&self, cmd: EditorCommand) {
        if let Some(link) = &self.link {
            link.send(cmd);
        }
    }

    pub fn toggle_pause(&mut self) {
        self.send(EditorCommand::Pause { paused: !self.paused });
    }

    pub fn step(&mut self) {
        if !self.paused {
            self.send(EditorCommand::Pause { paused: true });
        }
        self.send(EditorCommand::Step { frames: 1 });
    }

    pub fn reload(&mut self) {
        self.send(EditorCommand::Reload);
    }

    pub fn set_overlay(&mut self, on: bool) {
        self.overlay = on;
        self.send(EditorCommand::Overlay { on });
    }

    pub fn check(&mut self, spark: &Path, clock: &Instant) -> Result<LogLine, String> {
        if self.is_checking() {
            return Err(t("game.check_running").into());
        }
        let args = vec!["check".to_string(), self.root.display().to_string(), "--json".to_string()];
        self.check = Some(Job::spawn(JobKind::Check, spark, &args, &self.root)?);
        Ok(editor_line(JobKind::Check, Level::Info, t("game.checking"), clock))
    }

    pub fn build(&mut self, spark: &Path, clock: &Instant) -> Result<LogLine, String> {
        if self.is_building() {
            return Err(t("game.build_running").into());
        }
        let args = vec!["build".to_string(), self.root.display().to_string()];
        self.build = Some(Job::spawn(JobKind::Build, spark, &args, &self.root)?);
        Ok(editor_line(JobKind::Build, Level::Info, t("game.building"), clock))
    }

    /// Output + events since the last frame. Call every frame.
    pub fn poll(&mut self, clock: &Instant) -> (Vec<LogLine>, Vec<SessionEvent>) {
        let mut lines = Vec::new();
        let mut events = Vec::new();

        // The game connects back to our server shortly after it starts.
        if self.link.is_none() {
            if let Some(server) = &self.server {
                match server.accept::<EditorCommand, GameMessage>() {
                    Some(Ok(link)) => {
                        link.send(EditorCommand::WatchScene { interval_ms: Some(SCENE_INTERVAL_MS) });
                        if self.overlay {
                            link.send(EditorCommand::Overlay { on: true });
                        }
                        self.link = Some(link);
                        self.server = None;
                    }
                    Some(Err(e)) => lines.push(editor_line(JobKind::Run, Level::Warn, format!("devtools: {e}"), clock)),
                    None => {}
                }
            }
        }
        if let Some(link) = &self.link {
            let messages: Vec<GameMessage> = link.drain().collect();
            let alive = link.is_alive();
            for msg in messages {
                match msg {
                    GameMessage::Hello { protocol, engine, pid, gpu } => {
                        if protocol != spark_devtools::PROTOCOL_VERSION {
                            lines.push(editor_line(JobKind::Run, Level::Warn, tf("game.protocol_mismatch", &[("game", &protocol), ("editor", &spark_devtools::PROTOCOL_VERSION)]), clock));
                        }
                        lines.push(editor_line(JobKind::Run, Level::Info, tf("game.connected", &[("engine", &engine), ("gpu", &gpu)]), clock));
                        self.hello = Some(Hello { engine, gpu, pid });
                    }
                    GameMessage::Stats(s) => {
                        self.paused = s.paused;
                        if self.frame_ms.len() >= HISTORY {
                            self.frame_ms.pop_front();
                        }
                        self.frame_ms.push_back(s.frame_ms);
                        self.stats = Some(s);
                    }
                    GameMessage::Scene(scene) => self.scene = Some(scene),
                    GameMessage::Paused { paused } => self.paused = paused,
                    GameMessage::ScriptError { message } => {
                        if message != self.script_error {
                            self.script_error = message.clone();
                            events.push(SessionEvent::ScriptError(message));
                        }
                    }
                    GameMessage::Bye => {}
                }
            }
            if !alive {
                self.link = None;
            }
        }

        if let Some(job) = &mut self.run {
            let (out, finished) = job.poll(clock);
            lines.extend(out);
            if let Some(state) = finished {
                let text = match state {
                    JobState::Exited(Some(0)) => t("game.exited_ok").to_string(),
                    JobState::Exited(Some(code)) => tf("game.exited_code", &[("code", &code)]),
                    _ => t("game.exited").to_string(),
                };
                let level = if matches!(state, JobState::Exited(Some(c)) if c != 0) { Level::Error } else { Level::Info };
                lines.push(editor_line(JobKind::Run, level, text, clock));
                self.link = None;
                self.server = None;
                self.quit_sent = None;
                events.push(SessionEvent::GameExited);
                if let Some(spark) = self.restart.take() {
                    self.run = None;
                    match self.run(&spark, clock) {
                        Ok(l) => lines.extend(l),
                        Err(e) => lines.push(editor_line(JobKind::Run, Level::Error, e, clock)),
                    }
                }
            } else if self.quit_sent.is_some_and(|t| t.elapsed() > QUIT_GRACE) {
                job.kill();
            }
        }

        if let Some(job) = &mut self.check {
            let (out, finished) = job.poll(clock);
            lines.extend(out);
            if let Some(state) = finished {
                let report = CheckReport::parse(job.stdout());
                let text = match &report {
                    Some(r) if r.ok => t("game.check_ok").to_string(),
                    Some(r) => tf("game.check_errors", &[("count", &r.errors.len())]),
                    None => tf("game.check_broken", &[("state", &format!("{state:?}"))]),
                };
                let level = if report.as_ref().is_some_and(|r| r.ok) { Level::Info } else { Level::Error };
                lines.push(editor_line(JobKind::Check, level, text, clock));
                if let Some(r) = &report {
                    for e in &r.errors {
                        let place = match e.line {
                            Some(l) => format!("{}:{l}", e.file),
                            None => e.file.clone(),
                        };
                        lines.push(editor_line(JobKind::Check, Level::Error, format!("{place}: {}", e.message), clock));
                    }
                }
                events.push(SessionEvent::CheckDone(report));
            }
        }

        if let Some(job) = &mut self.build {
            let (out, finished) = job.poll(clock);
            lines.extend(out);
            if let Some(state) = finished {
                let ok = state == JobState::Exited(Some(0));
                let text = if ok { t("game.build_ok").to_string() } else { tf("game.build_failed", &[("state", &format!("{state:?}"))]) };
                lines.push(editor_line(JobKind::Build, if ok { Level::Info } else { Level::Error }, text, clock));
                events.push(SessionEvent::BuildDone(ok));
            }
        }
        (lines, events)
    }

    // ---------------------------------------------------------------- Debug panel

    pub fn debug_ui(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let state = self.state();
        ui.horizontal(|ui| {
            let (color, text) = state_badge(state);
            ui.label(RichText::new(format!("{} {text}", icon::DOT)).color(color).strong());
            if let Some(h) = &self.hello {
                ui.label(RichText::new(format!("pid {} · {}", h.pid, h.engine)).small().color(theme::TEXT_DIM));
            }
        });
        if let Some(h) = &self.hello {
            ui.label(RichText::new(format!("GPU: {}", h.gpu)).small().color(theme::TEXT_DIM));
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let running = state != GameState::Stopped;
            if running {
                if ui.button(format!("{} {}", icon::STOP, t("game.stop"))).clicked() {
                    actions.push(Action::Stop);
                }
            } else if ui.add(theme::primary_button(format!("{} {}", icon::RUN, t("game.run")))).clicked() {
                actions.push(Action::Run);
            }
            let live = self.link.is_some();
            let pause_label = if self.paused { format!("{} {}", icon::RUN, t("game.resume")) } else { format!("{} {}", icon::PAUSE, t("game.pause")) };
            if ui.add_enabled(live, egui::Button::new(pause_label)).clicked() {
                actions.push(Action::TogglePause);
            }
            if ui.add_enabled(live, egui::Button::new(format!("{} {}", icon::STEP, t("game.step")))).on_hover_text(t("game.step_hint")).clicked() {
                actions.push(Action::Step);
            }
            if ui.add_enabled(live, egui::Button::new(format!("{} {}", icon::RELOAD, t("game.reload")))).on_hover_text(t("game.reload_hint")).clicked() {
                actions.push(Action::Reload);
            }
            let mut overlay = self.overlay;
            if ui.add_enabled(live, egui::Checkbox::new(&mut overlay, t("game.overlay"))).changed() {
                self.set_overlay(overlay);
            }
        });

        if let Some(err) = &self.script_error {
            ui.add_space(6.0);
            egui::Frame::new().fill(Color32::from_rgb(0x45, 0x1e, 0x22)).corner_radius(4).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("{} {}", icon::ERROR, t("game.script_error"))).color(theme::ERROR).strong());
                ui.add(egui::Label::new(RichText::new(err).monospace().color(theme::TEXT)).wrap());
                if let Some(loc) = find_location(err) {
                    if ui.link(tf("console.open_at", &[("file", &loc.file), ("line", &loc.line)])).clicked() {
                        actions.push(Action::OpenAt(PathBuf::from(&loc.file), loc.line));
                    }
                }
            });
        }

        ui.add_space(8.0);
        let Some(s) = self.stats.as_ref().filter(|_| state != GameState::Stopped) else {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(if state == GameState::Starting { t("game.waiting") } else { t("game.not_running") }).color(theme::TEXT_DIM));
            });
            return;
        };
        egui::Grid::new("game-stats").num_columns(2).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
            let row = |ui: &mut egui::Ui, k: &str, v: String| {
                ui.label(RichText::new(k).color(theme::TEXT_DIM));
                ui.label(RichText::new(v).monospace());
                ui.end_row();
            };
            row(ui, t("stats.fps"), format!("{:.0}", s.fps));
            row(ui, t("stats.frame"), format!("{:.2} ms  ({:.2} – {:.2})", s.frame_ms, s.min_frame_ms, s.max_frame_ms));
            row(ui, t("stats.objects"), s.objects.to_string());
            row(ui, t("stats.draw_calls"), format!("{}  ({} {})", s.draw_calls, s.culled, t("stats.culled")));
            row(ui, t("stats.triangles"), group_digits(s.triangles));
            row(ui, t("stats.resolution"), format!("{} × {}", s.internal_size.0, s.internal_size.1));
            row(ui, t("stats.time"), format!("{:.1} s · #{}", s.time, s.frame));
        });
        ui.add_space(8.0);
        ui.label(RichText::new(t("stats.graph")).small().color(theme::TEXT_DIM));
        frame_graph(ui, &self.frame_ms);
    }

    // ---------------------------------------------------------------- Scene panel

    pub fn scene_ui(&mut self, ui: &mut egui::Ui) {
        let Self { scene, scene_filter, selected: selection, .. } = self;
        let Some(scene) = scene.as_ref() else {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| ui.label(RichText::new(t("scene.empty")).color(theme::TEXT_DIM)));
            return;
        };
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(scene_filter).hint_text(format!("{} {}", icon::SEARCH, t("scene.filter"))).desired_width(180.0));
            ui.label(RichText::new(tf("scene.count", &[("count", &scene.objects.len())])).small().color(theme::TEXT_DIM));
        });
        let c = &scene.camera;
        ui.label(
            RichText::new(format!("{} {} {}  →  {}  · fov {:.0}°", icon::SCENE, t("scene.camera"), vec3(&c.position), vec3(&c.forward), c.fov))
                .small()
                .color(theme::TEXT_DIM),
        );
        ui.separator();

        let selected = *selection;
        let mut clicked = None;
        let filter = scene_filter.to_lowercase();
        let inspector_height = if selected.is_some() { 190.0 } else { 0.0 };
        egui::ScrollArea::vertical().id_salt("scene-tree").auto_shrink([false, false]).max_height((ui.available_height() - inspector_height).max(80.0)).show(ui, |ui| {
            if filter.is_empty() {
                let mut children: BTreeMap<Option<u32>, Vec<&ObjectInfo>> = BTreeMap::new();
                let ids: std::collections::HashSet<u32> = scene.objects.iter().map(|o| o.id).collect();
                for o in &scene.objects {
                    // Parents that are not in the snapshot: show at the top level.
                    let parent = o.parent.filter(|p| ids.contains(p));
                    children.entry(parent).or_default().push(o);
                }
                tree(ui, &children, None, selected, &mut clicked, 0);
            } else {
                for o in scene.objects.iter().filter(|o| display_name(o).to_lowercase().contains(&filter)) {
                    if object_row(ui, o, selected == Some(o.id), false).clicked() {
                        clicked = Some(o.id);
                    }
                }
            }
        });
        if let Some(id) = clicked {
            *selection = if selected == Some(id) { None } else { Some(id) };
        }
        if let Some(obj) = selection.and_then(|id| scene.objects.iter().find(|o| o.id == id)) {
            ui.separator();
            inspector(ui, obj);
        }
    }
}

impl Drop for GameSession {
    /// Closing the project closes its game: politely first, then the hard way.
    fn drop(&mut self) {
        if let (Some(job), Some(link)) = (self.run.as_mut(), self.link.as_ref()) {
            if job.is_running() && link.send(EditorCommand::Quit) {
                let clock = Instant::now();
                let until = Instant::now() + Duration::from_millis(800);
                while Instant::now() < until && job.is_running() {
                    let _ = job.poll(&clock);
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
        // `Job::drop` kills whatever is still running.
    }
}

fn state_badge(state: GameState) -> (Color32, &'static str) {
    match state {
        GameState::Stopped => (theme::TEXT_DIM, t("game.state_stopped")),
        GameState::Starting => (theme::INFO, t("game.state_starting")),
        GameState::Running => (theme::OK, t("game.state_running")),
        GameState::Paused => (theme::WARN, t("game.state_paused")),
        GameState::Stopping => (theme::TEXT_DIM, t("game.state_stopping")),
    }
}

pub fn state_label(state: GameState) -> (Color32, &'static str) {
    state_badge(state)
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// Frame times as bars, with 60 and 30 FPS lines.
fn frame_graph(ui: &mut egui::Ui, samples: &VecDeque<f32>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(120.0), 90.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, theme::SURFACE);
    let max_ms = samples.iter().copied().fold(40.0_f32, f32::max).min(250.0);
    let y = |ms: f32| rect.bottom() - (ms / max_ms).clamp(0.0, 1.0) * rect.height();
    for (ms, label) in [(1000.0 / 60.0, "60"), (1000.0 / 30.0, "30")] {
        let yy = y(ms);
        painter.line_segment([pos2(rect.left(), yy), pos2(rect.right(), yy)], Stroke::new(1.0, Color32::from_gray(70)));
        painter.text(pos2(rect.left() + 3.0, yy - 1.0), egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(10.0), Color32::from_gray(110));
    }
    if samples.is_empty() {
        return;
    }
    let w = rect.width() / HISTORY as f32;
    let start = HISTORY - samples.len();
    for (i, &ms) in samples.iter().enumerate() {
        let x = rect.left() + (start + i) as f32 * w;
        let color = if ms > 1000.0 / 30.0 { theme::ERROR } else if ms > 1000.0 / 60.0 + 0.5 { theme::WARN } else { theme::OK };
        painter.rect_filled(egui::Rect::from_min_max(pos2(x, y(ms)), pos2(x + w.max(1.0), rect.bottom())), 0.0, color);
    }
}

fn display_name(o: &ObjectInfo) -> String {
    if o.name.is_empty() { format!("#{}", o.id) } else { o.name.clone() }
}

fn object_row(ui: &mut egui::Ui, o: &ObjectInfo, selected: bool, has_children: bool) -> egui::Response {
    let kind = if o.light {
        "💡"
    } else if o.body.is_some() {
        "⚙"
    } else if o.mesh.is_some() {
        icon::MODEL
    } else if has_children {
        icon::FOLDER
    } else {
        "○"
    };
    let mut text = RichText::new(format!("{kind} {}", display_name(o)));
    if !o.visible {
        text = text.color(theme::TEXT_DIM).italics();
    }
    ui.selectable_label(selected, text)
}

fn tree(
    ui: &mut egui::Ui,
    children: &BTreeMap<Option<u32>, Vec<&ObjectInfo>>,
    parent: Option<u32>,
    selected: Option<u32>,
    clicked: &mut Option<u32>,
    depth: usize,
) {
    let Some(list) = children.get(&parent) else { return };
    for o in list {
        let kids = children.contains_key(&Some(o.id)) && depth < 32;
        if kids {
            let id = ui.make_persistent_id(("scene-node", o.id));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth < 1)
                .show_header(ui, |ui| {
                    if object_row(ui, o, selected == Some(o.id), true).clicked() {
                        *clicked = Some(o.id);
                    }
                })
                .body(|ui| tree(ui, children, Some(o.id), selected, clicked, depth + 1));
        } else {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                if object_row(ui, o, selected == Some(o.id), false).clicked() {
                    *clicked = Some(o.id);
                }
            });
        }
    }
}

fn num(v: Option<f32>) -> String {
    match v {
        Some(v) => format!("{v:.2}"),
        None => "NaN".into(),
    }
}

fn vec3(v: &[Option<f32>; 3]) -> String {
    format!("({}, {}, {})", num(v[0]), num(v[1]), num(v[2]))
}

fn inspector(ui: &mut egui::Ui, o: &ObjectInfo) {
    ui.label(RichText::new(format!("{}  ·  id {}", display_name(o), o.id)).strong());
    egui::Grid::new("inspector").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        let row = |ui: &mut egui::Ui, k: &str, v: String| {
            ui.label(RichText::new(k).color(theme::TEXT_DIM));
            ui.label(RichText::new(v).monospace());
            ui.end_row();
        };
        row(ui, t("scene.position"), vec3(&o.position));
        row(ui, t("scene.world_position"), vec3(&o.world_position));
        row(ui, t("scene.rotation"), vec3(&o.rotation_deg));
        row(ui, t("scene.scale"), vec3(&o.scale));
        row(ui, t("scene.visible"), if o.visible { t("common.yes") } else { t("common.no") }.to_string());
        if let Some(m) = o.mesh {
            row(ui, t("scene.mesh"), format!("#{m}"));
        }
        if let Some([min, max]) = &o.bounds {
            row(ui, t("scene.bounds"), format!("{} … {}", vec3(min), vec3(max)));
        }
        if let Some(b) = &o.body {
            row(ui, t("scene.body"), format!("{} · v = {}", b.kind, vec3(&b.velocity)));
        }
        if let Some(p) = o.parent {
            row(ui, t("scene.parent"), format!("#{p}"));
        }
    });
}
