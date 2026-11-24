//! The decision table window (ROADMAP M5.5, D30): a block's DMN decision table as a grid, rules
//! in rows, the input columns, then the outputs, then the rule annotations, each cell its FEEL
//! text. The hit policy (and COLLECT's aggregation) is chosen above it. A cell commits when it
//! loses the keyboard; every change is one `dmn.set`, so it undoes in one step, and a table that
//! cannot run is refused with the reason in the status bar while the typed text stays.
//!
//! It opens from the block's page ("Open decision table"), in a window of its own beside the
//! inspector; the spreadsheet view (U6) is where it will also open.

use std::collections::BTreeMap;

use serde_json::json;
use stochcraft_engine::dmn::{AggregationDoc, HitPolicyDoc, RuleDoc, TableDoc};
use stochcraft_model::ElementId;

use crate::i18n::t;
use crate::{App, theme};

/// The decision table window's state while it is open.
#[derive(Debug, Default)]
pub(crate) struct Grid {
    /// The `.dmn` file and the decision's id.
    file: String,
    decision: String,
    /// Cells typed and not yet taken: (rule, column) to text, the columns numbered inputs first,
    /// then outputs, then annotations.
    typed: BTreeMap<(usize, usize), String>,
}

fn policy_name(h: HitPolicyDoc) -> &'static str {
    match h {
        HitPolicyDoc::Unique => t("Unique"),
        HitPolicyDoc::First => t("First"),
        HitPolicyDoc::Priority => t("Priority"),
        HitPolicyDoc::Collect => t("Collect"),
    }
}

fn aggregation_name(a: Option<AggregationDoc>) -> &'static str {
    match a {
        None => t("None"),
        Some(AggregationDoc::Sum) => t("Sum"),
        Some(AggregationDoc::Count) => t("Count"),
        Some(AggregationDoc::Min) => t("Minimum"),
        Some(AggregationDoc::Max) => t("Maximum"),
    }
}

/// Cell `c` of `rule`, the columns numbered as in [`Grid::typed`].
fn cell(rule: &mut RuleDoc, inputs: usize, outputs: usize, c: usize) -> Option<&mut String> {
    if c < inputs {
        rule.inputs.get_mut(c)
    } else if c < inputs + outputs {
        rule.outputs.get_mut(c - inputs)
    } else {
        rule.annotations.get_mut(c - inputs - outputs)
    }
}

impl App {
    /// Opens the decision table window on decision `decision` of the `.dmn` file `file`.
    pub(crate) fn open_decision_table(&mut self, file: &str, decision: &str) {
        self.dmn_table = Some(Grid {
            file: file.to_owned(),
            decision: decision.to_owned(),
            typed: BTreeMap::new(),
        });
    }

    /// On a block's page: the decision table it uses, if any, and a button that opens it.
    pub(crate) fn decision_link(&mut self, ui: &mut egui::Ui, id: &str) {
        let Some(u) = self
            .engine
            .model()
            .decisions
            .get(&ElementId::unchecked(id))
            .cloned()
        else {
            return;
        };
        let mut open = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::weak(ui, t("Decision table")));
            ui.label(format!("{} · {}", u.file, u.decision));
            open = ui
                .add(theme::secondary(ui, t("Open decision table")))
                .clicked();
        });
        ui.add_space(8.0);
        if open {
            self.open_decision_table(&u.file, &u.decision);
        }
    }

    /// Draws the decision table window while it is open; it closes when its table is gone (another
    /// model opened, an undo that removed it).
    pub(crate) fn decision_table_window(&mut self, ctx: &egui::Context) {
        let Some(mut grid) = self.dmn_table.take() else {
            return;
        };
        let Some(decision) = self
            .engine
            .model()
            .dmn
            .get(&grid.file)
            .and_then(|f| f.decision(&grid.decision))
            .cloned()
        else {
            return;
        };
        let mut table = TableDoc::from(&decision.table);
        let (inputs, outputs) = (table.inputs.len(), table.outputs.len());
        let heads: Vec<String> = table
            .inputs
            .iter()
            .map(|i| i.label.clone().unwrap_or_else(|| i.expression.clone()))
            .chain((0..outputs).map(|j| decision.output_name(j)))
            .chain(table.annotations.iter().cloned())
            .collect();
        let mut open = true;
        let mut changed = false;
        let mut remove = None;
        egui::Window::new(t("Decision table"))
            .id(egui::Id::unique("dmn-table"))
            .open(&mut open)
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.label(theme::strong(decision.label().to_owned()));
                ui.label(theme::weak(ui, grid.file.clone()));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(t("Hit policy"));
                    egui::ComboBox::from_id_salt("dmn-hit-policy")
                        .selected_text(policy_name(table.hit_policy))
                        .show_ui(ui, |ui| {
                            for h in [
                                HitPolicyDoc::Unique,
                                HitPolicyDoc::First,
                                HitPolicyDoc::Priority,
                                HitPolicyDoc::Collect,
                            ] {
                                if ui
                                    .selectable_label(table.hit_policy == h, policy_name(h))
                                    .clicked()
                                    && table.hit_policy != h
                                {
                                    table.hit_policy = h;
                                    if h != HitPolicyDoc::Collect {
                                        table.aggregation = None;
                                    }
                                    changed = true;
                                }
                            }
                        });
                    if table.hit_policy == HitPolicyDoc::Collect {
                        ui.label(t("Aggregation"));
                        egui::ComboBox::from_id_salt("dmn-aggregation")
                            .selected_text(aggregation_name(table.aggregation))
                            .show_ui(ui, |ui| {
                                for a in [
                                    None,
                                    Some(AggregationDoc::Sum),
                                    Some(AggregationDoc::Count),
                                    Some(AggregationDoc::Min),
                                    Some(AggregationDoc::Max),
                                ] {
                                    if ui
                                        .selectable_label(
                                            table.aggregation == a,
                                            aggregation_name(a),
                                        )
                                        .clicked()
                                        && table.aggregation != a
                                    {
                                        table.aggregation = a;
                                        changed = true;
                                    }
                                }
                            });
                    }
                });
                ui.add_space(6.0);
                egui::ScrollArea::both().max_height(360.0).show(ui, |ui| {
                    egui::Grid::new("dmn-grid").striped(true).show(ui, |ui| {
                        ui.label(theme::weak(ui, "#"));
                        for (c, h) in heads.iter().enumerate() {
                            let text = theme::strong(h.clone());
                            ui.label(if c < inputs { text } else { text.italics() });
                        }
                        ui.end_row();
                        for (r, rule) in table.rules.iter_mut().enumerate() {
                            ui.label((r + 1).to_string());
                            for c in 0..heads.len() {
                                let Some(current) = cell(rule, inputs, outputs, c) else {
                                    ui.label("");
                                    continue;
                                };
                                let mut text = grid
                                    .typed
                                    .get(&(r, c))
                                    .cloned()
                                    .unwrap_or_else(|| current.clone());
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut text)
                                        .id_salt(("dmn-cell", r, c))
                                        .desired_width(110.0),
                                );
                                if response.changed() {
                                    grid.typed.insert((r, c), text.clone());
                                }
                                if response.lost_focus() && *current != text {
                                    *current = text;
                                    changed = true;
                                }
                            }
                            if ui
                                .small_button("×")
                                .on_hover_text(t("Remove rule"))
                                .clicked()
                            {
                                remove = Some(r);
                            }
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(6.0);
                if ui.add(theme::secondary(ui, t("Add rule"))).clicked() {
                    let outputs_like = table
                        .rules
                        .last()
                        .map_or_else(|| vec!["\"\"".to_owned(); outputs], |r| r.outputs.clone());
                    table.rules.push(RuleDoc {
                        inputs: vec!["-".to_owned(); inputs],
                        outputs: outputs_like,
                        annotations: vec![String::new(); table.annotations.len()],
                        ..RuleDoc::default()
                    });
                    changed = true;
                }
            });
        if let Some(r) = remove
            && r < table.rules.len()
        {
            table.rules.remove(r);
            grid.typed.clear();
            changed = true;
        }
        if changed {
            let params = json!({
                "file": grid.file,
                "decision": grid.decision,
                "table": table,
            });
            if self.execute("dmn.set", params).is_some() {
                grid.typed.clear();
            }
        }
        if open {
            self.dmn_table = Some(grid);
        }
    }
}
