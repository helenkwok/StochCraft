//! `experiment.optimise`: whole-number decisions by COMPASS and the clean-up (ROADMAP M8, stage
//! O2). The heavy checks run many seeds of a search on queueing models whose answers are known
//! from Erlang C; they are ignored by default and run in CI's heavy tests job
//! (`cargo test --release -- --ignored`), which prints each rate with its 95% Wilson interval.
//! The search and the clean-up are conservative, so a rate must itself be at least 0.95.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // A failed step fails the test.

use std::io::Write;

use serde_json::{Value, json};
use stochcraft_engine::{COMMANDS, Engine, ErrorCode};
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

/// Erlang C's mean wait in queue for M/M/c with arrival rate `lambda` and service rate `mu`.
fn erlang_c_wait(lambda: f64, mu: f64, c: u32) -> f64 {
    let a = lambda / mu;
    let rho = a / f64::from(c);
    if rho >= 1.0 {
        return f64::INFINITY;
    }
    let mut b = 1.0;
    for k in 1..=c {
        b = a * b / (f64::from(k) + a * b);
    }
    let wait = b / (1.0 - rho * (1.0 - b));
    wait / (f64::from(c) * mu - lambda)
}

/// The two desks' cost an hour: 20 per server, 50 per customer waiting (λ Wq at each desk).
fn tandem_cost(c1: u32, c2: u32) -> f64 {
    20.0 * f64::from(c1 + c2)
        + 50.0 * 10.0 * erlang_c_wait(10.0, 1.0, c1)
        + 50.0 * 10.0 * erlang_c_wait(10.0, 2.0, c2)
}

/// One desk, arrivals 4 an hour, mean service 1 hour: 10 a server and 25 a customer waiting.
fn one_desk_cost(c: u32) -> f64 {
    10.0 * f64::from(c) + 25.0 * 4.0 * erlang_c_wait(4.0, 1.0, c)
}

/// Two M/M/c desks in tandem, hours: arrivals 10 an hour, mean service 1 hour at the first and
/// 0.5 at the second, exponential (or constant); 11 and 6 servers; a warm-up of a tenth of the
/// measured `hours` (100 for the heavy checks, fewer for quick ones).
fn tandem(service: &str, hours: f64) -> Engine {
    let time = |mean: f64| {
        if service == "constant" {
            json!({ "type": "constant", "value": mean })
        } else {
            json!({ "type": "exponential", "mean": mean })
        }
    };
    let mut e = Engine::new();
    ok(
        &mut e,
        "model.set",
        json!({ "model": {
            "name": "Two desks",
            "elements": [
                { "id": "arrivals", "kind": "source", "interarrival": time(0.1) },
                { "id": "desk1", "kind": "server", "capacity": 11, "service": time(1.0) },
                { "id": "desk2", "kind": "server", "capacity": 6, "service": time(0.5) },
                { "id": "exit", "kind": "sink" }
            ],
            "connections": [
                { "id": "f1", "from": "arrivals", "to": "desk1" },
                { "id": "f2", "from": "desk1", "to": "desk2" },
                { "id": "f3", "from": "desk2", "to": "exit" }
            ],
            "experiment": { "time_unit": "hour", "warmup": hours / 10.0, "duration": hours, "replications": 10, "seed": 1 }
        }}),
    );
    e
}

/// One desk: arrivals 4 an hour, mean service 1 hour, 6 servers; a warm-up of 10 hours and 100
/// measured.
fn one_desk() -> Engine {
    one_desk_with(6)
}

/// One desk as [`one_desk`], with `servers` servers.
fn one_desk_with(servers: u32) -> Engine {
    let mut e = Engine::new();
    ok(
        &mut e,
        "model.set",
        json!({ "model": {
            "name": "One desk",
            "elements": [
                { "id": "arrivals", "kind": "source", "interarrival": { "type": "exponential", "mean": 0.25 } },
                { "id": "desk", "kind": "server", "capacity": servers, "service": { "type": "exponential", "mean": 1.0 } },
                { "id": "exit", "kind": "sink" }
            ],
            "connections": [
                { "id": "f1", "from": "arrivals", "to": "desk" },
                { "id": "f2", "from": "desk", "to": "exit" }
            ],
            "experiment": { "time_unit": "hour", "warmup": 10.0, "duration": 100.0, "replications": 10, "seed": 1 }
        }}),
    );
    e
}

/// The two desks' search: 11–40 × 6–35 from (11, 6), 20 a server and 50 a customer waiting.
fn tandem_search(seed: u64, max_runs: u64) -> Value {
    json!({
        "decisions": [
            { "id": "desk1", "field": "capacity", "min": 11, "max": 40 },
            { "id": "desk2", "field": "capacity", "min": 6, "max": 35 }
        ],
        "objective": {
            "results": [
                { "element": "desk1", "metric": "mean_queue_length", "weight": 50.0 },
                { "element": "desk2", "metric": "mean_queue_length", "weight": 50.0 }
            ],
            "costs": [
                { "id": "desk1", "field": "capacity", "cost": 20.0 },
                { "id": "desk2", "field": "capacity", "cost": 20.0 }
            ]
        },
        "indifference": 2.0,
        "max_runs": max_runs,
        "seed": seed,
    })
}

/// The chosen design's decision values.
fn chosen(v: &Value) -> Vec<u32> {
    v["chosen"]["decisions"]
        .as_array()
        .map(|ds| {
            ds.iter()
                .map(|d| d["value"].as_f64().unwrap_or(-1.0) as u32)
                .collect()
        })
        .unwrap_or_default()
}

/// Every design a finalist list or start names.
fn designs(v: &Value) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    for f in v["finalists"].as_array().into_iter().flatten() {
        out.push(
            f["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d["value"].as_f64().unwrap() as u32)
                .collect(),
        );
    }
    out
}

#[test]
fn the_catalogue_example_compares_every_design_of_a_small_space() {
    let mut e = Engine::new();
    ok(&mut e, "model.set", COMMANDS[0].example());
    let example = COMMANDS
        .iter()
        .find(|c| c.name == "experiment.optimise")
        .unwrap()
        .example();
    let v = ok(&mut e, "experiment.optimise", example.clone());
    assert_eq!(v["all_designs"], true, "{v}");
    assert_eq!(v["designs_compared"], 3);
    assert_eq!(v["search"]["ended"], "all_designs");
    assert!(
        v["claim"].as_str().unwrap().contains("of all 3 designs"),
        "{v}"
    );
    assert!(v["runs"]["total"].as_u64().unwrap() <= 300);
    assert_eq!(v["runs"]["search"], 0);
    // Recorded in the next run's record, and the model is unchanged without `apply`.
    let model = ok(&mut e, "model.get", json!({}));
    let run = ok(&mut e, "run", json!({ "replications": 2 }));
    let record = &run["record"]["sensitivity"][0];
    assert_eq!(record["kind"], "optimisation", "{record}");
    assert_eq!(record["outcome"], v["outcome"]);
    assert_eq!(ok(&mut e, "model.get", json!({})), model);
    // The same parameters give the same answer, digit for digit.
    let again = ok(&mut e, "experiment.optimise", example);
    assert_eq!(again.to_string(), v.to_string());
}

#[test]
fn apply_makes_the_chosen_design_the_models_as_one_undo_step() {
    // Nine servers to start with, so the answer (6 or 7) changes the model.
    let mut e = one_desk_with(9);
    let before = ok(&mut e, "model.get", json!({}));
    let v = ok(
        &mut e,
        "experiment.optimise",
        json!({
            "decisions": [{ "id": "desk", "field": "capacity", "min": 5, "max": 12 }],
            "objective": {
                "results": [{ "element": "desk", "metric": "mean_queue_length", "weight": 25.0 }],
                "costs": [{ "id": "desk", "field": "capacity", "cost": 10.0 }]
            },
            "indifference": 5.0,
            "max_runs": 3000,
            "apply": true,
        }),
    );
    let c = chosen(&v);
    assert!(c == [6] || c == [7], "{v}");
    assert_eq!(v["outcome"], "selected", "{v}");
    assert_eq!(v["applied"], true);
    let after = ok(&mut e, "model.get", json!({}));
    let desk = |m: &Value| {
        m["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "desk")
            .unwrap()["capacity"]
            .clone()
    };
    assert_eq!(desk(&after), json!(c[0]));
    ok(&mut e, "undo", json!({}));
    assert_eq!(ok(&mut e, "model.get", json!({})), before);
}

#[test]
fn refusals_name_what_is_wrong() {
    let mut e = tandem("exponential", 20.0);
    let mut p = tandem_search(1, 500);
    p["decisions"][0]["id"] = json!("nowhere");
    let f = err(&mut e, "experiment.optimise", p);
    assert_eq!(f.code, ErrorCode::NotFound);
    assert!(f.message.contains("nowhere"), "{}", f.message);

    let mut p = tandem_search(1, 500);
    p["decisions"][0]["field"] = json!("mean");
    let f = err(&mut e, "experiment.optimise", p);
    assert_eq!(f.code, ErrorCode::InvalidParams);

    // The starting design (11, 6) costs 340, over a cap of 300.
    let mut p = tandem_search(1, 500);
    p["cost_cap"] = json!(300.0);
    let f = err(&mut e, "experiment.optimise", p);
    assert!(f.message.contains("over the cap"), "{}", f.message);

    let mut p = tandem_search(1, 500);
    p["objective"]["costs"][0]["id"] = json!("arrivals");
    let f = err(&mut e, "experiment.optimise", p);
    assert!(f.message.contains("not one"), "{}", f.message);

    let mut p = tandem_search(1, 500);
    p["indifference"] = json!(0.0);
    let f = err(&mut e, "experiment.optimise", p);
    assert_eq!(f.code, ErrorCode::InvalidParams);

    let mut p = tandem_search(1, 500);
    p["decisions"][1]["min"] = json!(7);
    let f = err(&mut e, "experiment.optimise", p);
    assert!(f.message.contains("start"), "{}", f.message);
}

/// A cap on the costs: no design over it is a finalist or a start's end, and the search stays
/// within its budget.
#[test]
fn designs_over_the_cap_are_never_compared() {
    let mut e = tandem("exponential", 20.0);
    let mut p = tandem_search(5, 1500);
    p["cost_cap"] = json!(420.0);
    let v = ok(&mut e, "experiment.optimise", p);
    for d in designs(&v) {
        assert!(d[0] + d[1] <= 21, "{d:?} is over the cap: {v}");
    }
    assert!(
        v["runs"]["total"].as_u64().unwrap() <= 1500,
        "{}",
        v["runs"]
    );
    let c = chosen(&v);
    assert!(c.iter().sum::<u32>() <= 21);
}

/// No randomness: constant times, one replication per design, compared exactly.
#[test]
fn a_model_with_constant_times_is_compared_exactly() {
    let mut e = tandem("constant", 20.0);
    let v = ok(&mut e, "experiment.optimise", tandem_search(1, 5000));
    assert_eq!(v["outcome"], "compared_exactly", "{v}");
    assert_eq!(v["confidence"], Value::Null);
    assert_eq!(v["no_randomness"], true);
    assert!(v["claim"].as_str().unwrap().contains("compared exactly"));
    // One replication per design tried (and two for the check that there is no randomness).
    assert_eq!(v["runs"]["search"], v["designs_tried"], "{}", v["runs"]);
    assert_eq!(v["runs"]["clean_up"], 0);
    // With constant times nobody waits at (10, 5) and up, so the fewest servers win: the
    // first desk needs 10 (it has 11 at least), the second 5 (it has 6 at least).
    assert_eq!(chosen(&v), vec![11, 6], "{v}");
}

/// A search stopped by its time limit is repeated exactly with the `repeat` it gives.
#[test]
fn a_search_stopped_by_its_time_limit_repeats_with_its_runs() {
    let mut e = tandem("exponential", 20.0);
    let mut p = tandem_search(3, 2000);
    p["time_limit"] = json!(0.0);
    let stopped = ok(&mut e, "experiment.optimise", p.clone());
    assert_eq!(stopped["search"]["ended"], "time_limit", "{stopped}");
    let repeat = &stopped["repeat"];
    let mut again = p;
    again.as_object_mut().unwrap().remove("time_limit");
    again["max_runs"] = repeat["max_runs"].clone();
    again["reserve_runs"] = repeat["reserve_runs"].clone();
    let repeated = ok(&mut e, "experiment.optimise", again);
    assert_eq!(repeated["search"]["ended"], "budget", "{repeated}");
    for key in ["chosen", "finalists", "outcome", "against_start"] {
        assert_eq!(repeated[key], stopped[key], "{key}");
    }
    assert_eq!(repeated["runs"]["search"], stopped["runs"]["search"]);
    assert_eq!(repeated["runs"]["clean_up"], stopped["runs"]["clean_up"]);
}

/// The counts of one heavy check.
#[derive(Default)]
struct Tally {
    seeds: u64,
    correct: u64,
    proven: u64,
    not_proven: u64,
    runs: u64,
}

impl Tally {
    fn add(&mut self, v: &Value, correct: bool) {
        self.seeds += 1;
        self.correct += u64::from(correct);
        self.proven += u64::from(v["outcome"] != "not_proven");
        self.not_proven += u64::from(v["outcome"] == "not_proven");
        self.runs += v["runs"]["total"].as_u64().unwrap_or(0);
    }

    fn check(&self, name: &str, seconds: f64) {
        let p = Proportion::new(self.correct, self.seeds);
        let (lo, hi) = p.wilson(0.95).unwrap();
        let _ = writeln!(
            std::io::stderr(),
            "optimise {name}: {}/{} correct = {:.4}, Wilson 95% [{lo:.4}, {hi:.4}]; proven or exact {}, not proven {}; mean runs {:.1}; {seconds:.1} s",
            self.correct,
            self.seeds,
            self.correct as f64 / self.seeds as f64,
            self.proven,
            self.not_proven,
            self.runs as f64 / self.seeds as f64,
        );
        assert!(
            self.correct * 100 >= 95 * self.seeds,
            "{name}: {}/{} correct (Wilson 95% [{lo:.4}, {hi:.4}]) is below 0.95",
            self.correct,
            self.seeds
        );
    }
}

/// Runs `seeds` searches on four threads; `judge` says whether each answer is correct.
fn many<M, P, J>(name: &str, seeds: u64, model: M, params: P, judge: J)
where
    M: Fn() -> Engine + Sync,
    P: Fn(u64) -> Value + Sync,
    J: Fn(&Value) -> bool + Sync,
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
    let mut tally = Tally::default();
    for v in &answers {
        tally.add(v, judge(v));
    }
    tally.check(name, start.elapsed().as_secs_f64());
}

/// The number of seeds of each heavy check.
const SEEDS: u64 = 100;

/// Two desks in tandem: Erlang C's optimum is (14, 8) at 475.71 an hour; (13, 8) costs 481.48
/// and (14, 7) 482.29, both more than δ = 2 worse. From (11, 6) over 11–40 × 6–35, the search
/// must choose (14, 8) or a design within 2 of it.
#[test]
#[ignore = "heavy: 100 searches of the two desks"]
fn two_desks_in_tandem() {
    let best = tandem_cost(14, 8);
    assert!((best - 475.71).abs() < 0.01, "{best}");
    many(
        "two desks",
        SEEDS,
        || tandem("exponential", 100.0),
        |seed| tandem_search(seed, 20_000),
        |v| {
            let c = chosen(v);
            c.len() == 2 && tandem_cost(c[0], c[1]) <= best + 2.0
        },
    );
}

/// The two desks under a cap of 21 servers (420 at 20 a server): (13, 8) at 481.48 and (14, 7)
/// at 482.29 are 0.80 apart, so with δ = 2 either is correct.
#[test]
#[ignore = "heavy: 100 searches of the two desks under a cap"]
fn two_desks_under_a_binding_cap() {
    let best = tandem_cost(13, 8);
    assert!((tandem_cost(14, 7) - best - 0.80).abs() < 0.01);
    many(
        "two desks, cap of 21",
        SEEDS,
        || tandem("exponential", 100.0),
        |seed| {
            let mut p = tandem_search(seed, 20_000);
            p["cost_cap"] = json!(420.0);
            p
        },
        |v| {
            let c = chosen(v);
            c.len() == 2 && c[0] + c[1] <= 21 && tandem_cost(c[0], c[1]) <= best + 2.0
        },
    );
}

/// A near tie on one desk: 6 servers cost 74.24 and 7 cost 74.50. With δ = 1 either is correct;
/// with δ = 0.1 the clean-up must choose 6, or run out of budget and say "not proven".
#[test]
#[ignore = "heavy: 200 searches of one desk"]
fn a_near_tie() {
    assert!((one_desk_cost(6) - 74.24).abs() < 0.01);
    assert!((one_desk_cost(7) - 74.50).abs() < 0.01);
    let params = |delta: f64| {
        move |seed: u64| {
            json!({
                "decisions": [{ "id": "desk", "field": "capacity", "min": 5, "max": 15 }],
                "objective": {
                    "results": [{ "element": "desk", "metric": "mean_queue_length", "weight": 25.0 }],
                    "costs": [{ "id": "desk", "field": "capacity", "cost": 10.0 }]
                },
                "indifference": delta,
                "max_runs": 5000,
                "seed": seed,
            })
        }
    };
    many("near tie, δ = 1", SEEDS, one_desk, params(1.0), |v| {
        let c = chosen(v);
        c == [6] || c == [7]
    });
    many("near tie, δ = 0.1", SEEDS, one_desk, params(0.1), |v| {
        v["outcome"] == "not_proven" || (v["outcome"] == "selected" && chosen(v) == [6])
    });
}

/// Costs alone: the fewest servers at the first desk with a mean wait of at most 0.05 hour
/// (tolerance 0.005) is 14 (Erlang C: 0.0435; 13 give 0.0951).
#[test]
#[ignore = "heavy: 100 searches for the fewest servers"]
fn the_fewest_servers_for_a_mean_wait() {
    assert!((erlang_c_wait(10.0, 1.0, 14) - 0.0435).abs() < 1e-4);
    assert!((erlang_c_wait(10.0, 1.0, 13) - 0.0951).abs() < 1e-4);
    many(
        "fewest servers",
        SEEDS,
        || tandem("exponential", 100.0),
        |seed| {
            json!({
                "decisions": [{ "id": "desk1", "field": "capacity", "min": 11, "max": 40 }],
                "objective": { "costs": [{ "id": "desk1", "field": "capacity", "cost": 1.0 }] },
                "constraints": [{ "element": "desk1", "metric": "mean_wait", "at_most": 0.05, "tolerance": 0.005 }],
                "indifference": 0.5,
                "max_runs": 20_000,
                "seed": seed,
            })
        },
        |v| chosen(v) == [14],
    );
}
