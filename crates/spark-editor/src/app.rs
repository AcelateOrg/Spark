//! The editor window: switches between the welcome screen and an open project, owns the settings.

use std::path::{Path, PathBuf};

use eframe::egui;
use spark_project::NotAProject;

use crate::i18n::{self, t, tf};
use crate::settings::Settings;
use crate::theme;
use crate::toasts::Toasts;
use crate::welcome::WelcomeScreen;
use crate::workspace::Workspace;

/// What a screen asks the app to do.
#[derive(Debug)]
pub enum AppAction {
    OpenProject(PathBuf),
    /// New game from the template in `dir` (created), then open it.
    CreateProject { dir: PathBuf, title: String },
    CloseProject,
    /// Language / scale / ... changed: apply and save.
    SettingsChanged,
    Quit,
}

enum Screen {
    Welcome(WelcomeScreen),
    Workspace(Box<Workspace>),
}

pub struct EditorApp {
    settings: Settings,
    screen: Screen,
    toasts: Toasts,
    applied_scale: f32,
    window_title: String,
    /// The window close button was pressed with unsaved files: ask first.
    confirm_quit: bool,
    quit_confirmed: bool,
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, target: Option<PathBuf>) -> Self {
        let settings = Settings::load();
        i18n::set_lang(settings.language());
        theme::apply(&cc.egui_ctx, settings.ui_scale);
        let mut app = Self {
            screen: Screen::Welcome(WelcomeScreen::new(&settings)),
            applied_scale: settings.ui_scale,
            settings,
            toasts: Toasts::default(),
            window_title: String::new(),
            confirm_quit: false,
            quit_confirmed: false,
        };
        if let Some(path) = target {
            app.open_project(&path);
        }
        app
    }

    fn open_project(&mut self, path: &Path) {
        let root = spark_project::find_root(path);
        match spark_project::detect(&root) {
            Ok(project) => {
                self.close_project();
                for w in &project.warnings {
                    self.toasts.warning(w.clone());
                }
                self.settings.touch_recent(&project.root, &project.title);
                self.settings.save();
                let ws = Workspace::open(project, &self.settings, &mut self.toasts);
                self.screen = Screen::Workspace(Box::new(ws));
            }
            Err(e) => {
                self.toasts.error(describe(&e));
                if let Screen::Welcome(w) = &mut self.screen {
                    w.refresh(&self.settings);
                }
            }
        }
    }

    fn create_project(&mut self, dir: PathBuf, title: String) {
        match spark_project::create(&dir, &title) {
            Ok(dir) => {
                if let Some(parent) = dir.parent() {
                    self.settings.projects_dir = Some(parent.to_path_buf());
                }
                self.toasts.success(tf("welcome.created", &[("name", &title)]));
                self.open_project(&dir);
            }
            Err(e) => self.toasts.error(tf("welcome.create_failed", &[("error", &e)])),
        }
    }

    /// Remembers the open files and leaves the project (the game is stopped by `Workspace::drop`).
    fn close_project(&mut self) {
        if let Screen::Workspace(ws) = &mut self.screen {
            ws.remember(&mut self.settings);
            self.settings.save();
        }
        self.screen = Screen::Welcome(WelcomeScreen::new(&self.settings));
    }

    fn apply_settings(&mut self, ctx: &egui::Context) {
        i18n::set_lang(self.settings.language());
        if self.settings.ui_scale != self.applied_scale {
            ctx.set_zoom_factor(self.settings.ui_scale);
            self.applied_scale = self.settings.ui_scale;
        }
        self.settings.save();
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match &self.screen {
            Screen::Welcome(_) => t("app.welcome_title").to_string(),
            Screen::Workspace(ws) => format!("{} — {}", ws.title(), t("app.name")),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.quit_confirmed {
            return;
        }
        if let Screen::Workspace(ws) = &self.screen {
            if ws.has_unsaved() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm_quit = true;
            }
        }
    }

    fn quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_quit {
            return;
        }
        let mut choice = None;
        egui::Modal::new(egui::Id::new("confirm-quit")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(t("app.unsaved_title"));
            ui.label(t("app.unsaved_body"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button(t("app.save_and_quit"))).clicked() {
                    choice = Some(true);
                }
                if ui.button(t("app.quit_without_saving")).clicked() {
                    choice = Some(false);
                }
                if ui.button(t("common.cancel")).clicked() {
                    self.confirm_quit = false;
                }
            });
        });
        if let Some(save) = choice {
            if let Screen::Workspace(ws) = &mut self.screen {
                if save {
                    ws.save_all(&mut self.toasts);
                }
            }
            self.confirm_quit = false;
            self.quit_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let action = match &mut self.screen {
            Screen::Welcome(w) => w.ui(ui, &mut self.settings),
            Screen::Workspace(ws) => ws.ui(ui, &mut self.settings, &mut self.toasts),
        };
        match action {
            Some(AppAction::OpenProject(p)) => self.open_project(&p),
            Some(AppAction::CreateProject { dir, title }) => self.create_project(dir, title),
            Some(AppAction::CloseProject) => self.close_project(),
            Some(AppAction::SettingsChanged) => self.apply_settings(&ctx),
            Some(AppAction::Quit) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            None => {}
        }
        self.handle_close_request(&ctx);
        self.quit_dialog(&ctx);
        self.update_title(&ctx);
        self.toasts.show(&ctx);
    }

    fn on_exit(&mut self) {
        if let Screen::Workspace(ws) = &mut self.screen {
            ws.remember(&mut self.settings);
        }
        self.settings.save();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }
}

/// "Not a Spark game" in the user's language.
pub fn describe(e: &NotAProject) -> String {
    match e {
        NotAProject::Missing(p) => tf("project.missing", &[("path", &p.display())]),
        NotAProject::NotADirectory(p) => tf("project.not_a_folder", &[("path", &p.display())]),
        NotAProject::NoEntry(p) => tf("project.no_entry", &[("path", &p.display())]),
        NotAProject::BadManifest(err) => tf("project.bad_manifest", &[("error", err)]),
        NotAProject::NotSpark(p) => tf("project.not_spark", &[("path", &p.display())]),
    }
}
