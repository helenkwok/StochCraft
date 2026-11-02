//! Processes on simulated time.
//!
//! A process is a future. The executor polls a process only when one of its own events comes
//! off the [`EventQueue`]; the futures here never use the `Waker`, they register the process's
//! next event in the kernel instead. Because each process waits on at most one thing at a time,
//! "my event came up" is the same as "what I was waiting for happened".

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use crate::EventQueue;

type ProcFuture = Pin<Box<dyn Future<Output = ()>>>;

/// Priority of a process start or a resource hand-over at the current time: ahead of ordinary
/// events at the same instant, so a released unit goes to the waiter before anyone else asks.
const WAKE_PRIORITY: i64 = 1;

struct Kernel {
    queue: EventQueue<u64>,
    /// The process currently being polled.
    current: Option<u64>,
    next_pid: u64,
    /// Processes spawned while another was being polled, not yet in `procs`.
    spawned: Vec<(u64, ProcFuture)>,
}

/// A simulation: processes plus the event queue that drives them.
pub struct Sim {
    kernel: Rc<RefCell<Kernel>>,
    /// Process futures by id. `None` while that process is being polled.
    procs: BTreeMap<u64, Option<ProcFuture>>,
}

/// The handle a process uses to reach the simulation. Cheap to clone.
#[derive(Clone)]
pub struct Ctx {
    kernel: Rc<RefCell<Kernel>>,
}

impl Default for Sim {
    fn default() -> Self {
        Self::new()
    }
}

impl Sim {
    /// An empty simulation with the clock at 0.
    pub fn new() -> Self {
        let kernel = Kernel {
            queue: EventQueue::new(0.0),
            current: None,
            next_pid: 0,
            spawned: Vec::new(),
        };
        Self {
            kernel: Rc::new(RefCell::new(kernel)),
            procs: BTreeMap::new(),
        }
    }

    /// A handle for building processes and resources.
    pub fn ctx(&self) -> Ctx {
        Ctx {
            kernel: Rc::clone(&self.kernel),
        }
    }

    pub fn now(&self) -> f64 {
        self.kernel.borrow().queue.now()
    }

    /// Starts a process at the current time. Returns its id.
    pub fn spawn(&mut self, process: impl Future<Output = ()> + 'static) -> u64 {
        let pid = self.ctx().spawn(process);
        self.adopt_spawned();
        pid
    }

    /// Number of processes that have not finished.
    pub fn live_processes(&self) -> usize {
        self.procs.len() + self.kernel.borrow().spawned.len()
    }

    fn adopt_spawned(&mut self) {
        let spawned = std::mem::take(&mut self.kernel.borrow_mut().spawned);
        for (pid, fut) in spawned {
            self.procs.insert(pid, Some(fut));
        }
    }

    /// Runs events until the next one would be later than `until`, then sets the clock to
    /// `until`. Returns the number of process activations.
    pub fn run_until(&mut self, until: f64) -> u64 {
        let mut steps = 0;
        loop {
            let next = self.kernel.borrow().queue.peek_time();
            match next {
                Some(t) if t <= until => {}
                _ => break,
            }
            let Some(ev) = self.kernel.borrow_mut().queue.pop() else {
                break;
            };
            self.step(ev.payload);
            steps += 1;
        }
        self.kernel.borrow_mut().queue.advance_to(until);
        steps
    }

    /// Runs until no events are left. Returns the number of process activations.
    pub fn run(&mut self) -> u64 {
        self.run_until(f64::MAX)
    }

    fn step(&mut self, pid: u64) {
        // A process that already finished may still have a stale event; skip it.
        let Some(slot) = self.procs.get_mut(&pid) else {
            return;
        };
        let Some(mut fut) = slot.take() else { return };
        self.kernel.borrow_mut().current = Some(pid);
        let mut cx = Context::from_waker(Waker::noop());
        let done = fut.as_mut().poll(&mut cx).is_ready();
        self.kernel.borrow_mut().current = None;
        if done {
            self.procs.remove(&pid);
        } else if let Some(slot) = self.procs.get_mut(&pid) {
            *slot = Some(fut);
        }
        self.adopt_spawned();
    }
}

impl Ctx {
    pub fn now(&self) -> f64 {
        self.kernel.borrow().queue.now()
    }

    /// Starts a process at the current time. Returns its id.
    pub fn spawn(&self, process: impl Future<Output = ()> + 'static) -> u64 {
        let mut k = self.kernel.borrow_mut();
        let pid = k.next_pid;
        k.next_pid += 1;
        k.spawned.push((pid, Box::pin(process)));
        let now = k.queue.now();
        // Scheduling at `now` cannot fail: `now` is finite and not in the past.
        let _ = k.queue.schedule_at(now, WAKE_PRIORITY, pid);
        pid
    }

    /// Waits `delay` time units. A negative or non-finite delay waits zero time.
    pub fn hold(&self, delay: f64) -> Hold {
        let delay = if delay.is_finite() && delay > 0.0 {
            delay
        } else {
            0.0
        };
        Hold {
            ctx: self.clone(),
            delay,
            scheduled: false,
        }
    }

    /// The id of the process being run, if called from inside one.
    pub fn current(&self) -> Option<u64> {
        self.kernel.borrow().current
    }

    fn wake_now(&self, pid: u64) {
        let mut k = self.kernel.borrow_mut();
        let now = k.queue.now();
        let _ = k.queue.schedule_at(now, WAKE_PRIORITY, pid);
    }
}

/// The future returned by [`Ctx::hold`].
pub struct Hold {
    ctx: Ctx,
    delay: f64,
    scheduled: bool,
}

impl Future for Hold {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.scheduled {
            return Poll::Ready(());
        }
        let mut k = self.ctx.kernel.borrow_mut();
        let Some(pid) = k.current else {
            // Polled outside the simulation: nothing to wait on.
            return Poll::Ready(());
        };
        let delay = self.delay;
        let _ = k.queue.schedule_in(delay, 0, pid);
        drop(k);
        self.scheduled = true;
        Poll::Pending
    }
}

struct ResourceState {
    capacity: u64,
    in_use: u64,
    /// Waiting processes in arrival order, each with the flag set when a unit is handed to it.
    waiters: VecDeque<(u64, Rc<Cell<bool>>)>,
}

/// A pool of identical units (a server, a machine, a forklift fleet). Processes acquire one unit
/// and give it back by dropping the guard; waiters are served first come, first served.
#[derive(Clone)]
pub struct Resource {
    ctx: Ctx,
    state: Rc<RefCell<ResourceState>>,
}

impl Resource {
    /// A resource with `capacity` units (at least 1).
    pub fn new(ctx: &Ctx, capacity: u64) -> Self {
        let state = ResourceState {
            capacity: capacity.max(1),
            in_use: 0,
            waiters: VecDeque::new(),
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

    /// Waits for a unit. Hold the returned guard while using it.
    pub fn acquire(&self) -> Acquire {
        Acquire {
            res: self.clone(),
            granted: None,
        }
    }

    fn release(&self) {
        let next = {
            let mut st = self.state.borrow_mut();
            match st.waiters.pop_front() {
                // Hand the unit straight to the next waiter: `in_use` stays the same.
                Some((pid, flag)) => {
                    flag.set(true);
                    Some(pid)
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

/// The future returned by [`Resource::acquire`].
pub struct Acquire {
    res: Resource,
    granted: Option<Rc<Cell<bool>>>,
}

impl Future for Acquire {
    type Output = ResourceGuard;
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<ResourceGuard> {
        if let Some(flag) = &self.granted {
            return if flag.get() {
                Poll::Ready(ResourceGuard {
                    res: self.res.clone(),
                })
            } else {
                Poll::Pending
            };
        }
        let pid = self.res.ctx.current();
        let mut st = self.res.state.borrow_mut();
        if st.in_use < st.capacity && st.waiters.is_empty() {
            st.in_use += 1;
            drop(st);
            return Poll::Ready(ResourceGuard {
                res: self.res.clone(),
            });
        }
        let Some(pid) = pid else {
            // Polled outside the simulation: it could never be woken, so fail closed by waiting.
            return Poll::Pending;
        };
        let flag = Rc::new(Cell::new(false));
        st.waiters.push_back((pid, Rc::clone(&flag)));
        drop(st);
        self.granted = Some(flag);
        Poll::Pending
    }
}

/// One unit of a [`Resource`]; dropping it gives the unit back.
pub struct ResourceGuard {
    res: Resource,
}

impl Drop for ResourceGuard {
    fn drop(&mut self) {
        self.res.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_advances_time() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = Rc::clone(&log);
        sim.spawn(async move {
            l.borrow_mut().push(ctx.now());
            ctx.hold(2.0).await;
            l.borrow_mut().push(ctx.now());
            ctx.hold(0.5).await;
            l.borrow_mut().push(ctx.now());
        });
        sim.run();
        assert_eq!(*log.borrow(), [0.0, 2.0, 2.5]);
        assert_eq!(sim.live_processes(), 0);
    }

    #[test]
    fn run_until_stops_and_sets_the_clock() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let ticks = Rc::new(Cell::new(0u32));
        let t = Rc::clone(&ticks);
        sim.spawn(async move {
            loop {
                ctx.hold(1.0).await;
                t.set(t.get() + 1);
            }
        });
        sim.run_until(10.5);
        assert_eq!(ticks.get(), 10);
        assert_eq!(sim.now(), 10.5);
        sim.run_until(12.0);
        assert_eq!(ticks.get(), 12);
    }

    #[test]
    fn processes_can_spawn_processes() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = Rc::clone(&log);
        sim.spawn(async move {
            for i in 0..3u32 {
                ctx.hold(1.0).await;
                let (c, l2) = (ctx.clone(), Rc::clone(&l));
                ctx.spawn(async move {
                    c.hold(10.0).await;
                    l2.borrow_mut().push((i, c.now()));
                });
            }
        });
        sim.run();
        assert_eq!(*log.borrow(), [(0, 11.0), (1, 12.0), (2, 13.0)]);
    }

    #[test]
    fn resource_serves_first_come_first_served() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let server = Resource::new(&ctx, 1);
        let log = Rc::new(RefCell::new(Vec::new()));
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
        let starts = Rc::new(RefCell::new(Vec::new()));
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
            let log = Rc::new(RefCell::new(Vec::new()));
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
}
