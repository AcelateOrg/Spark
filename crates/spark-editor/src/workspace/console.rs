//! The output of the game, `spark check` and `spark build`: level filters, search, clickable `file:line`.

use std::collections::VecDeque;
use std::path::PathBuf;

use eframe::egui::{self, Color32, FontId, RichText};

use super::Action;
use crate::i18n::{t, tf};
use crate::process::{JobKind, Level, LogLine, Stream};
use crate::theme::{self, icon};

/// Older lines are dropped (a chatty game must not eat the memory).
const MAX_LINES: usize = 20_000;

pub struct Console {
    lines: VecDeque<LogLine>,
    /// Shown levels: trace/debug, info, warn, error.
    show_debug: bool,
    show_info: bool,
    show_warn: bool,
    show_error: bool,
    search: String,
    pub clear_on_run: bool,
    /// Lines with `Level::Warn` / `Level::Error` since the last clear.
    warnings: usize,
    errors: usize,
    dropped: usize,
}

impl Default for Console {
    fn default() -> Self {
        Self {
            lines: VecDeque::new(),
            show_debug: false,
            show_info: true,
            show_warn: true,
            show_error: true,
            search: String::new(),
            clear_on_run: true,
            warnings: 0,
            errors: 0,
            dropped: 0,
        }
    }
}

impl Console {
    pub fn push(&mut self, line: LogLine) {
        match line.level {
            Level::Warn => self.warnings += 1,
            Level::Error => self.errors += 1,
            _ => {}
        }
        if self.lines.len() >= MAX_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(line);
    }

    pub fn extend(&mut self, lines: impl IntoIterator<Item = LogLine>) {
        for l in lines {
            self.push(l);
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.warnings = 0;
        self.errors = 0;
        self.dropped = 0;
    }

    /// `(warnings, errors)` since the last clear (tab title badge).
    pub fn counts(&self) -> (usize, usize) {
        (self.warnings, self.errors)
    }

    fn visible(&self, line: &LogLine) -> bool {
        let level_ok = match line.level {
            Level::Trace | Level::Debug => self.show_debug,
            Level::Info => self.show_info,
            Level::Warn => self.show_warn,
            Level::Error => self.show_error,
        };
        level_ok && (self.search.is_empty() || line.text.to_lowercase().contains(&self.search.to_lowercase()))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(format!("{} {}", icon::SEARCH, t("console.filter"))).desired_width(200.0));
            ui.separator();
            ui.toggle_value(&mut self.show_debug, t("console.debug"));
            ui.toggle_value(&mut self.show_info, t("console.info"));
            ui.toggle_value(&mut self.show_warn, RichText::new(format!("{} {}", t("console.warnings"), self.warnings)).color(theme::WARN));
            ui.toggle_value(&mut self.show_error, RichText::new(format!("{} {}", t("console.errors"), self.errors)).color(theme::ERROR));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t("console.clear")).clicked() {
                    self.clear();
                }
                if ui.button(t("console.copy")).on_hover_text(t("console.copy_hint")).clicked() {
                    let text: Vec<&str> = self.lines.iter().filter(|l| self.visible(l)).map(|l| l.text.as_str()).collect();
                    ui.ctx().copy_text(text.join("\n"));
                }
                ui.checkbox(&mut self.clear_on_run, t("console.clear_on_run"));
            });
        });
        ui.separator();

        let filtered: Vec<usize> = (0..self.lines.len()).filter(|&i| self.visible(&self.lines[i])).collect();
        if filtered.is_empty() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| ui.label(RichText::new(t("console.empty")).color(theme::TEXT_DIM)));
            return;
        }
        if self.dropped > 0 {
            ui.label(RichText::new(tf("console.dropped", &[("count", &self.dropped)])).small().color(theme::TEXT_DIM));
        }
        let font = FontId::monospace(12.5);
        let row_height = ui.fonts_mut(|f| f.row_height(&font)) + 2.0;
        egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show_rows(ui, row_height, filtered.len(), |ui, rows| {
            for &i in &filtered[rows] {
                let line = &self.lines[i];
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new(format!("{:>8.2}", line.time)).font(font.clone()).color(Color32::from_gray(90)));
                    ui.label(RichText::new(job_tag(line.job)).font(font.clone()).color(theme::TEXT_DIM));
                    let color = level_color(line.level, line.stream);
                    match &line.location {
                        Some(loc) => {
                            let r = ui.add(egui::Label::new(RichText::new(&line.text).font(font.clone()).color(color).underline()).sense(egui::Sense::click()));
                            if r.clicked() {
                                actions.push(Action::OpenAt(PathBuf::from(&loc.file), loc.line));
                            }
                            r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tf("console.open_at", &[("file", &loc.file), ("line", &loc.line)]));
                        }
                        None => {
                            ui.label(RichText::new(&line.text).font(font.clone()).color(color));
                        }
                    }
                });
            }
        });
    }
}

fn job_tag(job: JobKind) -> &'static str {
    match job {
        JobKind::Run => "game ",
        JobKind::Check => "check",
        JobKind::Build => "build",
    }
}

fn level_color(level: Level, stream: Stream) -> Color32 {
    match level {
        Level::Error => theme::ERROR,
        Level::Warn => theme::WARN,
        Level::Trace | Level::Debug => theme::TEXT_DIM,
        Level::Info if stream == Stream::Editor => theme::INFO,
        Level::Info => theme::TEXT,
    }
}
