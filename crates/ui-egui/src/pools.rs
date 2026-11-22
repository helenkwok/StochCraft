//! Shared resource pools in the window (M2.5): the model overview lists the pools, with their
//! units and roles, and a task with shared resources lists what it needs, one alternative per
//! row, tried in order. Every change is one engine command, so each undoes in one step.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use stochcraft_engine::ErrorCode;

use crate::editor::{params_schema, section};
use crate::form::{self, Schema};
use crate::i18n::{t, tf};
use crate::{App, theme};

/// Roles as the user types them: separated by commas, blanks dropped.
fn roles_of(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
        .collect()
}

fn roles_text(v: &Value) -> String {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

impl App {
    /// The model overview's pools: each with its name, units and roles, and a way to add one.
    pub(crate) fn pools_section(&mut self, ui: &mut egui::Ui, model: &Value) {
        section(ui, t("Resource pools"));
        ui.label(theme::weak(
            ui,
            t("Units that several tasks share. A task with shared resources takes them by pool or by role."),
        ));
        let schema = params_schema("resource.set");
        let unit = self.time_unit_label();
        let calendars = form::calendars(model);
        for r in model["resources"].as_array().cloned().unwrap_or_default() {
            let id = r["id"].as_str().unwrap_or("").to_owned();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                // An empty name shows the id, as the other places that name a pool do.
                ui.label(theme::strong(
                    r["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&id),
                ));
                ui.label(theme::weak(ui, &id).monospace());
                if ui
                    .add(egui::Button::new(
                        egui::RichText::new(t("Remove")).size(12.0),
                    ))
                    .on_hover_text(tf("Remove the pool {id}", &[("id", &id)]))
                    .clicked()
                {
                    self.execute("resource.remove", json!({ "id": id }));
                }
            });
            if let Some(n) = self.text_field(
                ui,
                &format!("pool/{id}/name"),
                t("Name"),
                r["name"].as_str().unwrap_or(""),
                true,
            ) {
                self.execute("resource.set", json!({ "id": id, "name": n }));
            }
            let mut cx = form::Cx {
                time_unit: &unit,
                numbers: &self.numbers,
                drafts: &mut self.drafts,
                calendars: &[],
            };
            if let Some(q) = form::field(
                ui,
                &mut cx,
                Schema::new(&schema),
                &format!("pool/{id}/quantity"),
                "quantity",
                &r["quantity"],
                false,
            ) {
                self.execute("resource.set", json!({ "id": id, "quantity": q }));
            }
            if let Some(text) = self.text_field(
                ui,
                &format!("pool/{id}/roles"),
                t("Roles"),
                &roles_text(&r["roles"]),
                true,
            ) {
                self.execute(
                    "resource.set",
                    json!({ "id": id, "roles": roles_of(&text) }),
                );
            }
            // Shifts and availability: the units, and whether they are on shift, while each
            // calendar is active. Left out of the file, `available` is true.
            for f in ["shifts", "available", "availability"] {
                let current = match (f, &r[f]) {
                    ("available", Value::Null) => json!(true),
                    (_, v) => v.clone(),
                };
                let mut cx = form::Cx {
                    time_unit: &unit,
                    numbers: &self.numbers,
                    drafts: &mut self.drafts,
                    calendars: &calendars,
                };
                if let Some(v) = form::field(
                    ui,
                    &mut cx,
                    Schema::new(&schema),
                    &format!("pool/{id}/{f}"),
                    f,
                    &current,
                    false,
                ) {
                    self.execute("resource.set", json!({ "id": id, f: v }));
                }
            }
        }
        ui.add_space(6.0);
        if ui.add(theme::secondary(ui, t("Add pool"))).clicked()
            && let Some(r) = self.execute("resource.add", json!({}))
            && let Some(id) = r["id"].as_str()
        {
            self.info(tf("Added pool {id}", &[("id", id)]));
        }
    }

    /// A task with shared resources: what each entity needs, the alternatives in order.
    pub(crate) fn needs_section(&mut self, ui: &mut egui::Ui, model: &Value, el: &Value) {
        let id = el["id"].as_str().unwrap_or("").to_owned();
        section(ui, t("Needs"));
        ui.label(theme::weak(
            ui,
            t("Each entity takes the first of these that can be met now, all at once, and gives the units back when it leaves."),
        ));
        let pools: Vec<(String, String)> = model["resources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let id = r["id"].as_str()?.to_owned();
                let name = r["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&id);
                Some((id.clone(), name.to_owned()))
            })
            .collect();
        let needs: Vec<Value> = el["needs"].as_array().cloned().unwrap_or_default();
        let mut changed: Option<Vec<Value>> = None;
        let schema = params_schema("resource.set");
        let unit = self.time_unit_label();
        for (i, need) in needs.iter().enumerate() {
            let label = tf("Alternative {n}", &[("n", &(i + 1).to_string())]);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                form::label_cell(ui, label.as_str(), 26.0);
                let by_roles = need.get("roles").is_some();
                let shown = if by_roles {
                    t("By role").to_owned()
                } else {
                    let p = need["resource"].as_str().unwrap_or("");
                    pools
                        .iter()
                        .find(|(i, _)| i == p)
                        .map_or_else(|| p.to_owned(), |(_, n)| n.clone())
                };
                let mut pick: Option<Value> = None;
                let r = craft_ui::ComboBox::from_id_salt((&id, "need", i))
                    .selected_text(shown)
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        for (pid, pname) in &pools {
                            if ui
                                .selectable_label(need["resource"] == *pid, pname)
                                .clicked()
                            {
                                pick = Some(json!({"resource": pid, "quantity": need["quantity"]}));
                            }
                        }
                        if ui.selectable_label(by_roles, t("By role")).clicked() && !by_roles {
                            pick = Some(json!({"roles": [], "quantity": need["quantity"]}));
                        }
                    });
                r.response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label.as_str())
                });
                if let Some(n) = pick {
                    let mut all = needs.clone();
                    if let Some(slot) = all.get_mut(i) {
                        *slot = n;
                    }
                    changed = Some(all);
                }
                if needs.len() > 1
                    && ui
                        .add(egui::Button::new(
                            egui::RichText::new(t("Remove")).size(12.0),
                        ))
                        .on_hover_text(tf("Remove alternative {n}", &[("n", &(i + 1).to_string())]))
                        .clicked()
                {
                    let mut all = needs.clone();
                    all.remove(i);
                    changed = Some(all);
                }
            });
            if need.get("roles").is_some()
                && let Some(text) = self.text_field(
                    ui,
                    &format!("{id}/need/{i}/roles"),
                    t("Roles"),
                    &roles_text(&need["roles"]),
                    // A need with no roles would take any pool, so it must name one.
                    false,
                )
            {
                let mut all = needs.clone();
                if let Some(slot) = all.get_mut(i) {
                    slot["roles"] = json!(roles_of(&text));
                }
                changed = Some(all);
            }
            let mut cx = form::Cx {
                time_unit: &unit,
                numbers: &self.numbers,
                drafts: &mut self.drafts,
                calendars: &[],
            };
            let quantity = need.get("quantity").cloned().unwrap_or(json!(1));
            if let Some(q) = form::field(
                ui,
                &mut cx,
                Schema::new(&schema),
                &format!("{id}/need/{i}/quantity"),
                "quantity",
                &quantity,
                false,
            ) {
                let mut all = needs.clone();
                if let Some(slot) = all.get_mut(i) {
                    slot["quantity"] = q;
                }
                changed = Some(all);
            }
        }
        ui.add_space(6.0);
        if ui.add(theme::secondary(ui, t("Add alternative"))).clicked() {
            let first = pools.first().map(|(p, _)| p.clone()).unwrap_or_default();
            let mut all = needs.clone();
            all.push(json!({"resource": first, "quantity": 1}));
            changed = Some(all);
        }
        if let Some(all) = changed {
            self.execute("block.set", json!({ "id": id, "needs": all }));
        }
    }

    /// A task with shared resources: a service time of its own for each pool it can draw on,
    /// one row per pool in its needs (a need by role names every pool with those roles). A
    /// ticked pool serves with its own time, before the task's time by property and its own
    /// service time; an unticked one takes those.
    pub(crate) fn by_resource_section(&mut self, ui: &mut egui::Ui, model: &Value, el: &Value) {
        let id = el["id"].as_str().unwrap_or("").to_owned();
        section(ui, t("Service time by resource"));
        ui.label(theme::weak(
            ui,
            t("Tick a pool to give its units a service time of their own. The other pools serve with the task's service time."),
        ));
        let entries: Vec<Value> = el["processing_by_resource"]["by_resource"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // The pools the task can draw on, in the model's order, as (id, name), and any pool it
        // has a time for that it no longer needs (a row to untick, since the model reports it).
        let drawn = |pool: &Value| {
            let pid = pool["id"].as_str().unwrap_or("");
            entries.iter().any(|b| b["resource"].as_str() == Some(pid))
                || el["needs"].as_array().into_iter().flatten().any(|n| {
                    if let Some(roles) = n["roles"].as_array() {
                        let have: Vec<&str> = pool["roles"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .collect();
                        roles
                            .iter()
                            .filter_map(Value::as_str)
                            .all(|r| have.contains(&r))
                    } else {
                        n["resource"].as_str() == Some(pid)
                    }
                })
        };
        let mut pools: Vec<(String, String)> = model["resources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| drawn(*p))
            .filter_map(|p| {
                let pid = p["id"].as_str()?.to_owned();
                let name = p["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&pid);
                Some((pid.clone(), name.to_owned()))
            })
            .collect();
        // A time for a pool the model does not have also gets a row, named by its id.
        for b in &entries {
            if let Some(rid) = b["resource"].as_str()
                && !pools.iter().any(|(p, _)| p == rid)
            {
                pools.push((rid.to_owned(), rid.to_owned()));
            }
        }
        let entry_of = |pid: &str| {
            entries
                .iter()
                .position(|b| b["resource"].as_str() == Some(pid))
        };
        let mut changed: Option<Vec<Value>> = None;
        let mut schema = params_schema("block.set");
        schema["properties"]["service"]["description"] = Value::Null;
        let unit = self.time_unit_label();
        for (pid, name) in &pools {
            let at = entry_of(pid);
            let mut own = at.is_some();
            ui.add_space(4.0);
            if ui.checkbox(&mut own, name.as_str()).changed() {
                let mut all = entries.clone();
                match (at, own) {
                    // The task's own time to start from, which the user then changes.
                    (None, true) => all.push(json!({ "resource": pid, "time": el["service"] })),
                    (Some(i), false) => {
                        all.remove(i);
                    }
                    _ => {}
                }
                changed = Some(all);
            }
            if let Some(i) = at {
                let mut cx = form::Cx {
                    time_unit: &unit,
                    numbers: &self.numbers,
                    drafts: &mut self.drafts,
                    calendars: &[],
                };
                ui.indent((&id, "by_resource", pid), |ui| {
                    if let Some(v) = form::field(
                        ui,
                        &mut cx,
                        Schema::new(&schema),
                        &format!("{id}/processing_by_resource/{pid}"),
                        "service",
                        &entries[i]["time"],
                        true,
                    ) {
                        let mut all = entries.clone();
                        all[i]["time"] = v;
                        changed = Some(all);
                    }
                });
            }
        }
        if let Some(all) = changed {
            let value = if all.is_empty() {
                Value::Null
            } else {
                json!({ "by_resource": all })
            };
            self.execute(
                "block.set",
                json!({ "id": id, "processing_by_resource": value }),
            );
        }
    }

    /// Adds a task with shared resources (`params` are `block.add`'s), needing one unit of the
    /// first pool. With no pool yet, a new one is added with it, in one step that Undo takes
    /// back whole. The `block.add` result, or None with the error shown.
    pub(crate) fn add_shared_task(&mut self, mut params: Value) -> Option<Value> {
        let model = self.engine.model();
        if let Some(pool) = model.resources.first() {
            params["needs"] = json!([{ "resource": pool.id.as_str(), "quantity": 1 }]);
            return self.execute("block.add", params);
        }
        // The task's need names the pool, so its id is chosen here, as the engine would choose
        // it: `resource-N` with the first N free. An id the opened file keeps for something the
        // model doesn't have is refused as a conflict, and the next is tried.
        let taken: BTreeSet<String> = model
            .elements
            .iter()
            .map(|e| e.id.as_str())
            .chain(model.connections.iter().map(|c| c.id.as_str()))
            .chain(model.resources.iter().map(|r| r.id.as_str()))
            .chain(model.calendars.iter().map(|c| c.id.as_str()))
            .map(str::to_owned)
            .collect();
        for pool in (1u64..)
            .map(|n| format!("resource-{n}"))
            .filter(|id| !taken.contains(id))
        {
            params["needs"] = json!([{ "resource": pool, "quantity": 1 }]);
            let batch = json!({ "commands": [
                { "command": "resource.add", "params": { "id": pool } },
                { "command": "block.add", "params": params },
            ] });
            match self.engine.execute("batch", batch) {
                Ok(r) => return Some(r["results"][1].clone()),
                Err(e) if e.code == ErrorCode::Conflict => {}
                Err(e) => {
                    self.command_failed(e);
                    return None;
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::roles_of;

    #[test]
    fn roles_are_split_at_commas() {
        assert_eq!(roles_of(" nurse, senior ,, "), ["nurse", "senior"]);
        assert!(roles_of("").is_empty());
    }
}
