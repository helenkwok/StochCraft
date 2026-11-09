//! Published industrial benchmarks, run from their committed model files and checked against the
//! published figures.
//!
//! Case A of Schmitt et al. (2026), "LLM-driven discrete-event simulation: A generative AI
//! framework for automated model generation, adaptation, and evaluation in manufacturing",
//! Journal of Manufacturing Systems 85, 642–661, https://doi.org/10.1016/j.jmsy.2026.02.015
//! (CC BY 4.0), Table 4: four machines in series with failures, buffers of 5 with a 10-second
//! transfer, weekend breaks and power per machine, 25 replications of 30 days with 1 day of
//! warm-up (`tests/models/case-a.bpmn`, written by `stochcraft-cli new -t case-a`).
//!
//! The paper's throughput is checked as published. Its work in progress (WIP) and energy per
//! part come from a model that samples WIP once a minute, counting a part in transfer twice,
//! and books each state's energy when the state ends. StochCraft measures both exactly, so they
//! are checked against the same published model measured exactly ("corrected"): the same 25
//! seeds, WIP as the time-average number of distinct parts from the first machine taking a part
//! to the end of the line, and energy as power integrated over exact state times, cut at the
//! warm-up and at the end. ROADMAP.md (M2.6, "Industrial benchmark") has the figures side by
//! side.
#![allow(clippy::panic)]

use std::path::Path;

use stochcraft_blocks::{Report, compile};
use stochcraft_experiment::Experiment;
use stochcraft_io::read_scenario;

/// The 95% Student-t quantile for 24 degrees of freedom (25 replications).
const T_24: f64 = 2.063_898_561_628_021;

/// The mean of `xs` and the half-width of its 95% confidence interval (25 values).
fn mean_hw(xs: &[f64]) -> (f64, f64) {
    assert_eq!(xs.len(), 25);
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0);
    (mean, T_24 * (var / n).sqrt())
}

fn case_a() -> Vec<Report> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models/case-a.bpmn");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let loaded = read_scenario(&bytes, None).unwrap_or_else(|e| panic!("{e}"));
    assert!(loaded.notes.is_empty(), "{:?}", loaded.notes);
    let x = loaded.model.experiment.clone();
    assert_eq!(
        (x.replications, x.warmup, x.duration),
        (25, 86_400.0, 2_505_600.0)
    );
    let c = compile(&loaded.model).unwrap_or_else(|e| panic!("{e:?}"));
    // Each replication on its own stream of the file's seed, as the apps run them.
    let streams = Experiment::new(u64::from(x.replications), x.seed);
    (0..u64::from(x.replications))
        .map(|i| c.run(streams.stream(i)).unwrap_or_else(|e| panic!("{e}")))
        .collect()
}

/// Each replication's value of `f`, summarised.
fn each(reports: &[Report], f: impl Fn(&Report) -> f64) -> (f64, f64) {
    mean_hw(&reports.iter().map(f).collect::<Vec<_>>())
}

fn value(
    r: &Report,
    id: &str,
    f: impl Fn(&stochcraft_blocks::ElementReport) -> Option<f64>,
) -> f64 {
    r.element(id)
        .and_then(f)
        .unwrap_or_else(|| panic!("no value at {id}"))
}

/// StochCraft's mean lies inside the published 95% confidence interval.
fn inside(what: &str, (mean, hw): (f64, f64), (lo, hi): (f64, f64)) {
    assert!(
        lo <= mean && mean <= hi,
        "{what}: StochCraft {mean} ± {hw}, published [{lo}, {hi}]"
    );
}

#[test]
fn case_a_matches_the_published_line() {
    let reports = case_a();
    const MACHINES: [&str; 4] = ["m1", "m2", "m3", "m4"];
    let hours = 2_505_600.0 / 3600.0;

    // Throughput, parts an hour: published 7.38408 [7.33537, 7.43279].
    let th = each(&reports, |r| {
        value(r, "done", |e| Some(e.entered as f64)) / hours
    });
    inside("throughput", th, (7.335_37, 7.432_79));

    // WIP, the time-average number of distinct parts on the line (the supply buffer left out):
    // corrected 4.40220 [4.32209, 4.48231].
    let wip = each(&reports, |r| {
        MACHINES
            .iter()
            .map(|m| value(r, m, |e| e.mean_work_in_progress))
            .sum()
    });
    inside("work in progress", wip, (4.322_09, 4.482_31));

    // Energy per part, kWh: corrected 14.14092 [14.10501, 14.17682] (mean of the ratios).
    let sec = each(&reports, |r| value(r, "done", |e| e.specific_energy));
    inside("energy per part", sec, (14.105_01, 14.176_82));

    // Each machine's share of time in each state, corrected, in %: processing, blocked
    // (transfers included), failed, and starved plus closed. The published model counts a
    // machine idle in a break as starved until it takes a part; StochCraft counts it off shift
    // (the order is failed, processing, blocked, off shift, starved), so only the two together
    // compare. The published shares are means of 25 replications as ours are, so they agree
    // within the half-width of a difference of two such means, √2 times ours.
    let shares = [
        ("m1", [65.64, 6.00, 14.90, 0.00 + 13.46]),
        ("m2", [59.48, 3.01, 14.88, 9.10 + 13.53]),
        ("m3", [27.49, 2.08, 1.84, 55.14 + 13.45]),
        ("m4", [8.41, 0.00, 0.97, 86.01 + 4.61]),
    ];
    for (m, want) in shares {
        let got = [
            each(&reports, |r| value(r, m, |e| e.time_processing)),
            each(&reports, |r| value(r, m, |e| e.time_blocked)),
            each(&reports, |r| value(r, m, |e| e.time_failed)),
            each(&reports, |r| {
                value(r, m, |e| e.time_starved) + value(r, m, |e| e.time_off_shift)
            }),
        ];
        for (i, ((mean, hw), want)) in got.into_iter().zip(want).enumerate() {
            let (mean, hw, want) = (100.0 * mean, 100.0 * hw, want);
            assert!(
                (mean - want).abs() <= std::f64::consts::SQRT_2 * hw + 1e-9,
                "{m} share {i}: StochCraft {mean} ± {hw}%, published {want}%"
            );
        }
    }
}
