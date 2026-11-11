//! Spark game projects, shared by the `spark` runner and Spark Editor:
//!
//! - [`manifest`]: `game.toml` (title, window, build settings, target engine version)
//! - [`detect`]: "is this folder really a Spark game?" + finding the project root
//! - [`template`]: the `spark new` game and the engine docs shipped into every game

pub mod detect;
pub mod manifest;
pub mod template;

pub use detect::{Confidence, NotAProject, Project, detect, find_root};
pub use manifest::Manifest;

use std::path::{Path, PathBuf};

/// Entry script of every game.
pub const ENTRY: &str = "main.luau";

/// Creates a new game in `dir` (must not exist yet or be empty) from the standard template.
pub fn create(dir: &Path, title: &str) -> Result<PathBuf, String> {
    if dir.join(ENTRY).exists() {
        return Err(format!("'{}' already has a {ENTRY}", dir.display()));
    }
    if dir.is_file() {
        return Err(format!("'{}' is a file", dir.display()));
    }
    if let Ok(mut entries) = std::fs::read_dir(dir) {
        if entries.next().is_some() {
            return Err(format!("'{}' is not empty", dir.display()));
        }
    }
    template::write_template(dir, title)?;
    Ok(dir.to_path_buf())
}

/// A folder / file name that works on every OS (for new projects).
pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n.len() <= 120
        && n != "."
        && n != ".."
        && !n.ends_with('.')
        && !n.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("My Game"));
        assert!(valid_name("Игра 2"));
        assert!(!valid_name(""));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("what?"));
        assert!(!valid_name(".."));
    }

    #[test]
    fn create_refuses_existing_games() {
        let dir = std::env::temp_dir().join(format!("spark-project-create-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create(&dir, "Test").unwrap();
        assert!(dir.join(ENTRY).is_file());
        assert!(create(&dir, "Test").is_err());
        let p = detect(&dir).unwrap();
        assert_eq!(p.confidence, Confidence::Manifest);
        assert_eq!(p.title, "Test");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
