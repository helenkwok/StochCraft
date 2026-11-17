//! Behaviour checks: checks on a replication's results that need no reference, made after
//! every replication and reported as warnings ([`Problem::Behaviour`]).
//!
//! Each holds two things the results say that must agree whatever the model, so a failure means
//! the results cannot be right. They find faults in the simulation (a count or a time recorded
//! in one place and not another), and, for [`Behaviour::NothingOut`], a model whose entities
//! never get through. Pernas-Álvarez and Crespo-Pereira (arXiv 2611.10602, 2026) found that
//! checks like these caught about half of the "silent" errors in models agents built, the most
//! common being a break counted as busy time.
//!
//! - Every element's entities add up ([`Behaviour::Unbalanced`]), and the model's
//!   ([`Behaviour::Unaccounted`]): what came in (or was inside as the statistics started) left,
//!   was lost, gave up or is still inside.
//! - Some entity reaches an exit ([`Behaviour::NothingOut`]).
//! - Little's law holds at every server, task with shared resources and delay
//!   ([`Behaviour::LittlesLaw`]).
//! - A server counts busy only units with a job: none idle, blocked or off shift
//!   ([`Behaviour::BusyNotWorking`]), and its units' shares of time in each state add up to 1
//!   ([`Behaviour::StatesTotal`]).
//! - Utilisation and the other shares of time stay within 0 to 1 ([`Behaviour::OutOfRange`]).

use std::collections::BTreeMap;

use stochcraft_model::{
    Behaviour, Diagnostic, ElementId, ElementKind, Model, PatienceUntil, Problem, Routing, Severity,
};

use crate::{ElementReport, Report};

/// The tolerance on Little's law, as a share of the most the time inside could add up to (every
/// entity that came in, or was inside at the start or the end, inside the whole window).
///
/// The two sides are sums of the same times, kept apart: the counts inside, time-weighted, and
/// the times each entity came in and went out. Those inside at the start and the end are counted
/// exactly (from the start, and to the end), so the law holds exactly over any window, however
/// short, with no allowance for end effects, and only rounding separates the two: about one part
/// in 10^16 per term added. Measured over every run in the workspace's tests (November 2026),
/// the largest gap was 1.1 × 10^-14 of this measure, 10^5 times below the tolerance. An entity
/// missed, or counted twice, moves one side by its time inside, which the tolerance hides only
/// when that time is under a billionth of the measure.
pub const LITTLE_TOLERANCE: f64 = 1e-9;

/// The tolerance on shares of time (utilisation, states, their total) and on the units counted
/// busy, as a share of 1 or of the units: rounding only, since each is a ratio of exact sums.
/// Measured over every run in the workspace's tests, the largest gap was 2.1 × 10^-14.
pub const SHARE_TOLERANCE: f64 = 1e-9;

/// The behaviour checks on one replication's results (`report`, from `model`), as warnings that
/// each concern one replication (`replications` and `of` 1).
pub fn check_report(model: &Model, report: &Report) -> Vec<Diagnostic> {
    let mut found: Vec<(Option<ElementId>, Behaviour)> = Vec::new();
    let window = report.measured_to - report.measured_from;
    for e in report.elements.iter().filter(|e| e.kind != "resource") {
        let Some(element) = model.element(&e.id) else {
            continue;
        };
        let kind = &element.kind;
        if let Some(b) = unbalanced(kind, e) {
            found.push((Some(e.id.clone()), b));
        }
        if let Some(b) = littles_law(e, window) {
            found.push((Some(e.id.clone()), b));
        }
        if let ElementKind::Server { capacity, .. } = kind {
            let units = f64::from(kind.most_units().unwrap_or(*capacity));
            if let Some(b) = busy_not_working(e, *capacity, units) {
                found.push((Some(e.id.clone()), b));
            }
            if let Some(b) = states_total(e) {
                found.push((Some(e.id.clone()), b));
            }
        }
        for b in out_of_range(e) {
            found.push((Some(e.id.clone()), b));
        }
    }
    // Pools report after the elements, by their own ids.
    for e in report.elements.iter().filter(|e| e.kind == "resource") {
        for b in out_of_range(e) {
            found.push((Some(e.id.clone()), b));
        }
    }
    if let Some(b) = unaccounted(model, report) {
        found.push((None, b));
    }
    if let Some(b) = nothing_out(model, report) {
        found.push((None, b));
    }
    found
        .into_iter()
        .map(|(at, found)| Diagnostic {
            severity: Severity::Warning,
            at,
            problem: Problem::Behaviour {
                found,
                replications: 1,
                of: 1,
            },
        })
        .collect()
}

/// The behaviour checks on every replication's results (`reports`, from `model`): one warning
/// per element and check that failed in any of them, saying in how many, with what it found
/// where it was furthest off.
pub fn check_reports(model: &Model, reports: &[Report]) -> Vec<Diagnostic> {
    // In the order first found, so the warnings come in model order.
    let mut merged: Vec<Diagnostic> = Vec::new();
    let mut index: BTreeMap<(Option<ElementId>, &'static str, Option<&'static str>), usize> =
        BTreeMap::new();
    let of = reports.len() as u64;
    for r in reports {
        for d in check_report(model, r) {
            let Problem::Behaviour { found, .. } = d.problem else {
                continue;
            };
            let metric = match &found {
                Behaviour::OutOfRange { metric, .. } => Some(*metric),
                _ => None,
            };
            let key = (d.at.clone(), found.code(), metric);
            match index.get(&key).and_then(|&i| merged.get_mut(i)) {
                Some(Diagnostic {
                    problem:
                        Problem::Behaviour {
                            found: worst,
                            replications,
                            ..
                        },
                    ..
                }) => {
                    *replications += 1;
                    if found.gap() > worst.gap() {
                        *worst = found;
                    }
                }
                _ => {
                    index.insert(key, merged.len());
                    merged.push(Diagnostic {
                        severity: Severity::Warning,
                        at: d.at,
                        problem: Problem::Behaviour {
                            found,
                            replications: 1,
                            of,
                        },
                    });
                }
            }
        }
    }
    merged
}

/// Whether element `kind` is a shortest-queue gateway, whose entities wait for room on their
/// way through it without being counted inside it (the element sending them holds them).
fn shortest_queue(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::ExclusiveGateway {
            routing: Routing::ShortestQueue,
            ..
        }
    )
}

/// A server with patience until done: an entity that gives up in service counts as abandoned
/// at once but stays inside until its unit is done with it, so its counts can be off by up to
/// one such entity per unit, at the start and at the end.
fn gives_up_in_service(kind: &ElementKind) -> Option<u64> {
    match kind {
        ElementKind::Server {
            patience: Some(p), ..
        } if p.until == PatienceUntil::Done => kind.most_units().map(u64::from),
        _ => None,
    }
}

/// Every element balances: what came in, or was inside as the statistics started, left, was
/// lost, gave up or is still inside. Not checked where entities are copied, combined or split
/// (parallel gateways, unbatches), nor at shortest-queue gateways (see [`shortest_queue`]).
fn unbalanced(kind: &ElementKind, e: &ElementReport) -> Option<Behaviour> {
    let came = e.entered + e.inside_at_start;
    let went = match kind {
        // Sources hold nothing: each arrival created leaves or is lost.
        ElementKind::Source { .. } => e.left + e.lost.unwrap_or(0),
        // A server's lost and balked arrivals never came in.
        ElementKind::Server { .. } => e.left + e.abandoned.unwrap_or(0) + e.inside_at_end,
        ElementKind::SharedTask { .. } | ElementKind::Delay { .. } | ElementKind::Sink { .. } => {
            e.left + e.inside_at_end
        }
        ElementKind::ExclusiveGateway { .. } if !shortest_queue(kind) => {
            e.left + e.lost.unwrap_or(0) + e.inside_at_end
        }
        // Each batch that leaves carries `size` entities.
        ElementKind::Batch { size } => e.left * u64::from(*size).max(1) + e.inside_at_end,
        _ => return None,
    };
    let slack = gives_up_in_service(kind).unwrap_or(0);
    (came.abs_diff(went) > slack).then_some(Behaviour::Unbalanced { came, went })
}

/// The model balances: the entities created, or inside anywhere as the statistics started,
/// reached an exit, left the model elsewhere, or are still inside. Where entities leave the
/// model elsewhere: an exclusive gateway's lost (no way to take); a source's arrivals turned
/// away at a full buffer with no way out for them (through a shortest-queue gateway, counted by
/// the gateway instead); and those giving up or balking at a server with no way marked for them.
/// Not checked where entities are copied, combined or split (parallel gateways, batches,
/// unbatches), nor where a shortest-queue gateway sends to another (whose entities wait for room
/// counted nowhere).
fn unaccounted(model: &Model, report: &Report) -> Option<Behaviour> {
    let kinds = || model.elements.iter().map(|e| &e.kind);
    if kinds().any(|k| {
        matches!(
            k,
            ElementKind::ParallelGateway | ElementKind::Batch { .. } | ElementKind::Unbatch
        )
    }) {
        return None;
    }
    let target = |from: &ElementId| {
        model
            .connections
            .iter()
            .find(|c| &c.from == from)
            .and_then(|c| model.element(&c.to))
    };
    let chained = model.elements.iter().any(|e| {
        shortest_queue(&e.kind)
            && model
                .connections
                .iter()
                .filter(|c| c.from == e.id)
                .any(|c| {
                    model
                        .element(&c.to)
                        .is_some_and(|t| shortest_queue(&t.kind))
                })
    });
    if chained {
        return None;
    }
    // Whether element `id` has a way marked by `marked` (abandoned, balked, turned away).
    let marked = |id: &ElementId, way: fn(&stochcraft_model::Connection) -> bool| {
        model.connections.iter().any(|c| &c.from == id && way(c))
    };
    let (mut came, mut went) = (0u64, 0u64);
    let mut slack = 0u64;
    for e in report.elements.iter().filter(|e| e.kind != "resource") {
        let Some(element) = model.element(&e.id) else {
            continue;
        };
        came += e.inside_at_start;
        went += e.inside_at_end;
        match &element.kind {
            ElementKind::Source { .. } => {
                came += e.entered;
                // Turned away where it sends: gone, unless that buffer has a way out for those
                // (then they take it) or it is a shortest-queue gateway (which counts them).
                let kept = target(&e.id)
                    .is_some_and(|t| shortest_queue(&t.kind) || marked(&t.id, |c| c.turned_away));
                if !kept {
                    went += e.lost.unwrap_or(0);
                }
            }
            ElementKind::Sink { .. } => went += e.left,
            ElementKind::ExclusiveGateway { .. } => went += e.lost.unwrap_or(0),
            kind @ ElementKind::Server { .. } => {
                if !marked(&e.id, |c| c.abandoned) {
                    went += e.abandoned.unwrap_or(0);
                }
                if !marked(&e.id, |c| c.balked) {
                    went += e.balked.unwrap_or(0);
                }
                slack += gives_up_in_service(kind).unwrap_or(0);
            }
            _ => {}
        }
    }
    (came.abs_diff(went) > slack).then_some(Behaviour::Unaccounted { came, went })
}

/// Some entity reaches an exit: checked when the model has an exit and some entity was
/// created, or inside as the statistics started.
fn nothing_out(model: &Model, report: &Report) -> Option<Behaviour> {
    let (mut came, mut out, mut exits) = (0u64, 0u64, false);
    for e in &report.elements {
        match model.element(&e.id).map(|x| &x.kind) {
            Some(ElementKind::Source { .. }) => came += e.entered,
            Some(ElementKind::Sink { .. }) => {
                exits = true;
                out += e.entered;
            }
            _ => {}
        }
        if e.kind != "resource" {
            came += e.inside_at_start;
        }
    }
    (exits && came > 0 && out == 0).then_some(Behaviour::NothingOut { came })
}

/// Little's law, over the run's own window: the time-average number inside times the window is
/// the time the entities spent inside in it, counted from when each came in and went out (from
/// the window's start for those inside then, to its end for those still inside). Exact over any
/// window, so the tolerance is for rounding only ([`LITTLE_TOLERANCE`]).
fn littles_law(e: &ElementReport, window: f64) -> Option<Behaviour> {
    let (Some(mean), Some(stays)) = (e.mean_work_in_progress, e.time_inside) else {
        return None;
    };
    if window <= 0.0 {
        return None;
    }
    let most = (e.entered + e.inside_at_start + e.inside_at_end) as f64 * window;
    let off = (mean * window - stays).abs();
    (off > LITTLE_TOLERANCE * most.max(window)).then(|| Behaviour::LittlesLaw {
        mean,
        from_stays: stays / window,
    })
}

/// A server counts busy (in its utilisation) the units with a job under way: each is
/// processing, setting up, or failed or out restarting part-way through it. So the units busy
/// lie between those processing or setting up and those in any of these four states; any more
/// means units idle, blocked or off shift (on a break) counted busy. `capacity` is what a server
/// without shifts has its utilisation over, `units` what its shares of time are over.
fn busy_not_working(e: &ElementReport, capacity: u32, units: f64) -> Option<Behaviour> {
    let (Some(u), Some(processing)) = (e.utilisation, e.time_processing) else {
        return None;
    };
    // With shifts or a calendar, utilisation is over the units on shift.
    let busy = u * e.on_shift.unwrap_or(f64::from(capacity));
    let share = |x: Option<f64>| x.unwrap_or(0.0);
    let working = units * (processing + share(e.time_setup));
    let could = working + units * (share(e.time_failed) + share(e.time_restarting));
    let tol = SHARE_TOLERANCE * units.max(1.0);
    (busy < working - tol || busy > could + tol).then_some(Behaviour::BusyNotWorking {
        busy,
        working,
        could,
    })
}

/// Each of a server's units is in one state at a time, so the shares of time it reports add up
/// to 1. A state it does not report cannot happen to it.
fn states_total(e: &ElementReport) -> Option<Behaviour> {
    let processing = e.time_processing?;
    let total: f64 = processing
        + [
            e.time_starved,
            e.time_blocked,
            e.time_failed,
            e.time_off_shift,
            e.time_setup,
            e.time_restarting,
        ]
        .into_iter()
        .flatten()
        .sum::<f64>();
    ((total - 1.0).abs() > SHARE_TOLERANCE).then_some(Behaviour::StatesTotal { total })
}

/// Shares of time stay within 0 to 1. (Not the OEE effectiveness, the OEE, or the demand met,
/// which can exceed 1; nor shares of counts, whose parts can fall either side of the warm-up.)
fn out_of_range(e: &ElementReport) -> Vec<Behaviour> {
    [
        ("utilisation", e.utilisation),
        ("availability", e.availability),
        ("blocking_probability", e.blocking_probability),
        ("time_off_shift", e.time_off_shift),
        ("time_processing", e.time_processing),
        ("time_starved", e.time_starved),
        ("time_blocked", e.time_blocked),
        ("time_failed", e.time_failed),
        ("time_restarting", e.time_restarting),
        ("time_setup", e.time_setup),
        ("oee_availability", e.oee_availability),
        ("setup_ratio", e.setup_ratio),
    ]
    .into_iter()
    .filter_map(|(metric, v)| {
        let value = v?;
        (!(-SHARE_TOLERANCE..=1.0 + SHARE_TOLERANCE).contains(&value))
            .then_some(Behaviour::OutOfRange { metric, value })
    })
    .collect()
}
