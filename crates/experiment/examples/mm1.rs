//! `cargo run --release -p stochcraft-experiment --example mm1 [replications] [seed]`
//!
//! Runs M/M/1 replications in parallel and prints mean time in system against theory.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use stochcraft_des::{Resource, Sim};
use stochcraft_experiment::{Experiment, Replication};
use stochcraft_stats::Tally;

fn mm1(mut rep: Replication, lambda: f64, mu: f64, warmup: f64, duration: f64) -> f64 {
    let mut sim = Sim::new();
    let ctx = sim.ctx();
    let server = Resource::new(&ctx, 1);
    let tally = Rc::new(RefCell::new(Tally::new()));
    let mut arrivals = rep.rng.fork();
    let service = Rc::new(RefCell::new(rep.rng.fork()));
    let (c, t) = (ctx.clone(), Rc::clone(&tally));
    sim.spawn(async move {
        loop {
            c.hold(arrivals.exponential(1.0 / lambda)).await;
            let (cc, srv, tt, rng) = (
                c.clone(),
                server.clone(),
                Rc::clone(&t),
                Rc::clone(&service),
            );
            c.spawn(async move {
                let arrived = cc.now();
                let _unit = srv.acquire().await;
                let s = rng.borrow_mut().exponential(1.0 / mu);
                cc.hold(s).await;
                if arrived >= warmup {
                    tt.borrow_mut().add(cc.now() - arrived);
                }
            });
        }
    });
    sim.run_until(warmup + duration);
    tally.borrow().mean().unwrap_or(f64::NAN)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let reps = args.next().and_then(|a| a.parse().ok()).unwrap_or(100);
    let seed = args.next().and_then(|a| a.parse().ok()).unwrap_or(1);
    let (lambda, mu) = (0.75, 1.0);
    let started = Instant::now();
    match Experiment::new(reps, seed).summarise(|r| mm1(r, lambda, mu, 1_000.0, 100_000.0)) {
        Ok(Some(s)) => println!(
            "M/M/1 (λ = {lambda}, μ = {mu}): W = {s}; theory {:.4}",
            1.0 / (mu - lambda)
        ),
        Ok(None) => println!("no replications"),
        Err(e) => eprintln!("error: {e}"),
    }
    println!(
        "{reps} replications in {:.2} s",
        started.elapsed().as_secs_f64()
    );
}
