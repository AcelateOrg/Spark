//! Errors of the game: from `spark check` (whole project, headless) and from the running game.

use std::path::PathBuf;

use eframe::egui::{self, RichText};

use super::Action;
use crate::i18n::{t, tf};
use crate::process::{CheckReport, find_location};
use crate::theme::{self, icon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Check,
    Runtime,
}

#[derive(Clone, Debug)]
pub struct Problem {
    pub source: Source,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub message: String,
}

#[derive(Default)]
pub struct Problems {
    items: Vec<Problem>,
    /// A `spark check` is running.
    pub checking: bool,
    /// Result summary of the last check: `(ok, frames, objects)`.
    last_check: Option<(bool, u64, u64)>,
}

impl Problems {
    pub fn count(&self) -> usize {
        self.items.len()
    }

    pub fn set_check(&mut self, report: Option<&CheckReport>) {
        self.checking = false;
        self.items.retain(|p| p.source != Source::Check);
        match report {
            Some(r) => {
                self.last_check = Some((r.ok, r.frames, r.objects));
                for e in &r.errors {
                    let file = (!e.file.is_empty()).then(|| e.file.clone());
                    self.items.push(Problem { source: Source::Check, file, line: e.line, message: e.message.clone() });
                }
            }
            None => self.last_check = None,
        }
    }

    /// The running game stopped on a script error (`None`: the error is gone).
    pub fn set_runtime(&mut self, message: Option<&str>) {
        self.items.retain(|p| p.source != Source::Runtime);
        if let Some(msg) = message {
            let loc = find_location(msg);
            self.items.insert(0, Problem {
                source: Source::Runtime,
                file: loc.as_ref().map(|l| l.file.clone()),
                line: loc.map(|l| l.line),
                message: msg.to_string(),
            });
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.checking, egui::Button::new(format!("{} {}", icon::CHECK, t("problems.check_now")))).clicked() {
                actions.push(Action::Check);
            }
            if self.checking {
                ui.spinner();
                ui.label(RichText::new(t("problems.checking")).color(theme::TEXT_DIM));
            } else if let Some((ok, frames, objects)) = self.last_check {
                let (color, text) = if ok {
                    (theme::OK, tf("problems.check_ok", &[("frames", &frames), ("objects", &objects)]))
                } else {
                    (theme::ERROR, tf("problems.check_failed", &[("count", &self.items.iter().filter(|p| p.source == Source::Check).count())]))
                };
                ui.label(RichText::new(text).color(color));
            }
        });
        ui.separator();
        if self.items.is_empty() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| ui.label(RichText::new(t("problems.none")).color(theme::TEXT_DIM)));
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for p in &self.items {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::ERROR).color(theme::ERROR));
                    let tag = match p.source {
                        Source::Check => t("problems.source_check"),
                        Source::Runtime => t("problems.source_runtime"),
                    };
                    ui.label(RichText::new(tag).small().color(theme::TEXT_DIM));
                    if let Some(file) = &p.file {
                        let place = match p.line {
                            Some(l) => format!("{file}:{l}"),
                            None => file.clone(),
                        };
                        if ui.link(RichText::new(place).monospace()).clicked() {
                            actions.push(Action::OpenAt(PathBuf::from(file), p.line.unwrap_or(1)));
                        }
                    }
                    ui.add(egui::Label::new(RichText::new(&p.message).color(theme::TEXT)).wrap());
                });
            }
        });
    }
}
