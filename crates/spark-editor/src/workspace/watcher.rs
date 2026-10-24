//! Watches the project folder: the explorer re-scans and open documents reload when files change on disk
//! (git checkout, an AI agent editing scripts, `spark docs` ...).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use notify::{RecursiveMode, Watcher as _};

/// Folders whose changes do not matter to the editor.
const IGNORED: &[&str] = &[".git", "target", "dist"];

pub struct Watcher {
    _watcher: notify::RecommendedWatcher,
    events: Receiver<Vec<PathBuf>>,
    root: PathBuf,
}

impl Watcher {
    pub fn new(root: &Path, ctx: &egui::Context) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if event.kind.is_access() {
                    return;
                }
                if tx.send(event.paths).is_ok() {
                    ctx.request_repaint();
                }
            }
        })
        .map_err(|e| e.to_string())?;
        watcher.watch(root, RecursiveMode::Recursive).map_err(|e| e.to_string())?;
        Ok(Self { _watcher: watcher, events: rx, root: root.to_path_buf() })
    }

    /// Paths changed since the last call (deduplicated, ignored folders removed).
    pub fn changes(&self) -> HashSet<PathBuf> {
        let mut out = HashSet::new();
        for paths in self.events.try_iter() {
            for p in paths {
                let rel = p.strip_prefix(&self.root).unwrap_or(&p);
                let ignored = rel.components().next().is_some_and(|c| IGNORED.iter().any(|i| c.as_os_str() == *i));
                if !ignored {
                    out.insert(p);
                }
            }
        }
        out
    }
}
