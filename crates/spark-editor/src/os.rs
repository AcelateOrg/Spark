//! OS integration: show a file in the system file manager, open with the default program.

use std::path::Path;
use std::process::Command;

/// Shows `path` selected in Explorer / Finder / the Linux file manager.
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    let result = {
        use std::os::windows::process::CommandExt;
        // `explorer /select,"path"` needs the raw argument (no extra quoting by Rust).
        Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn()
    };
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let result = Command::new("xdg-open").arg(if path.is_dir() { path } else { path.parent().unwrap_or(path) }).spawn();
    if let Err(e) = result {
        log::warn!("cannot show {}: {e}", path.display());
    }
}

/// Opens a file / folder / URL with the program the OS associates with it.
pub fn open(target: &str) {
    #[cfg(windows)]
    let result = Command::new("rundll32").args(["url.dll,FileProtocolHandler", target]).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(target).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let result = Command::new("xdg-open").arg(target).spawn();
    if let Err(e) = result {
        log::warn!("cannot open {target}: {e}");
    }
}

pub fn open_path(path: &Path) {
    open(&path.to_string_lossy());
}
