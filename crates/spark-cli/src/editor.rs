//! `spark editor [path]`: opens Spark Editor.
//!
//! The editor is a separate program (`spark-editor`) installed next to `spark`: player builds are copies
//! of the `spark` executable, so the editor UI must not live inside it.
//!
//! - `spark editor` - the welcome screen (recent projects, new game, open folder)
//! - `spark editor .` - checks that the folder (or a parent of it) is a Spark game, then opens it

use std::path::{Path, PathBuf};
use std::process::Command;

const USAGE: &str = "usage: spark editor [path/to/game]";

/// Overrides where the editor executable is (development: `target/debug/spark-editor`).
pub const ENV_EDITOR: &str = "SPARK_EDITOR";
/// Tells the editor which `spark` executable started it (it runs games with the same engine).
pub const ENV_SPARK: &str = "SPARK_EXE";

pub fn run(args: &[String]) -> i32 {
    let project = match args {
        [] => None,
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            return 0;
        }
        [path] => match open_target(Path::new(path)) {
            Ok(root) => Some(root),
            Err(e) => {
                eprintln!("spark editor: {e}");
                return 2;
            }
        },
        _ => {
            eprintln!("{USAGE}");
            return 2;
        }
    };
    let editor = match find_editor() {
        Ok(p) => p,
        Err(expected) => {
            eprintln!(
                "spark editor: Spark Editor is not installed (expected {}).\n  reinstall Spark, or build it from source: cargo build --release -p spark-editor",
                expected.display()
            );
            return 1;
        }
    };
    let mut cmd = Command::new(&editor);
    if let Some(root) = &project {
        cmd.arg(root);
    }
    if let Ok(me) = std::env::current_exe() {
        cmd.env(ENV_SPARK, me);
    }
    // Detached: the terminal is free again right away, like `code .` / `idea .`.
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    match cmd.spawn() {
        Ok(_) => {
            if let Some(root) = &project {
                println!("spark editor: opening {}", root.display());
            }
            0
        }
        Err(e) => {
            eprintln!("spark editor: cannot start {}: {e}", editor.display());
            1
        }
    }
}

/// The Spark game at `path` (or the nearest parent game), with warnings printed.
fn open_target(path: &Path) -> Result<PathBuf, String> {
    let root = spark_project::find_root(path);
    let project = spark_project::detect(&root).map_err(|e| e.to_string())?;
    for w in &project.warnings {
        eprintln!("spark editor: warning: {w}");
    }
    Ok(project.root)
}

fn editor_file_name() -> &'static str {
    if cfg!(windows) { "spark-editor.exe" } else { "spark-editor" }
}

/// `$SPARK_EDITOR`, else `spark-editor` next to this executable. `Err` = where it was expected.
fn find_editor() -> Result<PathBuf, PathBuf> {
    if let Some(p) = std::env::var_os(ENV_EDITOR).map(PathBuf::from) {
        return if p.is_file() { Ok(p) } else { Err(p) };
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(editor_file_name())))
        .unwrap_or_else(|| PathBuf::from(editor_file_name()));
    if beside.is_file() { Ok(beside) } else { Err(beside) }
}
