//! Resources: pools of identical units that processes acquire and give back.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::TimedOut;
use crate::sim::{Ctx, InterruptCause, deadline};

/// Where a request of `priority` goes in a queue kept in priority order (higher first, then
/// first come, first served): after the last entry of the same or higher priority. Searching
/// from the back makes the usual case, equal priorities, a push at the back.
pub(crate) fn insert_position<E>(
    queue: &VecDeque<E>,
    priority: i64,
    prio: impl Fn(&E) -> i64,
) -> usize {
    let mut at = queue.len();
    while at > 0 && queue.get(at - 1).is_some_and(|e| prio(e) < priority) {
        at -= 1;
    }
    at
}

struct Waiter {
    pid: u64,
    priority: i64,
    /// Set when a unit is handed to this waiter; holds the preemption flag of a preemptive
    /// resource's unit.
    grant: Rc<Grant>,
}

struct Grant {
    granted: Cell<bool>,
    /// The unit's preemption flag, for a preemptive resource.
    preempted: Option<Rc<Cell<bool>>>,
}

/// A unit held from a preemptive resource.
struct Holder {
    pid: Option<u64>,
    priority: i64,
    preempted: Rc<Cell<bool>>,
}

struct ResourceState {
    capacity: u64,
    in_use: u64,
    /// Waiting processes, highest priority first and then in arrival order.
    waiters: VecDeque<Waiter>,
    /// The units held, in the order they were taken; only kept by a preemptive resource.
    holders: Option<Vec<Holder>>,
}

/// A pool of identical units (a server, a machine, a forklift fleet). Processes acquire one unit
/// and give it back by dropping the guard.
///
/// Waiters are served by priority (higher first), then first come, first served;
/// [`Resource::acquire`] asks with priority 0. A [`Resource::preemptive`] resource also lets a
/// request take a unit from a holder of lower priority (see there).
#[derive(Clone)]
pub struct Resource {
    ctx: Ctx,
    state: Rc<RefCell<ResourceState>>,
}

impl Resource {
    /// A resource with `capacity` units (at least 1).
    pub fn new(ctx: &Ctx, capacity: u64) -> Self {
        Self::build(ctx, capacity, false)
    }

    /// A resource with `capacity` units (at least 1) whose requests preempt.
    ///
    /// When every unit is held, a request of higher priority than some holder takes the unit
    /// of the holder with the lowest priority (the one that took it last, among equals) instead
    /// of waiting. That holder's [`ResourceGuard::is_preempted`] turns true, dropping its guard
    /// gives nothing back, and the holder process receives an interrupt with
    /// [`InterruptCause::Preempted`]: its [`Ctx::hold_interruptible`] ends with the time it
    /// still had to run, so it can ask again and finish the rest. A request never preempts a
    /// holder of equal or higher priority; it waits like any other.
    ///
    /// Only a holder in [`Ctx::hold_interruptible`] at that moment can be preempted, since only
    /// it stops using the unit at once. A holder in a plain [`Ctx::hold`], or waiting for
    /// something else, keeps its unit (taking it would leave more users than units, and leave
    /// an interrupt to end some later, unrelated wait), and so does a guard taken outside a
    /// process. The decision is made when the request arrives: if no holder can be preempted
    /// then, the request waits like any other, ahead of lower priorities, and does not preempt
    /// a holder that starts an interruptible hold later. So hold a preemptive unit with
    /// [`Ctx::hold_interruptible`] for any time it may be taken away.
    pub fn preemptive(ctx: &Ctx, capacity: u64) -> Self {
        Self::build(ctx, capacity, true)
    }

    fn build(ctx: &Ctx, capacity: u64, preemptive: bool) -> Self {
        let state = ResourceState {
            capacity: capacity.max(1),
            in_use: 0,
            waiters: VecDeque::new(),
            holders: preemptive.then(Vec::new),
        };
        Self {
            ctx: ctx.clone(),
            state: Rc::new(RefCell::new(state)),
        }
    }

    pub fn capacity(&self) -> u64 {
        self.state.borrow().capacity
    }

    /// Units currently held.
    pub fn in_use(&self) -> u64 {
        self.state.borrow().in_use
    }

    /// Processes waiting for a unit.
    pub fn queue_len(&self) -> usize {
        self.state.borrow().waiters.len()
    }

    /// Takes a unit now if one is free and nobody is waiting for one, without waiting (and
    /// without preempting); `None` otherwise.
    pub fn try_acquire(&self) -> Option<ResourceGuard> {
        let st = self.state.borrow();
        if st.in_use >= st.capacity || !st.waiters.is_empty() {
            return None;
        }
        drop(st);
        self.try_take(0, None)
    }

    /// Waits for a unit, with priority 0. Hold the returned guard while using it.
    pub fn acquire(&self) -> Acquire {
        self.acquire_with_priority(0)
    }

    /// Waits for a unit, ahead of waiters with a lower priority.
    pub fn acquire_with_priority(&self, priority: i64) -> Acquire {
        Acquire {
            inner: self.request(priority, f64::INFINITY),
        }
    }

    /// Waits for a unit for at most `patience` time units (reneging). A unit handed over at the
    /// deadline itself still counts; a patience of zero takes a unit only if one is free now
    /// or freed at this instant. On time-out the request leaves the queue.
    pub fn acquire_within(&self, patience: f64) -> AcquireWithin {
        self.acquire_with_priority_within(0, patience)
    }

    /// [`Resource::acquire_within`] with a priority.
    pub fn acquire_with_priority_within(&self, priority: i64, patience: f64) -> AcquireWithin {
        self.request(priority, patience)
    }

    fn request(&self, priority: i64, patience: f64) -> AcquireWithin {
        AcquireWithin {
            res: self.clone(),
            priority,
            patience,
            waiting: None,
        }
    }

    /// Takes a unit now if one is free or can be preempted; otherwise `None`.
    fn try_take(&self, priority: i64, pid: Option<u64>) -> Option<ResourceGuard> {
        let mut st = self.state.borrow_mut();
        if st.in_use < st.capacity && st.waiters.is_empty() {
            st.in_use += 1;
            let preempted = st.holders.as_mut().map(|h| {
                let flag = Rc::new(Cell::new(false));
                h.push(Holder {
                    pid,
                    priority,
                    preempted: Rc::clone(&flag),
                });
                flag
            });
            return Some(self.guard(preempted));
        }
        // Preempt the holder with the lowest priority, the latest among equals, among those in an
        // interruptible hold (see `Resource::preemptive`).
        let holders = st.holders.as_mut()?;
        let victim = holders
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, h)| {
                h.priority < priority && h.pid.is_some_and(|p| self.ctx.is_interruptible(p))
            })
            .min_by_key(|(_, h)| h.priority)
            .map(|(i, _)| i)?;
        let v = holders.remove(victim);
        v.preempted.set(true);
        let flag = Rc::new(Cell::new(false));
        holders.push(Holder {
            pid,
            priority,
            preempted: Rc::clone(&flag),
        });
        drop(st);
        if let Some(p) = v.pid {
            self.ctx.deliver(p, InterruptCause::Preempted);
        }
        Some(self.guard(Some(flag)))
    }

    fn guard(&self, preempted: Option<Rc<Cell<bool>>>) -> ResourceGuard {
        ResourceGuard {
            res: self.clone(),
            preempted,
        }
    }

    fn release(&self, preempted: Option<&Rc<Cell<bool>>>) {
        let next = {
            let mut st = self.state.borrow_mut();
            if let (Some(holders), Some(flag)) = (&mut st.holders, preempted) {
                holders.retain(|h| !Rc::ptr_eq(&h.preempted, flag));
            }
            match st.waiters.pop_front() {
                // Hand the unit straight to the next waiter: `in_use` stays the same.
                Some(w) => {
                    w.grant.granted.set(true);
                    if let (Some(h), Some(flag)) = (&mut st.holders, &w.grant.preempted) {
                        h.push(Holder {
                            pid: Some(w.pid),
                            priority: w.priority,
                            preempted: Rc::clone(flag),
                        });
                    }
                    Some(w.pid)
                }
                None => {
                    st.in_use = st.in_use.saturating_sub(1);
                    None
                }
            }
        };
        if let Some(pid) = next {
            self.ctx.wake_now(pid);
        }
    }
}

/// The future returned by [`Resource::acquire`] and [`Resource::acquire_with_priority`].
pub struct Acquire {
    inner: AcquireWithin,
}

impl Future for Acquire {
    type Output = ResourceGuard;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<ResourceGuard> {
        match Pin::new(&mut self.inner).poll(cx) {
            Poll::Ready(Ok(g)) => Poll::Ready(g),
            // Without a deadline a request never times out.
            Poll::Ready(Err(TimedOut)) | Poll::Pending => Poll::Pending,
        }
    }
}

/// The future returned by [`Resource::acquire_within`]: the unit, or [`TimedOut`].
pub struct AcquireWithin {
    res: Resource,
    priority: i64,
    patience: f64,
    /// Once queued: the grant to watch and the deadline, if any.
    waiting: Option<(Rc<Grant>, Option<f64>)>,
}

impl Future for AcquireWithin {
    type Output = Result<ResourceGuard, TimedOut>;
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let ctx = self.res.ctx.clone();
        let now = ctx.now();
        if let Some((grant, until)) = &self.waiting {
            if grant.granted.get() {
                return Poll::Ready(Ok(self.res.guard(grant.preempted.clone())));
            }
            if let Some(until) = *until {
                if now >= until {
                    let mut st = self.res.state.borrow_mut();
                    st.waiters.retain(|w| !Rc::ptr_eq(&w.grant, grant));
                    return Poll::Ready(Err(TimedOut));
                }
                // Woken early (cannot happen today): wait for the deadline again.
                ctx.wake_current_at(until);
            }
            return Poll::Pending;
        }
        let pid = ctx.current();
        if let Some(g) = self.res.try_take(self.priority, pid) {
            return Poll::Ready(Ok(g));
        }
        let Some(pid) = pid else {
            // Polled outside the simulation: it could never be woken, so fail closed by waiting.
            return Poll::Pending;
        };
        let grant;
        {
            let mut st = self.res.state.borrow_mut();
            grant = Rc::new(Grant {
                granted: Cell::new(false),
                preempted: st.holders.is_some().then(|| Rc::new(Cell::new(false))),
            });
            let at = insert_position(&st.waiters, self.priority, |w| w.priority);
            st.waiters.insert(
                at,
                Waiter {
                    pid,
                    priority: self.priority,
                    grant: Rc::clone(&grant),
                },
            );
        }
        let until = deadline(now, self.patience);
        if let Some(until) = until {
            ctx.wake_current_at(until);
        }
        self.waiting = Some((grant, until));
        Poll::Pending
    }
}

/// One unit of a [`Resource`]; dropping it gives the unit back.
pub struct ResourceGuard {
    res: Resource,
    /// For a preemptive resource: set when the unit was taken away.
    preempted: Option<Rc<Cell<bool>>>,
}

impl ResourceGuard {
    /// Whether a request of higher priority took this unit away ([`Resource::preemptive`]).
    /// A preempted guard holds nothing, and dropping it gives nothing back.
    pub fn is_preempted(&self) -> bool {
        self.preempted.as_ref().is_some_and(|p| p.get())
    }
}

impl Drop for ResourceGuard {
    fn drop(&mut self) {
        if !self.is_preempted() {
            self.res.release(self.preempted.as_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sim;

    type Log<T> = Rc<RefCell<Vec<T>>>;

    fn log<T>() -> Log<T> {
        Rc::new(RefCell::new(Vec::new()))
    }

    #[test]
    fn resource_serves_first_come_first_served() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let server = Resource::new(&ctx, 1);
        let log = log();
        for (name, arrive) in [("a", 0.0), ("b", 1.0), ("c", 2.0)] {
            let (c, s, l) = (ctx.clone(), server.clone(), Rc::clone(&log));
            sim.spawn(async move {
                c.hold(arrive).await;
                let _unit = s.acquire().await;
                l.borrow_mut().push((name, c.now()));
                c.hold(5.0).await;
            });
        }
        sim.run();
        assert_eq!(*log.borrow(), [("a", 0.0), ("b", 5.0), ("c", 10.0)]);
        assert_eq!(server.in_use(), 0);
        assert_eq!(server.queue_len(), 0);
    }

    #[test]
    fn resource_capacity_two() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let servers = Resource::new(&ctx, 2);
        let starts = log();
        for _ in 0..4 {
            let (c, s, l) = (ctx.clone(), servers.clone(), Rc::clone(&starts));
            sim.spawn(async move {
                let _unit = s.acquire().await;
                l.borrow_mut().push(c.now());
                c.hold(3.0).await;
            });
        }
        sim.run_until(1.0);
        assert_eq!(servers.in_use(), 2);
        assert_eq!(servers.queue_len(), 2);
        sim.run();
        assert_eq!(*starts.borrow(), [0.0, 0.0, 3.0, 3.0]);
    }

    #[test]
    fn same_model_same_trace() {
        fn trace() -> Vec<(u32, f64)> {
            let mut sim = Sim::new();
            let ctx = sim.ctx();
            let res = Resource::new(&ctx, 1);
            let log = log();
            for i in 0..20u32 {
                let (c, r, l) = (ctx.clone(), res.clone(), Rc::clone(&log));
                sim.spawn(async move {
                    c.hold(f64::from(i % 3)).await;
                    let _u = r.acquire().await;
                    c.hold(0.5 + f64::from(i % 4)).await;
                    l.borrow_mut().push((i, c.now()));
                });
            }
            sim.run();
            log.borrow().clone()
        }
        assert_eq!(trace(), trace());
    }

    /// One server busy until 10; waiters arrive at 1..5 with priorities. They are served by
    /// priority, then in arrival order.
    #[test]
    fn waiters_are_served_by_priority_then_in_arrival_order() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let server = Resource::new(&ctx, 1);
        let order = log();
        let (c, s) = (ctx.clone(), server.clone());
        sim.spawn(async move {
            let _u = s.acquire().await;
            c.hold(10.0).await;
        });
        for (name, arrive, prio) in [
            ("a", 1.0, 0),
            ("b", 2.0, 5),
            ("c", 3.0, 0),
            ("d", 4.0, 5),
            ("e", 5.0, 9),
        ] {
            let (c, s, l) = (ctx.clone(), server.clone(), Rc::clone(&order));
            sim.spawn(async move {
                c.hold(arrive).await;
                let _u = s.acquire_with_priority(prio).await;
                l.borrow_mut().push((name, c.now()));
                c.hold(1.0).await;
            });
        }
        sim.run();
        assert_eq!(
            *order.borrow(),
            [
                ("e", 10.0),
                ("b", 11.0),
                ("d", 12.0),
                ("a", 13.0),
                ("c", 14.0)
            ]
        );
    }

    /// A customer who will wait at most 2 leaves at the deadline and its queue entry with it;
    /// one with patience 5 is served at 4.
    #[test]
    fn acquire_within_times_out_and_leaves_the_queue() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let server = Resource::new(&ctx, 1);
        let log = log();
        let (c, s) = (ctx.clone(), server.clone());
        sim.spawn(async move {
            let _u = s.acquire().await;
            c.hold(4.0).await;
        });
        for (name, patience) in [("impatient", 2.0), ("patient", 5.0)] {
            let (c, s, l) = (ctx.clone(), server.clone(), Rc::clone(&log));
            sim.spawn(async move {
                match s.acquire_within(patience).await {
                    Ok(_u) => {
                        l.borrow_mut().push((name, "served", c.now()));
                        c.hold(1.0).await;
                    }
                    Err(TimedOut) => l.borrow_mut().push((name, "reneged", c.now())),
                }
            });
        }
        sim.run_until(2.0);
        assert_eq!(
            server.queue_len(),
            1,
            "the reneged request has left the queue"
        );
        sim.run();
        assert_eq!(
            *log.borrow(),
            [("impatient", "reneged", 2.0), ("patient", "served", 4.0)]
        );
        assert_eq!((server.in_use(), server.queue_len()), (0, 0));
        assert_eq!(sim.live_processes(), 0);
    }

    /// A unit handed over exactly at the deadline is taken, not lost.
    #[test]
    fn a_unit_freed_at_the_deadline_is_taken() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let server = Resource::new(&ctx, 1);
        let got = log();
        let (c, s) = (ctx.clone(), server.clone());
        sim.spawn(async move {
            let _u = s.acquire().await;
            c.hold(3.0).await;
        });
        let (c, s, g) = (ctx.clone(), server.clone(), Rc::clone(&got));
        sim.spawn(async move {
            let r = s.acquire_within(3.0).await;
            g.borrow_mut().push((r.is_ok(), c.now()));
        });
        // Zero patience on a busy server balks at once.
        let (s, g) = (server.clone(), Rc::clone(&got));
        let c = ctx.clone();
        sim.spawn(async move {
            c.hold(1.0).await;
            let r = s.acquire_within(0.0).await;
            g.borrow_mut().push((r.is_ok(), c.now()));
        });
        sim.run();
        assert_eq!(*got.borrow(), [(false, 1.0), (true, 3.0)]);
        assert_eq!((server.in_use(), server.queue_len()), (0, 0));
    }

    /// A job of priority 0 needs 10 units of work; an urgent job of priority 1 arrives at 4,
    /// preempts it and works 3; the first job resumes at 7 and finishes its last 6 at 13.
    #[test]
    fn preemption_takes_the_unit_and_tells_the_holder() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let machine = Resource::preemptive(&ctx, 1);
        let log = log();
        for (name, arrive, prio, work) in [("normal", 0.0, 0, 10.0), ("urgent", 4.0, 1, 3.0)] {
            let (c, m, l) = (ctx.clone(), machine.clone(), Rc::clone(&log));
            sim.spawn(async move {
                c.hold(arrive).await;
                let mut left = work;
                loop {
                    let unit = m.acquire_with_priority(prio).await;
                    l.borrow_mut().push((name, "start", c.now()));
                    match c.hold_interruptible(left).await {
                        Ok(()) => break,
                        Err(i) => {
                            assert_eq!(i.cause, InterruptCause::Preempted);
                            assert!(unit.is_preempted());
                            l.borrow_mut().push((name, "preempted", c.now()));
                            left = i.remaining;
                        }
                    }
                }
                l.borrow_mut().push((name, "done", c.now()));
            });
        }
        sim.run();
        assert_eq!(
            *log.borrow(),
            [
                ("normal", "start", 0.0),
                ("urgent", "start", 4.0),
                ("normal", "preempted", 4.0),
                ("urgent", "done", 7.0),
                ("normal", "start", 7.0),
                ("normal", "done", 13.0),
            ]
        );
        assert_eq!((machine.in_use(), machine.queue_len()), (0, 0));
    }

    /// Two units held at priorities 2 and 1 (taken in that order) and one of priority 1 taken
    /// last: a priority-5 request preempts the latest holder of the lowest priority. Equal
    /// priority never preempts.
    #[test]
    fn preemption_picks_the_lowest_priority_then_the_latest() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let pool = Resource::preemptive(&ctx, 3);
        let log = log();
        for (name, arrive, prio) in [
            ("p2", 0.0, 2),
            ("p1-early", 1.0, 1),
            ("p1-late", 2.0, 1),
            ("p2-new", 3.0, 2),
            ("p5", 4.0, 5),
        ] {
            let (c, m, l) = (ctx.clone(), pool.clone(), Rc::clone(&log));
            sim.spawn(async move {
                c.hold(arrive).await;
                let _unit = m.acquire_with_priority(prio).await;
                if let Err(i) = c.hold_interruptible(100.0).await {
                    l.borrow_mut().push((name, c.now(), i.remaining));
                }
            });
        }
        sim.run_until(50.0);
        // p2-new preempted p1-late at 3 (2 > 1), then p5 preempted p1-early at 4.
        assert_eq!(
            *log.borrow(),
            [("p1-late", 3.0, 99.0), ("p1-early", 4.0, 97.0)]
        );
        assert_eq!((pool.in_use(), pool.queue_len()), (3, 0));
        sim.run();
        assert_eq!(pool.in_use(), 0);
    }

    /// #42: a holder in a plain hold keeps its unit. "low" works 10 in a plain hold and then 5
    /// in an interruptible one; "urgent", asking at 4, waits for the unit instead of taking it
    /// while "low" still works (which left two users on one unit), and no interrupt is left to
    /// cut "low"'s later hold short.
    #[test]
    fn preemption_spares_a_holder_in_a_plain_hold() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let machine = Resource::preemptive(&ctx, 1);
        let log = log();
        let (c, m, l) = (ctx.clone(), machine.clone(), Rc::clone(&log));
        sim.spawn(async move {
            let unit = m.acquire_with_priority(0).await;
            c.hold(10.0).await;
            l.borrow_mut().push(("low", "plain hold over", c.now()));
            let r = c.hold_interruptible(5.0).await;
            l.borrow_mut().push((
                "low",
                if r.is_ok() { "done" } else { "interrupted" },
                c.now(),
            ));
            assert!(!unit.is_preempted());
        });
        let (c, m, l) = (ctx.clone(), machine.clone(), Rc::clone(&log));
        sim.spawn(async move {
            c.hold(4.0).await;
            let _unit = m.acquire_with_priority(1).await;
            l.borrow_mut().push(("urgent", "start", c.now()));
            c.hold(1.0).await;
        });
        sim.run_until(4.0);
        assert_eq!((machine.in_use(), machine.queue_len()), (1, 1));
        sim.run();
        assert_eq!(
            *log.borrow(),
            [
                ("low", "plain hold over", 10.0),
                ("low", "done", 15.0),
                ("urgent", "start", 15.0),
            ]
        );
        assert_eq!((machine.in_use(), machine.queue_len()), (0, 0));
    }

    #[test]
    fn non_preemptive_resources_never_preempt() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let server = Resource::new(&ctx, 1);
        let log = log();
        for (name, arrive, prio) in [("low", 0.0, 0), ("high", 1.0, 9)] {
            let (c, s, l) = (ctx.clone(), server.clone(), Rc::clone(&log));
            sim.spawn(async move {
                c.hold(arrive).await;
                let unit = s.acquire_with_priority(prio).await;
                let r = c.hold_interruptible(5.0).await;
                l.borrow_mut()
                    .push((name, c.now(), r.is_ok(), unit.is_preempted()));
            });
        }
        sim.run();
        assert_eq!(
            *log.borrow(),
            [("low", 5.0, true, false), ("high", 10.0, true, false)]
        );
    }
}
