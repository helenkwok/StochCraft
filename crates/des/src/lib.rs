//! StochCraft's discrete-event simulation core.
//!
//! - [`Sim`] runs processes written as `async` blocks on simulated time, SimPy-style: a process
//!   holds for a delay, waits for a [`Resource`] or an item from a [`Store`], spawns other
//!   processes. It is single-threaded and needs no async runtime, so it runs unchanged in the
//!   browser. Parallelism belongs one level up, as independent replications.
//! - The building blocks of factory and logistics models:
//!   - [`Resource`]: units served by priority, then first come, first served; optionally
//!     preemptive ([`Resource::preemptive`]).
//!   - [`Store`]: unbounded or bounded ([`Store::put_wait`] blocks while full), first in first
//!     out or by priority ([`Store::with_priority`]).
//!   - Waits with a deadline ([`Resource::acquire_within`], [`Store::get_within`]) for
//!     reneging, and immediate refusals ([`Store::try_put`]) for balking.
//!   - Interrupts ([`Ctx::interrupt`], [`Ctx::hold_interruptible`]) for breakdowns and
//!     preemption.
//!   - [`Signal`] (wake everyone waiting now) and [`Gate`] (a condition that stays true while
//!     open) for gates, schedules and assembly.
//! - [`EventQueue`] is the underlying queue: events are ordered by simulated time, then by priority
//!   (higher first), then by scheduling order, so two runs of the same model always pop events
//!   in the same order. Times are `f64`; non-finite times are refused rather than stored.
//!
//! All counters are fixed-width (`u64`) so results match on native, wasm32 and wasm64.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

mod resource;
mod signal;
mod sim;
mod store;
pub use resource::{Acquire, AcquireWithin, Resource, ResourceGuard};
pub use signal::{Gate, Signal, Wait};
pub use sim::{Ctx, Hold, HoldInterruptible, Interrupt, InterruptCause, Interrupted, Sim};
pub use store::{Get, GetWithin, PutWait, Store};

/// A wait with a deadline ([`Resource::acquire_within`], [`Store::get_within`]) ended at the
/// deadline without getting what it waited for. It has left the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedOut;

impl std::fmt::Display for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the wait timed out")
    }
}

impl std::error::Error for TimedOut {}

/// Why an event could not be scheduled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleError {
    /// The time was NaN or infinite.
    NonFiniteTime,
    /// The time was earlier than the current simulated time.
    InThePast,
}

/// An event taken off the queue.
#[derive(Debug, Clone, PartialEq)]
pub struct Event<T> {
    pub time: f64,
    pub priority: i64,
    /// The order in which the event was scheduled (0, 1, 2, ...).
    pub seq: u64,
    pub payload: T,
}

struct Entry<T>(Event<T>);

impl<T> PartialEq for Entry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl<T> Eq for Entry<T> {}
impl<T> PartialOrd for Entry<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Entry<T> {
    /// `BinaryHeap` is a max-heap, so "greater" means "pops first": earlier time, then higher
    /// priority, then lower sequence number.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .0
            .time
            .total_cmp(&self.0.time)
            .then(self.0.priority.cmp(&other.0.priority))
            .then(other.0.seq.cmp(&self.0.seq))
    }
}

/// A deterministic event queue on simulated time.
pub struct EventQueue<T> {
    now: f64,
    next_seq: u64,
    heap: BinaryHeap<Entry<T>>,
}

impl<T> Default for EventQueue<T> {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl<T> EventQueue<T> {
    /// An empty queue whose clock starts at `start` (0.0 if `start` is not finite).
    pub fn new(start: f64) -> Self {
        let now = if start.is_finite() { start } else { 0.0 };
        Self {
            now,
            next_seq: 0,
            heap: BinaryHeap::new(),
        }
    }

    /// The current simulated time: the time of the last event popped.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// Number of pending events.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Schedules `payload` at absolute time `time`. Returns its sequence number.
    pub fn schedule_at(
        &mut self,
        time: f64,
        priority: i64,
        payload: T,
    ) -> Result<u64, ScheduleError> {
        if !time.is_finite() {
            return Err(ScheduleError::NonFiniteTime);
        }
        if time < self.now {
            return Err(ScheduleError::InThePast);
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.heap.push(Entry(Event {
            time,
            priority,
            seq,
            payload,
        }));
        Ok(seq)
    }

    /// Schedules `payload` `delay` time units from now.
    pub fn schedule_in(
        &mut self,
        delay: f64,
        priority: i64,
        payload: T,
    ) -> Result<u64, ScheduleError> {
        self.schedule_at(self.now + delay, priority, payload)
    }

    /// The sequence number the next scheduled event will get.
    pub(crate) fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// The time of the next event, if any.
    pub fn peek_time(&self) -> Option<f64> {
        self.heap.peek().map(|e| e.0.time)
    }

    /// Moves the clock forward to `time` without popping (ignored if `time` is earlier than now,
    /// later than the next event or not finite).
    pub fn advance_to(&mut self, time: f64) {
        let before_next = self.peek_time().is_none_or(|t| time <= t);
        if time.is_finite() && time >= self.now && before_next {
            self.now = time;
        }
    }

    /// Pops the next event and advances the clock to its time.
    pub fn pop(&mut self) -> Option<Event<T>> {
        let Entry(ev) = self.heap.pop()?;
        self.now = ev.time;
        Some(ev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pops_by_time_then_priority_then_order() {
        let mut q = EventQueue::new(0.0);
        assert_eq!(q.schedule_at(2.0, 0, "late"), Ok(0));
        assert_eq!(q.schedule_at(1.0, 0, "first-low"), Ok(1));
        assert_eq!(q.schedule_at(1.0, 5, "first-high"), Ok(2));
        assert_eq!(q.schedule_at(1.0, 0, "second-low"), Ok(3));
        let order: Vec<_> = std::iter::from_fn(|| q.pop()).map(|e| e.payload).collect();
        assert_eq!(order, ["first-high", "first-low", "second-low", "late"]);
        assert_eq!(q.now(), 2.0);
    }

    #[test]
    fn refuses_bad_times() {
        let mut q = EventQueue::new(5.0);
        assert_eq!(
            q.schedule_at(f64::NAN, 0, ()),
            Err(ScheduleError::NonFiniteTime)
        );
        assert_eq!(
            q.schedule_at(f64::INFINITY, 0, ()),
            Err(ScheduleError::NonFiniteTime)
        );
        assert_eq!(q.schedule_at(4.0, 0, ()), Err(ScheduleError::InThePast));
        assert!(q.is_empty());
    }

    #[test]
    fn schedule_in_is_relative_to_now() {
        let mut q = EventQueue::new(0.0);
        assert!(q.schedule_at(3.0, 0, 'a').is_ok());
        assert_eq!(q.pop().map(|e| e.time), Some(3.0));
        assert!(q.schedule_in(1.5, 0, 'b').is_ok());
        assert_eq!(q.pop().map(|e| e.time), Some(4.5));
    }
}
