//! The new primitives against queueing theory:
//! - a bounded store with balking gives M/M/1/K, whose blocking probability is known exactly;
//! - a preemptive resource gives the two-class M/M/1 with preemptive-resume priority, where the
//!   urgent class sees an M/M/1 queue of its own.

use std::cell::RefCell;
use std::rc::Rc;

use stochcraft_des::{Resource, Sim, Store};
use stochcraft_random::Rng;

/// The fraction of arrivals turned away by an M/M/1/K system: one server fed by a buffer of
/// K − 1. An arrival that finds the buffer full balks (`try_put`).
fn mm1k_blocking(seed: u64, lambda: f64, mu: f64, k: usize, duration: f64) -> f64 {
    let mut sim = Sim::new();
    let ctx = sim.ctx();
    let buffer: Store<()> = Store::bounded(&ctx, k - 1);
    let counts = Rc::new(RefCell::new((0u64, 0u64)));
    let mut master = Rng::new(seed);
    let mut arrivals = master.fork();
    let mut services = master.fork();
    let (c, b, n) = (ctx.clone(), buffer.clone(), Rc::clone(&counts));
    sim.spawn(async move {
        loop {
            c.hold(arrivals.exponential(1.0 / lambda)).await;
            let mut n = n.borrow_mut();
            n.0 += 1;
            if b.try_put(()).is_err() {
                n.1 += 1;
            }
        }
    });
    let (c, b) = (ctx.clone(), buffer.clone());
    sim.spawn(async move {
        loop {
            b.get().await;
            c.hold(services.exponential(1.0 / mu)).await;
        }
    });
    sim.run_until(duration);
    let (arrived, lost) = *counts.borrow();
    lost as f64 / arrived as f64
}

#[test]
fn bounded_store_with_balking_matches_mm1k() {
    let (lambda, mu, k) = (0.9, 1.0, 5);
    let rho: f64 = lambda / mu;
    let rho_k = (0..k).fold(1.0, |p, _| p * rho);
    let expected = (1.0 - rho) * rho_k / (1.0 - rho_k * rho);
    let reps: Vec<f64> = (0..8)
        .map(|r| mm1k_blocking(200 + r, lambda, mu, k, 50_000.0))
        .collect();
    let mean = reps.iter().sum::<f64>() / reps.len() as f64;
    eprintln!("P(block) = {mean:.4} (theory {expected:.4})");
    assert!(
        (mean - expected).abs() < 0.05 * expected,
        "P(block) = {mean}, theory {expected}"
    );
}

/// Mean times in system of the urgent and normal classes, each arriving at `lambda`, on one
/// preemptive server with exponential work of mean 1 / `mu`. A preempted job resumes its
/// remaining work later.
fn preemptive_priority(seed: u64, lambda: f64, mu: f64, warmup: f64, duration: f64) -> [f64; 2] {
    let mut sim = Sim::new();
    let ctx = sim.ctx();
    let server = Resource::preemptive(&ctx, 1);
    let totals = Rc::new(RefCell::new([(0.0f64, 0u64); 2]));
    let mut master = Rng::new(seed);
    for class in 0..2usize {
        let mut arrivals = master.fork();
        let work = Rc::new(RefCell::new(master.fork()));
        let (c, s, t) = (ctx.clone(), server.clone(), Rc::clone(&totals));
        sim.spawn(async move {
            loop {
                c.hold(arrivals.exponential(1.0 / lambda)).await;
                let (cc, srv, tot, w) = (c.clone(), s.clone(), Rc::clone(&t), Rc::clone(&work));
                c.spawn(async move {
                    let arrived = cc.now();
                    let mut left = w.borrow_mut().exponential(1.0 / mu);
                    // Class 0 is urgent.
                    let priority = if class == 0 { 1 } else { 0 };
                    loop {
                        let _unit = srv.acquire_with_priority(priority).await;
                        match cc.hold_interruptible(left).await {
                            Ok(()) => break,
                            Err(i) => left = i.remaining,
                        }
                    }
                    if arrived >= warmup {
                        let mut tot = tot.borrow_mut();
                        if let Some(entry) = tot.get_mut(class) {
                            entry.0 += cc.now() - arrived;
                            entry.1 += 1;
                        }
                    }
                });
            }
        });
    }
    sim.run_until(warmup + duration);
    let t = totals.borrow();
    [t[0].0 / t[0].1 as f64, t[1].0 / t[1].1 as f64]
}

#[test]
fn preemptive_resource_matches_preemptive_resume_priority_theory() {
    let (lambda, mu) = (0.3, 1.0);
    let (rho1, rho) = (lambda / mu, 2.0 * lambda / mu);
    // The urgent class does not see the other: M/M/1 at its own load.
    let urgent = 1.0 / (mu - lambda);
    // The normal class (same exponential work): (1/μ) / ((1 − ρ1)(1 − ρ)).
    let normal = (1.0 / mu) / ((1.0 - rho1) * (1.0 - rho));
    let reps: Vec<[f64; 2]> = (0..8)
        .map(|r| preemptive_priority(300 + r, lambda, mu, 1_000.0, 50_000.0))
        .collect();
    let mean = |c: usize| reps.iter().map(|w| w[c]).sum::<f64>() / reps.len() as f64;
    let (u, n) = (mean(0), mean(1));
    eprintln!("W urgent = {u:.4} (theory {urgent:.4}), normal = {n:.4} (theory {normal:.4})");
    assert!(
        (u - urgent).abs() < 0.05 * urgent,
        "urgent {u}, theory {urgent}"
    );
    assert!(
        (n - normal).abs() < 0.05 * normal,
        "normal {n}, theory {normal}"
    );
}
