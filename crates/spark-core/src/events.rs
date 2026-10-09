//! Named events with listeners, generic over the callback type `C` and deferred payload `P`.
//!
//! Rules (the whole contract):
//! - Listeners of one event run by priority (higher first), ties in registration order.
//! - [`EventBus::begin_emit`] takes a snapshot: listeners added during an emit do not get that event;
//!   listeners removed during an emit are skipped if they did not run yet.
//! - A `once` listener runs exactly once, even with nested emits of the same event.
//! - A handler may *consume* the event: the remaining (lower priority) listeners are skipped.
//! - Deferred events ([`EventBus::queue`]) are delivered when the owner calls [`EventBus::take_queue`];
//!   events queued while delivering go to the next batch (no infinite loops within a frame).
//! - Nested emits deeper than [`MAX_DEPTH`] are an error (almost always an infinite loop).

use std::collections::HashMap;

/// Max nesting of emits inside handlers.
pub const MAX_DEPTH: u32 = 64;

/// Handle to a listener.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ListenerId(pub u64);

#[derive(Clone, Debug)]
struct Listener<C> {
    id: ListenerId,
    priority: i32,
    once: bool,
    callback: C,
}

/// Snapshot of listeners for one emit. Feed each item to [`EventBus::claim`] right before calling it.
#[derive(Clone, Debug)]
pub struct Dispatch<C> {
    pub listeners: Vec<(ListenerId, C)>,
}

/// Event listeners plus a deferred queue.
#[derive(Debug)]
pub struct EventBus<C, P = ()> {
    listeners: HashMap<String, Vec<Listener<C>>>,
    names: HashMap<ListenerId, String>,
    queue: Vec<(String, P)>,
    next_id: u64,
    depth: u32,
}

impl<C, P> Default for EventBus<C, P> {
    fn default() -> Self {
        Self { listeners: HashMap::new(), names: HashMap::new(), queue: Vec::new(), next_id: 1, depth: 0 }
    }
}

impl<C: Clone, P> EventBus<C, P> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a listener. Higher `priority` runs first.
    pub fn on(&mut self, name: &str, priority: i32, once: bool, callback: C) -> ListenerId {
        let id = ListenerId(self.next_id);
        self.next_id += 1;
        let list = self.listeners.entry(name.to_string()).or_default();
        // Insert after every listener with priority >= ours: stable registration order.
        let at = list.iter().position(|l| l.priority < priority).unwrap_or(list.len());
        list.insert(at, Listener { id, priority, once, callback });
        self.names.insert(id, name.to_string());
        id
    }

    /// Removes a listener. Returns false if it was already removed.
    pub fn off(&mut self, id: ListenerId) -> bool {
        let Some(name) = self.names.remove(&id) else { return false };
        if let Some(list) = self.listeners.get_mut(&name) {
            list.retain(|l| l.id != id);
            if list.is_empty() {
                self.listeners.remove(&name);
            }
        }
        true
    }

    /// Removes every listener of one event.
    pub fn off_all(&mut self, name: &str) {
        if let Some(list) = self.listeners.remove(name) {
            for l in list {
                self.names.remove(&l.id);
            }
        }
    }

    pub fn is_active(&self, id: ListenerId) -> bool {
        self.names.contains_key(&id)
    }

    /// Event name of a listener.
    pub fn event_of(&self, id: ListenerId) -> Option<&str> {
        self.names.get(&id).map(String::as_str)
    }

    pub fn listener_count(&self, name: &str) -> usize {
        self.listeners.get(name).map_or(0, Vec::len)
    }

    /// Starts an emit: snapshot of listeners. Must be paired with [`EventBus::end_emit`].
    pub fn begin_emit(&mut self, name: &str) -> Result<Dispatch<C>, String> {
        if self.depth >= MAX_DEPTH {
            return Err(format!(
                "event '{name}' was emitted while {MAX_DEPTH} other emits were running - a handler probably emits the event that triggers it (infinite loop)"
            ));
        }
        self.depth += 1;
        let listeners = self.listeners.get(name).map(|l| l.iter().map(|l| (l.id, l.callback.clone())).collect()).unwrap_or_default();
        Ok(Dispatch { listeners })
    }

    pub fn end_emit(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Call right before running a snapshot listener. `false` = skip it (removed meanwhile).
    /// Removes `once` listeners, so they run exactly once.
    pub fn claim(&mut self, id: ListenerId) -> bool {
        let Some(name) = self.names.get(&id) else { return false };
        let once = self.listeners.get(name).and_then(|l| l.iter().find(|l| l.id == id)).is_some_and(|l| l.once);
        if once {
            self.off(id);
        }
        true
    }

    /// Queues an event for later delivery.
    pub fn queue(&mut self, name: &str, payload: P) {
        self.queue.push((name.to_string(), payload));
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Takes all queued events (in queue order). Events queued after this go to the next batch.
    pub fn take_queue(&mut self) -> Vec<(String, P)> {
        std::mem::take(&mut self.queue)
    }

    /// Removes all listeners and queued events.
    pub fn clear(&mut self) {
        self.listeners.clear();
        self.names.clear();
        self.queue.clear();
        self.depth = 0;
    }
}

/// Convenience for simple Rust usage: emits `name` calling `f(callback)` for each listener.
/// `f` returns `true` to consume the event. Returns whether it was consumed.
pub fn emit_with<C: Clone, P>(bus: &mut EventBus<C, P>, name: &str, mut f: impl FnMut(&C) -> bool) -> Result<bool, String> {
    let dispatch = bus.begin_emit(name)?;
    let mut consumed = false;
    for (id, cb) in &dispatch.listeners {
        if bus.claim(*id) && f(cb) {
            consumed = true;
            break;
        }
    }
    bus.end_emit();
    Ok(consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_then_registration_order() {
        let mut bus: EventBus<&str> = EventBus::new();
        bus.on("hit", 0, false, "a");
        bus.on("hit", 10, false, "high");
        bus.on("hit", 0, false, "b");
        bus.on("hit", -5, false, "low");
        bus.on("hit", 10, false, "high2");
        let mut got = vec![];
        emit_with(&mut bus, "hit", |c| {
            got.push(*c);
            false
        })
        .unwrap();
        assert_eq!(got, vec!["high", "high2", "a", "b", "low"]);
    }

    #[test]
    fn once_and_consume() {
        let mut bus: EventBus<&str> = EventBus::new();
        bus.on("e", 0, true, "once");
        bus.on("e", 0, false, "stopper");
        bus.on("e", 0, false, "never");
        let mut got = vec![];
        let consumed = emit_with(&mut bus, "e", |c| {
            got.push(*c);
            *c == "stopper"
        })
        .unwrap();
        assert!(consumed);
        emit_with(&mut bus, "e", |c| {
            got.push(*c);
            true
        })
        .unwrap();
        assert_eq!(got, vec!["once", "stopper", "stopper"]);
    }

    #[test]
    fn snapshot_semantics() {
        let mut bus: EventBus<u32> = EventBus::new();
        let a = bus.on("e", 0, false, 1);
        let b = bus.on("e", 0, false, 2);
        let d = bus.begin_emit("e").unwrap();
        assert!(bus.claim(d.listeners[0].0));
        bus.off(b); // removed by the first handler
        bus.on("e", 0, false, 3); // added by the first handler: not in this emit
        assert!(!bus.claim(d.listeners[1].0));
        assert_eq!(d.listeners.len(), 2);
        bus.end_emit();
        assert!(bus.is_active(a));
        assert_eq!(bus.listener_count("e"), 2);
    }

    #[test]
    fn once_with_nested_emit_runs_once() {
        let mut bus: EventBus<u32> = EventBus::new();
        let id = bus.on("e", 0, true, 7);
        let outer = bus.begin_emit("e").unwrap();
        let inner = bus.begin_emit("e").unwrap();
        assert!(bus.claim(inner.listeners[0].0));
        bus.end_emit();
        assert!(!bus.claim(outer.listeners[0].0));
        bus.end_emit();
        assert!(!bus.is_active(id));
    }

    #[test]
    fn depth_limit_and_queue() {
        let mut bus: EventBus<u32, i32> = EventBus::new();
        for _ in 0..MAX_DEPTH {
            bus.begin_emit("loop").unwrap();
        }
        assert!(bus.begin_emit("loop").is_err());
        bus.queue("a", 1);
        bus.queue("b", 2);
        let q = bus.take_queue();
        bus.queue("c", 3);
        assert_eq!(q, vec![("a".to_string(), 1), ("b".to_string(), 2)]);
        assert_eq!(bus.queued(), 1);
    }
}
