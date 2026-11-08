//! M/M/1 queue against queueing theory: arrivals at rate λ, one server at rate μ, so the mean
//! time in system is W = 1 / (μ − λ). This checks the process API, the resource's FIFO hand-over
//! and the random streams together.

use std::cell::RefCell;
use std::rc::Rc;

use stochcraft_des::{Resource, Sim};
use stochcraft_random::Rng;

#[derive(Default)]
struct Stats {
    served: u64,
    total_time_in_system: f64,
}

/// Mean time in system for customers arriving after `warmup`, over `duration` time units.
fn mm1_mean_time_in_system(
    seed: u64,
    lambda: f64,
    mu: f64,
    warmup: f64,
    duration: f64,
) -> (f64, u64) {
    let mut sim = Sim::new();
    let ctx = sim.ctx();
    let server = Resource::new(&ctx, 1);
    let stats = Rc::new(RefCell::new(Stats::default()));
    let mut master = Rng::new(seed);
    let mut arrivals_rng = master.fork();
    let service_rng = Rc::new(RefCell::new(master.fork()));

    let (c, s) = (ctx.clone(), Rc::clone(&stats));
    sim.spawn(async move {
        loop {
            c.hold(arrivals_rng.exponential(1.0 / lambda)).await;
            let (cc, srv, st, rng) = (
                c.clone(),
                server.clone(),
                Rc::clone(&s),
                Rc::clone(&service_rng),
            );
            c.spawn(async move {
                let arrived = cc.now();
                let _unit = srv.acquire().await;
                let service = rng.borrow_mut().exponential(1.0 / mu);
                cc.hold(service).await;
                if arrived >= warmup {
                    let mut st = st.borrow_mut();
                    st.served += 1;
                    st.total_time_in_system += cc.now() - arrived;
                }
            });
        }
    });
    sim.run_until(warmup + duration);
    let st = stats.borrow();
    (st.total_time_in_system / st.served.max(1) as f64, st.served)
}

#[test]
fn mm1_matches_theory() {
    let (lambda, mu) = (0.75, 1.0);
    let expected = 1.0 / (mu - lambda); // 4.0
    // Average over independent replications, as an experiment would.
    let reps: Vec<f64> = (0..8)
        .map(|r| mm1_mean_time_in_system(1000 + r, lambda, mu, 1_000.0, 50_000.0).0)
        .collect();
    let mean = reps.iter().sum::<f64>() / reps.len() as f64;
    let var = reps.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (reps.len() - 1) as f64;
    let half_width = 2.365 * (var / reps.len() as f64).sqrt(); // t(0.975, 7)
    eprintln!("W = {mean:.4} ± {half_width:.4} (theory {expected})");
    assert!(
        (mean - expected).abs() < 0.05 * expected,
        "W = {mean}, theory {expected}"
    );
}

#[test]
fn mm1_is_reproducible() {
    let a = mm1_mean_time_in_system(7, 0.75, 1.0, 100.0, 2_000.0);
    let b = mm1_mean_time_in_system(7, 0.75, 1.0, 100.0, 2_000.0);
    assert_eq!(a, b);
    assert!(a.1 > 1_000);
}
