//! Shared resource pools in the window (M2.5): the model overview lists the pools, with their
//! units and roles, and a task with shared resources lists what it needs, one alternative per
//! row, tried in order. Every change is one engine command, so each undoes in one step.

use serde_json::{Value, json};

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
        for r in model["resources"].as_array().cloned().unwrap_or_default() {
            let id = r["id"].as_str().unwrap_or("").to_owned();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(theme::strong(r["name"].as_str().unwrap_or(&id)));
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
            ) {
                self.execute("resource.set", json!({ "id": id, "name": n }));
            }
            let mut cx = form::Cx {
                time_unit: &unit,
                numbers: &self.numbers,
                drafts: &mut self.drafts,
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
            ) {
                self.execute(
                    "resource.set",
                    json!({ "id": id, "roles": roles_of(&text) }),
                );
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

    /// A new task with shared resources needs a pool to draw on: the first, or a new one.
    pub(crate) fn first_pool(&mut self) -> Option<String> {
        let model = stochcraft_engine::wire::model(self.engine.model());
        if let Some(id) = model["resources"][0]["id"].as_str() {
            return Some(id.to_owned());
        }
        let r = self.execute("resource.add", json!({}))?;
        r["id"].as_str().map(str::to_owned)
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
