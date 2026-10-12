//! The editor's logger: env_logger for the terminal, plus a copy of the engine's messages
//! (`print()` from scripts, script errors, asset warnings) while a game runs inside the editor
//! (the scene viewport), so they show up in the console panel.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use log::{Level, LevelFilter, Log, Metadata, Record};

static CAPTURE: AtomicBool = AtomicBool::new(false);
static LINES: Mutex<Vec<(Level, String)>> = Mutex::new(Vec::new());
/// Never keep more than this many unread lines (a script printing every frame).
const MAX_LINES: usize = 5000;

struct Tee {
    inner: env_logger::Logger,
}

/// Messages of the engine crates (not of the editor itself, not of wgpu / naga).
fn from_engine(target: &str) -> bool {
    (target == "spark" || target.starts_with("spark_") || target.starts_with("spark::")) && !target.starts_with("spark_editor")
}

impl Log for Tee {
    fn enabled(&self, m: &Metadata) -> bool {
        self.inner.enabled(m) || (from_engine(m.target()) && m.level() <= Level::Info)
    }

    fn log(&self, record: &Record) {
        if self.inner.matches(record) {
            self.inner.log(record);
        }
        if CAPTURE.load(Ordering::Relaxed) && from_engine(record.target()) && record.level() <= Level::Info {
            if let Ok(mut lines) = LINES.lock() {
                if lines.len() < MAX_LINES {
                    lines.push((record.level(), record.args().to_string()));
                }
            }
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// Installs the logger (`RUST_LOG` works as usual for the terminal output).
pub fn init(default_filter: &str) {
    let env = env_logger::Env::default().default_filter_or(default_filter);
    let inner = env_logger::Builder::from_env(env).format_timestamp(None).build();
    let max = inner.filter().max(LevelFilter::Info);
    if log::set_boxed_logger(Box::new(Tee { inner })).is_ok() {
        log::set_max_level(max);
    }
}

/// Starts / stops copying engine messages (the viewport turns it on while its game exists).
pub fn capture(on: bool) {
    CAPTURE.store(on, Ordering::Relaxed);
    if !on {
        if let Ok(mut lines) = LINES.lock() {
            lines.clear();
        }
    }
}

/// Engine messages since the last call.
pub fn take() -> Vec<(Level, String)> {
    LINES.lock().map(|mut l| std::mem::take(&mut *l)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn engine_targets() {
        assert!(super::from_engine("spark_script::api"));
        assert!(super::from_engine("spark_render"));
        assert!(!super::from_engine("spark_editor::workspace"));
        assert!(!super::from_engine("wgpu_core::device"));
    }
}
