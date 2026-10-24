//! Asset previews: images (zoom / pan / pixel picker), sounds (waveform, play / seek / loop /
//! volume) and 3D models (drawn by the engine on egui's GPU device: orbit camera, animations,
//! stats). Each open preview is a tab; files saved on disk reload their preview.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Vec2};
use eframe::egui_wgpu::{self, wgpu};
use engine::{AnimParams, Color, ObjectId, Quat, Renderer, Vec3, World};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::{PlaybackState, Region};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween};

use super::Action;
use crate::i18n::{t, tf};
use crate::theme::{self, icon};

const VIEW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Waveform resolution (min / max pairs over the whole sound).
const PEAKS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Sound,
    Model,
}

/// Files with a built-in preview.
pub fn kind(path: &Path) -> Option<Kind> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase)?;
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "tga" | "webp" | "hdr" => Kind::Image,
        "wav" | "ogg" | "mp3" | "flac" => Kind::Sound,
        "glb" | "gltf" => Kind::Model,
        _ => return None,
    })
}

pub fn icon_for(path: &Path) -> &'static str {
    match kind(path) {
        Some(Kind::Image) => icon::IMAGE,
        Some(Kind::Sound) => icon::SOUND,
        Some(Kind::Model) => icon::MODEL,
        None => icon::FILE,
    }
}

enum Preview {
    Image(ImagePreview),
    Sound(SoundPreview),
    Model(ModelPreview),
}

/// Every open preview plus the shared audio output.
#[derive(Default)]
pub struct AssetPreviews {
    gpu: Option<egui_wgpu::RenderState>,
    previews: HashMap<PathBuf, Preview>,
    audio: Option<AudioManager<DefaultBackend>>,
    audio_error: Option<String>,
}

impl AssetPreviews {
    pub fn set_gpu(&mut self, gpu: Option<&egui_wgpu::RenderState>) {
        if self.gpu.is_none() {
            self.gpu = gpu.cloned();
        }
    }

    pub fn open(&mut self, path: &Path) {
        if self.previews.contains_key(path) {
            return;
        }
        let Some(kind) = kind(path) else { return };
        let p = path.to_path_buf();
        let preview = match kind {
            Kind::Image => Preview::Image(ImagePreview::new(p)),
            Kind::Sound => Preview::Sound(SoundPreview::new(p)),
            Kind::Model => Preview::Model(ModelPreview::new(p, self.gpu.clone())),
        };
        self.previews.insert(path.to_path_buf(), preview);
    }

    pub fn close(&mut self, path: &Path) {
        if let Some(Preview::Sound(s)) = self.previews.get_mut(path) {
            s.stop();
        }
        self.previews.remove(path);
    }

    /// Files moved on disk (`from` may be a folder).
    pub fn rename(&mut self, from: &Path, to: &Path) {
        let moved: Vec<PathBuf> = self.previews.keys().filter(|p| p.starts_with(from)).cloned().collect();
        for old in moved {
            let new = to.join(old.strip_prefix(from).unwrap_or(Path::new("")));
            self.close(&old);
            self.open(&new);
        }
    }

    /// Saved / replaced files reload their preview.
    pub fn files_changed(&mut self, changes: &HashSet<PathBuf>) {
        for (path, preview) in &mut self.previews {
            if changes.contains(path) && path.is_file() {
                match preview {
                    Preview::Image(p) => p.reload(),
                    Preview::Sound(p) => p.reload(),
                    Preview::Model(p) => p.reload(),
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, path: &Path, actions: &mut Vec<Action>) {
        if !self.previews.contains_key(path) {
            self.open(path);
        }
        let Self { previews, audio, audio_error, .. } = self;
        match previews.get_mut(path) {
            Some(Preview::Image(p)) => p.ui(ui, actions),
            Some(Preview::Sound(p)) => p.ui(ui, audio, audio_error, actions),
            Some(Preview::Model(p)) => p.ui(ui, actions),
            None => {
                ui.label(RichText::new(t("editor.closed")).color(theme::TEXT_DIM));
            }
        }
    }
}

// ---------------------------------------------------------------------- shared bits

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn fmt_bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1024.0 * 1024.0 {
        format!("{:.1} MB", n / 1024.0 / 1024.0)
    } else if n >= 1024.0 {
        format!("{:.1} KB", n / 1024.0)
    } else {
        format!("{n} B")
    }
}

fn fmt_time(secs: f64) -> String {
    let secs = secs.max(0.0);
    let m = (secs / 60.0).floor();
    format!("{}:{:04.1}", m as u64, secs - m * 60.0)
}

fn fmt_count(n: usize) -> String {
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

fn wheel_steps(ui: &egui::Ui) -> f32 {
    ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Page => delta.y * 3.0,
                    egui::MouseWheelUnit::Point => delta.y / 40.0,
                }),
                _ => None,
            })
            .sum()
    })
}

fn centered(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) {
    let rect = ui.available_rect_before_wrap();
    ui.put(rect, egui::Label::new(text).wrap());
}

fn hint(painter: &egui::Painter, rect: Rect, text: &str) {
    let font = egui::FontId::proportional(12.0);
    let galley = painter.layout(text.to_string(), font, Color32::from_gray(220), (rect.width() - 24.0).max(40.0));
    let pos = rect.left_bottom() + egui::vec2(8.0, -8.0 - galley.size().y);
    let bg = Rect::from_min_size(pos - egui::vec2(4.0, 4.0), galley.size() + egui::vec2(8.0, 8.0));
    painter.rect_filled(bg, 4.0, Color32::from_black_alpha(150));
    painter.galley(pos, galley, Color32::WHITE);
}

fn info_right(ui: &mut egui::Ui, text: String) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add(egui::Label::new(RichText::new(text).color(theme::TEXT_DIM)).truncate());
    });
}

// ---------------------------------------------------------------------- image

struct ImagePreview {
    path: PathBuf,
    job: Option<JoinHandle<Result<egui::ColorImage, String>>>,
    image: Option<egui::ColorImage>,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
    size_on_disk: u64,
    /// Image pixels per screen point; `None` = fit to the panel.
    zoom: Option<f32>,
    pan: Vec2,
    smooth: bool,
    checker: bool,
}

impl ImagePreview {
    fn new(path: PathBuf) -> Self {
        let mut p = Self {
            path,
            job: None,
            image: None,
            texture: None,
            error: None,
            size_on_disk: 0,
            zoom: None,
            pan: Vec2::ZERO,
            smooth: true,
            checker: true,
        };
        p.reload();
        p
    }

    fn reload(&mut self) {
        let path = self.path.clone();
        self.size_on_disk = file_size(&path);
        self.job = Some(std::thread::spawn(move || {
            let img = image::open(&path).map_err(|e| e.to_string())?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            Ok(egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
        }));
    }

    fn options(&self) -> egui::TextureOptions {
        if self.smooth { egui::TextureOptions::LINEAR } else { egui::TextureOptions::NEAREST }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if self.job.as_ref().is_some_and(JoinHandle::is_finished) {
            match self.job.take().map(JoinHandle::join) {
                Some(Ok(Ok(image))) => {
                    let first = self.image.is_none();
                    // Small images are pixel art more often than not: sharp pixels.
                    if first && image.size[0].max(image.size[1]) <= 256 {
                        self.smooth = false;
                    }
                    let name = self.path.display().to_string();
                    let options = self.options();
                    match &mut self.texture {
                        Some(tex) => tex.set(image.clone(), options),
                        None => self.texture = Some(ctx.load_texture(name, image.clone(), options)),
                    }
                    self.image = Some(image);
                    self.error = None;
                }
                Some(Ok(Err(e))) => self.error = Some(e),
                _ => self.error = Some("loader crashed".into()),
            }
        } else if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        let ppp = ctx.pixels_per_point();
        let dims = self.image.as_ref().map(|i| i.size);

        ui.horizontal(|ui| {
            if ui.selectable_label(self.zoom.is_none(), t("asset.fit")).on_hover_text(t("asset.fit_hint")).clicked() {
                self.zoom = None;
                self.pan = Vec2::ZERO;
            }
            if ui.selectable_label(self.zoom.is_some_and(|z| (z * ppp - 1.0).abs() < 1e-3), "1:1").on_hover_text(t("asset.actual_hint")).clicked() {
                self.zoom = Some(1.0 / ppp);
                self.pan = Vec2::ZERO;
            }
            ui.separator();
            let before = self.smooth;
            ui.checkbox(&mut self.smooth, t("asset.smooth")).on_hover_text(t("asset.smooth_hint"));
            if before != self.smooth {
                let options = self.options();
                if let (Some(tex), Some(img)) = (&mut self.texture, &self.image) {
                    tex.set(img.clone(), options);
                }
            }
            ui.checkbox(&mut self.checker, t("asset.checker")).on_hover_text(t("asset.checker_hint"));
            ui.separator();
            if ui.button(t("explorer.open_external")).clicked() {
                actions.push(Action::OpenExternal(self.path.clone()));
            }
            if let Some([w, h]) = dims {
                info_right(ui, format!("{w} × {h} px · {}", fmt_bytes(self.size_on_disk)));
            }
        });

        if let Some(e) = &self.error {
            centered(ui, RichText::new(tf("asset.error", &[("error", e)])).color(theme::ERROR));
            return;
        }
        let (Some(tex), Some([w, h])) = (&self.texture, dims) else {
            centered(ui, RichText::new(t("asset.loading")).color(theme::TEXT_DIM));
            return;
        };
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let img = Vec2::new(w as f32, h as f32);
        let fit = (rect.width() / img.x).min(rect.height() / img.y).max(1e-4);
        let mut scale = self.zoom.unwrap_or(fit);

        // Wheel: zoom around the cursor; drag: pan; double-click: fit.
        let wheel = if response.hovered() { wheel_steps(ui) } else { 0.0 };
        if wheel != 0.0 {
            let new = (scale * 1.15f32.powf(wheel)).clamp(fit.min(1.0) * 0.1, 64.0);
            if let Some(cursor) = response.hover_pos() {
                // Keep the image point under the cursor in place.
                let center = rect.center() + self.pan;
                let k = new / scale;
                self.pan += (cursor - center) * (1.0 - k);
            }
            scale = new;
            self.zoom = Some(scale);
        }
        if response.dragged() {
            self.pan += response.drag_delta();
            if self.zoom.is_none() {
                self.zoom = Some(scale);
            }
        }
        if response.double_clicked() {
            self.zoom = None;
            self.pan = Vec2::ZERO;
            scale = fit;
        }

        let painter = ui.painter_at(rect);
        let shown = Rect::from_center_size(rect.center() + self.pan, img * scale);
        if self.checker {
            let visible = shown.intersect(rect);
            painter.rect_filled(visible, 0.0, Color32::from_gray(0x9a));
            let cell = 10.0;
            let (x0, y0) = (((visible.left() - shown.left()) / cell).floor() as i64, ((visible.top() - shown.top()) / cell).floor() as i64);
            let (x1, y1) = (((visible.right() - shown.left()) / cell).ceil() as i64, ((visible.bottom() - shown.top()) / cell).ceil() as i64);
            for y in y0..y1 {
                for x in x0..x1 {
                    if (x + y) % 2 == 0 {
                        let min = shown.min + egui::vec2(x as f32 * cell, y as f32 * cell);
                        let r = Rect::from_min_size(min, egui::vec2(cell, cell)).intersect(visible);
                        painter.rect_filled(r, 0.0, Color32::from_gray(0x6a));
                    }
                }
            }
        } else {
            painter.rect_filled(shown.intersect(rect), 0.0, Color32::BLACK);
        }
        painter.image(tex.id(), shown, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        painter.rect_stroke(shown, 0.0, egui::Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Outside);

        // Pixel under the cursor.
        let mut status = format!("{:.0}%", scale * ppp * 100.0);
        if let (Some(cursor), Some(image)) = (response.hover_pos(), &self.image) {
            let p = (cursor - shown.min) / scale;
            if p.x >= 0.0 && p.y >= 0.0 && (p.x as usize) < w && (p.y as usize) < h {
                let (x, y) = (p.x as usize, p.y as usize);
                let [r, g, b, a] = image.pixels[y * w + x].to_srgba_unmultiplied();
                status = format!("{status} · {x}, {y} · RGBA {r} {g} {b} {a} · #{r:02x}{g:02x}{b:02x}{a:02x}");
            }
        }
        hint(&painter, rect, &format!("{status}\n{}", t("asset.image_hint")));
    }
}

// ---------------------------------------------------------------------- sound

struct LoadedSound {
    data: StaticSoundData,
    /// (min, max) of the mono mix per slice of the sound.
    peaks: Vec<(f32, f32)>,
    stereo: bool,
}

struct SoundPreview {
    path: PathBuf,
    job: Option<JoinHandle<Result<LoadedSound, String>>>,
    sound: Option<LoadedSound>,
    error: Option<String>,
    size_on_disk: u64,
    handle: Option<StaticSoundHandle>,
    looping: bool,
    /// Linear amplitude 0..1.
    volume: f32,
    /// Where Play starts when nothing is playing (seconds).
    cursor: f64,
}

impl SoundPreview {
    fn new(path: PathBuf) -> Self {
        let mut p = Self { path, job: None, sound: None, error: None, size_on_disk: 0, handle: None, looping: false, volume: 0.8, cursor: 0.0 };
        p.reload();
        p
    }

    fn reload(&mut self) {
        self.stop();
        let path = self.path.clone();
        self.size_on_disk = file_size(&path);
        self.job = Some(std::thread::spawn(move || {
            let data = StaticSoundData::from_file(&path).map_err(|e| e.to_string())?;
            let frames = &data.frames;
            let stereo = frames.iter().any(|f| (f.left - f.right).abs() > 1e-6);
            let n = frames.len();
            let buckets = PEAKS.min(n.max(1));
            let mut peaks = Vec::with_capacity(buckets);
            for b in 0..buckets {
                let (s, e) = (b * n / buckets, ((b + 1) * n / buckets).max(b * n / buckets + 1).min(n));
                let (mut lo, mut hi) = (0.0f32, 0.0f32);
                for f in &frames[s.min(n)..e] {
                    let v = (f.left + f.right) * 0.5;
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
                peaks.push((lo, hi));
            }
            Ok(LoadedSound { data, peaks, stereo })
        }));
    }

    fn stop(&mut self) {
        if let Some(mut h) = self.handle.take() {
            h.stop(Tween::default());
        }
    }

    fn duration(&self) -> f64 {
        self.sound.as_ref().map_or(0.0, |s| s.data.duration().as_secs_f64())
    }

    fn playing(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !matches!(h.state(), PlaybackState::Stopped))
    }

    fn position(&self) -> f64 {
        match &self.handle {
            Some(h) if !matches!(h.state(), PlaybackState::Stopped) => h.position(),
            _ => self.cursor,
        }
    }

    fn decibels(&self) -> Decibels {
        if self.volume <= 0.001 { Decibels::SILENCE } else { Decibels(20.0 * self.volume.log10()) }
    }

    fn play(&mut self, audio: &mut Option<AudioManager<DefaultBackend>>, audio_error: &mut Option<String>, from: f64) {
        let Some(sound) = &self.sound else { return };
        if audio.is_none() {
            match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
                Ok(m) => *audio = Some(m),
                Err(e) => {
                    *audio_error = Some(format!("{e:?}"));
                    return;
                }
            }
        }
        let Some(manager) = audio.as_mut() else { return };
        let mut data = sound.data.start_position(from).volume(self.decibels());
        if self.looping {
            data = data.loop_region(0.0..);
        }
        self.stop();
        match manager.play(data) {
            Ok(h) => {
                self.handle = Some(h);
                *audio_error = None;
            }
            Err(e) => *audio_error = Some(format!("{e:?}")),
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if self.job.as_ref().is_some_and(JoinHandle::is_finished) {
            match self.job.take().map(JoinHandle::join) {
                Some(Ok(Ok(s))) => {
                    self.sound = Some(s);
                    self.error = None;
                    self.cursor = self.cursor.min(self.duration());
                }
                Some(Ok(Err(e))) => self.error = Some(e),
                _ => self.error = Some("loader crashed".into()),
            }
        } else if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        if self.handle.as_ref().is_some_and(|h| matches!(h.state(), PlaybackState::Stopped)) {
            self.handle = None;
            self.cursor = 0.0;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, audio: &mut Option<AudioManager<DefaultBackend>>, audio_error: &mut Option<String>, actions: &mut Vec<Action>) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        let ready = self.sound.is_some();
        let duration = self.duration();

        ui.horizontal(|ui| {
            let paused = self.handle.as_ref().is_some_and(|h| matches!(h.state(), PlaybackState::Paused | PlaybackState::Pausing));
            let playing = self.playing() && !paused;
            let (label, tip) = if playing { (icon::PAUSE, t("game.pause")) } else { (icon::RUN, t("asset.play")) };
            let space = ui.input(|i| i.key_pressed(egui::Key::Space)) && ui.memory(|m| m.focused().is_none());
            if ui.add_enabled(ready, theme::primary_button(label)).on_hover_text(format!("{tip} (Space)")).clicked() || (ready && space) {
                match &mut self.handle {
                    Some(h) if playing => h.pause(Tween::default()),
                    Some(h) if paused => h.resume(Tween::default()),
                    _ => {
                        let from = if self.cursor >= duration - 0.01 { 0.0 } else { self.cursor };
                        self.play(audio, audio_error, from);
                    }
                }
            }
            if ui.add_enabled(self.handle.is_some(), egui::Button::new(icon::STOP)).on_hover_text(t("viewport.stop")).clicked() {
                self.stop();
                self.cursor = 0.0;
            }
            if ui.selectable_label(self.looping, "🔁").on_hover_text(t("asset.loop_hint")).clicked() {
                self.looping = !self.looping;
                if let Some(h) = &mut self.handle {
                    if self.looping { h.set_loop_region(0.0..) } else { h.set_loop_region(None::<Region>) }
                }
            }
            ui.label(if self.volume <= 0.001 { "🔇" } else { "🔊" }).on_hover_text(t("asset.volume"));
            if ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false)).changed() {
                let db = self.decibels();
                if let Some(h) = &mut self.handle {
                    h.set_volume(db, Tween::default());
                }
            }
            ui.label(RichText::new(format!("{} / {}", fmt_time(self.position()), fmt_time(duration))).monospace());
            ui.separator();
            if ui.button(t("explorer.open_external")).clicked() {
                actions.push(Action::OpenExternal(self.path.clone()));
            }
            if let Some(s) = &self.sound {
                let ch = if s.stereo { t("asset.stereo") } else { t("asset.mono") };
                info_right(ui, format!("{} Hz · {ch} · {}", s.data.sample_rate, fmt_bytes(self.size_on_disk)));
            }
        });
        if let Some(e) = audio_error.as_ref() {
            ui.colored_label(theme::ERROR, tf("asset.no_audio", &[("error", e)]));
        }

        if let Some(e) = &self.error {
            centered(ui, RichText::new(tf("asset.error", &[("error", e)])).color(theme::ERROR));
            return;
        }
        let Some(sound) = &self.sound else {
            centered(ui, RichText::new(t("asset.loading")).color(theme::TEXT_DIM));
            return;
        };

        let rect = ui.available_rect_before_wrap().shrink2(egui::vec2(8.0, 8.0));
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect.expand(2.0));
        painter.rect_filled(rect, 4.0, theme::SURFACE);
        let mid = rect.center().y;
        painter.line_segment([Pos2::new(rect.left(), mid), Pos2::new(rect.right(), mid)], egui::Stroke::new(1.0, theme::BORDER));
        let pos = self.position();
        let played_x = if duration > 0.0 { rect.left() + (pos / duration) as f32 * rect.width() } else { rect.left() };
        let cols = rect.width().max(1.0) as usize;
        let n = sound.peaks.len();
        let half = rect.height() * 0.45;
        for c in 0..cols {
            let (s, e) = (c * n / cols, ((c + 1) * n / cols).max(c * n / cols + 1).min(n));
            let (mut lo, mut hi) = (0.0f32, 0.0f32);
            for &(a, b) in &sound.peaks[s.min(n)..e] {
                lo = lo.min(a);
                hi = hi.max(b);
            }
            let x = rect.left() + c as f32 + 0.5;
            let color = if x <= played_x { theme::ACCENT } else { Color32::from_rgb(0x6f, 0x8f, 0xb8) };
            let (top, bottom) = (mid - hi.clamp(-1.0, 1.0) * half, mid - lo.clamp(-1.0, 1.0) * half);
            painter.line_segment([Pos2::new(x, top.min(mid - 0.5)), Pos2::new(x, bottom.max(mid + 0.5))], egui::Stroke::new(1.0, color));
        }
        painter.line_segment([Pos2::new(played_x, rect.top()), Pos2::new(played_x, rect.bottom())], egui::Stroke::new(1.5, Color32::WHITE));

        // Click / drag on the waveform: seek.
        if let Some(p) = response.interact_pointer_pos().filter(|_| response.clicked() || response.dragged()) {
            let at = (((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64) * duration;
            match &mut self.handle {
                Some(h) => h.seek_to(at),
                None => self.cursor = at,
            }
        }
        if let Some(p) = response.hover_pos() {
            let at = (((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64) * duration;
            painter.line_segment([Pos2::new(p.x, rect.top()), Pos2::new(p.x, rect.bottom())], egui::Stroke::new(1.0, Color32::from_white_alpha(60)));
            response.clone().on_hover_text_at_pointer(fmt_time(at));
        }
        hint(&painter, rect, t("asset.sound_hint"));
        if self.playing() {
            ctx.request_repaint();
        }
    }
}

impl Drop for SoundPreview {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------- model

struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    id: egui::TextureId,
    size: (u32, u32),
}

#[derive(Default)]
struct ModelStats {
    nodes: usize,
    meshes: usize,
    vertices: usize,
    triangles: usize,
    skins: usize,
    size: Vec3,
}

struct Orbit {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    dist: f32,
    /// Bounding radius (camera speeds and clip planes).
    radius: f32,
}

impl Orbit {
    fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }
}

struct ModelPreview {
    path: PathBuf,
    gpu: Option<egui_wgpu::RenderState>,
    renderer: Option<Renderer>,
    target: Option<Target>,
    world: Option<World>,
    root: Option<ObjectId>,
    error: Option<String>,
    gpu_error: Option<String>,
    stats: ModelStats,
    clips: Vec<(String, f32)>,
    /// Playing clip (index into `clips`).
    clip: Option<usize>,
    paused: bool,
    speed: f32,
    spin: bool,
    orbit: Orbit,
    size_on_disk: u64,
    last_frame: Instant,
}

impl ModelPreview {
    fn new(path: PathBuf, gpu: Option<egui_wgpu::RenderState>) -> Self {
        let mut p = Self {
            path,
            gpu,
            renderer: None,
            target: None,
            world: None,
            root: None,
            error: None,
            gpu_error: None,
            stats: ModelStats::default(),
            clips: Vec::new(),
            clip: None,
            paused: false,
            speed: 1.0,
            spin: false,
            orbit: Orbit { target: Vec3::ZERO, yaw: 0.6, pitch: -0.35, dist: 4.0, radius: 1.0 },
            size_on_disk: 0,
            last_frame: Instant::now(),
        };
        p.reload();
        p
    }

    fn reload(&mut self) {
        self.size_on_disk = file_size(&self.path);
        let first = self.world.is_none() && self.error.is_none();
        let mut world = World::new();
        world.scene.sky = None;
        world.scene.background = Color::rgb(0.11, 0.115, 0.13);
        world.scene.ambient = Color::rgb(0.42, 0.44, 0.5);
        world.scene.sun.direction = Vec3::new(-0.45, -1.0, -0.6);
        world.scene.sun.intensity = 1.1;
        let id = match world.assets.load_model(&self.path.to_string_lossy()) {
            Ok(id) => id,
            Err(e) => {
                self.error = Some(e);
                self.world = None;
                self.root = None;
                return;
            }
        };
        let root = engine::animation::spawn_model(&mut world, id, None);
        let Some(model) = world.assets.model(id) else { return };
        let mut stats = ModelStats { nodes: model.nodes.len(), skins: model.skins.len(), size: model.bounds.size(), ..Default::default() };
        for prims in &model.meshes {
            for p in prims {
                stats.meshes += 1;
                if let Some(m) = world.assets.mesh(p.mesh) {
                    stats.vertices += m.vertices.len();
                    stats.triangles += m.indices.len() / 3;
                }
            }
        }
        let bounds = model.bounds;
        self.clips = model.clips.iter().map(|c| (c.name.clone(), c.duration)).collect();
        self.stats = stats;
        self.world = Some(world);
        self.root = root;
        self.error = None;
        // Keep the camera on reloads of the same file; frame it the first time.
        let radius = (bounds.size().length() * 0.5).max(0.01);
        let center = bounds.center();
        if first || (self.orbit.radius - radius).abs() > radius * 0.5 {
            self.orbit.target = if center.is_finite() { center } else { Vec3::ZERO };
            self.orbit.radius = if radius.is_finite() { radius } else { 1.0 };
            self.frame();
        }
        match self.clip.filter(|&c| c < self.clips.len()) {
            Some(c) => self.play_clip(Some(c)),
            None => self.clip = None,
        }
    }

    /// Camera distance that shows the whole model.
    fn frame(&mut self) {
        let fov = 45f32.to_radians();
        self.orbit.dist = self.orbit.radius / (fov * 0.5).sin() * 1.1;
    }

    fn reset_view(&mut self) {
        self.orbit.yaw = 0.6;
        self.orbit.pitch = -0.35;
        self.frame();
    }

    fn play_clip(&mut self, clip: Option<usize>) {
        self.clip = clip;
        self.paused = false;
        let (Some(world), Some(root)) = (&mut self.world, self.root) else { return };
        match clip.and_then(|c| self.clips.get(c)) {
            Some((name, _)) => {
                let p = AnimParams { looping: true, speed: self.speed, fade: 0.0, restart: true };
                if let Err(e) = engine::animation::play(world, root, name, p) {
                    log::warn!("{e}");
                }
            }
            None => engine::animation::stop(world, root, 0.0),
        }
    }

    fn ensure_gpu(&mut self, size: (u32, u32)) -> bool {
        let Some(rs) = &self.gpu else { return false };
        let renderer = self.renderer.get_or_insert_with(|| {
            Renderer::from_device(rs.instance.clone(), &rs.adapter, rs.device.clone(), rs.queue.clone(), size.0, size.1)
        });
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return true;
        }
        renderer.resize(size.0, size.1);
        let texture = rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("spark model preview"),
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
        let (Some(r), Some(target), Some(world)) = (&mut self.renderer, &self.target, &mut self.world) else { return };
        let (w, h) = target.size;
        world.canvas.set_output_size(w, h);
        let internal = world.render.internal_size(w, h);
        world.canvas.begin_frame(internal);
        let o = &self.orbit;
        let cam = &mut world.scene.camera;
        cam.rotation = o.rotation();
        cam.position = o.target + cam.rotation * Vec3::new(0.0, 0.0, o.dist);
        cam.fov = 45.0;
        cam.near = (o.dist * 0.01).max(0.001);
        cam.far = o.dist * 4.0 + o.radius * 4.0;
        self.gpu_error = r.render_to_view(world, &target.view, VIEW_FORMAT).err().map(|e| e.to_string());
    }

    fn ui(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        ui.horizontal(|ui| {
            let names: Vec<String> = self.clips.iter().map(|(n, d)| format!("{n} ({d:.1} s)")).collect();
            let selected = self.clip.and_then(|c| names.get(c).cloned()).unwrap_or_else(|| t("asset.rest_pose").to_string());
            let mut pick = self.clip;
            ui.add_enabled_ui(!self.clips.is_empty(), |ui| {
                egui::ComboBox::from_id_salt(("model-clip", &self.path)).selected_text(selected).width(180.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut pick, None, t("asset.rest_pose"));
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(&mut pick, Some(i), n);
                    }
                })
                .response
                .on_hover_text(t("asset.animation"));
            });
            if pick != self.clip {
                self.play_clip(pick);
            }
            if self.clip.is_some() {
                let (label, tip) = if self.paused { (icon::RUN, t("game.resume")) } else { (icon::PAUSE, t("game.pause")) };
                if ui.button(label).on_hover_text(tip).clicked() {
                    self.paused = !self.paused;
                }
                ui.label(t("asset.speed"));
                if ui.add(egui::DragValue::new(&mut self.speed).speed(0.02).range(0.0..=5.0).suffix("×")).changed() {
                    if let (Some(world), Some(root)) = (&mut self.world, self.root) {
                        engine::animation::set_speed(world, root, self.speed);
                    }
                }
            }
            ui.separator();
            ui.toggle_value(&mut self.spin, "⟲").on_hover_text(t("asset.spin_hint"));
            if ui.button("🎥").on_hover_text(t("asset.frame_hint")).clicked() {
                self.reset_view();
            }
            if ui.button(icon::RELOAD).on_hover_text(t("asset.reload_hint")).clicked() {
                self.reload();
            }
            ui.separator();
            if self.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gltf")) && ui.button(t("asset.open_text")).clicked() {
                actions.push(Action::OpenText(self.path.clone()));
            }
            if ui.button(t("explorer.open_external")).clicked() {
                actions.push(Action::OpenExternal(self.path.clone()));
            }
            info_right(ui, fmt_bytes(self.size_on_disk));
        });

        if let Some(e) = &self.error {
            centered(ui, RichText::new(tf("asset.error", &[("error", e)])).color(theme::ERROR));
            return;
        }
        if self.gpu.is_none() {
            centered(ui, RichText::new(t("asset.no_gpu")).color(theme::TEXT_DIM));
            return;
        }
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ppp = ctx.pixels_per_point();
        let size = ((rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32);
        if rect.width() < 2.0 || rect.height() < 2.0 || !self.ensure_gpu(size) {
            return;
        }

        // LMB: orbit; RMB / MMB / Shift+LMB: pan; wheel: zoom; double-click: frame.
        let shift = ui.input(|i| i.modifiers.shift);
        let d = response.drag_delta();
        let pan = response.dragged_by(egui::PointerButton::Middle)
            || response.dragged_by(egui::PointerButton::Secondary)
            || (shift && response.dragged_by(egui::PointerButton::Primary));
        if pan {
            let rot = self.orbit.rotation();
            let k = self.orbit.dist * 0.0018;
            self.orbit.target += (rot * Vec3::X * -d.x + rot * Vec3::Y * d.y) * k;
        } else if response.dragged_by(egui::PointerButton::Primary) {
            self.orbit.yaw -= d.x * 0.01;
            self.orbit.pitch = (self.orbit.pitch - d.y * 0.01).clamp(-1.55, 1.55);
        }
        let wheel = if response.hovered() { wheel_steps(ui) } else { 0.0 };
        if wheel != 0.0 {
            self.orbit.dist = (self.orbit.dist * 0.88f32.powf(wheel)).clamp(self.orbit.radius * 0.05, self.orbit.radius * 200.0);
        }
        if response.double_clicked() {
            self.reset_view();
        }

        let animating = self.clip.is_some() && !self.paused;
        if let Some(world) = &mut self.world {
            if animating {
                engine::animation::update(world, dt);
            }
        }
        if self.spin && !response.dragged() {
            self.orbit.yaw += dt * 0.6;
        }
        if animating || self.spin {
            ctx.request_repaint();
        }
        self.render();
        if let Some(target) = &self.target {
            ui.painter().image(target.id, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        let painter = ui.painter_at(rect);
        if let Some(e) = &self.gpu_error {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, e, egui::FontId::proportional(12.0), theme::ERROR);
        }
        let s = &self.stats;
        let mut text = tf(
            "asset.stats",
            &[("nodes", &fmt_count(s.nodes)), ("meshes", &fmt_count(s.meshes)), ("vertices", &fmt_count(s.vertices)), ("triangles", &fmt_count(s.triangles))],
        );
        text.push_str(&format!("\n{}", tf("asset.size_m", &[("x", &format!("{:.2}", s.size.x)), ("y", &format!("{:.2}", s.size.y)), ("z", &format!("{:.2}", s.size.z))])));
        if !self.clips.is_empty() || s.skins > 0 {
            text.push_str(&format!("\n{}", tf("asset.anim_stats", &[("clips", &self.clips.len()), ("skins", &s.skins)])));
        }
        text.push_str(&format!("\n{}", t("asset.model_hint")));
        hint(&painter, rect, &text);
    }
}

impl Drop for ModelPreview {
    fn drop(&mut self) {
        if let (Some(rs), Some(target)) = (&self.gpu, &self.target) {
            rs.renderer.write().free_texture(&target.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind(Path::new("a/b.PNG")), Some(Kind::Image));
        assert_eq!(kind(Path::new("x.ogg")), Some(Kind::Sound));
        assert_eq!(kind(Path::new("m.glb")), Some(Kind::Model));
        assert_eq!(kind(Path::new("m.gltf")), Some(Kind::Model));
        assert_eq!(kind(Path::new("main.luau")), None);
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_time(83.3), "1:23.3");
        assert_eq!(fmt_count(1234567), "1 234 567");
        assert_eq!(fmt_bytes(2048), "2.0 KB");
    }
}
