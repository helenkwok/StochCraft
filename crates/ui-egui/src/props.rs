//! What the properties panel measures each field of the selection against, as established
//! simulation and design tools do: a value that differs from its reference is drawn in bold
//! with a reset beside it. The reference is the value in the scenario the opened one inherits
//! from, when the block is there, and otherwise what a new block of its kind has (or, for a
//! field a new block leaves out, the value the file leaves out). For several selected blocks,
//! the panel shows the value they share, and where they differ, that they do.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

/// What a field's value is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Against {
    /// What a new block (or flow) has.
    Default,
    /// The value in the scenario the opened one inherits from.
    Inherited,
}

/// Field `f` of a block or flow as the panel compares it: the value the file leaves out spelt
/// out (a server available outside the calendars, a flow that is not rework), and an empty list
/// as none.
pub(crate) fn normal(f: &str, v: &Value) -> Value {
    match (f, v) {
        ("available" | "transfer_holds_sender", Value::Null) => json!(true),
        (
            "idle_outside"
            | "queue_per_working_server"
            | "rework"
            | "abandoned"
            | "turned_away"
            | "balked",
            Value::Null,
        ) => json!(false),
        ("on_shift_end", Value::Null) => json!("finish"),
        ("priority", Value::Null) => json!(0.0),
        (_, Value::Array(a)) if a.is_empty() => Value::Null,
        _ => v.clone(),
    }
}

/// Whether two values are the same: numbers by value (1 and 1.0 are one number), and a key one
/// object leaves out the same as null in the other.
pub(crate) fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y))
        }
        (Value::Object(x), Value::Object(y)) => x.keys().chain(y.keys()).all(|k| {
            same(
                x.get(k).unwrap_or(&Value::Null),
                y.get(k).unwrap_or(&Value::Null),
            )
        }),
        _ => a == b,
    }
}

/// What field `f` of a block of `kind` is measured against: its value in `inherited` (the block
/// in the scenario the opened one inherits from) when there is one, else a new block's.
pub(crate) fn block_reference(kind: &str, f: &str, inherited: Option<&Value>) -> (Value, Against) {
    match inherited {
        Some(b) => (normal(f, &b[f]), Against::Inherited),
        None => (
            normal(f, &crate::editor::new_block(kind)[f]),
            Against::Default,
        ),
    }
}

/// What field `f` of a flow is measured against: its value in the inherited scenario's flow
/// when there is one, else none (no probability, no condition, not rework).
pub(crate) fn flow_reference(f: &str, inherited: Option<&Value>) -> (Value, Against) {
    match inherited {
        Some(c) => (normal(f, &c[f]), Against::Inherited),
        None => (normal(f, &Value::Null), Against::Default),
    }
}

/// Whether `current`, field `f`'s value, differs from its `reference`.
pub(crate) fn differs(f: &str, current: &Value, reference: &Value) -> bool {
    !same(&normal(f, current), reference)
}

/// The value that puts field `f` back to `reference`, as its command takes it: none of a list
/// is an empty list.
pub(crate) fn reset_value(current: &Value, reference: &Value) -> Value {
    if reference.is_null() && current.is_array() {
        json!([])
    } else {
        reference.clone()
    }
}

/// One field's values over several blocks, as the panel shows them: where they agree, the
/// value; where objects of one kind (distributions of one type, say) differ, field by field;
/// anywhere else they differ, null, with the key of that place (`key`, then `/field` for each
/// level down) added to `mixed`.
pub(crate) fn merge(values: &[Value], key: &str, mixed: &mut BTreeSet<String>) -> Value {
    let Some(first) = values.first() else {
        return Value::Null;
    };
    if values.iter().all(|v| same(v, first)) {
        return first.clone();
    }
    let one_kind = values
        .iter()
        .all(|v| v.is_object() && same(&v["type"], &first["type"]));
    if !one_kind {
        mixed.insert(key.to_owned());
        return Value::Null;
    }
    let keys: BTreeSet<&String> = values
        .iter()
        .filter_map(Value::as_object)
        .flat_map(Map::keys)
        .collect();
    let mut out = Map::new();
    for k in keys {
        let at: Vec<Value> = values.iter().map(|v| v[k.as_str()].clone()).collect();
        let merged = merge(&at, &format!("{key}/{k}"), mixed);
        out.insert(k.clone(), merged);
    }
    Value::Object(out)
}

/// One block's value `old` after an edit that turned `shown` (the merged value the panel
/// showed) into `new`: what the edit changed, and the block's own values everywhere else, so
/// setting the mean of several distributions keeps each one's other parameters. A change of an
/// object's kind (another distribution) replaces the whole value.
pub(crate) fn patch(old: &Value, shown: &Value, new: &Value) -> Value {
    if same(shown, new) {
        return old.clone();
    }
    match (old, shown, new) {
        (Value::Object(o), Value::Object(s), Value::Object(n))
            if same(
                s.get("type").unwrap_or(&Value::Null),
                n.get("type").unwrap_or(&Value::Null),
            ) =>
        {
            let mut out = o.clone();
            let keys: BTreeSet<&String> = s.keys().chain(n.keys()).collect();
            for k in keys {
                let (sv, nv) = (
                    s.get(k).unwrap_or(&Value::Null),
                    n.get(k).unwrap_or(&Value::Null),
                );
                if same(sv, nv) {
                    continue;
                }
                let v = patch(o.get(k).unwrap_or(&Value::Null), sv, nv);
                if v.is_null() {
                    out.remove(k);
                } else {
                    out.insert(k.clone(), v);
                }
            }
            Value::Object(out)
        }
        _ => new.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_left_out_is_the_default_it_stands_for() {
        assert!(!differs("available", &Value::Null, &json!(true)));
        assert!(differs("available", &json!(false), &json!(true)));
        assert!(!differs("timed", &json!([]), &Value::Null));
        assert!(!differs("capacity", &json!(1), &json!(1.0)));
        let (reference, against) = block_reference("server", "capacity", None);
        assert_eq!((reference, against), (json!(1), Against::Default));
        // A new task with a resource serves in a mean of 0.8, so a mean of 1 is a change.
        let (service, _) = block_reference("server", "service", None);
        assert!(differs(
            "service",
            &json!({"type": "exponential", "mean": 1.0}),
            &service
        ));
        assert!(!differs(
            "service",
            &json!({"type": "exponential", "mean": 0.8}),
            &service
        ));
        // A field a new block leaves out is measured against none.
        let (queue, _) = block_reference("server", "queue_capacity", None);
        assert!(queue.is_null());
        assert!(differs("queue_capacity", &json!(10), &queue));
    }

    #[test]
    fn an_inherited_value_is_the_reference_when_the_block_is_there() {
        let base = json!({"id": "serve", "kind": "server", "capacity": 1});
        let (reference, against) = block_reference("server", "capacity", Some(&base));
        assert_eq!((reference, against), (json!(1), Against::Inherited));
        assert!(differs("capacity", &json!(2), &json!(1)));
        // Not set in the inherited scenario either: the same as there.
        let (queue, _) = block_reference("server", "queue_capacity", Some(&base));
        assert!(!differs("queue_capacity", &Value::Null, &queue));
        let (rework, _) = flow_reference("rework", None);
        assert!(!differs("rework", &Value::Null, &rework));
        assert!(differs("rework", &json!(true), &rework));
    }

    #[test]
    fn a_reset_list_is_an_empty_one() {
        assert_eq!(
            reset_value(&json!([{"calendar": "c"}]), &Value::Null),
            json!([])
        );
        assert_eq!(reset_value(&json!(10), &Value::Null), Value::Null);
        assert_eq!(reset_value(&json!(3), &json!(1)), json!(1));
    }

    #[test]
    fn shared_values_show_and_differing_ones_are_marked_mixed() {
        let values = [
            json!({"type": "exponential", "mean": 1.0}),
            json!({"type": "exponential", "mean": 2.0}),
        ];
        let mut mixed = BTreeSet::new();
        let shown = merge(&values, "multi/service", &mut mixed);
        assert_eq!(shown, json!({"type": "exponential", "mean": null}));
        assert_eq!(mixed, BTreeSet::from(["multi/service/mean".to_owned()]));

        let mut mixed = BTreeSet::new();
        let kinds = [
            json!({"type": "exponential", "mean": 1.0}),
            json!({"type": "constant", "value": 1.0}),
        ];
        assert_eq!(merge(&kinds, "multi/service", &mut mixed), Value::Null);
        assert_eq!(mixed, BTreeSet::from(["multi/service".to_owned()]));

        let mut mixed = BTreeSet::new();
        assert_eq!(
            merge(&[json!(2), json!(2.0)], "multi/capacity", &mut mixed),
            json!(2)
        );
        assert!(mixed.is_empty());
    }

    #[test]
    fn an_edit_to_one_part_keeps_each_blocks_other_parts() {
        let shown = json!({"type": "normal", "mean": 5.0, "std_dev": null});
        let new = json!({"type": "normal", "mean": 6.0, "std_dev": null});
        let a = json!({"type": "normal", "mean": 5.0, "std_dev": 1.0});
        let b = json!({"type": "normal", "mean": 5.0, "std_dev": 2.0});
        assert_eq!(
            patch(&a, &shown, &new),
            json!({"type": "normal", "mean": 6.0, "std_dev": 1.0})
        );
        assert_eq!(
            patch(&b, &shown, &new),
            json!({"type": "normal", "mean": 6.0, "std_dev": 2.0})
        );
        // Another kind replaces the whole value; no edit leaves each as it was.
        let other = json!({"type": "constant", "value": 5.0});
        assert_eq!(patch(&a, &shown, &other), other);
        assert_eq!(patch(&b, &shown, &shown), b);
        // A plain value is replaced.
        assert_eq!(patch(&json!(1), &Value::Null, &json!(3)), json!(3));
    }
}
