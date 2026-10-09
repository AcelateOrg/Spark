//! `game.toml` next to `main.luau`: window title, size, icon, ... (a small TOML subset).
//!
//! ```toml
//! title = "My Game"
//! width = 1280
//! height = 720
//! fullscreen = false
//! icon = "icon.png"
//! splash = true        # "Powered by Spark" intro (off by default)
//! spark = "0.1"        # engine version the game is made for (warning on a mismatch)
//! ```

use std::path::Path;

pub const FILE: &str = "game.toml";
const KEYS: &str = "title, width, height, fullscreen, vsync, icon, show_fps, splash, save_name, build_name, exclude, spark";

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub title: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub vsync: bool,
    /// Window icon (PNG/JPEG path relative to the game folder).
    pub icon: Option<String>,
    /// FPS counter in the title bar (default: only when no title is set).
    pub show_fps: Option<bool>,
    /// "Powered by Spark" splash before the game starts (default off).
    pub splash: bool,
    /// Folder name for save data (default: the title).
    pub save_name: Option<String>,
    /// Executable / folder / zip name for `spark build` (default: the title).
    pub build_name: Option<String>,
    /// Extra files / folders that `spark build` leaves out.
    pub exclude: Vec<String>,
    /// Engine version the game was made for, e.g. "0.1".
    pub spark: Option<String>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            title: None,
            width: 1280,
            height: 720,
            fullscreen: false,
            vsync: true,
            icon: None,
            show_fps: None,
            splash: false,
            save_name: None,
            build_name: None,
            exclude: Vec::new(),
            spark: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Str(String),
    Num(f64),
    Bool(bool),
    List(Vec<String>),
}

impl Manifest {
    /// Reads `dir/game.toml` (missing file = defaults).
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(FILE);
        if !engine::vfs::is_file(&path) {
            return Ok(Self::default());
        }
        let text = engine::vfs::read_string(&path)?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut m = Self::default();
        for (n, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim().to_string();
            if line.is_empty() || line.starts_with('[') {
                continue;
            }
            let at = |e: String| format!("line {}: {e}", n + 1);
            let (k, v) = line.split_once('=').ok_or_else(|| at(format!("expected key = value, got '{line}'")))?;
            let key = k.trim();
            let val = parse_value(v.trim()).map_err(at)?;
            let num = |v: &Val| match v {
                Val::Num(x) if *x >= 1.0 && x.fract() == 0.0 => Ok(*x as u32),
                _ => Err(at(format!("{key} must be a whole number"))),
            };
            let boolean = |v: &Val| match v {
                Val::Bool(b) => Ok(*b),
                _ => Err(at(format!("{key} must be true or false"))),
            };
            let string = |v: &Val| match v {
                Val::Str(s) => Ok(s.clone()),
                _ => Err(at(format!("{key} must be a \"string\""))),
            };
            match key {
                "title" | "name" => m.title = Some(string(&val)?),
                "width" => m.width = num(&val)?,
                "height" => m.height = num(&val)?,
                "fullscreen" => m.fullscreen = boolean(&val)?,
                "vsync" => m.vsync = boolean(&val)?,
                "icon" => m.icon = Some(string(&val)?),
                "show_fps" => m.show_fps = Some(boolean(&val)?),
                "splash" => m.splash = boolean(&val)?,
                "save_name" => m.save_name = Some(string(&val)?),
                "build_name" => m.build_name = Some(string(&val)?),
                "spark" => {
                    let v = string(&val)?;
                    if parse_version(&v).is_none() {
                        return Err(at(format!("spark must be a version like \"0.1\" or \"0.1.2\", got \"{v}\"")));
                    }
                    m.spark = Some(v);
                }
                "exclude" => match val {
                    Val::List(l) => m.exclude = l,
                    _ => return Err(at("exclude must be a list like [\"tools\", \"notes.txt\"]".into())),
                },
                _ => return Err(at(format!("unknown key '{key}' (allowed: {KEYS})"))),
            }
        }
        Ok(m)
    }

    /// A warning when the game was made for an incompatible engine version (`engine` = this engine, "0.1.0").
    /// Compatible = same major version, or for 0.x the same minor version, and not older than the game wants.
    pub fn version_warning(&self, engine: &str) -> Option<String> {
        let want_s = self.spark.as_deref()?;
        let want = parse_version(want_s)?;
        let have = parse_version(engine)?;
        let same_line = if want.0 == 0 { have.0 == 0 && have.1 == want.1 } else { have.0 == want.0 };
        if same_line && have >= want {
            return None;
        }
        let fix = if have < want {
            "update Spark: spark update".to_string()
        } else {
            format!("run `spark docs .`, adapt the game, then set spark = \"{}.{}\" in game.toml", have.0, have.1)
        };
        Some(format!("this game is made for Spark {want_s}, this is Spark {engine}: the API may differ ({fix})"))
    }
}

/// "0.1" / "0.1.2" / "1" -> (major, minor, patch).
fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim().trim_start_matches('v').split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next().map_or(Some(0), |p| p.parse().ok())?;
    let patch = it.next().map_or(Some(0), |p| p.parse().ok())?;
    if it.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        match c {
            '"' if prev != '\\' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
        prev = c;
    }
    line
}

fn parse_string(v: &str) -> Result<String, String> {
    let inner = v
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .ok_or_else(|| format!("bad string {v}"))?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

fn parse_value(v: &str) -> Result<Val, String> {
    if v.starts_with('"') || v.starts_with('\'') {
        return parse_string(v).map(Val::Str);
    }
    if let Some(inner) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return inner
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(parse_string)
            .collect::<Result<Vec<_>, _>>()
            .map(Val::List);
    }
    match v {
        "true" => Ok(Val::Bool(true)),
        "false" => Ok(Val::Bool(false)),
        _ => v.replace('_', "").parse::<f64>().map(Val::Num).map_err(|_| format!("cannot read value '{v}' (strings need \"quotes\")")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest() {
        let m = Manifest::parse(
            "# my game\ntitle = \"Ночь # 1\"  # comment\nwidth = 1600\nheight=900\nfullscreen = true\nsplash = true\nicon = 'icon.png'\nexclude = [\"tools\", \"a b\"]\n",
        )
        .unwrap();
        assert_eq!(m.title.as_deref(), Some("Ночь # 1"));
        assert_eq!((m.width, m.height), (1600, 900));
        assert!(m.fullscreen && m.vsync && m.splash);
        assert!(!Manifest::default().splash);
        assert_eq!(m.icon.as_deref(), Some("icon.png"));
        assert_eq!(m.exclude, vec!["tools".to_string(), "a b".to_string()]);
    }

    #[test]
    fn rejects_unknown_keys() {
        let e = Manifest::parse("titel = \"x\"").unwrap_err();
        assert!(e.contains("unknown key 'titel'"), "{e}");
        assert!(Manifest::parse("width = \"big\"").is_err());
        assert!(Manifest::parse("spark = \"latest\"").is_err());
    }

    #[test]
    fn engine_version_check() {
        let game = |v: &str| Manifest::parse(&format!("spark = \"{v}\"")).unwrap();
        assert_eq!(Manifest::default().version_warning("0.1.0"), None);
        assert_eq!(game("0.1").version_warning("0.1.0"), None);
        assert_eq!(game("0.1").version_warning("0.1.7"), None);
        assert_eq!(game("0.1.2").version_warning("0.1.3"), None);
        assert!(game("0.1.2").version_warning("0.1.1").unwrap().contains("spark update"));
        assert!(game("0.1").version_warning("0.2.0").unwrap().contains("spark = \"0.2\""));
        assert!(game("0.2").version_warning("0.1.0").is_some());
        assert_eq!(game("1.2").version_warning("1.4.0"), None);
        assert!(game("1.2").version_warning("2.0.0").is_some());
    }
}
