//! Decision tables in a run (ROADMAP M5.5, D30), against counts worked by hand: two sources make
//! a known mix of parcels (30 of size 5, then 70 of size 20, each limited), so every count is
//! exact by arithmetic, with no draw involved.
#![allow(clippy::panic)]

use stochcraft_blocks::{BlockId, Builder, Report, compile};
use stochcraft_model::dmn::{
    Decision, DecisionUse, Definitions, HitPolicy, Input, Output, Rule, Table,
};
use stochcraft_model::{Dist, ElementId, Experiment, Model, PropertyValue, Severity};
use stochcraft_random::Rng;

const FILE: &str = "sorting.dmn";

/// A table reading `size`, writing `output`, with `rules` (input cell, output cell).
fn decision(id: &str, policy: HitPolicy, output: &str, rules: &[(&str, &str)]) -> Decision {
    Decision {
        id: id.into(),
        name: id.into(),
        variable: None,
        table: Table {
            hit_policy: policy,
            inputs: vec![Input {
                expression: "size".into(),
                ..Input::default()
            }],
            outputs: vec![Output {
                name: Some(output.into()),
                ..Output::default()
            }],
            rules: rules
                .iter()
                .map(|(i, o)| Rule {
                    inputs: vec![(*i).into()],
                    outputs: vec![(*o).into()],
                    ..Rule::default()
                })
                .collect(),
            ..Table::default()
        },
    }
}

/// The two sources of the known mix, both into `to`.
fn parcels(m: &mut Builder, to: BlockId) {
    for (name, size, count) in [("small", 5.0, 30), ("large", 20.0, 70)] {
        let s = m.source(name, Dist::Constant(1.0), Some(count));
        m.property(s, "size", None, PropertyValue::Number(size));
        m.connect(s, to);
    }
}

fn with_table(mut model: Model, user: &str, d: Decision) -> Model {
    model.decisions.insert(
        ElementId::unchecked(user),
        DecisionUse {
            file: FILE.into(),
            decision: d.id.clone(),
            route: None,
        },
    );
    model.dmn.insert(
        FILE.into(),
        Definitions {
            id: "sorting".into(),
            decisions: vec![d],
            ..Definitions::default()
        },
    );
    model
}

fn experiment() -> Experiment {
    Experiment {
        warmup: 0.0,
        duration: 1_000.0,
        replications: 1,
        ..Experiment::default()
    }
}

fn run(model: &Model) -> Report {
    let compiled = match compile(model) {
        Ok(c) => c,
        Err(ds) => panic!("{ds:?}"),
    };
    compiled.run(Rng::new(7)).unwrap_or_else(|e| panic!("{e}"))
}

fn entered(r: &Report, id: &str) -> u64 {
    r.element(id).map_or(0, |e| e.entered)
}

/// A divert sorting by a UNIQUE table straight to the chute it names: 30 parcels of size 5
/// go to chute_a, 70 of size 20 to chute_b, none round the loop (the chutes always have room).
#[test]
fn a_divert_routed_by_a_table_gives_the_mix_exactly() {
    let mut m = Builder::new();
    let sort = m.divert("sort");
    parcels(&mut m, sort);
    let (a, b, round) = (m.sink("chute_a"), m.sink("chute_b"), m.sink("loop"));
    m.connect(sort, a);
    m.connect(sort, b);
    m.connect(sort, round);
    m.default_route(sort, round);
    let model = with_table(
        m.build(experiment()),
        "sort",
        decision(
            "chute",
            HitPolicy::Unique,
            "chute",
            &[("< 10", "\"chute_a\""), (">= 10", "\"chute_b\"")],
        ),
    );
    assert_eq!(model.validate(), vec![]);
    let r = run(&model);
    assert_eq!(entered(&r, "chute_a"), 30);
    assert_eq!(entered(&r, "chute_b"), 70);
    assert_eq!(entered(&r, "loop"), 0);
}

/// A task (a business rule task) writes a band with a FIRST table; the gateway after it routes by
/// a condition on that band: 70 heavy, 30 light.
#[test]
fn a_task_writes_its_outputs_and_a_gateway_routes_on_them() {
    let mut m = Builder::new();
    let classify = m.delay("classify", Dist::Constant(0.0));
    parcels(&mut m, classify);
    let split = m.exclusive("split");
    m.connect(classify, split);
    let (heavy, light) = (m.sink("heavy_dock"), m.sink("light_dock"));
    m.route(split, heavy, None, Some("getProperty('band') = 'heavy'"));
    m.connect(split, light);
    m.default_route(split, light);
    let model = with_table(
        m.build(experiment()),
        "classify",
        decision(
            "band",
            HitPolicy::First,
            "band",
            &[(">= 10", "\"heavy\""), ("-", "\"light\"")],
        ),
    );
    assert_eq!(model.validate(), vec![]);
    let r = run(&model);
    assert_eq!(entered(&r, "heavy_dock"), 70);
    assert_eq!(entered(&r, "light_dock"), 30);
}

/// The same route by a gateway's own table, named by the blocks' names: COLLECT with COUNT
/// gives 2 for a size of 20 (both rules match) and 1 for a size of 5.
#[test]
fn a_gateway_routes_on_a_collected_count() {
    let mut m = Builder::new();
    let split = m.exclusive("split");
    parcels(&mut m, split);
    let (one, two) = (m.sink("one"), m.sink("two"));
    m.connect(split, one);
    m.connect(split, two);
    let mut d = decision(
        "count",
        HitPolicy::Collect,
        "matches",
        &[("> 0", "1"), ("> 10", "1")],
    );
    d.table.aggregation = Some(stochcraft_model::dmn::Aggregation::Count);
    let mut model = with_table(m.build(experiment()), "split", d);
    // Route by number: the sinks are named "1" and "2".
    for e in &mut model.elements {
        match e.id.as_str() {
            "one" => e.name = "1".into(),
            "two" => e.name = "2".into(),
            _ => {}
        }
    }
    let r = run(&model);
    assert_eq!(entered(&r, "one"), 30);
    assert_eq!(entered(&r, "two"), 70);
}

#[test]
fn an_overlapping_unique_table_does_not_run() {
    let mut m = Builder::new();
    let sort = m.divert("sort");
    parcels(&mut m, sort);
    let (a, b) = (m.sink("chute_a"), m.sink("chute_b"));
    m.connect(sort, a);
    m.connect(sort, b);
    m.default_route(sort, b);
    let model = with_table(
        m.build(experiment()),
        "sort",
        decision(
            "chute",
            HitPolicy::Unique,
            "chute",
            &[("<= 10", "\"chute_a\""), (">= 10", "\"chute_b\"")],
        ),
    );
    let Err(ds) = compile(&model) else {
        panic!("an overlapping UNIQUE table ran");
    };
    let overlap = ds
        .iter()
        .find(|d| d.problem.code() == "decision_overlap")
        .unwrap_or_else(|| panic!("{ds:?}"));
    assert_eq!(overlap.severity, Severity::Error);
    assert!(overlap.to_string().contains("size = 10"), "{overlap}");
}

#[test]
fn a_way_the_table_names_must_exist_and_a_file_must_be_read() {
    let mut m = Builder::new();
    let sort = m.exclusive("sort");
    parcels(&mut m, sort);
    let a = m.sink("chute_a");
    m.connect(sort, a);
    let model = with_table(
        m.build(experiment()),
        "sort",
        decision(
            "chute",
            HitPolicy::First,
            "chute",
            &[("< 10", "\"chute_a\""), ("-", "\"chute_z\"")],
        ),
    );
    let codes: Vec<&str> = model.validate().iter().map(|d| d.problem.code()).collect();
    assert_eq!(codes, vec!["decision_route"]);
    let mut unread = model;
    unread.dmn.clear();
    let codes: Vec<&str> = unread.validate().iter().map(|d| d.problem.code()).collect();
    assert_eq!(codes, vec!["decision_unusable"]);
}

/// A table that leaves sizes from 10 to 15 uncovered is a warning; such parcels get no band,
/// so the gateway's default takes them.
#[test]
fn inputs_no_rule_covers_are_a_warning() {
    let mut m = Builder::new();
    let classify = m.delay("classify", Dist::Constant(0.0));
    parcels(&mut m, classify);
    let sink = m.sink("out");
    m.connect(classify, sink);
    let model = with_table(
        m.build(experiment()),
        "classify",
        decision(
            "band",
            HitPolicy::Unique,
            "band",
            &[("< 10", "\"light\""), ("> 15", "\"heavy\"")],
        ),
    );
    let ds = model.validate();
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].severity, Severity::Warning);
    assert_eq!(ds[0].problem.code(), "decision_gap");
    assert!(ds[0].to_string().contains("size = 10"), "{}", ds[0]);
    assert_eq!(entered(&run(&model), "out"), 100);
}
