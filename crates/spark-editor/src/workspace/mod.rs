//! An open project: menu, run toolbar, dockable panels (explorer, code tabs, console, problems, debug, scene),
//! status bar, shortcuts and file dialogs.
//!
//! Panels never act directly: they push [`Action`]s, the workspace applies them after drawing. That keeps
//! borrowing simple and gives one place for every command (menu, shortcut, button, context menu).

mod console;
mod documents;
mod explorer;
mod game;
mod problems;
mod watcher;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText};
use egui_dock::tab_viewer::OnCloseResponse;
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use spark_project::Project;

use crate::app::AppAction;
use crate::i18n::{self, Lang, t, tf};
use crate::process::{self, JobKind, Level, editor_line};
use crate::settings::Settings;
use crate::theme::{self, icon};
use crate::toasts::Toasts;

use console::Console;
use documents::{Documents, EditorEvent};
use explorer::Explorer;
use game::{GameSession, GameState, SessionEvent};
use problems::Problems;
use watcher::Watcher;

/// Every command of the workspace.
#[derive(Clone, Debug)]
pub enum Action {
    OpenFile(PathBuf),
    /// Open and go to a line; relative paths are relative to the project.
    OpenAt(PathBuf, u32),
    Save(PathBuf),
    SaveActive,
    SaveAll,
    CloseActive,
    /// Close without asking (after "discard" in the unsaved dialog).
    ForceClose(PathBuf),
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf),
    Delete(PathBuf),
    Reveal(PathBuf),
    OpenExternal(PathBuf),
    Run,
    Stop,
    Restart,
    TogglePause,
    Step,
    Reload,
    Check,
    Build,
    ShowTab(Tab),
    ResetLayout,
    Settings,
    CloseProject,
    Quit,
}

/// A dockable panel.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    /// Shown in the code area when no file is open; cannot be closed.
    Start,
    Explorer,
    Doc(PathBuf),
    Console,
    Problems,
    Debug,
    Scene,
}

enum Dialog {
    NewFile { dir: PathBuf, name: String, error: Option<String> },
    NewFolder { dir: PathBuf, name: String, error: Option<String> },
    Rename { path: PathBuf, name: String, error: Option<String> },
    Delete { path: PathBuf },
    /// Closing a document with unsaved changes.
    CloseUnsaved { path: PathBuf },
    /// Leaving the project with unsaved documents.
    CloseProject,
    Settings,
}

pub struct Workspace {
    project: Project,
    dock: DockState<Tab>,
    dock_lang: Option<Lang>,
    docs: Documents,
    explorer: Explorer,
    console: Console,
    problems: Problems,
    game: GameSession,
    watcher: Option<Watcher>,
    dialog: Option<Dialog>,
    focus_dialog: bool,
    active_doc: Option<PathBuf>,
    clock: Instant,
    watcher_started: bool,
}

fn default_layout() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Start]);
    let tree = dock.main_surface_mut();
    let [center, _explorer] = tree.split_left(NodeIndex::root(), 0.18, vec![Tab::Explorer]);
    let [center, _debug] = tree.split_right(center, 0.74, vec![Tab::Debug, Tab::Scene]);
    let [_code, _bottom] = tree.split_below(center, 0.70, vec![Tab::Console, Tab::Problems]);
    dock
}

impl Workspace {
    pub fn open(project: Project, settings: &Settings, toasts: &mut Toasts) -> Self {
        let mut ws = Self {
            explorer: Explorer::new(&project.root, &project.title),
            game: GameSession::new(&project.root),
            dock: default_layout(),
            dock_lang: None,
            docs: Documents::default(),
            console: Console::default(),
            problems: Problems::default(),
            watcher: None,
            dialog: None,
            focus_dialog: false,
            active_doc: None,
            clock: Instant::now(),
            watcher_started: false,
            project,
        };
        // Files of the last session, else the entry script.
        let root = ws.project.root.clone();
        let recent = settings.recent(&root).cloned().unwrap_or_default();
        for rel in &recent.open_files {
            let path = root.join(rel);
            if path.is_file() {
                ws.open_file(&path, None, toasts);
            }
        }
        match recent.active_file.map(|a| root.join(a)).filter(|p| p.is_file()) {
            Some(active) => ws.open_file(&active, None, toasts),
            None if ws.active_doc.is_none() => {
                let entry = ws.project.entry.clone();
                ws.open_file(&entry, None, toasts);
            }
            None => {}
        }
        ws.console.push(editor_line(JobKind::Run, Level::Info, tf("workspace.opened", &[("path", &root.display())]), &ws.clock));
        ws
    }

    pub fn title(&self) -> &str {
        &self.project.title
    }

    pub fn has_unsaved(&self) -> bool {
        self.docs.any_dirty()
    }

    pub fn save_all(&mut self, toasts: &mut Toasts) -> bool {
        let mut ok = true;
        for doc in self.docs.iter_mut().filter(|d| d.is_dirty()) {
            if let Err(e) = doc.save() {
                toasts.error(e);
                ok = false;
            }
        }
        ok
    }

    /// Stores the open files (relative to the project) for the next session.
    pub fn remember(&self, settings: &mut Settings) {
        let root = &self.project.root;
        let rel = |p: &Path| p.strip_prefix(root).map(Path::to_path_buf).ok();
        let open: Vec<PathBuf> = self
            .dock
            .iter_all_tabs()
            .filter_map(|(_, tab)| match tab {
                Tab::Doc(p) => rel(p),
                _ => None,
            })
            .collect();
        let active = self.active_doc.as_deref().and_then(rel);
        settings.remember_files(root, open, active);
    }

    // ------------------------------------------------------------------ frame

    pub fn ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings, toasts: &mut Toasts) -> Option<AppAction> {
        let ctx = ui.ctx().clone();
        if !self.watcher_started {
            self.watcher_started = true;
            match Watcher::new(&self.project.root, &ctx) {
                Ok(w) => self.watcher = Some(w),
                Err(e) => log::warn!("file watcher: {e}"),
            }
        }
        self.poll(toasts);

        let mut actions = Vec::new();
        self.shortcuts(ui, &mut actions);

        egui::Panel::top("ws-menu").frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(6, 2))).show(ui, |ui| {
            self.menu_bar(ui, &mut actions);
        });
        egui::Panel::top("ws-toolbar").frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(8, 4))).show(ui, |ui| {
            self.toolbar(ui, &mut actions);
        });
        egui::Panel::bottom("ws-status").frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(8, 3))).show(ui, |ui| {
            self.status_bar(ui);
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG)).show(ui, |ui| {
            self.dock_ui(ui, settings, &mut actions);
        });

        let mut result = self.dialogs(&ctx, settings, toasts, &mut actions);
        for action in actions {
            if let Some(a) = self.handle(action, settings, toasts) {
                result = Some(a);
            }
        }
        if self.game.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        result
    }

    /// Game output, file changes.
    fn poll(&mut self, toasts: &mut Toasts) {
        let (lines, events) = self.game.poll(&self.clock);
        self.console.extend(lines);
        for e in events {
            match e {
                SessionEvent::CheckDone(report) => {
                    self.problems.set_check(report.as_ref());
                    match &report {
                        Some(r) if r.ok => toasts.success(t("game.check_ok")),
                        Some(r) => {
                            toasts.error(tf("game.check_errors", &[("count", &r.errors.len())]));
                            self.show_tab(Tab::Problems);
                        }
                        None => toasts.error(t("game.check_no_report")),
                    }
                }
                SessionEvent::ScriptError(msg) => {
                    self.problems.set_runtime(msg.as_deref());
                    if msg.is_some() {
                        toasts.error(t("game.script_error"));
                    }
                }
                SessionEvent::GameExited => self.problems.set_runtime(None),
                SessionEvent::BuildDone(true) => toasts.success(tf("game.build_ok_path", &[("path", &self.project.root.join("dist").display())])),
                SessionEvent::BuildDone(false) => {
                    toasts.error(t("game.build_failed_toast"));
                    self.show_tab(Tab::Console);
                }
            }
        }

        let changes = self.watcher.as_ref().map(Watcher::changes).unwrap_or_default();
        if !changes.is_empty() {
            self.explorer.invalidate();
            for doc in self.docs.iter_mut() {
                if changes.contains(&doc.path) && !doc.on_disk_change() {
                    log::info!("{} was removed on disk", doc.path.display());
                }
            }
        }
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let none = Modifiers::NONE;
        let ctrl = Modifiers::COMMAND;
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let table = [
            (KeyboardShortcut::new(ctrl_shift, Key::S), Action::SaveAll),
            (KeyboardShortcut::new(ctrl, Key::S), Action::SaveActive),
            (KeyboardShortcut::new(ctrl, Key::W), Action::CloseActive),
            (KeyboardShortcut::new(Modifiers::SHIFT, Key::F5), Action::Stop),
            (KeyboardShortcut::new(ctrl, Key::F5), Action::Restart),
            (KeyboardShortcut::new(none, Key::F5), Action::Run),
            (KeyboardShortcut::new(none, Key::F6), Action::TogglePause),
            (KeyboardShortcut::new(none, Key::F7), Action::Step),
            (KeyboardShortcut::new(ctrl, Key::R), Action::Reload),
            (KeyboardShortcut::new(none, Key::F8), Action::Check),
            (KeyboardShortcut::new(ctrl, Key::Comma), Action::Settings),
        ];
        if self.dialog.is_some() {
            return;
        }
        ui.input_mut(|i| {
            for (shortcut, action) in table {
                if i.consume_shortcut(&shortcut) {
                    actions.push(action);
                }
            }
        });
    }

    // ------------------------------------------------------------------ chrome

    fn menu_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let fmt = |s: &KeyboardShortcut| ui.ctx().format_shortcut(s);
        let sc_save = fmt(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S));
        let sc_save_all = fmt(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S));
        let sc_close = fmt(&KeyboardShortcut::new(Modifiers::COMMAND, Key::W));
        let sc_settings = fmt(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma));
        let sc_restart = fmt(&KeyboardShortcut::new(Modifiers::COMMAND, Key::F5));
        let sc_reload = fmt(&KeyboardShortcut::new(Modifiers::COMMAND, Key::R));
        let root = self.project.root.clone();
        let item = |ui: &mut egui::Ui, text: &str, shortcut: &str, enabled: bool| -> bool {
            let clicked = ui.add_enabled(enabled, egui::Button::new(text).shortcut_text(shortcut)).clicked();
            if clicked {
                ui.close();
            }
            clicked
        };
        let state = self.game.state();
        let running = state != GameState::Stopped;
        let live = self.game.is_connected();
        let has_doc = self.active_doc.is_some();

        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(t("menu.file"), |ui| {
                if item(ui, t("menu.new_file"), "", true) {
                    actions.push(Action::NewFile(root.clone()));
                }
                if item(ui, t("menu.new_folder"), "", true) {
                    actions.push(Action::NewFolder(root.clone()));
                }
                ui.separator();
                if item(ui, t("menu.save"), &sc_save, has_doc) {
                    actions.push(Action::SaveActive);
                }
                if item(ui, t("menu.save_all"), &sc_save_all, self.docs.any_dirty()) {
                    actions.push(Action::SaveAll);
                }
                if item(ui, t("menu.close_tab"), &sc_close, has_doc) {
                    actions.push(Action::CloseActive);
                }
                ui.separator();
                if item(ui, t("menu.reveal_project"), "", true) {
                    actions.push(Action::Reveal(root.clone()));
                }
                if item(ui, t("menu.settings"), &sc_settings, true) {
                    actions.push(Action::Settings);
                }
                ui.separator();
                if item(ui, t("menu.close_project"), "", true) {
                    actions.push(Action::CloseProject);
                }
                if item(ui, t("menu.exit"), "", true) {
                    actions.push(Action::Quit);
                }
            });
            ui.menu_button(t("menu.game"), |ui| {
                if item(ui, &format!("{} {}", icon::RUN, t("game.run")), "F5", !running) {
                    actions.push(Action::Run);
                }
                if item(ui, &format!("{} {}", icon::STOP, t("game.stop")), "Shift+F5", running) {
                    actions.push(Action::Stop);
                }
                if item(ui, &format!("{} {}", icon::RELOAD, t("game.restart")), &sc_restart, running) {
                    actions.push(Action::Restart);
                }
                ui.separator();
                let pause = if state == GameState::Paused { t("game.resume") } else { t("game.pause") };
                if item(ui, pause, "F6", live) {
                    actions.push(Action::TogglePause);
                }
                if item(ui, t("game.step"), "F7", live) {
                    actions.push(Action::Step);
                }
                if item(ui, t("game.reload"), &sc_reload, live) {
                    actions.push(Action::Reload);
                }
                ui.separator();
                if item(ui, &format!("{} {}", icon::CHECK, t("game.check")), "F8", !self.game.is_checking()) {
                    actions.push(Action::Check);
                }
                if item(ui, &format!("{} {}", icon::BUILD, t("game.build")), "", !self.game.is_building()) {
                    actions.push(Action::Build);
                }
                if item(ui, t("menu.open_dist"), "", root.join("dist").is_dir()) {
                    actions.push(Action::Reveal(root.join("dist")));
                }
            });
            ui.menu_button(t("menu.view"), |ui| {
                for (tab, label) in [
                    (Tab::Explorer, t("tab.explorer")),
                    (Tab::Console, t("tab.console")),
                    (Tab::Problems, t("tab.problems")),
                    (Tab::Debug, t("tab.debug")),
                    (Tab::Scene, t("tab.scene")),
                ] {
                    if item(ui, label, "", true) {
                        actions.push(Action::ShowTab(tab));
                    }
                }
                ui.separator();
                if item(ui, t("menu.reset_layout"), "", true) {
                    actions.push(Action::ResetLayout);
                }
            });
            ui.menu_button(t("menu.help"), |ui| {
                let docs = root.join("docs");
                if item(ui, t("menu.api_docs"), "", docs.is_dir()) {
                    actions.push(Action::Reveal(docs.clone()));
                }
                let agents = root.join("AGENTS.md");
                if item(ui, "AGENTS.md", "", agents.is_file()) {
                    actions.push(Action::OpenFile(agents));
                }
                ui.separator();
                ui.label(RichText::new(format!("{} {}", t("app.name"), crate::VERSION)).small().color(theme::TEXT_DIM));
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let state = self.game.state();
        let running = state != GameState::Stopped;
        let live = self.game.is_connected();
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.project.title).strong().color(egui::Color32::WHITE));
            ui.separator();
            if running {
                if ui.button(format!("{} {}", icon::STOP, t("game.stop"))).on_hover_text("Shift+F5").clicked() {
                    actions.push(Action::Stop);
                }
                if ui.button(icon::RELOAD).on_hover_text(t("game.restart")).clicked() {
                    actions.push(Action::Restart);
                }
            } else if ui.add(theme::primary_button(format!("{} {}", icon::RUN, t("game.run")))).on_hover_text("F5").clicked() {
                actions.push(Action::Run);
            }
            let pause = if state == GameState::Paused { icon::RUN } else { icon::PAUSE };
            let pause_hint = if state == GameState::Paused { t("game.resume") } else { t("game.pause") };
            if ui.add_enabled(live, egui::Button::new(pause)).on_hover_text(format!("{pause_hint} (F6)")).clicked() {
                actions.push(Action::TogglePause);
            }
            if ui.add_enabled(live, egui::Button::new(icon::STEP)).on_hover_text(format!("{} (F7)", t("game.step"))).clicked() {
                actions.push(Action::Step);
            }
            ui.separator();
            let checking = self.game.is_checking();
            if ui.add_enabled(!checking, egui::Button::new(format!("{} {}", icon::CHECK, t("game.check")))).on_hover_text(format!("{} (F8)", t("game.check_hint"))).clicked() {
                actions.push(Action::Check);
            }
            let building = self.game.is_building();
            if ui.add_enabled(!building, egui::Button::new(format!("{} {}", icon::BUILD, t("game.build")))).on_hover_text(t("game.build_hint")).clicked() {
                actions.push(Action::Build);
            }
            if checking || building {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(icon::SETTINGS).on_hover_text(t("menu.settings")).clicked() {
                    actions.push(Action::Settings);
                }
                let (color, label) = game::state_label(state);
                let text = match self.game.fps() {
                    Some(fps) if running => format!("{} {label} · {fps:.0} FPS", icon::DOT),
                    _ => format!("{} {label}", icon::DOT),
                };
                if ui.add(egui::Label::new(RichText::new(text).color(color)).sense(egui::Sense::click())).on_hover_text(t("tab.debug")).clicked() {
                    actions.push(Action::ShowTab(Tab::Debug));
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            ui.label(RichText::new(format!("{} {}", icon::FOLDER, self.project.root.display())).small().color(theme::TEXT_DIM));
            let (warnings, errors) = self.console.counts();
            let problems = self.problems.count();
            if problems > 0 {
                ui.label(RichText::new(format!("{} {}", icon::ERROR, tf("status.problems", &[("count", &problems)]))).small().color(theme::ERROR));
            }
            if warnings + errors > 0 {
                ui.label(RichText::new(tf("status.log", &[("warnings", &warnings), ("errors", &errors)])).small().color(theme::TEXT_DIM));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(doc) = self.active_doc.as_deref().and_then(|p| self.docs.get(p)) {
                    ui.label(RichText::new(doc.lang.name()).small().color(theme::TEXT_DIM));
                    ui.label(RichText::new("UTF-8").small().color(theme::TEXT_DIM));
                    ui.label(RichText::new(tf("status.cursor", &[("line", &doc.cursor.0), ("col", &doc.cursor.1)])).small().color(theme::TEXT_DIM));
                    if doc.is_dirty() {
                        ui.label(RichText::new(t("status.unsaved")).small().color(theme::WARN));
                    }
                }
            });
        });
    }

    fn localize_dock(&mut self) {
        let lang = i18n::lang();
        if self.dock_lang == Some(lang) {
            return;
        }
        self.dock_lang = Some(lang);
        let tr = &mut self.dock.translations;
        tr.tab_context_menu.close_button = t("dock.close").into();
        tr.tab_context_menu.eject_button = t("dock.eject").into();
        tr.tab_context_menu.hide_tab_bar_button = t("dock.hide_tab_bar").into();
        tr.tab_context_menu.show_tab_bar_button = t("dock.show_tab_bar").into();
        tr.leaf.close_button_disabled_tooltip = t("dock.cannot_close").into();
        tr.leaf.close_all_button = t("dock.close_all").into();
        tr.leaf.minimize_button = t("dock.minimize").into();
    }

    fn dock_ui(&mut self, ui: &mut egui::Ui, settings: &Settings, actions: &mut Vec<Action>) {
        self.localize_dock();
        let mut style = egui_dock::Style::from_egui(ui.style());
        style.tab_bar.bg_fill = theme::PANEL;
        style.tab_bar.hline_color = theme::BORDER;
        style.separator.width = 1.0;
        style.separator.color_idle = theme::BORDER;
        style.separator.color_hovered = theme::ACCENT_DIM;
        style.separator.color_dragged = theme::ACCENT;
        style.tab.tab_body.bg_fill = theme::BG;
        style.tab.tab_body.stroke = egui::Stroke::NONE;
        style.tab.focused.bg_fill = theme::BG;
        style.tab.active.bg_fill = theme::BG;
        style.tab.inactive.bg_fill = theme::PANEL;
        style.tab.hovered.bg_fill = theme::SURFACE;

        let mut viewer = Viewer {
            root: &self.project.root,
            docs: &mut self.docs,
            explorer: &mut self.explorer,
            console: &mut self.console,
            problems: &mut self.problems,
            game: &mut self.game,
            settings,
            actions,
        };
        DockArea::new(&mut self.dock)
            .style(style)
            .show_leaf_collapse_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_inside(ui, &mut viewer);
        if let Some((_, Tab::Doc(path))) = self.dock.find_active_focused() {
            self.active_doc = Some(path.clone());
        }
        // The active document was closed by its tab button.
        if let Some(p) = &self.active_doc {
            if self.docs.get(p).is_none() {
                self.active_doc = self.dock.iter_all_tabs().find_map(|(_, t)| if let Tab::Doc(p) = t { Some(p.clone()) } else { None });
            }
        }
    }

    // ------------------------------------------------------------------ dialogs

    fn dialogs(&mut self, ctx: &egui::Context, settings: &mut Settings, toasts: &mut Toasts, actions: &mut Vec<Action>) -> Option<AppAction> {
        let is_file = matches!(self.dialog, Some(Dialog::NewFile { .. }));
        let dialog = self.dialog.as_mut()?;
        let mut close = false;
        let mut result = None;
        let focus = std::mem::take(&mut self.focus_dialog);
        let modal = egui::Modal::new(egui::Id::new("workspace-dialog")).show(ctx, |ui| {
            ui.set_width(440.0);
            match dialog {
                Dialog::NewFile { dir, name, error } | Dialog::NewFolder { dir, name, error } => {
                    ui.heading(if is_file { t("dialog.new_file") } else { t("dialog.new_folder") });
                    let rel = dir.strip_prefix(&self.project.root).unwrap_or(dir);
                    ui.label(RichText::new(tf("dialog.in_folder", &[("path", &format!("/{}", rel.display()))])).small().color(theme::TEXT_DIM));
                    ui.add_space(6.0);
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                    if focus {
                        r.request_focus();
                    }
                    if let Some(e) = error {
                        ui.colored_label(theme::ERROR, e.as_str());
                    }
                    ui.add_space(8.0);
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let (ok, cancel) = buttons(ui, t("dialog.create"));
                    if ok || enter {
                        let target = dir.join(name.trim());
                        match create_entry(&target, name.trim(), is_file) {
                            Ok(()) => {
                                self.explorer.invalidate();
                                self.explorer.reveal(&target);
                                if is_file {
                                    actions.push(Action::OpenFile(target));
                                }
                                close = true;
                            }
                            Err(e) => *error = Some(e),
                        }
                    }
                    close |= cancel;
                }
                Dialog::Rename { path, name, error } => {
                    ui.heading(t("dialog.rename"));
                    ui.add_space(6.0);
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                    if focus {
                        r.request_focus();
                    }
                    if let Some(e) = error {
                        ui.colored_label(theme::ERROR, e.as_str());
                    }
                    ui.add_space(8.0);
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let (ok, cancel) = buttons(ui, t("dialog.rename_ok"));
                    if ok || enter {
                        let target = path.with_file_name(name.trim());
                        if !valid_file_name(name.trim()) {
                            *error = Some(t("dialog.bad_name").into());
                        } else if target == *path {
                            close = true;
                        } else if target.exists() {
                            *error = Some(tf("dialog.exists", &[("name", &name.trim())]));
                        } else {
                            match std::fs::rename(&*path, &target) {
                                Ok(()) => {
                                    self.docs.rename(path, &target);
                                    for (_, tab) in self.dock.iter_all_tabs_mut() {
                                        if let Tab::Doc(p) = tab {
                                            *p = moved(p, path, &target);
                                        }
                                    }
                                    if let Some(a) = &mut self.active_doc {
                                        *a = moved(a, path, &target);
                                    }
                                    self.explorer.invalidate();
                                    self.explorer.reveal(&target);
                                    close = true;
                                }
                                Err(e) => *error = Some(e.to_string()),
                            }
                        }
                    }
                    close |= cancel;
                }
                Dialog::Delete { path } => {
                    ui.heading(t("dialog.delete"));
                    let rel = path.strip_prefix(&self.project.root).unwrap_or(path);
                    ui.label(tf("dialog.delete_body", &[("name", &rel.display())]));
                    ui.add_space(8.0);
                    let (ok, cancel) = buttons(ui, t("dialog.delete_ok"));
                    if ok {
                        match trash::delete(&*path) {
                            Ok(()) => {
                                let gone: Vec<PathBuf> = self.docs.iter().map(|d| d.path.clone()).filter(|p| p.starts_with(&*path)).collect();
                                for p in gone {
                                    actions.push(Action::ForceClose(p));
                                }
                                self.explorer.invalidate();
                                toasts.info(tf("dialog.deleted", &[("name", &rel.display())]));
                            }
                            Err(e) => toasts.error(tf("dialog.delete_failed", &[("error", &e)])),
                        }
                        close = true;
                    }
                    close |= cancel;
                }
                Dialog::CloseUnsaved { path } => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.heading(t("dialog.unsaved_title"));
                    ui.label(tf("dialog.unsaved_body", &[("name", &name)]));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_button(t("dialog.save"))).clicked() {
                            actions.push(Action::Save(path.clone()));
                            actions.push(Action::ForceClose(path.clone()));
                            close = true;
                        }
                        if ui.button(t("dialog.discard")).clicked() {
                            actions.push(Action::ForceClose(path.clone()));
                            close = true;
                        }
                        if ui.button(t("common.cancel")).clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::CloseProject => {
                    ui.heading(t("app.unsaved_title"));
                    ui.label(t("dialog.close_project_body"));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_button(t("dialog.save_all"))).clicked() {
                            let mut ok = true;
                            for doc in self.docs.iter_mut().filter(|d| d.is_dirty()) {
                                if let Err(e) = doc.save() {
                                    toasts.error(e);
                                    ok = false;
                                }
                            }
                            if ok {
                                result = Some(AppAction::CloseProject);
                            }
                            close = true;
                        }
                        if ui.button(t("dialog.discard")).clicked() {
                            result = Some(AppAction::CloseProject);
                            close = true;
                        }
                        if ui.button(t("common.cancel")).clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Settings => {
                    egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.75).show(ui, |ui| {
                        if crate::welcome::settings_page(ui, settings) {
                            result = Some(AppAction::SettingsChanged);
                        }
                    });
                    ui.add_space(8.0);
                    if ui.add(theme::primary_button(t("dialog.done"))).clicked() {
                        close = true;
                    }
                }
            }
        });
        if modal.should_close() || close {
            self.dialog = None;
        }
        result
    }

    fn open_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
        self.focus_dialog = true;
    }

    // ------------------------------------------------------------------ actions

    fn handle(&mut self, action: Action, settings: &mut Settings, toasts: &mut Toasts) -> Option<AppAction> {
        match action {
            Action::OpenFile(path) => self.open_file(&path, None, toasts),
            Action::OpenAt(path, line) => {
                let path = self.resolve(&path);
                self.explorer.reveal(&path);
                self.open_file(&path, Some(line), toasts);
            }
            Action::Save(path) => self.save(&path, settings, toasts),
            Action::SaveActive => {
                if let Some(p) = self.active_doc.clone() {
                    self.save(&p, settings, toasts);
                }
            }
            Action::SaveAll => {
                let dirty: Vec<PathBuf> = self.docs.iter().filter(|d| d.is_dirty()).map(|d| d.path.clone()).collect();
                for p in dirty {
                    self.save(&p, settings, toasts);
                }
            }
            Action::CloseActive => {
                if let Some(p) = self.active_doc.clone() {
                    if self.docs.get(&p).is_some_and(|d| d.is_dirty()) {
                        self.open_dialog(Dialog::CloseUnsaved { path: p });
                    } else {
                        self.close_doc(&p);
                    }
                }
            }
            Action::ForceClose(path) => self.close_doc(&path),
            Action::NewFile(dir) => self.open_dialog(Dialog::NewFile { dir, name: "new.luau".into(), error: None }),
            Action::NewFolder(dir) => self.open_dialog(Dialog::NewFolder { dir, name: String::new(), error: None }),
            Action::Rename(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.open_dialog(Dialog::Rename { path, name, error: None });
            }
            Action::Delete(path) => {
                if path != self.project.root {
                    self.open_dialog(Dialog::Delete { path });
                }
            }
            Action::Reveal(path) => crate::os::reveal(&path),
            Action::OpenExternal(path) => crate::os::open_path(&path),
            Action::Run | Action::Restart => self.run_game(settings, toasts),
            Action::Stop => self.game.stop(),
            Action::TogglePause => self.game.toggle_pause(),
            Action::Step => self.game.step(),
            Action::Reload => {
                if settings.save_before_run {
                    self.save_all(toasts);
                }
                self.game.reload();
            }
            Action::Check => self.check(settings, toasts, true),
            Action::Build => {
                if let Some(spark) = self.spark(settings, toasts) {
                    self.save_all(toasts);
                    match self.game.build(&spark, &self.clock) {
                        Ok(line) => self.console.push(line),
                        Err(e) => toasts.error(e),
                    }
                    self.show_tab(Tab::Console);
                }
            }
            Action::ShowTab(tab) => self.show_tab(tab),
            Action::ResetLayout => {
                let open: Vec<PathBuf> = self.dock.iter_all_tabs().filter_map(|(_, t)| if let Tab::Doc(p) = t { Some(p.clone()) } else { None }).collect();
                self.dock = default_layout();
                self.dock_lang = None;
                for p in open {
                    self.add_doc_tab(p);
                }
            }
            Action::Settings => self.open_dialog(Dialog::Settings),
            Action::CloseProject => {
                if self.has_unsaved() {
                    self.open_dialog(Dialog::CloseProject);
                } else {
                    return Some(AppAction::CloseProject);
                }
            }
            Action::Quit => return Some(AppAction::Quit),
        }
        None
    }

    /// `file` from a message (usually relative to the game) -> a path in the project.
    fn resolve(&self, file: &Path) -> PathBuf {
        if file.is_absolute() {
            return file.to_path_buf();
        }
        let direct = self.project.root.join(file);
        if direct.exists() {
            return direct;
        }
        // `require` paths are sometimes reported without `src/`.
        let in_src = self.project.root.join("src").join(file);
        if in_src.exists() { in_src } else { direct }
    }

    fn open_file(&mut self, path: &Path, line: Option<u32>, toasts: &mut Toasts) {
        if !explorer::is_text_file(path) {
            crate::os::open_path(path);
            return;
        }
        match self.docs.open(path) {
            Ok(doc) => {
                if line.is_some() {
                    doc.goto = line;
                }
            }
            Err(e) => {
                toasts.error(format!("{}: {e}", path.display()));
                return;
            }
        }
        self.add_doc_tab(path.to_path_buf());
        self.active_doc = Some(path.to_path_buf());
    }

    /// Focuses the document's tab, or adds it next to the other code tabs.
    fn add_doc_tab(&mut self, path: PathBuf) {
        let tab = Tab::Doc(path);
        if let Some(at) = self.dock.find_tab(&tab) {
            let _ = self.dock.set_active_tab(at);
            self.dock.set_focused_node_and_surface(at.node_path());
            return;
        }
        let code_leaf = self
            .dock
            .find_tab_from(|t| matches!(t, Tab::Doc(_)))
            .or_else(|| self.dock.find_tab(&Tab::Start))
            .map(|at| at.node_path());
        match code_leaf {
            Some(node) => {
                if let Ok(leaf) = self.dock.leaf_mut(node) {
                    leaf.append_tab(tab);
                }
                self.dock.set_focused_node_and_surface(node);
            }
            None => self.dock.push_to_focused_leaf(tab),
        }
    }

    fn close_doc(&mut self, path: &Path) {
        if let Some(at) = self.dock.find_tab(&Tab::Doc(path.to_path_buf())) {
            self.dock.remove_tab(at);
        }
        self.docs.close(path);
        if self.active_doc.as_deref() == Some(path) {
            self.active_doc = None;
        }
    }

    fn show_tab(&mut self, tab: Tab) {
        match self.dock.find_tab(&tab) {
            Some(at) => {
                let _ = self.dock.set_active_tab(at);
            }
            None => match &tab {
                Tab::Console | Tab::Problems => {
                    let near = self.dock.find_tab(&Tab::Console).or_else(|| self.dock.find_tab(&Tab::Problems));
                    self.push_near(near, tab);
                }
                Tab::Debug | Tab::Scene => {
                    let near = self.dock.find_tab(&Tab::Debug).or_else(|| self.dock.find_tab(&Tab::Scene));
                    self.push_near(near, tab);
                }
                _ => self.dock.main_surface_mut().push_to_first_leaf(tab),
            },
        }
    }

    fn push_near(&mut self, near: Option<egui_dock::TabPath>, tab: Tab) {
        match near.and_then(|at| self.dock.leaf_mut(at.node_path()).ok()) {
            Some(leaf) => leaf.append_tab(tab),
            None => self.dock.main_surface_mut().push_to_first_leaf(tab),
        }
    }

    fn save(&mut self, path: &Path, settings: &Settings, toasts: &mut Toasts) {
        let Some(doc) = self.docs.get_mut(path) else { return };
        match doc.save() {
            Ok(()) => {
                let script = matches!(doc.lang, crate::syntax::Lang::Luau);
                if script && settings.check_on_save && !self.game.is_running() {
                    self.check(settings, toasts, false);
                }
            }
            Err(e) => toasts.error(e),
        }
    }

    fn spark(&self, settings: &Settings, toasts: &mut Toasts) -> Option<PathBuf> {
        let found = process::find_spark(settings.spark_path.as_deref());
        if found.is_none() {
            toasts.error(t("game.no_spark"));
        }
        found
    }

    fn run_game(&mut self, settings: &Settings, toasts: &mut Toasts) {
        let Some(spark) = self.spark(settings, toasts) else { return };
        if settings.save_before_run {
            self.save_all(toasts);
        }
        if self.console.clear_on_run && !self.game.is_running() {
            self.console.clear();
        }
        self.problems.set_runtime(None);
        match self.game.run(&spark, &self.clock) {
            Ok(lines) => self.console.extend(lines),
            Err(e) => {
                self.console.push(editor_line(JobKind::Run, Level::Error, e.clone(), &self.clock));
                toasts.error(e);
            }
        }
    }

    /// `spark check`. `loud`: show the console / errors even when started automatically.
    fn check(&mut self, settings: &Settings, toasts: &mut Toasts, loud: bool) {
        if self.game.is_checking() {
            return;
        }
        let Some(spark) = self.spark(settings, toasts) else { return };
        if loud {
            self.save_all(toasts);
            self.show_tab(Tab::Problems);
        }
        match self.game.check(&spark, &self.clock) {
            Ok(line) => {
                self.problems.checking = true;
                self.console.push(line);
            }
            Err(e) => toasts.error(e),
        }
    }
}

/// Draws the tabs; borrows the parts of the workspace the panels need.
struct Viewer<'a> {
    root: &'a Path,
    docs: &'a mut Documents,
    explorer: &'a mut Explorer,
    console: &'a mut Console,
    problems: &'a mut Problems,
    game: &'a mut GameSession,
    settings: &'a Settings,
    actions: &'a mut Vec<Action>,
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(("ws-tab", &*tab))
    }

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::Start => format!("{} {}", icon::INFO, t("tab.start")).into(),
            Tab::Explorer => format!("{} {}", icon::FOLDER, t("tab.explorer")).into(),
            Tab::Doc(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match self.docs.get(path) {
                    Some(d) if d.is_dirty() => RichText::new(format!("{name} {}", icon::DOT)).into(),
                    _ => name.into(),
                }
            }
            Tab::Console => {
                let (_, errors) = self.console.counts();
                if errors > 0 {
                    RichText::new(format!("{} {} ({errors})", icon::CONSOLE, t("tab.console"))).color(theme::ERROR).into()
                } else {
                    format!("{} {}", icon::CONSOLE, t("tab.console")).into()
                }
            }
            Tab::Problems => {
                let n = self.problems.count();
                if n > 0 {
                    RichText::new(format!("{} {} ({n})", icon::WARN, t("tab.problems"))).color(theme::ERROR).into()
                } else {
                    format!("{} {}", icon::WARN, t("tab.problems")).into()
                }
            }
            Tab::Debug => format!("{} {}", icon::BUG, t("tab.debug")).into(),
            Tab::Scene => format!("{} {}", icon::SCENE, t("tab.scene")).into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Start => start_page(ui, self.actions),
            Tab::Explorer => self.explorer.ui(ui, self.settings.show_hidden_files, self.actions),
            Tab::Doc(path) => match self.docs.get_mut(path) {
                Some(doc) => {
                    if let EditorEvent::Save = documents::editor_ui(ui, doc, self.settings.editor_font_size) {
                        self.actions.push(Action::Save(path.clone()));
                    }
                }
                None => {
                    ui.label(RichText::new(t("editor.closed")).color(theme::TEXT_DIM));
                }
            },
            Tab::Console => self.console.ui(ui, self.actions),
            Tab::Problems => self.problems.ui(ui, self.actions),
            Tab::Debug => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.game.debug_ui(ui, self.actions));
            }
            Tab::Scene => self.game.scene_ui(ui),
        }
    }

    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        if let Tab::Doc(path) = tab {
            let rel = path.strip_prefix(self.root).unwrap_or(path);
            response.clone().on_hover_text(rel.display().to_string());
            if response.middle_clicked() {
                self.actions.push(Action::CloseActive);
            }
        }
    }

    fn on_close(&mut self, tab: &mut Tab) -> OnCloseResponse {
        match tab {
            Tab::Doc(path) => {
                if self.docs.get(path).is_some_and(|d| d.is_dirty()) {
                    // Ask first; the dialog closes the tab (`ForceClose`).
                    self.actions.push(Action::OpenFile(path.clone()));
                    self.actions.push(Action::CloseActive);
                    OnCloseResponse::Focus
                } else {
                    self.docs.close(path);
                    OnCloseResponse::Close
                }
            }
            _ => OnCloseResponse::Close,
        }
    }

    fn is_closeable(&self, tab: &Tab) -> bool {
        !matches!(tab, Tab::Start)
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        // Every panel scrolls itself (the code editor needs its own scroll area for go-to-line).
        [false, false]
    }

    fn allowed_in_windows(&self, tab: &mut Tab) -> bool {
        !matches!(tab, Tab::Start)
    }
}

/// The code area when no file is open: what to do next.
fn start_page(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    ui.add_space(ui.available_height() * 0.18);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(t("start.title")).size(22.0).color(egui::Color32::WHITE));
        ui.add_space(6.0);
        ui.label(RichText::new(t("start.body")).color(theme::TEXT_DIM));
        ui.add_space(16.0);
        if ui.add(theme::primary_button(format!("{} {}", icon::RUN, t("game.run")))).clicked() {
            actions.push(Action::Run);
        }
        ui.add_space(20.0);
        egui::Grid::new("start-shortcuts").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
            for (keys, what) in [
                ("F5", t("game.run")),
                ("Shift+F5", t("game.stop")),
                ("F6", t("game.pause")),
                ("F7", t("game.step")),
                ("F8", t("game.check")),
                ("Ctrl+R", t("game.reload")),
                ("Ctrl+S", t("menu.save")),
                ("Ctrl+W", t("menu.close_tab")),
            ] {
                ui.label(RichText::new(keys).monospace().color(theme::ACCENT));
                ui.label(RichText::new(what).color(theme::TEXT_DIM));
                ui.end_row();
            }
        });
    });
}

/// OK / Cancel row.
fn buttons(ui: &mut egui::Ui, ok: &str) -> (bool, bool) {
    let mut out = (false, false);
    ui.horizontal(|ui| {
        out.0 = ui.add(theme::primary_button(ok)).clicked();
        out.1 = ui.button(t("common.cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape));
    });
    out
}

fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
}

fn create_entry(target: &Path, name: &str, file: bool) -> Result<(), String> {
    if !valid_file_name(name) {
        return Err(t("dialog.bad_name").into());
    }
    if target.exists() {
        return Err(tf("dialog.exists", &[("name", &name)]));
    }
    if file {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(target, "").map_err(|e| e.to_string())
    } else {
        std::fs::create_dir_all(target).map_err(|e| e.to_string())
    }
}

/// `p` after `from` was renamed to `to` (unchanged when `p` is not inside `from`).
fn moved(p: &Path, from: &Path, to: &Path) -> PathBuf {
    match p.strip_prefix(from) {
        Ok(rest) if rest.as_os_str().is_empty() => to.to_path_buf(),
        Ok(rest) => to.join(rest),
        Err(_) => p.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert!(valid_file_name("player.luau"));
        assert!(!valid_file_name("a/b"));
        assert!(!valid_file_name(".."));
        assert!(!valid_file_name(""));
    }

    #[test]
    fn moved_paths() {
        let (from, to) = (Path::new("/g/src"), Path::new("/g/scripts"));
        assert_eq!(moved(Path::new("/g/src/a.luau"), from, to), PathBuf::from("/g/scripts/a.luau"));
        assert_eq!(moved(from, from, to), to.to_path_buf());
        assert_eq!(moved(Path::new("/g/main.luau"), from, to), PathBuf::from("/g/main.luau"));
    }

    #[test]
    fn default_layout_has_every_panel() {
        let dock = default_layout();
        for tab in [Tab::Start, Tab::Explorer, Tab::Console, Tab::Problems, Tab::Debug, Tab::Scene] {
            assert!(dock.find_tab(&tab).is_some(), "{tab:?}");
        }
    }
}
