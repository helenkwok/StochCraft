#![allow(clippy::panic)] // Test helpers stop on a broken model.
//! A compiled model run through the parallel experiment runner: one `Compiled` shared by every
//! worker thread, results identical for any thread count, and intervals covering theory.

use stochcraft_blocks::{Builder, Compiled, ExperimentSummary, Metric, compile, summarise};
use stochcraft_experiment::Experiment;
use stochcraft_model::{Dist, Experiment as Settings};

fn mm1() -> Compiled {
    let mut b = Builder::new();
    let src = b.source("arrivals", Dist::exponential_rate(0.75), None);
    let srv = b.server("machine", 1, Dist::exponential_rate(1.0));
    let out = b.sink("exit");
    b.connect(src, srv);
    b.connect(srv, out);
    let model = b.build(Settings {
        warmup: 1_000.0,
        duration: 20_000.0,
        replications: 24,
        seed: 7,
        ..Settings::default()
    });
    match compile(&model) {
        Ok(c) => c,
        Err(d) => panic!("model should compile: {d:?}"),
    }
}

/// Runs the model's own experiment settings (replications and seed) on `threads` workers.
fn run(compiled: &Compiled, threads: usize) -> ExperimentSummary {
    let x = &compiled.model().experiment;
    let reports = Experiment::new(u64::from(x.replications), x.seed)
        .threads(threads)
        .run(|r| compiled.run(r.rng));
    match reports {
        Ok(r) => summarise(&r),
        Err(e) => panic!("replications failed: {e}"),
    }
}

#[test]

fn parallel_runs_match_theory_and_do_not_depend_on_threads() {
    let compiled = mm1();
    let one = run(&compiled, 1);
    let four = run(&compiled, 4);
    assert_eq!(
        one, four,
        "results must not depend on the number of threads"
    );
    assert_eq!(one.replications, 24);
    for (element, metric, theory) in [
        ("exit", Metric::MeanTimeInSystem, 4.0),
        ("machine", Metric::MeanQueueLength, 2.25),
        ("machine", Metric::Utilisation, 0.75),
    ] {
        let got = one.get(element, metric);
        eprintln!("{element} {}: {got:?} (theory {theory})", metric.name());
        assert!(
            got.is_some_and(|g| g.contains(theory)),
            "{element} {}: {got:?}",
            metric.name()
        );
    }
}
