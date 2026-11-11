//! Open files and the code editor widget (syntax highlighting, line numbers, go-to-line).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use eframe::egui::{self, Color32, FontId, RichText, TextBuffer, text::LayoutJob};

use crate::i18n::{t, tf};
use crate::syntax::{self, Lang};
use crate::theme;

/// Files bigger than this are not opened as text.
const MAX_TEXT_FILE: u64 = 8 * 1024 * 1024;

pub struct Document {
    pub path: PathBuf,
    pub text: String,
    saved: String,
    pub lang: Lang,
    /// The file used `\r\n`: kept on save (the editor works with `\n`).
    crlf: bool,
    disk_mtime: Option<SystemTime>,
    /// Changed on disk while it had unsaved edits: ask what to keep.
    pub conflict: bool,
    /// Scroll to / put the cursor on this line (1-based) on the next frame.
    pub goto: Option<u32>,
    /// 1-based cursor line / column (status bar).
    pub cursor: (usize, usize),
    highlight: Option<(u64, Arc<egui::Galley>)>,
}

impl Document {
    pub fn load(path: &Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if meta.len() > MAX_TEXT_FILE {
            return Err(tf("editor.too_big", &[("size", &(meta.len() / 1024 / 1024))]));
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let raw = String::from_utf8(bytes).map_err(|_| t("editor.not_text").to_string())?;
        let crlf = raw.contains("\r\n");
        let text = if crlf { raw.replace("\r\n", "\n") } else { raw };
        Ok(Self {
            path: path.to_path_buf(),
            saved: text.clone(),
            text,
            lang: Lang::from_path(path),
            crlf,
            disk_mtime: meta.modified().ok(),
            conflict: false,
            goto: None,
            cursor: (1, 1),
            highlight: None,
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.text != self.saved
    }

    pub fn save(&mut self) -> Result<(), String> {
        let out = if self.crlf { self.text.replace('\n', "\r\n") } else { self.text.clone() };
        std::fs::write(&self.path, out).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.saved = self.text.clone();
        self.disk_mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        self.conflict = false;
        Ok(())
    }

    /// Called when the file changed on disk: reloads clean documents, flags a conflict for dirty ones.
    /// Returns false when the file is gone.
    pub fn on_disk_change(&mut self) -> bool {
        let Ok(meta) = std::fs::metadata(&self.path) else { return false };
        let mtime = meta.modified().ok();
        if mtime == self.disk_mtime {
            return true; // our own save
        }
        if self.is_dirty() {
            self.conflict = true;
            self.disk_mtime = mtime;
        } else {
            self.reload();
        }
        true
    }

    pub fn reload(&mut self) {
        if let Ok(fresh) = Self::load(&self.path) {
            let (goto, cursor) = (self.goto, self.cursor);
            *self = fresh;
            self.goto = goto;
            self.cursor = cursor;
        }
    }
}

/// Every open document, by path.
#[derive(Default)]
pub struct Documents {
    docs: HashMap<PathBuf, Document>,
}

impl Documents {
    pub fn get(&self, path: &Path) -> Option<&Document> {
        self.docs.get(path)
    }

    pub fn get_mut(&mut self, path: &Path) -> Option<&mut Document> {
        self.docs.get_mut(path)
    }

    /// Opens (or returns the already open) document.
    pub fn open(&mut self, path: &Path) -> Result<&mut Document, String> {
        if !self.docs.contains_key(path) {
            let doc = Document::load(path)?;
            self.docs.insert(path.to_path_buf(), doc);
        }
        Ok(self.docs.get_mut(path).expect("just inserted"))
    }

    pub fn close(&mut self, path: &Path) {
        self.docs.remove(path);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Document> {
        self.docs.values()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Document> {
        self.docs.values_mut()
    }

    pub fn any_dirty(&self) -> bool {
        self.docs.values().any(Document::is_dirty)
    }

    /// After a rename / move on disk.
    pub fn rename(&mut self, from: &Path, to: &Path) {
        let moved: Vec<PathBuf> = self.docs.keys().filter(|p| p.starts_with(from)).cloned().collect();
        for old in moved {
            if let Some(mut doc) = self.docs.remove(&old) {
                let new = to.join(old.strip_prefix(from).unwrap_or(Path::new("")));
                let new = if old == from { to.to_path_buf() } else { new };
                doc.path = new.clone();
                doc.lang = Lang::from_path(&new);
                self.docs.insert(new, doc);
            }
        }
    }
}

pub enum EditorEvent {
    None,
    /// Ctrl+S inside the editor (handled globally too, but focus may swallow it).
    Save,
}

/// The code editor for one document.
pub fn editor_ui(ui: &mut egui::Ui, doc: &mut Document, font_size: f32) -> EditorEvent {
    if doc.conflict {
        egui::Frame::new().fill(Color32::from_rgb(0x4a, 0x3a, 0x16)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} {}", theme::icon::WARN, t("editor.conflict"))).color(theme::WARN));
                if ui.button(t("editor.reload_disk")).clicked() {
                    doc.reload();
                }
                if ui.button(t("editor.keep_mine")).clicked() {
                    doc.conflict = false;
                }
            });
        });
    }

    let font = FontId::monospace(font_size);
    let row_height = ui.fonts_mut(|f| f.row_height(&font));
    let lines = doc.text.split('\n').count().max(1);
    let digits = lines.to_string().len().max(3);
    let id = egui::Id::new(("code-editor", &doc.path));

    let mut scroll = egui::ScrollArea::both().id_salt(("code-scroll", &doc.path)).auto_shrink([false, false]);
    let goto = doc.goto.take();
    if let Some(line) = goto {
        scroll = scroll.vertical_scroll_offset(((line.saturating_sub(1)) as f32 * row_height - 160.0).max(0.0));
    }

    let Document { text, lang, highlight, cursor, .. } = doc;
    let lang = *lang;
    let mut event = EditorEvent::None;
    egui::Frame::new().fill(theme::BG).show(ui, |ui| {
        scroll.show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                // Line numbers: one monospace label, rows match the text rows exactly (no wrapping).
                let numbers: String = (1..=lines).map(|n| format!("{n:>digits$}\n")).collect();
                ui.add(egui::Label::new(RichText::new(numbers).font(font.clone()).color(Color32::from_gray(90))).selectable(false));

                let mut layouter = |ui: &egui::Ui, buf: &dyn TextBuffer, _wrap: f32| {
                    let src = buf.as_str();
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    src.hash(&mut h);
                    font_size.to_bits().hash(&mut h);
                    let key = h.finish();
                    if let Some((k, galley)) = highlight.as_ref() {
                        if *k == key {
                            return galley.clone();
                        }
                    }
                    let galley = ui.fonts_mut(|f| f.layout_job(layout_job(src, lang, &font)));
                    *highlight = Some((key, galley.clone()));
                    galley
                };
                let output = egui::TextEdit::multiline(text)
                    .id(id)
                    .font(font.clone())
                    .code_editor()
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::ZERO)
                    .desired_width(f32::INFINITY)
                    .desired_rows(lines)
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .show(ui);

                if let Some(range) = output.cursor_range {
                    let idx = range.primary.index.0;
                    let before: String = text.chars().take(idx).collect();
                    let line = before.matches('\n').count() + 1;
                    let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) + 1;
                    *cursor = (line, col);
                }
                if let Some(line) = goto {
                    let idx: usize = text.split('\n').take(line.saturating_sub(1) as usize).map(|l| l.chars().count() + 1).sum();
                    let mut state = output.state;
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(idx))));
                    state.store(ui.ctx(), id);
                    ui.memory_mut(|m| m.request_focus(id));
                }
                if output.response.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
                    event = EditorEvent::Save;
                }
            });
        });
    });
    event
}

fn layout_job(text: &str, lang: Lang, font: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (range, token) in syntax::highlight(lang, text) {
        job.append(&text[range], 0.0, egui::TextFormat::simple(font.clone(), theme::token_color(token)));
    }
    job.wrap.max_width = f32::INFINITY;
    job
}
