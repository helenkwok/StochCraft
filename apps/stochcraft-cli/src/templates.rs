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

/// Schmitt et al. (2026), Case A (Table 4): four machines in series with failures, buffers of 5
/// with a 10-second transfer, weekend breaks and power per machine, in seconds from Monday
/// 00:00. Material is always there for the first machine: a supply buffer kept full feeds it
/// as soon as it has room.
fn case_a() -> Vec<(&'static str, Value)> {
    // (id, name, cycle time, availability, mean time to repair), Table 4.
    let machines = [
        ("m1", "SV36262", 320.0, 0.85, 1409.0),
        ("m2", "SV32173", 290.0, 0.85, 718.0),
        ("m3", "SV91991", 134.0, 0.98, 6717.0),
        ("m4", "SV91989", 41.0, 0.99, 671.0),
    ];
    let weekly =
        |start: &str, end: &str| json!({"start": start, "end": end, "repeat": "FREQ=WEEKLY"});
    let mut steps = vec![
        (
            "model.rename",
            json!({"name": "Case A (Schmitt et al. 2026)"}),
        ),
        (
            "calendar.add",
            json!({"id": "weekend", "name": "Weekend breaks", "events": [
                weekly("2024-01-05T17:00:00", "2024-01-06T07:00:00"),
                weekly("2024-01-06T17:00:00", "2024-01-07T07:00:00"),
            ]}),
        ),
        (
            "block.add",
            json!({"kind": "source", "id": "supply", "name": "Supply",
                   "interarrival": {"type": "constant", "value": 60.0}}),
        ),
        (
            "block.add",
            json!({"kind": "server", "id": "raw", "name": "Raw material", "capacity": 1,
                   "service": {"type": "constant", "value": 0.0}, "queue_capacity": 5}),
        ),
        ("connect", json!({"from": "supply", "to": "raw"})),
    ];
    let mut from = "raw";
    for (i, (id, name, cycle, a, mttr)) in machines.into_iter().enumerate() {
        let mut block = json!({
            "kind": "server", "id": id, "name": name, "capacity": 1,
            "service": {"type": "constant", "value": cycle},
            "queue_capacity": if i == 0 { 0 } else { 5 },
            "breakdown": {
                "time_to_failure": exp(mttr * a / (1.0 - a)),
                "repair": exp(mttr),
                "interruptible": true,
            },
            "availability": [{"calendar": "weekend", "available": false}],
            "power": {"processing": 50.0, "idle": 10.0},
        });
        if i > 0 {
            block["transfer"] = json!({"type": "constant", "value": 10.0});
        }
        steps.push(("block.add", block));
        steps.push(("connect", json!({"from": from, "to": id})));
        from = id;
    }
    steps.push((
        "block.add",
        json!({"kind": "sink", "id": "done", "name": "Finished parts"}),
    ));
    steps.push(("connect", json!({"from": from, "to": "done"})));
    steps.push((
        "experiment.set",
        json!({"time_unit": "s", "warmup": 86400.0, "duration": 2505600.0,
               "replications": 25, "seed": 1, "start": "2024-01-01T00:00:00"}),
    ));
    steps
}

/// Schmitt et al. (2026), Case B (Table 6): an automated assembly line of seven machines,
/// two presses in parallel, 8.9% rejected at quality, in seconds from Monday 00:00. Buffers of
/// 2 with 10 s and of 3 with 32 s, each a minimum time in the buffer (the sending machine is
/// free once the part is in); parts go to the press whose buffer has the shorter queue. The
/// availabilities are those of the authors' data file, which Table 6 rounds. Material is always
/// there for loading, and every machine follows the weekend breaks of Case A.
fn case_b() -> Vec<(&'static str, Value)> {
    // (id, name, cycle time, availability, mean time to repair, idle kW, processing kW,
    // buffer, transfer s); the conveyor never fails.
    let machines = [
        ("loading", "Loading", 12.0, 0.9049, 68.0, 0.25, 0.72, 0, 0.0),
        ("conveyor", "Conveyor", 6.0, 1.0, 0.0, 0.0, 0.0, 2, 10.0),
        (
            "washing", "Washing", 14.0, 0.8089, 269.0, 4.28, 35.24, 2, 10.0,
        ),
        (
            "handling", "Handling", 25.0, 0.9779, 74.0, 0.5, 0.74, 2, 10.0,
        ),
        (
            "press1", "Press 1", 175.0, 0.8779, 73.0, 1.25, 1.28, 3, 32.0,
        ),
        (
            "press2", "Press 2", 176.0, 0.8769, 74.0, 1.25, 1.27, 3, 32.0,
        ),
        (
            "quality", "Quality", 41.0, 0.8587, 66.0, 0.58, 0.84, 3, 32.0,
        ),
    ];
    let weekly =
        |start: &str, end: &str| json!({"start": start, "end": end, "repeat": "FREQ=WEEKLY"});
    let mut steps = vec![
        (
            "model.rename",
            json!({"name": "Case B (Schmitt et al. 2026)"}),
        ),
        (
            "calendar.add",
            json!({"id": "weekend", "name": "Weekend breaks", "events": [
                weekly("2024-01-05T17:00:00", "2024-01-06T07:00:00"),
                weekly("2024-01-06T17:00:00", "2024-01-07T07:00:00"),
            ]}),
        ),
        (
            "block.add",
            json!({"kind": "source", "id": "supply", "name": "Supply",
                   "interarrival": {"type": "constant", "value": 2.0}}),
        ),
        (
            "block.add",
            json!({"kind": "server", "id": "raw", "name": "Raw material", "capacity": 1,
                   "service": {"type": "constant", "value": 0.0}, "queue_capacity": 5}),
        ),
        ("connect", json!({"from": "supply", "to": "raw"})),
        (
            "block.add",
            json!({"kind": "exclusive_gateway", "id": "route", "name": "To a press",
                   "routing": "shortest_queue"}),
        ),
        (
            "block.add",
            json!({"kind": "exclusive_gateway", "id": "inspect", "name": "Accepted?"}),
        ),
        (
            "block.add",
            json!({"kind": "sink", "id": "done", "name": "Accepted parts"}),
        ),
        (
            "block.add",
            json!({"kind": "sink", "id": "scrap", "name": "Rejected parts"}),
        ),
    ];
    for (id, name, cycle, a, mttr, idle, processing, buffer, transfer) in machines {
        let mut block = json!({
            "kind": "server", "id": id, "name": name, "capacity": 1,
            "service": {"type": "constant", "value": cycle},
            "queue_capacity": buffer,
            "availability": [{"calendar": "weekend", "available": false}],
            "power": {"processing": processing, "idle": idle},
        });
        if a < 1.0 {
            block["breakdown"] = json!({
                "time_to_failure": exp(mttr * a / (1.0 - a)),
                "repair": exp(mttr),
                "interruptible": true,
            });
        }
        if transfer > 0.0 {
            block["transfer"] = json!({"type": "constant", "value": transfer});
            block["transfer_holds_sender"] = json!(false);
        }
        steps.push(("block.add", block));
    }
    for (from, to) in [
        ("raw", "loading"),
        ("loading", "conveyor"),
        ("conveyor", "washing"),
        ("washing", "handling"),
        ("handling", "route"),
        ("route", "press1"),
        ("route", "press2"),
        ("press1", "quality"),
        ("press2", "quality"),
        ("quality", "inspect"),
    ] {
        steps.push(("connect", json!({"from": from, "to": to})));
    }
    steps.push((
        "connect",
        json!({"from": "inspect", "to": "done", "probability": 0.911}),
    ));
    steps.push((
        "connect",
        json!({"from": "inspect", "to": "scrap", "probability": 0.089}),
    ));
    steps.push((
        "experiment.set",
        json!({"time_unit": "s", "warmup": 86400.0, "duration": 604800.0,
               "replications": 25, "seed": 1, "start": "2024-01-01T00:00:00"}),
    ));
    steps
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
    Template {
        name: "case-a",
        summary: "Case A of Schmitt et al. (2026): four machines with failures, buffers of 5 with a 10 s transfer, weekend breaks and energy, in seconds",
        steps: case_a,
    },
    Template {
        name: "case-b",
        summary: "Case B of Schmitt et al. (2026): seven machines, two presses in parallel fed by the shorter queue, 8.9% rejects, minimum times in the buffers, weekend breaks and energy, in seconds",
        steps: case_b,
    },
];
