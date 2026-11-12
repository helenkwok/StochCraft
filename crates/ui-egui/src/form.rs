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

use serde_json::{Map, Value, json};

use craft_ui::localized::{LocaleNumbers, NumberField, Unit};

use crate::i18n::{t, td, tf};
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

/// The time unit, the region's numbers, the model's calendars and the edits in progress, shared
/// by the fields of one form.
pub struct Cx<'a> {
    /// The model's time unit, for the fields that hold times.
    pub time_unit: &'a str,
    /// How numbers are written and read (the user's region).
    pub numbers: &'a LocaleNumbers,
    /// Text being typed, by field key, until it commits.
    pub drafts: &'a mut HashMap<String, String>,
    /// The model's calendars, id and name, for the fields that name one.
    pub calendars: &'a [(String, String)],
}

/// The model's calendars, id and name, for [`Cx::calendars`].
pub fn calendars(model: &Value) -> Vec<(String, String)> {
    model["calendars"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = c["id"].as_str()?.to_owned();
            let name = c["name"].as_str().unwrap_or("").to_owned();
            Some((id, name))
        })
        .collect()
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
        "share" => "Share",
        "shape" => "Shape",
        "scale" => "Scale",
        "k" => "Stages (k)",
        "probability" => "Probability",
        "trials" => "Trials",
        "points" => "Points",
        "discrete" => "Discrete values",
        "capacity" => "Capacity",
        "limit" => "Arrival limit",
        "queue_capacity" => "Waiting room",
        "size" => "Batch size",
        "priority" => "Priority",
        "quantity" => "Units",
        "interarrival" => "Time between arrivals",
        "service" => "Service time",
        "duration" => "Duration",
        "warmup" => "Warm-up",
        "replications" => "Replications",
        "seed" => "Seed",
        "time_unit" => "Time unit",
        "type" => "Type",
        "timed" => "Arrivals by calendar",
        "idle_outside" => "No arrivals outside calendars",
        "transfer" => "Transfer time",
        "transfer_holds_sender" => "Sender waits during transfer",
        "planned_run_time" => "Planned run time per item",
        "breakdown" => "Breakdowns",
        "time_to_failure" => "Time to failure",
        "repair" => "Repair time",
        "interruptible" => "Failure interrupts the job",
        "hands_on_while_down" => "Hands on finished parts while down",
        "available" => "Available outside calendars",
        "availability" => "Availability by calendar",
        "shifts" => "Units by calendar",
        "calendar" => "Calendar",
        "setup" => "Setup",
        "time" => "Setup time",
        "on_change_of" => "Only when this property changes",
        "by_change" => "Times for particular changes",
        "from" => "From",
        "to" => "To",
        _ => "",
    };
    if !known.is_empty() {
        return t(known).to_owned();
    }
    let s = name.replace('_', " ");
    let mut c = s.chars();
    let english = c
        .next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default();
    td(&english)
}

/// How a distribution or enum choice reads: "truncated_normal" → "Truncated normal".
pub fn choice_label(tag: &str) -> String {
    match tag {
        "log_normal" => t("Log-normal").into(),
        "" => t("from its value").into(),
        "long" => t("Whole number").into(),
        "double" => t("Decimal number").into(),
        "boolean" => t("Yes or no").into(),
        "string" => t("Text").into(),
        "dateTime" => t("Date and time").into(),
        "user" => t("Data points").into(),
        "hour" => t("hours").into(),
        "day" => t("days").into(),
        "year" => t("years").into(),
        "min" => t("minutes").into(),
        "s" => t("seconds").into(),
        "ms" => t("milliseconds").into(),
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
        Node::Tagged(n) => tagged(ui, cx, schema, n, key, label, current, nullable),
        Node::Typed(n, "object") => object(ui, cx, schema, n, key, label, help, current, nullable),
        Node::Enum(options) => row(ui, label, help, |ui| {
            let now = current.as_str().unwrap_or("");
            let mut picked = None;
            craft_ui::ComboBox::from_id_salt(key)
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
        Node::Typed(n, "boolean") => {
            // Left out, a setting has the schema's default (`hands_on_while_down` is true).
            let mut b = current
                .as_bool()
                .or_else(|| node["default"].as_bool())
                .or_else(|| n["default"].as_bool())
                .unwrap_or(false);
            let r = ui.checkbox(&mut b, label);
            let r = match help {
                Some(h) => r.on_hover_text(h),
                None => r,
            };
            r.changed().then(|| json!(b))
        }
        Node::Typed(n, ty @ ("number" | "integer")) => {
            let integer = ty == "integer";
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
                    let r = ui.checkbox(&mut on, "").on_hover_text(t("Set a value"));
                    // Named after the field, for screen readers (the box has no text of its own).
                    let ticked = on;
                    r.widget_info(|| {
                        egui::WidgetInfo::selected(egui::Role::CheckBox, true, ticked, label)
                    });
                    if r.changed() {
                        // A first value of the field's scale: room for ten waiting, or a
                        // thousand arrivals.
                        let first = if key.ends_with("queue_capacity") {
                            10
                        } else {
                            1000
                        };
                        out = Some(if on { json!(first) } else { Value::Null });
                    }
                    if on {
                        out = out.or_else(|| {
                            number(ui, cx, key, label, current, integer, n, unit.as_deref())
                        });
                    } else {
                        ui.label(theme::weak(ui, t("unlimited")));
                    }
                    out.clone()
                })
            } else {
                row(ui, label, help, |ui| {
                    number(ui, cx, key, label, current, integer, n, unit.as_deref())
                })
            }
        }
        Node::Typed(n, "array") => {
            let item = schema.resolve(&n["items"]);
            // Rows of numbers and text (a distribution's points, a mix's values and shares).
            let numbers = item["properties"].as_object().is_some_and(|m| {
                m.values()
                    .all(|p| matches!(p["type"].as_str(), Some("number" | "integer" | "string")))
            }) && item["properties"].get("calendar").is_none();
            if numbers {
                array(ui, cx, schema, n, key, label, current, time)
            } else {
                records(ui, cx, schema, item, key, label, help, current)
            }
        }
        Node::Keyed(n) => keyed(ui, cx, schema, n, key, label, current, time),
        Node::Typed(_, "string") => row(ui, label, help, |ui| {
            text(ui, cx, key, label, current, 260.0)
        }),
        _ => {
            ui.label(theme::weak(
                ui,
                tf("{field}: not editable here yet", &[("field", label)]),
            ));
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

/// A number field: craft-ui's `NumberField`, which reads and writes numbers the way the user's
/// region does (its decimal mark and grouping), commits on Enter or leaving the field, steps with
/// the arrow keys, and leaves text that isn't a number in the field, outlined with the reason,
/// until it is fixed or Esc restores the value. Returns the new value when it changed.
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
    let unit = unit.map(|u| Unit::new(&format!("{{n}} {u}")));
    let r;
    let out = if integer {
        let max = if node["format"] == "uint32" {
            f64::from(u32::MAX)
        } else {
            u64::MAX as f64
        };
        let mut v = current.as_u64().unwrap_or(0);
        let mut field = NumberField::new(&mut v, cx.numbers)
            .id_salt(key)
            .range(0.0..=max)
            .desired_width(110.0);
        if let Some(u) = unit {
            field = field.unit(u);
        }
        r = ui.add(field);
        r.changed().then(|| json!(v))
    } else {
        let mut v = current.as_f64().unwrap_or(0.0);
        let mut field = NumberField::new(&mut v, cx.numbers)
            .id_salt(key)
            .decimals(0..=6)
            .desired_width(110.0);
        if let Some(u) = unit {
            field = field.unit(u);
        }
        r = ui.add(field);
        r.changed().then(|| json!(v))
    };
    // Named for assistive technology (and tests) by the row's label.
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
    out
}

/// A tagged union (a distribution): its kind as a choice, then the chosen kind's fields. When
/// `nullable`, the first choice is None (a transfer time that can be removed).
#[allow(clippy::too_many_arguments)]
fn tagged(
    ui: &mut egui::Ui,
    cx: &mut Cx<'_>,
    schema: Schema<'_>,
    node: &Value,
    key: &str,
    label: &str,
    current: &Value,
    nullable: bool,
) -> Option<Value> {
    let variants: Vec<&Value> = node["oneOf"].as_array().into_iter().flatten().collect();
    let tag_of = |v: &Value| v["properties"]["type"]["const"].as_str().map(str::to_owned);
    let now = current["type"].as_str().unwrap_or("").to_owned();
    let mut out = None;

    let picked = row(ui, label, node["description"].as_str(), |ui| {
        let mut picked = None;
        let shown = if nullable && current.is_null() {
            t("None").to_owned()
        } else {
            choice_label(&now)
        };
        craft_ui::ComboBox::from_id_salt(key)
            .selected_text(shown)
            .width(170.0)
            .show_ui(ui, |ui| {
                if nullable
                    && ui.selectable_label(current.is_null(), t("None")).clicked()
                    && !current.is_null()
                {
                    out = Some(Value::Null);
                }
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

/// A first value for a field being turned on or a row being added: the schema's required fields,
/// a distribution as an exponential with mean 1, a setting at its default.
fn first_value(schema: Schema<'_>, node: &Value) -> Value {
    match schema.non_null(node) {
        Node::Tagged(n) => {
            let variants: Vec<&Value> = n["oneOf"].as_array().into_iter().flatten().collect();
            fn tag_of(v: &Value) -> Option<&str> {
                v["properties"]["type"]["const"].as_str()
            }
            variants
                .iter()
                .find(|v| tag_of(v) == Some("exponential"))
                .or_else(|| variants.first())
                .and_then(|v| Some(default_variant(v, tag_of(v)?, &Value::Null)))
                .unwrap_or(Value::Null)
        }
        Node::Typed(n, "object") => {
            let mut obj = Map::new();
            for f in n["required"].as_array().into_iter().flatten() {
                if let Some(f) = f.as_str() {
                    obj.insert(f.to_owned(), first_value(schema, &n["properties"][f]));
                }
            }
            Value::Object(obj)
        }
        Node::Typed(n, "boolean") => json!(
            node["default"]
                .as_bool()
                .or_else(|| n["default"].as_bool())
                .unwrap_or(false)
        ),
        Node::Typed(_, "number" | "integer") => json!(1),
        Node::Typed(_, "array") => json!([]),
        Node::Typed(_, "string") => json!(""),
        Node::Enum(options) => options.first().map_or(Value::Null, |o| json!(o)),
        Node::Keyed(_) => json!({"number": 1.0}),
        _ => Value::Null,
    }
}

/// Whether property `f` of an object may be left out or `null` (an `Option` field).
fn optional_field(schema: Schema<'_>, object: &Value, f: &str) -> bool {
    let required = object["required"]
        .as_array()
        .is_some_and(|r| r.iter().any(|x| x == f));
    let node = schema.resolve(&object["properties"][f]);
    let null = node["anyOf"]
        .as_array()
        .is_some_and(|a| a.iter().any(|s| s["type"] == "null"))
        || node["type"]
            .as_array()
            .is_some_and(|a| a.iter().any(|t| t == "null"));
    !required && null
}

/// An object (a server's breakdowns): its fields, indented under its label, required ones first.
/// When `nullable`, a checkbox beside the label turns it on (with a first value) or off.
#[allow(clippy::too_many_arguments)]
fn object(
    ui: &mut egui::Ui,
    cx: &mut Cx<'_>,
    schema: Schema<'_>,
    node: &Value,
    key: &str,
    label: &str,
    help: Option<&str>,
    current: &Value,
    nullable: bool,
) -> Option<Value> {
    let mut out = None;
    if nullable {
        let mut on = !current.is_null();
        out = row(ui, label, help, |ui| {
            let r = ui.checkbox(&mut on, "").on_hover_text(t("Set a value"));
            let ticked = on;
            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, ticked, label));
            if !on {
                ui.label(theme::weak(ui, t("None")));
            }
            r.changed().then(|| {
                if on {
                    first_value(schema, node)
                } else {
                    Value::Null
                }
            })
        });
        if current.is_null() {
            return out;
        }
    } else {
        ui.label(theme::strong(label));
    }
    let fields: Vec<&str> = node["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .chain(
            node["properties"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys().map(String::as_str)),
        )
        .fold(Vec::new(), |mut acc, f| {
            if !acc.contains(&f) {
                acc.push(f);
            }
            acc
        });
    ui.indent(key, |ui| {
        for f in fields {
            let fnode = &node["properties"][f];
            let changed = edit(
                ui,
                cx,
                schema,
                fnode,
                &format!("{key}/{f}"),
                &label_for(f),
                fnode["description"].as_str(),
                &current[f],
                false,
                optional_field(schema, node, f),
            );
            if let Some(nv) = changed {
                let mut obj = current.as_object().cloned().unwrap_or_default();
                if nv.is_null() {
                    obj.remove(f);
                } else {
                    obj.insert(f.to_owned(), nv);
                }
                out = Some(Value::Object(obj));
            }
        }
    });
    out
}

/// A list of settings that each apply while a calendar is active (a server's availability, a
/// source's arrivals, a pool's shifts): one entry per calendar, its calendar chosen from the
/// model's, with a remove button, and a button to add one.
#[allow(clippy::too_many_arguments)]
fn records(
    ui: &mut egui::Ui,
    cx: &mut Cx<'_>,
    schema: Schema<'_>,
    item: &Value,
    key: &str,
    label: &str,
    help: Option<&str>,
    current: &Value,
) -> Option<Value> {
    let items = current.as_array().cloned().unwrap_or_default();
    let mut out: Option<Vec<Value>> = None;
    let mut removed = None;
    let r = ui.label(theme::strong(label));
    if let Some(h) = help {
        r.on_hover_text(h);
    }
    let fields: Vec<String> = item["properties"]
        .as_object()
        .map(|m| m.keys().filter(|k| *k != "calendar").cloned().collect())
        .unwrap_or_default();
    let calendars = cx.calendars.to_vec();
    let name_of = |id: &str| {
        calendars.iter().find(|(i, _)| i == id).map_or_else(
            || id.to_owned(),
            |(i, n)| if n.is_empty() { i.clone() } else { n.clone() },
        )
    };
    let used = |id: &str| items.iter().any(|it| it["calendar"] == id);
    // Entries by calendar, or plain rows (a changeover's times for particular changes).
    let by_calendar = item["properties"]["calendar"].is_object();
    ui.indent(key, |ui| {
        for (i, it) in items.iter().enumerate() {
            let k = format!("{key}/{i}");
            if !by_calendar {
                ui.horizontal(|ui| {
                    ui.label(theme::weak(ui, format!("{} {}", label, i + 1)));
                    if ui.small_button(t("Remove")).clicked() {
                        removed = Some(i);
                    }
                });
            }
            ui.horizontal(|ui| {
                if !by_calendar {
                    return;
                }
                let cal_label = label_for("calendar");
                label_cell(ui, cal_label.as_str(), 26.0);
                let now = it["calendar"].as_str().unwrap_or("");
                craft_ui::ComboBox::from_id_salt(format!("{k}/calendar"))
                    .selected_text(name_of(now))
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        for (id, _) in &calendars {
                            // The first active entry applies, so each calendar has one entry.
                            let taken = id != now && used(id);
                            if ui
                                .add_enabled(
                                    !taken,
                                    egui::Button::selectable(id == now, name_of(id)),
                                )
                                .clicked()
                                && id != now
                            {
                                let mut all = out.clone().unwrap_or_else(|| items.clone());
                                all[i]["calendar"] = json!(id);
                                out = Some(all);
                            }
                        }
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::ComboBox,
                            true,
                            format!("{cal_label} {}", i + 1),
                        )
                    });
                if ui.small_button(t("Remove")).clicked() {
                    removed = Some(i);
                }
            });
            ui.indent(&k, |ui| {
                for f in &fields {
                    let fnode = &item["properties"][f.as_str()];
                    // Within an entry, "available" is the entry's own setting.
                    let flabel = if f == "available" {
                        t("Available").to_owned()
                    } else {
                        label_for(f)
                    };
                    if let Some(v) = edit(
                        ui,
                        cx,
                        schema,
                        fnode,
                        &format!("{k}/{f}"),
                        &flabel,
                        fnode["description"].as_str(),
                        &it[f.as_str()],
                        false,
                        false,
                    ) {
                        let mut all = out.clone().unwrap_or_else(|| items.clone());
                        all[i][f.as_str()] = v;
                        out = Some(all);
                    }
                }
            });
        }
        if !by_calendar {
            if ui.small_button(t("Add row")).clicked() {
                let mut all = out.clone().unwrap_or_else(|| items.clone());
                all.push(first_value(schema, item));
                out = Some(all);
            }
            return;
        }
        match calendars.iter().find(|(id, _)| !used(id)) {
            Some((first, _)) => {
                if ui.small_button(t("Add calendar entry")).clicked() {
                    let mut all = out.clone().unwrap_or_else(|| items.clone());
                    let mut new = first_value(schema, item);
                    new["calendar"] = json!(first);
                    all.push(new);
                    out = Some(all);
                }
            }
            None if calendars.is_empty() => {
                ui.label(theme::weak(ui, t("The model has no calendars yet.")));
            }
            // Every calendar has its entry.
            None => {}
        }
    });
    // Removed last, so the other entries' edits this frame use the indexes they were drawn with.
    if let Some(i) = removed {
        let mut all = out.unwrap_or(items);
        all.remove(i);
        out = Some(all);
    }
    out.map(Value::Array)
}

/// An array of objects (a user distribution's points, a mix's values and shares): one row each,
/// with a remove button, and a row to add.
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
    let mut removed = None;
    ui.label(theme::strong(label));
    // In the order the schema requires them (a mix: the value, then its share).
    let fields: Vec<String> = item["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .chain(
            item["properties"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys().cloned()),
        )
        .fold(Vec::new(), |mut acc, f| {
            if !acc.contains(&f) {
                acc.push(f);
            }
            acc
        });
    let is_text = |f: &str| item["properties"][f]["type"] == "string";
    let texts = fields.iter().any(|f| is_text(f));
    for (i, it) in items.iter().enumerate() {
        ui.horizontal(|ui| {
            for f in &fields {
                let k = format!("{key}/{i}/{f}");
                ui.label(theme::weak(ui, label_for(f)));
                if is_text(f) {
                    if let Some(v) = text(
                        ui,
                        cx,
                        &k,
                        &format!("{} {}", label_for(f), i + 1),
                        &it[f],
                        110.0,
                    ) {
                        let mut all = out.clone().unwrap_or_else(|| items.clone());
                        all[i][f] = v;
                        out = Some(all);
                    }
                    continue;
                }
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
            if items.len() > 1 && ui.small_button(t("Remove")).clicked() {
                removed = Some(i);
            }
        });
    }
    let add = if texts {
        t("Add value")
    } else {
        t("Add point")
    };
    if ui.small_button(add).clicked() {
        let mut all = out.clone().unwrap_or_else(|| items.clone());
        let mut last = items.last().cloned().unwrap_or_else(|| {
            if texts {
                // A mix's first value, as its schema has it.
                first_value(schema, item)
            } else {
                json!({"probability": 1.0, "value": 1.0})
            }
        });
        // A new value of a mix starts empty, its share the last one's.
        for f in fields.iter().filter(|f| is_text(f)) {
            last[f.as_str()] = json!("");
        }
        all.push(last);
        out = Some(all);
    }
    // Removed last, so the other entries' edits this frame use the indexes they were drawn with.
    if let Some(i) = removed {
        let mut all = out.unwrap_or(items);
        all.remove(i);
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
        "number" => t("Number").into(),
        "boolean" => t("Yes or no").into(),
        "text" => t("Text").into(),
        "random" => t("Random").into(),
        "mix" => t("Mix").into(),
        "expression" => t("Expression").into(),
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
        craft_ui::ComboBox::from_id_salt(key)
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
            // Two values, half each; a text value carries over as the first.
            "mix" => json!([
                {"value": current["text"].as_str().unwrap_or("A"), "share": 1.0},
                {"value": "B", "share": 1.0},
            ]),
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
