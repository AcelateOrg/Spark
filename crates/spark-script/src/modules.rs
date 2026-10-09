//! `require`: a game split into files.
//!
//! ```lua
//! local player = require("./src/player")     -- next to this file (.luau added, or src/player/init.luau)
//! local hud = require("../ui/hud")           -- one folder up
//! local config = require("@game/config")     -- from the game folder (where main.luau is)
//! ```
//! A module runs once and its return value is cached; every file is watched for hot reload.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

use mlua::{Lua, Table, Value};

use crate::convert::{LuaResult, rt};

pub(crate) struct Modules {
    /// Game folder (`@game/`).
    pub root: PathBuf,
    /// Every loaded module file with its modification time at load (hot reload).
    pub files: Vec<(PathBuf, Option<SystemTime>)>,
    /// Modules being loaded right now (cycle detection).
    loading: Vec<String>,
}

pub(crate) type SharedModules = Rc<RefCell<Modules>>;

pub(crate) fn new(root: PathBuf) -> SharedModules {
    Rc::new(RefCell::new(Modules { root, files: Vec::new(), loading: Vec::new() }))
}

const CACHE: &str = "spark.modules";

pub(crate) fn install(lua: &Lua, g: &Table, modules: SharedModules) -> LuaResult<()> {
    lua.set_named_registry_value(CACHE, lua.create_table()?)?;
    lua.set_app_data(modules);
    g.set("require", lua.create_function(require)?)?;
    Ok(())
}

/// Game-relative file of the Luau code that called `require` (chunk names are `@relative/path.luau`).
fn caller(lua: &Lua) -> String {
    for level in 0..10 {
        let src = lua.inspect_stack(level, |d| d.source().source.map(|s| s.into_owned()));
        match src {
            None => break,
            Some(Some(s)) => {
                if let Some(rel) = s.strip_prefix('@') {
                    return rel.replace('\\', "/");
                }
            }
            Some(None) => {}
        }
    }
    "main.luau".into()
}

/// `spec` as a game-relative path ('/' separated), without the extension.
fn resolve_spec(from: &str, spec: &str) -> Result<String, String> {
    let (base, rest) = if let Some(r) = spec.strip_prefix("@game/") {
        (String::new(), r)
    } else if spec.starts_with("./") || spec.starts_with("../") {
        (from.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default(), spec)
    } else {
        let hint = spec.trim_start_matches('/').trim_end_matches(".luau");
        return Err(format!(
            "require(\"{spec}\"): a path starts with \"./\" (next to this file), \"../\" (folder up) or \"@game/\" \
             (the game folder), e.g. require(\"./{hint}\")"
        ));
    };
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for p in rest.split(['/', '\\']) {
        match p {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("require(\"{spec}\"): goes above the game folder"));
                }
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(format!("require(\"{spec}\"): needs a file name"));
    }
    Ok(parts.join("/"))
}

fn find_file(root: &std::path::Path, rel: &str) -> Result<String, Vec<String>> {
    let candidates: Vec<String> = if rel.ends_with(".luau") || rel.ends_with(".lua") {
        vec![rel.to_string()]
    } else {
        vec![format!("{rel}.luau"), format!("{rel}.lua"), format!("{rel}/init.luau")]
    };
    for c in &candidates {
        if spark_core::vfs::is_file(&root.join(c)) {
            return Ok(c.clone());
        }
    }
    Err(candidates)
}

fn require(lua: &Lua, spec: String) -> LuaResult<Value> {
    let from = caller(lua);
    let modules = lua.app_data_ref::<SharedModules>().ok_or_else(|| rt("require is not available here"))?.clone();
    let root = modules.borrow().root.clone();
    let rel = resolve_spec(&from, &spec).map_err(rt)?;
    let rel = find_file(&root, &rel).map_err(|tried| {
        rt(format!("require(\"{spec}\") from {from}: no such module (looked for {})", tried.join(", ")))
    })?;
    let cache: Table = lua.named_registry_value(CACHE)?;
    let cached: Value = cache.raw_get(rel.as_str())?;
    if !cached.is_nil() {
        return Ok(cached);
    }
    {
        let m = modules.borrow();
        if let Some(i) = m.loading.iter().position(|x| *x == rel) {
            let mut chain: Vec<&str> = m.loading[i..].iter().map(String::as_str).collect();
            chain.push(&rel);
            return Err(rt(format!(
                "require cycle: {} - modules can't require each other while loading; move the shared part into \
                 a third module, or call require inside a function",
                chain.join(" -> ")
            )));
        }
    }
    let path = root.join(&rel);
    let source = spark_core::vfs::read_string(&path).map_err(rt)?;
    // Watched before it runs: saving a fix for a broken module reloads the game.
    let mtime = spark_core::vfs::modified(&path);
    modules.borrow_mut().files.push((path, mtime));
    modules.borrow_mut().loading.push(rel.clone());
    let result = lua.load(source.as_str()).set_name(format!("@{rel}")).into_function().and_then(|f| f.call::<Value>(()));
    modules.borrow_mut().loading.pop();
    let value = match result? {
        Value::Nil => Value::Boolean(true),
        v => v,
    };
    cache.raw_set(rel.as_str(), value.clone())?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::resolve_spec;

    #[test]
    fn specs() {
        assert_eq!(resolve_spec("main.luau", "./src/player").unwrap(), "src/player");
        assert_eq!(resolve_spec("src/player.luau", "./input").unwrap(), "src/input");
        assert_eq!(resolve_spec("src/ui/hud.luau", "../world").unwrap(), "src/world");
        assert_eq!(resolve_spec("src/ui/hud.luau", "@game/config").unwrap(), "config");
        assert!(resolve_spec("main.luau", "player").unwrap_err().contains("require(\"./player\")"));
        assert!(resolve_spec("main.luau", "../x").is_err());
    }
}
