//! A four-machine serial line with finite buffers and blocking after service, against exact
//! targets from its continuous-time Markov chain.
//!
//! The line is task T1.2 of Pernas-Álvarez and Crespo-Pereira, "Agentic discrete-event
//! simulation for manufacturing system design", arXiv:2611.10602v2, 2026: M1 → B1 → M2 → B2 →
//! M3 → B3 → M4 → exit, raw material always waiting in front of M1, buffers of 3 parts, and
//! exponential processing times with means 3.3, 4.3, 3.0 and 3.7 minutes. The paper gives no
//! reference values; the targets here are exact, from the stationary distribution of the line's
//! Markov chain, solved in this file.
//!
//! The chain is built for any number of machines and any buffer size, and is checked on its own
//! before the simulation is compared with it: on two machines it must give the closed form of
//! `buffers.rs`, a line run backwards must have the same throughput (the reversibility of
//! exponential lines with blocking after service), and every machine must pass on parts at the
//! line's throughput.
#![allow(clippy::panic)]

use std::collections::HashMap;

use stochcraft_blocks::{Builder, ExperimentSummary, Metric, Report, compile, summarise};
use stochcraft_model::{Dist, Experiment, Model};
use stochcraft_random::Rng;

/// Mean processing times of M1 to M4, in minutes.
const MEANS: [f64; 4] = [3.3, 4.3, 3.0, 3.7];
/// Places in each of B1, B2 and B3.
const BUFFER: u32 = 3;

const WORKING: u8 = 0;
const BLOCKED: u8 = 1;
const STARVED: u8 = 2;

/// A state of the line: what each machine is doing and how many parts wait in each buffer.
///
/// `status[i]` is machine i's state: working on a part, blocked (holding a finished part it
/// cannot pass on because the next buffer is full and the next machine busy) or starved (no
/// part). The first machine is never starved. `level[i]` is the number of parts waiting in the
/// buffer after machine i (buffer i + 1 in the paper's names), 0..=`BUFFER`; a part being worked
/// on or held blocked is at its machine, not in a buffer.
///
/// This is the physical state itself rather than the compact "parts in each stage" encoding of
/// the two-machine line: with more than two machines a blocked machine's part belongs to the
/// next stage, which shifts where the machine before it blocks, and the explicit state avoids
/// getting that wrong. Not every combination can happen (a starved machine has an empty buffer
/// in front of it, a blocked machine a full one after it and a busy machine after that), so the
/// states are found by following the transitions from an empty line.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct State {
    status: Vec<u8>,
    level: Vec<u32>,
}

/// Machine `j` is free: it takes the next part from the buffer in front of it, or straight from
/// the machine before when that buffer has no places, and a blocked machine before it can then
/// pass on its part and take the next one in turn. The first machine always finds raw material.
fn pull(s: &mut State, j: usize) {
    if j == 0 {
        s.status[0] = WORKING;
        return;
    }
    let up = j - 1;
    if s.level[up] > 0 {
        s.level[up] -= 1;
        s.status[j] = WORKING;
        if s.status[up] == BLOCKED {
            s.level[up] += 1;
            pull(s, up);
        }
    } else if s.status[up] == BLOCKED {
        s.status[j] = WORKING;
        pull(s, up);
    } else {
        s.status[j] = STARVED;
    }
}

/// Machine `j` finishes its part: the last machine sends it out of the line; any other passes it
/// to a starved next machine, else into the buffer if there is room, else holds it, blocked.
/// Once the part is passed on, machine `j` takes its next part.
fn finish(s: &State, j: usize, buffer: u32) -> State {
    let mut s = s.clone();
    let last = s.status.len() - 1;
    if j < last {
        if s.status[j + 1] == STARVED {
            s.status[j + 1] = WORKING;
        } else if s.level[j] < buffer {
            s.level[j] += 1;
        } else {
            s.status[j] = BLOCKED;
            return s;
        }
    }
    pull(&mut s, j);
    s
}

/// The line's long-run behaviour, exact from its Markov chain.
#[derive(Debug)]
struct Exact {
    /// Parts out of the last machine per unit of time.
    throughput: f64,
    /// For each machine, the long-run fraction of time working, blocked and starved.
    working: Vec<f64>,
    blocked: Vec<f64>,
    starved: Vec<f64>,
    states: usize,
}

/// Solves the chain of a line of exponential machines with the given mean processing times and
/// `buffer` places between each pair: finds every reachable state, builds the generator Q (one
/// transition per working machine, at rate 1 / mean), and solves π Q = 0 with Σ π = 1 by
/// Gaussian elimination with partial pivoting.
fn exact(means: &[f64], buffer: u32) -> Exact {
    let m = means.len();
    let start = {
        let mut s = State {
            status: vec![STARVED; m],
            level: vec![0; m - 1],
        };
        s.status[0] = WORKING;
        s
    };
    let mut index: HashMap<State, usize> = HashMap::from([(start.clone(), 0)]);
    let mut states = vec![start];
    let mut transitions: Vec<(usize, usize, f64)> = Vec::new();
    let mut next = 0;
    while next < states.len() {
        let s = states[next].clone();
        for (j, mean) in means.iter().enumerate() {
            if s.status[j] != WORKING {
                continue;
            }
            let t = finish(&s, j, buffer);
            let to = match index.get(&t) {
                Some(&k) => k,
                None => {
                    index.insert(t.clone(), states.len());
                    states.push(t);
                    states.len() - 1
                }
            };
            transitions.push((next, to, 1.0 / mean));
        }
        next += 1;
    }

    let n = states.len();
    // Rows of A are the balance equations (Q transposed), the last replaced by Σ π = 1.
    let mut a = vec![vec![0.0; n + 1]; n];
    for &(from, to, rate) in &transitions {
        a[to][from] += rate;
        a[from][from] -= rate;
    }
    a[n - 1] = vec![1.0; n + 1];
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))
            .unwrap_or(col);
        a.swap(col, pivot);
        let pivot_row = a[col].clone();
        let p = pivot_row[col];
        assert!(p.abs() > 1e-300, "singular at column {col}");
        for (i, row) in a.iter_mut().enumerate() {
            if i != col && row[col] != 0.0 {
                let f = row[col] / p;
                for (x, y) in row.iter_mut().zip(&pivot_row).skip(col) {
                    *x -= f * y;
                }
            }
        }
    }
    let pi: Vec<f64> = (0..n).map(|i| a[i][n] / a[i][i]).collect();

    // The solution is a distribution and balances every state's flow in and out.
    assert!(pi.iter().all(|&p| p >= -1e-15), "negative probability");
    assert!((pi.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    let mut flow = vec![0.0; n];
    for &(from, to, rate) in &transitions {
        flow[to] += pi[from] * rate;
        flow[from] -= pi[from] * rate;
    }
    let residual = flow.iter().fold(0.0_f64, |r, f| r.max(f.abs()));
    assert!(residual < 1e-13, "residual {residual}");

    let share = |i: usize, what: u8| -> f64 {
        states
            .iter()
            .zip(&pi)
            .filter(|(s, _)| s.status[i] == what)
            .map(|(_, p)| p)
            .sum()
    };
    let working: Vec<f64> = (0..m).map(|i| share(i, WORKING)).collect();
    let throughput = working[m - 1] / means[m - 1];
    Exact {
        throughput,
        blocked: (0..m).map(|i| share(i, BLOCKED)).collect(),
        starved: (0..m).map(|i| share(i, STARVED)).collect(),
        working,
        states: n,
    }
}

/// Every part passes every machine, so each machine's working share over its mean time is the
/// line's throughput (flow conservation), and each machine's shares add up to one.
fn assert_conserves(e: &Exact, means: &[f64]) {
    for (i, mean) in means.iter().enumerate() {
        let rate = e.working[i] / mean;
        assert!(
            (rate - e.throughput).abs() < 1e-12 * e.throughput.max(1.0),
            "machine {i}: {rate} parts per unit of time vs throughput {}",
            e.throughput
        );
        let total = e.working[i] + e.blocked[i] + e.starved[i];
        assert!(
            (total - 1.0).abs() < 1e-12,
            "machine {i}: shares sum to {total}"
        );
    }
    assert!(e.starved[0] == 0.0, "the first machine starved");
    assert!(
        e.blocked[means.len() - 1] == 0.0,
        "the last machine blocked"
    );
}

#[test]
fn the_chain_gives_the_two_machine_closed_form() {
    // As `two_machine_line` in buffers.rs: rates 1 and 1.25 with 2 places between them is
    // M/M/1/4 in the parts past the first machine, with p_n ∝ 0.8^n on 0..=4.
    let e = exact(&[1.0, 0.8], 2);
    assert_conserves(&e, &[1.0, 0.8]);
    let rho: f64 = 0.8;
    let rho4 = rho * rho * rho * rho;
    let p0 = (1.0 - rho) / (1.0 - rho4 * rho);
    let throughput = 1.25 * (1.0 - p0);
    let blocked = p0 * rho4;
    assert!((e.throughput - throughput).abs() < 1e-14, "{e:?}");
    assert!((e.blocked[0] - blocked).abs() < 1e-14, "{e:?}");
    assert!((e.starved[1] - p0).abs() < 1e-14, "{e:?}");
    assert_eq!(e.states, 5);

    // With no places at all, each pair still blocks: rates 1 and 1, two states of the second
    // machine busy (first working or blocked) and one with it starved, each 1/3.
    let e = exact(&[1.0, 1.0], 0);
    assert_conserves(&e, &[1.0, 1.0]);
    assert!((e.throughput - 2.0 / 3.0).abs() < 1e-14, "{e:?}");
}

#[test]
fn the_chain_of_the_paper_line_is_consistent() {
    let e = exact(&MEANS, BUFFER);
    eprintln!("{} states: {e:?}", e.states);
    eprintln!("throughput {} parts/h", 60.0 * e.throughput);
    assert_conserves(&e, &MEANS);
    // Reversibility: the same line run backwards (M4 first) has the same throughput.
    let mut back = MEANS;
    back.reverse();
    let r = exact(&back, BUFFER);
    assert_conserves(&r, &back);
    assert!(
        (r.throughput - e.throughput).abs() < 1e-12,
        "{} vs {}",
        r.throughput,
        e.throughput
    );
}

const REPS: u64 = 20;
/// Fixed seeds, so the run is repeatable; each check is a 95% interval.
const SEED: u64 = 31_000;

/// The line as a StochCraft model. M1's raw material is a source at twice M1's rate into a
/// waiting room of 40 in front of M1: arrivals that find the room full are turned away there,
/// which stands for raw material left in the store, so the parts in the line are never lost.
/// M1 is starved only when 40 arrivals in a row are turned away, a chance below 2^-40 (as for
/// the first machine in `buffers.rs`). M2, M3 and M4 each have a buffer of 3 in front of them,
/// and a machine sending into a full one keeps its finished part, blocked.
fn line_model(duration: f64) -> Model {
    let mut b = Builder::new();
    let raw = b.source("raw", Dist::exponential_rate(2.0 / MEANS[0]), None);
    let m1 = b.buffered_server("m1", 1, Dist::Exponential { mean: MEANS[0] }, 40);
    let m2 = b.buffered_server("m2", 1, Dist::Exponential { mean: MEANS[1] }, BUFFER);
    let m3 = b.buffered_server("m3", 1, Dist::Exponential { mean: MEANS[2] }, BUFFER);
    let m4 = b.buffered_server("m4", 1, Dist::Exponential { mean: MEANS[3] }, BUFFER);
    let exit = b.sink("exit");
    b.connect(raw, m1);
    b.connect(m1, m2);
    b.connect(m2, m3);
    b.connect(m3, m4);
    b.connect(m4, exit);
    let model = b.build(Experiment {
        warmup: 2_000.0,
        duration,
        replications: REPS as u32,
        ..Experiment::default()
    });
    assert_eq!(model.validate(), vec![]);
    model
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

/// Throughput, each machine's utilisation (the share of time processing: StochCraft counts a
/// machine holding a finished part, blocked, as not busy, as the chain does) and the blocking of
/// M1 to M3, each within the 95% interval of 20 replications of 200,000 minutes.
#[test]
#[ignore = "slow in debug builds (about 20 s); CI runs it in release with the ignored tests"]
fn the_paper_line_matches_its_markov_chain() {
    let e = exact(&MEANS, BUFFER);
    let duration = 200_000.0;
    let reps = reports(&line_model(duration));
    let s = summarise(&reps);
    if let Some(g) = s.get("exit", Metric::Entered) {
        eprintln!(
            "throughput {} ± {} parts/h (theory {})",
            60.0 * g.mean / duration,
            60.0 * g.half_width / duration,
            60.0 * e.throughput
        );
    }
    covers(&s, "exit", Metric::Entered, e.throughput * duration);
    for (i, m) in ["m1", "m2", "m3", "m4"].into_iter().enumerate() {
        covers(&s, m, Metric::Utilisation, e.working[i]);
        if i < 3 {
            covers(&s, m, Metric::BlockingProbability, e.blocked[i]);
        }
        // No part is lost inside the line.
        if i > 0 {
            covers(&s, m, Metric::Lost, 0.0);
        }
    }
    // The first machine is never starved.
    for r in &reps {
        let starved = r.element("m1").and_then(|m| m.time_starved);
        assert!(starved.is_some_and(|x| x < 1e-9), "m1 starved {starved:?}");
    }
}

/// A short run, quick enough for every build, still within a wide margin of the targets.
#[test]
fn a_short_run_of_the_paper_line_is_near_its_targets() {
    let e = exact(&MEANS, BUFFER);
    let duration = 20_000.0;
    let s = summarise(&reports(&line_model(duration)));
    let near = |element: &str, metric: Metric, theory: f64| {
        let got = s.get(element, metric);
        assert!(
            got.is_some_and(|g| (g.mean - theory).abs() <= 3.0 * g.half_width),
            "{element} {}: {got:?} vs {theory}",
            metric.name()
        );
    };
    near("exit", Metric::Entered, e.throughput * duration);
    near("m2", Metric::Utilisation, e.working[1]);
}
