//! Scenario tests (`spark test`): a test file runs as a task next to the game and drives it like a
//! player (keys, mouse, waiting), checks the state and records snapshots of the world.
//!
//! ```lua
//! -- tests/bridge.luau
//! test.step("pull the lever")
//! test.wait_until(function() return find("lever") ~= nil end, 5, "lever spawned")
//! test.tap("e")
//! test.wait(2)
//! test.eq(save.get("bridge_down"), true, "bridge is down")
//! test.snapshot("after_bridge", { flags = save.get("flags") })
//! ```
//!
//! The `test` global only exists in test runs. Input is queued here and fed to the engine by the
//! runner before the next frame, exactly like real input.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;

use mlua::{Lua, Table, Value};
use spark_core::{Key, MouseButton, Vec3, World};

use crate::convert::*;

/// Input requested by a test, applied by the runner before the next frame.
#[derive(Clone, Debug, PartialEq)]
pub enum TestInput {
    Key(Key, bool),
    Button(MouseButton, bool),
    Move(f32, f32),
    Text(String),
}

impl TestInput {
    pub fn apply(&self, world: &mut World) {
        let i = &mut world.input;
        match self {
            TestInput::Key(k, d) => i.key_event(*k, *d),
            TestInput::Button(b, d) => i.mouse_button_event(*b, *d),
            TestInput::Move(x, y) => i.mouse_moved(spark_core::Vec2::new(*x, *y)),
            TestInput::Text(t) => i.text_event(t),
        }
    }
}

/// State shared between the `test` global and [`crate::ScriptGame`].
#[derive(Default)]
pub struct TestState {
    pub input: Vec<TestInput>,
    /// `(name, snapshot text)` in the order they were taken.
    pub snapshots: Vec<(String, String)>,
    /// Name given with `test.step(...)`, shown when the test fails.
    pub step: Option<String>,
}

pub(crate) type SharedTest = Rc<RefCell<TestState>>;

/// Marks failures of `test.*` checks (the runner tells them apart from game errors).
pub const FAIL_PREFIX: &str = "test failed: ";

const PRELUDE: &str = r#"
local t = test
local function fail(msg, level) error("test failed: " .. msg, (level or 1) + 1) end
local function show(v)
    if type(v) == "string" then return string.format("%q", v) end
    return tostring(v)
end
function t.fail(msg) fail(tostring(msg or "failed"), 2) end
function t.expect(cond, msg)
    if not cond then fail(tostring(msg or "expectation failed"), 2) end
    return cond
end
function t.eq(actual, expected, msg)
    if actual ~= expected then
        fail((msg and (msg .. ": ") or "") .. "expected " .. show(expected) .. ", got " .. show(actual), 2)
    end
end
function t.near(actual, expected, eps, msg)
    eps = eps or 0.01
    local d
    if typeof(actual) == "vector" then
        d = length(actual - expected)
    else
        d = math.abs(actual - expected)
    end
    if d > eps then
        fail((msg and (msg .. ": ") or "") .. "expected " .. show(expected) .. " +- " .. eps .. ", got " .. show(actual), 2)
    end
end
function t.wait(s) return wait(s) end
function t.frames(n) return wait_frames(n) end
function t.wait_until(cond, timeout, msg)
    local ok = wait_until(cond, timeout or 10)
    if not ok then fail("timed out after " .. (timeout or 10) .. " s waiting for " .. tostring(msg or "condition"), 2) end
end
function t.wait_event(name, timeout)
    local r = table.pack(wait_event(name, timeout or 10))
    if r[1] == nil and r.n <= 1 then fail("timed out after " .. (timeout or 10) .. " s waiting for event '" .. name .. "'", 2) end
    return table.unpack(r, 1, r.n)
end
function t.tap(key) t.press(key) wait_frames(1) t.release(key) wait_frames(1) end
function t.hold(key, seconds) t.press(key) wait(seconds or 0.5) t.release(key) wait_frames(1) end
function t.click(x, y, button)
    button = button or "left"
    if x then t.mouse(x, y) wait_frames(1) end
    t.button(button, true) wait_frames(1) t.button(button, false) wait_frames(1)
end
function t.type(text) t.text(text) wait_frames(1) end
function t.find(name, timeout)
    local o
    t.wait_until(function() o = find(name) return o ~= nil end, timeout or 5, "object '" .. name .. "'")
    return o
end
"#;

fn key(name: &str, what: &str) -> LuaResult<Key> {
    Key::from_name(name).ok_or_else(|| rt(format!("{what}: unknown key '{name}' (names like \"e\", \"space\", \"left\", \"f1\")")))
}

fn state(lua: &Lua) -> LuaResult<SharedTest> {
    Ok(lua.app_data_ref::<SharedTest>().ok_or_else(|| rt("test: not a test run"))?.clone())
}

fn push(lua: &Lua, i: TestInput) -> LuaResult<()> {
    state(lua)?.borrow_mut().input.push(i);
    Ok(())
}

/// Installs the `test` global (test runs only).
pub(crate) fn install(lua: &Lua, shared: SharedTest) -> LuaResult<()> {
    lua.set_app_data(shared);
    let t = lua.create_table()?;
    t.set("press", lua.create_function(|lua, k: String| push(lua, TestInput::Key(key(&k, "test.press")?, true)))?)?;
    t.set("release", lua.create_function(|lua, k: String| push(lua, TestInput::Key(key(&k, "test.release")?, false)))?)?;
    t.set(
        "button",
        lua.create_function(|lua, (b, down): (String, bool)| {
            let btn = MouseButton::from_name(&b).ok_or_else(|| rt(format!("test.button: unknown mouse button '{b}' (left, right, middle)")))?;
            push(lua, TestInput::Button(btn, down))
        })?,
    )?;
    t.set("mouse", lua.create_function(|lua, (x, y): (f32, f32)| push(lua, TestInput::Move(x, y)))?)?;
    t.set("text", lua.create_function(|lua, s: String| push(lua, TestInput::Text(s)))?)?;
    t.set(
        "step",
        lua.create_function(|lua, name: String| {
            log::info!("test step: {name}");
            state(lua)?.borrow_mut().step = Some(name);
            Ok(())
        })?,
    )?;
    t.set(
        "snapshot",
        lua.create_function(|lua, (name, extra): (String, Value)| {
            let mut text = with(lua, |w| Ok(world_snapshot(w)))?;
            if !extra.is_nil() {
                flatten(&mut text, "state", &extra, 0)?;
            }
            let st = state(lua)?;
            let mut st = st.borrow_mut();
            if st.snapshots.iter().any(|(n, _)| *n == name) {
                return Err(rt(format!("test.snapshot: name '{name}' is used twice in this test")));
            }
            st.snapshots.push((name, text));
            Ok(())
        })?,
    )?;
    lua.globals().set("test", t)?;
    lua.load(PRELUDE).set_name("=test").exec()?;
    Ok(())
}

fn num(n: f32) -> String {
    let s = format!("{n:.2}");
    if s == "-0.00" { "0.00".into() } else { s }
}

fn vec3(v: Vec3) -> String {
    format!("({}, {}, {})", num(v.x), num(v.y), num(v.z))
}

/// The world as `key = value` lines (sorted, rounded): named objects in full, unnamed ones counted.
pub fn world_snapshot(w: &World) -> String {
    let mut lines = Vec::new();
    let c = &w.scene.camera;
    lines.push(format!("camera = pos {} forward {}", vec3(c.position), vec3(c.forward())));
    let mut unnamed = 0usize;
    let mut named: Vec<(String, String)> = Vec::new();
    for (_, o) in w.scene.iter() {
        if o.name.is_empty() {
            unnamed += 1;
            continue;
        }
        let e = o.euler();
        let mut v = format!(
            "pos {} rot {} scale {}",
            vec3(o.position),
            vec3(Vec3::new(e.x.to_degrees(), e.y.to_degrees(), e.z.to_degrees())),
            vec3(o.scale)
        );
        if let Some(p) = o.parent().and_then(|p| w.scene.get(p)) {
            let _ = write!(v, " parent {:?}", p.name);
        }
        if let Some(b) = &o.body {
            let _ = write!(v, " body {}", b.kind.name());
        }
        if !o.visible {
            v.push_str(" hidden");
        }
        named.push((o.name.clone(), v));
    }
    named.sort();
    let mut prev: Option<(String, usize)> = None;
    for (name, v) in named {
        // Same name several times: obj "coin", obj "coin"[2], ...
        let n = match &prev {
            Some((p, n)) if *p == name => n + 1,
            _ => 1,
        };
        let key = if n == 1 { format!("obj {name:?}") } else { format!("obj {name:?}[{n}]") };
        lines.push(format!("{key} = {v}"));
        prev = Some((name, n));
    }
    lines.push(format!("objects.unnamed = {unnamed}"));
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Appends `prefix.key = value` lines for a Lua value (tables recursively, keys sorted).
fn flatten(out: &mut String, prefix: &str, v: &Value, depth: usize) -> LuaResult<()> {
    if depth > 12 {
        return Err(rt(format!("test.snapshot: '{prefix}' is nested too deep (a cycle?)")));
    }
    match v {
        Value::Table(t) => flatten_table(out, prefix, t, depth),
        Value::Nil => Ok(()),
        Value::Boolean(b) => Ok(writeln!(out, "{prefix} = {b}").unwrap_or(())),
        Value::Integer(i) => Ok(writeln!(out, "{prefix} = {i}").unwrap_or(())),
        Value::Number(n) => {
            let s = if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", *n as i64) } else { num(*n as f32) };
            Ok(writeln!(out, "{prefix} = {s}").unwrap_or(()))
        }
        Value::String(s) => Ok(writeln!(out, "{prefix} = {:?}", s.to_string_lossy()).unwrap_or(())),
        Value::Vector(x) => Ok(writeln!(out, "{prefix} = {}", vec3(Vec3::new(x.x(), x.y(), x.z()))).unwrap_or(())),
        other => {
            // Objects and other userdata: their tostring() (e.g. the object name).
            let s = match other {
                Value::UserData(_) => other.to_string().unwrap_or_else(|_| "<userdata>".into()),
                _ => format!("<{}>", other.type_name()),
            };
            Ok(writeln!(out, "{prefix} = {s}").unwrap_or(()))
        }
    }
}

fn flatten_table(out: &mut String, prefix: &str, t: &Table, depth: usize) -> LuaResult<()> {
    let mut entries: Vec<(String, Value)> = Vec::new();
    for pair in t.clone().pairs::<Value, Value>() {
        let (k, v) = pair?;
        let key = match &k {
            Value::String(s) => s.to_string_lossy(),
            Value::Integer(i) => format!("{i:08}"),
            Value::Number(n) => format!("{n}"),
            other => format!("<{}>", other.type_name()),
        };
        entries.push((key, v));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    if entries.is_empty() {
        let _ = writeln!(out, "{prefix} = {{}}");
    }
    for (k, v) in entries {
        // Integer keys were zero-padded for sorting only.
        let shown = if k.len() == 8 && k.bytes().all(|b| b.is_ascii_digit()) { k.trim_start_matches('0').to_string() } else { k };
        let shown = if shown.is_empty() { "0".into() } else { shown };
        flatten(out, &format!("{prefix}.{shown}"), &v, depth + 1)?;
    }
    Ok(())
}

/// One line of a snapshot diff.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Added(String, String),
    Removed(String, String),
    Changed(String, String, String),
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Change::Added(k, v) => write!(f, "+ {k} = {v}   (new)"),
            Change::Removed(k, v) => write!(f, "- {k} = {v}   (gone)"),
            Change::Changed(k, a, b) => {
                // Objects: only the fields that changed (`pos (0, 1, 0) -> (0, 3, 0)`).
                let (fa, fb) = (fields(a), fields(b));
                let same_names = fa.len() == fb.len() && fa.iter().zip(&fb).all(|(x, y)| x.0 == y.0);
                if same_names && fa.len() > 1 {
                    let parts: Vec<String> = fa
                        .iter()
                        .zip(&fb)
                        .filter(|(x, y)| x.1 != y.1)
                        .map(|(x, y)| if x.0.is_empty() { format!("{} -> {}", x.1, y.1) } else { format!("{} {} -> {}", x.0, x.1, y.1) })
                        .collect();
                    write!(f, "~ {k}: {}", parts.join(", "))
                } else {
                    write!(f, "~ {k}: {a}  ->  {b}")
                }
            }
        }
    }
}

/// Fields without a value in object lines.
const FLAGS: &[&str] = &["hidden"];

/// `pos (1, 2, 3) rot (0, 0, 0) hidden` -> [("pos", "(1, 2, 3)"), ("rot", "(0, 0, 0)"), ("hidden", "")].
fn fields(v: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut depth = 0i32;
    let mut word = String::new();
    let mut words: Vec<String> = Vec::new();
    for c in v.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if c == ' ' && depth == 0 {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    for w in words {
        let is_name = w.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        match out.last_mut() {
            // `body dynamic`: a word right after a field name is its value (flags like `hidden` stand alone).
            Some(last) if last.1.is_empty() && !last.0.is_empty() && (!is_name || !FLAGS.contains(&last.0.as_str())) => last.1 = w,
            Some(last) if !is_name => {
                last.1.push(' ');
                last.1.push_str(&w);
            }
            _ if is_name => out.push((w, String::new())),
            _ => out.push((String::new(), w)),
        }
    }
    out
}

fn parse_lines(s: &str) -> Vec<(String, String)> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match l.split_once(" = ") {
            Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
            None => (l.trim().to_string(), String::new()),
        })
        .collect()
}

/// Differences between an expected snapshot and the actual one, by key.
pub fn diff(expected: &str, actual: &str) -> Vec<Change> {
    let old = parse_lines(expected);
    let new = parse_lines(actual);
    let old_map: std::collections::HashMap<&str, &str> = old.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let new_map: std::collections::HashMap<&str, &str> = new.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut out = Vec::new();
    for (k, v) in &old {
        match new_map.get(k.as_str()) {
            None => out.push(Change::Removed(k.clone(), v.clone())),
            Some(nv) if nv != v => out.push(Change::Changed(k.clone(), v.clone(), nv.to_string())),
            _ => {}
        }
    }
    for (k, v) in &new {
        if !old_map.contains_key(k.as_str()) {
            out.push(Change::Added(k.clone(), v.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_by_key() {
        let a = "obj \"a\" = pos (0.00, 0.00, 0.00)\nobj \"b\" = pos (1.00, 0.00, 0.00)\nstate.x = 1\n";
        let b = "obj \"a\" = pos (0.00, 0.00, 0.00)\nstate.x = 2\nstate.y = true\n";
        let d = diff(a, b);
        assert_eq!(
            d,
            vec![
                Change::Removed("obj \"b\"".into(), "pos (1.00, 0.00, 0.00)".into()),
                Change::Changed("state.x".into(), "1".into(), "2".into()),
                Change::Added("state.y".into(), "true".into()),
            ]
        );
        assert!(diff(a, a).is_empty());
        let c = Change::Changed("obj \"d\"".into(), "pos (0.00, 1.00, 0.00) rot (0.00, 0.00, 0.00) body dynamic".into(), "pos (0.00, 3.00, 0.00) rot (0.00, 0.00, 0.00) body static".into());
        assert_eq!(c.to_string(), "~ obj \"d\": pos (0.00, 1.00, 0.00) -> (0.00, 3.00, 0.00), body dynamic -> static");
        let c = Change::Changed("state.x".into(), "1".into(), "2".into());
        assert_eq!(c.to_string(), "~ state.x: 1  ->  2");
    }

    #[test]
    fn rounding_hides_noise() {
        assert_eq!(num(-0.0001), "0.00");
        assert_eq!(num(1.005_f32 + 0.0001), "1.01");
    }
}
