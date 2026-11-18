//! The behaviour checks ([`stochcraft_blocks::checks`]): none fires on runs that are right,
//! with a warm-up, breakdowns, a break, a finite buffer, impatient entities, a delay and a
//! batch; and each finds a report made wrong on purpose.
#![allow(clippy::panic)]

use stochcraft_blocks::checks::{check_report, check_reports};
use stochcraft_blocks::{
    Breakdown, Builder, ElementReport, Event, OnShiftEnd, Patience, PatienceUntil, Report, compile,
};
use stochcraft_model::{Behaviour, DateTime, Diagnostic, Experiment, Model, Problem};
use stochcraft_random::{Dist, Rng};

fn at(h: u32, m: u32) -> DateTime {
    DateTime::new(2016, 1, 1, h, m, 0).unwrap_or(DateTime(0))
}

/// A break from `from` to `to` (hours and minutes of 1 January 2016; the time unit is minutes).
fn once(from: (u32, u32), to: (u32, u32)) -> Event {
    Event {
        start: at(from.0, from.1),
        end: at(to.0, to.1),
        rule: None,
    }
}

/// A line with something of everything the checks look at: arrivals turned away at a full
/// buffer; a press with two units that fail (part-way through jobs) and stop for two breaks,
/// finishing the job under way; a desk whose entities give up waiting, or during their job; a
/// delay; and, with `batch`, a batch of 2 before the exit.
fn line(batch: bool) -> Model {
    let mut b = Builder::new();
    b.calendar(
        "break",
        vec![once((0, 50), (1, 10)), once((2, 30), (2, 45))],
    );
    let parts = b.source("parts", Dist::exponential_rate(1.0), None);
    let press = b.buffered_server("press", 2, Dist::exponential_rate(0.6), 5);
    b.breakdown(
        press,
        Breakdown {
            time_to_failure: Dist::Exponential { mean: 30.0 },
            repair: Dist::Exponential { mean: 4.0 },
            interruptible: true,
            hands_on_while_down: true,
        },
    );
    b.server_shifts(press, 2, &[("break", 0)], OnShiftEnd::Finish);
    let desk = b.server("desk", 1, Dist::exponential_rate(1.1));
    b.patience(
        desk,
        Patience {
            time: Dist::Exponential { mean: 3.0 },
            until: PatienceUntil::Done,
        },
    );
    let cure = b.delay("cure", Dist::Uniform { min: 1.0, max: 5.0 });
    let out = b.sink("out");
    b.connect(parts, press);
    b.connect(press, desk);
    b.connect(desk, cure);
    if batch {
        let pack = b.batch("pack", 2);
        b.connect(cure, pack);
        b.connect(pack, out);
    } else {
        b.connect(cure, out);
    }
    b.build(Experiment {
        warmup: 30.0,
        duration: 200.0,
        replications: 4,
        start: Some(at(0, 0)),
        ..Experiment::default()
    })
}

fn run(model: &Model, seed: u64) -> Report {
    let compiled = compile(model).unwrap_or_else(|d| panic!("{d:?}"));
    compiled
        .run(Rng::new(seed))
        .unwrap_or_else(|e| panic!("{e}"))
}

/// The checks' findings on `report`, by element (`None`: the model) and code.
fn found(model: &Model, report: &Report) -> Vec<(Option<String>, &'static str)> {
    check_report(model, report)
        .iter()
        .map(|d| (d.at.as_ref().map(|a| a.to_string()), d.problem.code()))
        .collect()
}

fn element<'a>(r: &'a mut Report, id: &str) -> &'a mut ElementReport {
    r.elements
        .iter_mut()
        .find(|e| e.id.as_str() == id)
        .unwrap_or_else(|| panic!("no {id}"))
}

/// A report of the line that the checks pass, with what it must have to test them.
fn good() -> (Model, Report) {
    good_line(true)
}

fn good_line(batch: bool) -> (Model, Report) {
    let model = line(batch);
    let r = run(&model, 3);
    assert_eq!(found(&model, &r), vec![], "{r:?}");
    let press = r.element("press").unwrap_or_else(|| panic!("no press"));
    assert!(press.lost.is_some_and(|n| n > 0), "{press:?}");
    assert!(press.time_failed.is_some_and(|f| f > 0.0), "{press:?}");
    assert!(press.time_off_shift.is_some_and(|f| f > 0.0), "{press:?}");
    let desk = r.element("desk").unwrap_or_else(|| panic!("no desk"));
    assert!(desk.abandoned.is_some_and(|n| n > 0), "{desk:?}");
    assert!(r.elements.iter().any(|e| e.inside_at_start > 0));
    (model, r)
}

fn only(found: Vec<(Option<String>, &'static str)>, at: Option<&str>, code: &str) {
    assert!(
        found.iter().any(|(a, c)| a.as_deref() == at && *c == code),
        "{at:?} {code} not in {found:?}"
    );
}

#[test]
fn no_check_fires_on_runs_that_are_right() {
    for batch in [true, false] {
        let model = line(batch);
        for seed in 1..=20 {
            let r = run(&model, seed);
            assert_eq!(found(&model, &r), vec![], "seed {seed}");
        }
        // No warm-up, and a long one.
        for (warmup, duration) in [(0.0, 50.0), (500.0, 10.0)] {
            let mut m = model.clone();
            m.experiment.warmup = warmup;
            m.experiment.duration = duration;
            let r = run(&m, 5);
            assert_eq!(found(&m, &r), vec![], "{warmup} {duration}");
        }
        // Too short for any entity to reach the exit: some arrivals, turned away at the press's
        // full buffer, still left the model, so nothing is found.
        let mut m = model.clone();
        m.experiment.duration = 0.5;
        let r = run(&m, 5);
        assert_eq!(r.element("out").map(|e| e.entered), Some(0));
        assert_eq!(found(&m, &r), vec![]);
    }
}

#[test]
fn an_entity_missing_from_an_element_is_found() {
    let (model, mut r) = good();
    element(&mut r, "cure").left -= 1;
    only(found(&model, &r), Some("cure"), "unbalanced");
    let (model, mut r) = good();
    element(&mut r, "press").inside_at_end += 1;
    only(found(&model, &r), Some("press"), "unbalanced");
}

#[test]
fn an_entity_missing_from_the_model_is_found() {
    // One more reaching the exit than came in, with a batch and without: the exit's own count
    // still adds up.
    for batch in [true, false] {
        let (model, mut r) = good_line(batch);
        let out = element(&mut r, "out");
        out.entered += 1;
        out.left += 1;
        let f = found(&model, &r);
        only(f.clone(), None, "unaccounted");
        assert!(!f.iter().any(|(a, _)| a.as_deref() == Some("out")), "{f:?}");
    }
}

/// An entity that gives up during its job (patience until done) is counted once: abandoned as
/// it gives up, its stand-in taking its way, though it stays inside until its unit is done with
/// it. So the desk's count is exact, even where the statistics start or end with such entities
/// inside, and one entity missing there is found.
#[test]
fn an_entity_given_up_during_its_job_is_counted_once() {
    let model = line(false);
    let (mut at_start, mut at_end) = (false, false);
    for seed in 1..=40 {
        for warmup in [30.0, 31.5, 47.0] {
            let mut m = model.clone();
            m.experiment.warmup = warmup;
            let r = run(&m, seed);
            assert_eq!(found(&m, &r), vec![], "seed {seed} warm-up {warmup}");
            let desk = r.element("desk").unwrap_or_else(|| panic!("no desk"));
            at_start |= desk.given_up_inside[0] > 0;
            at_end |= desk.given_up_inside[1] > 0;
        }
    }
    // The boundaries were crossed with such entities inside.
    assert!(at_start && at_end, "{at_start} {at_end}");
    // One entity missing from the desk, which the allowance of its units hid before.
    let (model, mut r) = good_line(false);
    element(&mut r, "desk").left -= 1;
    only(found(&model, &r), Some("desk"), "unbalanced");
}

#[test]
fn a_model_whose_entities_never_get_out_is_found() {
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(1.0), None);
    let wait = b.delay("wait", Dist::Constant(1_000.0));
    let out = b.sink("out");
    b.connect(src, wait);
    b.connect(wait, out);
    let model = b.build(Experiment {
        duration: 100.0,
        ..Experiment::default()
    });
    let r = run(&model, 1);
    assert_eq!(found(&model, &r), vec![(None, "nothing_out")]);
    // Long enough, it gets out.
    let mut longer = model.clone();
    longer.experiment.duration = 2_000.0;
    assert_eq!(found(&longer, &run(&longer, 1)), vec![]);
}

#[test]
fn a_time_average_out_of_line_with_the_stays_is_found() {
    let (model, mut r) = good();
    let desk = element(&mut r, "desk");
    desk.mean_work_in_progress = desk.mean_work_in_progress.map(|m| m * 1.001);
    only(found(&model, &r), Some("desk"), "littles_law");
    // An entity's stay of one minute missing.
    let (model, mut r) = good();
    let cure = element(&mut r, "cure");
    cure.time_inside = cure.time_inside.map(|t| t - 1.0);
    only(found(&model, &r), Some("cure"), "littles_law");
}

#[test]
fn a_break_counted_as_busy_is_found() {
    // The press counted busy through its breaks: the time off shift added to its utilisation.
    let (model, mut r) = good();
    let press = element(&mut r, "press");
    let off = press.time_off_shift.unwrap_or(0.0);
    press.utilisation = press.utilisation.map(|u| u + off);
    only(found(&model, &r), Some("press"), "busy_not_working");
    // Time failed part-way through a job counted busy.
    let (model, mut r) = good();
    let press = element(&mut r, "press");
    let failed = press.time_failed.unwrap_or(0.0);
    press.utilisation = press.utilisation.map(|u| u + failed);
    only(found(&model, &r), Some("press"), "busy_not_working");
    // And the other way: processing not counted busy.
    let (model, mut r) = good();
    let press = element(&mut r, "press");
    press.utilisation = press.utilisation.map(|u| u * 0.9);
    only(found(&model, &r), Some("press"), "busy_not_working");
}

#[test]
fn a_unit_failed_with_a_job_is_not_busy() {
    // Without breaks, the press's utilisation is its share of time processing exactly, though
    // its units fail part-way through jobs and keep holding them while down.
    let mut b = Builder::new();
    let parts = b.source("parts", Dist::exponential_rate(1.0), None);
    let press = b.server("press", 2, Dist::exponential_rate(0.6));
    b.breakdown(
        press,
        Breakdown {
            time_to_failure: Dist::Exponential { mean: 30.0 },
            repair: Dist::Exponential { mean: 10.0 },
            interruptible: true,
            hands_on_while_down: true,
        },
    );
    let out = b.sink("out");
    b.connect(parts, press);
    b.connect(press, out);
    let model = b.build(Experiment {
        duration: 2_000.0,
        ..Experiment::default()
    });
    let r = run(&model, 5);
    let p = r.element("press").unwrap_or_else(|| panic!("no press"));
    let (u, processing, failed) = (
        p.utilisation.unwrap_or(f64::NAN),
        p.time_processing.unwrap_or(f64::NAN),
        p.time_failed.unwrap_or(0.0),
    );
    assert!(failed > 0.05, "{p:?}");
    assert!((u - processing).abs() < 1e-12, "{u} vs {processing}");
    // Its units held more jobs than they worked on: the contents count the entities inside.
    assert!(p.mean_contents.is_some_and(|c| c / 2.0 > u + 0.01), "{p:?}");
}

#[test]
fn states_not_adding_up_are_found() {
    let (model, mut r) = good();
    let press = element(&mut r, "press");
    press.time_starved = press.time_starved.map(|s| s + 0.01);
    only(found(&model, &r), Some("press"), "states_total");
}

#[test]
fn a_share_outside_0_to_1_is_found() {
    let (model, mut r) = good();
    element(&mut r, "press").availability = Some(1.2);
    only(found(&model, &r), Some("press"), "out_of_range");
    let (model, mut r) = good();
    element(&mut r, "desk").utilisation = Some(-0.1);
    only(found(&model, &r), Some("desk"), "out_of_range");
}

#[test]
fn replications_are_merged_into_one_warning_with_the_worst() {
    let (model, a) = good();
    let mut b = a.clone();
    let mut c = a.clone();
    element(&mut b, "cure").left -= 1;
    element(&mut c, "cure").left -= 3;
    let merged = check_reports(&model, &[a, b, c]);
    let [
        Diagnostic {
            problem:
                Problem::Behaviour {
                    found: Behaviour::Unbalanced { came, went },
                    replications: 2,
                    of: 3,
                },
            ..
        },
    ] = merged.as_slice()
    else {
        panic!("{merged:?}");
    };
    assert_eq!(came - went, 3);
    let text = merged[0].to_string();
    assert!(text.contains("in 2 of 3 replications"), "{text}");
}

/// Those who give up take their way to a callback desk with one place, which holds them up when
/// it is full: waiting for room on the way, they are in neither element, and are counted as
/// leaving the desk they gave up at. So the model's count is exact, also where the statistics
/// start or end with such entities waiting, and one of them missing is found.
#[test]
fn entities_waiting_for_room_on_a_marked_way_are_counted() {
    let mut b = Builder::new();
    let calls = b.source("calls", Dist::exponential_rate(2.0), None);
    let desk = b.server("desk", 1, Dist::exponential_rate(1.0));
    b.patience(
        desk,
        Patience {
            time: Dist::Exponential { mean: 1.0 },
            until: PatienceUntil::Start,
        },
    );
    let callback = b.buffered_server("callback", 1, Dist::Constant(5.0), 1);
    let out = b.sink("out");
    b.connect(calls, desk);
    b.connect(desk, out);
    b.abandoned(desk, callback);
    b.connect(callback, out);
    let model = b.build(Experiment {
        warmup: 20.0,
        duration: 200.0,
        ..Experiment::default()
    });
    let (mut at_start, mut at_end) = (false, false);
    let mut missing = None;
    for seed in 1..=20 {
        let r = run(&model, seed);
        assert_eq!(found(&model, &r), vec![], "seed {seed}");
        let desk = r.element("desk").unwrap_or_else(|| panic!("no desk"));
        at_start |= desk.leaving[0] > 0;
        if desk.leaving[1] > 0 {
            at_end = true;
            missing.get_or_insert(r);
        }
    }
    assert!(at_start && at_end, "{at_start} {at_end}");
    // One of those waiting at the end missing.
    let Some(mut r) = missing else {
        panic!("none waiting at the end");
    };
    element(&mut r, "desk").leaving[1] -= 1;
    assert_eq!(found(&model, &r), vec![(None, "unaccounted")]);
}

/// Where entities are copied, combined and split, the model's count holds with the copies and
/// the batches as new entities, and those combined into them ended: a parallel gateway sends
/// each part to a press and a copy to a delay, the press's parts are packed in threes and
/// unpacked again.
#[test]
fn copies_and_batches_are_counted_as_new_entities() {
    let mut b = Builder::new();
    let parts = b.source("parts", Dist::exponential_rate(1.0), None);
    let split = b.parallel("split");
    let press = b.server("press", 2, Dist::exponential_rate(0.7));
    let pack = b.batch("pack", 3);
    let unpack = b.unbatch("unpack");
    let cure = b.delay("cure", Dist::Uniform { min: 1.0, max: 5.0 });
    let out = b.sink("out");
    b.connect(parts, split);
    b.connect(split, press);
    b.connect(split, cure);
    b.connect(press, pack);
    b.connect(pack, unpack);
    b.connect(unpack, out);
    b.connect(cure, out);
    let model = b.build(Experiment {
        warmup: 20.0,
        duration: 200.0,
        ..Experiment::default()
    });
    for seed in 1..=20 {
        for warmup in [0.0, 20.0, 33.3] {
            let mut m = model.clone();
            m.experiment.warmup = warmup;
            assert_eq!(
                found(&m, &run(&m, seed)),
                vec![],
                "seed {seed} warm-up {warmup}"
            );
        }
    }
    // A copy, a batch or an unpacked part too many or too few is found.
    for (id, by) in [("split", 1i64), ("pack", -1), ("unpack", 1)] {
        let mut r = run(&model, 3);
        let e = element(&mut r, id);
        e.left = e.left.saturating_add_signed(by);
        assert!(
            found(&model, &r).contains(&(None, "unaccounted")),
            "{id}: {:?}",
            found(&model, &r)
        );
    }
}

/// Entities that leave the model other than at an exit (here, all of them give up, with no way
/// marked for them) have got out: no warning that nothing did.
#[test]
fn leaving_by_any_way_out_counts_as_getting_out() {
    let mut b = Builder::new();
    let calls = b.source("calls", Dist::exponential_rate(1.0), None);
    let desk = b.server("desk", 1, Dist::Constant(1_000.0));
    b.patience(
        desk,
        Patience {
            time: Dist::Constant(2.0),
            until: PatienceUntil::Start,
        },
    );
    let out = b.sink("out");
    b.connect(calls, desk);
    b.connect(desk, out);
    let model = b.build(Experiment {
        duration: 100.0,
        ..Experiment::default()
    });
    let r = run(&model, 1);
    assert_eq!(r.element("out").map(|e| e.entered), Some(0), "{r:?}");
    assert!(
        r.element("desk")
            .and_then(|e| e.abandoned)
            .is_some_and(|n| n > 0)
    );
    assert_eq!(found(&model, &r), vec![]);
}
