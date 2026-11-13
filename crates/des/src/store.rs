//! Stores: queues of items that processes put and get.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::TimedOut;
use crate::resource::insert_position;
use crate::sim::{Ctx, deadline};

type Slot<T> = Rc<RefCell<Option<T>>>;
type PriorityFn<T> = Rc<dyn Fn(&T) -> i64>;

/// A put waiting for room in a full bounded store.
struct Putter<T> {
    item: T,
    /// The waiting process and the flag set when its item is in; `None` for [`Store::put`],
    /// which does not wait.
    waiting: Option<(u64, Rc<Cell<bool>>)>,
}

struct StoreState<T> {
    /// Items with their priorities, highest first and then in the order they came in.
    items: VecDeque<(i64, T)>,
    /// The most items held; `usize::MAX` when unbounded.
    capacity: usize,
    priority: Option<PriorityFn<T>>,
    /// Waiting getters in arrival order, each with the slot an item is handed into.
    getters: VecDeque<(u64, Slot<T>)>,
    /// Puts waiting for room, in arrival order.
    putters: VecDeque<Putter<T>>,
}

impl<T> StoreState<T> {
    fn priority_of(&self, item: &T) -> i64 {
        self.priority.as_ref().map_or(0, |f| f(item))
    }

    fn has_room(&self) -> bool {
        self.items.len() < self.capacity && self.putters.is_empty()
    }

    fn insert(&mut self, item: T) {
        let p = self.priority_of(&item);
        let at = insert_position(&self.items, p, |(q, _)| *q);
        self.items.insert(at, (p, item));
    }

    /// Takes the first item and lets waiting puts fill the room it leaves. Returns the item and
    /// the processes whose puts went in.
    fn take(&mut self) -> Option<(T, Vec<u64>)> {
        let (_, item) = self.items.pop_front()?;
        Some((item, self.refill()))
    }

    /// Lets waiting puts fill the room there is. Returns the processes whose puts went in.
    fn refill(&mut self) -> Vec<u64> {
        let mut woken = Vec::new();
        while self.items.len() < self.capacity {
            let Some(p) = self.putters.pop_front() else {
                break;
            };
            self.insert(p.item);
            if let Some((pid, done)) = p.waiting {
                done.set(true);
                woken.push(pid);
            }
        }
        woken
    }
}

/// A store of items (parts on a conveyor, jobs in a buffer, orders in a queue).
///
/// - Items come out first in, first out, or by priority with [`Store::with_priority`]
///   (higher first, then first in among equals).
/// - [`Store::new`] is unbounded. [`Store::bounded`] holds at most `capacity` items: then
///   [`Store::put_wait`] waits for room (the upstream machine is blocked), [`Store::try_put`]
///   refuses, and [`Store::put`] never waits but leaves its item in line behind the waiting
///   puts. Waiting puts go in first come, first served as items are taken.
/// - [`Store::get`] waits until an item is available, [`Store::get_within`] for at most a
///   given time. Waiting getters are served first come, first served.
pub struct Store<T> {
    ctx: Ctx,
    state: Rc<RefCell<StoreState<T>>>,
}

impl<T> Clone for Store<T> {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx.clone(),
            state: Rc::clone(&self.state),
        }
    }
}

impl<T> Store<T> {
    /// An unbounded first-in, first-out store.
    pub fn new(ctx: &Ctx) -> Self {
        Self::build(ctx, usize::MAX)
    }

    /// A store that holds at most `capacity` items (at least 1).
    pub fn bounded(ctx: &Ctx, capacity: usize) -> Self {
        Self::build(ctx, capacity.max(1))
    }

    fn build(ctx: &Ctx, capacity: usize) -> Self {
        let state = StoreState {
            items: VecDeque::new(),
            capacity,
            priority: None,
            getters: VecDeque::new(),
            putters: VecDeque::new(),
        };
        Self {
            ctx: ctx.clone(),
            state: Rc::new(RefCell::new(state)),
        }
    }

    /// Orders the items by `priority` (higher first, then first in among equals) rather than
    /// first in, first out. Set it before the first put.
    pub fn with_priority(self, priority: impl Fn(&T) -> i64 + 'static) -> Self {
        self.state.borrow_mut().priority = Some(Rc::new(priority));
        self
    }

    /// Items in the store (not counting puts waiting for room).
    pub fn len(&self) -> usize {
        self.state.borrow().items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.state.borrow().items.is_empty()
    }

    /// The most items the store holds (`usize::MAX` if unbounded).
    pub fn capacity(&self) -> usize {
        self.state.borrow().capacity
    }

    /// Processes waiting to get an item.
    pub fn getters_waiting(&self) -> usize {
        self.state.borrow().getters.len()
    }

    /// Puts waiting for room.
    pub fn puts_waiting(&self) -> usize {
        self.state.borrow().putters.len()
    }

    /// Adds an item without waiting. If a process is waiting, the item goes straight to it; if
    /// a bounded store is full, the item waits in line for room behind earlier puts.
    pub fn put(&self, item: T) {
        if let Err(item) = self.try_put(item) {
            self.state.borrow_mut().putters.push_back(Putter {
                item,
                waiting: None,
            });
        }
    }

    /// Adds an item if there is room now (balking); otherwise gives it back.
    pub fn try_put(&self, item: T) -> Result<(), T> {
        let woken = {
            let mut st = self.state.borrow_mut();
            match st.getters.pop_front() {
                Some((pid, slot)) => {
                    *slot.borrow_mut() = Some(item);
                    pid
                }
                None if st.has_room() => {
                    st.insert(item);
                    return Ok(());
                }
                None => return Err(item),
            }
        };
        self.ctx.wake_now(woken);
        Ok(())
    }

    /// Adds an item, waiting while a bounded store is full.
    pub fn put_wait(&self, item: T) -> PutWait<T> {
        PutWait {
            store: self.clone(),
            item: Some(item),
            done: None,
        }
    }

    /// Takes out the first item held (in the order gets would take them) for which `f` holds,
    /// without waiting: someone giving up their place in the queue. Puts waiting for room fill
    /// the room it leaves, as after a get. Items still waiting to be put in are not looked at.
    pub fn take_where(&self, f: impl Fn(&T) -> bool) -> Option<T> {
        let (item, woken) = {
            let mut st = self.state.borrow_mut();
            let at = st.items.iter().position(|(_, x)| f(x))?;
            let (_, item) = st.items.remove(at)?;
            (item, st.refill())
        };
        self.wake_all(woken);
        Some(item)
    }

    /// Adds an item at the head of the line, without waiting: one that was taken out and comes
    /// back to go first (a job whose server was taken out of service, retried at once). If a
    /// process is waiting, the item goes straight to it. It goes in even when a bounded store is
    /// full, since it held a place before. In a store ordered by priority it goes ahead of every
    /// item of its priority or lower (first in a first-in, first-out store).
    pub fn put_front(&self, item: T) {
        let woken = {
            let mut st = self.state.borrow_mut();
            match st.getters.pop_front() {
                Some((pid, slot)) => {
                    *slot.borrow_mut() = Some(item);
                    pid
                }
                None => {
                    let p = st.priority_of(&item);
                    let at = st
                        .items
                        .iter()
                        .position(|(q, _)| *q <= p)
                        .unwrap_or(st.items.len());
                    st.items.insert(at, (p, item));
                    return;
                }
            }
        };
        self.ctx.wake_now(woken);
    }

    /// Waits for the next item.
    pub fn get(&self) -> Get<T> {
        Get {
            inner: self.get_within(f64::INFINITY),
        }
    }

    /// Waits for the next item for at most `patience` time units. An item put at the deadline
    /// itself still counts; on time-out the getter leaves the queue.
    pub fn get_within(&self, patience: f64) -> GetWithin<T> {
        GetWithin {
            store: self.clone(),
            patience,
            waiting: None,
        }
    }

    fn wake_all(&self, pids: Vec<u64>) {
        for pid in pids {
            self.ctx.wake_now(pid);
        }
    }
}

/// The future returned by [`Store::put_wait`].
pub struct PutWait<T> {
    store: Store<T>,
    item: Option<T>,
    done: Option<Rc<Cell<bool>>>,
}

impl<T> Unpin for PutWait<T> {}

impl<T> Future for PutWait<T> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if let Some(done) = &self.done {
            return if done.get() {
                Poll::Ready(())
            } else {
                Poll::Pending
            };
        }
        let Some(item) = self.item.take() else {
            return Poll::Ready(());
        };
        let item = match self.store.try_put(item) {
            Ok(()) => return Poll::Ready(()),
            Err(item) => item,
        };
        let pid = self.store.ctx.current();
        let done = Rc::new(Cell::new(false));
        self.store.state.borrow_mut().putters.push_back(Putter {
            item,
            // Outside the simulation the put still goes in line; it just cannot be woken.
            waiting: pid.map(|p| (p, Rc::clone(&done))),
        });
        self.done = Some(done);
        Poll::Pending
    }
}

/// The future returned by [`Store::get`].
pub struct Get<T> {
    inner: GetWithin<T>,
}

impl<T> Unpin for Get<T> {}

impl<T> Future for Get<T> {
    type Output = T;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        match Pin::new(&mut self.inner).poll(cx) {
            Poll::Ready(Ok(item)) => Poll::Ready(item),
            // Without a deadline a get never times out.
            Poll::Ready(Err(TimedOut)) | Poll::Pending => Poll::Pending,
        }
    }
}

/// The future returned by [`Store::get_within`]: the item, or [`TimedOut`].
pub struct GetWithin<T> {
    store: Store<T>,
    patience: f64,
    waiting: Option<(Slot<T>, Option<f64>)>,
}

impl<T> Unpin for GetWithin<T> {}

impl<T> Future for GetWithin<T> {
    type Output = Result<T, TimedOut>;
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let ctx = self.store.ctx.clone();
        let now = ctx.now();
        if let Some((slot, until)) = &self.waiting {
            if let Some(item) = slot.borrow_mut().take() {
                return Poll::Ready(Ok(item));
            }
            if let Some(until) = *until {
                if now >= until {
                    let mut st = self.store.state.borrow_mut();
                    st.getters.retain(|(_, s)| !Rc::ptr_eq(s, slot));
                    return Poll::Ready(Err(TimedOut));
                }
                ctx.wake_current_at(until);
            }
            return Poll::Pending;
        }
        let pid = ctx.current();
        let taken = {
            let mut st = self.store.state.borrow_mut();
            if st.getters.is_empty() {
                st.take()
            } else {
                None
            }
        };
        if let Some((item, woken)) = taken {
            self.store.wake_all(woken);
            return Poll::Ready(Ok(item));
        }
        let Some(pid) = pid else {
            // Polled outside the simulation: it could never be woken.
            return Poll::Pending;
        };
        let slot = Rc::new(RefCell::new(None));
        self.store
            .state
            .borrow_mut()
            .getters
            .push_back((pid, Rc::clone(&slot)));
        let until = deadline(now, self.patience);
        if let Some(until) = until {
            ctx.wake_current_at(until);
        }
        self.waiting = Some((slot, until));
        Poll::Pending
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
    fn store_hands_items_to_waiting_getters_in_order() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let store: Store<u32> = Store::new(&ctx);
        let log = log();
        for name in ["g1", "g2"] {
            let (c, s, l) = (ctx.clone(), store.clone(), Rc::clone(&log));
            sim.spawn(async move {
                let item = s.get().await;
                l.borrow_mut().push((name, item, c.now()));
            });
        }
        let (c, s) = (ctx.clone(), store.clone());
        sim.spawn(async move {
            c.hold(1.0).await;
            s.put(10);
            s.put(20);
            s.put(30);
        });
        sim.run();
        assert_eq!(*log.borrow(), [("g1", 10, 1.0), ("g2", 20, 1.0)]);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn store_get_takes_queued_items_without_waiting() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let store: Store<&str> = Store::new(&ctx);
        store.put("a");
        store.put("b");
        let got = log();
        let (s, g) = (store.clone(), Rc::clone(&got));
        sim.spawn(async move {
            for _ in 0..2 {
                let item = s.get().await;
                g.borrow_mut().push(item);
            }
        });
        sim.run();
        assert_eq!(*got.borrow(), ["a", "b"]);
        assert!(store.is_empty());
    }

    /// A machine making a part every 1 feeds a buffer of 2 that a slower machine empties every
    /// 3: once the buffer is full, the first machine is blocked until room frees.
    #[test]
    fn a_full_bounded_store_blocks_the_putter() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let buffer: Store<u32> = Store::bounded(&ctx, 2);
        let puts = log();
        let (c, b, l) = (ctx.clone(), buffer.clone(), Rc::clone(&puts));
        sim.spawn(async move {
            for part in 0..5 {
                c.hold(1.0).await;
                b.put_wait(part).await;
                l.borrow_mut().push((part, c.now()));
            }
        });
        let gets = log();
        let (c, b, l) = (ctx.clone(), buffer.clone(), Rc::clone(&gets));
        sim.spawn(async move {
            for _ in 0..5 {
                let part = b.get().await;
                l.borrow_mut().push((part, c.now()));
                c.hold(3.0).await;
            }
        });
        sim.run_until(5.5);
        assert_eq!((buffer.len(), buffer.puts_waiting()), (2, 1));
        sim.run();
        // Part 0 goes straight to the waiting getter; 1 and 2 fill the buffer; 3 goes in at 4,
        // after the get at 4 makes room; 4 waits from 5 for the get at 7.
        assert_eq!(
            *puts.borrow(),
            [(0, 1.0), (1, 2.0), (2, 3.0), (3, 4.0), (4, 7.0)]
        );
        assert_eq!(
            *gets.borrow(),
            [(0, 1.0), (1, 4.0), (2, 7.0), (3, 10.0), (4, 13.0)]
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn blocked_puts_go_in_first_come_first_served() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let buffer: Store<&str> = Store::bounded(&ctx, 1);
        assert_eq!(buffer.try_put("first"), Ok(()));
        assert_eq!(buffer.try_put("balked"), Err("balked"));
        for (name, at) in [("a", 1.0), ("b", 2.0)] {
            let (c, b) = (ctx.clone(), buffer.clone());
            sim.spawn(async move {
                c.hold(at).await;
                b.put_wait(name).await;
            });
        }
        // `put` does not wait but keeps its place in line.
        let (c, b) = (ctx.clone(), buffer.clone());
        sim.spawn(async move {
            c.hold(3.0).await;
            b.put("c");
        });
        let got = log();
        let (c, b, g) = (ctx.clone(), buffer.clone(), Rc::clone(&got));
        sim.spawn(async move {
            c.hold(5.0).await;
            for _ in 0..4 {
                let item = b.get().await;
                g.borrow_mut().push(item);
                assert!(b.len() <= 1);
            }
        });
        sim.run();
        assert_eq!(*got.borrow(), ["first", "a", "b", "c"]);
        assert_eq!(buffer.puts_waiting(), 0);
    }

    #[test]
    fn priority_store_gives_the_most_urgent_item_first() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let orders: Store<(&str, i64)> = Store::new(&ctx).with_priority(|o: &(&str, i64)| o.1);
        for o in [("a", 0), ("b", 2), ("c", 1), ("d", 2), ("e", 0)] {
            orders.put(o);
        }
        let got = log();
        let (s, g) = (orders.clone(), Rc::clone(&got));
        sim.spawn(async move {
            for _ in 0..5 {
                let order = s.get().await;
                g.borrow_mut().push(order.0);
            }
        });
        sim.run();
        assert_eq!(*got.borrow(), ["b", "d", "c", "a", "e"]);
    }

    #[test]
    fn put_front_goes_first_or_straight_to_a_waiting_getter() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let store: Store<u32> = Store::bounded(&ctx, 2);
        store.put(1);
        store.put(2);
        // Full, but it held a place before: it goes in, first.
        store.put_front(9);
        assert_eq!(store.len(), 3);
        let got = log();
        let (s, g) = (store.clone(), Rc::clone(&got));
        sim.spawn(async move {
            for _ in 0..4 {
                let x = s.get().await;
                g.borrow_mut().push(x);
            }
        });
        let (c, s) = (ctx.clone(), store.clone());
        sim.spawn(async move {
            c.hold(1.0).await;
            // The getter is waiting: it gets it at once.
            s.put_front(7);
        });
        sim.run();
        assert_eq!(*got.borrow(), [9, 1, 2, 7]);

        // By priority: ahead of its own priority and lower, behind higher.
        let store: Store<u32> = Store::new(&ctx).with_priority(|x| i64::from(*x / 10));
        for x in [30, 10, 11] {
            store.put(x);
        }
        store.put_front(12);
        let got = log();
        let (s, g) = (store.clone(), Rc::clone(&got));
        sim.spawn(async move {
            for _ in 0..4 {
                let x = s.get().await;
                g.borrow_mut().push(x);
            }
        });
        sim.run();
        assert_eq!(*got.borrow(), [30, 12, 10, 11]);
    }

    #[test]
    fn take_where_removes_a_held_item_and_lets_a_waiting_put_in() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let buffer: Store<u32> = Store::bounded(&ctx, 2);
        buffer.put(1);
        buffer.put(2);
        let puts = log();
        let (c, b, l) = (ctx.clone(), buffer.clone(), Rc::clone(&puts));
        sim.spawn(async move {
            b.put_wait(3).await;
            l.borrow_mut().push((3, c.now()));
        });
        let (c, b) = (ctx.clone(), buffer.clone());
        sim.spawn(async move {
            c.hold(1.0).await;
            // A waiting put is not held yet, so it cannot be taken out.
            assert_eq!(b.take_where(|x| *x == 3), None);
            assert_eq!(b.take_where(|x| *x == 9), None);
            assert_eq!(b.take_where(|x| *x == 1), Some(1));
        });
        sim.run();
        assert_eq!(*puts.borrow(), [(3, 1.0)]);
        let rest = log();
        let (b, r) = (buffer.clone(), Rc::clone(&rest));
        sim.spawn(async move {
            for _ in 0..2 {
                let x = b.get().await;
                r.borrow_mut().push(x);
            }
        });
        sim.run();
        assert_eq!(*rest.borrow(), [2, 3]);
    }

    #[test]
    fn get_within_times_out_and_leaves_the_queue() {
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let store: Store<u32> = Store::new(&ctx);
        let log = log();
        for (name, patience) in [("impatient", 1.0), ("patient", 5.0)] {
            let (c, s, l) = (ctx.clone(), store.clone(), Rc::clone(&log));
            sim.spawn(async move {
                let r = s.get_within(patience).await;
                l.borrow_mut().push((name, r, c.now()));
            });
        }
        let (c, s) = (ctx.clone(), store.clone());
        sim.spawn(async move {
            c.hold(1.0).await;
            assert_eq!(s.getters_waiting(), 1, "the impatient getter has left at 1");
            c.hold(1.0).await;
            s.put(7);
        });
        sim.run();
        assert_eq!(
            *log.borrow(),
            [("impatient", Err(TimedOut), 1.0), ("patient", Ok(7), 2.0)]
        );
        assert_eq!((store.len(), store.getters_waiting()), (0, 0));
    }
}
