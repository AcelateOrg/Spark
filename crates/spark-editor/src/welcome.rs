//! The welcome screen (`spark editor` without a project): recent projects, new game, open folder,
//! editor settings. Laid out like the JetBrains project manager.

use std::path::PathBuf;

use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, Margin, RichText, Sense, Stroke, vec2};

use crate::app::AppAction;
use crate::i18n::{Lang, t, tf};
use crate::process;
use crate::settings::{Settings, now_unix};
use crate::theme::{self, icon};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Projects,
    Settings,
    About,
}

struct RecentRow {
    path: PathBuf,
    title: String,
    exists: bool,
    last_opened: u64,
}

struct NewProject {
    name: String,
    location: String,
    error: Option<String>,
    /// Focus the name field once when the dialog opens.
    focus: bool,
}

pub struct WelcomeScreen {
    page: Page,
    search: String,
    rows: Vec<RecentRow>,
    new_project: Option<NewProject>,
}

impl WelcomeScreen {
    pub fn new(settings: &Settings) -> Self {
        let mut s = Self { page: Page::Projects, search: String::new(), rows: Vec::new(), new_project: None };
        s.refresh(settings);
        s
    }

    /// Re-reads the recent list (and which projects still exist).
    pub fn refresh(&mut self, settings: &Settings) {
        self.rows = settings
            .recent
            .iter()
            .map(|r| RecentRow { path: r.path.clone(), title: r.title.clone(), exists: r.path.join(spark_project::ENTRY).is_file(), last_opened: r.last_opened })
            .collect();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) -> Option<AppAction> {
        let mut action = None;
        // A folder dropped on the window opens it.
        if let Some(path) = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).next()) {
            action = Some(AppAction::OpenProject(path));
        }

        egui::Panel::left("welcome-nav")
            .exact_size(232.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::SURFACE).inner_margin(Margin::same(16)))
            .show(ui, |ui| self.sidebar(ui, settings, &mut action));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(28, 22)))
            .show(ui, |ui| match self.page {
                Page::Projects => self.projects(ui, settings, &mut action),
                Page::Settings => {
                    if settings_page(ui, settings) {
                        action = Some(AppAction::SettingsChanged);
                    }
                }
                Page::About => about_page(ui),
            });

        if let Some(a) = self.new_project_dialog(ui.ctx(), settings) {
            action = Some(a);
        }
        if matches!(action, Some(AppAction::SettingsChanged)) {
            self.refresh(settings);
        }
        action
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, settings: &mut Settings, action: &mut Option<AppAction>) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
            ui.painter().rect_filled(rect, 8.0, theme::ACCENT);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "S", FontId::proportional(20.0), Color32::WHITE);
            ui.vertical(|ui| {
                ui.label(RichText::new(t("app.name")).strong().size(15.0).color(Color32::WHITE));
                ui.label(RichText::new(crate::VERSION).small().color(theme::TEXT_DIM));
            });
        });
        ui.add_space(18.0);
        for (page, label) in [(Page::Projects, t("welcome.projects")), (Page::Settings, t("welcome.customize")), (Page::About, t("welcome.about"))] {
            let selected = self.page == page;
            let text = if selected { RichText::new(label).color(Color32::WHITE).strong() } else { RichText::new(label) };
            if ui.add_sized([ui.available_width(), 30.0], egui::Button::selectable(selected, text)).clicked() {
                self.page = page;
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            if language_picker(ui, settings) {
                *action = Some(AppAction::SettingsChanged);
            }
        });
    }

    fn projects(&mut self, ui: &mut egui::Ui, settings: &mut Settings, action: &mut Option<AppAction>) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(format!("{}  {}", icon::SEARCH, t("welcome.search"))).desired_width(280.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(t("welcome.open")).on_hover_text(t("welcome.open_hint")).clicked() {
                    if let Some(dir) = rfd::FileDialog::new().set_title(t("welcome.open_title")).set_directory(settings.projects_dir()).pick_folder() {
                        *action = Some(AppAction::OpenProject(dir));
                    }
                }
                if ui.add(theme::primary_button(t("welcome.new_project"))).clicked() {
                    self.new_project = Some(NewProject { name: unique_name(settings), location: settings.projects_dir().display().to_string(), error: None, focus: true });
                }
            });
        });
        ui.add_space(14.0);

        if self.rows.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(t("welcome.empty_title")).size(20.0).color(Color32::WHITE));
                ui.add_space(6.0);
                ui.label(RichText::new(t("welcome.empty_body")).color(theme::TEXT_DIM));
            });
            return;
        }

        let query = self.search.to_lowercase();
        let mut remove = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, row) in self.rows.iter().enumerate() {
                if !query.is_empty() && !row.title.to_lowercase().contains(&query) && !row.path.to_string_lossy().to_lowercase().contains(&query) {
                    continue;
                }
                match recent_row(ui, row) {
                    RowAction::None => {}
                    RowAction::Open => *action = Some(AppAction::OpenProject(row.path.clone())),
                    RowAction::Remove => remove = Some(i),
                    RowAction::Reveal => crate::os::reveal(&row.path),
                }
            }
        });
        if let Some(i) = remove {
            let row = self.rows.remove(i);
            settings.remove_recent(&row.path);
            settings.save();
        }
    }

    fn new_project_dialog(&mut self, ctx: &egui::Context, settings: &Settings) -> Option<AppAction> {
        let np = self.new_project.as_mut()?;
        let mut result = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("new-project")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(t("welcome.new_project"));
            ui.add_space(10.0);
            egui::Grid::new("new-project-grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                ui.label(t("welcome.name"));
                let r = ui.add(egui::TextEdit::singleline(&mut np.name).desired_width(360.0));
                if r.changed() {
                    np.error = None;
                }
                if std::mem::take(&mut np.focus) {
                    r.request_focus();
                }
                ui.end_row();
                ui.label(t("welcome.location"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut np.location).desired_width(270.0));
                    if ui.button(t("common.browse")).clicked() {
                        if let Some(dir) = rfd::FileDialog::new().set_directory(&np.location).pick_folder() {
                            np.location = dir.display().to_string();
                        }
                    }
                });
                ui.end_row();
            });
            let target = PathBuf::from(&np.location).join(np.name.trim());
            ui.add_space(6.0);
            ui.label(RichText::new(tf("welcome.will_create", &[("path", &target.display())])).small().color(theme::TEXT_DIM));
            if let Some(e) = &np.error {
                ui.colored_label(theme::ERROR, e);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.add(theme::primary_button(t("welcome.create"))).clicked() || enter {
                    if !spark_project::valid_name(&np.name) {
                        np.error = Some(t("welcome.bad_name").into());
                    } else if target.exists() && std::fs::read_dir(&target).map(|mut d| d.next().is_some()).unwrap_or(true) {
                        np.error = Some(tf("welcome.exists", &[("path", &target.display())]));
                    } else {
                        result = Some(AppAction::CreateProject { dir: target.clone(), title: np.name.trim().to_string() });
                    }
                }
                if ui.button(t("common.cancel")).clicked() {
                    close = true;
                }
            });
        });
        if modal.should_close() || close || result.is_some() {
            self.new_project = None;
        }
        let _ = settings;
        result
    }
}

enum RowAction {
    None,
    Open,
    Remove,
    Reveal,
}

fn recent_row(ui: &mut egui::Ui, row: &RecentRow) -> RowAction {
    let mut out = RowAction::None;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
    let painter = ui.painter_at(rect);
    if resp.hovered() {
        painter.rect_filled(rect, 6.0, theme::PANEL);
    }
    let avatar = egui::Rect::from_min_size(rect.min + vec2(8.0, 9.0), vec2(36.0, 36.0));
    let color = if row.exists { theme::avatar_color(&row.title) } else { Color32::from_gray(70) };
    painter.rect_filled(avatar, 8.0, color);
    painter.text(avatar.center(), Align2::CENTER_CENTER, theme::initials(&row.title), FontId::proportional(14.0), Color32::WHITE);
    let x = avatar.right() + 12.0;
    painter.text(egui::pos2(x, rect.top() + 9.0), Align2::LEFT_TOP, &row.title, FontId::proportional(15.0), if row.exists { Color32::WHITE } else { theme::TEXT_DIM });
    let sub = if row.exists { row.path.display().to_string() } else { format!("{}  —  {}", row.path.display(), t("welcome.missing")) };
    painter.text(egui::pos2(x, rect.top() + 30.0), Align2::LEFT_TOP, sub, FontId::proportional(12.0), if row.exists { theme::TEXT_DIM } else { theme::ERROR });
    if row.last_opened > 0 {
        painter.text(
            egui::pos2(rect.right() - 44.0, rect.center().y),
            Align2::RIGHT_CENTER,
            ago(row.last_opened),
            FontId::proportional(12.0),
            theme::TEXT_DIM,
        );
    }
    // "⋯" menu on the right, the same menu on right click.
    let menu_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 20.0, rect.center().y), vec2(26.0, 26.0));
    let menu = ui.put(menu_rect, egui::Button::new(icon::MENU).frame(resp.hovered()));
    let menu_ui = |ui: &mut egui::Ui, out: &mut RowAction| {
        if ui.button(t("welcome.open_project")).clicked() {
            *out = RowAction::Open;
        }
        if ui.button(t("welcome.reveal")).clicked() {
            *out = RowAction::Reveal;
        }
        ui.separator();
        if ui.button(t("welcome.remove_recent")).clicked() {
            *out = RowAction::Remove;
        }
    };
    egui::Popup::menu(&menu).show(|ui| menu_ui(ui, &mut out));
    resp.context_menu(|ui| menu_ui(ui, &mut out));
    if resp.clicked() && row.exists {
        out = RowAction::Open;
    }
    resp.on_hover_text(row.path.display().to_string());
    out
}

/// "5 min ago", "yesterday" ...
fn ago(unix: u64) -> String {
    let s = now_unix().saturating_sub(unix);
    match s {
        0..60 => t("time.just_now").into(),
        60..3600 => tf("time.minutes", &[("n", &(s / 60))]),
        3600..86400 => tf("time.hours", &[("n", &(s / 3600))]),
        86400..172800 => t("time.yesterday").into(),
        _ => tf("time.days", &[("n", &(s / 86400))]),
    }
}

fn unique_name(settings: &Settings) -> String {
    let base = t("welcome.default_name");
    let dir = settings.projects_dir();
    (1..100).map(|i| if i == 1 { base.to_string() } else { format!("{base} {i}") }).find(|n| !dir.join(n).exists()).unwrap_or_else(|| base.into())
}

/// Language combo box. Returns true when changed.
pub fn language_picker(ui: &mut egui::Ui, settings: &mut Settings) -> bool {
    let before = settings.language;
    let current = match settings.language {
        None => format!("{} ({})", t("settings.system"), Lang::system().native_name()),
        Some(l) => l.native_name().to_string(),
    };
    egui::ComboBox::from_id_salt("language").selected_text(format!("🌐 {current}")).width(ui.available_width().min(200.0)).show_ui(ui, |ui| {
        ui.selectable_value(&mut settings.language, None, t("settings.system"));
        for l in Lang::ALL {
            ui.selectable_value(&mut settings.language, Some(l), l.native_name());
        }
    });
    before != settings.language
}

/// Editor settings (welcome screen "Customize" and the workspace settings window). True when changed.
pub fn settings_page(ui: &mut egui::Ui, settings: &mut Settings) -> bool {
    let before = settings.clone();
    ui.heading(t("settings.title"));
    ui.add_space(12.0);
    egui::Grid::new("settings-grid").num_columns(2).spacing([24.0, 12.0]).show(ui, |ui| {
        ui.label(t("settings.language"));
        language_picker(ui, settings);
        ui.end_row();

        ui.label(t("settings.ui_scale"));
        ui.add(egui::Slider::new(&mut settings.ui_scale, 0.75..=2.0).step_by(0.05).suffix("×"));
        ui.end_row();

        ui.label(t("settings.font_size"));
        ui.add(egui::Slider::new(&mut settings.editor_font_size, 9.0..=28.0).step_by(1.0).suffix(" px"));
        ui.end_row();

        ui.label(t("settings.spark_path"));
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let mut text = settings.spark_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text(t("settings.auto")).desired_width(320.0));
                if r.changed() {
                    settings.spark_path = (!text.trim().is_empty()).then(|| PathBuf::from(text.trim()));
                }
                if ui.button(t("common.browse")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_file() {
                        settings.spark_path = Some(p);
                    }
                }
            });
            let found = process::find_spark(settings.spark_path.as_deref());
            match found {
                Some(p) => ui.label(RichText::new(tf("settings.spark_found", &[("path", &p.display())])).small().color(theme::OK)),
                None => ui.label(RichText::new(t("settings.spark_missing")).small().color(theme::ERROR)),
            };
        });
        ui.end_row();

        ui.label(t("settings.run"));
        ui.vertical(|ui| {
            ui.checkbox(&mut settings.save_before_run, t("settings.save_before_run"));
            ui.checkbox(&mut settings.check_on_save, t("settings.check_on_save"));
        });
        ui.end_row();

        ui.label(t("settings.explorer"));
        ui.checkbox(&mut settings.show_hidden_files, t("settings.show_hidden"));
        ui.end_row();
    });
    if let Some(path) = crate::settings::Settings::file() {
        ui.add_space(16.0);
        ui.label(RichText::new(tf("settings.stored_in", &[("path", &path.display())])).small().color(theme::TEXT_DIM));
    }
    *settings != before
}

fn about_page(ui: &mut egui::Ui) {
    ui.heading(t("app.name"));
    ui.label(RichText::new(tf("about.version", &[("version", &crate::VERSION)])).color(theme::TEXT_DIM));
    ui.add_space(12.0);
    ui.label(t("about.body"));
    ui.add_space(12.0);
    ui.hyperlink_to("acelate.com/spark", "https://acelate.com/spark");
    ui.hyperlink_to("github.com/AcelateOrg/Spark", "https://github.com/AcelateOrg/Spark");
    ui.add_space(20.0);
    egui::Frame::new().fill(theme::SURFACE).corner_radius(6).inner_margin(Margin::same(12)).stroke(Stroke::new(1.0, theme::BORDER)).show(ui, |ui| {
        ui.label(RichText::new(t("about.cli_title")).strong());
        ui.monospace("spark editor              # → this screen\nspark editor .            # open the game in this folder\nspark new mygame          # new game from the terminal");
    });
}
