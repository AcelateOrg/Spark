//! Game files: a folder on disk, or a pack embedded in the executable (`spark build`).
//!
//! Every engine file read (scripts, textures, sounds, models, fonts, shaders, game.toml) goes
//! through [`read`]. When a pack is mounted, paths under its root are served from the pack
//! first and from the disk second (so loose files next to the exe still work, e.g. for mods).
//!
//! Pack layout, appended to the end of the executable:
//! `[entry data ...][index][pack_start: u64][index_start: u64][MAGIC: 8 bytes]`
//! index = `count: u32` then per entry `name_len: u16, name (utf-8, '/' separated), offset: u64,
//! stored: u64, size: u64, method: u8 (0 = stored, 1 = deflate)`; offsets are relative to `pack_start`.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

const MAGIC: &[u8; 8] = b"SPARKPK1";
const FOOTER: u64 = 24;

struct Entry {
    offset: u64,
    stored: u64,
    size: u64,
    deflate: bool,
}

/// Game files packed into an executable.
pub struct Pack {
    file: Mutex<File>,
    start: u64,
    entries: HashMap<String, Entry>,
    names: Vec<String>,
}

struct Mounted {
    root: PathBuf,
    root_key: String,
    pack: Pack,
}

static MOUNTED: OnceLock<Mounted> = OnceLock::new();

impl Pack {
    /// The pack embedded in the running executable, if any.
    pub fn embedded() -> Option<Pack> {
        Self::open(&std::env::current_exe().ok()?).ok().flatten()
    }

    /// Reads the pack at the end of `path` (`Ok(None)` = the file has no pack).
    pub fn open(path: &Path) -> Result<Option<Pack>, String> {
        let err = |e: std::io::Error| format!("cannot read pack '{}': {e}", path.display());
        let mut f = File::open(path).map_err(err)?;
        let Some((start, index)) = footer(&mut f).map_err(err)? else { return Ok(None) };
        f.seek(SeekFrom::Start(index)).map_err(err)?;
        let len = f.metadata().map_err(err)?.len();
        let mut buf = Vec::new();
        (&mut f).take(len - FOOTER - index).read_to_end(&mut buf).map_err(err)?;
        let bad = || format!("'{}': damaged game pack (rebuild with spark build)", path.display());
        let mut r = Reader(&buf);
        let count = r.u32().ok_or_else(bad)?;
        let mut entries = HashMap::new();
        let mut names = Vec::new();
        for _ in 0..count {
            let n = r.u16().ok_or_else(bad)? as usize;
            let name = String::from_utf8(r.bytes(n).ok_or_else(bad)?.to_vec()).map_err(|_| bad())?;
            let e = Entry {
                offset: r.u64().ok_or_else(bad)?,
                stored: r.u64().ok_or_else(bad)?,
                size: r.u64().ok_or_else(bad)?,
                deflate: r.u8().ok_or_else(bad)? == 1,
            };
            entries.insert(name.to_lowercase(), e);
            names.push(name);
        }
        Ok(Some(Pack { file: Mutex::new(f), start, entries, names }))
    }

    /// File names in the pack ('/' separated, as packed).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(&name.to_lowercase())
    }

    /// Contents of one packed file (`name` is case-insensitive, '/' separated).
    pub fn read(&self, name: &str) -> Option<Result<Vec<u8>, String>> {
        let e = self.entries.get(&name.to_lowercase())?;
        let mut data = vec![0u8; e.stored as usize];
        let res = {
            let mut f = self.file.lock().unwrap_or_else(|p| p.into_inner());
            f.seek(SeekFrom::Start(self.start + e.offset)).and_then(|_| f.read_exact(&mut data))
        };
        if let Err(e) = res {
            return Some(Err(format!("cannot read '{name}' from the game pack: {e}")));
        }
        if !e.deflate {
            return Some(Ok(data));
        }
        Some(
            miniz_oxide::inflate::decompress_to_vec_with_limit(&data, e.size as usize)
                .map_err(|_| format!("'{name}' is damaged in the game pack")),
        )
    }
}

/// `(pack_start, index_start)` from the footer, if the file ends with one.
fn footer(f: &mut File) -> std::io::Result<Option<(u64, u64)>> {
    let len = f.metadata()?.len();
    if len < FOOTER {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(len - FOOTER))?;
    let mut b = [0u8; FOOTER as usize];
    f.read_exact(&mut b)?;
    if &b[16..] != MAGIC {
        return Ok(None);
    }
    let start = u64::from_le_bytes(b[0..8].try_into().unwrap());
    let index = u64::from_le_bytes(b[8..16].try_into().unwrap());
    if start > index || index > len - FOOTER {
        return Ok(None);
    }
    Ok(Some((start, index)))
}

/// Size of `exe` without an appended pack (to rebuild from an executable that already has one).
pub fn strip_pack(exe: &[u8]) -> &[u8] {
    let n = exe.len();
    if n < FOOTER as usize || &exe[n - 8..] != MAGIC {
        return exe;
    }
    let start = u64::from_le_bytes(exe[n - 24..n - 16].try_into().unwrap()) as usize;
    if start <= n { &exe[..start] } else { exe }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.bytes(2).map(|b| u16::from_le_bytes(b.try_into().unwrap()))
    }
    fn u32(&mut self) -> Option<u32> {
        self.bytes(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }
    fn u64(&mut self) -> Option<u64> {
        self.bytes(8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }
}

/// Builds a pack: add files, then [`PackWriter::finish`] appends it to an executable image.
#[derive(Default)]
pub struct PackWriter {
    data: Vec<u8>,
    index: Vec<u8>,
    count: u32,
    /// Sum of original file sizes.
    pub raw_bytes: u64,
}

/// Already compressed formats: deflate would only cost time.
const STORED_EXT: &[&str] = &["png", "jpg", "jpeg", "ogg", "mp3", "flac", "webp", "zip", "gz", "ktx2", "basis"];

impl PackWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one file under `name` ('/' separated, relative to the game folder).
    pub fn add(&mut self, name: &str, bytes: &[u8]) {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let packed = if STORED_EXT.contains(&ext.as_str()) || bytes.len() < 64 {
            None
        } else {
            let c = miniz_oxide::deflate::compress_to_vec(bytes, 6);
            (c.len() < bytes.len() - bytes.len() / 20).then_some(c)
        };
        let offset = self.data.len() as u64;
        let deflate = packed.is_some();
        let stored = packed.as_deref().unwrap_or(bytes);
        self.data.extend_from_slice(stored);
        self.index.extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.index.extend_from_slice(name.as_bytes());
        self.index.extend_from_slice(&offset.to_le_bytes());
        self.index.extend_from_slice(&(stored.len() as u64).to_le_bytes());
        self.index.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.index.push(deflate as u8);
        self.count += 1;
        self.raw_bytes += bytes.len() as u64;
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    /// `exe` (any old pack removed) + this pack.
    pub fn finish(self, exe: &[u8]) -> Vec<u8> {
        let exe = strip_pack(exe);
        let start = exe.len() as u64;
        let mut out = Vec::with_capacity(exe.len() + self.data.len() + self.index.len() + 32);
        out.extend_from_slice(exe);
        out.extend_from_slice(&self.data);
        let index_start = out.len() as u64;
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.index);
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&index_start.to_le_bytes());
        out.extend_from_slice(MAGIC);
        out
    }
}

/// Serves files under `root` from `pack` for the rest of the process (once; later calls are ignored).
pub fn mount(root: &Path, pack: Pack) {
    let root_key = key_of(root);
    let _ = MOUNTED.set(Mounted { root: root.to_path_buf(), root_key, pack });
}

/// True when the game runs from a pack embedded in the executable.
pub fn is_packed() -> bool {
    MOUNTED.get().is_some()
}

/// Root folder of the mounted pack (the executable's folder).
pub fn pack_root() -> Option<&'static Path> {
    MOUNTED.get().map(|m| m.root.as_path())
}

/// Lowercase, '/' separated, `.` / `..` resolved.
fn key_of(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut prefix = String::new();
    for c in path.components() {
        match c {
            Component::Prefix(p) => {
                // `\\?\C:` and `C:` are the same folder.
                let s = p.as_os_str().to_string_lossy().to_lowercase();
                prefix = s.trim_start_matches(r"\\?\").to_string();
            }
            Component::RootDir => {}
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(n) => parts.push(n.to_string_lossy().to_lowercase()),
        }
    }
    let body = parts.join("/");
    if prefix.is_empty() { body } else { format!("{prefix}/{body}") }
}

/// The pack name for `path`, if it lies under the mounted root (or is relative).
fn pack_name(m: &Mounted, path: &Path) -> Option<String> {
    if path.is_relative() {
        return Some(key_of(path));
    }
    let k = key_of(path);
    k.strip_prefix(&m.root_key).and_then(|r| r.strip_prefix('/')).map(str::to_string)
}

fn from_pack(path: &Path) -> Option<Result<Vec<u8>, String>> {
    let m = MOUNTED.get()?;
    let name = pack_name(m, path)?;
    m.pack.read(&name)
}

/// Contents of a game file (pack first, then disk).
pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    if let Some(r) = from_pack(path) {
        return r;
    }
    std::fs::read(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))
}

/// A text file, without a UTF-8 BOM (Windows editors like to add one; Luau and TOML reject it).
pub fn read_string(path: &Path) -> Result<String, String> {
    let bytes = read(path)?;
    let s = String::from_utf8(bytes).map_err(|_| format!("'{}' is not UTF-8 text", path.display()))?;
    Ok(match s.strip_prefix('\u{feff}') {
        Some(t) => t.to_string(),
        None => s,
    })
}

/// True if the file exists in the pack or on disk.
pub fn is_file(path: &Path) -> bool {
    if let Some(m) = MOUNTED.get() {
        if pack_name(m, path).is_some_and(|n| m.pack.contains(&n)) {
            return true;
        }
    }
    path.is_file()
}

/// Modification time on disk; `None` for packed files (they never change, so no hot reload).
pub fn modified(path: &Path) -> Option<SystemTime> {
    if let Some(m) = MOUNTED.get() {
        if pack_name(m, path).is_some_and(|n| m.pack.contains(&n)) {
            return None;
        }
    }
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_roundtrip() {
        let mut w = PackWriter::new();
        let text = "print('hi')\n".repeat(50);
        w.add("main.luau", text.as_bytes());
        w.add("src/Player.luau", b"return {}");
        w.add("assets/a.png", &[1, 2, 3]);
        let exe = b"MZ fake exe".to_vec();
        let out = w.finish(&exe);
        // Rebuilding from a packed exe drops the old pack.
        assert_eq!(strip_pack(&out), &exe[..]);
        let dir = std::env::temp_dir().join(format!("spark-vfs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("game.exe");
        std::fs::write(&file, &out).unwrap();
        let pack = Pack::open(&file).unwrap().expect("has a pack");
        assert_eq!(pack.read("main.luau").unwrap().unwrap(), text.as_bytes());
        assert_eq!(pack.read("SRC/player.luau").unwrap().unwrap(), b"return {}");
        assert_eq!(pack.read("assets/a.png").unwrap().unwrap(), vec![1, 2, 3]);
        assert!(pack.read("nope").is_none());
        assert_eq!(pack.names().len(), 3);
        std::fs::write(dir.join("plain.exe"), &exe).unwrap();
        assert!(Pack::open(&dir.join("plain.exe")).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys() {
        assert_eq!(key_of(Path::new("a/./b/../C.png")), "a/c.png");
        if cfg!(windows) {
            assert_eq!(key_of(Path::new(r"\\?\C:\Games\X")), key_of(Path::new(r"C:\games\x")));
        }
    }
}
