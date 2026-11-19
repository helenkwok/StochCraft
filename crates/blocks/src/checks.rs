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
//! - Some entity leaves the model, at an exit or by any other way out ([`Behaviour::NothingOut`]).
//! - Little's law holds at every server, task with shared resources and delay
//!   ([`Behaviour::LittlesLaw`]).
//! - A server counts busy exactly the units processing or setting up: none idle, blocked, off
//!   shift, failed or restarting ([`Behaviour::BusyNotWorking`]), and its units' shares of time in each state add up to 1
//!   ([`Behaviour::StatesTotal`]).
//! - Utilisation and the other shares of time stay within 0 to 1 ([`Behaviour::OutOfRange`]).

use std::collections::BTreeMap;

use stochcraft_model::{
    Behaviour, Diagnostic, ElementId, ElementKind, Model, Problem, Routing, Severity,
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
            routing: Routing::ShortestQueue | Routing::Divert,
            ..
        }
    )
}

/// The entities inside element `e` at the start and the end, less those that had given up
/// during their job while their unit carried on with it (patience until done): each of those
/// is counted once, as abandoned when it gave up (a stand-in taking its way out).
fn inside(e: &ElementReport) -> (u64, u64) {
    let [gone_start, gone_end] = e.given_up_inside;
    (
        e.inside_at_start.saturating_sub(gone_start),
        e.inside_at_end.saturating_sub(gone_end),
    )
}

/// Every element balances: what came in, or was inside as the statistics started, left, was
/// lost, gave up or is still inside. Not checked where entities are copied, combined or split
/// (parallel gateways, unbatches), whose counts in and out are of different things, nor at
/// shortest-queue gateways (see [`shortest_queue`]); the model's count covers them.
fn unbalanced(kind: &ElementKind, e: &ElementReport) -> Option<Behaviour> {
    let (inside_at_start, inside_at_end) = inside(e);
    let came = e.entered + inside_at_start;
    let went = match kind {
        // Sources hold nothing: each arrival created leaves or is lost.
        ElementKind::Source { .. } => e.left + e.lost.unwrap_or(0),
        // A server's lost and balked arrivals never came in.
        ElementKind::Server { .. } => e.left + e.abandoned.unwrap_or(0) + inside_at_end,
        ElementKind::SharedTask { .. } | ElementKind::Delay { .. } | ElementKind::Sink { .. } => {
            e.left + inside_at_end
        }
        ElementKind::ExclusiveGateway { .. } if !shortest_queue(kind) => {
            e.left + e.lost.unwrap_or(0) + inside_at_end
        }
        // Each batch that leaves carries `size` entities.
        ElementKind::Batch { size } => e.left * u64::from(*size).max(1) + inside_at_end,
        _ => return None,
    };
    (came != went).then_some(Behaviour::Unbalanced { came, went })
}

/// The model's entities: those created at a source or inside anywhere as the statistics started
/// (`came`), those that left the model (`out`) and those inside anywhere at the end (`inside`);
/// those started and ended where entities are copied, combined or split (`made`, `ended`); and
/// whether every entity is counted in one place only, so that `came + made` must equal
/// `out + ended + inside` ([`unaccounted`]). `present`: the entities created at a source or
/// inside anywhere as the statistics started, those where entities are copied, combined or
/// split included ([`nothing_out`]).
struct Flows {
    came: u64,
    present: u64,
    made: u64,
    out: u64,
    ended: u64,
    inside: u64,
    exact: bool,
}

/// Counts the model's entities ([`Flows`]). Each entity is in one place: an element, or on its
/// way out of one by a way marked for those giving up, balking or turned away while it waits
/// for room there ([`ElementReport::leaving`]). Entities leave the model at an exit; at an
/// exclusive gateway with no way to take (its lost); turned away from a source's full buffer
/// with no way out for them (through a shortest-queue gateway, counted by the gateway instead);
/// and giving up or balking at a server with no way marked for them. A parallel gateway that
/// copies, a batch and an unbatch end the entities that come in and start those that leave,
/// as new ones. Not exact (the counts are of different things) at a parallel gateway that
/// joins, whose second and later inputs are not counted as entering, nor where a
/// shortest-queue gateway sends to another, holding entities that wait for room on the way.
fn flows(model: &Model, report: &Report) -> Flows {
    let into = |id: &ElementId| model.connections.iter().filter(|c| &c.to == id).count();
    let target = |from: &ElementId| {
        model
            .connections
            .iter()
            .find(|c| &c.from == from)
            .and_then(|c| model.element(&c.to))
    };
    // Whether element `id` has a way marked by `marked` (abandoned, balked, turned away).
    let marked = |id: &ElementId, way: fn(&stochcraft_model::Connection) -> bool| {
        model.connections.iter().any(|c| &c.from == id && way(c))
    };
    let sq = |id: &ElementId| model.element(id).is_some_and(|e| shortest_queue(&e.kind));
    let mut f = Flows {
        came: 0,
        present: 0,
        made: 0,
        out: 0,
        ended: 0,
        inside: 0,
        exact: !model.connections.iter().any(|c| sq(&c.from) && sq(&c.to)),
    };
    for e in report.elements.iter().filter(|e| e.kind != "resource") {
        let Some(element) = model.element(&e.id) else {
            continue;
        };
        let [leaving_at_start, leaving_at_end] = e.leaving;
        f.came += leaving_at_start;
        f.inside += leaving_at_end;
        f.present += leaving_at_start + inside(e).0;
        match &element.kind {
            // What comes in ends here, and what leaves starts here: only the counts across
            // the element's edges matter, not those inside it.
            ElementKind::Batch { .. } | ElementKind::Unbatch | ElementKind::ParallelGateway => {
                f.ended += e.entered;
                f.made += e.left;
                if matches!(element.kind, ElementKind::ParallelGateway) && into(&e.id) > 1 {
                    f.exact = false;
                }
                continue;
            }
            ElementKind::Source { .. } => {
                f.came += e.entered;
                f.present += e.entered;
                // Turned away where it sends: gone, unless that buffer has a way out for those
                // (then they take it) or it is a shortest-queue gateway (which counts them).
                let kept = target(&e.id)
                    .is_some_and(|t| shortest_queue(&t.kind) || marked(&t.id, |c| c.turned_away));
                if !kept {
                    f.out += e.lost.unwrap_or(0);
                }
            }
            ElementKind::Sink { .. } => f.out += e.left,
            ElementKind::ExclusiveGateway { .. } => f.out += e.lost.unwrap_or(0),
            ElementKind::Server { .. } => {
                if !marked(&e.id, |c| c.abandoned) {
                    f.out += e.abandoned.unwrap_or(0);
                }
                if !marked(&e.id, |c| c.balked) {
                    f.out += e.balked.unwrap_or(0);
                }
            }
            _ => {}
        }
        let (inside_at_start, inside_at_end) = inside(e);
        f.came += inside_at_start;
        f.inside += inside_at_end;
    }
    f
}

/// The model balances: the entities created, or inside anywhere as the statistics started,
/// left the model or are still inside ([`flows`], which says where it cannot be checked).
fn unaccounted(model: &Model, report: &Report) -> Option<Behaviour> {
    let f = flows(model, report);
    let (came, went) = (f.came + f.made, f.out + f.ended + f.inside);
    (f.exact && came != went).then_some(Behaviour::Unaccounted { came, went })
}

/// Some entity left the model, by any way out ([`flows`]): an exit, or giving up, balking,
/// being turned away or finding no way at a gateway with no way marked for those. Checked when
/// some entity was created, or inside anywhere as the statistics started (a batch filling
/// included). An entity that leaves by any
/// way has got through the model, as at any exit; a gateway with no way for some is warned
/// about as the model is checked ([`stochcraft_model::Problem::NoDefault`]).
fn nothing_out(model: &Model, report: &Report) -> Option<Behaviour> {
    let f = flows(model, report);
    (f.present > 0 && f.out == 0).then_some(Behaviour::NothingOut { came: f.present })
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

/// A server counts busy (in its utilisation) exactly the units processing or setting up: not
/// those idle, blocked or off shift (on a break), nor those failed or restarting part-way through
/// a job, whose time is capacity lost. `capacity` is what a server without shifts has its
/// utilisation over, `units` what its shares of time are over.
fn busy_not_working(e: &ElementReport, capacity: u32, units: f64) -> Option<Behaviour> {
    let (Some(u), Some(processing)) = (e.utilisation, e.time_processing) else {
        return None;
    };
    // With shifts or a calendar, utilisation is over the units on shift.
    let busy = u * e.on_shift.unwrap_or(f64::from(capacity));
    let working = units * (processing + e.time_setup.unwrap_or(0.0));
    let tol = SHARE_TOLERANCE * units.max(1.0);
    ((busy - working).abs() > tol).then_some(Behaviour::BusyNotWorking { busy, working })
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
