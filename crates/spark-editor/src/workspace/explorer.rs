//! The project file tree. Scanned once and re-scanned when the file watcher reports changes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, vec2};

use super::Action;
use crate::i18n::t;
use crate::theme::{self, icon};

/// Never shown.
const ALWAYS_HIDDEN: &[&str] = &[".git", ".DS_Store", "Thumbs.db"];
/// Generated folders: shown dimmed.
const GENERATED: &[&str] = &["dist", "screenshots", "target"];
const MAX_ENTRIES: usize = 20_000;
const ROW: f32 = 22.0;
const INDENT: f32 = 14.0;

struct Node {
    name: String,
    path: PathBuf,
    dir: bool,
    dimmed: bool,
    children: Vec<Node>,
}

pub struct Explorer {
    root: PathBuf,
    title: String,
    nodes: Vec<Node>,
    expanded: HashSet<PathBuf>,
    pub selected: Option<PathBuf>,
    needs_scan: bool,
    show_hidden: bool,
}

impl Explorer {
    pub fn new(root: &Path, title: &str) -> Self {
        let mut expanded = HashSet::new();
        expanded.insert(root.join("src"));
        Self { root: root.to_path_buf(), title: title.to_string(), nodes: Vec::new(), expanded, selected: None, needs_scan: true, show_hidden: false }
    }

    /// Something changed on disk: re-scan before the next draw.
    pub fn invalidate(&mut self) {
        self.needs_scan = true;
    }

    /// Expands the folders down to `path` and selects it.
    pub fn reveal(&mut self, path: &Path) {
        let mut p = path.parent();
        while let Some(dir) = p {
            if dir == self.root || !dir.starts_with(&self.root) {
                break;
            }
            self.expanded.insert(dir.to_path_buf());
            p = dir.parent();
        }
        self.selected = Some(path.to_path_buf());
    }

    fn scan(&mut self) {
        let mut budget = MAX_ENTRIES;
        self.nodes = scan_dir(&self.root, self.show_hidden, false, &mut budget);
        self.needs_scan = false;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, show_hidden: bool, actions: &mut Vec<Action>) {
        if show_hidden != self.show_hidden {
            self.show_hidden = show_hidden;
            self.needs_scan = true;
        }
        if self.needs_scan {
            self.scan();
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.title).strong().color(Color32::WHITE));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("−").on_hover_text(t("explorer.collapse_all")).clicked() {
                    self.expanded.clear();
                }
                if ui.small_button(icon::RELOAD).on_hover_text(t("explorer.refresh")).clicked() {
                    self.needs_scan = true;
                }
                if ui.small_button("🗀+").on_hover_text(t("explorer.new_folder")).clicked() {
                    actions.push(Action::NewFolder(self.target_dir()));
                }
                if ui.small_button("🖹+").on_hover_text(t("explorer.new_file")).clicked() {
                    actions.push(Action::NewFile(self.target_dir()));
                }
            });
        });
        ui.separator();
        let root = self.root.clone();
        let resp = egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let nodes = std::mem::take(&mut self.nodes);
                for n in &nodes {
                    self.row(ui, n, 0, actions);
                }
                self.nodes = nodes;
                // Empty space below the tree: context menu for the project root.
                let rest = ui.allocate_response(vec2(ui.available_width(), ui.available_height().max(40.0)), Sense::click());
                rest.context_menu(|ui| dir_menu(ui, &root, &root, actions));
            });
        let _ = resp;
    }

    /// Folder for "new file": the selected folder, the selected file's folder, else the root.
    fn target_dir(&self) -> PathBuf {
        match &self.selected {
            Some(p) if p.is_dir() => p.clone(),
            Some(p) => p.parent().map(Path::to_path_buf).unwrap_or_else(|| self.root.clone()),
            None => self.root.clone(),
        }
    }

    fn row(&mut self, ui: &mut egui::Ui, node: &Node, depth: usize, actions: &mut Vec<Action>) {
        let width = ui.available_width().max(160.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW), Sense::click());
        let open = node.dir && self.expanded.contains(&node.path);
        let selected = self.selected.as_deref() == Some(node.path.as_path());
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 3.0, theme::ACCENT_DIM.gamma_multiply(0.7));
        } else if resp.hovered() {
            painter.rect_filled(rect, 3.0, Color32::from_white_alpha(10));
        }
        let mut x = rect.left() + 6.0 + depth as f32 * INDENT;
        let y = rect.center().y;
        if node.dir {
            painter.text(egui::pos2(x, y), Align2::LEFT_CENTER, if open { "⏷" } else { "⏵" }, FontId::proportional(11.0), theme::TEXT_DIM);
        }
        x += 14.0;
        let (glyph, color) = file_icon(node, open);
        painter.text(egui::pos2(x, y), Align2::LEFT_CENTER, glyph, FontId::proportional(13.0), color);
        x += 20.0;
        let text_color = if node.dimmed { theme::TEXT_DIM } else { theme::TEXT };
        painter.text(egui::pos2(x, y), Align2::LEFT_CENTER, &node.name, FontId::proportional(13.5), text_color);

        if resp.clicked() {
            self.selected = Some(node.path.clone());
            if node.dir {
                if !self.expanded.remove(&node.path) {
                    self.expanded.insert(node.path.clone());
                }
            } else {
                actions.push(Action::OpenFile(node.path.clone()));
            }
        }
        if resp.double_clicked() && !node.dir && !is_text_file(&node.path) && super::assets::kind(&node.path).is_none() {
            actions.push(Action::OpenExternal(node.path.clone()));
        }
        let root = self.root.clone();
        resp.context_menu(|ui| {
            self.selected = Some(node.path.clone());
            if node.dir {
                dir_menu(ui, &root, &node.path, actions);
            } else {
                file_menu(ui, &node.path, actions);
            }
        });
        if open {
            for c in &node.children {
                self.row(ui, c, depth + 1, actions);
            }
        }
    }
}

fn dir_menu(ui: &mut egui::Ui, root: &Path, dir: &Path, actions: &mut Vec<Action>) {
    if ui.button(t("explorer.new_file")).clicked() {
        actions.push(Action::NewFile(dir.to_path_buf()));
    }
    if ui.button(t("explorer.new_folder")).clicked() {
        actions.push(Action::NewFolder(dir.to_path_buf()));
    }
    ui.separator();
    if ui.button(t("explorer.reveal")).clicked() {
        actions.push(Action::Reveal(dir.to_path_buf()));
    }
    if ui.button(t("explorer.copy_path")).clicked() {
        ui.ctx().copy_text(dir.display().to_string());
    }
    if dir != root {
        ui.separator();
        if ui.button(t("explorer.rename")).clicked() {
            actions.push(Action::Rename(dir.to_path_buf()));
        }
        if ui.button(t("explorer.delete")).clicked() {
            actions.push(Action::Delete(dir.to_path_buf()));
        }
    }
}

fn file_menu(ui: &mut egui::Ui, path: &Path, actions: &mut Vec<Action>) {
    let preview = super::assets::kind(path).is_some();
    if (is_text_file(path) || preview) && ui.button(t("explorer.open")).clicked() {
        actions.push(Action::OpenFile(path.to_path_buf()));
    }
    if preview && is_text_file(path) && ui.button(t("asset.open_text")).clicked() {
        actions.push(Action::OpenText(path.to_path_buf()));
    }
    if ui.button(t("explorer.open_external")).clicked() {
        actions.push(Action::OpenExternal(path.to_path_buf()));
    }
    if ui.button(t("explorer.reveal")).clicked() {
        actions.push(Action::Reveal(path.to_path_buf()));
    }
    if ui.button(t("explorer.copy_path")).clicked() {
        ui.ctx().copy_text(path.display().to_string());
    }
    ui.separator();
    if ui.button(t("explorer.rename")).clicked() {
        actions.push(Action::Rename(path.to_path_buf()));
    }
    if ui.button(t("explorer.delete")).clicked() {
        actions.push(Action::Delete(path.to_path_buf()));
    }
}

fn scan_dir(dir: &Path, show_hidden: bool, dimmed: bool, budget: &mut usize) -> Vec<Node> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Node> = Vec::new();
    for e in entries.flatten() {
        if *budget == 0 {
            break;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if ALWAYS_HIDDEN.contains(&name.as_str()) || (!show_hidden && name.starts_with('.')) {
            continue;
        }
        *budget -= 1;
        let path = e.path();
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let dim = dimmed || (is_dir && GENERATED.contains(&name.as_str()));
        let children = if is_dir { scan_dir(&path, show_hidden, dim, budget) } else { Vec::new() };
        out.push(Node { name, path, dir: is_dir, dimmed: dim, children });
    }
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// Files the code editor opens (everything else opens in the OS program).
pub fn is_text_file(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    matches!(
        ext.as_str(),
        "luau" | "lua" | "wgsl" | "toml" | "md" | "txt" | "json" | "gltf" | "csv" | "ini" | "cfg" | "glsl" | "luaurc" | "gitignore" | ""
    ) || path.file_name().is_some_and(|n| n == ".luaurc" || n == ".gitignore")
}

fn file_icon(node: &Node, open: bool) -> (&'static str, Color32) {
    if node.dir {
        return (if open { icon::FOLDER_OPEN } else { icon::FOLDER }, Color32::from_rgb(0xd8, 0xa6, 0x4a));
    }
    let ext = node.path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    match ext.as_str() {
        "luau" | "lua" => (icon::SCRIPT, Color32::from_rgb(0x56, 0xa8, 0xf5)),
        "wgsl" | "glsl" => (icon::SHADER, Color32::from_rgb(0xc6, 0x7a, 0xf0)),
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "hdr" | "ktx2" => (icon::IMAGE, Color32::from_rgb(0x5f, 0xb8, 0x65)),
        "wav" | "ogg" | "mp3" | "flac" => (icon::SOUND, Color32::from_rgb(0xe0, 0x8a, 0x4a)),
        "gltf" | "glb" | "obj" => (icon::MODEL, Color32::from_rgb(0x4a, 0xc0, 0xc0)),
        "toml" => (icon::SETTINGS, theme::TEXT_DIM),
        _ => (icon::FILE, theme::TEXT_DIM),
    }
}
