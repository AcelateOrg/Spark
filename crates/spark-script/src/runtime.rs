//! Timers, tasks (coroutines) and events for scripts, on top of `spark_core::{Scheduler, EventBus}`.
//!
//! Frame order (also documented in docs/LUAU_API.md):
//! 1. input events  2. fixed_update xN  3. update  4. timers + tasks  5. emit_later events  6. late_update  7. render

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use mlua::{ErrorContext, Function, Lua, MultiValue, Thread, Value};
use spark_core::{EventBus, Key, ListenerId, MouseButton, ObjectId, Scheduler, TimerId};

use crate::convert::*;

#[derive(Clone)]
pub(crate) enum Job {
    Call { f: Function, owner: Option<ObjectId> },
    Resume { task: u64, token: u64, kind: ResumeKind },
    Until { task: u64, token: u64, pred: Function },
}

#[derive(Clone, Copy)]
pub(crate) enum ResumeKind {
    /// `wait(s)`: resume with the real elapsed game time since `start`.
    Elapsed(f64),
    /// `wait_frames(n)`: resume with nothing.
    Nothing,
    /// Timeout of `wait_until` / `wait_event`: resume with nil.
    Timeout,
}

#[derive(Clone)]
pub(crate) enum Handler {
    Call { f: Function, owner: Option<ObjectId> },
    Resume { task: u64, token: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Running,
    Waiting,
    Done,
    Cancelled,
    Failed,
}

impl TaskStatus {
    pub fn name(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

struct TaskState {
    thread: Thread,
    /// Bumped on every resume: stale timers / listeners of an old wait are ignored.
    token: u64,
    status: Rc<Cell<TaskStatus>>,
    timers: Vec<TimerId>,
    listener: Option<ListenerId>,
}

#[derive(Default)]
pub(crate) struct Runtime {
    pub sched: Scheduler<Job>,
    pub bus: EventBus<Handler, Vec<Value>>,
    tasks: HashMap<u64, TaskState>,
    next_task: u64,
}

impl Runtime {
    /// Drops every Lua reference (call before dropping the Lua state).
    pub fn clear(&mut self) {
        self.sched.clear();
        self.bus.clear();
        self.tasks.clear();
    }

}

pub(crate) type SharedRuntime = Rc<RefCell<Runtime>>;

pub(crate) fn runtime(lua: &Lua) -> LuaResult<SharedRuntime> {
    Ok(lua.app_data_ref::<SharedRuntime>().ok_or_else(|| rt("spark: runtime is not attached"))?.clone())
}

pub(crate) fn alive(lua: &Lua, owner: Option<ObjectId>) -> LuaResult<bool> {
    match owner {
        None => Ok(true),
        Some(o) => with(lua, |w| Ok(w.scene.contains(o))),
    }
}

// ---------------------------------------------------------------- tasks

/// Handle returned by `task(fn)`.
pub struct LuaTask {
    pub id: u64,
    pub status: Rc<Cell<TaskStatus>>,
}

pub(crate) fn spawn_task(lua: &Lua, f: Function, args: MultiValue) -> LuaResult<LuaTask> {
    let thread = lua.create_thread(f)?;
    let status = Rc::new(Cell::new(TaskStatus::Running));
    let id = {
        let rtc = runtime(lua)?;
        let mut r = rtc.borrow_mut();
        r.next_task += 1;
        let id = r.next_task;
        r.tasks.insert(id, TaskState { thread, token: 0, status: status.clone(), timers: Vec::new(), listener: None });
        id
    };
    resume_task(lua, id, args)?;
    Ok(LuaTask { id, status })
}

/// Stops a task: its pending wait is dropped, it never resumes.
pub(crate) fn cancel_task(lua: &Lua, id: u64) -> LuaResult<bool> {
    let rtc = runtime(lua)?;
    let mut guard = rtc.borrow_mut();
    let r = &mut *guard;
    let Some(t) = r.tasks.remove(&id) else { return Ok(false) };
    for tm in t.timers {
        r.sched.cancel(tm);
    }
    if let Some(l) = t.listener {
        r.bus.off(l);
    }
    t.status.set(TaskStatus::Cancelled);
    Ok(true)
}

fn finish_task(lua: &Lua, id: u64, status: TaskStatus) -> LuaResult<()> {
    if let Some(t) = runtime(lua)?.borrow_mut().tasks.remove(&id) {
        t.status.set(status);
    }
    Ok(())
}

fn token_ok(lua: &Lua, task: u64, token: u64) -> LuaResult<bool> {
    Ok(runtime(lua)?.borrow().tasks.get(&task).is_some_and(|t| t.token == token))
}

fn resume_task(lua: &Lua, id: u64, args: MultiValue) -> LuaResult<()> {
    let thread = {
        let rtc = runtime(lua)?;
        let mut guard = rtc.borrow_mut();
        let r = &mut *guard;
        let Some(t) = r.tasks.get_mut(&id) else { return Ok(()) };
        t.status.set(TaskStatus::Running);
        t.token += 1;
        for tm in t.timers.drain(..) {
            r.sched.cancel(tm);
        }
        if let Some(l) = t.listener.take() {
            r.bus.off(l);
        }
        t.thread.clone()
    };
    match thread.resume::<MultiValue>(args) {
        Err(e) => {
            finish_task(lua, id, TaskStatus::Failed)?;
            Err(e.context("in task"))
        }
        Ok(values) if thread.is_resumable() => schedule_wait(lua, id, values),
        Ok(_) => finish_task(lua, id, TaskStatus::Done),
    }
}

/// The task yielded `(kind, a, b)` from one of the `wait*` functions: register what wakes it up.
fn schedule_wait(lua: &Lua, id: u64, values: MultiValue) -> LuaResult<()> {
    let mut it = values.into_iter();
    let kind = match it.next() {
        None => "frames".to_string(),
        Some(Value::String(s)) => s.to_string_lossy(),
        Some(other) => return Err(rt(format!("task yielded an unexpected value ({}) - use wait(), wait_frames(), wait_until() or wait_event()", other.type_name()))),
    };
    let a = it.next().unwrap_or(Value::Nil);
    let b = it.next().unwrap_or(Value::Nil);
    let timeout = if b.is_nil() { None } else { Some(to_num(&b, &format!("{kind} timeout"))? as f64) };

    let rtc = runtime(lua)?;
    let mut guard = rtc.borrow_mut();
    let Runtime { sched, bus, tasks, .. } = &mut *guard;
    // Cancelled while running (task:cancel() from inside itself): just stop.
    let Some(t) = tasks.get_mut(&id) else { return Ok(()) };
    let token = t.token;
    t.status.set(TaskStatus::Waiting);
    match kind.as_str() {
        "time" => {
            let s = if a.is_nil() { 0.0 } else { to_num(&a, "wait")? as f64 };
            let start = sched.now();
            t.timers.push(sched.after(s, Job::Resume { task: id, token, kind: ResumeKind::Elapsed(start) }));
        }
        "frames" => {
            let n = if a.is_nil() { 1.0 } else { to_num(&a, "wait_frames")? };
            t.timers.push(sched.after_frames(n.max(1.0) as u64, Job::Resume { task: id, token, kind: ResumeKind::Nothing }));
        }
        "until" => {
            let Value::Function(pred) = a else {
                return Err(rt(format!("wait_until: expected a function returning true/false, got {}", a.type_name())));
            };
            t.timers.push(sched.after_frames(1, Job::Until { task: id, token, pred }));
        }
        "event" => {
            let name = to_str(&a, "wait_event")?;
            t.listener = Some(bus.on(&name, 0, true, Handler::Resume { task: id, token }));
        }
        other => return Err(rt(format!("task yielded unknown wait kind '{other}'"))),
    }
    if let Some(s) = timeout {
        t.timers.push(sched.after(s, Job::Resume { task: id, token, kind: ResumeKind::Timeout }));
    }
    Ok(())
}

// ---------------------------------------------------------------- frame steps

/// Starts a new scheduler batch (once per frame, before anything else).
pub(crate) fn begin_frame(lua: &Lua, dt: f64) -> LuaResult<()> {
    runtime(lua)?.borrow_mut().sched.advance(dt);
    // Engine input events, in a fixed order: keys (enum order), mouse buttons, wheel.
    let (pressed, released, mouse_down, mouse_up, wheel, mouse) = with(lua, |w| {
        let i = &w.input;
        let names = |f: &dyn Fn(Key) -> bool| Key::ALL.iter().filter(|k| f(**k)).map(|k| k.name()).collect::<Vec<_>>();
        let buttons = [("left", MouseButton::Left), ("right", MouseButton::Right), ("middle", MouseButton::Middle)];
        let down = buttons.iter().filter(|(_, b)| i.mouse_pressed(*b)).map(|(n, _)| *n).collect::<Vec<_>>();
        let up = buttons.iter().filter(|(_, b)| i.mouse_released(*b)).map(|(n, _)| *n).collect::<Vec<_>>();
        Ok((names(&|k| i.pressed(k)), names(&|k| i.released(k)), down, up, i.wheel, i.mouse_position))
    })?;
    let has = |n: &str| -> LuaResult<bool> { Ok(runtime(lua)?.borrow().bus.listener_count(n) > 0) };
    if has("key_pressed")? {
        for k in pressed {
            emit(lua, "key_pressed", MultiValue::from_vec(vec![Value::String(lua.create_string(k)?)]))?;
        }
    }
    if has("key_released")? {
        for k in released {
            emit(lua, "key_released", MultiValue::from_vec(vec![Value::String(lua.create_string(k)?)]))?;
        }
    }
    for (event, list) in [("mouse_pressed", mouse_down), ("mouse_released", mouse_up)] {
        if has(event)? {
            for b in list {
                let args = vec![Value::String(lua.create_string(b)?), Value::Number(mouse.x as f64), Value::Number(mouse.y as f64)];
                emit(lua, event, MultiValue::from_vec(args))?;
            }
        }
    }
    if wheel != 0.0 && has("wheel")? {
        emit(lua, "wheel", MultiValue::from_vec(vec![Value::Number(wheel as f64)]))?;
    }
    Ok(())
}

/// Runs every due timer / task wake-up of this frame.
pub(crate) fn run_timers(lua: &Lua) -> LuaResult<()> {
    let rtc = runtime(lua)?;
    loop {
        let fired = rtc.borrow_mut().sched.pop_due();
        let Some(fired) = fired else { break };
        match fired.callback {
            Job::Call { f, owner } => {
                if !alive(lua, owner)? {
                    rtc.borrow_mut().sched.cancel(fired.id);
                    continue;
                }
                f.call::<()>(fired.count).map_err(|e| e.context("in timer callback (after/every)"))?;
            }
            Job::Resume { task, token, kind } => {
                if !token_ok(lua, task, token)? {
                    continue;
                }
                let args = match kind {
                    ResumeKind::Elapsed(start) => {
                        let now = rtc.borrow().sched.now();
                        MultiValue::from_vec(vec![Value::Number(now - start)])
                    }
                    ResumeKind::Nothing => MultiValue::new(),
                    ResumeKind::Timeout => MultiValue::from_vec(vec![Value::Nil]),
                };
                resume_task(lua, task, args)?;
            }
            Job::Until { task, token, pred } => {
                if !token_ok(lua, task, token)? {
                    continue;
                }
                let done = pred.call::<Value>(()).map_err(|e| e.context("in wait_until condition"))?;
                if !matches!(done, Value::Nil | Value::Boolean(false)) {
                    resume_task(lua, task, MultiValue::from_vec(vec![Value::Boolean(true)]))?;
                } else {
                    let mut guard = rtc.borrow_mut();
                    let Runtime { sched, tasks, .. } = &mut *guard;
                    if let Some(t) = tasks.get_mut(&task) {
                        t.timers.push(sched.after_frames(1, Job::Until { task, token, pred }));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Delivers events queued with `emit_later` (events queued meanwhile wait for the next frame).
pub(crate) fn flush_deferred(lua: &Lua) -> LuaResult<()> {
    let queue = runtime(lua)?.borrow_mut().bus.take_queue();
    for (name, args) in queue {
        emit(lua, &name, MultiValue::from_vec(args))?;
    }
    Ok(())
}

/// Immediate emit. Returns true if a handler consumed the event (returned `true`).
pub(crate) fn emit(lua: &Lua, name: &str, args: MultiValue) -> LuaResult<bool> {
    let rtc = runtime(lua)?;
    let dispatch = rtc.borrow_mut().bus.begin_emit(name).map_err(rt)?;
    let result = (|| -> LuaResult<bool> {
        for (id, handler) in dispatch.listeners {
            if !rtc.borrow_mut().bus.claim(id) {
                continue;
            }
            match handler {
                Handler::Call { f, owner } => {
                    if !alive(lua, owner)? {
                        rtc.borrow_mut().bus.off(id);
                        continue;
                    }
                    let v = f.call::<Value>(args.clone()).map_err(|e| e.context(format!("in handler of event '{}'", display_name(name))))?;
                    if v == Value::Boolean(true) {
                        return Ok(true);
                    }
                }
                Handler::Resume { task, token } => {
                    if token_ok(lua, task, token)? {
                        let a = if args.is_empty() { MultiValue::from_vec(vec![Value::Boolean(true)]) } else { args.clone() };
                        resume_task(lua, task, a)?;
                    }
                }
            }
        }
        Ok(false)
    })();
    rtc.borrow_mut().bus.end_emit();
    result
}

/// Object events are stored as "name@<object bits>".
pub(crate) fn object_event(name: &str, id: ObjectId) -> String {
    format!("{name}@{}", id.to_bits())
}

fn display_name(name: &str) -> &str {
    name.split('@').next().unwrap_or(name)
}
