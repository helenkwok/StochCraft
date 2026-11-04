//! Starting models for `stochcraft-cli new`, written as engine commands so they are built the
//! same way any other model is.

use serde_json::{Value, json};

/// A template: its name, what it is, and the commands that build it.
pub struct Template {
    pub name: &'static str,
    pub summary: &'static str,
    pub steps: fn() -> Vec<(&'static str, Value)>,
}

fn exp(mean: f64) -> Value {
    json!({"type": "exponential", "mean": mean})
}

fn mm1() -> Vec<(&'static str, Value)> {
    vec![
        ("model.rename", json!({"name": "M/M/1"})),
        (
            "block.add",
            json!({"kind": "source", "id": "arrivals", "name": "Arrivals", "interarrival": exp(1.0 / 0.75)}),
        ),
        (
            "block.add",
            json!({"kind": "server", "id": "server", "name": "Server", "capacity": 1, "service": exp(1.0)}),
        ),
        (
            "block.add",
            json!({"kind": "sink", "id": "exit", "name": "Exit"}),
        ),
        ("connect", json!({"from": "arrivals", "to": "server"})),
        ("connect", json!({"from": "server", "to": "exit"})),
        (
            "experiment.set",
            json!({"warmup": 1000.0, "duration": 20000.0, "replications": 10, "seed": 1}),
        ),
    ]
}

fn mm2() -> Vec<(&'static str, Value)> {
    let mut steps = mm1();
    steps[0] = ("model.rename", json!({"name": "M/M/2"}));
    steps[1] = (
        "block.add",
        json!({"kind": "source", "id": "arrivals", "name": "Arrivals", "interarrival": exp(1.0 / 1.5)}),
    );
    steps[2] = (
        "block.add",
        json!({"kind": "server", "id": "server", "name": "Servers", "capacity": 2, "service": exp(1.0)}),
    );
    steps
}

fn tandem() -> Vec<(&'static str, Value)> {
    vec![
        ("model.rename", json!({"name": "Tandem line"})),
        (
            "block.add",
            json!({"kind": "source", "id": "parts", "name": "Parts", "interarrival": exp(1.0)}),
        ),
        (
            "block.add",
            json!({"kind": "server", "id": "cut", "name": "Cut", "capacity": 1,
                   "service": {"type": "triangular", "min": 0.5, "mode": 0.7, "max": 1.0}}),
        ),
        (
            "block.add",
            json!({"kind": "delay", "id": "conveyor", "name": "Conveyor",
                   "duration": {"type": "constant", "value": 2.0}}),
        ),
        (
            "block.add",
            json!({"kind": "server", "id": "weld", "name": "Weld", "capacity": 1,
                   "service": {"type": "erlang", "mean": 0.8, "k": 3}}),
        ),
        (
            "block.add",
            json!({"kind": "sink", "id": "shipped", "name": "Shipped"}),
        ),
        ("connect", json!({"from": "parts", "to": "cut"})),
        ("connect", json!({"from": "cut", "to": "conveyor"})),
        ("connect", json!({"from": "conveyor", "to": "weld"})),
        ("connect", json!({"from": "weld", "to": "shipped"})),
        (
            "experiment.set",
            json!({"warmup": 100.0, "duration": 2000.0, "replications": 10, "seed": 1}),
        ),
    ]
}

pub static TEMPLATES: &[Template] = &[
    Template {
        name: "mm1",
        summary: "M/M/1 queue: arrivals 0.75 per minute, one server with mean 1 minute (W = 4)",
        steps: mm1,
    },
    Template {
        name: "mm2",
        summary: "M/M/2 queue: arrivals 1.5 per minute, two servers with mean 1 minute",
        steps: mm2,
    },
    Template {
        name: "tandem",
        summary: "Tandem line: cut, a 2-minute conveyor and weld, in minutes",
        steps: tandem,
    },
];
