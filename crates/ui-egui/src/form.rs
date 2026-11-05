//! Forms generated from the engine's command schemas.
//!
//! A command's parameters come with a JSON schema (schemars, draft 2020-12). This module draws an
//! editor for one parameter from its schema and current value: numbers and integers as text
//! fields that commit on Enter or when they lose focus, booleans as checkboxes, string enums and
//! tagged unions (`oneOf` with a constant `type`, such as distributions) as choices, arrays of
//! objects as small tables. An edit returns the parameter's new value, which the caller sends as
//! one command, so one edit is one undo step. Text that does not parse stays in its field,
//! marked, until it is fixed or Esc restores the value.
//!
//! Because the forms follow the schemas, a parameter the engine gains appears here without
//! interface code of its own.

use std::collections::HashMap;

use egui::RichText;
use serde_json::{Map, Value, json};

use crate::theme;

/// A command's parameter schema, with `$ref`s resolved against its `$defs`.
#[derive(Clone, Copy)]
pub struct Schema<'a> {
    root: &'a Value,
}

impl<'a> Schema<'a> {
    pub fn new(root: &'a Value) -> Self {
        Self { root }
    }

    /// The schema of property `name` of the root object.
    pub fn property(&self, name: &str) -> Option<&'a Value> {
        self.root["properties"].get(name)
    }

    /// The root's description, if any.
    pub fn description(&self, node: &'a Value) -> Option<&'a str> {
        node["description"].as_str()
    }

    fn resolve(&self, node: &'a Value) -> &'a Value {
        match node["$ref"]
            .as_str()
            .and_then(|r| r.strip_prefix("#/$defs/"))
        {
            Some(name) => &self.root["$defs"][name],
            None => node,
        }
    }

    /// The schema without its "or null" (`anyOf [x, null]`, `type: [t, "null"]`).
    fn non_null(&self, node: &'a Value) -> Node<'a> {
        let node = self.resolve(node);
        if let Some(any) = node["anyOf"].as_array()
            && let Some(inner) = any.iter().find(|s| s["type"] != "null")
        {
            return self.non_null(inner);
        }
        if let Some(types) = node["type"].as_array()
            && let Some(t) = types
                .iter()
                .find_map(|t| t.as_str().filter(|t| *t != "null"))
        {
            return Node::Typed(node, t);
        }
        if let Some(variants) = node["oneOf"].as_array() {
            // String constants and enums, however schemars grouped them: one choice.
            if variants
                .iter()
                .all(|v| v["type"] == "string" && (v["const"].is_string() || v["enum"].is_array()))
            {
                let options = variants
                    .iter()
                    .flat_map(|v| {
                        v["const"].as_str().into_iter().chain(
                            v["enum"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str),
                        )
                    })
                    .collect();
                return Node::Enum(options);
            }
            // `{"number": 3}` or `{"random": …}`: each variant one required key, no `type` tag.
            let keyed = variants.iter().all(|v| {
                v["properties"]["type"]["const"].is_null()
                    && v["required"].as_array().is_some_and(|r| r.len() == 1)
            });
            return if keyed {
                Node::Keyed(node)
            } else {
                Node::Tagged(node)
            };
        }
        if let Some(e) = node["enum"].as_array() {
            return Node::Enum(e.iter().filter_map(Value::as_str).collect());
        }
        match node["type"].as_str() {
            Some(t) => Node::Typed(node, t),
            None => Node::Other,
        }
    }
}

enum Node<'a> {
    /// A JSON type: "number", "integer", "boolean", "string", "array", "object".
    Typed(&'a Value, &'a str),
    /// A tagged union: `oneOf` objects, each with a constant `type`.
    Tagged(&'a Value),
    /// A union of single-key objects (`{"number": 3}`, `{"expression": "…"}`).
    Keyed(&'a Value),
    /// A choice of strings.
    Enum(Vec<&'a str>),
    Other,
}

/// The time unit and the edits in progress, shared by the fields of one form.
pub struct Cx<'a> {
    /// The model's time unit, for the fields that hold times.
    pub time_unit: &'a str,
    /// Text being typed, by field key, until it commits.
    pub drafts: &'a mut HashMap<String, String>,
}

/// How a field reads: "std_dev" → "Standard deviation".
pub fn label(name: &str) -> String {
    let known = match name {
        "mean" => "Mean",
        "std_dev" => "Standard deviation",
        "min" => "Minimum",
        "max" => "Maximum",
        "mode" => "Most likely",
        "value" => "Value",
        "shape" => "Shape",
        "scale" => "Scale",
        "k" => "Stages (k)",
        "probability" => "Probability",
        "trials" => "Trials",
        "points" => "Points",
        "discrete" => "Discrete values",
        "capacity" => "Capacity",
        "limit" => "Arrival limit",
        "interarrival" => "Time between arrivals",
        "service" => "Service time",
        "duration" => "Duration",
        "warmup" => "Warm-up",
        "replications" => "Replications",
        "seed" => "Seed",
        "time_unit" => "Time unit",
        "type" => "Type",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    let s = name.replace('_', " ");
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// How a distribution or enum choice reads: "truncated_normal" → "Truncated normal".
pub fn choice_label(tag: &str) -> String {
    match tag {
        "log_normal" => "Log-normal".into(),
        "" => "from its value".into(),
        "long" => "Whole number".into(),
        "double" => "Decimal number".into(),
        "boolean" => "Yes or no".into(),
        "string" => "Text".into(),
        "dateTime" => "Date and time".into(),
        "user" => "Data points".into(),
        "hour" => "hours".into(),
        "day" => "days".into(),
        "year" => "years".into(),
        "min" => "minutes".into(),
        "s" => "seconds".into(),
        "ms" => "milliseconds".into(),
        other => label(other),
    }
}

/// Whether a distribution's parameter is a time (in the model's time unit) rather than a count or
/// a shape.
fn is_time(dist: &str, field: &str) -> bool {
    match dist {
        "poisson" | "binomial" | "beta" => false,
        "gamma" | "weibull" => field == "scale",
        _ => matches!(field, "value" | "mean" | "min" | "max" | "mode" | "std_dev"),
    }
}

/// Draws the editor for one parameter: `name` within the command's schema, with its current
/// value. `time` says the value (or a distribution's time parameters) is in the model's time
/// unit. Returns the new value when the user commits a change.
pub fn field(
    ui: &mut egui::Ui,
    cx: &mut Cx<'_>,
    schema: Schema<'_>,
    key: &str,
    name: &str,
    current: &Value,
    time: bool,
) -> Option<Value> {
    let node = schema.property(name)?;
    let help = schema.description(node).map(str::to_owned);
    let label = label(name);
    let nullable = help.as_deref().is_some_and(|h| h.contains("`null`"));
    edit(
        ui,
        cx,
        schema,
        node,
        key,
        &label,
        help.as_deref(),
        current,
        time,
        nullable,
    )
}

#[allow(clippy::too_many_arguments)]
fn edit(
    ui: &mut egui::Ui,
    cx: &mut Cx<'_>,
    schema: Schema<'_>,
    node: &Value,
    key: &str,
    label: &str,
    help: Option<&str>,
    current: &Value,
    time: bool,
    nullable: bool,
) -> Option<Value> {
    match schema.non_null(node) {
        Node::Tagged(n) => tagged(ui, cx, schema, n, key, label, current),
        Node::Enum(options) => row(ui, label, help, |ui| {
            let now = current.as_str().unwrap_or("");
            let mut picked = None;
            egui::ComboBox::from_id_salt(key)
                .selected_text(choice_label(now))
                .show_ui(ui, |ui| {
                    for o in &options {
                        if ui.selectable_label(*o == now, choice_label(o)).clicked() && *o != now {
                            picked = Some(json!(o));
                        }
                    }
                })
                .response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label));
            picked
        }),
        Node::Typed(_, "boolean") => {
            let mut b = current.as_bool().unwrap_or(false);
            let r = ui.checkbox(&mut b, label);
            let r = match help {
                Some(h) => r.on_hover_text(h),
                None => r,
            };
            r.changed().then(|| json!(b))
        }
        Node::Typed(n, t @ ("number" | "integer")) => {
            let integer = t == "integer";
            let unit = if time {
                Some(cx.time_unit.to_owned())
            } else {
                None
            };
            if nullable {
                // An optional limit: a checkbox to have one, then its value.
                let mut on = !current.is_null();
                let mut out = None;
                row(ui, label, help, |ui| {
                    if ui
                        .checkbox(&mut on, "")
                        .on_hover_text("Set a value")
                        .changed()
                    {
                        out = Some(if on { json!(1000) } else { Value::Null });
                    }
                    if on {
                        out = out.or_else(|| {
                            number(ui, cx, key, label, current, integer, n, unit.as_deref())
                        });
                    } else {
                        ui.label(theme::weak(ui, "none"));
                    }
                    out.clone()
                })
            } else {
                row(ui, label, help, |ui| {
                    number(ui, cx, key, label, current, integer, n, unit.as_deref())
                })
            }
        }
        Node::Typed(n, "array") => array(ui, cx, schema, n, key, label, current, time),
        Node::Keyed(n) => keyed(ui, cx, schema, n, key, label, current, time),
        Node::Typed(_, "string") => row(ui, label, help, |ui| {
            text(ui, cx, key, label, current, 260.0)
        }),
        _ => {
            ui.label(theme::weak(ui, format!("{label}: not editable here yet")));
            None
        }
    }
}

/// A form's label column: left-aligned text in a fixed width, so the editors line up.
pub fn label_cell(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    height: f32,
) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(170.0, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(170.0, height));
            ui.add(egui::Label::new(text).truncate())
        },
    )
    .inner
}

/// The outline a text field shows at rest, so it reads as a field on the canvas (focus draws
/// its own).
pub fn field_outline(ui: &egui::Ui, r: &egui::Response) {
    if !r.has_focus() {
        let p = theme::palette(ui.ctx());
        let stroke = if r.hovered() { p.text_weak } else { p.border };
        ui.painter().rect_stroke(
            r.rect,
            4.0,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
    }
}

/// A form row: the label in a fixed column, the editor beside it.
fn row<R>(
    ui: &mut egui::Ui,
    label: &str,
    help: Option<&str>,
    editor: impl FnOnce(&mut egui::Ui) -> Option<R>,
) -> Option<R> {
    ui.horizontal(|ui| {
        let r = label_cell(ui, label, 26.0);
        if let Some(h) = help {
            r.on_hover_text(h);
        }
        editor(ui)
    })
    .inner
}

/// A number field. Commits on Enter or losing focus when the text parses and is in range; Esc
/// restores the value.
#[allow(clippy::too_many_arguments)]
fn number(
    ui: &mut egui::Ui,
    cx: &mut Cx<'_>,
    key: &str,
    label: &str,
    current: &Value,
    integer: bool,
    node: &Value,
    unit: Option<&str>,
) -> Option<Value> {
    let shown = match current {
        Value::Number(n) => n
            .as_f64()
            .map(|v| {
                if integer {
                    format!("{}", n)
                } else {
                    crate::results::number(v)
                }
            })
            .unwrap_or_default(),
        _ => String::new(),
    };
    let editing = cx.drafts.contains_key(key);
    let mut text = cx.drafts.get(key).cloned().unwrap_or_else(|| shown.clone());
    let error = editing.then(|| parse(&text, integer, node).err()).flatten();
    let p = theme::palette(ui.ctx());
    let mut edit = egui::TextEdit::singleline(&mut text)
        .desired_width(110.0)
        .align(egui::Align2::RIGHT_CENTER)
        .font(egui::TextStyle::Monospace);
    if error.is_some() {
        edit = edit.text_color(p.error);
    }
    let r = ui.add(edit);
    field_outline(ui, &r);
    // Labelled for assistive technology (and tests) by the row's label.
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
    if let Some(u) = unit {
        ui.label(theme::weak(ui, u));
    }
    let mut out = None;
    if r.changed() || r.has_focus() {
        cx.drafts.insert(key.to_owned(), text.clone());
    }
    let escape = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape));
    if escape {
        cx.drafts.remove(key);
        r.surrender_focus();
    } else if r.lost_focus() {
        match parse(&text, integer, node) {
            Ok(v) => {
                cx.drafts.remove(key);
                if text != shown {
                    out = Some(v);
                }
            }
            Err(_) => {
                cx.drafts.insert(key.to_owned(), text.clone());
            }
        }
    }
    if let Some(e) = error {
        ui.label(RichText::new(e).color(p.error).size(12.0));
    }
    out
}

fn parse(text: &str, integer: bool, node: &Value) -> Result<Value, String> {
    let t = text.trim().replace(',', "");
    if integer {
        let v: u64 = t.parse().map_err(|_| "a whole number".to_owned())?;
        if node["format"] == "uint32" && v > u64::from(u32::MAX) {
            return Err("too large".into());
        }
        Ok(json!(v))
    } else {
        let v: f64 = t.parse().map_err(|_| "a number".to_owned())?;
        if !v.is_finite() {
            return Err("a finite number".into());
        }
        Ok(json!(v))
    }
}

/// A tagged union (a distribution): its kind as a choice, then the chosen kind's fields.
fn tagged(
    ui: &mut egui::Ui,
    cx: &mut Cx<'_>,
    schema: Schema<'_>,
    node: &Value,
    key: &str,
    label: &str,
    current: &Value,
) -> Option<Value> {
    let variants: Vec<&Value> = node["oneOf"].as_array().into_iter().flatten().collect();
    let tag_of = |v: &Value| v["properties"]["type"]["const"].as_str().map(str::to_owned);
    let now = current["type"].as_str().unwrap_or("").to_owned();
    let mut out = None;

    let picked = row(ui, label, node["description"].as_str(), |ui| {
        let mut picked = None;
        egui::ComboBox::from_id_salt(key)
            .selected_text(choice_label(&now))
            .width(170.0)
            .show_ui(ui, |ui| {
                for v in &variants {
                    if let Some(tag) = tag_of(v) {
                        let r = ui.selectable_label(tag == now, choice_label(&tag));
                        let r = match v["description"].as_str() {
                            Some(d) => r.on_hover_text(d),
                            None => r,
                        };
                        if r.clicked() && tag != now {
                            picked = Some(tag);
                        }
                    }
                }
            })
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label));
        picked
    });
    if let Some(tag) = picked
        && let Some(v) = variants.iter().find(|v| tag_of(v).as_deref() == Some(&tag))
    {
        out = Some(default_variant(v, &tag, current));
    }

    // The chosen kind's fields, indented under it, in the schema's required order.
    if let Some(v) = variants.iter().find(|v| tag_of(v).as_deref() == Some(&now)) {
        let fields: Vec<&str> = v["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .chain(
                v["properties"]
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.keys().map(String::as_str)),
            )
            .filter(|f| *f != "type")
            .fold(Vec::new(), |mut acc, f| {
                if !acc.contains(&f) {
                    acc.push(f);
                }
                acc
            });
        ui.indent(key, |ui| {
            for f in fields {
                let k = format!("{key}/{f}");
                let fnode = &v["properties"][f];
                let changed = edit(
                    ui,
                    cx,
                    schema,
                    fnode,
                    &k,
                    &label_for(f),
                    fnode["description"].as_str(),
                    &current[f],
                    is_time(&now, f),
                    false,
                );
                if let Some(nv) = changed {
                    let mut obj = current.as_object().cloned().unwrap_or_default();
                    obj.insert(f.to_owned(), nv);
                    out = Some(Value::Object(obj));
                }
            }
        });
    }
    out
}

fn label_for(f: &str) -> String {
    label(f)
}

/// A new value of kind `tag`, keeping what carries over from `current` (a mean stays a mean).
fn default_variant(variant: &Value, tag: &str, current: &Value) -> Value {
    let mut obj = Map::new();
    obj.insert("type".into(), json!(tag));
    let old_mean = current["mean"]
        .as_f64()
        .or_else(|| current["value"].as_f64())
        .unwrap_or(1.0);
    for (f, node) in variant["properties"].as_object().into_iter().flatten() {
        if f == "type" {
            continue;
        }
        let v = if let Some(old) = current.get(f).filter(|_| current["type"] != "user") {
            old.clone()
        } else {
            match (f.as_str(), node["type"].as_str()) {
                ("mean" | "value" | "mode", _) => json!(old_mean),
                ("min", _) => json!(old_mean * 0.5),
                ("max", _) => json!(old_mean * 1.5),
                ("std_dev", _) => json!(old_mean * 0.25),
                ("probability", _) => json!(0.5),
                (_, Some("integer")) => json!(2),
                (_, Some("boolean")) => json!(false),
                (_, Some("array")) => json!([
                    {"probability": 0.5, "value": old_mean * 0.5},
                    {"probability": 1.0, "value": old_mean * 1.5}
                ]),
                _ => json!(1.0),
            }
        };
        obj.insert(f.clone(), v);
    }
    Value::Object(obj)
}

/// An array of objects (a user distribution's points): one row each, with a remove button, and
/// a row to add.
#[allow(clippy::too_many_arguments)]
fn array(
    ui: &mut egui::Ui,
    cx: &mut Cx<'_>,
    schema: Schema<'_>,
    node: &Value,
    key: &str,
    label: &str,
    current: &Value,
    time: bool,
) -> Option<Value> {
    let item = schema.resolve(&node["items"]);
    let items = current.as_array().cloned().unwrap_or_default();
    let mut out: Option<Vec<Value>> = None;
    ui.label(theme::strong(label));
    let fields: Vec<String> = item["properties"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    for (i, it) in items.iter().enumerate() {
        ui.horizontal(|ui| {
            for f in &fields {
                let k = format!("{key}/{i}/{f}");
                ui.label(theme::weak(ui, label_for(f)));
                let unit = (time && f == "value").then_some(cx.time_unit.to_owned());
                if let Some(v) = number(
                    ui,
                    cx,
                    &k,
                    &format!("{} {}", label_for(f), i + 1),
                    &it[f],
                    item["properties"][f]["type"] == "integer",
                    &item["properties"][f],
                    unit.as_deref(),
                ) {
                    let mut all = out.clone().unwrap_or_else(|| items.clone());
                    all[i][f] = v;
                    out = Some(all);
                }
            }
            if items.len() > 1 && ui.small_button("Remove").clicked() {
                let mut all = out.clone().unwrap_or_else(|| items.clone());
                all.remove(i);
                out = Some(all);
            }
        });
    }
    if ui.small_button("Add point").clicked() {
        let mut all = out.clone().unwrap_or_else(|| items.clone());
        let last = items
            .last()
            .cloned()
            .unwrap_or_else(|| json!({"probability": 1.0, "value": 1.0}));
        all.push(last);
        out = Some(all);
    }
    out.map(Value::Array)
}

/// A text field. Commits on Enter or losing focus; Esc restores the value.
pub fn text(
    ui: &mut egui::Ui,
    cx: &mut Cx<'_>,
    key: &str,
    label: &str,
    current: &Value,
    width: f32,
) -> Option<Value> {
    let shown = current.as_str().unwrap_or("").to_owned();
    let mut t = cx.drafts.get(key).cloned().unwrap_or_else(|| shown.clone());
    let r = ui.add(
        egui::TextEdit::singleline(&mut t)
            .desired_width(width)
            .font(egui::TextStyle::Monospace),
    );
    field_outline(ui, &r);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
    if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        cx.drafts.remove(key);
        r.surrender_focus();
        None
    } else if r.lost_focus() {
        cx.drafts.remove(key);
        (t != shown).then(|| json!(t))
    } else {
        if r.has_focus() || r.changed() {
            cx.drafts.insert(key.to_owned(), t);
        }
        None
    }
}

/// How a keyed union's choice reads.
fn key_label(k: &str) -> String {
    match k {
        "number" => "Number".into(),
        "boolean" => "Yes or no".into(),
        "text" => "Text".into(),
        "random" => "Random".into(),
        "expression" => "Expression".into(),
        other => label(other),
    }
}

/// A union of single-key objects: the key as a choice, then its value's editor.
#[allow(clippy::too_many_arguments)]
fn keyed(
    ui: &mut egui::Ui,
    cx: &mut Cx<'_>,
    schema: Schema<'_>,
    node: &Value,
    key: &str,
    label: &str,
    current: &Value,
    time: bool,
) -> Option<Value> {
    let variants: Vec<(&str, &Value)> = node["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let k = v["required"][0].as_str()?;
            Some((k, &v["properties"][k]))
        })
        .collect();
    let now = current
        .as_object()
        .and_then(|o| o.keys().next())
        .map_or("", String::as_str)
        .to_owned();
    let mut out = None;
    let picked = row(ui, label, node["description"].as_str(), |ui| {
        let mut picked = None;
        egui::ComboBox::from_id_salt(key)
            .selected_text(key_label(&now))
            .width(170.0)
            .show_ui(ui, |ui| {
                for (k, _) in &variants {
                    if ui.selectable_label(*k == now, key_label(k)).clicked() && *k != now {
                        picked = Some((*k).to_owned());
                    }
                }
            })
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label));
        picked
    });
    if let Some(k) = picked {
        // A starting value of the new kind, carrying a number over where it can.
        let n = current["number"].as_f64().unwrap_or(1.0);
        let v = match k.as_str() {
            "number" => json!(n),
            "boolean" => json!(false),
            "text" => json!(""),
            "random" => json!({"type": "exponential", "mean": n}),
            "expression" => json!(format!("{n}")),
            _ => Value::Null,
        };
        out = Some(json!({ k: v }));
    }
    if let Some((k, inner)) = variants.iter().find(|(k, _)| *k == now) {
        let k_key = format!("{key}/{k}");
        let changed = ui
            .indent(&k_key, |ui| {
                edit(
                    ui,
                    cx,
                    schema,
                    inner,
                    &k_key,
                    &key_label(k),
                    None,
                    &current[*k],
                    time,
                    false,
                )
            })
            .inner;
        if let Some(v) = changed {
            out = Some(json!({ (*k): v }));
        }
    }
    out
}
