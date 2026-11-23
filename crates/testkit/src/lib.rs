//! StochCraft's test tooling: the golden models and their pinned results.
//!
//! Each golden model is run through the engine (as the command line and the app run it) with a
//! fixed number of replications, and its results are reduced to a digest. The digests are
//! pinned in `tests/golden/results.txt`; the `golden` test checks them on whatever target it is
//! compiled for, natively and as `wasm32-wasip1` under a WASI runtime (`cargo xtask golden`),
//! so a seeded model gives exactly the same numbers everywhere.
//!
//! When a change is meant to alter results, `STOCHCRAFT_BLESS=1 cargo test -p
//! stochcraft-testkit` rewrites the pinned digests (natively), and the diff shows which models
//! changed.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use stochcraft_engine::Engine;

/// A golden model: its file name under `tests/models/` and its bytes, built into the test so it
/// needs no file system (a WASI runtime need not grant one), with the files it names beside it
/// (an agent-based model's state charts), by their references.
pub struct Golden {
    pub name: &'static str,
    pub bytes: &'static [u8],
    pub files: &'static [(&'static str, &'static [u8])],
}

pub static GOLDEN: &[Golden] = &[
    Golden {
        name: "mm1.bpmn",
        bytes: include_bytes!("../../../tests/models/mm1.bpmn"),
        files: &[],
    },
    Golden {
        name: "tandem.xpdl",
        bytes: include_bytes!("../../../tests/models/tandem.xpdl"),
        files: &[],
    },
    Golden {
        name: "distributions.bpmn",
        bytes: include_bytes!("../../../tests/models/distributions.bpmn"),
        files: &[],
    },
    // Gateways with conditions, a parallel split and join, entity properties, a shared pool and
    // a finite buffer.
    Golden {
        name: "repair-shop.bpmn",
        bytes: include_bytes!("../../../tests/models/repair-shop.bpmn"),
        files: &[],
    },
    // A task whose two pools serve in times of their own (a service time by resource), with
    // whole-minute times, so units come free together and arrivals meet them.
    Golden {
        name: "able-baker.bpmn",
        bytes: include_bytes!("../../../tests/models/able-baker.bpmn"),
        files: &[],
    },
    // An epidemic among 100 people as agents (ROADMAP M4.3): a population whose SCXML state
    // chart sends messages to random others at a rate, with counts over time.
    Golden {
        name: "sir-agents.bpmn",
        bytes: include_bytes!("../../../tests/models/sir-agents.bpmn"),
        files: &[(
            "sir-person.scxml",
            include_bytes!("../../../tests/models/sir-person.scxml"),
        )],
    },
];

/// Replications per golden run: enough to exercise the experiment, few enough to be quick on a
/// slow runtime.
pub const REPLICATIONS: u32 = 4;

/// The pinned digests, as `tests/golden/results.txt` holds them.
pub const PINNED: &str = include_str!("../../../tests/golden/results.txt");

/// Runs a golden model with its own scenario (seed, warm-up, duration) and
/// [`REPLICATIONS`] replications, and returns the results: `run`'s for a process model,
/// `agents.run`'s for an agent-based one.
pub fn run(g: &Golden) -> Result<Value, String> {
    let mut engine = Engine::new();
    let opened = engine
        .open_bytes_with(g.name, g.bytes, None, &mut |href: &str| {
            g.files
                .iter()
                .find(|(name, _)| *name == href)
                .map(|(_, bytes)| bytes.to_vec())
                .ok_or_else(|| format!("`{href}` is not among the golden model's files"))
        })
        .map_err(|e| format!("{}: {e}", g.name))?;
    let command = if opened["kind"] == "agents" {
        "agents.run"
    } else {
        "run"
    };
    engine
        .execute(command, json!({ "replications": REPLICATIONS }))
        .map_err(|e| format!("{}: {e}", g.name))
}

/// The digest of a run's results: every element's statistics, exactly (JSON prints each number
/// so that it reads back to the same bits), or for an agent-based model the times, every
/// statistic over time and every population's counts. The run record is left out: it names the
/// StochCraft version, which changes without the results changing.
pub fn digest(results: &Value) -> String {
    let part = match results.get("elements") {
        Some(elements) => elements.clone(),
        None => json!({
            "times": results["times"],
            "statistics": results["statistics"],
            "populations": results["populations"],
        }),
    };
    let text = serde_json::to_string(&part).unwrap_or_default();
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A search of whole-number decisions (`experiment.optimise`) on a small queue: one desk,
/// arrivals 4 an hour, mean service 1 hour, 20 hours measured, its servers from 1 to 80 (more
/// than the 50 designs that skip the search, so COMPASS runs), with a small budget.
pub fn optimise() -> Result<Value, String> {
    let mut engine = Engine::new();
    let model = json!({ "model": {
        "name": "One desk",
        "elements": [
            { "id": "arrivals", "kind": "source", "interarrival": { "type": "exponential", "mean": 0.25 } },
            { "id": "desk", "kind": "server", "capacity": 9, "service": { "type": "exponential", "mean": 1.0 } },
            { "id": "exit", "kind": "sink" }
        ],
        "connections": [
            { "id": "f1", "from": "arrivals", "to": "desk" },
            { "id": "f2", "from": "desk", "to": "exit" }
        ],
        "experiment": { "time_unit": "hour", "warmup": 2.0, "duration": 20.0, "replications": 4, "seed": 2026 }
    }});
    engine
        .execute("model.set", model)
        .map_err(|e| format!("optimise: {e}"))?;
    engine
        .execute(
            "experiment.optimise",
            json!({
                "decisions": [{ "id": "desk", "field": "capacity", "min": 1, "max": 80 }],
                "objective": {
                    "results": [{ "element": "desk", "metric": "mean_queue_length", "weight": 25.0 }],
                    "costs": [{ "id": "desk", "field": "capacity", "cost": 10.0 }]
                },
                "indifference": 1.0,
                "max_runs": 400,
                "starts": 1,
            }),
        )
        .map_err(|e| format!("optimise: {e}"))
}

/// A search of a rule (`experiment.optimise`, stage O4): two M/M/1 servers behind a gateway, its
/// routing a random split or the shortest queue, beside the first server's units from 1 to 60
/// (more than the 50 designs that skip the search, so COMPASS runs from a start at each routing).
pub fn optimise_rules() -> Result<Value, String> {
    let exp = |mean: f64| json!({ "type": "exponential", "mean": mean });
    let mut engine = Engine::new();
    let model = json!({ "model": {
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
        "experiment": { "warmup": 5.0, "duration": 50.0, "replications": 4, "seed": 2026 }
    }});
    engine
        .execute("model.set", model)
        .map_err(|e| format!("optimise rules: {e}"))?;
    engine
        .execute(
            "experiment.optimise",
            json!({
                "decisions": [
                    { "id": "split", "field": "routing" },
                    { "id": "s1", "field": "capacity", "min": 1, "max": 60 }
                ],
                "objective": {
                    "results": [{ "element": "exit", "metric": "mean_time_in_system" }],
                    "costs": [{ "id": "s1", "field": "capacity", "cost": 0.1 }]
                },
                "indifference": 0.2,
                "max_runs": 400,
                "starts": 1,
            }),
        )
        .map_err(|e| format!("optimise rules: {e}"))
}

/// A search of a mean (`experiment.optimise`, stage O4): an M/M/1 server's mean service time from
/// 0.2 to 0.9, its mean number inside plus −4 per unit of the mean, by Nelder–Mead.
pub fn optimise_mean() -> Result<Value, String> {
    let exp = |mean: f64| json!({ "type": "exponential", "mean": mean });
    let mut engine = Engine::new();
    let model = json!({ "model": {
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
        "experiment": { "warmup": 5.0, "duration": 50.0, "replications": 4, "seed": 2026 }
    }});
    engine
        .execute("model.set", model)
        .map_err(|e| format!("optimise mean: {e}"))?;
    engine
        .execute(
            "experiment.optimise",
            json!({
                "decisions": [{ "id": "server", "field": "mean", "min": 0.2, "max": 0.9 }],
                "objective": {
                    "results": [{ "element": "server", "metric": "mean_work_in_progress" }],
                    "costs": [{ "id": "server", "field": "mean", "cost": -4.0 }]
                },
                "indifference": 0.05,
                "max_runs": 400,
                "replications_per_visit": 4,
            }),
        )
        .map_err(|e| format!("optimise mean: {e}"))
}

/// The digest of a whole answer, exactly.
pub fn digest_all(answer: &Value) -> String {
    let text = serde_json::to_string(answer).unwrap_or_default();
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Every golden model's digest, as lines of `name digest`, sorted by name, and the search's.
pub fn digests() -> Result<String, String> {
    let mut lines = Vec::new();
    for g in GOLDEN {
        lines.push(format!("{} {}", g.name, digest(&run(g)?)));
    }
    lines.push(format!("optimise {}", digest_all(&optimise()?)));
    lines.push(format!("optimise-rules {}", digest_all(&optimise_rules()?)));
    lines.push(format!("optimise-mean {}", digest_all(&optimise_mean()?)));
    lines.sort();
    Ok(lines.join("\n") + "\n")
}

/// The pinned digests without comments and blank lines.
pub fn pinned() -> String {
    let lines: Vec<&str> = PINNED
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    lines.join("\n") + "\n"
}
