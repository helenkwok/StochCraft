//! Flow blocks against exact queueing results, over independent replications with a 95%
//! confidence interval that must cover the theoretical value.

use stochcraft_blocks::{Model, Report};
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::Summary;

fn replicate(
    reps: u64,
    run: impl Fn(Rng) -> Option<Report>,
    metric: impl Fn(&Report) -> Option<f64>,
) -> Summary {
    let values: Vec<f64> = (0..reps)
        .filter_map(|r| metric(&run(Rng::new(10_000 + r))?))
        .collect();
    assert_eq!(
        values.len() as u64,
        reps,
        "every replication must run and report"
    );
    Summary::of(&values).unwrap_or(Summary {
        n: 0,
        mean: f64::NAN,
        std_dev: f64::NAN,
        half_width: f64::NAN,
    })
}

fn mmc(lambda: f64, mu: f64, c: u32, rng: Rng) -> Option<Report> {
    let mut m = Model::new();
    let src = m.source("arrivals", Dist::exponential_rate(lambda), None);
    let srv = m.server("servers", c, Dist::exponential_rate(mu));
    let out = m.sink("exit");
    m.connect(src, srv);
    m.connect(srv, out);
    m.run(rng, 1_000.0, 20_000.0).ok()
}

#[test]
fn mm1_time_in_system_queue_and_utilisation() {
    // λ = 0.75, μ = 1: W = 1/(μ−λ) = 4, Lq = ρ²/(1−ρ) = 2.25, Wq = 3, utilisation ρ = 0.75.
    let run = |r| mmc(0.75, 1.0, 1, r);
    let w = replicate(16, run, |r| r.block("exit")?.mean_time_in_system);
    let lq = replicate(16, run, |r| r.block("servers")?.mean_queue_length);
    let wq = replicate(16, run, |r| r.block("servers")?.mean_wait);
    let u = replicate(16, run, |r| r.block("servers")?.utilisation);
    eprintln!("W {w}\nLq {lq}\nWq {wq}\nU {u}");
    assert!(w.contains(4.0), "W {w}");
    assert!(lq.contains(2.25), "Lq {lq}");
    assert!(wq.contains(3.0), "Wq {wq}");
    assert!(u.contains(0.75), "U {u}");
}

#[test]
fn mm2_matches_erlang_c() {
    // λ = 1.5, μ = 1, c = 2: offered load a = 1.5, ρ = 0.75.
    // P0 = 1 / (1 + a + a²/(2!(1−ρ))) = 1/7; Erlang C = (a²/(2!(1−ρ))) P0 = 4.5/7;
    // Wq = C / (cμ − λ) = 9/7 ≈ 1.2857; W = Wq + 1/μ = 16/7 ≈ 2.2857.
    let run = |r| mmc(1.5, 1.0, 2, r);
    let w = replicate(16, run, |r| r.block("exit")?.mean_time_in_system);
    let wq = replicate(16, run, |r| r.block("servers")?.mean_wait);
    let u = replicate(16, run, |r| r.block("servers")?.utilisation);
    eprintln!("W {w}\nWq {wq}\nU {u}");
    assert!(w.contains(16.0 / 7.0), "W {w}");
    assert!(wq.contains(9.0 / 7.0), "Wq {wq}");
    assert!(u.contains(0.75), "U {u}");
}
