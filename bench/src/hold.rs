//! The classic hold model: a fixed population of processes, each holding for an exponential
//! time (mean 1) over and over. Every event takes the earliest entry off the event list and puts
//! one back, so the list stays at the population's size and the run measures the cost of one
//! event at that size.
//!
//! Check: with mean hold 1, a run to time T makes about P·T holds, plus the P first
//! activations at time 0.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use stochcraft_des::Sim;
use stochcraft_random::Rng;

/// Population sizes measured by default.
pub const POPULATIONS: [u64; 3] = [10, 1_000, 100_000];

/// The largest relative error in the event count accepted as a pass.
const TOLERANCE: f64 = 0.01;

/// Runs each population for about `events` events. Returns whether every run passed its check.
pub fn run(events: u64) -> bool {
    println!(
        "Hold model: P processes, each holding Exp(mean 1) over and over; a pass is events within {:.0}% of P·(1 + T)",
        TOLERANCE * 100.0
    );
    let mut ok = true;
    for p in POPULATIONS {
        // Long enough for about `events` holds in all.
        let until = events as f64 / p as f64;
        let started = Instant::now();
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let rng = Rc::new(RefCell::new(Rng::new(1)));
        for _ in 0..p {
            let (c, r) = (ctx.clone(), Rc::clone(&rng));
            sim.spawn(async move {
                loop {
                    let t = r.borrow_mut().exponential(1.0);
                    c.hold(t).await;
                }
            });
        }
        let done = sim.run_until(until);
        drop(sim);
        let secs = started.elapsed().as_secs_f64();
        let expected = p as f64 * (1.0 + until);
        let error = done as f64 / expected - 1.0;
        let pass = error.abs() <= TOLERANCE;
        ok &= pass;
        println!(
            "des     P = {p:>7}  {done:>9} events (expected {expected:>9.0}, {:>+5.2}%)  {secs:>7.3} s  {:>6.2} M events/s  {}",
            error * 100.0,
            done as f64 / secs / 1e6,
            if pass { "ok" } else { "CHECK FAILED" }
        );
    }
    ok
}
