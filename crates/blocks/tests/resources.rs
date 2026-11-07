//! Shared resource pools against exact results: two tasks sharing c units (one M/M/c with the
//! arrivals pooled), a non-preemptive priority queue (Cobham's formulas), the machine-repair
//! model (M/M/R//N), and deterministic cases worked by hand for alternatives, roles and several
//! units at once.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, ExperimentSummary, Metric, Report, compile, summarise};
use stochcraft_model::{Dist, ElementId, Experiment, Model, Need};
use stochcraft_random::Rng;

const REPS: u64 = 16;
/// Each check is a 95% interval, so about one in twenty would miss by chance; the seed is fixed
/// so the tests are repeatable.
const SEED: u64 = 50_000;

fn experiment(warmup: f64, duration: f64) -> Experiment {
    Experiment {
        warmup,
        duration,
        replications: REPS as u32,
        ..Experiment::default()
    }
}

fn reports(model: &Model) -> Vec<Report> {
    let compiled = compile(model);
    let Ok(compiled) = compiled else {
        panic!("{compiled:?}")
    };
    (0..REPS)
        .map(|r| {
            compiled
                .run(Rng::new(SEED + r))
                .unwrap_or_else(|e| panic!("{e}"))
        })
        .collect()
}

fn covers(s: &ExperimentSummary, element: &str, metric: Metric, theory: f64) {
    let got = s.get(element, metric);
    eprintln!("{element} {}: {got:?} (theory {theory})", metric.name());
    assert!(
        got.is_some_and(|g| g.n == REPS && g.contains(theory)),
        "{element} {}: {got:?} vs {theory}",
        metric.name()
    );
}

fn pool(id: &str, quantity: u32) -> Need {
    Need::Resource {
        id: ElementId::unchecked(id),
        quantity,
    }
}

/// Erlang C: the probability an arrival waits in M/M/c with offered load a = λ/μ.
fn erlang_c(c: u32, a: f64) -> f64 {
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..c {
        term *= a / f64::from(k);
        sum += term;
    }
    let last = term * a / f64::from(c) * (f64::from(c) / (f64::from(c) - a));
    last / (sum + last)
}

/// Two tasks share a pool of c = 3 units, with Poisson arrivals at 0.9 and 1.2 and the same
/// exponential service at rate 1: together they are one M/M/3 with λ = 2.1, and with equal
/// priorities the units go first come first served across both, so each task waits the M/M/3
/// mean Wq = C(c, a) / (cμ − λ).
#[test]
fn two_tasks_sharing_a_pool_wait_as_one_mmc() {
    let (l1, l2, mu, c) = (0.9, 1.2, 1.0, 3u32);
    let mut b = Builder::new();
    b.pool("staff", c, &[]);
    let s1 = b.source("calls", Dist::exponential_rate(l1), None);
    let s2 = b.source("walkins", Dist::exponential_rate(l2), None);
    let t1 = b.shared_task(
        "phone",
        Dist::exponential_rate(mu),
        vec![pool("staff", 1)],
        0.0,
    );
    let t2 = b.shared_task(
        "desk",
        Dist::exponential_rate(mu),
        vec![pool("staff", 1)],
        0.0,
    );
    let out = b.sink("done");
    b.connect(s1, t1);
    b.connect(s2, t2);
    b.connect(t1, out);
    b.connect(t2, out);
    let model = b.build(experiment(1_000.0, 20_000.0));
    let s = summarise(&reports(&model));
    let lambda = l1 + l2;
    let a = lambda / mu;
    let wq = erlang_c(c, a) / (f64::from(c) * mu - lambda);
    covers(&s, "phone", Metric::MeanWait, wq);
    covers(&s, "desk", Metric::MeanWait, wq);
    covers(&s, "staff", Metric::Utilisation, a / f64::from(c));
    covers(&s, "staff", Metric::MeanContents, a);
}

/// Non-preemptive priority M/M/1 (Cobham): one unit shared by an urgent task (priority 1) and a
/// routine one (priority 0), both served exponentially at rate μ. With W0 = Σ λi E[S²] / 2 = ρ/μ,
/// the urgent task waits W0 / (1 − ρ1) and the routine one W0 / ((1 − ρ1)(1 − ρ)).
#[test]
fn a_higher_priority_task_is_served_first() {
    let (l1, l2, mu) = (0.3, 0.45, 1.0);
    let mut b = Builder::new();
    b.pool("clerk", 1, &[]);
    let s1 = b.source("urgent_in", Dist::exponential_rate(l1), None);
    let s2 = b.source("routine_in", Dist::exponential_rate(l2), None);
    let t1 = b.shared_task(
        "urgent",
        Dist::exponential_rate(mu),
        vec![pool("clerk", 1)],
        1.0,
    );
    let t2 = b.shared_task(
        "routine",
        Dist::exponential_rate(mu),
        vec![pool("clerk", 1)],
        0.0,
    );
    let out = b.sink("done");
    b.connect(s1, t1);
    b.connect(s2, t2);
    b.connect(t1, out);
    b.connect(t2, out);
    let model = b.build(experiment(1_000.0, 20_000.0));
    let s = summarise(&reports(&model));
    let (r1, r) = (l1 / mu, (l1 + l2) / mu);
    let w0 = r / mu;
    covers(&s, "urgent", Metric::MeanWait, w0 / (1.0 - r1));
    covers(
        &s,
        "routine",
        Metric::MeanWait,
        w0 / ((1.0 - r1) * (1.0 - r)),
    );
    covers(&s, "clerk", Metric::Utilisation, r);
}

/// The machine-repair model M/M/R//N: N machines each run for an exponential time at rate λ,
/// then wait for one of R repairmen, who repair at rate μ. The number broken is a birth-death
/// chain on 0..=N with birth rate (N − n)λ and death rate min(n, R)μ; the repairmen's
/// utilisation is E[min(n, R)] / R and the mean number running is N − E[n].
#[test]
fn machines_waiting_for_repairmen_match_the_closed_form() {
    let (n, r, lambda, mu) = (6usize, 2u32, 0.25, 1.0);
    let mut p = vec![1.0f64];
    for k in 1..=n {
        let birth = (n - (k - 1)) as f64 * lambda;
        let death = f64::from(r.min(k as u32)) * mu;
        p.push(p[k - 1] * birth / death);
    }
    let total: f64 = p.iter().sum();
    let p: Vec<f64> = p.iter().map(|x| x / total).collect();
    let broken: f64 = p.iter().enumerate().map(|(k, pk)| k as f64 * pk).sum();
    let busy: f64 = p
        .iter()
        .enumerate()
        .map(|(k, pk)| f64::from(r.min(k as u32)) * pk)
        .sum();

    // N machines, all made at the start, then circulating for ever: running → repair → running.
    let mut b = Builder::new();
    b.pool("repairmen", r, &[]);
    let start = b.source("machines", Dist::Constant(0.0), Some(n as u64));
    let running = b.delay("running", Dist::exponential_rate(lambda));
    let repair = b.shared_task(
        "repair",
        Dist::exponential_rate(mu),
        vec![pool("repairmen", 1)],
        0.0,
    );
    let back = b.exclusive("back");
    b.connect(start, back);
    b.connect(back, running);
    b.connect(running, repair);
    b.connect(repair, back);
    // Longer than the other tests: the two checks move together (running = N − broken), and
    // 20,000 left them about one half-width out with this seed, a 1-in-20 miss that a run of
    // 400,000 showed to be chance (both well inside, no bias).
    let model = b.build(experiment(500.0, 60_000.0));
    let s = summarise(&reports(&model));
    covers(&s, "repairmen", Metric::Utilisation, busy / f64::from(r));
    covers(&s, "running", Metric::MeanContents, n as f64 - broken);
    covers(&s, "repair", Metric::MeanQueueLength, broken - busy);
}

/// Alternatives are tried in order (`orResource`): with arrivals every 1 and service of 3, the
/// first entity takes the preferred pool's only unit and the next two the backup's; then each
/// unit comes free just as the next arrival needs one, so nobody waits.
#[test]
fn alternatives_are_tried_in_order() {
    let mut b = Builder::new();
    b.pool("preferred", 1, &[]);
    b.pool("backup", 2, &[]);
    let src = b.source("in", Dist::Constant(1.0), None);
    let t = b.shared_task(
        "work",
        Dist::Constant(3.0),
        vec![pool("preferred", 1), pool("backup", 1)],
        0.0,
    );
    let out = b.sink("out");
    b.connect(src, t);
    b.connect(t, out);
    let r = b.run(Rng::new(1), 0.0, 30.0);
    let Ok(r) = r else { panic!("{r:?}") };
    // Three units, one arrival a time unit, three time units each: never a wait, all busy.
    let work = r.element("work").and_then(|e| e.mean_wait);
    assert_eq!(work, Some(0.0), "{r:?}");
    let pref = r.element("preferred").map(|e| e.entered);
    let back = r.element("backup").map(|e| e.entered);
    // Arrivals at 1, 2, …, 30 (30 of them): the preferred unit takes those at 1, 4, 7, …, 28
    // (10), the backup the rest (20).
    assert_eq!((pref, back), (Some(10), Some(20)), "{r:?}");
}

/// Roles pick the pools (`getResourceByRoles`), and a need for several units takes them all at
/// once, from the matching pools in model order: here 2 units with the role `nurse`, from a pool
/// of 1 nurse and a pool of 2 doctors who are also nurses.
#[test]
fn roles_choose_the_pools_and_units_are_taken_together() {
    let mut b = Builder::new();
    b.pool("nurses", 1, &["nurse"]);
    b.pool("doctors", 2, &["nurse", "doctor"]);
    let src = b.source("in", Dist::Constant(2.0), None);
    let t = b.shared_task(
        "turn_patient",
        Dist::Constant(1.0),
        vec![Need::Roles {
            roles: vec!["nurse".into()],
            quantity: 2,
        }],
        0.0,
    );
    let out = b.sink("out");
    b.connect(src, t);
    b.connect(t, out);
    let r = b.run(Rng::new(1), 0.0, 20.0);
    let Ok(r) = r else { panic!("{r:?}") };
    // Each patient (at 2, 4, …, 20) takes the nurse and one doctor for 1; the nine served before
    // the end keep the nurse busy 9 of 20 time units, and one of the doctors' two units as long.
    let util = |id: &str| r.element(id).and_then(|e| e.utilisation);
    let near = |x: Option<f64>, v: f64| x.is_some_and(|x| (x - v).abs() < 1e-9);
    assert!(near(util("nurses"), 0.45), "{r:?}");
    assert!(near(util("doctors"), 0.225), "{r:?}");
}

/// A need for more units than are free waits until enough are free at once, holding none in the
/// meantime; an entity that can be served is not held back by one waiting ahead of it that
/// cannot.
#[test]
fn a_large_need_waits_without_holding_back_smaller_ones() {
    let mut b = Builder::new();
    b.pool("crew", 3, &[]);
    // At time 0 a small job (1 unit for 10) starts; at 1 a big job (3 units) arrives and must
    // wait until 10; at 2 another small job arrives and, since 2 units are free, starts at once.
    let small = b.source("small_in", Dist::Constant(0.0), Some(1));
    let big = b.source("big_in", Dist::Constant(1.0), Some(1));
    let small2 = b.source("small2_in", Dist::Constant(2.0), Some(1));
    let t1 = b.shared_task(
        "small_job",
        Dist::Constant(10.0),
        vec![pool("crew", 1)],
        0.0,
    );
    let t2 = b.shared_task("big_job", Dist::Constant(1.0), vec![pool("crew", 3)], 0.0);
    let t3 = b.shared_task(
        "small_job2",
        Dist::Constant(1.0),
        vec![pool("crew", 1)],
        0.0,
    );
    let out = b.sink("out");
    for (s, t) in [(small, t1), (big, t2), (small2, t3)] {
        b.connect(s, t);
        b.connect(t, out);
    }
    let r = b.run(Rng::new(1), 0.0, 20.0);
    let Ok(r) = r else { panic!("{r:?}") };
    let wait = |id: &str| r.element(id).and_then(|e| e.mean_wait);
    assert_eq!(wait("big_job"), Some(9.0), "{r:?}");
    assert_eq!(wait("small_job2"), Some(0.0), "{r:?}");
}
