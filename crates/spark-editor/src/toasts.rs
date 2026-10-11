//! Short notifications in the bottom-right corner.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, RichText};

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Info,
    Success,
    Warning,
    Error,
}

struct Toast {
    kind: Kind,
    text: String,
    created: Instant,
    ttl: Duration,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    pub fn push(&mut self, kind: Kind, text: impl Into<String>) {
        let text = text.into();
        let ttl = Duration::from_secs(match kind {
            Kind::Error => 8,
            Kind::Warning => 6,
            _ => 4,
        });
        // The same message again only restarts its timer.
        if let Some(t) = self.items.iter_mut().find(|t| t.text == text && t.kind == kind) {
            t.created = Instant::now();
            return;
        }
        self.items.push(Toast { kind, text, created: Instant::now(), ttl });
        if self.items.len() > 5 {
            self.items.remove(0);
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.push(Kind::Info, text);
    }
    pub fn success(&mut self, text: impl Into<String>) {
        self.push(Kind::Success, text);
    }
    pub fn warning(&mut self, text: impl Into<String>) {
        self.push(Kind::Warning, text);
    }
    pub fn error(&mut self, text: impl Into<String>) {
        self.push(Kind::Error, text);
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        self.items.retain(|t| t.created.elapsed() < t.ttl);
        if self.items.is_empty() {
            return;
        }
        let mut close = None;
        egui::Area::new(egui::Id::new("spark-toasts"))
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -36.0))
            .order(egui::Order::Tooltip)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                for (i, t) in self.items.iter().enumerate() {
                    let (icon, color) = match t.kind {
                        Kind::Info => (theme::icon::INFO, theme::INFO),
                        Kind::Success => (theme::icon::CHECK, theme::OK),
                        Kind::Warning => (theme::icon::WARN, theme::WARN),
                        Kind::Error => (theme::icon::ERROR, theme::ERROR),
                    };
                    let r = egui::Frame::popup(ui.style())
                        .fill(theme::SURFACE)
                        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.6)))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icon).color(color));
                                ui.add(egui::Label::new(RichText::new(&t.text).color(Color32::WHITE)).wrap());
                            });
                        })
                        .response;
                    if r.interact(egui::Sense::click()).clicked() {
                        close = Some(i);
                    }
                    ui.add_space(4.0);
                }
            });
        if let Some(i) = close {
            self.items.remove(i);
        }
        // Keep repainting so toasts disappear on time.
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
