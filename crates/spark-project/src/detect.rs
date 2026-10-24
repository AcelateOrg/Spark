//! Is a folder a Spark game? Used by `spark editor <path>` and by the editor's "Open" dialog.
//!
//! A Spark game is a folder with `main.luau`. How sure we are:
//! - [`Confidence::Manifest`]: `game.toml` declares the engine version (`spark = "0.1"`) - made by `spark new`
//! - [`Confidence::Layout`]: `main.luau` + a valid `game.toml` without the version
//! - [`Confidence::Script`]: only `main.luau`, but it defines Spark callbacks (`function start()` ...)
//!
//! Anything else (no `main.luau`, a broken `game.toml`, a plain Lua project) is rejected with a reason.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::manifest::{self, Manifest};
use crate::ENTRY;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Script,
    Layout,
    Manifest,
}

/// A detected Spark game.
#[derive(Clone, Debug)]
pub struct Project {
    /// Canonical project folder.
    pub root: PathBuf,
    /// `root/main.luau`
    pub entry: PathBuf,
    pub manifest: Manifest,
    /// `game.toml` title or the folder name.
    pub title: String,
    pub confidence: Confidence,
    /// Non-fatal problems (engine version mismatch ...).
    pub warnings: Vec<String>,
}

/// Why a folder is not a Spark game.
#[derive(Clone, Debug, PartialEq)]
pub enum NotAProject {
    Missing(PathBuf),
    NotADirectory(PathBuf),
    /// No `main.luau`.
    NoEntry(PathBuf),
    /// `game.toml` exists but cannot be read.
    BadManifest(String),
    /// `main.luau` without `game.toml` that does not look like a Spark game.
    NotSpark(PathBuf),
}

impl fmt::Display for NotAProject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(p) => write!(f, "'{}' does not exist", p.display()),
            Self::NotADirectory(p) => write!(f, "'{}' is not a folder", p.display()),
            Self::NoEntry(p) => write!(f, "'{}' has no {ENTRY} - not a Spark game (create one: spark new <name>)", p.display()),
            Self::BadManifest(e) => write!(f, "{e}"),
            Self::NotSpark(p) => write!(
                f,
                "'{}' has a {ENTRY}, but no {} and no Spark callbacks (start / update / render) - not a Spark game",
                p.display(),
                manifest::FILE
            ),
        }
    }
}

impl std::error::Error for NotAProject {}

/// Spark callbacks / globals that only a Spark game defines or uses.
const SPARK_MARKERS: &[&str] = &[
    "function start(",
    "function update(",
    "function fixed_update(",
    "function late_update(",
    "function render(",
    "spawn(Mesh.",
    "draw.text(",
    "draw.sprite(",
];

/// Checks that `path` (a folder, or `main.luau` itself) is a Spark game.
pub fn detect(path: &Path) -> Result<Project, NotAProject> {
    if !path.exists() {
        return Err(NotAProject::Missing(path.to_path_buf()));
    }
    let dir = if path.is_file() {
        match path.file_name().and_then(|n| n.to_str()) {
            Some(ENTRY) => path.parent().unwrap_or(path).to_path_buf(),
            _ => return Err(NotAProject::NotADirectory(path.to_path_buf())),
        }
    } else {
        path.to_path_buf()
    };
    let root = normalize(&dir);
    let entry = root.join(ENTRY);
    if !entry.is_file() {
        return Err(NotAProject::NoEntry(root));
    }
    let has_manifest = root.join(manifest::FILE).is_file();
    let manifest = Manifest::load(&root).map_err(NotAProject::BadManifest)?;
    let confidence = if manifest.spark.is_some() {
        Confidence::Manifest
    } else if has_manifest {
        Confidence::Layout
    } else {
        let src = std::fs::read_to_string(&entry).unwrap_or_default();
        if !SPARK_MARKERS.iter().any(|m| src.contains(m)) {
            return Err(NotAProject::NotSpark(root));
        }
        Confidence::Script
    };
    let mut warnings = Vec::new();
    if let Some(w) = manifest.version_warning(spark_core::VERSION) {
        warnings.push(w);
    }
    let title = manifest.display_title(&root);
    Ok(Project { root, entry, manifest, title, confidence, warnings })
}

/// The game folder containing `path`: `path` itself or the nearest parent with `main.luau` + `game.toml`
/// (so `spark editor .` works from `mygame/src` too). Falls back to `path`.
pub fn find_root(path: &Path) -> PathBuf {
    let start = normalize(path);
    let mut cur = if start.is_file() { start.parent().map(Path::to_path_buf) } else { Some(start.clone()) };
    let mut depth = 0;
    while let Some(dir) = cur {
        if dir.join(ENTRY).is_file() && (depth == 0 || dir.join(manifest::FILE).is_file()) {
            return dir;
        }
        depth += 1;
        if depth > 16 {
            break;
        }
        cur = dir.parent().map(Path::to_path_buf);
    }
    start
}

/// Absolute path without `\\?\` on Windows (nicer to show, works with every tool).
fn normalize(p: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| {
        if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_path_buf()) }
    });
    #[cfg(windows)]
    {
        let s = abs.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    abs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spark-detect-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (rel, text) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn confidence_levels() {
        let a = temp("manifest", &[("main.luau", ""), ("game.toml", "title = \"A\"\nspark = \"0.1\"\n")]);
        let p = detect(&a).unwrap();
        assert_eq!((p.confidence, p.title.as_str()), (Confidence::Manifest, "A"));
        let b = temp("layout", &[("main.luau", ""), ("game.toml", "width = 800\n")]);
        assert_eq!(detect(&b).unwrap().confidence, Confidence::Layout);
        let c = temp("script", &[("main.luau", "function update(dt)\nend\n")]);
        assert_eq!(detect(&c).unwrap().confidence, Confidence::Script);
        // A file path to main.luau works too.
        assert_eq!(detect(&c.join("main.luau")).unwrap().root, detect(&c).unwrap().root);
        for d in [a, b, c] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn rejects_non_games() {
        let empty = temp("empty", &[]);
        assert!(matches!(detect(&empty), Err(NotAProject::NoEntry(_))));
        let lua = temp("lua", &[("main.luau", "print('hi')\n")]);
        assert!(matches!(detect(&lua), Err(NotAProject::NotSpark(_))));
        let bad = temp("bad", &[("main.luau", ""), ("game.toml", "titel = 1\n")]);
        assert!(matches!(detect(&bad), Err(NotAProject::BadManifest(_))));
        assert!(matches!(detect(&empty.join("nope")), Err(NotAProject::Missing(_))));
        for d in [empty, lua, bad] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn finds_root_from_subfolder() {
        let g = temp("root", &[("main.luau", ""), ("game.toml", ""), ("src/player.luau", "")]);
        let root = detect(&g).unwrap().root;
        assert_eq!(find_root(&g.join("src")), root);
        assert_eq!(find_root(&g.join("src/player.luau")), root);
        assert_eq!(find_root(&g), root);
        let _ = std::fs::remove_dir_all(g);
    }
}
