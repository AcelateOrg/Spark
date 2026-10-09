//! Script globals for the scheduler and events: after/every/task/wait*/on/once/emit...

use mlua::{Function, Lua, MultiValue, Table, UserData, UserDataFields, UserDataMethods, Value, MetaMethod};
use spark_core::{ListenerId, ObjectId, TimerId};

use crate::convert::*;
use crate::runtime::*;
use crate::types::Obj;

/// `wait*` are tiny Luau wrappers around coroutine.yield; the Rust side decides when to resume.
const PRELUDE: &str = r#"
local yield, yieldable = coroutine.yield, coroutine.isyieldable
local function check(name)
    if not yieldable() then
        error(name .. "() only works inside task(fn) or start(); in update() use after()/every() or start a task", 3)
    end
end
function wait(seconds) check("wait") return yield("time", seconds or 0) end
function wait_frames(n) check("wait_frames") return yield("frames", n or 1) end
function wait_until(condition, timeout) check("wait_until") return yield("until", condition, timeout) end
function wait_event(name, timeout) check("wait_event") return yield("event", name, timeout) end
"#;

#[derive(Default)]
struct Opts {
    owner: Option<ObjectId>,
    times: Option<u32>,
    priority: i32,
    once: bool,
}

fn parse_opts(v: &Value, what: &str, allowed: &[&str]) -> LuaResult<Opts> {
    let mut o = Opts::default();
    match v {
        Value::Nil => {}
        Value::UserData(ud) => {
            o.owner = Some(ud.borrow::<Obj>().map_err(|_| rt(format!("{what}: options must be a table or an owner object")))?.0)
        }
        Value::Number(_) | Value::Integer(_) if allowed.contains(&"times") => o.times = Some(to_num(v, what)?.max(1.0) as u32),
        Value::Table(t) => {
            for pair in t.pairs::<String, Value>() {
                let (k, val) = pair?;
                if !allowed.contains(&k.as_str()) {
                    return Err(rt(format!("{what}: unknown option '{k}' (allowed: {})", allowed.join(", "))));
                }
                let w = format!("{what} option '{k}'");
                match k.as_str() {
                    "owner" => o.owner = crate::types::to_parent(&val, &w)?,
                    "times" => o.times = Some(to_num(&val, &w)?.max(1.0) as u32),
                    "priority" => o.priority = to_num(&val, &w)? as i32,
                    "once" => o.once = to_bool(&val, &w)?,
                    _ => {}
                }
            }
        }
        _ => return Err(rt(format!("{what}: options must be a table like {{ owner = obj }}, got {}", v.type_name()))),
    }
    Ok(o)
}

// ---------------------------------------------------------------- handles

/// Handle returned by after/every/after_frames/next_frame.
#[derive(Clone, Copy)]
pub struct LuaTimer(pub TimerId);

impl UserData for LuaTimer {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("active", |lua, this| Ok(runtime(lua)?.borrow().sched.exists(this.0)));
        f.add_field_method_get("paused", |lua, this| Ok(runtime(lua)?.borrow().sched.is_paused(this.0)));
        f.add_field_method_get("remaining", |lua, this| Ok(runtime(lua)?.borrow().sched.remaining(this.0)));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("cancel", |lua, this, ()| Ok(runtime(lua)?.borrow_mut().sched.cancel(this.0)));
        m.add_method("pause", |lua, this, ()| Ok(runtime(lua)?.borrow_mut().sched.pause(this.0)));
        m.add_method("resume", |lua, this, ()| Ok(runtime(lua)?.borrow_mut().sched.resume(this.0)));
        m.add_meta_method(MetaMethod::ToString, |lua, this, ()| {
            let active = runtime(lua)?.borrow().sched.exists(this.0);
            Ok(format!("Timer(#{}{})", this.0.0, if active { "" } else { " finished" }))
        });
    }
}

/// Handle returned by on/once.
#[derive(Clone, Copy)]
pub struct LuaListener(pub ListenerId);

impl UserData for LuaListener {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("active", |lua, this| Ok(runtime(lua)?.borrow().bus.is_active(this.0)));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("off", |lua, this, ()| Ok(runtime(lua)?.borrow_mut().bus.off(this.0)));
        m.add_meta_method(MetaMethod::ToString, |lua, this, ()| {
            let r = runtime(lua)?;
            let r = r.borrow();
            Ok(match r.bus.event_of(this.0) {
                Some(n) => format!("Listener(\"{}\")", n.split('@').next().unwrap_or(n)),
                None => "Listener(off)".to_string(),
            })
        });
    }
}

impl UserData for LuaTask {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("status", |_, this| Ok(this.status.get().name()));
        f.add_field_method_get("alive", |_, this| Ok(matches!(this.status.get(), TaskStatus::Running | TaskStatus::Waiting)));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("cancel", |lua, this, ()| cancel_task(lua, this.id));
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("Task(#{} {})", this.id, this.status.get().name())));
    }
}

// ---------------------------------------------------------------- shared helpers (also used by Object methods)

pub(crate) fn listen(lua: &Lua, event: &str, f: Function, opts: &Value, once: bool, scope: Option<ObjectId>) -> LuaResult<LuaListener> {
    let what = if once { "once" } else { "on" };
    let o = parse_opts(opts, what, &["priority", "owner", "once"])?;
    let owner = o.owner.or(scope);
    if owner.is_some() && !alive(lua, owner)? {
        return Err(rt(format!("{what}: owner object was destroyed")));
    }
    let name = match scope {
        Some(id) => object_event(event, id),
        None => event.to_string(),
    };
    let id = runtime(lua)?.borrow_mut().bus.on(&name, o.priority, once || o.once, Handler::Call { f, owner });
    Ok(LuaListener(id))
}

fn check_event_name(v: &Value, what: &str) -> LuaResult<String> {
    let name = to_str(v, what)?;
    if name.is_empty() || name.contains('@') {
        return Err(rt(format!("{what}: event name must be non-empty and must not contain '@', got '{name}'")));
    }
    Ok(name)
}

pub(crate) fn emit_now(lua: &Lua, name: &str, args: MultiValue) -> LuaResult<bool> {
    emit(lua, name, args)
}

pub(crate) fn emit_queued(lua: &Lua, name: &str, args: MultiValue) -> LuaResult<()> {
    runtime(lua)?.borrow_mut().bus.queue(name, args.into_iter().collect());
    Ok(())
}

/// Object methods: obj:on(name, fn, opts) / obj:once / obj:emit / obj:emit_later.
pub(crate) fn add_object_methods<M: UserDataMethods<Obj>>(m: &mut M) {
    m.add_method("on", |lua, this, (name, f, opts): (Value, Function, Value)| {
        let name = check_event_name(&name, "obj:on")?;
        listen(lua, &name, f, &opts, false, Some(this.0))
    });
    m.add_method("once", |lua, this, (name, f, opts): (Value, Function, Value)| {
        let name = check_event_name(&name, "obj:once")?;
        listen(lua, &name, f, &opts, true, Some(this.0))
    });
    m.add_method("emit", |lua, this, (name, args): (Value, MultiValue)| {
        let name = check_event_name(&name, "obj:emit")?;
        emit_now(lua, &object_event(&name, this.0), args)
    });
    m.add_method("emit_later", |lua, this, (name, args): (Value, MultiValue)| {
        let name = check_event_name(&name, "obj:emit_later")?;
        emit_queued(lua, &object_event(&name, this.0), args)
    });
    m.add_method("after", |lua, this, (s, f): (f64, Function)| {
        let id = runtime(lua)?.borrow_mut().sched.after(s, Job::Call { f, owner: Some(this.0) });
        Ok(LuaTimer(id))
    });
    m.add_method("every", |lua, this, (s, f, times): (f64, Function, Option<u32>)| {
        let id = runtime(lua)?.borrow_mut().sched.every(s, times, Job::Call { f, owner: Some(this.0) });
        Ok(LuaTimer(id))
    });
}

// ---------------------------------------------------------------- globals

pub(crate) fn install(lua: &Lua, g: &Table) -> LuaResult<()> {
    g.set(
        "after",
        lua.create_function(|lua, (s, f, opts): (f64, Function, Value)| {
            let o = parse_opts(&opts, "after", &["owner"])?;
            Ok(LuaTimer(runtime(lua)?.borrow_mut().sched.after(s, Job::Call { f, owner: o.owner })))
        })?,
    )?;
    g.set(
        "every",
        lua.create_function(|lua, (s, f, opts): (f64, Function, Value)| {
            if s <= 0.0 {
                return Err(rt("every: interval must be > 0 seconds (for every frame use update())"));
            }
            let o = parse_opts(&opts, "every", &["owner", "times"])?;
            Ok(LuaTimer(runtime(lua)?.borrow_mut().sched.every(s, o.times, Job::Call { f, owner: o.owner })))
        })?,
    )?;
    g.set(
        "after_frames",
        lua.create_function(|lua, (n, f, opts): (u64, Function, Value)| {
            let o = parse_opts(&opts, "after_frames", &["owner"])?;
            Ok(LuaTimer(runtime(lua)?.borrow_mut().sched.after_frames(n, Job::Call { f, owner: o.owner })))
        })?,
    )?;
    g.set(
        "next_frame",
        lua.create_function(|lua, (f, opts): (Function, Value)| {
            let o = parse_opts(&opts, "next_frame", &["owner"])?;
            Ok(LuaTimer(runtime(lua)?.borrow_mut().sched.after_frames(1, Job::Call { f, owner: o.owner })))
        })?,
    )?;
    g.set("task", lua.create_function(|lua, (f, args): (Function, MultiValue)| spawn_task(lua, f, args))?)?;
    g.set(
        "on",
        lua.create_function(|lua, (name, f, opts): (Value, Function, Value)| {
            let name = check_event_name(&name, "on")?;
            listen(lua, &name, f, &opts, false, None)
        })?,
    )?;
    g.set(
        "once",
        lua.create_function(|lua, (name, f, opts): (Value, Function, Value)| {
            let name = check_event_name(&name, "once")?;
            listen(lua, &name, f, &opts, true, None)
        })?,
    )?;
    g.set(
        "off",
        lua.create_function(|lua, v: Value| match &v {
            Value::UserData(ud) => {
                let l = ud.borrow::<LuaListener>().map_err(|_| rt("off: expected a listener returned by on()/once()"))?;
                Ok(runtime(lua)?.borrow_mut().bus.off(l.0))
            }
            Value::String(s) => {
                runtime(lua)?.borrow_mut().bus.off_all(&s.to_string_lossy());
                Ok(true)
            }
            _ => Err(rt("off: expected a listener or an event name")),
        })?,
    )?;
    g.set(
        "emit",
        lua.create_function(|lua, (name, args): (Value, MultiValue)| {
            let name = check_event_name(&name, "emit")?;
            emit_now(lua, &name, args)
        })?,
    )?;
    g.set(
        "emit_later",
        lua.create_function(|lua, (name, args): (Value, MultiValue)| {
            let name = check_event_name(&name, "emit_later")?;
            emit_queued(lua, &name, args)
        })?,
    )?;
    lua.load(PRELUDE).set_name("=spark").exec()?;
    Ok(())
}
