//! DEVStone LI, HI and HO (Wainer, Glinsky and Gutierrez-Alcaraz, Simulation 87(7), 2011), as
//! described in Risco-Martín, Mittal, Fabero Jiménez, Zapater and Hermida Correa,
//! "Reconsidering the performance of DEVS modeling and simulation environments using the
//! DEVStone benchmark" (arXiv:2402.05483, 2024): the atomic model of its Algorithm 1, the
//! layouts of its Figures 1 to 3, and its transition and event counts (Eqs. 7 to 17), which match
//! those of Herguedas-Pinedo et al. (ANNSIM'25, Eqs. 3 to 5). Times are aggregated as in
//! Herguedas-Pinedo et al., Eq. 14. Built from the papers' descriptions alone.
//!
//! A model of width w and depth d has d − 1 levels, each a coupled model holding the next level
//! and w − 1 atomic models, and an innermost level holding one atomic model. An atomic model is
//! passive until input arrives; then (δext) it appends the events to its list and becomes active
//! with σ = 0; at σ (λ, then δint) it sends its whole list and becomes passive again. Input that
//! arrives as it is about to send (a collision) is handled by δcon = δext after δint. There is
//! no CPU delay in the transitions (Δint = Δext = 0). One event is injected at time 0 into every
//! input port of the top model; the run ends when no event is left.
//!
//! - LI: each level's input feeds the next level and every atomic model at that level.
//! - HI: as LI, and atomic model i's output feeds atomic model i + 1 (a shift register).
//! - HO: as HI, with two input and two output ports per level: the second input feeds the
//!   level's atomic models, the first the next level's two inputs, and every atomic model's
//!   output goes to its level's second output.
//!
//! Here the couplings are flattened when the model is built: each atomic model is a `des`
//! process with a store as its input port, and sends straight to the atomic models (or top
//! output) its output reaches. A process engine has no coupled-model protocol to pay for, so HO
//! costs what HI costs plus delivering the outputs that reach the top model's second output.
//! Each run is checked against the closed forms: δint = δext = events = 1 + (d − 1)(w − 1) for
//! LI and 1 + (d − 1)·w(w − 1)/2 for HI and HO.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use stochcraft_des::{Sim, Store};
use stochcraft_random::math::{exp, ln};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Li,
    Hi,
    Ho,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Li, Kind::Hi, Kind::Ho];

    fn name(self) -> &'static str {
        match self {
            Kind::Li => "LI",
            Kind::Hi => "HI",
            Kind::Ho => "HO",
        }
    }

    /// The paper's closed form for δint, δext and events alike, for one injected event.
    pub fn transitions(self, w: u64, d: u64) -> u64 {
        match self {
            Kind::Li => 1 + (d - 1) * (w - 1),
            Kind::Hi | Kind::Ho => 1 + (d - 1) * w * (w - 1) / 2,
        }
    }

    /// Events that reach the top model's outputs: the innermost atomic model's one event, and
    /// for HO every event the top level's atomic models send.
    fn outputs(self, w: u64, d: u64) -> u64 {
        match self {
            Kind::Ho if d > 1 => 1 + w * (w - 1) / 2,
            _ => 1,
        }
    }

    /// The order of complexity used as the weight in the aggregate (Herguedas-Pinedo et al.,
    /// Eqs. 8 to 10): w·d for LI, w²·d for HI and HO.
    fn complexity(self, w: u64, d: u64) -> f64 {
        match self {
            Kind::Li => (w * d) as f64,
            Kind::Hi | Kind::Ho => (w * w * d) as f64,
        }
    }
}

/// What a run did: δint and δext transitions, events received by atomic models, and events
/// that reached the top model's outputs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub internal: u64,
    pub external: u64,
    pub events: u64,
    pub outputs: u64,
}

#[derive(Default)]
struct Tally {
    internal: Cell<u64>,
    external: Cell<u64>,
    events: Cell<u64>,
    outputs: Cell<u64>,
}

fn bump(c: &Cell<u64>, by: u64) {
    c.set(c.get() + by);
}

/// Takes every event waiting at an input port: the rest of the bag.
fn drain(port: &Store<u64>) -> u64 {
    let mut n = 0;
    while let Some(k) = port.take_where(|_| true) {
        n += k;
    }
    n
}

/// Builds and runs one model of width `w` (at least 2) and depth `d` (at least 1).
pub fn simulate(kind: Kind, w: u64, d: u64) -> Counts {
    assert!(w >= 2 && d >= 1, "DEVStone needs w ≥ 2 and d ≥ 1");
    let per_level = (w - 1) as usize;
    let regular = (d - 1) as usize * per_level;
    let atomics = regular + 1;
    let mut sim = Sim::new();
    let ctx = sim.ctx();
    let ports: Rc<Vec<Store<u64>>> = Rc::new((0..atomics).map(|_| Store::new(&ctx)).collect());
    let counts = Rc::new(Tally::default());
    for i in 0..atomics {
        let innermost = i == regular;
        let k = i % per_level;
        // Where this atomic model's output goes.
        let next = (!innermost && kind != Kind::Li && k + 1 < per_level).then_some(i + 1);
        let to_top = innermost || (kind == Kind::Ho && i < per_level);
        let (c, p, n) = (ctx.clone(), Rc::clone(&ports), Rc::clone(&counts));
        sim.spawn(async move {
            loop {
                // Passive until input arrives: δext.
                let mut list = p[i].get().await + drain(&p[i]);
                bump(&n.external, 1);
                bump(&n.events, list);
                loop {
                    // Active with σ = 0: λ sends the whole list, then δint empties it.
                    c.hold(0.0).await;
                    if let Some(j) = next {
                        p[j].put(list);
                    }
                    if to_top {
                        bump(&n.outputs, list);
                    }
                    bump(&n.internal, 1);
                    // Input that arrived for this instant: δcon, δext after δint.
                    let bag = drain(&p[i]);
                    if bag == 0 {
                        break;
                    }
                    bump(&n.external, 1);
                    bump(&n.events, bag);
                    list = bag;
                }
            }
        });
    }
    // The generator: one event into every input port of the top model at time 0, which the
    // couplings take to every atomic model once.
    let p = Rc::clone(&ports);
    sim.spawn(async move {
        for port in p.iter() {
            port.put(1);
        }
    });
    sim.run();
    Counts {
        internal: counts.internal.get(),
        external: counts.external.get(),
        events: counts.events.get(),
        outputs: counts.outputs.get(),
    }
}

/// Whether the counts match the closed forms.
pub fn check(kind: Kind, w: u64, d: u64, c: &Counts) -> bool {
    let t = kind.transitions(w, d);
    *c == Counts {
        internal: t,
        external: t,
        events: t,
        outputs: kind.outputs(w, d),
    }
}

/// Runs LI, HI and HO for every width and depth in `step, 2·step, …, max`, prints each run
/// and the aggregate per type and over the three. Returns whether every run passed its check.
pub fn run(max: u64, step: u64) -> bool {
    let step = step.max(2);
    let sizes: Vec<u64> = (1..).map(|i| i * step).take_while(|s| *s <= max).collect();
    println!(
        "DEVStone LI, HI and HO, no CPU delay, w and d in {sizes:?}; times are build, run and tear-down; a pass is δint = δext = events = the closed form"
    );
    let mut ok = true;
    let mut per_type = Vec::new();
    for kind in Kind::ALL {
        let (mut weighted, mut weights) = (0.0, 0.0);
        let (mut transitions, mut seconds) = (0u64, 0.0);
        for &w in &sizes {
            for &d in &sizes {
                let started = Instant::now();
                let c = simulate(kind, w, d);
                let secs = started.elapsed().as_secs_f64();
                let pass = check(kind, w, d, &c);
                ok &= pass;
                let t = c.internal + c.external;
                transitions += t;
                seconds += secs;
                let weight = kind.complexity(w, d);
                weighted += weight * ln(1.0 + secs);
                weights += weight;
                println!(
                    "{:<2} w = {w:>4} d = {d:>4}  {:>11} events  {secs:>8.3} s  {:>6.2} M transitions/s  {}",
                    kind.name(),
                    c.events,
                    t as f64 / secs / 1e6,
                    if pass { "ok" } else { "CHECK FAILED" }
                );
            }
        }
        // Herguedas-Pinedo et al., Eq. 14: exp(Σ C·ln(1 + t) / Σ C).
        let aggregate = exp(weighted / weights);
        println!(
            "{:<2} aggregate t̂ = {aggregate:.4} s over {} models  ({:.2} M transitions/s overall)",
            kind.name(),
            sizes.len() * sizes.len(),
            transitions as f64 / seconds / 1e6
        );
        per_type.push(aggregate);
    }
    // Eq. 16: the geometric mean over the types.
    let overall = per_type.iter().map(|t| ln(*t)).sum::<f64>() / per_type.len() as f64;
    println!("LI, HI, HO  aggregate t̂ = {:.4} s", exp(overall));
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_match_the_closed_forms() {
        for kind in Kind::ALL {
            for (w, d) in [(2, 1), (2, 2), (3, 1), (3, 4), (7, 5), (20, 3), (31, 17)] {
                let c = simulate(kind, w, d);
                assert!(
                    check(kind, w, d, &c),
                    "{kind:?} w = {w} d = {d}: {c:?}, expected {} each and {} outputs",
                    kind.transitions(w, d),
                    kind.outputs(w, d)
                );
            }
        }
    }
}
