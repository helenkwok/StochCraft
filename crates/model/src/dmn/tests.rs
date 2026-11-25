#![allow(clippy::panic)]

//! Decision tables worked by hand: each hit policy, the checks, and what is refused.

use std::collections::BTreeMap;

use super::*;

/// A decision `d` with a table of `inputs` (property names), `outputs` (names) and `rules`
/// (input cells, then output cells).
fn table(
    policy: HitPolicy,
    inputs: &[&str],
    outputs: &[&str],
    rules: &[(&[&str], &[&str])],
) -> Decision {
    Decision {
        id: "d".into(),
        name: "d".into(),
        variable: None,
        table: Table {
            hit_policy: policy,
            inputs: inputs
                .iter()
                .map(|n| Input {
                    expression: (*n).into(),
                    ..Input::default()
                })
                .collect(),
            outputs: outputs
                .iter()
                .map(|n| Output {
                    name: Some((*n).into()),
                    ..Output::default()
                })
                .collect(),
            rules: rules
                .iter()
                .map(|(i, o)| Rule {
                    inputs: i.iter().map(|s| (*s).to_owned()).collect(),
                    outputs: o.iter().map(|s| (*s).to_owned()).collect(),
                    ..Rule::default()
                })
                .collect(),
            ..Table::default()
        },
    }
}

fn reader(pairs: &[(&str, Value)]) -> impl Fn(&str) -> Option<Value> + use<> {
    let map: BTreeMap<String, Value> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    move |name: &str| map.get(name).cloned()
}

fn num(x: f64) -> Value {
    Value::Number(x)
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

fn row(values: &[Value]) -> Outcome {
    Outcome::Row(values.iter().cloned().map(Some).collect())
}

fn compiled(d: &Decision) -> Compiled {
    match d.compile() {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    }
}

fn refused(cell: &str) -> String {
    match parse_tests(cell) {
        Ok(t) => panic!("`{cell}` was accepted as {t:?}"),
        Err(e) => e.message,
    }
}

#[test]
fn unary_tests_follow_s_feel() {
    let t = |cell: &str, v: Option<Value>| match parse_tests(cell) {
        Ok(t) => t.satisfies(v.as_ref(), None),
        Err(e) => panic!("{cell}: {e}"),
    };
    // Comparisons, and a value alone.
    assert!(t("< 5", Some(num(4.0))));
    assert!(!t("< 5", Some(num(5.0))));
    assert!(t("<= 5", Some(num(5.0))));
    assert!(t(">= -2.5", Some(num(-2.5))));
    assert!(t("> .5", Some(num(0.75))));
    assert!(t("7", Some(num(7.0))));
    assert!(t("\"gold\"", Some(text("gold"))));
    assert!(t("true", Some(Value::Boolean(true))));
    // Intervals, open and closed at either end, in both of FEEL's spellings.
    assert!(t("[1..5]", Some(num(1.0))) && t("[1..5]", Some(num(5.0))));
    assert!(!t("(1..5]", Some(num(1.0))) && !t("]1..5]", Some(num(1.0))));
    assert!(!t("[1..5)", Some(num(5.0))) && !t("[1..5[", Some(num(5.0))));
    assert!(t("]1..5[", Some(num(3.0))));
    // Lists, and not(...).
    assert!(t("\"A\", \"B\"", Some(text("B"))));
    assert!(!t("\"A\", \"B\"", Some(text("C"))));
    assert!(t("< 0, [10..20]", Some(num(15.0))));
    assert!(t("not(\"A\", \"B\")", Some(text("C"))));
    assert!(!t("not(\"A\", \"B\")", Some(text("A"))));
    assert!(t("not([1..5])", Some(num(6.0))));
    // `-` matches any value but null.
    assert!(t("-", Some(text("x"))));
    assert!(!t("-", None));
    // Kinds are never mixed: 5 is not "5", and comparing a string with a number is null.
    assert!(!t("5", Some(text("5"))));
    assert!(!t("< 5", Some(text("4"))));
    assert!(!t("not(5)", Some(text("4"))));
    // Null: no comparison holds, but `null = 1` is false, so not(1) holds.
    assert!(!t("1", None));
    assert!(!t("< 1", None));
    assert!(t("not(1)", None));
    // Escapes in strings.
    assert!(t(r#""a \"b\" é""#, Some(text("a \"b\" é"))));
    assert!(t(r#""x, y""#, Some(text("x, y"))));
}

#[test]
fn constructs_outside_the_subset_are_refused_by_name() {
    assert!(refused("").contains("write - for any value"));
    assert!(refused("x").contains("is a name"));
    assert!(refused("< limit").contains("is a name"));
    assert!(refused("date(\"2026-01-01\")").contains("dates, times and durations"));
    assert!(refused("= 5").contains("= and !="));
    assert!(refused("!= 5").contains("= and !="));
    assert!(refused("< \"M\"").contains("compares strings"));
    assert!(refused("[\"a\"..\"c\"]").contains("only ranges of numbers"));
    assert!(refused("[5..1]").contains("starts above"));
    assert!(refused("1, \"a\"").contains("mixes values"));
    assert!(refused("x + 1").contains("not in the supported subset"));
    assert!(refused("null").contains("null"));
    assert!(refused("\"open").contains("closing"));
    assert!(HitPolicy::from_name("ANY").is_err_and(|e| e.message.contains("ANY")));
    assert!(HitPolicy::from_name("RULE ORDER").is_err());
    assert!(HitPolicy::from_name("OUTPUT ORDER").is_err());
    assert!(parse_literal("a * 2").is_err());
    let mut d = table(HitPolicy::Unique, &["size"], &["c"], &[(&["< 5"], &["x"])]);
    assert!(
        d.compile()
            .is_err_and(|e| e.place == "rule 1, output 1" && e.message.contains("is a name"))
    );
    d.table.rules[0].outputs[0] = "\"x\"".into();
    d.table.inputs[0].expression = "size * 2".into();
    assert!(
        d.compile()
            .is_err_and(|e| e.message.contains("input expression"))
    );
}

/// UNIQUE: three disjoint bands of a size, worked by hand.
#[test]
fn unique_returns_the_one_rule_that_matches() {
    let d = table(
        HitPolicy::Unique,
        &["size"],
        &["band"],
        &[
            (&["< 10"], &["\"small\""]),
            (&["[10..50)"], &["\"medium\""]),
            (&[">= 50"], &["\"large\""]),
        ],
    );
    let c = compiled(&d);
    let at = |x: f64| c.evaluate(&reader(&[("size", num(x))]));
    assert_eq!(at(9.99), row(&[text("small")]));
    assert_eq!(at(10.0), row(&[text("medium")]));
    assert_eq!(at(49.9), row(&[text("medium")]));
    assert_eq!(at(50.0), row(&[text("large")]));
    // Without the property: null.
    assert_eq!(c.evaluate(&reader(&[])), Outcome::Null);
    assert_eq!(c.check(&[false]), vec![]);
}

#[test]
fn an_overlapping_unique_table_is_reported_with_an_example() {
    // [10..50] and >= 50 both hold at exactly 50.
    let d = table(
        HitPolicy::Unique,
        &["size"],
        &["band"],
        &[
            (&["< 10"], &["\"small\""]),
            (&["[10..50]"], &["\"medium\""]),
            (&[">= 50"], &["\"large\""]),
        ],
    );
    let c = compiled(&d);
    assert_eq!(
        c.check(&[false]),
        vec![Finding::Overlap {
            first: 2,
            second: 3,
            example: "size = 50".into()
        }]
    );
    // Evaluated anyway, two hits under UNIQUE give null.
    assert_eq!(c.evaluate(&reader(&[("size", num(50.0))])), Outcome::Null);
}

#[test]
fn gaps_are_found_and_whole_numbers_close_them() {
    // < 5 and >= 6 leave (5, 6) uncovered, unless the size is a whole number.
    let d = table(
        HitPolicy::Unique,
        &["size", "class"],
        &["chute"],
        &[
            (&["<= 5", "\"A\", \"B\""], &["1"]),
            (&[">= 6", "-"], &["2"]),
        ],
    );
    let c = compiled(&d);
    assert_eq!(
        c.check(&[false, false]),
        vec![Finding::Gap {
            example: "size = 4 and class = \"other\"".into()
        }]
    );
    let mut d2 = d.clone();
    d2.table.rules[0].inputs[1] = "-".into();
    assert_eq!(
        compiled(&d2).check(&[false, false]),
        vec![Finding::Gap {
            example: "size = 5.5".into()
        }]
    );
    assert_eq!(compiled(&d2).check(&[true, false]), vec![]);
    // A class's own values close the gap of "any other string".
    let mut d3 = d;
    d3.table.inputs[1].values = Some("\"A\", \"B\"".into());
    d3.table.inputs[0].type_ref = Some("integer".into());
    assert_eq!(compiled(&d3).check(&[false, false]), vec![]);
    // A default output closes every gap.
    let mut d4 = d2.clone();
    d4.table.outputs[0].default = Some("0".into());
    assert_eq!(compiled(&d4).check(&[false, false]), vec![]);
    assert_eq!(
        compiled(&d4).evaluate(&reader(&[("size", num(5.5))])),
        row(&[num(0.0)])
    );
}

/// FIRST: a catch-all last rule; the first match in rule order wins.
#[test]
fn first_returns_the_first_match_in_rule_order() {
    let d = table(
        HitPolicy::First,
        &["weight", "service"],
        &["route"],
        &[
            (&["> 30", "-"], &["\"freight\""]),
            (&["-", "\"express\""], &["\"air\""]),
            (&["-", "-"], &["\"road\""]),
        ],
    );
    let c = compiled(&d);
    let at = |w: f64, s: &str| c.evaluate(&reader(&[("weight", num(w)), ("service", text(s))]));
    // Rules 1 and 2 both match a heavy express parcel: rule 1 comes first.
    assert_eq!(at(40.0, "express"), row(&[text("freight")]));
    assert_eq!(at(2.0, "express"), row(&[text("air")]));
    assert_eq!(at(2.0, "standard"), row(&[text("road")]));
    assert_eq!(c.check(&[false, false]), vec![]);
}

/// PRIORITY: the output values list "high", "medium", "low" from highest to lowest.
#[test]
fn priority_returns_the_highest_priority_match() {
    let mut d = table(
        HitPolicy::Priority,
        &["age", "debt"],
        &["risk"],
        &[
            (&["< 25", "-"], &["\"medium\""]),
            (&["-", "> 1000"], &["\"high\""]),
            (&["-", "-"], &["\"low\""]),
        ],
    );
    d.table.outputs[0].values = Some("\"high\", \"medium\", \"low\"".into());
    let c = compiled(&d);
    let at = |a: f64, debt: f64| c.evaluate(&reader(&[("age", num(a)), ("debt", num(debt))]));
    // All three match: "high" (rule 2) outranks "medium" (rule 1) and "low" (rule 3).
    assert_eq!(at(20.0, 5000.0), row(&[text("high")]));
    assert_eq!(at(20.0, 0.0), row(&[text("medium")]));
    assert_eq!(at(40.0, 0.0), row(&[text("low")]));
    // Without output values PRIORITY cannot rank, and an output outside them is refused.
    let mut bare = d.clone();
    bare.table.outputs[0].values = None;
    assert!(
        bare.compile()
            .is_err_and(|e| e.message.contains("PRIORITY"))
    );
    let mut outside = d;
    outside.table.rules[0].outputs[0] = "\"none\"".into();
    assert!(
        outside
            .compile()
            .is_err_and(|e| e.message.contains("not one of the output's values"))
    );
}

/// COLLECT: the surcharges of every rule that applies, listed, summed, counted, and their
/// least and greatest.
#[test]
fn collect_lists_or_aggregates_every_match() {
    let mut d = table(
        HitPolicy::Collect,
        &["weight", "service"],
        &["surcharge"],
        &[
            (&["> 20", "-"], &["5"]),
            (&["-", "\"express\""], &["8"]),
            (&["> 30", "-"], &["2.5"]),
        ],
    );
    let input = reader(&[("weight", num(35.0)), ("service", text("express"))]);
    // All three rules match: 5, 8 and 2.5.
    assert_eq!(
        compiled(&d).evaluate(&input),
        Outcome::Rows(vec![vec![num(5.0)], vec![num(8.0)], vec![num(2.5)]])
    );
    assert!(compiled(&d).gives_list());
    for (a, want) in [
        (Aggregation::Sum, 15.5),
        (Aggregation::Count, 3.0),
        (Aggregation::Min, 2.5),
        (Aggregation::Max, 8.0),
    ] {
        d.table.aggregation = Some(a);
        let c = compiled(&d);
        assert_eq!(c.evaluate(&input), Outcome::Number(want), "{a:?}");
        assert_eq!(c.properties(&input), Some(vec![Some(num(want))]));
    }
    // 25 kg standard: only rule 1, 5.
    d.table.aggregation = Some(Aggregation::Sum);
    let light = reader(&[("weight", num(25.0)), ("service", text("standard"))]);
    assert_eq!(compiled(&d).evaluate(&light), Outcome::Number(5.0));
    // No match: null, not 0 (DMN 10.3.4.5).
    let none = reader(&[("weight", num(1.0)), ("service", text("standard"))]);
    assert_eq!(compiled(&d).evaluate(&none), Outcome::Null);
    // An aggregation outside COLLECT, or of strings, is refused.
    d.table.hit_policy = HitPolicy::First;
    assert!(d.compile().is_err_and(|e| e.message.contains("COLLECT")));
    d.table.hit_policy = HitPolicy::Collect;
    // Text in every rule, so that the rows agree and only the aggregation can refuse it.
    for r in &mut d.table.rules {
        r.outputs[0] = "\"x\"".into();
    }
    assert!(
        d.compile().is_err_and(|e| e
            .message
            .contains("the aggregation SUM needs number outputs")),
        "{:?}",
        d.compile()
    );
    // COUNT counts the rules, whatever they give.
    d.table.aggregation = Some(Aggregation::Count);
    assert_eq!(compiled(&d).evaluate(&input), Outcome::Number(3.0));
}

#[test]
fn outputs_are_named_by_the_decision_when_there_is_one() {
    let mut d = table(HitPolicy::Unique, &[], &["x"], &[(&[], &["1"])]);
    d.name = "Chute".into();
    d.variable = Some("chute".into());
    d.table.outputs[0].name = None;
    assert_eq!(d.output_name(0), "chute");
    let c = compiled(&d);
    assert_eq!(c.outputs(), vec![("chute", Some(Kind::Number))]);
    // With no inputs, its one rule always matches.
    assert_eq!(c.evaluate(&reader(&[])), row(&[num(1.0)]));
    assert_eq!(c.check(&[]), vec![]);
}

#[test]
fn compound_outputs_write_each_property() {
    let d = table(
        HitPolicy::Unique,
        &["zone"],
        &["chute", "priority"],
        &[
            (&["1, 2"], &["\"north\"", "1"]),
            (&["> 2"], &["\"south\"", "2"]),
        ],
    );
    let c = compiled(&d);
    assert_eq!(
        c.properties(&reader(&[("zone", num(2.0))])),
        Some(vec![Some(text("north")), Some(num(1.0))])
    );
    assert_eq!(
        c.properties(&reader(&[("zone", num(0.0))])),
        Some(vec![None, None])
    );
    assert_eq!(
        c.output_values(0),
        vec![Literal::Text("north".into()), Literal::Text("south".into())]
    );
    // The gap below 1, and between 1 and 2 for a non-integer zone.
    assert_eq!(
        c.check(&[false]),
        vec![Finding::Gap {
            example: "zone = 0".into()
        }]
    );
}

#[test]
fn rules_that_both_match_a_missing_property_overlap() {
    // Disjoint for true and for false, but `null = true` and `null = false` are both false, so
    // both `not(...)` rules match an entity without the property.
    let d = table(
        HitPolicy::Unique,
        &["approved"],
        &["route"],
        &[
            (&["not(true)"], &["\"hold\""]),
            (&["not(false)"], &["\"send\""]),
        ],
    );
    let c = compiled(&d);
    assert_eq!(c.evaluate(&reader(&[])), Outcome::Null);
    assert_eq!(
        c.check(&[false]),
        vec![Finding::Overlap {
            first: 1,
            second: 2,
            example: "approved = null".into()
        }]
    );
    // A missing property is no gap: `-` does not match it, and is not meant to.
    let open = table(HitPolicy::Unique, &["size"], &["band"], &[(&["-"], &["1"])]);
    assert_eq!(compiled(&open).check(&[false]), vec![]);
}

#[test]
fn inputs_reading_one_property_see_one_value() {
    // No size is both below 0 and 0 or more.
    let d = table(
        HitPolicy::Unique,
        &["size", "size"],
        &["band"],
        &[
            (&["< 0", "-"], &["\"negative\""]),
            (&["-", ">= 0"], &["\"other\""]),
        ],
    );
    assert_eq!(compiled(&d).check(&[false, false]), vec![]);
    // 0 is both at most 0 and at least 0; the property is named once in the example.
    let mut d2 = d;
    d2.table.rules[0].inputs[0] = "<= 0".into();
    assert_eq!(
        compiled(&d2).check(&[false, false]),
        vec![Finding::Overlap {
            first: 1,
            second: 2,
            example: "size = 0".into()
        }]
    );
}

#[test]
fn pieces_beyond_two_to_the_fifty_third_are_values_a_float_can_hold() {
    // 2⁵⁴ + 1 is 2⁵⁴ again; the next float above it, 2⁵⁴ + 4, matches the last two rules.
    let b = 18_014_398_509_481_984.0_f64;
    let d = table(
        HitPolicy::Unique,
        &["size"],
        &["band"],
        &[
            (&["< 18014398509481984"], &["1"]),
            (&[">= 18014398509481984"], &["2"]),
            (&["> 18014398509481984"], &["3"]),
        ],
    );
    let c = compiled(&d);
    let above = b.next_up();
    assert_eq!(c.evaluate(&reader(&[("size", num(above))])), Outcome::Null);
    assert_eq!(
        c.check(&[false]),
        vec![Finding::Overlap {
            first: 2,
            second: 3,
            example: format!("size = {}", value_feel(&num(above)))
        }]
    );
    // The same as whole numbers.
    assert_eq!(
        c.check(&[true]),
        vec![Finding::Overlap {
            first: 2,
            second: 3,
            example: format!("size = {}", value_feel(&num(above)))
        }]
    );
}

#[test]
fn a_table_a_block_uses_is_checked_over_its_property_s_values() {
    // As whole numbers, nothing is both at most 0.5 and at least 0.5.
    let mut d = table(
        HitPolicy::Unique,
        &["size"],
        &["band"],
        &[(&["<= 0.5"], &["1"]), (&[">= 0.5"], &["2"])],
    );
    d.table.inputs[0].type_ref = Some("integer".into());
    let c = compiled(&d);
    assert_eq!(c.check(&[false]), vec![]);
    assert_eq!(c.check_bound(&[true]), vec![]);
    // Bound to a property of type double, 0.5 can reach it, whatever the table says.
    assert_eq!(
        c.check_bound(&[false]),
        vec![Finding::Overlap {
            first: 1,
            second: 2,
            example: "size = 0.5".into()
        }]
    );
}

#[test]
fn a_bound_long_input_is_checked_over_the_numbers_a_long_holds() {
    let d = table(
        HitPolicy::Unique,
        &["size"],
        &["band"],
        &[(&["-"], &["1"]), (&["< -9223372036854775808"], &["2"])],
    );
    let c = compiled(&d);
    // Bound to a long: no long is below -2⁶³, so only `-` matches any of them.
    assert_eq!(c.check_bound(&[true]), vec![]);
    // Bound to a double, the next number below it matches both rules.
    let below = (-9_223_372_036_854_775_808.0_f64).next_down();
    assert_eq!(
        c.check_bound(&[false]),
        vec![Finding::Overlap {
            first: 1,
            second: 2,
            example: format!("size = {}", value_feel(&num(below)))
        }]
    );
}

/// A UNIQUE table with flag, `middle` inputs no rule tests, and end: rules 1 and 2 test only end,
/// rule 3 also flag, so rules 1 and 3 both match flag = false and end = true.
fn wide(middle: usize) -> Decision {
    let names: Vec<String> = std::iter::once("flag".to_owned())
        .chain((1..=middle).map(|i| format!("p{i}")))
        .chain(std::iter::once("end".to_owned()))
        .collect();
    let inputs: Vec<&str> = names.iter().map(String::as_str).collect();
    let cells = |flag: &'static str, end: &'static str| -> Vec<&'static str> {
        std::iter::once(flag)
            .chain(std::iter::repeat_n("-", middle))
            .chain(std::iter::once(end))
            .collect()
    };
    let (r1, r2, r3) = (
        cells("-", "true"),
        cells("-", "false"),
        cells("false", "true"),
    );
    let mut d = table(
        HitPolicy::Unique,
        &inputs,
        &["route"],
        &[(&r1, &["1"]), (&r2, &["2"]), (&r3, &["3"])],
    );
    for i in &mut d.table.inputs {
        i.type_ref = Some("boolean".into());
    }
    d
}

#[test]
fn a_check_stopped_by_its_limit_says_so() {
    // Small enough to look at in full: the overlap is found.
    let small = compiled(&wide(2)).check(&[]);
    assert!(
        matches!(
            small.as_slice(),
            [Finding::Overlap {
                first: 1,
                second: 3,
                ..
            }]
        ),
        "{small:?}"
    );
    // With 18 inputs between, the search starts at flag = true, where rules 1 and 2 both stay
    // open through every input, and reaches its limit before it gets to flag = false: it says
    // the table was not checked in full, rather than nothing.
    assert_eq!(compiled(&wide(18)).check(&[]), vec![Finding::Unchecked]);
}

#[test]
fn a_value_outside_an_input_s_values_is_an_error_that_gives_null() {
    let mut d = table(
        HitPolicy::Unique,
        &["size"],
        &["band"],
        &[(&["[0..20]"], &["1"]), (&["> 10"], &["2"])],
    );
    d.table.inputs[0].values = Some("[0..10]".into());
    d.table.outputs[0].default = Some("0".into());
    let c = compiled(&d);
    let at = |x: f64| c.evaluate(&reader(&[("size", num(x))]));
    // Values the input allows: as before.
    assert_eq!(at(5.0), row(&[num(1.0)]));
    assert_eq!(at(10.0), row(&[num(1.0)]));
    // Others are an error: null, not the rule they match, nor the default.
    assert_eq!(at(25.0), Outcome::Null);
    assert_eq!(at(15.0), Outcome::Null);
    assert_eq!(at(-5.0), Outcome::Null);
    assert_eq!(
        c.properties(&reader(&[("size", num(25.0))])),
        Some(vec![None])
    );
    // A missing property is no value outside them: no rule matches, so the default.
    assert_eq!(c.evaluate(&reader(&[])), row(&[num(0.0)]));
    // Over the values the input allows, no rules overlap.
    assert_eq!(c.check(&[false]), vec![]);
}
