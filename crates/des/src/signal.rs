//! Conditions: processes waiting until something happens.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::sim::Ctx;

/// Processes waiting on a [`Signal`] or a closed [`Gate`], in the order they began to wait.
struct Waiters {
    ctx: Ctx,
    list: RefCell<Vec<(u64, Rc<Cell<bool>>)>>,
}

impl Waiters {
    fn new(ctx: &Ctx) -> Rc<Self> {
        Rc::new(Self {
            ctx: ctx.clone(),
            list: RefCell::new(Vec::new()),
        })
    }

    /// Wakes every waiter now, in the order they began to wait. Returns how many.
    fn wake_all(&self) -> usize {
        let list = std::mem::take(&mut *self.list.borrow_mut());
        for (pid, flag) in &list {
            flag.set(true);
            self.ctx.wake_now(*pid);
        }
        list.len()
    }

    fn len(&self) -> usize {
        self.list.borrow().len()
    }
}

/// An event processes wait for: [`Signal::notify_all`] wakes every process waiting at that
/// moment. Nothing is remembered, so a process that begins to wait afterwards waits for the
/// next notification (use a [`Gate`] for a condition that stays true).
#[derive(Clone)]
pub struct Signal {
    waiters: Rc<Waiters>,
}

impl Signal {
    pub fn new(ctx: &Ctx) -> Self {
        Self {
            waiters: Waiters::new(ctx),
        }
    }

    /// Waits for the next notification.
    pub fn wait(&self) -> Wait {
        Wait {
            waiters: Rc::clone(&self.waiters),
            gate: None,
            woken: None,
        }
    }

    /// Wakes every waiting process now, in the order they began to wait. Returns how many.
    pub fn notify_all(&self) -> usize {
        self.waiters.wake_all()
    }

    /// Processes waiting.
    pub fn waiting(&self) -> usize {
        self.waiters.len()
    }
}

/// A condition that stays true once set: while the gate is open, [`Gate::wait`] goes straight
/// through; while it is closed, it waits until the gate opens (a shift starting, a batch being
/// complete, a part arriving for assembly).
#[derive(Clone)]
pub struct Gate {
    waiters: Rc<Waiters>,
    open: Rc<Cell<bool>>,
}

impl Gate {
    /// A gate, open or closed.
    pub fn new(ctx: &Ctx, open: bool) -> Self {
        Self {
            waiters: Waiters::new(ctx),
            open: Rc::new(Cell::new(open)),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Opens the gate and lets every waiting process through now, in the order they began to
    /// wait. Returns how many.
    pub fn open(&self) -> usize {
        self.open.set(true);
        self.waiters.wake_all()
    }

    /// Closes the gate: later waits wait for it to open again.
    pub fn close(&self) {
        self.open.set(false);
    }

    /// Waits until the gate is open (at once if it is).
    pub fn wait(&self) -> Wait {
        Wait {
            waiters: Rc::clone(&self.waiters),
            gate: Some(Rc::clone(&self.open)),
            woken: None,
        }
    }

    /// Processes waiting.
    pub fn waiting(&self) -> usize {
        self.waiters.len()
    }
}

/// The future returned by [`Signal::wait`] and [`Gate::wait`].
pub struct Wait {
    waiters: Rc<Waiters>,
    /// For a gate: whether it is open.
    gate: Option<Rc<Cell<bool>>>,
    woken: Option<Rc<Cell<bool>>>,
}

impl Future for Wait {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if let Some(woken) = &self.woken {
            return if woken.get() {
                Poll::Ready(())
            } else {
                Poll::Pending
            };
        }
        if self.gate.as_ref().is_some_and(|g| g.get()) {
            return Poll::Ready(());
        }
        let Some(pid) = self.waiters.ctx.current() else {
            // Polled outside the simulation: it could never be woken.
            return Poll::Pending;
        };
        let woken = Rc::new(Cell::new(false));
        self.waiters
            .list
            .borrow_mut()
            .push((pid, Rc::clone(&woken)));
        self.woken = Some(woken);
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sim;

    #[test]
    fn notify_all_wakes_everyone_waiting_and_only_them() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let signal = Signal::new(&ctx);
        let log = Rc::new(RefCell::new(Vec::new()));
        for (name, start) in [("a", 0.0), ("b", 1.0), ("late", 3.0)] {
            let (c, s, l) = (ctx.clone(), signal.clone(), Rc::clone(&log));
            sim.spawn(async move {
                c.hold(start).await;
                s.wait().await;
                l.borrow_mut().push((name, c.now()));
            });
        }
        let (c, s) = (ctx.clone(), signal.clone());
        sim.spawn(async move {
            c.hold(2.0).await;
            assert_eq!(s.notify_all(), 2);
            c.hold(3.0).await;
            assert_eq!(s.notify_all(), 1);
            assert_eq!(s.notify_all(), 0);
        });
        sim.run();
        assert_eq!(*log.borrow(), [("a", 2.0), ("b", 2.0), ("late", 5.0)]);
        assert_eq!(signal.waiting(), 0);
    }

    /// A shift gate: closed from 0 to 8, open until 16, closed again.
    #[test]
    fn a_gate_holds_waiters_while_closed_and_lets_them_through_while_open() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let shift = Gate::new(&ctx, false);
        let log = Rc::new(RefCell::new(Vec::new()));
        for (name, arrive) in [("a", 1.0), ("b", 9.0), ("c", 17.0)] {
            let (c, g, l) = (ctx.clone(), shift.clone(), Rc::clone(&log));
            sim.spawn(async move {
                c.hold(arrive).await;
                g.wait().await;
                l.borrow_mut().push((name, c.now()));
            });
        }
        let (c, g) = (ctx.clone(), shift.clone());
        sim.spawn(async move {
            c.hold(8.0).await;
            assert_eq!(g.open(), 1);
            c.hold(8.0).await;
            g.close();
            c.hold(8.0).await;
            g.open();
        });
        sim.run();
        assert_eq!(*log.borrow(), [("a", 8.0), ("b", 9.0), ("c", 24.0)]);
        assert!(shift.is_open());
    }
}
