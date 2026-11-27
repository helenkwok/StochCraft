//! A shared task that needs units of several pools at once (a job needing a machine and a
//! worker): the units are taken all together, only when every requirement can be met, and given
//! back together. The cases are worked by hand with constant times.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, ElementReport, Report};
use stochcraft_model::{Dist, ElementId, Experiment, Need, Problem, Requirement, Severity};
use stochcraft_random::Rng;

fn pool(id: &str, quantity: u32) -> Need {
    Need::Resource {
        id: ElementId::unchecked(id),
        quantity,
    }
}

/// A requirement with one alternative: `quantity` units of pool `id`.
fn one(id: &str, quantity: u32) -> Requirement {
    Requirement::new(vec![pool(id, quantity)])
}

fn element<'a>(r: &'a Report, id: &str) -> &'a ElementReport {
    r.element(id).unwrap_or_else(|| panic!("no {id} in {r:?}"))
}

fn exactly(what: &str, got: Option<f64>, want: f64) {
    assert!(
        got.is_some_and(|g| (g - want).abs() < 1e-9),
        "{what}: {got:?} vs {want}"
    );
}

/// One machine and one worker. Upkeep takes the worker alone from 0 to 10. Jobs, which need the
/// machine and the worker at once, arrive at 1 and 2; a check, which needs only the machine,
/// arrives at 3.
///
/// - The jobs wait: the worker is busy, and they hold no machine meanwhile.
/// - So the check finds the machine free and is served at once (3 to 4), overtaking them.
/// - At 10 the worker is free: the first job takes the machine and the worker together, until
///   12.
/// - At 12 it gives both back, and the second job takes both, until 14.
///
/// Waits: the jobs 9 and 10 (9.5 on average), the check 0. Over 20, the machine is busy 1 + 2 + 2
/// = 5 (25%), the worker 10 + 2 + 2 = 14 (70%), and each was taken three times.
#[test]
fn a_job_takes_the_machine_and_the_worker_together_and_gives_both_back() {
    let mut b = Builder::new();
    b.pool("machine", 1, &[]);
    b.pool("worker", 1, &[]);
    let upkeep_in = b.source("upkeep_in", Dist::Constant(0.0), Some(1));
    let jobs_in = b.source("jobs_in", Dist::Constant(1.0), Some(2));
    let check_in = b.source("check_in", Dist::Constant(3.0), Some(1));
    let upkeep = b.shared_task("upkeep", Dist::Constant(10.0), vec![pool("worker", 1)], 0.0);
    let job = b.shared_task_all(
        "job",
        Dist::Constant(2.0),
        vec![one("machine", 1), one("worker", 1)],
        0.0,
    );
    let check = b.shared_task("check", Dist::Constant(1.0), vec![pool("machine", 1)], 0.0);
    let out = b.sink("out");
    for (s, t) in [(upkeep_in, upkeep), (jobs_in, job), (check_in, check)] {
        b.connect(s, t);
        b.connect(t, out);
    }
    let r = b.run(Rng::new(1), 0.0, 20.0);
    let Ok(r) = r else { panic!("{r:?}") };
    exactly("job wait", element(&r, "job").mean_wait, 9.5);
    exactly("check wait", element(&r, "check").mean_wait, 0.0);
    exactly("upkeep wait", element(&r, "upkeep").mean_wait, 0.0);
    exactly("machine", element(&r, "machine").utilisation, 0.25);
    exactly("worker", element(&r, "worker").utilisation, 0.7);
    assert_eq!(element(&r, "machine").entered, 3, "{r:?}");
    assert_eq!(element(&r, "worker").entered, 3, "{r:?}");
    assert_eq!(element(&r, "job").left, 2, "{r:?}");
}

/// When an earlier requirement's first alternative would leave a later one unmet, it takes its
/// next: the task needs (A or B) and A, with one unit of each, so the first requirement takes B
/// and the second A, and nobody waits. A pool both requirements draw on is taken once: the
/// other task needs two units of C, one for each requirement.
#[test]
fn the_first_way_that_meets_every_requirement_is_taken() {
    let mut b = Builder::new();
    b.pool("a", 1, &[]);
    b.pool("b", 1, &[]);
    b.pool("c", 2, &[]);
    let src = b.source("in", Dist::Constant(5.0), None);
    let both = b.shared_task_all(
        "both",
        Dist::Constant(1.0),
        vec![
            Requirement::new(vec![pool("a", 1), pool("b", 1)]),
            one("a", 1),
        ],
        0.0,
    );
    let src2 = b.source("in2", Dist::Constant(5.0), None);
    let twice = b.shared_task_all(
        "twice",
        Dist::Constant(1.0),
        vec![one("c", 1), one("c", 1)],
        0.0,
    );
    let out = b.sink("out");
    b.connect(src, both);
    b.connect(both, out);
    b.connect(src2, twice);
    b.connect(twice, out);
    let r = b.run(Rng::new(1), 0.0, 18.0);
    let Ok(r) = r else { panic!("{r:?}") };
    // Arrivals at 5, 10 and 15, each served at once for 1.
    exactly("both wait", element(&r, "both").mean_wait, 0.0);
    let taken = |id: &str| element(&r, id).entered;
    assert_eq!(taken("both"), 3, "{r:?}");
    assert_eq!((taken("a"), taken("b")), (3, 3), "{r:?}");
    // Both units of C at once, taken once a job: 2 units for 1, three times, over 2 × 18.
    exactly("twice wait", element(&r, "twice").mean_wait, 0.0);
    assert_eq!(taken("c"), 3, "{r:?}");
    exactly("c", element(&r, "c").utilisation, 6.0 / 36.0);
}

/// The model's checks look at each requirement: a pool the model lacks is reported as for a
/// task with one requirement, and requirements that together ask more of one pool than it has
/// can never be met. A service time by pool follows the first requirement's pool only, so one
/// for a pool only a later requirement uses is reported.
#[test]
fn each_requirement_is_checked() {
    let errors = |requirements: Vec<Requirement>, by_resource: Option<&str>| {
        let mut b = Builder::new();
        b.pool("machine", 2, &[]);
        b.pool("worker", 1, &[]);
        let src = b.source("in", Dist::exponential_rate(1.0), None);
        let job = b.shared_task_all("job", Dist::exponential_rate(2.0), requirements, 0.0);
        if let Some(p) = by_resource {
            b.processing_by_resource(job, vec![(p, Dist::Constant(1.0))]);
        }
        let out = b.sink("out");
        b.connect(src, job);
        b.connect(job, out);
        b.build(Experiment::default())
            .validate()
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.problem)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        errors(vec![one("machine", 1), one("worker", 1)], None),
        Vec::new()
    );
    assert_eq!(
        errors(vec![one("machine", 1), one("crane", 1)], None),
        vec![Problem::UnknownResource {
            id: ElementId::unchecked("crane")
        }]
    );
    assert_eq!(
        errors(vec![one("worker", 1), one("worker", 1)], None),
        vec![Problem::NeedNeverMet {
            needed: 2,
            available: 1
        }]
    );
    assert_eq!(
        errors(vec![one("machine", 1), one("worker", 1)], Some("machine")),
        Vec::new()
    );
    assert_eq!(
        errors(vec![one("machine", 1), one("worker", 1)], Some("worker")),
        vec![Problem::TimeForPoolNotNeeded {
            id: ElementId::unchecked("worker")
        }]
    );
}

/// Within an alternative by role, the units are spread over the pools that have the role, as
/// many from the first as it has: here that would take A, which the later requirement names,
/// so B serves the role instead. Pools A and B each have one unit with the role `worker`; the
/// task needs a worker (any) and A. Nobody waits, and each pool is taken once a job.
#[test]
fn a_role_leaves_a_pool_that_a_later_requirement_names() {
    let mut b = Builder::new();
    b.pool("a", 1, &["worker"]);
    b.pool("b", 1, &["worker"]);
    let src = b.source("in", Dist::Constant(5.0), None);
    let job = b.shared_task_all(
        "job",
        Dist::Constant(1.0),
        vec![
            Requirement::new(vec![Need::Roles {
                roles: vec!["worker".into()],
                quantity: 1,
            }]),
            one("a", 1),
        ],
        0.0,
    );
    let out = b.sink("out");
    b.connect(src, job);
    b.connect(job, out);
    let r = b.run(Rng::new(1), 0.0, 18.0);
    let Ok(r) = r else { panic!("{r:?}") };
    // Arrivals at 5, 10 and 15, each served at once for 1.
    exactly("job wait", element(&r, "job").mean_wait, 0.0);
    assert_eq!(element(&r, "job").left, 3, "{r:?}");
    assert_eq!(element(&r, "a").entered, 3, "{r:?}");
    assert_eq!(element(&r, "b").entered, 3, "{r:?}");
}
