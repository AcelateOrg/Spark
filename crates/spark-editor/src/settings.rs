//! Editor settings and recent projects: `<config dir>/Spark/editor.toml`
//! (Windows: `%APPDATA%\Spark\editor.toml`, Linux: `~/.config/Spark/editor.toml`, macOS: `~/Library/Application Support/Spark`).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

const MAX_RECENT: usize = 30;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// `None` = follow the OS language.
    pub language: Option<Lang>,
    pub ui_scale: f32,
    pub editor_font_size: f32,
    /// Explicit `spark` executable (`None` = auto-detect).
    pub spark_path: Option<PathBuf>,
    /// Save files automatically when the editor loses focus / before running the game.
    pub save_before_run: bool,
    /// Run `spark check` after every save.
    pub check_on_save: bool,
    pub show_hidden_files: bool,
    /// Where "New project" puts games by default.
    pub projects_dir: Option<PathBuf>,
    pub recent: Vec<RecentProject>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: None,
            ui_scale: 1.0,
            editor_font_size: 14.0,
            spark_path: None,
            save_before_run: true,
            check_on_save: false,
            show_hidden_files: false,
            projects_dir: None,
            recent: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RecentProject {
    pub path: PathBuf,
    pub title: String,
    /// Unix seconds.
    pub last_opened: u64,
    /// Files open when the project was closed (restored on open), relative to the project.
    pub open_files: Vec<PathBuf>,
    pub active_file: Option<PathBuf>,
}

impl Default for RecentProject {
    fn default() -> Self {
        Self { path: PathBuf::new(), title: String::new(), last_opened: 0, open_files: Vec::new(), active_file: None }
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Settings {
    pub fn file() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("Spark").join("editor.toml"))
    }

    /// Settings from disk; defaults when missing or broken (a broken file is kept as `editor.toml.bak`).
    pub fn load() -> Self {
        let Some(path) = Self::file() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&path) else { return Self::default() };
        match toml::from_str::<Self>(&text) {
            Ok(mut s) => {
                s.sanitize();
                s
            }
            Err(e) => {
                log::warn!("{}: {e} (using defaults)", path.display());
                let _ = std::fs::copy(&path, path.with_extension("toml.bak"));
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::file() else { return };
        let result = toml::to_string_pretty(self).map_err(|e| e.to_string()).and_then(|text| {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            // Write + rename: a crash never leaves half a settings file.
            let tmp = path.with_extension("toml.tmp");
            std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
        });
        if let Err(e) = result {
            log::warn!("cannot save {}: {e}", path.display());
        }
    }

    fn sanitize(&mut self) {
        self.ui_scale = self.ui_scale.clamp(0.75, 2.5);
        self.editor_font_size = self.editor_font_size.clamp(8.0, 32.0);
        self.recent.truncate(MAX_RECENT);
    }

    pub fn language(&self) -> Lang {
        self.language.unwrap_or_else(Lang::system)
    }

    /// Default parent folder for new games: the last used one, else `Documents/Spark Games`.
    pub fn projects_dir(&self) -> PathBuf {
        self.projects_dir
            .clone()
            .or_else(|| dirs::document_dir().map(|d| d.join("Spark Games")))
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn recent(&self, root: &Path) -> Option<&RecentProject> {
        self.recent.iter().find(|r| r.path == root)
    }

    /// Moves the project to the top of the recent list.
    pub fn touch_recent(&mut self, root: &Path, title: &str) {
        let mut entry = self
            .recent
            .iter()
            .position(|r| r.path == root)
            .map(|i| self.recent.remove(i))
            .unwrap_or_else(|| RecentProject { path: root.to_path_buf(), ..Default::default() });
        entry.title = title.to_string();
        entry.last_opened = now_unix();
        self.recent.insert(0, entry);
        self.recent.truncate(MAX_RECENT);
    }

    pub fn remember_files(&mut self, root: &Path, open: Vec<PathBuf>, active: Option<PathBuf>) {
        if let Some(r) = self.recent.iter_mut().find(|r| r.path == root) {
            r.open_files = open;
            r.active_file = active;
        }
    }

    pub fn remove_recent(&mut self, root: &Path) {
        self.recent.retain(|r| r.path != root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_recent_order() {
        let mut s = Settings { language: Some(Lang::Ru), ..Default::default() };
        s.touch_recent(Path::new("/a"), "A");
        s.touch_recent(Path::new("/b"), "B");
        s.touch_recent(Path::new("/a"), "A2");
        assert_eq!(s.recent.len(), 2);
        assert_eq!(s.recent[0].title, "A2");
        s.remember_files(Path::new("/a"), vec![PathBuf::from("main.luau")], None);
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back, s);
        // Old / partial files still load.
        let partial: Settings = toml::from_str("ui_scale = 1.25\n").unwrap();
        assert_eq!(partial.ui_scale, 1.25);
        assert!(partial.recent.is_empty());
    }
}
