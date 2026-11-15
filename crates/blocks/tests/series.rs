//! Results over time (`Compiled::record_every`): what is recorded is what a direct look at the
//! run shows at that time, recording changes nothing else, the mean across replications
//! converges to queueing theory, and MSER-5 finds the start-up of a queue started empty.
#![allow(clippy::panic)]

use stochcraft_blocks::series::{Quantity, WARMUP_QUANTITY};
use stochcraft_blocks::{
    Builder, Compiled, ExperimentSummary, Report, compile, record_interval, run_warnings, summarise,
};
use stochcraft_model::{Dist, ElementId, Experiment, Model, Need, Stop, TimeUnit};
use stochcraft_random::Rng;

fn compiled(model: &Model) -> Compiled {
    let c = compile(model);
    let Ok(c) = c else { panic!("{c:?}") };
    c
}

fn run(c: &Compiled, seed: u64) -> Report {
    c.run(Rng::new(seed)).unwrap_or_else(|e| panic!("{e}"))
}

fn summary(c: &Compiled, replications: u64, seed: u64) -> ExperimentSummary {
    let reports: Vec<Report> = (0..replications).map(|r| run(c, seed + r)).collect();
    summarise(&reports)
}

/// Every kind of state: a server with a queue, a delay, a task with a shared pool.
fn plant() -> Model {
    let mut b = Builder::new();
    b.pool("staff", 2, &[]);
    let src = b.source("arrivals", Dist::exponential_rate(1.5), None);
    let desk = b.server("desk", 2, Dist::exponential_rate(1.0));
    let walk = b.delay("walk", Dist::Uniform { min: 0.5, max: 1.5 });
    let help = b.shared_task(
        "help",
        Dist::exponential_rate(1.2),
        vec![Need::Resource {
            id: ElementId::unchecked("staff"),
            quantity: 1,
        }],
        0.0,
    );
    let out = b.sink("exit");
    b.connect(src, desk);
    b.connect(desk, walk);
    b.connect(walk, help);
    b.connect(help, out);
    b.build(Experiment {
        warmup: 20.0,
        duration: 200.0,
        replications: 1,
        ..Experiment::default()
    })
}

/// M/M/1 with arrival rate `lambda` and service rate 1, started empty.
fn mm1(lambda: f64, warmup: f64, duration: f64) -> Model {
    let mut b = Builder::new();
    let src = b.source("arrivals", Dist::exponential_rate(lambda), None);
    let desk = b.server("desk", 1, Dist::exponential_rate(1.0));
    let out = b.sink("exit");
    b.connect(src, desk);
    b.connect(desk, out);
    b.build(Experiment {
        warmup,
        duration,
        ..Experiment::default()
    })
}

fn values(r: &Report, element: usize, q: Quantity) -> &[f64] {
    let rec = r
        .recording
        .as_ref()
        .unwrap_or_else(|| panic!("no recording"));
    rec.elements[element]
        .iter()
        .find(|(x, _)| *x == q)
        .map(|(_, v)| v.as_slice())
        .unwrap_or_else(|| panic!("{q:?} not recorded"))
}

#[test]
fn the_interval_is_the_files_or_a_hundredth_of_the_run() {
    let mut x = Experiment {
        warmup: 20.0,
        duration: 460.0,
        time_unit: TimeUnit::Minutes,
        ..Experiment::default()
    };
    assert_eq!(record_interval(&x), Some(4.8));
    // baseResultFrequency is in seconds: 15 minutes.
    x.result_frequency = Some(900.0);
    assert_eq!(record_interval(&x), Some(15.0));
    // A second over 480 minutes would be 28,800 points: taken 29 at a time.
    x.result_frequency = Some(1.0);
    let every = record_interval(&x).unwrap_or_default();
    assert!((every - 29.0 / 60.0).abs() < 1e-12, "{every}");
    assert!(480.0 / every <= 1_000.0);
    x.warmup = 0.0;
    x.duration = 0.0;
    assert_eq!(record_interval(&x), None);
}

#[test]
fn a_deterministic_line_records_what_happens() {
    // D/D/1: an arrival every 2 (the first at 2), served in 3, so a queue builds by one every
    // 6: the state at each whole time is known exactly.
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(2.0), None);
    let desk = b.server("desk", 1, Dist::Constant(3.0));
    let out = b.sink("out");
    b.connect(src, desk);
    b.connect(desk, out);
    let model = b.build(Experiment {
        duration: 20.0,
        ..Experiment::default()
    });
    let c = compiled(&model).record_every(Some(1.0));
    let r = run(&c, 1);
    let rec = r
        .recording
        .as_ref()
        .unwrap_or_else(|| panic!("no recording"));
    let times: Vec<f64> = (0..=20).map(f64::from).collect();
    assert_eq!(rec.times, times);
    // Arrivals at 2, 4, 6, …; services start at 2, 5, 8, … (one every 3) and end 3 later.
    for (k, &t) in times.iter().enumerate() {
        let arrived = (t / 2.0).floor();
        let done = if t >= 5.0 {
            ((t - 5.0) / 3.0).floor() + 1.0
        } else {
            0.0
        };
        let busy = if arrived > done { 1.0 } else { 0.0 };
        let inside = arrived - done;
        let at = |e, q| values(&r, e, q)[k];
        assert_eq!(at(0, Quantity::Entered), arrived, "created at {t}");
        assert_eq!(at(1, Quantity::Entered), arrived, "entered at {t}");
        assert_eq!(at(1, Quantity::Busy), busy, "busy at {t}");
        assert_eq!(at(1, Quantity::WorkInProgress), inside, "inside at {t}");
        assert_eq!(
            at(1, Quantity::QueueLength),
            inside - busy,
            "waiting at {t}"
        );
        assert_eq!(at(1, Quantity::Left), done, "left at {t}");
        assert_eq!(at(2, Quantity::Entered), done, "out at {t}");
    }
}

#[test]
fn recorded_values_are_what_a_live_run_shows_then() {
    let model = plant();
    let c = compiled(&model).record_every(Some(2.0));
    let plain = compiled(&model);
    for seed in [3, 4] {
        let r = run(&c, seed);
        let rec = r.recording.clone().unwrap_or_default();
        assert_eq!(rec.times.len(), 111, "0, 2, … 220");
        // Advanced to each recorded time without recording, the run shows the same state.
        let mut live = plain.live(Rng::new(seed));
        for (k, &t) in rec.times.iter().enumerate() {
            let a = live.advance_to(t);
            assert!(a.is_ok(), "{a:?}");
            let states = live.states();
            for (i, s) in states.iter().enumerate() {
                for (q, v) in &rec.elements[i] {
                    let seen = match q {
                        Quantity::QueueLength => s.waiting,
                        Quantity::Busy => s.busy,
                        _ => continue,
                    };
                    assert_eq!(v[k], seen, "{:?} {q:?} at {t}", s.id);
                }
            }
        }
        // Counts run from time 0: what they gained after the warm-up (time 20) is what the
        // report counts.
        let warm = rec.times.iter().position(|t| *t == 20.0).unwrap_or(0);
        for (i, e) in r.elements.iter().enumerate().take(model.elements.len()) {
            for (q, v) in &rec.elements[i] {
                let counted = match q {
                    Quantity::Entered => e.entered,
                    Quantity::Left => e.left,
                    _ => continue,
                };
                assert_eq!(v[110] - v[warm], counted as f64, "{} {q:?}", e.id);
            }
        }
        // And the number inside at the end is the report's.
        assert_eq!(
            values(&r, 1, Quantity::WorkInProgress)[110],
            r.elements[1].work_in_progress.unwrap_or(0) as f64
        );
    }
}

#[test]
fn recording_changes_no_result() {
    // With a warm-up, with a stop condition, and in uneven slices.
    let mut stopping = plant();
    stopping.experiment.warmup = 0.0;
    stopping.experiment.stop = Some(Stop::Count {
        element: ElementId::unchecked("exit"),
        count: 150,
    });
    for model in [plant(), stopping] {
        let plain = compiled(&model);
        let recording = compiled(&model).record_every(Some(1.7));
        for seed in 1..=3 {
            let a = run(&plain, seed);
            let mut b = run(&recording, seed);
            let rec = b.recording.take().unwrap_or_default();
            assert_eq!(a, b, "seed {seed}");
            // Recorded up to the end, which the stop condition may have brought forward.
            let last = rec.times.last().copied().unwrap_or(-1.0);
            assert!(
                last <= b.measured_to && b.measured_to - last < 1.7,
                "{last}"
            );
            // The same replication in slices of 7 events records the same values.
            let mut live = recording.live(Rng::new(seed));
            while !live.is_done() {
                let s = live.advance_by(7);
                assert!(s.is_ok(), "{s:?}");
            }
            assert_eq!(live.report().recording, Some(rec), "seed {seed}");
        }
    }
}

#[test]
fn the_mean_queue_converges_to_theory() {
    // M/M/1 at ρ = 0.8: the mean queue length is ρ²/(1 − ρ) = 3.2. The mean across 200
    // replications, averaged over the second half of the run, is within 5% of it.
    let model = mm1(0.8, 0.0, 2_000.0);
    let c = compiled(&model).record_every(record_interval(&model.experiment));
    let s = summary(&c, 200, 100);
    let series = s.series.unwrap_or_else(|| panic!("no series"));
    assert_eq!(series.times.len(), 101);
    assert!(series.n.iter().all(|n| *n == 200));
    let q = &series
        .element("desk")
        .and_then(|e| e.band(Quantity::QueueLength))
        .unwrap_or_else(|| panic!("no queue"))
        .clone();
    let late = &q.mean[50..];
    let mean = late.iter().sum::<f64>() / late.len() as f64;
    assert!((mean - 3.2).abs() < 0.16, "{mean}");
    // The band: the mean's interval sits inside the spread of single runs.
    for k in 1..101 {
        assert!(q.low[k] <= q.mean[k] - q.half_width[k] || q.half_width[k] == 0.0);
        assert!(q.mean[k] + q.half_width[k] <= q.high[k] || q.half_width[k] == 0.0);
    }
}

/// The warnings of a run of `model` with the given replications (seeds 1, 2, …).
fn warnings(model: &Model, replications: u64) -> Vec<String> {
    let c = compiled(model).record_every(record_interval(&model.experiment));
    let s = summary(&c, replications, 1);
    run_warnings(model, &s)
        .iter()
        .map(|d| d.problem.code().to_owned())
        .collect()
}

#[test]
fn mser5_finds_the_start_up_of_a_busy_queue_started_empty() {
    // M/M/1 at ρ = 0.9 started empty fills slowly (its relaxation time is about 1/(1 − √ρ)²,
    // some 380 time units): a warm-up is suggested, and without one the run says so.
    let model = mm1(0.9, 0.0, 2_000.0);
    let c = compiled(&model).record_every(record_interval(&model.experiment));
    for seed in [1, 1_000, 2_000] {
        let s = summary(&c, 30, seed);
        let w = s
            .series
            .as_ref()
            .and_then(|s| s.warmup.clone())
            .unwrap_or_else(|| panic!("no suggestion"));
        let desk = &w.elements[0];
        assert_eq!(desk.id.as_str(), "desk");
        let t = desk.warmup.unwrap_or_else(|| panic!("{w:?}"));
        assert!(t > 0.0 && t <= 1_000.0, "seed {seed}: {t}");
        assert_eq!(w.warmup, Some(t));
        assert_eq!(w.at.as_ref().map(ElementId::as_str), Some("desk"));
        // Starting empty lowers the mean number inside.
        assert!(desk.shift.is_some_and(|s| s.mean < 0.0), "{desk:?}");
        assert!(desk.matters(), "seed {seed}: {desk:?}");
    }
    let found = warnings(&model, 30);
    assert_eq!(found, vec!["no_warmup".to_owned()]);
    // With a warm-up, or a stop condition, there is nothing to warn about.
    assert!(warnings(&mm1(0.9, 500.0, 1_500.0), 30).is_empty());
    let mut stopping = model.clone();
    stopping.experiment.stop = Some(Stop::Count {
        element: ElementId::unchecked("exit"),
        count: 1_000_000,
    });
    assert!(warnings(&stopping, 30).is_empty());
}

#[test]
fn a_light_queue_needs_no_warm_up() {
    // M/M/1 at ρ = 0.3 settles within a few time units, far less than the first interval of a
    // run of 5,000: whatever MSER-5 cuts is chance, and nothing is warned about.
    let model = mm1(0.3, 0.0, 5_000.0);
    let c = compiled(&model).record_every(record_interval(&model.experiment));
    for seed in [1, 1_000, 2_000] {
        let s = summary(&c, 10, seed);
        let w = s
            .series
            .as_ref()
            .and_then(|s| s.warmup.clone())
            .unwrap_or_else(|| panic!("no suggestion"));
        assert!(!w.elements[0].matters(), "seed {seed}: {w:?}");
        let found: Vec<String> = run_warnings(&model, &s)
            .iter()
            .map(|d| d.problem.code().to_owned())
            .collect();
        assert!(found.is_empty(), "seed {seed}: {found:?}");
    }
}

#[test]
fn one_replication_is_warned_about() {
    let model = mm1(0.5, 100.0, 1_000.0);
    assert_eq!(warnings(&model, 1), vec!["one_replication".to_owned()]);
    assert!(warnings(&model, 2).is_empty());
}

#[test]
fn a_stop_condition_gives_series_of_different_lengths() {
    let mut model = mm1(0.5, 0.0, 1_000.0);
    model.experiment.stop = Some(Stop::Count {
        element: ElementId::unchecked("exit"),
        count: 200,
    });
    let c = compiled(&model).record_every(Some(10.0));
    let s = summary(&c, 8, 1);
    let series = s.series.unwrap_or_else(|| panic!("no series"));
    // Each replication stops after about 400; none reaches 1,000.
    assert!(series.times.len() < 101, "{}", series.times.len());
    assert_eq!(series.n[0], 8);
    assert!(series.n.last().is_some_and(|n| *n < 8), "{:?}", series.n);
    assert!(series.n.windows(2).all(|w| w[0] >= w[1]));
    assert_eq!(
        series.warmup, None,
        "no suggestion from series of different lengths"
    );
    let wip = series
        .element("desk")
        .and_then(|e| e.band(WARMUP_QUANTITY))
        .unwrap_or_else(|| panic!("no work in progress"));
    assert_eq!(wip.mean.len(), series.times.len());
}
