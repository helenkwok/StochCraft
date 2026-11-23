//! Networks (ROADMAP M4.3, stage A3): each generator against what it must give, worked out here
//! from the network itself, links made and removed during a run, messages along links, and SIR
//! on a complete network against A2's well-mixed answer.
#![allow(clippy::panic)] // A model that fails to build or run fails the test.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use stochcraft_agents::network::distance;
use stochcraft_agents::{
    Action, Chart, ChartSpec, Model, ModelError, Network, Population, Run, SendSpec, Simulation,
    StateSpec, Statistic, TransitionSpec, read_links, write_links,
};
use stochcraft_random::math::ln;

fn build(spec: &ChartSpec) -> Arc<Chart> {
    match spec.build() {
        Ok(c) => Arc::new(c),
        Err(e) => panic!("{e}"),
    }
}

fn simulate(model: &Model, seed: u64, end: f64) -> Simulation {
    let mut sim = match Simulation::new(model, seed) {
        Ok(s) => s,
        Err(e) => panic!("{e}"),
    };
    if let Err(e) = sim.run_until(end) {
        panic!("{e}");
    }
    sim
}

fn run(model: &Model, seed: u64, end: f64) -> Run {
    match model.run(seed, end) {
        Ok(r) => r,
        Err(e) => panic!("{e}"),
    }
}

/// An agent that does nothing (with a position, for distance networks).
fn node() -> Arc<Chart> {
    build(
        &ChartSpec::new("node")
            .data("x", "-1")
            .data("y", "-1")
            .state(StateSpec::state("here")),
    )
}

/// The links of `network` over `n` agents of one population, from `seed`.
fn links(network: Network, n: u64, seed: u64) -> Vec<(u64, u64)> {
    let model = Model::new().population(Population::new("p", node()).count(n).network(network));
    simulate(&model, seed, 0.0).links("p")
}

fn adjacency(n: u64, links: &[(u64, u64)]) -> Vec<BTreeSet<u64>> {
    let mut adj = vec![BTreeSet::new(); n as usize];
    for &(a, b) in links {
        adj[a as usize].insert(b);
        adj[b as usize].insert(a);
    }
    adj
}

/// Every link once, the lower serial first, no agent linked to itself.
fn assert_simple(links: &[(u64, u64)], n: u64) {
    for &(a, b) in links {
        assert!(a < b && b < n, "link ({a}, {b})");
    }
    let set: BTreeSet<_> = links.iter().collect();
    assert_eq!(set.len(), links.len(), "a link given twice");
}

/// The mean of the local clustering coefficients (Watts and Strogatz's C): for each agent, the
/// share of the pairs of its neighbours that are linked.
fn clustering(adj: &[BTreeSet<u64>]) -> f64 {
    let mut sum = 0.0;
    for nb in adj {
        let nb: Vec<u64> = nb.iter().copied().collect();
        let k = nb.len();
        let mut linked = 0;
        for (i, &a) in nb.iter().enumerate() {
            for &b in &nb[i + 1..] {
                if adj[a as usize].contains(&b) {
                    linked += 1;
                }
            }
        }
        sum += linked as f64 / (k * (k - 1) / 2) as f64;
    }
    sum / adj.len() as f64
}

/// The sum of the shortest path lengths over all ordered pairs, by breadth-first search from
/// every agent (the network is connected).
fn total_distance(adj: &[BTreeSet<u64>]) -> u64 {
    let mut total = 0;
    for from in 0..adj.len() {
        let mut dist = vec![u64::MAX; adj.len()];
        dist[from] = 0;
        let mut queue = VecDeque::from([from]);
        while let Some(a) = queue.pop_front() {
            for &b in &adj[a] {
                if dist[b as usize] == u64::MAX {
                    dist[b as usize] = dist[a] + 1;
                    queue.push_back(b as usize);
                }
            }
        }
        assert!(dist.iter().all(|&d| d != u64::MAX), "connected");
        total += dist.iter().sum::<u64>();
    }
    total
}

/// A ring of 100 with 2 links each side (degree k = 4): clustering 3(k − 2)/(4(k − 1)) = 1/2
/// exactly (3 of each agent's 6 pairs of neighbours are linked), and a mean shortest path of
/// 127,500 / 9,900 = 12.8788 (from one agent, ⌈r/2⌉ steps to each at ring distance r).
#[test]
fn a_ring_lattice_has_its_clustering_and_path_length() {
    let n = 100;
    let l = links(Network::ring(2), n, 1);
    assert_eq!(l.len(), 200);
    assert_simple(&l, n);
    let adj = adjacency(n, &l);
    assert!(adj.iter().all(|s| s.len() == 4));
    let k = 4.0;
    let c = clustering(&adj);
    assert_eq!(c, 3.0 * (k - 2.0) / (4.0 * (k - 1.0)));
    assert_eq!(c, 0.5);
    let total = total_distance(&adj);
    assert_eq!(total, 127_500);
    let mean = total as f64 / (n * (n - 1)) as f64;
    assert!((mean - 12.8788).abs() < 5e-5, "{mean}");
}

#[test]
fn a_small_world_keeps_its_links_as_it_rewires() {
    let n = 200;
    let lattice = links(Network::ring(3), n, 4);
    assert_eq!(links(Network::small_world(3, 0.0), n, 4), lattice);
    for p in [0.1, 1.0] {
        let l = links(Network::small_world(3, p), n, 4);
        assert_simple(&l, n);
        assert_eq!(l.len(), 600, "rewiring {p} keeps n·k links");
        let degrees: usize = adjacency(n, &l).iter().map(BTreeSet::len).sum();
        assert_eq!(degrees, 2 * l.len());
        assert_ne!(l, lattice, "rewiring {p} moved links");
    }
    // Rewiring every link leaves few of the lattice's: an agent's d-th link stays only if it
    // is drawn back to the same agent, so far fewer than the 10% a light rewiring keeps.
    let all = links(Network::small_world(3, 1.0), n, 4);
    let kept = all.iter().filter(|x| lattice.contains(x)).count();
    assert!(kept < 60, "{kept} of 600 lattice links left");
}

/// G(n, M): M = n·d / 2 links, so the mean degree is d exactly.
#[test]
fn a_random_network_has_its_mean_degree() {
    for (n, d) in [(1000, 4.0), (100, 3.3), (100, 99.0)] {
        let l = links(Network::random(d), n, 2);
        assert_simple(&l, n);
        let mean = 2.0 * l.len() as f64 / n as f64;
        assert_eq!(mean, d, "n = {n}");
    }
    // Different seeds give different networks of the same size.
    assert_ne!(
        links(Network::random(4.0), 1000, 2),
        links(Network::random(4.0), 1000, 3)
    );
    // A mean degree above n − 1 is refused.
    let model = Model::new().population(
        Population::new("p", node())
            .count(10)
            .network(Network::random(9.5)),
    );
    assert!(matches!(
        Simulation::new(&model, 1),
        Err(ModelError::Invalid(_))
    ));
}

/// The degree distribution of preferential attachment with m links per agent is
/// p(k) = 2m(m + 1) / (k(k + 1)(k + 2)) for k ≥ m (Dorogovtsev, Mendes and Samukhin 2000;
/// Bollobás, Riordan, Spencer and Tusnády 2001), whose tail falls as k^−3 (Barabási and Albert
/// 1999). The discrete maximum-likelihood estimate of the exponent over k ≥ k_min (Clauset,
/// Shalizi and Newman 2009, 1 + n [Σ ln(k / (k_min − 1/2))]^−1) tends, for that distribution,
/// to 1 + 1 / E[ln(k / (k_min − 1/2)) | k ≥ k_min], worked out here; it is near 3 and comes
/// nearer as k_min grows. The run's estimate must lie within four of its standard errors
/// ((α − 1) / √n) of it.
#[test]
fn preferential_attachment_has_a_tail_of_exponent_3() {
    let (n, m) = (20_000u64, 3u64);
    let l = links(Network::scale_free(3, m), n, 5);
    assert_simple(&l, n);
    assert_eq!(l.len() as u64, 3 + m * (n - 3));
    let degrees: Vec<u64> = adjacency(n, &l).iter().map(|s| s.len() as u64).collect();
    assert!(degrees.iter().all(|&k| k >= m), "every degree is m or more");
    let expected = |k_min: u64| {
        let (mut num, mut den) = (0.0, 0.0);
        for k in k_min..2_000_000u64 {
            let p = 1.0 / (k as f64 * (k + 1) as f64 * (k + 2) as f64);
            num += p * ln(k as f64 / (k_min as f64 - 0.5));
            den += p;
        }
        1.0 + den / num
    };
    let k_min = 12;
    let target = expected(k_min);
    assert!((target - 3.0).abs() < 0.15, "{target}");
    assert!((expected(100) - 3.0).abs() < (target - 3.0).abs());
    let tail: Vec<f64> = degrees
        .iter()
        .filter(|&&k| k >= k_min)
        .map(|&k| k as f64)
        .collect();
    let s: f64 = tail.iter().map(|&k| ln(k / (k_min as f64 - 0.5))).sum();
    let alpha = 1.0 + tail.len() as f64 / s;
    let se = (alpha - 1.0) / (tail.len() as f64).sqrt();
    assert!(
        (alpha - target).abs() < 4.0 * se,
        "exponent {alpha}, expected {target}, standard error {se}, {} in the tail",
        tail.len()
    );
}

/// Two agents are linked if and only if they are within the range; their positions are in the
/// rectangle and in their variables.
#[test]
fn a_distance_network_links_exactly_the_pairs_within_range() {
    let (n, range) = (300u64, 1.0);
    let model = Model::new().population(
        Population::new("p", node())
            .count(n)
            .network(Network::distance(10.0, 5.0, range)),
    );
    let sim = simulate(&model, 6, 0.0);
    let links: BTreeSet<(u64, u64)> = sim.links("p").into_iter().collect();
    assert!(links.len() > 100, "{}", links.len());
    let pos: Vec<(f64, f64)> = (0..n)
        .map(|s| match sim.position("p", s) {
            Some(p) => p,
            None => panic!("agent {s} has no position"),
        })
        .collect();
    for (s, &(x, y)) in pos.iter().enumerate() {
        assert!((0.0..10.0).contains(&x) && (0.0..5.0).contains(&y));
        let a = sim.agent("p", s as u64);
        assert_eq!(a.and_then(|a| a.value("x")).map(|v| v.number()), Some(x));
        assert_eq!(a.and_then(|a| a.value("y")).map(|v| v.number()), Some(y));
    }
    for a in 0..n {
        for b in a + 1..n {
            let (pa, pb) = (pos[a as usize], pos[b as usize]);
            let (dx, dy) = (pa.0 - pb.0, pa.1 - pb.1);
            let d = distance(pa, pb);
            let plain = (dx * dx + dy * dy).sqrt();
            assert!((d - plain).abs() <= 1e-12 * plain, "{d} against {plain}");
            assert_eq!(links.contains(&(a, b)), d <= range, "agents {a} and {b}");
        }
    }
    // Without the variables `x` and `y` the model is refused.
    let bare = build(&ChartSpec::new("bare").state(StateSpec::state("here")));
    let model = Model::new().population(
        Population::new("p", bare)
            .count(3)
            .network(Network::distance(1.0, 1.0, 0.5)),
    );
    assert!(matches!(
        Simulation::new(&model, 1),
        Err(ModelError::Invalid(_))
    ));
}

#[test]
fn links_go_to_csv_and_back() {
    let n = 50;
    let l = links(Network::small_world(2, 0.3), n, 8);
    let text = write_links(&l);
    assert!(text.starts_with("from,to\n"));
    assert_eq!(read_links(&text, n), Ok(l.clone()));
    assert_eq!(links(Network::csv(&text), n, 99), l);
    assert_eq!(links(Network::links(l.clone()), n, 99), l);
    // Bad lines are refused with their line numbers.
    let line = |t: &str| read_links(t, n).map_err(|e| e.line);
    assert_eq!(line("from,to\n0,1\n3,3\n"), Err(3), "a link to itself");
    assert_eq!(line("0,1\n2,3\n1,0\n"), Err(3), "the same link both ways");
    assert_eq!(line("0,1\n\n0,50\n"), Err(3), "no agent 50");
    assert_eq!(line("0,1,2\n"), Err(1), "three fields");
    assert_eq!(line("0,1\n1.5,2\n"), Err(2), "not a whole number");
    let model = Model::new().population(
        Population::new("p", node())
            .count(n)
            .network(Network::csv("0,1\n4,4\n")),
    );
    match Simulation::new(&model, 1) {
        Err(ModelError::Invalid(e)) => assert!(e.message.contains("line 2"), "{e}"),
        other => panic!("{other:?}"),
    }
}

/// Agent 0 links to agent 1 at time 1 and unlinks at time 2; at 1.5 it messages the agents it
/// is linked to. Both agents' conditions on their degree are tested again when the link is made
/// and removed, so they move at exactly 1 and 2; agent 2, never linked, stays.
#[test]
fn links_made_and_removed_during_a_run() {
    let chart = build(
        &ChartSpec::new("linker")
            .data("at", "-1")
            .data("gone", "-1")
            .data("got", "0")
            .state(
                StateSpec::state("life")
                    .on_entry(SendSpec::new("go").after(1.0))
                    .on_entry(SendSpec::new("talk").after(1.5))
                    .on_entry(SendSpec::new("cut").after(2.0))
                    .transition(
                        TransitionSpec::on("go")
                            .when("serial() = 0")
                            .then(Action::connect("p", "1"))
                            .then(Action::connect("p", "0"))
                            .then(Action::connect("p", "1")),
                    )
                    .transition(
                        TransitionSpec::on("talk")
                            .when("serial() = 0")
                            .then(SendSpec::new("hi").to_linked("p")),
                    )
                    .transition(
                        TransitionSpec::on("cut")
                            .when("serial() = 0")
                            .then(Action::disconnect("p", "1")),
                    )
                    .transition(TransitionSpec::on("hi").then(Action::assign("got", "$got + 1")))
                    .child(
                        StateSpec::state("wait").transition(
                            TransitionSpec::eventless()
                                .when("degree('p') >= 1")
                                .to("linked")
                                .then(Action::assign("at", "time()")),
                        ),
                    )
                    .child(
                        StateSpec::state("linked").transition(
                            TransitionSpec::eventless()
                                .when("degree('p') = 0")
                                .to("alone")
                                .then(Action::assign("gone", "time()")),
                        ),
                    )
                    .child(StateSpec::state("alone")),
            ),
    );
    assert_eq!(chart.waits_on(), ["p"]);
    let model = Model::new().population(
        Population::new("p", chart)
            .count(3)
            .network(Network::empty()),
    );
    let sim = simulate(&model, 1, 1.75);
    assert_eq!(sim.links("p"), [(0, 1)], "one link, made once");
    assert_eq!(sim.linked("p", 1), [0]);
    let sim = simulate(&model, 1, 5.0);
    assert!(sim.links("p").is_empty());
    let value = |s: u64, v: &str| {
        sim.agent("p", s)
            .and_then(|a| a.value(v))
            .map_or(f64::NAN, |x| x.number())
    };
    for s in 0..2 {
        assert_eq!((value(s, "at"), value(s, "gone")), (1.0, 2.0), "agent {s}");
    }
    assert_eq!((value(2, "at"), value(2, "gone")), (-1.0, -1.0));
    assert_eq!(
        [value(0, "got"), value(1, "got"), value(2, "got")],
        [0.0, 1.0, 0.0]
    );
    // A chart that reads a network its population does not have is refused.
    let reader = build(
        &ChartSpec::new("reader")
            .data("d", "degree('p')")
            .state(StateSpec::state("here")),
    );
    let model = Model::new().population(Population::new("p", reader).count(2));
    match Simulation::new(&model, 1) {
        Err(ModelError::Invalid(e)) => assert!(e.message.contains("no network"), "{e}"),
        other => panic!("{other:?}"),
    }
}

/// The final size of SIR in a well-mixed population of `n` with one infective, from the
/// embedded jump chain (as A2's check works it out): with s susceptible the next event is an
/// infection with probability r0 s / (r0 s + n − 1).
fn sir_final_size(n: usize, r0: f64) -> Vec<f64> {
    let mut prob = vec![vec![0.0; n + 2]; n];
    prob[n - 1][1] = 1.0;
    let mut final_size = vec![0.0; n + 1];
    for s in (0..n).rev() {
        for i in (1..=n).rev() {
            let w = prob[s][i];
            if w == 0.0 {
                continue;
            }
            let infect = r0 * s as f64 / (r0 * s as f64 + (n - 1) as f64);
            if s > 0 {
                prob[s - 1][i + 1] += w * infect;
            }
            if i == 1 {
                final_size[n - s] += w * (1.0 - infect);
            } else {
                prob[s][i - 1] += w * (1.0 - infect);
            }
        }
    }
    final_size
}

/// SIR with each infective contacting a random agent: `linked` picks among its links, otherwise
/// among the whole population (A2's model).
fn sir(n: u64, linked: bool) -> Model {
    let contact = SendSpec::new("contact");
    let contact = if linked {
        contact.to_linked_random("people")
    } else {
        contact.to_random("people")
    };
    let chart = build(
        &ChartSpec::new("person")
            .state(
                StateSpec::state("start")
                    .transition(
                        TransitionSpec::eventless()
                            .when("serial() = 0")
                            .to("infected"),
                    )
                    .transition(TransitionSpec::eventless().to("susceptible")),
            )
            .state(
                StateSpec::state("susceptible")
                    .transition(TransitionSpec::on("contact").to("infected")),
            )
            .state(
                StateSpec::state("infected")
                    .on_entry(SendSpec::new("talk").id("t").at_rate("1.5"))
                    .on_entry(SendSpec::new("recover").at_rate("1"))
                    .on_exit(Action::cancel("t"))
                    .transition(
                        TransitionSpec::on("talk")
                            .then(contact)
                            .then(SendSpec::new("talk").id("t").at_rate("1.5")),
                    )
                    .transition(TransitionSpec::on("recover").to("recovered")),
            )
            .state(StateSpec::state("recovered")),
    );
    let mut people = Population::new("people", chart).count(n);
    if linked {
        people = people.network(Network::random((n - 1) as f64));
    }
    Model::new()
        .population(people)
        .statistic(Statistic::count("susceptible", "people").when("In('susceptible')"))
}

fn final_size(run: &Run, n: u64) -> f64 {
    let left = run
        .series
        .first()
        .and_then(|s| s.values.last())
        .copied()
        .unwrap_or(f64::NAN);
    n as f64 - left
}

fn check_sir(reps: u64) {
    let n = 100;
    let dist = sir_final_size(n as usize, 1.5);
    let exact_mean: f64 = dist.iter().enumerate().map(|(k, p)| k as f64 * p).sum();
    assert!((exact_mean - 20.3696).abs() < 5e-5, "{exact_mean}");
    let exact_var: f64 = dist
        .iter()
        .enumerate()
        .map(|(k, p)| (k as f64 - exact_mean) * (k as f64 - exact_mean) * p)
        .sum();
    let model = sir(n, true);
    let sizes: Vec<f64> = (1..=reps)
        .map(|seed| final_size(&run(&model, seed, 1e6), n))
        .collect();
    let m = sizes.iter().sum::<f64>() / reps as f64;
    let se = (exact_var / reps as f64).sqrt();
    assert!(
        (m - exact_mean).abs() < 4.0 * se,
        "mean final size {m}, exact {exact_mean}, standard error {se}"
    );
}

/// On a complete network a random linked agent is a random other agent: the run is A2's
/// well-mixed SIR draw for draw (the pick comes from the same stream, among the same agents in
/// the same order), and its mean final size matches the exact 20.3696 for N = 100, R0 = 1.5.
#[test]
fn sir_on_a_complete_network_is_well_mixed_sir() {
    let n = 100;
    let (net, mixed) = (sir(n, true), sir(n, false));
    let sim = simulate(&net, 1, 0.0);
    assert_eq!(sim.links("people").len() as u64, n * (n - 1) / 2);
    for seed in 1..=20 {
        assert_eq!(run(&net, seed, 1e6), run(&mixed, seed, 1e6), "seed {seed}");
    }
    check_sir(300);
}

#[test]
#[ignore = "heavy: 4,000 runs of SIR on a complete network"]
fn sir_on_a_complete_network_over_many_runs() {
    check_sir(4000);
}

/// The same seed gives the same network and results; another population, with agents and a
/// network of its own, changes neither.
#[test]
fn networks_are_deterministic_and_their_own() {
    let spreader = || {
        build(
            &ChartSpec::new("spreader").data("heard", "0").state(
                StateSpec::state("s")
                    .on_entry(SendSpec::new("go").after_expr("exponential(1)"))
                    .transition(
                        TransitionSpec::on("go")
                            .then(SendSpec::new("news").to_linked_random("a"))
                            .then(SendSpec::new("go").after_expr("exponential(1)")),
                    )
                    .transition(
                        TransitionSpec::on("news").then(Action::assign("heard", "$heard + 1")),
                    ),
            ),
        )
    };
    let a = || {
        Population::new("a", spreader())
            .count(60)
            .network(Network::small_world(3, 0.2))
    };
    let stat = || Statistic::sum("heard", "a", "$heard");
    let alone = Model::new()
        .population(a())
        .statistic(stat())
        .save_every(1.0);
    let b = build(
        &ChartSpec::new("b")
            .data("x", "0")
            .data("y", "0")
            .state(StateSpec::state("here")),
    );
    let with_b = Model::new()
        .population(
            Population::new("b", b)
                .count(25)
                .network(Network::distance(1.0, 1.0, 0.3)),
        )
        .population(a())
        .statistic(stat())
        .save_every(1.0);
    let first = run(&alone, 3, 10.0);
    assert_eq!(first, run(&alone, 3, 10.0));
    assert_eq!(first.series, run(&with_b, 3, 10.0).series);
    assert_ne!(first.series, run(&alone, 4, 10.0).series);
    let links = |m: &Model, seed| simulate(m, seed, 0.0).links("a");
    assert_eq!(links(&alone, 3), links(&with_b, 3));
    assert_ne!(links(&alone, 3), links(&alone, 4));
}

/// An agent that ends keeps its links until what it sent has gone, in order: it unlinks agent 1,
/// messages the agents it is still linked to (only agent 2), and enters a final state in one
/// macrostep. Then it leaves, and its links with it.
#[test]
fn an_agent_that_ends_sends_along_its_links_as_they_are_then() {
    let chart = build(
        &ChartSpec::new("leaver")
            .data("got", "0")
            .state(
                StateSpec::state("s")
                    .on_entry(SendSpec::new("go").after(1.0))
                    .transition(
                        TransitionSpec::on("go")
                            .when("serial() = 0")
                            .to("gone")
                            .then(Action::disconnect("p", "1"))
                            .then(SendSpec::new("hi").to_linked("p")),
                    )
                    .transition(TransitionSpec::on("hi").then(Action::assign("got", "$got + 1"))),
            )
            .state(StateSpec::final_state("gone")),
    );
    let model = Model::new().population(
        Population::new("p", chart)
            .count(3)
            .network(Network::links(vec![(0, 1), (0, 2), (1, 2)])),
    );
    let sim = simulate(&model, 1, 2.0);
    assert_eq!(sim.alive("p"), [1, 2]);
    assert_eq!(sim.links("p"), [(1, 2)]);
    let got = |s: u64| {
        sim.agent("p", s)
            .and_then(|a| a.value("got"))
            .map_or(f64::NAN, |v| v.number())
    };
    assert_eq!((got(1), got(2)), (0.0, 1.0));
    let c = &sim.counts()[0];
    assert_eq!((c.removed, c.undelivered), (1, 0));
}
