//! Tandem lines: `stations` M/M/1 stations in series, each with service rate μ, fed by Poisson
//! arrivals at rate λ. Every station then sees Poisson arrivals at rate λ (Burke), so the line
//! is a Jackson network and the mean time in system is `stations` times the M/M/1 value:
//! W = stations / (μ − λ).
//!
//! Built twice: on the `des` crate directly (a process per station, a store in front of each),
//! and through the `blocks` Builder (a source, `stations` servers and a sink), which is how a
//! StochCraft model runs. The first measures the engine, the second the engine plus the block
//! layer. The two draw from different streams (blocks gives each element its own), so each is
//! checked against theory, not against the other.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use stochcraft_blocks::{Builder, compile};
use stochcraft_des::{Sim, Store};
use stochcraft_model::Experiment;
use stochcraft_random::{Dist, Rng};

use crate::{Acc, LAMBDA, MU};

/// Station counts measured by default.
pub const STATIONS: [usize; 3] = [5, 50, 200];

/// The largest relative error in W accepted as a pass with `n` customers: three standard errors
/// of one M/M/1 station's mean at ρ = 0.9. The `parallel` workload's 64 replications of 10⁶
/// customers put that standard error at about 1.8%, and it shrinks as 1/√n. A line's error is
/// no larger than one station's, since its stations' errors are not perfectly correlated.
/// Successive customers' times are strongly correlated at this load, so 10⁵ customers still
/// leave a few percent of noise: the check catches a broken model, not a small bias.
fn tolerance(n: u64) -> f64 {
    3.0 * 0.018 * (1e6 / n.max(1) as f64).sqrt()
}

/// The share of customers left out of W as a warm-up: the lines start empty, and a 200-station
/// line takes a while to fill, which without a warm-up leaves W several percent low at 10⁵
/// customers. On `des` the first customers to arrive are left out; through `blocks`, those that
/// leave before the time the first ones take to arrive (the model's warm-up).
const WARMUP: f64 = 0.1;

struct Run {
    /// Whether every customer arrived and left.
    drained: bool,
    /// Customers in W.
    measured: u64,
    mean_time_in_system: f64,
    /// Process activations (des) or events (blocks).
    events: u64,
}

fn des_tandem(stations: usize, n: u64, rng: Rng) -> Run {
    let mut sim = Sim::new();
    let ctx = sim.ctx();
    // Each customer is its arrival number and time.
    let stores: Rc<Vec<Store<(u64, f64)>>> =
        Rc::new((0..stations).map(|_| Store::new(&ctx)).collect());
    let rng = Rc::new(RefCell::new(rng));
    let acc = Rc::new(Acc::default());
    let out = Rc::new(Cell::new(0u64));
    let warm = (WARMUP * n as f64) as u64;
    let (c, s, r) = (ctx.clone(), Rc::clone(&stores), Rc::clone(&rng));
    sim.spawn(async move {
        for k in 0..n {
            let t = r.borrow_mut().exponential(1.0 / LAMBDA);
            c.hold(t).await;
            s[0].put((k, c.now()));
        }
    });
    for i in 0..stations {
        let (c, s, r, a, o) = (
            ctx.clone(),
            Rc::clone(&stores),
            Rc::clone(&rng),
            Rc::clone(&acc),
            Rc::clone(&out),
        );
        sim.spawn(async move {
            loop {
                let (k, arrived) = s[i].get().await;
                let t = r.borrow_mut().exponential(1.0 / MU);
                c.hold(t).await;
                match s.get(i + 1) {
                    Some(next) => next.put((k, arrived)),
                    None => {
                        o.set(o.get() + 1);
                        if k >= warm {
                            a.add(c.now() - arrived);
                        }
                    }
                }
            }
        });
    }
    let events = sim.run();
    let o = acc.outcome();
    Run {
        drained: out.get() == n,
        measured: o.customers,
        mean_time_in_system: o.mean_time_in_system,
        events,
    }
}

/// Runs the line through `blocks` for `duration` after the warm-up; long enough for every
/// customer to arrive and leave, the run ends when no event is left.
fn blocks_tandem(stations: usize, n: u64, rng: Rng, duration: f64) -> Run {
    let mut m = Builder::new();
    let arrivals = m.source("arrivals", Dist::exponential_rate(LAMBDA), Some(n));
    let mut last = arrivals;
    for i in 1..=stations {
        let s = m.server(&format!("s{i}"), 1, Dist::exponential_rate(MU));
        m.connect(last, s);
        last = s;
    }
    let exit = m.sink("exit");
    m.connect(last, exit);
    let experiment = Experiment {
        warmup: WARMUP * n as f64 / LAMBDA,
        duration,
        replications: 1,
        ..Experiment::default()
    };
    let compiled = compile(&m.build(experiment)).expect("the tandem model is valid");
    let mut live = compiled.live(rng);
    let end = live.end();
    live.advance_to(end).expect("the tandem run finishes");
    let report = live.report();
    let sink = report.element("exit").expect("the model has a sink");
    Run {
        // Nothing left anywhere in the line.
        drained: live.next_event_time().is_none()
            && report
                .elements
                .iter()
                .all(|e| e.work_in_progress.unwrap_or(0) == 0),
        measured: sink.left,
        mean_time_in_system: sink.mean_time_in_system.unwrap_or(f64::NAN),
        events: live.events(),
    }
}

fn report(path: &str, stations: usize, n: u64, f: impl FnOnce() -> Run) -> bool {
    let started = Instant::now();
    let r = f();
    let secs = started.elapsed().as_secs_f64();
    let theory = stations as f64 / (MU - LAMBDA);
    let error = r.mean_time_in_system / theory - 1.0;
    let ok = r.drained && error.abs() <= tolerance(r.measured);
    let visits = n as f64 * stations as f64;
    println!(
        "{path:<7} N = {stations:>3}  {n:>8} customers  W = {:>8.2} (theory {theory:>6.0}, {:>+5.1}%)  {:>7.3} s  {:>6.2} M station visits/s  {:>6.2} M events/s  {}",
        r.mean_time_in_system,
        error * 100.0,
        secs,
        visits / secs / 1e6,
        r.events as f64 / secs / 1e6,
        if ok { "ok" } else { "CHECK FAILED" }
    );
    ok
}

/// Runs every station count on both paths with `n` customers each. Returns whether every run
/// passed its check.
pub fn run(n: u64) -> bool {
    println!(
        "Tandem M/M/1 lines, λ = {LAMBDA}, μ = {MU} at every station, theory W = N / (μ − λ), the first {:.0}% of customers left out; a pass is every customer out and W within about {:.0}%",
        WARMUP * 100.0,
        tolerance(((1.0 - WARMUP) * n as f64) as u64) * 100.0
    );
    let mut ok = true;
    for stations in STATIONS {
        ok &= report("des", stations, n, || des_tandem(stations, n, Rng::new(1)));
        ok &= report("blocks", stations, n, || {
            blocks_tandem(stations, n, Rng::new(1), 1e15)
        });
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_that_has_not_drained_fails_its_check() {
        assert!(des_tandem(5, 1_000, Rng::new(1)).drained);
        assert!(blocks_tandem(5, 1_000, Rng::new(1), 1e15).drained);
        // Cut off long before the last of 1,000 customers (about 1,100 time units) has left.
        assert!(!blocks_tandem(5, 1_000, Rng::new(1), 100.0).drained);
    }
}
