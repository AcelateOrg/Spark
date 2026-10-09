//! Deterministic timer scheduler, generic over the callback type `C`.
//!
//! Rules (the whole contract):
//! - Time only moves in [`Scheduler::advance`]. Nothing fires on its own.
//! - After `advance`, [`Scheduler::pop_due`] returns due timers one by one:
//!   first time-based timers ordered by due time (ties: scheduling order),
//!   then frame-based timers in scheduling order.
//! - Timers scheduled while popping never fire in the same batch (earliest: next `advance`),
//!   so a callback that schedules itself with delay 0 cannot freeze the game.
//! - A repeating timer catches up: if 3 intervals passed during one long frame it fires 3 times
//!   (so results do not depend on frame rate). Safety cap: [`MAX_CATCH_UP`] per batch.
//! - Cancelling or pausing a timer while a batch is being popped takes effect immediately.

use std::collections::{BTreeMap, HashMap};

/// Max times one repeating timer may fire during a single batch.
pub const MAX_CATCH_UP: u32 = 1000;

/// Handle to a scheduled timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    /// (due time as ordered bits, seq)
    Time(u64, u64),
    /// (due frame, seq)
    Frame(u64, u64),
}

#[derive(Clone, Copy, Debug)]
enum Paused {
    Seconds(f64),
    Frames(u64),
}

#[derive(Debug)]
struct Entry<C> {
    callback: C,
    /// Repeat interval in seconds (`None` = one-shot).
    interval: Option<f64>,
    /// Remaining repeats (`None` = forever).
    remaining: Option<u32>,
    /// How many times it has fired.
    fired: u32,
    /// Fires in this batch (catch-up counter).
    batch_fires: u32,
    key: Option<Key>,
    paused: Option<Paused>,
}

/// A timer that just fired.
#[derive(Clone, Debug)]
pub struct Fired<C> {
    pub id: TimerId,
    pub callback: C,
    /// 1 for the first fire, 2 for the second... (useful for `every`).
    pub count: u32,
    /// `true` if this was the last fire (the timer no longer exists).
    pub finished: bool,
}

/// Timers driven by game time and frames.
#[derive(Debug)]
pub struct Scheduler<C> {
    entries: HashMap<TimerId, Entry<C>>,
    time_queue: BTreeMap<Key, TimerId>,
    frame_queue: BTreeMap<Key, TimerId>,
    now: f64,
    frame: u64,
    next_id: u64,
    next_seq: u64,
    batch_limit: u64,
}

impl<C> Default for Scheduler<C> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            time_queue: BTreeMap::new(),
            frame_queue: BTreeMap::new(),
            now: 0.0,
            frame: 0,
            next_id: 1,
            next_seq: 0,
            batch_limit: 0,
        }
    }
}

fn time_bits(t: f64) -> u64 {
    // Non-negative finite floats keep their order as raw bits.
    t.max(0.0).to_bits()
}

impl<C: Clone> Scheduler<C> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scheduler time in seconds (sum of all `advance` calls).
    pub fn now(&self) -> f64 {
        self.now
    }

    /// Number of `advance` calls so far.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Number of live timers (including paused ones).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn insert(&mut self, callback: C, interval: Option<f64>, remaining: Option<u32>) -> TimerId {
        let id = TimerId(self.next_id);
        self.next_id += 1;
        self.entries.insert(id, Entry { callback, interval, remaining, fired: 0, batch_fires: 0, key: None, paused: None });
        id
    }

    fn queue_time(&mut self, id: TimerId, due: f64, seq: Option<u64>) {
        let seq = seq.unwrap_or_else(|| {
            self.next_seq += 1;
            self.next_seq
        });
        let key = Key::Time(time_bits(due), seq);
        self.time_queue.insert(key, id);
        if let Some(e) = self.entries.get_mut(&id) {
            e.key = Some(key);
        }
    }

    fn queue_frame(&mut self, id: TimerId, due_frame: u64) {
        self.next_seq += 1;
        let key = Key::Frame(due_frame, self.next_seq);
        self.frame_queue.insert(key, id);
        if let Some(e) = self.entries.get_mut(&id) {
            e.key = Some(key);
        }
    }

    fn unqueue(&mut self, key: Key) {
        match key {
            Key::Time(..) => self.time_queue.remove(&key),
            Key::Frame(..) => self.frame_queue.remove(&key),
        };
    }

    /// Fires once after `seconds` of game time.
    pub fn after(&mut self, seconds: f64, callback: C) -> TimerId {
        let id = self.insert(callback, None, Some(1));
        self.queue_time(id, self.now + seconds.max(0.0), None);
        id
    }

    /// Fires every `interval` seconds (first time after one interval). `times = None` = forever.
    pub fn every(&mut self, interval: f64, times: Option<u32>, callback: C) -> TimerId {
        let interval = interval.max(1e-6);
        let id = self.insert(callback, Some(interval), times.map(|t| t.max(1)));
        self.queue_time(id, self.now + interval, None);
        id
    }

    /// Fires once after `frames` more `advance` calls (1 = next frame).
    pub fn after_frames(&mut self, frames: u64, callback: C) -> TimerId {
        let id = self.insert(callback, None, Some(1));
        self.queue_frame(id, self.frame + frames.max(1));
        id
    }

    /// Removes a timer. Returns false if it already finished or was cancelled.
    pub fn cancel(&mut self, id: TimerId) -> bool {
        match self.entries.remove(&id) {
            Some(e) => {
                if let Some(k) = e.key {
                    self.unqueue(k);
                }
                true
            }
            None => false,
        }
    }

    /// Freezes a timer (keeps its remaining time).
    pub fn pause(&mut self, id: TimerId) -> bool {
        let (now, frame) = (self.now, self.frame);
        let Some(e) = self.entries.get_mut(&id) else { return false };
        let Some(key) = e.key.take() else { return true };
        e.paused = Some(match key {
            Key::Time(bits, _) => Paused::Seconds((f64::from_bits(bits) - now).max(0.0)),
            Key::Frame(f, _) => Paused::Frames(f.saturating_sub(frame).max(1)),
        });
        self.unqueue(key);
        true
    }

    /// Continues a paused timer.
    pub fn resume(&mut self, id: TimerId) -> bool {
        let Some(e) = self.entries.get_mut(&id) else { return false };
        match e.paused.take() {
            Some(Paused::Seconds(s)) => self.queue_time(id, self.now + s, None),
            Some(Paused::Frames(f)) => self.queue_frame(id, self.frame + f),
            None => {}
        }
        true
    }

    pub fn exists(&self, id: TimerId) -> bool {
        self.entries.contains_key(&id)
    }

    pub fn is_paused(&self, id: TimerId) -> bool {
        self.entries.get(&id).is_some_and(|e| e.paused.is_some())
    }

    /// Seconds until the timer fires (`None` if it does not exist or is frame-based).
    pub fn remaining(&self, id: TimerId) -> Option<f64> {
        let e = self.entries.get(&id)?;
        match (e.key, e.paused) {
            (Some(Key::Time(bits, _)), _) => Some((f64::from_bits(bits) - self.now).max(0.0)),
            (_, Some(Paused::Seconds(s))) => Some(s),
            _ => None,
        }
    }

    /// Moves time forward and starts a new batch. Then call [`Scheduler::pop_due`] until it returns `None`.
    pub fn advance(&mut self, dt: f64) {
        self.now += dt.max(0.0);
        self.frame += 1;
        self.batch_limit = self.next_seq;
        for e in self.entries.values_mut() {
            e.batch_fires = 0;
        }
    }

    /// Next due timer of the current batch.
    pub fn pop_due(&mut self) -> Option<Fired<C>> {
        let (key, id) = self.next_due()?;
        self.unqueue(key);
        let now = self.now;
        let e = self.entries.get_mut(&id)?;
        e.key = None;
        e.fired += 1;
        e.batch_fires += 1;
        if let Some(r) = e.remaining.as_mut() {
            *r -= 1;
        }
        let count = e.fired;
        let callback = e.callback.clone();
        let finished = e.remaining == Some(0) || e.interval.is_none();
        if finished {
            self.entries.remove(&id);
        } else if let (Some(interval), Key::Time(bits, seq)) = (e.interval, key) {
            let next = f64::from_bits(bits) + interval;
            if e.batch_fires >= MAX_CATCH_UP {
                // Way behind: skip the missed ticks, continue from now.
                self.queue_time(id, now + interval, None);
            } else {
                // Same seq: missed ticks still belong to this batch (catch-up).
                self.queue_time(id, next, Some(seq));
            }
        }
        Some(Fired { id, callback, count, finished })
    }

    fn next_due(&self) -> Option<(Key, TimerId)> {
        if let Some((&key, &id)) = self.time_queue.first_key_value() {
            if let Key::Time(bits, seq) = key {
                if f64::from_bits(bits) <= self.now && seq <= self.batch_limit {
                    return Some((key, id));
                }
            }
        }
        if let Some((&key, &id)) = self.frame_queue.first_key_value() {
            if let Key::Frame(f, seq) = key {
                if f <= self.frame && seq <= self.batch_limit {
                    return Some((key, id));
                }
            }
        }
        None
    }

    /// Removes every timer.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.time_queue.clear();
        self.frame_queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(s: &mut Scheduler<&'static str>) -> Vec<&'static str> {
        std::iter::from_fn(|| s.pop_due().map(|f| f.callback)).collect()
    }

    #[test]
    fn order_by_due_then_schedule_order_then_frames() {
        let mut s = Scheduler::new();
        s.after_frames(1, "frame");
        s.after(0.5, "b");
        s.after(0.2, "a");
        s.after(0.5, "c");
        s.after(2.0, "late");
        s.advance(1.0);
        assert_eq!(drain(&mut s), vec!["a", "b", "c", "frame"]);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn scheduled_during_batch_waits_for_next_advance() {
        let mut s = Scheduler::new();
        s.after(0.0, "first");
        s.advance(0.1);
        assert_eq!(s.pop_due().unwrap().callback, "first");
        s.after(0.0, "inner");
        s.after_frames(1, "inner_frame");
        assert!(s.pop_due().is_none());
        s.advance(0.1);
        assert_eq!(drain(&mut s), vec!["inner", "inner_frame"]);
    }

    #[test]
    fn every_catches_up_and_counts() {
        let mut s = Scheduler::new();
        let id = s.every(0.1, Some(5), "tick");
        s.advance(0.35);
        let fired: Vec<_> = std::iter::from_fn(|| s.pop_due()).map(|f| (f.count, f.finished)).collect();
        assert_eq!(fired, vec![(1, false), (2, false), (3, false)]);
        s.advance(1.0);
        let fired: Vec<_> = std::iter::from_fn(|| s.pop_due()).map(|f| (f.count, f.finished)).collect();
        assert_eq!(fired, vec![(4, false), (5, true)]);
        assert!(!s.exists(id));
    }

    #[test]
    fn frame_rate_independent() {
        let count = |dt: f64, frames: u32| {
            let mut s = Scheduler::new();
            s.every(0.25, None, ());
            let mut n = 0;
            for _ in 0..frames {
                s.advance(dt);
                while s.pop_due().is_some() {
                    n += 1;
                }
            }
            n
        };
        // 10.1 s either way (avoids float edge exactly on a tick).
        assert_eq!(count(1.0 / 60.0, 606), 40);
        assert_eq!(count(1.0 / 20.0, 202), 40);
        assert_eq!(count(0.5, 20), 40);
    }

    #[test]
    fn cancel_pause_resume() {
        let mut s = Scheduler::new();
        let a = s.after(1.0, "a");
        let b = s.after(1.0, "b");
        s.advance(0.4);
        assert!(s.pause(a));
        assert!((s.remaining(a).unwrap() - 0.6).abs() < 1e-9);
        assert!(s.cancel(b));
        assert!(!s.cancel(b));
        s.advance(5.0);
        assert!(s.pop_due().is_none());
        s.resume(a);
        s.advance(0.5);
        assert!(s.pop_due().is_none());
        s.advance(0.1);
        assert_eq!(drain(&mut s), vec!["a"]);
    }

    #[test]
    fn cancel_mid_batch() {
        let mut s = Scheduler::new();
        s.after(0.1, "a");
        let b = s.after(0.2, "b");
        s.advance(1.0);
        assert_eq!(s.pop_due().unwrap().callback, "a");
        s.cancel(b);
        assert!(s.pop_due().is_none());
    }
}
