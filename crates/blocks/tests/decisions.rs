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
/// so the gateway's default takes them: 10 parcels of size 12 besides the usual mix.
#[test]
fn inputs_no_rule_covers_are_a_warning() {
    let mut m = Builder::new();
    let classify = m.delay("classify", Dist::Constant(0.0));
    parcels(&mut m, classify);
    let medium = m.source("medium", Dist::Constant(1.0), Some(10));
    m.property(medium, "size", None, PropertyValue::Number(12.0));
    m.connect(medium, classify);
    let split = m.exclusive("split");
    m.connect(classify, split);
    let (light, heavy, unbanded) = (m.sink("light"), m.sink("heavy"), m.sink("unbanded"));
    m.route(split, light, None, Some("getProperty('band') = 'light'"));
    m.route(split, heavy, None, Some("getProperty('band') = 'heavy'"));
    m.connect(split, unbanded);
    m.default_route(split, unbanded);
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
    let r = run(&model);
    assert_eq!(entered(&r, "light"), 30);
    assert_eq!(entered(&r, "heavy"), 70);
    assert_eq!(entered(&r, "unbanded"), 10);
}

/// Gives block `id` the name `name`.
fn name(model: &mut Model, id: &str, name: &str) {
    if let Some(e) = model.elements.iter_mut().find(|e| e.id.as_str() == id) {
        e.name = name.into();
    }
}

/// A table giving `true` and `false` routes each to the block of that name, compared exactly: a
/// boolean is not equal to every other non-empty name (as XPath's `=` would have it), so the
/// first way, to "Reject", takes none.
#[test]
fn a_boolean_output_routes_by_its_own_name_only() {
    let mut m = Builder::new();
    let sort = m.exclusive("sort");
    parcels(&mut m, sort);
    let (reject, yes, no) = (m.sink("reject"), m.sink("yes"), m.sink("no"));
    m.connect(sort, reject);
    m.connect(sort, yes);
    m.connect(sort, no);
    let mut model = with_table(
        m.build(experiment()),
        "sort",
        decision(
            "small",
            HitPolicy::First,
            "small",
            &[("< 10", "true"), ("-", "false")],
        ),
    );
    name(&mut model, "reject", "Reject");
    name(&mut model, "yes", "true");
    name(&mut model, "no", "false");
    assert_eq!(model.validate(), vec![]);
    let r = run(&model);
    assert_eq!(entered(&r, "reject"), 0);
    assert_eq!(entered(&r, "yes"), 30);
    assert_eq!(entered(&r, "no"), 70);
}

/// A value that names two ways (one block's id, another's name) is an error, not a choice
/// made by the order of the ways.
#[test]
fn a_value_that_names_two_ways_is_an_error() {
    let mut m = Builder::new();
    let sort = m.exclusive("sort");
    parcels(&mut m, sort);
    let (a, b) = (m.sink("chute_a"), m.sink("chute_b"));
    m.connect(sort, a);
    m.connect(sort, b);
    let mut model = with_table(
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
    name(&mut model, "chute_b", "chute_a");
    let ds = model.validate();
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].problem.code(), "decision_unusable");
    assert!(ds[0].to_string().contains("more than one"), "{}", ds[0]);
}

/// A count routes to the way whose name reads as that number, compared as numbers, as routing
/// compares them: ways named `1` and `1.0` both name 1, which is an error, not a choice made by
/// the order of the ways.
#[test]
fn a_count_that_names_two_ways_as_numbers_is_an_error() {
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
        &[("-", "1"), ("> 10", "1")],
    );
    d.table.aggregation = Some(stochcraft_model::dmn::Aggregation::Count);
    let mut model = with_table(m.build(experiment()), "split", d);
    // Named "1.0" and "2": routed as 1 and 2.
    name(&mut model, "one", "1.0");
    name(&mut model, "two", "2");
    assert_eq!(model.validate(), vec![]);
    let r = run(&model);
    assert_eq!(entered(&r, "one"), 30);
    assert_eq!(entered(&r, "two"), 70);
    // Named "1.0" and "1": both name 1.
    name(&mut model, "two", "1");
    let ds = model.validate();
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].problem.code(), "decision_unusable");
    assert!(ds[0].to_string().contains("more than one"), "{}", ds[0]);
    assert!(compile(&model).is_err());
}

/// With an aggregation, a table gives its default as it is when no rule matches, so the default
/// must fit the property too: a COUNT into a whole-number property takes a default of 2, but not
/// 1.5 (which would be rounded) or text.
#[test]
fn an_aggregation_s_default_must_fit_its_property_s_type() {
    let mut m = Builder::new();
    let classify = m.delay("classify", Dist::Constant(0.0));
    parcels(&mut m, classify);
    let start = m.source("start", Dist::Constant(1.0), Some(1));
    m.property(
        start,
        "matches",
        Some(stochcraft_model::PropertyType::Long),
        PropertyValue::Number(0.0),
    );
    m.connect(start, classify);
    let sink = m.sink("out");
    m.connect(classify, sink);
    let mut d = decision(
        "count",
        HitPolicy::Collect,
        "matches",
        &[("> 0", "1"), ("> 10", "1")],
    );
    d.table.aggregation = Some(stochcraft_model::dmn::Aggregation::Count);
    d.table.outputs[0].default = Some("2".into());
    let model = with_table(m.build(experiment()), "classify", d);
    assert_eq!(model.validate(), vec![]);
    for default in ["1.5", "\"none\""] {
        let mut bad = model.clone();
        if let Some(t) = bad.dmn.get_mut(FILE).map(|f| &mut f.decisions[0].table) {
            t.outputs[0].default = Some(default.into());
        }
        let ds = bad.validate();
        let codes: Vec<&str> = ds.iter().map(|d| d.problem.code()).collect();
        assert_eq!(codes, vec!["decision_unusable"], "{default}: {ds:?}");
    }
}

/// A table writing text into a property declared boolean is an error: the text would be held as
/// true, and a later test for that text would never match.
#[test]
fn an_output_must_fit_its_property_s_type() {
    let mut m = Builder::new();
    let classify = m.delay("classify", Dist::Constant(0.0));
    parcels(&mut m, classify);
    let start = m.source("start", Dist::Constant(1.0), Some(1));
    m.property(
        start,
        "band",
        Some(stochcraft_model::PropertyType::Boolean),
        PropertyValue::Boolean(false),
    );
    m.connect(start, classify);
    let sink = m.sink("out");
    m.connect(classify, sink);
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
    let ds = model.validate();
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].problem.code(), "decision_unusable");
    assert!(ds[0].to_string().contains("cannot hold"), "{}", ds[0]);
    // A whole-number property takes whole numbers, not fractions.
    let mut whole = model.clone();
    for p in whole.elements.iter_mut().flat_map(|e| &mut e.properties) {
        if p.name == "band" {
            p.ty = Some(stochcraft_model::PropertyType::Long);
            p.value = PropertyValue::Number(0.0);
        }
    }
    if let Some(t) = whole.dmn.get_mut(FILE).map(|f| &mut f.decisions[0].table) {
        t.rules[0].outputs[0] = "2".into();
        t.rules[1].outputs[0] = "1".into();
    }
    assert_eq!(whole.validate(), vec![]);
    if let Some(t) = whole.dmn.get_mut(FILE).map(|f| &mut f.decisions[0].table) {
        t.rules[1].outputs[0] = "1.5".into();
    }
    let codes: Vec<&str> = whole.validate().iter().map(|d| d.problem.code()).collect();
    assert_eq!(codes, vec!["decision_unusable"]);
}

/// What a decision table writes is in its block's statistics, as an assignment's values are:
/// a score of 7 for the 70 large parcels and 3 for the 30 small ones, a mean of 5.8.
#[test]
fn a_table_s_outputs_are_in_its_block_s_statistics() {
    let mut m = Builder::new();
    let classify = m.delay("classify", Dist::Constant(0.0));
    parcels(&mut m, classify);
    let sink = m.sink("out");
    m.connect(classify, sink);
    let model = with_table(
        m.build(experiment()),
        "classify",
        decision(
            "score",
            HitPolicy::First,
            "score",
            &[(">= 10", "7"), ("-", "3")],
        ),
    );
    assert_eq!(model.validate(), vec![]);
    let r = run(&model);
    let properties = r
        .element("classify")
        .map(|e| e.properties.clone())
        .unwrap_or_default();
    assert_eq!(properties.len(), 1, "{properties:?}");
    assert_eq!(properties[0].0, "score");
    let mean = properties[0].1.unwrap_or(f64::NAN);
    assert!((mean - 5.8).abs() < 1e-12, "{mean}");
}

/// A use left behind by a deleted block (`a_old`, before `classify` in id order) gives no type to
/// what it would write: the live table's number decides that `score` is a number.
#[test]
fn a_deleted_block_s_table_gives_no_property_its_type() {
    let mut m = Builder::new();
    let classify = m.delay("classify", Dist::Constant(0.0));
    parcels(&mut m, classify);
    let sink = m.sink("out");
    m.connect(classify, sink);
    let mut model = with_table(
        m.build(experiment()),
        "classify",
        decision(
            "score",
            HitPolicy::First,
            "score",
            &[(">= 10", "7"), ("-", "3")],
        ),
    );
    let old = decision("old", HitPolicy::First, "score", &[("-", "\"high\"")]);
    if let Some(f) = model.dmn.get_mut(FILE) {
        f.decisions.push(old);
    }
    model.decisions.insert(
        ElementId::unchecked("a_old"),
        DecisionUse {
            file: FILE.into(),
            decision: "old".into(),
            route: None,
        },
    );
    assert_eq!(model.validate(), vec![]);
    assert_eq!(
        model.property_types().get("score"),
        Some(&stochcraft_model::PropertyType::Double)
    );
}
