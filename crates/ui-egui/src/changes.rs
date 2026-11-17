//! What changed in a model between two runs, in the window's words: "Serve › Capacity: 1 → 2",
//! "Added Inspect", "Experiment › Runs: 10 → 20". Read from the two models as the app keeps
//! results against them (the engine's wire form without where blocks are drawn), so an edit
//! that was undone, or made and then reversed, is not listed.

use serde_json::{Map, Value};

use crate::form;
use crate::i18n::{t, tf};
use crate::results;

/// The changes from model `before` to model `after`, blocks first, then connections, pools,
/// calendars and the experiment.
pub fn changes(before: &Value, after: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let name_of = |m: &Value, id: &str| -> String {
        by_id(m, "elements", id)
            .and_then(|e| e["name"].as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or(id)
            .to_owned()
    };
    // Blocks.
    for e in list(after, "elements") {
        let id = e["id"].as_str().unwrap_or("");
        let name = name_of(after, id);
        match by_id(before, "elements", id) {
            None => out.push(tf("Added {block}", &[("block", &name)])),
            Some(b) => {
                let old = name_of(before, id);
                if old != name {
                    out.push(tf(
                        "Renamed {before} to {after}",
                        &[("before", &old), ("after", &name)],
                    ));
                }
                fields(&name, b, e, &["id", "name", "kind", "bounds"], &mut out);
            }
        }
    }
    for b in list(before, "elements") {
        let id = b["id"].as_str().unwrap_or("");
        if by_id(after, "elements", id).is_none() {
            out.push(tf("Removed {block}", &[("block", &name_of(before, id))]));
        }
    }
    // Connections, named by the blocks at their ends.
    let ends = |m: &Value, c: &Value| {
        let end = |k: &str| {
            let id = c[k].as_str().unwrap_or("");
            let name = name_of(m, id);
            if name.is_empty() { id.to_owned() } else { name }
        };
        (end("from"), end("to"))
    };
    for c in list(after, "connections") {
        let id = c["id"].as_str().unwrap_or("");
        let (from, to) = ends(after, c);
        match by_id(before, "connections", id) {
            None => out.push(tf(
                "Connected {from} to {to}",
                &[("from", &from), ("to", &to)],
            )),
            Some(b) => {
                let prefix = format!("{from} \u{2192} {to}");
                fields(&prefix, b, c, &["id", "from", "to"], &mut out);
            }
        }
    }
    for c in list(before, "connections") {
        let id = c["id"].as_str().unwrap_or("");
        if by_id(after, "connections", id).is_none() {
            let (from, to) = ends(before, c);
            out.push(tf(
                "Disconnected {from} from {to}",
                &[("from", &from), ("to", &to)],
            ));
        }
    }
    // Pools and calendars, by their names.
    for (key, kind) in [("resources", t("Pool")), ("calendars", t("Calendar"))] {
        for x in list(after, key) {
            let id = x["id"].as_str().unwrap_or("");
            let name = x["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id);
            let prefix = format!("{kind} {name}");
            match by_id(before, key, id) {
                None => out.push(tf("Added {block}", &[("block", &prefix)])),
                Some(b) => fields(&prefix, b, x, &["id"], &mut out),
            }
        }
        for x in list(before, key) {
            let id = x["id"].as_str().unwrap_or("");
            if by_id(after, key, id).is_none() {
                let name = x["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id);
                out.push(tf(
                    "Removed {block}",
                    &[("block", &format!("{kind} {name}"))],
                ));
            }
        }
    }
    fields(
        t("Experiment"),
        &before["experiment"],
        &after["experiment"],
        &[],
        &mut out,
    );
    out
}

fn list<'a>(m: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    m[key].as_array().into_iter().flatten()
}

fn by_id<'a>(m: &'a Value, key: &str, id: &str) -> Option<&'a Value> {
    list(m, key).find(|x| x["id"] == id)
}

/// Each field that differs between `before` and `after` (objects), but those in `skip`, as a
/// line under `prefix`.
fn fields(prefix: &str, before: &Value, after: &Value, skip: &[&str], out: &mut Vec<String>) {
    let empty = Map::new();
    let (a, b) = (
        before.as_object().unwrap_or(&empty),
        after.as_object().unwrap_or(&empty),
    );
    let mut keys: Vec<&String> = b.keys().collect();
    keys.extend(a.keys().filter(|k| !b.contains_key(*k)));
    for k in keys {
        if skip.contains(&k.as_str()) {
            continue;
        }
        let (x, y) = (
            a.get(k).unwrap_or(&Value::Null),
            b.get(k).unwrap_or(&Value::Null),
        );
        if x != y {
            out.push(field(prefix, k, x, y));
        }
    }
}

/// One changed field: its old and new value where they are plain, or the one part of a
/// distribution that changed, or else only that it changed.
fn field(prefix: &str, key: &str, before: &Value, after: &Value) -> String {
    let label = form::label(key);
    if let (Some(x), Some(y)) = (plain(before), plain(after)) {
        return line(prefix, &label, &x, &y);
    }
    // A distribution or another object of the same type with one field changed.
    if let (Some(x), Some(y)) = (before.as_object(), after.as_object()) {
        if x.get("type") != y.get("type") {
            if let (Some(a), Some(b)) = (x["type"].as_str(), y["type"].as_str()) {
                return line(
                    prefix,
                    &label,
                    &form::choice_label(a),
                    &form::choice_label(b),
                );
            }
        } else {
            let differ: Vec<&String> = y
                .keys()
                .chain(x.keys().filter(|k| !y.contains_key(*k)))
                .filter(|k| x.get(*k) != y.get(*k))
                .collect();
            if let [k] = differ.as_slice() {
                let (a, b) = (x.get(*k).and_then(plain), y.get(*k).and_then(plain));
                if let (Some(a), Some(b)) = (a, b) {
                    let part = tf(
                        "{field}, {part}",
                        &[("field", &label), ("part", &form::label(k))],
                    );
                    return line(prefix, &part, &a, &b);
                }
            }
        }
    }
    tf(
        "{block} \u{203a} {field} changed",
        &[("block", prefix), ("field", &label)],
    )
}

fn line(prefix: &str, field: &str, before: &str, after: &str) -> String {
    tf(
        "{block} \u{203a} {field}: {before} \u{2192} {after}",
        &[
            ("block", prefix),
            ("field", field),
            ("before", before),
            ("after", after),
        ],
    )
}

/// A plain value as the window writes it: a number in the user's region, a choice by its label,
/// none as "–". `None` for a list or an object.
fn plain(v: &Value) -> Option<String> {
    match v {
        Value::Null => Some("–".to_owned()),
        Value::Number(n) => n.as_f64().map(results::grouped),
        Value::String(s) => Some(form::choice_label(s)),
        Value::Bool(true) => Some(t("on").to_owned()),
        Value::Bool(false) => Some(t("off").to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model() -> Value {
        json!({
            "name": "Line",
            "elements": [
                { "id": "in", "name": "Parts arrive", "kind": "source",
                  "interarrival": { "type": "exponential", "mean": 6.0 } },
                { "id": "paint", "name": "Paint", "kind": "server", "capacity": 1,
                  "service": { "type": "exponential", "mean": 5.2 } },
                { "id": "out", "name": "Shipped", "kind": "sink" }
            ],
            "connections": [
                { "id": "c1", "from": "in", "to": "paint" },
                { "id": "c2", "from": "paint", "to": "out" }
            ],
            "experiment": { "time_unit": "min", "warmup": 0.0, "duration": 480.0,
                            "replications": 10, "seed": 7 }
        })
    }

    #[test]
    fn nothing_changed_lists_nothing() {
        assert!(changes(&model(), &model()).is_empty());
    }

    #[test]
    fn a_change_names_the_block_the_field_and_both_values() {
        let mut after = model();
        after["elements"][1]["service"]["mean"] = json!(4.6);
        after["elements"][1]["capacity"] = json!(2);
        after["experiment"]["replications"] = json!(20);
        let c = changes(&model(), &after);
        assert_eq!(
            c,
            [
                "Paint \u{203a} Capacity: 1 \u{2192} 2",
                "Paint \u{203a} Service time, Mean: 5.2 \u{2192} 4.6",
                "Experiment \u{203a} Runs: 10 \u{2192} 20",
            ]
        );
    }

    #[test]
    fn blocks_and_connections_added_or_removed_are_named() {
        let mut after = model();
        after["elements"][1]["name"] = json!("Paint booth");
        after["elements"][1]["service"] = json!({ "type": "constant", "value": 5.0 });
        if let Some(list) = after["elements"].as_array_mut() {
            list.push(json!({ "id": "check", "name": "Inspect", "kind": "delay" }));
            list.remove(0);
        }
        if let Some(list) = after["connections"].as_array_mut() {
            list.remove(0);
            list.push(json!({ "id": "c3", "from": "paint", "to": "check" }));
        }
        let c = changes(&model(), &after);
        assert!(
            c.contains(&"Renamed Paint to Paint booth".to_owned()),
            "{c:?}"
        );
        assert!(
            c.contains(
                &"Paint booth \u{203a} Service time: Exponential \u{2192} Constant".to_owned()
            ),
            "{c:?}"
        );
        assert!(c.contains(&"Added Inspect".to_owned()), "{c:?}");
        assert!(c.contains(&"Removed Parts arrive".to_owned()), "{c:?}");
        assert!(
            c.contains(&"Connected Paint booth to Inspect".to_owned()),
            "{c:?}"
        );
        assert!(
            c.contains(&"Disconnected Parts arrive from Paint".to_owned()),
            "{c:?}"
        );
    }
}
