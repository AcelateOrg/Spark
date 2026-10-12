//! Scene viewport: the game drawn inside the editor on egui's own GPU device (no second window,
//! no pixel copies), an editor fly camera (hold RMB + WASD, like Unity / Roblox Studio) and Play
//! mode: the game runs in-process and gets the panel's keyboard and mouse.
//!
//! Edit mode builds the game (`start` + one frame) and freezes it; the fly camera replaces the
//! game camera while drawing. Play builds a fresh game and runs it every editor frame; Stop
//! throws the played state away (like Unity). Saved files rebuild the frozen preview; while
//! playing, the game's own hot reload picks them up.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Pos2, Rect, RichText};
use eframe::egui_wgpu::{self, wgpu};
use engine::script::ScriptGame;
use engine::{Camera, Game, Key as SKey, MouseButton as SButton, Quat, Renderer, Vec2, Vec3, World, run_frame};

use crate::i18n::{t, tf};
use crate::process::{JobKind, Level, LogLine, editor_line};
use crate::theme::{self, icon};

/// The view the engine draws into (same as a window surface: sRGB).
const VIEW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const EDIT_DT: f32 = 1.0 / 60.0;
/// Saved files are applied to the frozen preview after this pause (editors save in bursts).
const RELOAD_DELAY: Duration = Duration::from_millis(250);
/// Files that change what the game shows (not error.log, not editor files).
const GAME_FILES: &[&str] =
    &["luau", "lua", "toml", "json", "png", "jpg", "jpeg", "ktx2", "hdr", "glb", "gltf", "obj", "wav", "ogg", "mp3", "flac", "wgsl", "ttf", "otf"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Frozen preview, fly camera.
    Edit,
    Play,
    /// Play state kept, fly camera.
    Pause,
}

pub enum ViewportEvent {
    /// The in-editor game hit a script error (`Some`) or is fine again (`None`).
    Error(Option<String>),
}

struct Target {
    _texture: wgpu::Texture,
    /// sRGB view the engine renders into.
    view: wgpu::TextureView,
    /// egui's handle (an `Rgba8Unorm` view of the same texture: egui works in gamma space).
    id: egui::TextureId,
    size: (u32, u32),
}

struct Session {
    game: ScriptGame,
    world: World,
}

/// Who owns the mouse cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    None,
    /// RMB held: fly-camera look; the cursor is kept at `anchor`.
    /// `last`: where the cursor was put back to; `None` until the first motion (that one is skipped:
    /// it may still be the jump of the first warp).
    Look { anchor: Pos2, last: Option<Pos2> },
    /// The game asked for `input.lock_mouse` (first-person look); kept at the panel center.
    Game { last: Option<Pos2> },
}

/// The editor camera.
#[derive(Clone, Copy, Debug)]
struct FlyCam {
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    /// Meters per second.
    speed: f32,
    ready: bool,
}

impl Default for FlyCam {
    fn default() -> Self {
        Self { pos: Vec3::new(0.0, 3.0, 8.0), yaw: 0.0, pitch: -0.35, speed: 6.0, ready: false }
    }
}

impl FlyCam {
    fn from_camera(c: &Camera, speed: f32) -> Self {
        let f = c.forward();
        Self { pos: c.position, yaw: (-f.x).atan2(-f.z), pitch: f.y.clamp(-1.0, 1.0).asin(), speed, ready: true }
    }

    fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    fn forward(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }

    fn right(&self) -> Vec3 {
        self.rotation() * Vec3::X
    }

    fn up(&self) -> Vec3 {
        self.rotation() * Vec3::Y
    }
}

pub struct Viewport {
    entry: PathBuf,
    gpu: Option<egui_wgpu::RenderState>,
    renderer: Option<Renderer>,
    target: Option<Target>,
    session: Option<Session>,
    mode: Mode,
    cam: FlyCam,
    grab: Grab,
    cursor_hidden: bool,
    focused: bool,
    focus_next: bool,
    mute: bool,
    stale_since: Option<Instant>,
    last_frame: Instant,
    last_error: Option<String>,
    gpu_error: Option<String>,
    events: Vec<ViewportEvent>,
    fps: f32,
    fps_frames: u32,
    fps_timer: Instant,
}

impl Viewport {
    pub fn new(entry: &Path) -> Self {
        Self {
            entry: entry.to_path_buf(),
            gpu: None,
            renderer: None,
            target: None,
            session: None,
            mode: Mode::Edit,
            cam: FlyCam::default(),
            grab: Grab::None,
            cursor_hidden: false,
            focused: false,
            focus_next: false,
            mute: false,
            stale_since: None,
            last_frame: Instant::now(),
            last_error: None,
            gpu_error: None,
            events: Vec::new(),
            fps: 0.0,
            fps_frames: 0,
            fps_timer: Instant::now(),
        }
    }

    /// egui's GPU device (eframe's wgpu backend); `None` = no in-editor rendering.
    pub fn set_gpu(&mut self, gpu: Option<&egui_wgpu::RenderState>) {
        if self.gpu.is_none() {
            self.gpu = gpu.cloned();
        }
    }

    /// The game is playing and has the keyboard: editor shortcuts must not steal its keys.
    pub fn wants_keyboard(&self) -> bool {
        self.mode == Mode::Play && self.focused
    }

    pub fn take_events(&mut self) -> Vec<ViewportEvent> {
        std::mem::take(&mut self.events)
    }

    /// Engine messages (script `print`, errors) for the console.
    pub fn take_log(&mut self, clock: &Instant) -> Vec<LogLine> {
        if self.session.is_none() {
            return Vec::new();
        }
        crate::logcap::take()
            .into_iter()
            .map(|(level, text)| {
                let level = match level {
                    log::Level::Error => Level::Error,
                    log::Level::Warn => Level::Warn,
                    log::Level::Info => Level::Info,
                    _ => Level::Debug,
                };
                editor_line(JobKind::Run, level, text, clock)
            })
            .collect()
    }

    /// Files changed on disk: rebuild the frozen preview soon.
    pub fn files_changed(&mut self, changes: &HashSet<PathBuf>) {
        let relevant = changes.iter().any(|p| {
            p.extension().and_then(|e| e.to_str()).is_some_and(|e| GAME_FILES.contains(&e.to_ascii_lowercase().as_str()))
        });
        if relevant {
            self.stale_since = Some(Instant::now());
        }
    }

    pub fn play(&mut self) {
        self.build(true);
        self.mode = Mode::Play;
        self.focus_next = true;
    }

    pub fn stop(&mut self) {
        self.build(false);
        self.mode = Mode::Edit;
    }

    pub fn toggle_play(&mut self) {
        if self.mode == Mode::Edit { self.play() } else { self.stop() }
    }

    pub fn toggle_pause(&mut self) {
        match self.mode {
            Mode::Play => self.mode = Mode::Pause,
            Mode::Pause => {
                self.mode = Mode::Play;
                self.focus_next = true;
                self.last_frame = Instant::now();
            }
            Mode::Edit => {}
        }
    }

    /// One frame of game logic while paused.
    pub fn step(&mut self) {
        if self.mode == Mode::Play {
            self.mode = Mode::Pause;
        }
        if self.mode == Mode::Pause {
            self.advance(EDIT_DT);
        }
    }

    /// A fresh game (Play: hot reload + sound; Edit: `start` and one frame).
    fn build(&mut self, play: bool) {
        crate::logcap::capture(true);
        let mut game = ScriptGame::new(self.entry.clone()).hot_reload(play);
        let mut world = World::new();
        world.physics.set_backend(Box::new(engine::physics::RapierBackend::new()));
        if play && !self.mute {
            if let Some(audio) = engine::audio::KiraBackend::new() {
                world.audio.set_backend(Box::new(audio));
            }
        }
        if let Some(t) = &self.target {
            world.canvas.set_output_size(t.size.0, t.size.1);
        }
        game.start(&mut world);
        if !play {
            run_frame(&mut game, &mut world, EDIT_DT);
        }
        if !self.cam.ready {
            self.cam = FlyCam::from_camera(&world.scene.camera, self.cam.speed);
        }
        self.session = Some(Session { game, world });
        self.stale_since = None;
        self.last_frame = Instant::now();
        self.fps_frames = 0;
        self.fps_timer = self.last_frame;
        self.report_error();
    }

    fn advance(&mut self, dt: f32) {
        let Some(s) = &mut self.session else { return };
        if let Some(t) = &self.target {
            s.world.canvas.set_output_size(t.size.0, t.size.1);
        }
        s.world.debug.record_frame(dt);
        run_frame(&mut s.game, &mut s.world, dt);
        self.report_error();
    }

    fn report_error(&mut self) {
        let error = self.session.as_ref().and_then(|s| s.game.error()).map(str::to_string);
        if error != self.last_error {
            self.last_error = error.clone();
            self.events.push(ViewportEvent::Error(error));
        }
    }

    fn ensure_gpu(&mut self, size: (u32, u32)) -> bool {
        let Some(rs) = &self.gpu else { return false };
        let renderer = self.renderer.get_or_insert_with(|| {
            let r = Renderer::from_device(rs.instance.clone(), &rs.adapter, rs.device.clone(), rs.queue.clone(), size.0, size.1);
            log::info!("viewport renderer on {}", r.info());
            r
        });
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return true;
        }
        renderer.resize(size.0, size.1);
        let texture = rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("spark viewport"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[VIEW_FORMAT],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(VIEW_FORMAT), ..Default::default() });
        let egui_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut egui_renderer = rs.renderer.write();
        let id = match &self.target {
            Some(old) => {
                egui_renderer.update_egui_texture_from_wgpu_texture(&rs.device, &egui_view, wgpu::FilterMode::Linear, old.id);
                old.id
            }
            None => egui_renderer.register_native_texture(&rs.device, &egui_view, wgpu::FilterMode::Linear),
        };
        self.target = Some(Target { _texture: texture, view, id, size });
        true
    }

    fn render(&mut self) {
        let (Some(r), Some(target), Some(s)) = (&mut self.renderer, &self.target, &mut self.session) else { return };
        let game_camera = s.world.scene.camera;
        if self.mode != Mode::Play {
            let cam = &mut s.world.scene.camera;
            cam.position = self.cam.pos;
            cam.rotation = self.cam.rotation();
        }
        let result = r.render_to_view(&s.world, &target.view, VIEW_FORMAT);
        s.world.scene.camera = game_camera;
        match result {
            Ok(_) => self.gpu_error = None,
            Err(e) => {
                if r.is_device_lost() {
                    // The device belongs to egui: nothing to recover here, eframe will fail too.
                    log::error!("viewport: {e}");
                }
                self.gpu_error = Some(e);
            }
        }
    }

    fn set_cursor(&mut self, ctx: &egui::Context, grab: Grab) {
        let hide = grab != Grab::None;
        if hide != self.cursor_hidden {
            self.cursor_hidden = hide;
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(!hide));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(if hide {
                egui::CursorGrab::Confined
            } else {
                egui::CursorGrab::None
            }));
        }
        self.grab = grab;
    }

    // ------------------------------------------------------------------ ui

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.toolbar(ui);
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        if self.gpu.is_none() {
            ui.put(rect, egui::Label::new(RichText::new(t("viewport.no_gpu")).color(theme::TEXT_DIM)));
            return;
        }
        let ppp = ctx.pixels_per_point();
        let size = ((rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32);
        if rect.width() < 2.0 || rect.height() < 2.0 || !self.ensure_gpu(size) {
            return;
        }
        if self.session.is_none() {
            self.build(false);
        }
        if self.mode == Mode::Edit && self.stale_since.is_some_and(|s| s.elapsed() >= RELOAD_DELAY) {
            self.build(false);
        }
        if self.stale_since.is_some() {
            ctx.request_repaint_after(RELOAD_DELAY);
        }

        // Focus: a click in the panel gives the game (or the fly camera) the keyboard.
        if self.focus_next || response.is_pointer_button_down_on() {
            self.focus_next = false;
            response.request_focus();
        }
        let focused = response.has_focus();
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
                )
            });
        }
        if self.focused && !focused {
            if let Some(s) = &mut self.session {
                s.world.input.release_all();
            }
        }
        self.focused = focused;

        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let pointer_moved = ui.input(|i| i.events.iter().rev().find_map(|e| if let egui::Event::PointerMoved(p) = e { Some(*p) } else { None }));

        if self.mode == Mode::Play {
            if self.grab != Grab::None && !matches!(self.grab, Grab::Game { .. }) {
                self.set_cursor(&ctx, Grab::None);
            }
            self.feed_input(ui, rect, pointer_moved);
            self.advance(dt);
            self.game_cursor(&ctx, rect, pointer_moved);
            if self.session.as_ref().is_some_and(|s| s.world.quit_requested()) {
                self.stop();
            }
            self.count_fps(now);
            ctx.request_repaint();
        } else {
            if matches!(self.grab, Grab::Game { .. }) {
                self.set_cursor(&ctx, Grab::None);
            }
            self.fly(ui, &response, rect, dt, pointer_moved);
        }

        self.render();
        if let Some(target) = &self.target {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().image(target.id, rect, uv, egui::Color32::WHITE);
        }
        self.overlay(ui, rect, focused);
    }

    fn count_fps(&mut self, now: Instant) {
        self.fps_frames += 1;
        let elapsed = (now - self.fps_timer).as_secs_f32();
        if elapsed >= 0.5 {
            self.fps = self.fps_frames as f32 / elapsed;
            if let Some(s) = &mut self.session {
                s.world.time.fps = self.fps;
            }
            self.fps_frames = 0;
            self.fps_timer = now;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.mode == Mode::Edit {
                if ui.add(theme::primary_button(format!("{} {}", icon::RUN, t("viewport.play")))).on_hover_text(format!("{} (Ctrl+P)", t("viewport.play_hint"))).clicked() {
                    self.play();
                }
            } else if ui.button(format!("{} {}", icon::STOP, t("viewport.stop"))).on_hover_text("Ctrl+P").clicked() {
                self.stop();
            }
            let playing = self.mode != Mode::Edit;
            let (pause, hint) = if self.mode == Mode::Pause { (icon::RUN, t("game.resume")) } else { (icon::PAUSE, t("game.pause")) };
            if ui.add_enabled(playing, egui::Button::new(pause)).on_hover_text(format!("{hint} (Ctrl+Shift+P)")).clicked() {
                self.toggle_pause();
            }
            if ui.add_enabled(playing, egui::Button::new(icon::STEP)).on_hover_text(t("game.step_hint")).clicked() {
                self.step();
            }
            if ui.button(icon::RELOAD).on_hover_text(t("viewport.reload_hint")).clicked() {
                if self.mode == Mode::Edit { self.build(false) } else { self.play() }
            }
            ui.separator();
            if ui.button("🎥").on_hover_text(t("viewport.camera_reset")).clicked() {
                if let Some(s) = &self.session {
                    self.cam = FlyCam::from_camera(&s.world.scene.camera, self.cam.speed);
                }
            }
            let sound = if self.mute { "🔇" } else { "🔊" };
            if ui.selectable_label(!self.mute, sound).on_hover_text(t("viewport.sound_hint")).clicked() {
                self.mute = !self.mute;
                if self.mode != Mode::Edit {
                    self.play();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("?").color(theme::TEXT_DIM)).on_hover_text(t("viewport.help"));
                let (color, label) = match self.mode {
                    Mode::Edit => (theme::TEXT_DIM, t("viewport.mode_edit").to_string()),
                    Mode::Play => (theme::OK, format!("{} · {:.0} FPS", t("viewport.mode_play"), self.fps)),
                    Mode::Pause => (theme::WARN, t("viewport.mode_pause").to_string()),
                };
                ui.add(egui::Label::new(RichText::new(format!("{} {label}", icon::DOT)).color(color)).truncate());
            });
        });
        // Play / pause from anywhere in the editor (Unity keys).
        let (play, pause) = ui.input_mut(|i| {
            let pause = i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::P));
            (i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::P)), pause)
        });
        if play {
            self.toggle_play();
        }
        if pause {
            self.toggle_pause();
        }
    }

    fn overlay(&self, ui: &mut egui::Ui, rect: Rect, focused: bool) {
        let painter = ui.painter_at(rect);
        let font = egui::FontId::proportional(12.0);
        if let Some(e) = &self.gpu_error {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, e, font.clone(), theme::ERROR);
        }
        match self.mode {
            Mode::Edit | Mode::Pause => {
                let text = if matches!(self.grab, Grab::Look { .. }) {
                    tf("viewport.speed", &[("speed", &format!("{:.1}", self.cam.speed))])
                } else {
                    t("viewport.hint").to_string()
                };
                let pos = rect.left_bottom() + egui::vec2(8.0, -8.0);
                let galley = painter.layout(text, font, egui::Color32::from_gray(220), (rect.width() - 24.0).max(40.0));
                let bg = Rect::from_min_size(pos - egui::vec2(4.0, galley.size().y + 4.0), galley.size() + egui::vec2(8.0, 8.0));
                painter.rect_filled(bg, 4.0, egui::Color32::from_black_alpha(150));
                painter.galley(pos - egui::vec2(0.0, galley.size().y), galley, egui::Color32::WHITE);
            }
            Mode::Play if !focused => {
                let text = t("viewport.click_to_play");
                let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(15.0), egui::Color32::WHITE);
                let bg = Rect::from_center_size(rect.center(), galley.size() + egui::vec2(24.0, 14.0));
                painter.rect_filled(bg, 6.0, egui::Color32::from_black_alpha(170));
                painter.galley(rect.center() - galley.size() / 2.0, galley, egui::Color32::WHITE);
            }
            Mode::Play => {}
        }
    }

    // ------------------------------------------------------------------ play input

    fn feed_input(&mut self, ui: &egui::Ui, rect: Rect, pointer_moved: Option<Pos2>) {
        let focused = self.focused;
        let locked = matches!(self.grab, Grab::Game { .. });
        let Some(s) = &mut self.session else { return };
        if !focused {
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        let input = &mut s.world.input;
        let mut release_grab = false;
        ui.input(|i| {
            for e in &i.events {
                match e {
                    egui::Event::Key { key, physical_key, pressed, .. } => {
                        if let Some(k) = map_key(physical_key.unwrap_or(*key)) {
                            input.key_event(k, *pressed);
                            if k == SKey::Escape && *pressed && locked {
                                release_grab = true;
                            }
                        }
                    }
                    egui::Event::Text(text) => input.text_event(text),
                    egui::Event::PointerButton { pos, button, pressed, .. } if !*pressed || rect.contains(*pos) => {
                        if let Some(b) = map_button(*button) {
                            input.mouse_button_event(b, *pressed);
                        }
                    }
                    egui::Event::MouseWheel { unit, delta, .. } if i.pointer.hover_pos().is_some_and(|p| rect.contains(p)) => {
                        input.wheel_event(match unit {
                            egui::MouseWheelUnit::Line => delta.y,
                            egui::MouseWheelUnit::Page => delta.y * 3.0,
                            egui::MouseWheelUnit::Point => delta.y / 40.0,
                        });
                    }
                    _ => {}
                }
            }
            sync_modifier(input, i.modifiers.shift, SKey::LShift, SKey::RShift);
            sync_modifier(input, i.modifiers.ctrl, SKey::LCtrl, SKey::RCtrl);
            sync_modifier(input, i.modifiers.alt, SKey::LAlt, SKey::RAlt);
        });
        if !locked {
            if let Some(p) = pointer_moved {
                let local = (p - rect.min) * ppp;
                input.mouse_moved(Vec2::new(local.x, local.y));
            }
        }
        if release_grab {
            // Esc frees the mouse (the game still sees the key, e.g. to open its menu).
            ui.ctx().memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            self.focused = false;
        }
    }

    /// Hides and pins the cursor while the game wants raw mouse look; the motion becomes `mouse_delta`.
    fn game_cursor(&mut self, ctx: &egui::Context, rect: Rect, pointer_moved: Option<Pos2>) {
        let want = self.focused && self.session.as_ref().is_some_and(|s| s.world.input.lock_mouse);
        let center = rect.center();
        match (want, self.grab) {
            (true, Grab::Game { last }) => {
                if let Some(p) = pointer_moved {
                    if let Some(last) = last {
                        let d = (p - last) * ctx.pixels_per_point();
                        if let Some(s) = &mut self.session {
                            s.world.input.mouse_motion(Vec2::new(d.x, d.y));
                        }
                    }
                    if p != center {
                        ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(center));
                    }
                    self.grab = Grab::Game { last: Some(center) };
                }
            }
            (true, _) => {
                self.set_cursor(ctx, Grab::Game { last: None });
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(center));
            }
            (false, Grab::Game { .. }) => self.set_cursor(ctx, Grab::None),
            (false, _) => {}
        }
    }

    // ------------------------------------------------------------------ fly camera

    fn fly(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect, dt: f32, pointer_moved: Option<Pos2>) {
        let ctx = ui.ctx().clone();
        let (rmb_down, rmb_pressed, wheel, keys, shift) = ui.input(|i| {
            let wheel: f32 = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                        egui::MouseWheelUnit::Point => delta.y / 40.0,
                    }),
                    _ => None,
                })
                .sum();
            let k = |key| if i.key_down(key) { 1.0 } else { 0.0 };
            let keys = Vec3::new(
                k(egui::Key::D) - k(egui::Key::A),
                k(egui::Key::E) - k(egui::Key::Q),
                k(egui::Key::W) - k(egui::Key::S),
            );
            let arrows = Vec3::new(k(egui::Key::ArrowRight) - k(egui::Key::ArrowLeft), 0.0, k(egui::Key::ArrowUp) - k(egui::Key::ArrowDown));
            (i.pointer.secondary_down(), i.pointer.button_pressed(egui::PointerButton::Secondary), wheel, (keys, arrows), i.modifiers.shift)
        });
        let hovered = response.hovered() || response.contains_pointer();

        // RMB: look around (cursor hidden and pinned), WASD / QE fly, wheel = speed.
        match self.grab {
            Grab::None if rmb_pressed && hovered => {
                if let Some(p) = ui.input(|i| i.pointer.press_origin().or(i.pointer.latest_pos())) {
                    response.request_focus();
                    self.set_cursor(&ctx, Grab::Look { anchor: p, last: None });
                }
            }
            Grab::Look { .. } if !rmb_down => self.set_cursor(&ctx, Grab::None),
            Grab::Look { anchor, last } => {
                if let Some(p) = pointer_moved {
                    if let Some(last) = last {
                        let d = p - last;
                        let sens = 0.0045;
                        self.cam.yaw -= d.x * sens;
                        self.cam.pitch = (self.cam.pitch - d.y * sens).clamp(-1.55, 1.55);
                    }
                    if p != anchor {
                        ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(anchor));
                    }
                    self.grab = Grab::Look { anchor, last: Some(anchor) };
                }
            }
            _ => {}
        }
        let looking = matches!(self.grab, Grab::Look { .. });
        let boost = if shift { 3.0 } else { 1.0 };
        let (wasd, arrows) = keys;
        let mut dir = Vec3::ZERO;
        if looking {
            dir += wasd;
            if wheel != 0.0 {
                self.cam.speed = (self.cam.speed * 1.2f32.powf(wheel)).clamp(0.2, 500.0);
            }
        } else if hovered && wheel != 0.0 {
            // Wheel without RMB: move along the view.
            self.cam.pos += self.cam.forward() * wheel * (self.cam.speed * 0.25).max(0.2);
        }
        if self.focused || looking {
            dir += arrows;
        }
        if dir != Vec3::ZERO {
            let world = self.cam.right() * dir.x + Vec3::Y * dir.y + self.cam.forward() * dir.z;
            self.cam.pos += world.normalize_or_zero() * self.cam.speed * boost * dt;
        }
        // MMB drag: pan.
        if response.dragged_by(egui::PointerButton::Middle) {
            let d = response.drag_delta();
            let k = self.cam.speed * 0.004;
            self.cam.pos += (-self.cam.right() * d.x + self.cam.up() * d.y) * k;
        }
        if looking || dir != Vec3::ZERO || response.dragged() {
            ctx.request_repaint();
        }
        let _ = rect;
    }
}

impl Drop for Viewport {
    fn drop(&mut self) {
        crate::logcap::capture(false);
        if let (Some(rs), Some(target)) = (&self.gpu, &self.target) {
            rs.renderer.write().free_texture(&target.id);
        }
    }
}

/// Presses / releases a modifier from egui's modifier state (egui may not send key events for them).
fn sync_modifier(input: &mut engine::Input, on: bool, left: SKey, right: SKey) {
    if on {
        if !input.down(left) && !input.down(right) {
            input.key_event(left, true);
        }
    } else {
        for k in [left, right] {
            if input.down(k) {
                input.key_event(k, false);
            }
        }
    }
}

/// egui key (physical position) -> Spark key.
fn map_key(key: egui::Key) -> Option<SKey> {
    let name = match key.name() {
        "Equals" => "equal",
        "OpenBracket" => "bracketleft",
        "CloseBracket" => "bracketright",
        "Backtick" => "backquote",
        "ShiftLeft" => "lshift",
        "ShiftRight" => "rshift",
        "ControlLeft" => "lctrl",
        "ControlRight" => "rctrl",
        "AltLeft" => "lalt",
        "AltRight" => "ralt",
        "SuperLeft" => "lsuper",
        "SuperRight" => "rsuper",
        other => return SKey::from_name(&other.to_ascii_lowercase()),
    };
    SKey::from_name(name)
}

fn map_button(b: egui::PointerButton) -> Option<SButton> {
    Some(match b {
        egui::PointerButton::Primary => SButton::Left,
        egui::PointerButton::Secondary => SButton::Right,
        egui::PointerButton::Middle => SButton::Middle,
        egui::PointerButton::Extra1 => SButton::Back,
        egui::PointerButton::Extra2 => SButton::Forward,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_by_position() {
        assert_eq!(map_key(egui::Key::W), Some(SKey::W));
        assert_eq!(map_key(egui::Key::Space), Some(SKey::Space));
        assert_eq!(map_key(egui::Key::ArrowLeft), Some(SKey::Left));
        assert_eq!(map_key(egui::Key::Escape), Some(SKey::Escape));
        assert_eq!(map_key(egui::Key::Num7), Some(SKey::Num7));
        assert_eq!(map_key(egui::Key::F5), Some(SKey::F5));
        assert_eq!(map_key(egui::Key::OpenBracket), Some(SKey::BracketLeft));
        assert_eq!(map_key(egui::Key::ShiftLeft), Some(SKey::LShift));
        assert_eq!(map_key(egui::Key::PageDown), Some(SKey::PageDown));
    }

    #[test]
    fn fly_cam_matches_the_game_camera() {
        let mut c = Camera::default();
        c.position = Vec3::new(3.0, 4.0, 5.0);
        c.look_at(Vec3::new(-2.0, 0.0, 1.0));
        let f = FlyCam::from_camera(&c, 6.0);
        assert!(f.forward().distance(c.forward()) < 1e-4, "{} vs {}", f.forward(), c.forward());
    }
}
