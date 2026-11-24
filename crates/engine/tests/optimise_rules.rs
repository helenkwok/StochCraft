//! `experiment.optimise` with rules and a continuous mean (ROADMAP M8, stage O4). The heavy
//! checks run many seeds of a search on queueing models whose answers are known exactly; they
//! are ignored by default and run in CI's heavy tests job (`cargo test --release -- --ignored`).
//!
//! **The criteria.** A search is correct in at least 95% of seeds (95 of 100), as in O2; the
//! 95% Wilson interval of each rate is printed for information only. The coverage of a 95%
//! interval is a different kind of rate: at best it is 0.95, so 95 of 100 would fail about half
//! the time by chance alone. It is held to at least 88 of 100, which a true coverage of 0.95
//! misses with probability below 0.01 (binomial), and printed with its Wilson interval.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // A failed step fails the test.

use std::io::Write;

use serde_json::{Value, json};
use stochcraft_engine::{Engine, ErrorCode, Journal, ReplayMode, ReplayOptions};
use stochcraft_stats::Proportion;

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

fn err(e: &mut Engine, command: &str, params: Value) -> stochcraft_engine::CommandError {
    match e.execute(command, params.clone()) {
        Ok(v) => panic!("{command} {params} should fail: {v}"),
        Err(f) => f,
    }
}

/// Two identical M/M/1 servers behind an exclusive gateway: arrivals at rate 1, mean service 1
/// at each server, `measured` time units after a warm-up of a tenth of it. With the gateway's
/// standard routing (no probabilities: an even random split) each server is M/M/1 with arrivals
/// at rate ½, so the mean time in system is 1 / (1 − ½) = 2 exactly.
fn two_servers(measured: f64) -> Engine {
    let exp = |mean: f64| json!({ "type": "exponential", "mean": mean });
    let mut e = Engine::new();
    ok(
        &mut e,
        "model.set",
        json!({ "model": {
            "name": "Two servers",
            "elements": [
                { "id": "arrivals", "kind": "source", "interarrival": exp(1.0) },
                { "id": "split", "kind": "exclusive_gateway" },
                { "id": "s1", "kind": "server", "capacity": 1, "service": exp(1.0) },
                { "id": "s2", "kind": "server", "capacity": 1, "service": exp(1.0) },
                { "id": "exit", "kind": "sink" }
            ],
            "connections": [
                { "id": "f0", "from": "arrivals", "to": "split" },
                { "id": "f1", "from": "split", "to": "s1" },
                { "id": "f2", "from": "split", "to": "s2" },
                { "id": "f3", "from": "s1", "to": "exit" },
                { "id": "f4", "from": "s2", "to": "exit" }
            ],
            "experiment": { "time_unit": "hour", "warmup": measured / 10.0, "duration": measured, "replications": 10, "seed": 1 }
        }}),
    );
    e
}

/// The routing search: the gateway's routing, its standard random split against the shortest
/// queue, minimising the mean time in system.
fn routing_search(seed: u64) -> Value {
    json!({
        "decisions": [{ "id": "split", "field": "routing" }],
        "objective": { "results": [{ "element": "exit", "metric": "mean_time_in_system" }] },
        "indifference": 0.1,
        "max_runs": 2000,
        "seed": seed,
    })
}

/// An M/M/1 server with arrivals at rate 1 and a mean service of 0.8, `measured` time units
/// after a warm-up of a tenth of it.
fn one_server(measured: f64) -> Engine {
    let exp = |mean: f64| json!({ "type": "exponential", "mean": mean });
    let mut e = Engine::new();
    ok(
        &mut e,
        "model.set",
        json!({ "model": {
            "name": "One server",
            "elements": [
                { "id": "arrivals", "kind": "source", "interarrival": exp(1.0) },
                { "id": "server", "kind": "server", "capacity": 1, "service": exp(0.8) },
                { "id": "exit", "kind": "sink" }
            ],
            "connections": [
                { "id": "f1", "from": "arrivals", "to": "server" },
                { "id": "f2", "from": "server", "to": "exit" }
            ],
            "experiment": { "time_unit": "hour", "warmup": measured / 10.0, "duration": measured, "replications": 10, "seed": 1 }
        }}),
    );
    e
}

/// The mean search: the server's mean service s in [0.2, 0.9], minimising its mean number inside
/// (s / (1 − s)) plus −4 per unit of s. The objective s / (1 − s) − 4s has its least value −1
/// at s = 0.5.
fn mean_search(seed: u64) -> Value {
    json!({
        "decisions": [{ "id": "server", "field": "mean", "min": 0.2, "max": 0.9 }],
        "objective": {
            "results": [{ "element": "server", "metric": "mean_work_in_progress" }],
            "costs": [{ "id": "server", "field": "mean", "cost": -4.0 }]
        },
        "indifference": 0.05,
        "max_runs": 4000,
        "seed": seed,
    })
}

/// The exact objective of the mean search at s.
fn mm1_cost(s: f64) -> f64 {
    s / (1.0 - s) - 4.0 * s
}

/// The chosen design's first decision value.
fn chosen_value(v: &Value) -> Value {
    v["chosen"]["decisions"][0]["value"].clone()
}

/// The finalist whose first decision has `value`.
fn finalist<'a>(v: &'a Value, value: &Value) -> Option<&'a Value> {
    v["finalists"]
        .as_array()?
        .iter()
        .find(|f| &f["decisions"][0]["value"] == value)
}

#[test]
fn the_shortest_queue_is_chosen_over_a_random_split() {
    let mut e = two_servers(1000.0);
    let v = ok(&mut e, "experiment.optimise", routing_search(1));
    assert_eq!(chosen_value(&v), json!("shortest_queue"), "{v}");
    assert_eq!(v["all_designs"], true);
    assert_eq!(v["designs_compared"], 2);
    assert_eq!(v["outcome"], "selected");
    assert!(
        v["claim"].as_str().unwrap().contains("of all 2 designs"),
        "{v}"
    );
    let split = finalist(&v, &json!("standard")).unwrap();
    assert!(split["objective"]["half_width"].as_f64().unwrap() > 0.0);
    assert_eq!(split["starting_design"], true);
    assert!(v["method"].as_str().unwrap().contains("rule"));
    // The same parameters give the same answer, digit for digit.
    let again = ok(&mut e, "experiment.optimise", routing_search(1));
    assert_eq!(again.to_string(), v.to_string());
}

#[test]
fn a_mean_is_searched_by_nelder_mead() {
    let mut e = one_server(1000.0);
    let v = ok(&mut e, "experiment.optimise", mean_search(1));
    let s = chosen_value(&v).as_f64().unwrap();
    assert!((s - 0.5).abs() <= 0.05, "{s}: {v}");
    assert!(mm1_cost(s) <= -1.0 + 0.05, "{s}");
    assert!(
        v["runs"]["total"].as_u64().unwrap() <= 4000,
        "{}",
        v["runs"]
    );
    let claim = v["claim"].as_str().unwrap();
    assert!(claim.contains("is local"), "{claim}");
    assert!(!claim.contains("of all"), "{claim}");
    assert_eq!(v["all_designs"], false);
    assert!(v["method"].as_str().unwrap().contains("Nelder"));
    // The starting design (0.8) is a finalist, and the answer repeats.
    assert!(finalist(&v, &json!(0.8)).is_some(), "{v}");
    let again = ok(&mut e, "experiment.optimise", mean_search(1));
    assert_eq!(again.to_string(), v.to_string());
}

#[test]
fn a_rule_beside_a_mean_is_searched_for_each_level() {
    let mut e = two_servers(200.0);
    let p = json!({
        "decisions": [
            { "id": "split", "field": "routing" },
            { "id": "s1", "field": "mean", "min": 0.5, "max": 1.5 }
        ],
        "objective": { "results": [{ "element": "exit", "metric": "mean_time_in_system" }] },
        "indifference": 0.2,
        "max_runs": 3000,
    });
    let v = ok(&mut e, "experiment.optimise", p);
    let levels: Vec<&Value> = v["search"]["starts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| &s["start"][0]["value"])
        .collect();
    assert_eq!(
        levels,
        [&json!("standard"), &json!("shortest_queue")],
        "{v}"
    );
    assert!(v["runs"]["total"].as_u64().unwrap() <= 3000);
}

#[test]
fn a_search_over_a_mean_stopped_by_its_time_limit_repeats_with_its_runs() {
    let mut e = one_server(200.0);
    let mut p = mean_search(3);
    p["time_limit"] = json!(0.0);
    let stopped = ok(&mut e, "experiment.optimise", p.clone());
    assert_eq!(stopped["search"]["ended"], "time_limit", "{stopped}");
    let repeat = &stopped["repeat"];
    let mut again = p;
    again.as_object_mut().unwrap().remove("time_limit");
    again["max_runs"] = repeat["max_runs"].clone();
    again["reserve_runs"] = repeat["reserve_runs"].clone();
    let repeated = ok(&mut e, "experiment.optimise", again);
    for key in ["chosen", "finalists", "outcome", "against_start"] {
        assert_eq!(repeated[key], stopped[key], "{key}");
    }
    assert_eq!(repeated["runs"]["search"], stopped["runs"]["search"]);
}

/// Applied answers are journalled as `decisions.set` edits with the rule's name and the mean's
/// number, alone and in a batch, and replay to the same model, from the journal and from its
/// text.
#[test]
fn applied_rules_and_means_replay_as_edits() {
    let cases: [(fn() -> Engine, Value); 2] = [
        (|| two_servers(200.0), routing_search(1)),
        (|| one_server(200.0), mean_search(1)),
    ];
    for (engine, mut search) in cases {
        search["apply"] = json!(true);
        for in_batch in [false, true] {
            let mut e = engine();
            let before = e.model().clone();
            let v = if in_batch {
                ok(
                    &mut e,
                    "batch",
                    json!({ "commands": [{ "command": "experiment.optimise", "params": search.clone() }] }),
                )["results"][0]
                    .clone()
            } else {
                ok(&mut e, "experiment.optimise", search.clone())
            };
            assert_eq!(v["applied"], true, "{v}");
            assert_ne!(e.model(), &before);
            let journal = e.journal_document();
            let text = serde_json::to_string(&journal).unwrap();
            let read: Journal = serde_json::from_str(&text).unwrap();
            for j in [&journal, &read] {
                for mode in [ReplayMode::EditsOnly, ReplayMode::All] {
                    let mut r = Engine::new();
                    let options = ReplayOptions {
                        mode,
                        ..ReplayOptions::default()
                    };
                    r.replay_journal(j, options).unwrap();
                    assert_eq!(r.model(), e.model(), "{mode:?}, batch {in_batch}");
                }
            }
        }
    }
}

#[test]
fn decisions_set_takes_rules_by_name_and_means_by_number() {
    let mut e = two_servers(100.0);
    let v = ok(
        &mut e,
        "decisions.set",
        json!({ "decisions": [
            { "id": "split", "field": "routing", "value": "shortest_queue" },
            { "id": "s1", "field": "mean", "value": 0.75 }
        ]}),
    );
    assert_eq!(v["changed"], true);
    let model = ok(&mut e, "model.get", json!({}));
    let split = model["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "split")
        .unwrap()
        .clone();
    assert_eq!(split["routing"], "shortest_queue", "{split}");
    // A rule's value is a name of one of its levels; a number is refused, and so is an unknown
    // level.
    for bad in [json!(1), json!("fastest"), Value::Null] {
        let f = err(
            &mut e,
            "decisions.set",
            json!({ "decisions": [{ "id": "split", "field": "routing", "value": bad }] }),
        );
        assert_eq!(f.code, ErrorCode::InvalidParams, "{bad}: {}", f.message);
    }
    // A rule is not a number for a sweep.
    let f = err(
        &mut e,
        "experiment.sweep",
        json!({ "parameter": { "id": "split", "field": "routing" }, "values": [0.0, 1.0], "results": [{ "element": "exit", "metric": "mean_time_in_system" }] }),
    );
    assert_eq!(f.code, ErrorCode::InvalidParams, "{}", f.message);
}

#[test]
fn refusals_name_what_is_wrong() {
    let cases: [(Value, &str); 9] = [
        (
            json!([{ "id": "split", "field": "routing", "min": 0, "max": 1 }]),
            "has no range",
        ),
        (
            json!([{ "id": "split", "field": "routing", "values": ["standard", "fastest"] }]),
            "not a routing",
        ),
        (
            json!([{ "id": "split", "field": "routing", "values": ["standard", "standard"] }]),
            "given twice",
        ),
        (
            json!([{ "id": "split", "field": "routing", "values": ["shortest_queue"] }]),
            "at least two levels",
        ),
        (
            json!([{ "id": "split", "field": "routing", "start": null }]),
            "null",
        ),
        (
            json!([{ "id": "s1", "field": "mean", "min": 0.5, "max": 1.5, "step": 1 }]),
            "a mean is a number in a range",
        ),
        (
            json!([{ "id": "s1", "field": "mean", "min": 0.5, "max": null }]),
            "null",
        ),
        (
            json!([{ "id": "s1", "field": "mean", "min": 2.0, "max": 3.0 }]),
            "outside its range",
        ),
        (
            json!([{ "id": "s1", "field": "on_shift_end" }]),
            "changes nothing",
        ),
    ];
    for (decisions, says) in cases {
        let mut e = two_servers(100.0);
        let p = json!({
            "decisions": decisions,
            "objective": { "results": [{ "element": "exit", "metric": "mean_time_in_system" }] },
            "indifference": 0.1,
            "max_runs": 100,
        });
        let f = err(&mut e, "experiment.optimise", p);
        assert!(
            f.message.contains(says),
            "{decisions}: `{}` should say `{says}`",
            f.message
        );
    }
    // A divert needs a default way: the level is refused before any run.
    let mut e = two_servers(100.0);
    let p = json!({
        "decisions": [{ "id": "split", "field": "routing", "values": ["standard", "divert"] }],
        "objective": { "results": [{ "element": "exit", "metric": "mean_time_in_system" }] },
        "indifference": 0.1,
        "max_runs": 100,
    });
    let f = err(&mut e, "experiment.optimise", p);
    assert_eq!(f.code, ErrorCode::InvalidModel, "{}", f.message);
    assert!(f.message.contains("divert"), "{}", f.message);
    // A rule has no cost; a mean's cost cannot be capped.
    let mut p = routing_search(1);
    p["objective"]["costs"] = json!([{ "id": "split", "field": "routing", "cost": 1.0 }]);
    let f = err(&mut e, "experiment.optimise", p);
    assert!(f.message.contains("no cost"), "{}", f.message);
    let mut e = one_server(100.0);
    let mut p = mean_search(1);
    p["cost_cap"] = json!(10.0);
    let f = err(&mut e, "experiment.optimise", p);
    assert!(f.message.contains("cannot be capped"), "{}", f.message);
    // With a mean, the other decisions may take at most 8 combinations.
    let mut e = two_servers(100.0);
    let p = json!({
        "decisions": [
            { "id": "s1", "field": "capacity", "min": 1, "max": 9 },
            { "id": "s2", "field": "mean", "min": 0.5, "max": 1.5 }
        ],
        "objective": { "results": [{ "element": "exit", "metric": "mean_time_in_system" }] },
        "indifference": 0.1,
        "max_runs": 100,
    });
    let f = err(&mut e, "experiment.optimise", p);
    assert!(f.message.contains("at most 8"), "{}", f.message);
}

/// The counts of one heavy check.
#[derive(Default)]
struct Tally {
    seeds: u64,
    correct: u64,
    runs: u64,
}

impl Tally {
    fn report(&self, name: &str, what: &str, seconds: f64) -> (f64, f64) {
        let p = Proportion::new(self.correct, self.seeds);
        let (lo, hi) = p.wilson(0.95).unwrap();
        let _ = writeln!(
            std::io::stderr(),
            "optimise {name}: {}/{} {what} = {:.4} (the Wilson 95% interval [{lo:.4}, {hi:.4}] is for information); mean runs {:.1}; {seconds:.1} s",
            self.correct,
            self.seeds,
            self.correct as f64 / self.seeds as f64,
            self.runs as f64 / self.seeds.max(1) as f64,
        );
        (lo, hi)
    }
}

/// Runs `seeds` searches on four threads.
fn many<M, P>(seeds: u64, model: M, params: P) -> (Vec<Value>, f64)
where
    M: Fn() -> Engine + Sync,
    P: Fn(u64) -> Value + Sync,
{
    let start = std::time::Instant::now();
    let answers: Vec<Value> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4u64)
            .map(|w| {
                let (model, params) = (&model, &params);
                scope.spawn(move || {
                    let mut out = Vec::new();
                    let mut seed = w;
                    while seed < seeds {
                        let mut e = model();
                        out.push((seed, ok(&mut e, "experiment.optimise", params(seed + 1))));
                        seed += 4;
                    }
                    out
                })
            })
            .collect();
        let mut all: Vec<(u64, Value)> = workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect();
        all.sort_by_key(|(s, _)| *s);
        all.into_iter().map(|(_, v)| v).collect()
    });
    (answers, start.elapsed().as_secs_f64())
}

/// The number of seeds of each heavy check.
const SEEDS: u64 = 100;

/// Two servers behind a gateway: joining the shortest queue is chosen in at least 95 of 100
/// seeds, and the random split's clean-up interval covers its exact mean time in system, 2.
#[test]
#[ignore = "heavy: 100 searches of the gateway's routing"]
fn the_routing_of_two_servers() {
    let (answers, seconds) = many(SEEDS, || two_servers(2000.0), routing_search);
    let mut chosen = Tally::default();
    let mut covered = Tally::default();
    for v in &answers {
        chosen.seeds += 1;
        chosen.correct += u64::from(chosen_value(v) == json!("shortest_queue"));
        chosen.runs += v["runs"]["total"].as_u64().unwrap_or(0);
        let split = finalist(v, &json!("standard")).unwrap();
        let mean = split["objective"]["mean"].as_f64().unwrap();
        let half = split["objective"]["half_width"].as_f64().unwrap();
        covered.seeds += 1;
        covered.correct += u64::from((mean - 2.0).abs() <= half);
    }
    chosen.report("routing", "chose the shortest queue", seconds);
    covered.report(
        "routing",
        "random-split intervals covered the exact 2",
        seconds,
    );
    assert!(
        chosen.correct * 100 >= 95 * chosen.seeds,
        "{}/{} chose the shortest queue, below the required 95%",
        chosen.correct,
        chosen.seeds
    );
    assert!(
        covered.correct >= 88,
        "{}/{} random-split intervals covered 2, below the required 88",
        covered.correct,
        covered.seeds
    );
}

/// The M/M/1 cost s / (1 − s) − 4s: the chosen mean service is within 0.05 of 0.5 and its exact
/// objective within δ = 0.05 of −1, in at least 95 of 100 seeds.
#[test]
#[ignore = "heavy: 100 searches of a mean service time"]
fn the_mean_service_of_an_mm1_cost() {
    assert!((mm1_cost(0.5) + 1.0).abs() < 1e-12);
    assert!((mm1_cost(0.4) + 0.9333).abs() < 1e-4);
    assert!((mm1_cost(0.6) + 0.9).abs() < 1e-12);
    let (answers, seconds) = many(SEEDS, || one_server(1000.0), mean_search);
    let mut tally = Tally::default();
    for v in &answers {
        let s = chosen_value(v).as_f64().unwrap_or(f64::NAN);
        tally.seeds += 1;
        tally.correct += u64::from((s - 0.5).abs() <= 0.05 && mm1_cost(s) <= -1.0 + 0.05);
        tally.runs += v["runs"]["total"].as_u64().unwrap_or(0);
    }
    tally.report("M/M/1 mean", "within 0.05 of s = 0.5 and of -1", seconds);
    assert!(
        tally.correct * 100 >= 95 * tally.seeds,
        "{}/{} correct, below the required 95%",
        tally.correct,
        tally.seeds
    );
}
