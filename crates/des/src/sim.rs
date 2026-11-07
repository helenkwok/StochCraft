//! Processes on simulated time.
//!
//! A process is a future. The executor polls a process only when one of its own events comes
//! off the [`EventQueue`]; the futures here never use the `Waker`, they register the process's
//! next event in the kernel instead.
//!
//! A process may have more than one event pending: a wait with a deadline has its timer and
//! the hand-over that may beat it, and an interrupted hold leaves its timer behind. Whichever
//! comes first wakes the process; the others are stale. Since every wait a process makes is
//! registered while it runs, an event scheduled before the process last started running
//! belongs to a wait that is over. So each process remembers the queue's sequence number at
//! its last activation, and events for it with an earlier sequence number are skipped.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use crate::EventQueue;

type ProcFuture = Pin<Box<dyn Future<Output = ()>>>;

/// Priority of a process start or a hand-over at the current time: ahead of ordinary events at
/// the same instant, so a released unit goes to the waiter before anyone else asks.
pub(crate) const WAKE_PRIORITY: i64 = 1;

/// A live process.
struct Proc {
    /// Its future; `None` while it is being polled.
    fut: Option<ProcFuture>,
    /// The queue's next sequence number when it last started running: events for it scheduled
    /// earlier are stale.
    activated: u64,
    /// Whether it is in a wait that an interrupt ends ([`Ctx::hold_interruptible`]).
    interruptible: bool,
    /// Interrupts not yet seen by an interruptible wait, oldest first.
    interrupts: VecDeque<Interrupt>,
    /// What it belongs to, for reports ([`Sim::spawn_tagged`]); processes it starts inherit it.
    tag: Option<u64>,
}

struct Kernel {
    queue: EventQueue<u64>,
    /// The process currently being polled.
    current: Option<u64>,
    next_pid: u64,
    procs: BTreeMap<u64, Proc>,
    /// The first wake that could not be scheduled: the process and the time it happened.
    unschedulable: Option<(u64, f64)>,
}

impl Kernel {
    /// Schedules `pid` at `time`. Callers keep `time` not in the past, but a wait that ends
    /// later than the largest time that can be represented (`now + delay` overflowing) cannot be
    /// scheduled: the process would never wake. The first such failure is kept, and
    /// [`Sim::run_watched`] stops with [`Stopped::Unschedulable`].
    fn schedule(&mut self, pid: u64, time: f64, priority: i64) {
        if self.queue.schedule_at(time, priority, pid).is_err() && self.unschedulable.is_none() {
            self.unschedulable = Some((pid, self.queue.now()));
        }
    }

    fn current_proc(&mut self) -> Option<&mut Proc> {
        let pid = self.current?;
        self.procs.get_mut(&pid)
    }
}

/// How often (in events) [`Sim::run_watched`] asks whether it has been cancelled: often enough
/// to stop within a fraction of a millisecond, rarely enough to cost nothing measurable.
const CANCEL_CHECK_EVERY: u64 = 1024;

/// What [`Sim::run_watched`] watches for.
#[derive(Clone, Copy)]
pub struct Watch<'a> {
    /// The most events that may happen at one simulated time. A model in which processes can
    /// loop without time moving (say, steps that all take zero time, joined in a cycle) would
    /// otherwise never end; past this many events at one time the run stops as
    /// [`Stopped::TimeStopped`].
    pub max_events_at_one_time: u64,
    /// Asked every so often; once it returns true the run stops as [`Stopped::Cancelled`].
    pub cancelled: &'a dyn Fn() -> bool,
}

/// Why [`Sim::run_watched`] stopped before its end time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stopped {
    /// The watch was cancelled, at this simulated time.
    Cancelled { time: f64 },
    /// More than `events` events happened at simulated time `time`: the model seems to loop
    /// without time moving. `tag` is the tag of the process whose event passed the limit (see
    /// [`Sim::spawn_tagged`]), which is part of the loop.
    TimeStopped {
        time: f64,
        events: u64,
        tag: Option<u64>,
    },
    /// At simulated time `time`, a process with tag `tag` asked to wait until later than the
    /// largest time that can be represented, so it could never wake.
    Unschedulable { time: f64, tag: Option<u64> },
}

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stopped::Cancelled { .. } => f.write_str("the run was cancelled"),
            Stopped::TimeStopped { time, events, .. } => write!(
                f,
                "simulated time stopped moving at {time}: more than {events} events happened \
                 at that time"
            ),
            Stopped::Unschedulable { time, .. } => write!(
                f,
                "at {time} a process asked to wait until later than the largest time that can \
                 be represented"
            ),
        }
    }
}

impl std::error::Error for Stopped {}

/// A simulation: processes plus the event queue that drives them.
pub struct Sim {
    kernel: Rc<RefCell<Kernel>>,
}

impl Drop for Sim {
    /// Processes hold handles to the kernel that holds them; dropping them here breaks that
    /// cycle. They are dropped outside the kernel's borrow, since dropping a process may give
    /// back a resource unit.
    fn drop(&mut self) {
        let procs = std::mem::take(&mut self.kernel.borrow_mut().procs);
        drop(procs);
    }
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
            procs: BTreeMap::new(),
            unschedulable: None,
        };
        Self {
            kernel: Rc::new(RefCell::new(kernel)),
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
        self.ctx().spawn(process)
    }

    /// Starts a process at the current time, tagged with `tag` (for example the model element it
    /// runs), as are the processes it starts. [`Stopped`] names the tag of the process involved.
    /// Returns its id.
    pub fn spawn_tagged(&mut self, tag: u64, process: impl Future<Output = ()> + 'static) -> u64 {
        self.ctx().spawn_with(Some(tag), process)
    }

    /// Number of processes that have not finished.
    pub fn live_processes(&self) -> usize {
        self.kernel.borrow().procs.len()
    }

    /// Runs events until the next one would be later than `until`, then sets the clock to
    /// `until`. Returns the number of process activations.
    ///
    /// It runs for as long as the model has events before `until`, which for a model whose
    /// processes loop without simulated time moving is forever; [`Sim::run_watched`] guards
    /// against that, and can be cancelled.
    pub fn run_until(&mut self, until: f64) -> u64 {
        let mut steps = 0;
        while let Some(ev) = self.pop_until(until) {
            self.step(ev.payload, ev.seq);
            steps += 1;
        }
        self.kernel.borrow_mut().queue.advance_to(until);
        steps
    }

    /// As [`Sim::run_until`], but stops early, with the reason, when `watch` is cancelled, when
    /// more than [`Watch::max_events_at_one_time`] events happen at one simulated time, or when a
    /// process asks to wait beyond the largest time that can be represented. The checks only
    /// read counters, so a run that is not stopped runs exactly as [`Sim::run_until`] would.
    pub fn run_watched(&mut self, until: f64, watch: &Watch<'_>) -> Result<u64, Stopped> {
        let mut steps: u64 = 0;
        let mut instant = self.now();
        let mut at_instant: u64 = 0;
        loop {
            if steps.is_multiple_of(CANCEL_CHECK_EVERY) && (watch.cancelled)() {
                return Err(Stopped::Cancelled { time: self.now() });
            }
            let Some(ev) = self.pop_until(until) else {
                break;
            };
            if ev.time == instant {
                at_instant += 1;
            } else {
                instant = ev.time;
                at_instant = 1;
            }
            if at_instant > watch.max_events_at_one_time {
                let k = self.kernel.borrow();
                return Err(Stopped::TimeStopped {
                    time: instant,
                    events: watch.max_events_at_one_time,
                    tag: k.procs.get(&ev.payload).and_then(|p| p.tag),
                });
            }
            self.step(ev.payload, ev.seq);
            steps += 1;
            let k = self.kernel.borrow();
            if let Some((pid, time)) = k.unschedulable {
                return Err(Stopped::Unschedulable {
                    time,
                    tag: k.procs.get(&pid).and_then(|p| p.tag),
                });
            }
        }
        self.kernel.borrow_mut().queue.advance_to(until);
        Ok(steps)
    }

    /// When the next event happens, if any is scheduled.
    pub fn next_event_time(&self) -> Option<f64> {
        self.kernel.borrow().queue.peek_time()
    }

    /// Takes the next event off the queue if it is not later than `until`.
    fn pop_until(&mut self, until: f64) -> Option<crate::Event<u64>> {
        let mut k = self.kernel.borrow_mut();
        match k.queue.peek_time() {
            Some(t) if t <= until => k.queue.pop(),
            _ => None,
        }
    }

    /// Runs until no events are left. Returns the number of process activations.
    pub fn run(&mut self) -> u64 {
        self.run_until(f64::MAX)
    }

    /// Runs process `pid` for its event with sequence number `seq`.
    fn step(&mut self, pid: u64, seq: u64) {
        let mut fut = {
            let mut k = self.kernel.borrow_mut();
            let next_seq = k.queue.next_seq();
            // A finished process, or an event another one beat (see the module notes): skip.
            let Some(p) = k.procs.get_mut(&pid) else {
                return;
            };
            if seq < p.activated {
                return;
            }
            let Some(fut) = p.fut.take() else { return };
            p.activated = next_seq;
            p.interruptible = false;
            k.current = Some(pid);
            fut
        };
        let mut cx = Context::from_waker(Waker::noop());
        let done = fut.as_mut().poll(&mut cx).is_ready();
        let finished = {
            let mut k = self.kernel.borrow_mut();
            k.current = None;
            if done {
                k.procs.remove(&pid)
            } else {
                if let Some(p) = k.procs.get_mut(&pid) {
                    p.fut = Some(fut);
                }
                None
            }
        };
        // Dropped outside the borrow: dropping a process may give back a resource unit.
        drop(finished);
    }
}

impl Ctx {
    pub fn now(&self) -> f64 {
        self.kernel.borrow().queue.now()
    }

    /// Starts a process at the current time, with the tag of the process starting it (see
    /// [`Sim::spawn_tagged`]). Returns its id.
    pub fn spawn(&self, process: impl Future<Output = ()> + 'static) -> u64 {
        let tag = self.kernel.borrow_mut().current_proc().and_then(|p| p.tag);
        self.spawn_with(tag, process)
    }

    fn spawn_with(&self, tag: Option<u64>, process: impl Future<Output = ()> + 'static) -> u64 {
        let mut k = self.kernel.borrow_mut();
        let pid = k.next_pid;
        k.next_pid += 1;
        k.procs.insert(
            pid,
            Proc {
                fut: Some(Box::pin(process)),
                activated: 0,
                interruptible: false,
                interrupts: VecDeque::new(),
                tag,
            },
        );
        let now = k.queue.now();
        k.schedule(pid, now, WAKE_PRIORITY);
        pid
    }

    /// Waits `delay` time units. A negative or non-finite delay waits zero time.
    pub fn hold(&self, delay: f64) -> Hold {
        Hold {
            ctx: self.clone(),
            delay: clamp_delay(delay),
            until: None,
        }
    }

    /// Waits `delay` time units unless the process is interrupted first ([`Ctx::interrupt`],
    /// or a preemptive [`crate::Resource`] taking its unit). An interrupt that arrived while
    /// the process was busy with something else ends this wait at once. A negative or
    /// non-finite delay waits zero time.
    pub fn hold_interruptible(&self, delay: f64) -> HoldInterruptible {
        HoldInterruptible {
            ctx: self.clone(),
            delay: clamp_delay(delay),
            until: None,
        }
    }

    /// Interrupts process `pid` with `cause`. If it is in [`Ctx::hold_interruptible`] it wakes
    /// now, ahead of other events at this instant; otherwise its next interruptible wait ends
    /// at once. Interrupts are delivered one per wait, oldest first. Returns `false` if the
    /// process has finished (the interrupt is dropped).
    pub fn interrupt(&self, pid: u64, cause: u64) -> bool {
        self.deliver(pid, InterruptCause::Other(cause))
    }

    pub(crate) fn deliver(&self, pid: u64, cause: InterruptCause) -> bool {
        let mut k = self.kernel.borrow_mut();
        let at = k.queue.now();
        let Some(p) = k.procs.get_mut(&pid) else {
            return false;
        };
        p.interrupts.push_back(Interrupt { cause, at });
        if p.interruptible {
            p.interruptible = false;
            k.schedule(pid, at, WAKE_PRIORITY);
        }
        true
    }

    /// Whether process `pid` is in [`Ctx::hold_interruptible`], so an interrupt would wake it now.
    pub(crate) fn is_interruptible(&self, pid: u64) -> bool {
        self.kernel
            .borrow()
            .procs
            .get(&pid)
            .is_some_and(|p| p.interruptible)
    }

    /// The id of the process being run, if called from inside one.
    pub fn current(&self) -> Option<u64> {
        self.kernel.borrow().current
    }

    /// Wakes `pid` now, ahead of ordinary events at this instant.
    pub(crate) fn wake_now(&self, pid: u64) {
        let mut k = self.kernel.borrow_mut();
        let now = k.queue.now();
        k.schedule(pid, now, WAKE_PRIORITY);
    }

    /// Schedules the running process to wake at `time` (at least now). Returns `false` outside
    /// a process.
    pub(crate) fn wake_current_at(&self, time: f64) -> bool {
        let mut k = self.kernel.borrow_mut();
        let Some(pid) = k.current else {
            return false;
        };
        let time = time.max(k.queue.now());
        k.schedule(pid, time, 0);
        true
    }

    fn set_interruptible(&self) {
        if let Some(p) = self.kernel.borrow_mut().current_proc() {
            p.interruptible = true;
        }
    }

    fn take_interrupt(&self) -> Option<Interrupt> {
        self.kernel
            .borrow_mut()
            .current_proc()
            .and_then(|p| p.interrupts.pop_front())
    }
}

fn clamp_delay(delay: f64) -> f64 {
    if delay.is_finite() && delay > 0.0 {
        delay
    } else {
        0.0
    }
}

/// The deadline of a wait with `patience` from now: `None` (no deadline) when the patience is
/// infinite; zero for a negative or NaN patience.
pub(crate) fn deadline(now: f64, patience: f64) -> Option<f64> {
    if patience == f64::INFINITY {
        None
    } else if patience > 0.0 {
        Some(now + patience)
    } else {
        Some(now)
    }
}

/// The future returned by [`Ctx::hold`].
pub struct Hold {
    ctx: Ctx,
    delay: f64,
    /// When the hold ends, once it has started.
    until: Option<f64>,
}

impl Future for Hold {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        let now = self.ctx.now();
        let until = match self.until {
            // The timer came up (only this hold's own events wake a process in it).
            Some(until) if now >= until => return Poll::Ready(()),
            Some(until) => until,
            None => now + self.delay,
        };
        if !self.ctx.wake_current_at(until) {
            // Polled outside the simulation: nothing to wait on.
            return Poll::Ready(());
        }
        self.until = Some(until);
        Poll::Pending
    }
}

/// Why a process was interrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptCause {
    /// A preemptive [`crate::Resource`] gave the process's unit to a request of higher
    /// priority. The process's [`crate::ResourceGuard`] no longer holds a unit.
    Preempted,
    /// [`Ctx::interrupt`] with this cause (a breakdown, a recall, ...).
    Other(u64),
}

/// An interrupt as a process receives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interrupt {
    pub cause: InterruptCause,
    /// When it was sent.
    pub at: f64,
}

/// How an interruptible hold ended early.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interrupted {
    pub cause: InterruptCause,
    /// When the interrupt was sent.
    pub at: f64,
    /// The time the hold still had to run when it ended (its whole delay if the interrupt
    /// was already waiting when it began).
    pub remaining: f64,
}

/// The future returned by [`Ctx::hold_interruptible`]: `Ok(())` when the delay has passed,
/// `Err` if the process was interrupted first.
pub struct HoldInterruptible {
    ctx: Ctx,
    delay: f64,
    until: Option<f64>,
}

impl Future for HoldInterruptible {
    type Output = Result<(), Interrupted>;
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let now = self.ctx.now();
        if let Some(i) = self.ctx.take_interrupt() {
            let remaining = self.until.map_or(self.delay, |u| (u - now).max(0.0));
            return Poll::Ready(Err(Interrupted {
                cause: i.cause,
                at: i.at,
                remaining,
            }));
        }
        let until = match self.until {
            Some(until) if now >= until => return Poll::Ready(Ok(())),
            Some(until) => until,
            None => now + self.delay,
        };
        if !self.ctx.wake_current_at(until) {
            return Poll::Ready(Ok(()));
        }
        self.ctx.set_interruptible();
        self.until = Some(until);
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

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

    /// A process that loops without time moving stops the run at the limit, naming the tag of
    /// the process looping, which it got from the process that started it. The run is on a
    /// thread of its own, so that if the guard breaks the test fails instead of hanging.
    #[test]
    fn run_watched_stops_when_time_stops_moving() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut sim = Sim::new();
            let ctx = sim.ctx();
            sim.spawn_tagged(7, async move {
                ctx.hold(2.0).await;
                let c = ctx.clone();
                ctx.spawn(async move {
                    loop {
                        c.hold(0.0).await;
                    }
                });
            });
            let watch = Watch {
                max_events_at_one_time: 100,
                cancelled: &|| false,
            };
            let _ = tx.send(sim.run_watched(10.0, &watch));
        });
        let r = rx.recv_timeout(std::time::Duration::from_secs(30));
        assert_eq!(
            r,
            Ok(Err(Stopped::TimeStopped {
                time: 2.0,
                events: 100,
                tag: Some(7),
            }))
        );
    }

    /// Cancelling stops a run that would go on for ever; watching changes nothing else.
    #[test]
    fn run_watched_can_be_cancelled_and_otherwise_runs_as_run_until() {
        let ticker = |sim: &mut Sim, ticks: &Rc<Cell<u32>>| {
            let (ctx, t) = (sim.ctx(), Rc::clone(ticks));
            sim.spawn(async move {
                loop {
                    ctx.hold(1.0).await;
                    t.set(t.get() + 1);
                }
            });
        };
        let (mut sim, ticks) = (Sim::new(), Rc::new(Cell::new(0u32)));
        ticker(&mut sim, &ticks);
        let asked = Cell::new(0u32);
        let cancelled = || {
            asked.set(asked.get() + 1);
            asked.get() > 3
        };
        let watch = Watch {
            max_events_at_one_time: 1,
            cancelled: &cancelled,
        };
        // Ends at 1e7 even if cancelling ever stops working, so the test cannot hang.
        let r = sim.run_watched(1e7, &watch);
        assert!(matches!(r, Err(Stopped::Cancelled { .. })), "{r:?}");
        assert_eq!(ticks.get(), 3 * CANCEL_CHECK_EVERY as u32 - 1);

        let (mut plain, plain_ticks) = (Sim::new(), Rc::new(Cell::new(0u32)));
        ticker(&mut plain, &plain_ticks);
        let (mut watched, watched_ticks) = (Sim::new(), Rc::new(Cell::new(0u32)));
        ticker(&mut watched, &watched_ticks);
        let steps = plain.run_until(100.5);
        let watch = Watch {
            max_events_at_one_time: 1,
            cancelled: &|| false,
        };
        assert_eq!(watched.run_watched(100.5, &watch), Ok(steps));
        assert_eq!((plain_ticks.get(), plain.now()), (100, 100.5));
        assert_eq!((watched_ticks.get(), watched.now()), (100, 100.5));
    }

    /// #42: a wait that would end beyond the largest time that can be represented cannot be
    /// scheduled; the process could never wake. The run says so instead of carrying on.
    #[test]
    fn run_watched_reports_a_wait_that_cannot_be_scheduled() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        sim.spawn_tagged(3, async move {
            ctx.hold(f64::MAX).await;
            // f64::MAX + f64::MAX overflows.
            ctx.hold(f64::MAX).await;
        });
        let watch = Watch {
            max_events_at_one_time: 100,
            cancelled: &|| false,
        };
        assert_eq!(
            sim.run_watched(f64::MAX, &watch),
            Err(Stopped::Unschedulable {
                time: f64::MAX,
                tag: Some(3),
            })
        );
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

    /// A machine works 10 time units per part; a breakdown at 4 interrupts it, and it finishes
    /// the remaining 6 after a repair of 3.
    #[test]
    fn interrupt_ends_a_hold_early_with_the_time_remaining() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let log = Rc::new(RefCell::new(Vec::new()));
        let (c, l) = (ctx.clone(), Rc::clone(&log));
        let machine = sim.spawn(async move {
            match c.hold_interruptible(10.0).await {
                Ok(()) => l.borrow_mut().push(("done", c.now(), 0.0)),
                Err(i) => {
                    assert_eq!(i.cause, InterruptCause::Other(7));
                    assert_eq!(i.at, 4.0);
                    l.borrow_mut().push(("broken", c.now(), i.remaining));
                    c.hold(3.0).await;
                    let r = c.hold_interruptible(i.remaining).await;
                    assert!(r.is_ok());
                    l.borrow_mut().push(("done", c.now(), 0.0));
                }
            }
        });
        let c = ctx.clone();
        sim.spawn(async move {
            c.hold(4.0).await;
            assert!(c.interrupt(machine, 7));
        });
        sim.run();
        assert_eq!(
            *log.borrow(),
            [("broken", 4.0, 6.0), ("done", 13.0, 0.0)],
            "the stale timer at 10 must not end the repair hold"
        );
        assert!(
            !ctx.interrupt(machine, 1),
            "finished processes are not interrupted"
        );
    }

    #[test]
    fn an_interrupt_waits_for_the_next_interruptible_hold() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let log = Rc::new(RefCell::new(Vec::new()));
        let (c, l) = (ctx.clone(), Rc::clone(&log));
        let p = sim.spawn(async move {
            // A plain hold is not interruptible.
            c.hold(5.0).await;
            l.borrow_mut().push(c.now());
            let first = c.hold_interruptible(2.0).await;
            let second = c.hold_interruptible(2.0).await;
            let third = c.hold_interruptible(2.0).await;
            l.borrow_mut().push(c.now());
            assert_eq!(
                (first.map_err(|i| (i.cause, i.at, i.remaining))),
                Err((InterruptCause::Other(1), 1.0, 2.0))
            );
            assert_eq!(second.map_err(|i| i.cause), Err(InterruptCause::Other(2)));
            assert_eq!(third, Ok(()));
        });
        let c = ctx.clone();
        sim.spawn(async move {
            c.hold(1.0).await;
            c.interrupt(p, 1);
            c.interrupt(p, 2);
        });
        sim.run();
        assert_eq!(*log.borrow(), [5.0, 7.0]);
    }
}
