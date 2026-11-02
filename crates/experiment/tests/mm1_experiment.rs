//! M/M/1 through the experiment runner: the 95% confidence interval for mean time in system
//! should cover the theoretical W = 1 / (μ − λ), and the results must not depend on the number of
//! threads.

use std::cell::RefCell;
use std::rc::Rc;

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

#[test]
fn confidence_interval_covers_theory() {
    let (lambda, mu) = (0.75, 1.0);
    let s = Experiment::new(20, 2026).summarise(|r| mm1(r, lambda, mu, 1_000.0, 20_000.0));
    let s = s.ok().flatten();
    eprintln!("W = {s:?}");
    assert!(
        s.is_some_and(|s| s.n == 20 && s.contains(4.0) && s.half_width < 0.3),
        "{s:?}"
    );
}

#[test]
fn thread_count_does_not_change_results() {
    let run = |threads| {
        Experiment::new(6, 5)
            .threads(threads)
            .run(|r| mm1(r, 0.75, 1.0, 100.0, 2_000.0))
    };
    let one = run(1);
    assert!(one.is_ok());
    assert_eq!(one, run(3));
}
