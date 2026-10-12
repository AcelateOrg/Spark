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

/// The OS mouse cursor in screen pixels (Windows; `None` elsewhere: use egui's pointer events).
pub fn cursor_pos() -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: plain Win32 call writing into a local POINT.
        if unsafe { GetCursorPos(&mut p) } != 0 {
            return Some((p.x, p.y));
        }
    }
    None
}

/// Moves the OS mouse cursor (screen pixels).
pub fn set_cursor_pos(p: (i32, i32)) {
    #[cfg(windows)]
    // SAFETY: plain Win32 call.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos(p.0, p.1);
    }
    #[cfg(not(windows))]
    let _ = p;
}

pub fn open_path(path: &Path) {
    open(&path.to_string_lossy());
}
