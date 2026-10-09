//! `spark build path/to/game [--out DIR] [--loose] [--no-zip]`: a build that players double-click.
//!
//! Default: ONE executable. It is a copy of the running `spark` binary with the Windows subsystem
//! switched to GUI (no console window) and every game file (scripts, textures, sounds, models,
//! shaders, fonts, game.toml) packed at its end (see `engine::vfs`). Nothing else is needed next to it.
//! `--loose`: the old layout - the exe plus the game files in a folder (easy to mod).

use std::path::{Path, PathBuf};

use engine::vfs::{PackWriter, strip_pack};

use crate::manifest::Manifest;

/// Never copied into a build.
const SKIP_DIRS: &[&str] = &["tools", "dist", "screenshots", "target", ".git", ".vscode", ".idea"];
const SKIP_EXT: &[&str] = &["log", "tmp", "bak", "blend", "blend1", "psd", "kra", "xcf"];

pub fn run(args: &[String]) -> i32 {
    match build(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("spark build: {e}");
            1
        }
    }
}

fn build(args: &[String]) -> Result<(), String> {
    let mut game: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut zip = true;
    let mut loose = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a folder")?)),
            "--no-zip" => zip = false,
            "--loose" => loose = true,
            "--help" | "-h" => {
                println!("{}", crate::USAGE);
                return Ok(());
            }
            _ if game.is_none() => game = Some(PathBuf::from(a)),
            _ => return Err(format!("unexpected argument '{a}'\n{}", crate::USAGE)),
        }
    }
    let game = game.unwrap_or_else(|| PathBuf::from("."));
    let dir = if game.is_file() { game.parent().map(Path::to_path_buf).unwrap_or_default() } else { game.clone() };
    let dir = dir.canonicalize().map_err(|e| format!("cannot open '{}': {e}", game.display()))?;
    if !dir.join("main.luau").is_file() {
        return Err(format!("'{}' has no main.luau", dir.display()));
    }
    let m = Manifest::load(&dir)?;
    let title = m.title.clone().unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or("game".into()));
    let name = file_name(m.build_name.as_deref().unwrap_or(&title));
    let out_root = out.unwrap_or_else(|| PathBuf::from("dist"));
    std::fs::create_dir_all(&out_root).map_err(|e| format!("cannot create '{}': {e}", out_root.display()))?;
    let exe_name = if cfg!(windows) { format!("{name}.exe") } else { name.clone() };

    // The player executable: this binary without any old pack, console hidden.
    let me = std::env::current_exe().map_err(|e| format!("cannot find spark.exe: {e}"))?;
    let raw = std::fs::read(&me).map_err(|e| format!("cannot read '{}': {e}", me.display()))?;
    let mut exe = strip_pack(&raw).to_vec();
    if cfg!(windows) && !set_gui_subsystem(&mut exe) {
        eprintln!("spark build: warning: could not hide the console window (unknown exe format)");
    }

    let mut files = Vec::new();
    collect(&dir, &dir, &m, &mut files)?;

    // What gets zipped (relative to out_root).
    let shipped: PathBuf;
    if loose {
        let target = out_root.join(&name);
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(|e| format!("cannot clear '{}': {e} (is the game still running?)", target.display()))?;
        }
        std::fs::create_dir_all(&target).map_err(|e| format!("cannot create '{}': {e}", target.display()))?;
        write_exe(&target.join(&exe_name), &exe)?;
        let mut bytes = 0u64;
        for rel in &files {
            let dest = target.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("cannot create '{}': {e}", parent.display()))?;
            }
            bytes += std::fs::copy(dir.join(rel), &dest).map_err(|e| format!("cannot copy '{}': {e}", rel.display()))?;
        }
        println!("spark build: {} -> {} ({} files, {:.1} MB + exe)", dir.display(), target.display(), files.len(), bytes as f64 / 1e6);
        shipped = PathBuf::from(&name);
    } else {
        let mut pack = PackWriter::new();
        for rel in &files {
            let bytes = std::fs::read(dir.join(rel)).map_err(|e| format!("cannot read '{}': {e}", rel.display()))?;
            pack.add(&rel.to_string_lossy().replace('\\', "/"), &bytes);
        }
        let (count, raw_bytes) = (pack.count(), pack.raw_bytes);
        let base = exe.len();
        let full = pack.finish(&exe);
        let path = out_root.join(&exe_name);
        write_exe(&path, &full)?;
        println!(
            "spark build: {} -> {} ({count} files, {:.1} MB packed into {:.1} MB, engine {:.1} MB)",
            dir.display(),
            path.display(),
            raw_bytes as f64 / 1e6,
            (full.len() - base) as f64 / 1e6,
            base as f64 / 1e6
        );
        shipped = PathBuf::from(&exe_name);
    }

    // Zip (Windows 10+ and macOS ship bsdtar, which writes zip files). Browsers and chats often
    // block a bare .exe download, a zip passes.
    if zip {
        let zip_path = out_root.join(format!("{name}.zip"));
        let _ = std::fs::remove_file(&zip_path);
        let status = std::process::Command::new("tar")
            .arg("-a")
            .arg("-c")
            .arg("-f")
            .arg(format!("{name}.zip"))
            .arg(&shipped)
            .current_dir(&out_root)
            .status();
        match status {
            Ok(s) if s.success() && zip_path.is_file() => {
                let size = std::fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);
                println!("spark build: {} ({:.1} MB)", zip_path.display(), size as f64 / 1e6);
            }
            _ => eprintln!("spark build: warning: could not create the zip (zip it by hand)"),
        }
    }
    println!("spark build: done. Players run {exe_name}");
    Ok(())
}

fn write_exe(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("cannot write '{}': {e} (is the game still running?)", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// A safe file name from a title (keeps Unicode letters).
fn file_name(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '(' | ')') { c } else { '_' })
        .collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "game".into() } else { s }
}

fn skipped(rel: &Path, is_dir: bool, m: &Manifest) -> bool {
    let name = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.starts_with('.') {
        return true;
    }
    if is_dir && rel.components().count() == 1 && SKIP_DIRS.contains(&name.as_str()) {
        return true;
    }
    if !is_dir {
        let ext = rel.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if SKIP_EXT.contains(&ext.as_str()) || name == "error.log" {
            return true;
        }
    }
    let rel_s = rel.to_string_lossy().replace('\\', "/");
    m.exclude.iter().any(|e| {
        let e = e.trim_matches('/');
        rel_s == e || rel_s.starts_with(&format!("{e}/"))
    })
}

/// Game files to ship (relative paths), minus development leftovers.
fn collect(root: &Path, dir: &Path, m: &Manifest, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("cannot read '{}': {e}", dir.display()))?;
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        let is_dir = path.is_dir();
        if skipped(&rel, is_dir, m) {
            continue;
        }
        if is_dir {
            collect(root, &path, m, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

/// Switches a PE executable from the console subsystem (3) to the GUI subsystem (2).
fn set_gui_subsystem(exe: &mut [u8]) -> bool {
    if exe.len() < 0x40 || &exe[0..2] != b"MZ" {
        return false;
    }
    let pe = u32::from_le_bytes([exe[0x3c], exe[0x3d], exe[0x3e], exe[0x3f]]) as usize;
    // "PE\0\0" + COFF header (20 bytes) + optional header; Subsystem is at offset 68 in both PE32 and PE32+.
    let field = pe + 4 + 20 + 68;
    if exe.len() < field + 2 || &exe[pe..pe + 4] != b"PE\0\0" {
        return false;
    }
    let magic = u16::from_le_bytes([exe[pe + 24], exe[pe + 25]]);
    if magic != 0x10b && magic != 0x20b {
        return false;
    }
    exe[field..field + 2].copy_from_slice(&2u16.to_le_bytes());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_skips() {
        assert_eq!(file_name("Ночь: игра?"), "Ночь_ игра_");
        let m = Manifest { exclude: vec!["notes".into()], ..Default::default() };
        assert!(skipped(Path::new("tools"), true, &m));
        assert!(!skipped(Path::new("textures"), true, &m));
        assert!(skipped(Path::new("notes/a.txt"), false, &m));
        assert!(skipped(Path::new("error.log"), false, &m));
        assert!(!skipped(Path::new("sounds/a.ogg"), false, &m));
    }

    #[test]
    fn patches_subsystem() {
        let mut exe = vec![0u8; 512];
        exe[0..2].copy_from_slice(b"MZ");
        exe[0x3c] = 0x80;
        exe[0x80..0x84].copy_from_slice(b"PE\0\0");
        exe[0x80 + 24..0x80 + 26].copy_from_slice(&0x20bu16.to_le_bytes());
        exe[0x80 + 92] = 3;
        assert!(set_gui_subsystem(&mut exe));
        assert_eq!(exe[0x80 + 92], 2);
        assert!(!set_gui_subsystem(&mut [0u8; 10]));
    }
}
