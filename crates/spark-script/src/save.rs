//! `save` global: tiny persistent key/value storage (JSON file in the user's data folder).
//!
//! ```lua
//! save.set("best", 42)
//! local best = save.get("best", 0)
//! ```

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use mlua::{Lua, Table, Value};
use serde_json::{Map, Number, Value as Json};

use crate::convert::*;

/// Where the save file lives (`None` = in memory only, e.g. headless runs). Set by `ScriptGame`.
#[derive(Clone, Default)]
pub(crate) struct SavePath(pub Option<PathBuf>);

struct Store {
    path: Option<PathBuf>,
    data: RefCell<Map<String, Json>>,
    /// Changed since the last write. `save.set` in a loop costs one write per frame, not per call.
    dirty: std::cell::Cell<bool>,
}

impl Drop for Store {
    fn drop(&mut self) {
        if let Err(e) = write_file(self) {
            log::error!("{e}");
        }
    }
}

/// `%APPDATA%/SparkGames/<game>/save.json` (Windows), `~/Library/Application Support/SparkGames/<game>/save.json`
/// (macOS) or `$XDG_DATA_HOME` / `~/.local/share/SparkGames/<game>/save.json` (Linux).
pub fn default_save_path(game: &str) -> Option<PathBuf> {
    let clean: String = game
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' })
        .collect();
    let clean = clean.trim().trim_matches('.').to_string();
    let clean = if clean.is_empty() { "game".to_string() } else { clean };
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| Path::new(&h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))
    }?;
    Some(base.join("SparkGames").join(clean).join("save.json"))
}

fn load(path: &Path) -> Map<String, Json> {
    let Ok(text) = std::fs::read_to_string(path) else { return Map::new() };
    match serde_json::from_str::<Json>(&text) {
        Ok(Json::Object(m)) => m,
        _ => {
            // Keep the damaged file for recovery instead of overwriting it with the next save.
            let bak = path.with_extension("json.bak");
            let _ = std::fs::rename(path, &bak);
            log::warn!("save file '{}' is damaged; moved to '{}', starting fresh", path.display(), bak.display());
            Map::new()
        }
    }
}

/// Marks the store changed; the file is written by [`flush`] (end of frame) or on exit.
fn persist(store: &Store) -> LuaResult<()> {
    store.dirty.set(true);
    Ok(())
}

/// Writes the save file if something changed (atomic: temp file + rename).
pub(crate) fn flush(lua: &Lua) -> LuaResult<()> {
    match lua.app_data_ref::<Store>() {
        Some(s) => write_file(&s).map_err(rt),
        None => Ok(()),
    }
}

fn write_file(store: &Store) -> Result<(), String> {
    if !store.dirty.replace(false) {
        return Ok(());
    }
    let Some(path) = &store.path else { return Ok(()) };
    let text = serde_json::to_string_pretty(&*store.data.borrow()).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("save: cannot create '{}': {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("save: cannot write '{}': {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("save: cannot write '{}': {e}", path.display()))
}

fn to_json(v: &Value, what: &str, depth: u32) -> LuaResult<Json> {
    if depth > 32 {
        return Err(rt(format!("{what}: table is nested too deeply (cycle?)")));
    }
    Ok(match v {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(i) => Json::Number((*i).into()),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 9.0e15 {
                Json::Number((*n as i64).into())
            } else {
                Json::Number(Number::from_f64(*n).ok_or_else(|| rt(format!("{what}: cannot save NaN / infinity")))?)
            }
        }
        Value::String(s) => Json::String(s.to_string_lossy()),
        Value::Vector(v) => {
            let mut m = Map::new();
            m.insert("$vec".into(), Json::Array(vec![v.x().into(), v.y().into(), v.z().into()]));
            Json::Object(m)
        }
        Value::Table(t) => {
            let len = t.raw_len();
            let mut count = 0usize;
            for pair in t.clone().pairs::<Value, Value>() {
                pair?;
                count += 1;
            }
            if len > 0 && count == len {
                let mut a = Vec::with_capacity(len);
                for i in 1..=len {
                    a.push(to_json(&t.raw_get::<Value>(i)?, what, depth + 1)?);
                }
                Json::Array(a)
            } else {
                let mut m = Map::new();
                for pair in t.clone().pairs::<Value, Value>() {
                    let (k, val) = pair?;
                    let key = match &k {
                        Value::String(s) => s.to_string_lossy(),
                        Value::Integer(i) => i.to_string(),
                        Value::Number(n) => n.to_string(),
                        _ => return Err(rt(format!("{what}: table keys must be strings or numbers, got {}", k.type_name()))),
                    };
                    m.insert(key, to_json(&val, what, depth + 1)?);
                }
                Json::Object(m)
            }
        }
        other => {
            return Err(rt(format!(
                "{what}: cannot save a {} (only numbers, strings, booleans, vectors and tables of them)",
                other.type_name()
            )));
        }
    })
}

fn to_lua(lua: &Lua, j: &Json) -> LuaResult<Value> {
    Ok(match j {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) if i.abs() < (1 << 53) => Value::Number(i as f64),
            _ => Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => Value::String(lua.create_string(s)?),
        Json::Array(a) => {
            let t = lua.create_table_with_capacity(a.len(), 0)?;
            for (i, v) in a.iter().enumerate() {
                t.raw_set(i + 1, to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
        Json::Object(m) => {
            if let (1, Some(Json::Array(v))) = (m.len(), m.get("$vec")) {
                let c = |i: usize| v.get(i).and_then(Json::as_f64).unwrap_or(0.0) as f32;
                return Ok(Value::Vector(mlua::Vector::new(c(0), c(1), c(2))));
            }
            let t = lua.create_table()?;
            for (k, v) in m {
                t.raw_set(k.as_str(), to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
    })
}

fn store(lua: &Lua) -> LuaResult<mlua::AppDataRef<'_, Store>> {
    lua.app_data_ref::<Store>().ok_or_else(|| rt("save: storage is not available"))
}

pub(crate) fn install(lua: &Lua, g: &Table) -> LuaResult<()> {
    let path = lua.app_data_ref::<SavePath>().map(|p| p.0.clone()).unwrap_or_default();
    let data = path.as_deref().map(load).unwrap_or_default();
    lua.set_app_data(Store { path, data: RefCell::new(data), dirty: Default::default() });

    let save = lua.create_table()?;
    save.set(
        "get",
        lua.create_function(|lua, (key, default): (String, Value)| {
            let s = store(lua)?;
            let v = s.data.borrow().get(&key).cloned();
            match v {
                Some(j) if !j.is_null() => to_lua(lua, &j),
                _ => Ok(default),
            }
        })?,
    )?;
    save.set(
        "set",
        lua.create_function(|lua, (key, value): (String, Value)| {
            let j = to_json(&value, &format!("save.set(\"{key}\")"), 0)?;
            let s = store(lua)?;
            if j.is_null() {
                s.data.borrow_mut().remove(&key);
            } else {
                s.data.borrow_mut().insert(key, j);
            }
            persist(&s)
        })?,
    )?;
    save.set("flush", lua.create_function(|lua, ()| flush(lua))?)?;
    save.set("has", lua.create_function(|lua, key: String| Ok(store(lua)?.data.borrow().contains_key(&key)))?)?;
    save.set(
        "delete",
        lua.create_function(|lua, key: String| {
            let s = store(lua)?;
            let had = s.data.borrow_mut().remove(&key).is_some();
            persist(&s)?;
            Ok(had)
        })?,
    )?;
    save.set(
        "clear",
        lua.create_function(|lua, ()| {
            let s = store(lua)?;
            s.data.borrow_mut().clear();
            persist(&s)
        })?,
    )?;
    save.set(
        "keys",
        lua.create_function(|lua, ()| Ok(store(lua)?.data.borrow().keys().cloned().collect::<Vec<_>>()))?,
    )?;
    save.set(
        "path",
        lua.create_function(|lua, ()| Ok(store(lua)?.path.as_ref().map(|p| p.display().to_string())))?,
    )?;
    g.set("save", save)?;
    Ok(())
}
