//! The decision table window (ROADMAP M5.5, D30): a block's DMN decision table as a grid, rules
//! in rows, the input columns, then the outputs, then the rule annotations, each cell its FEEL
//! text, with the hit policy (and COLLECT's aggregation) above it.
//!
//! It shows the table read-only for now: the app does not write `.dmn` files yet, so a table is
//! edited in its `.dmn` file, or with the `dmn.set` command (which `file.save` writes beside the
//! model). Editing here comes with the app writing those files.
//!
//! It opens from the block's page ("Open decision table"), in a window of its own beside the
//! inspector; the spreadsheet view (U6) is where it will also open.

use stochcraft_engine::dmn::{AggregationDoc, HitPolicyDoc, TableDoc};
use stochcraft_model::ElementId;

use crate::i18n::t;
use crate::{App, theme};

/// The decision table window's state while it is open.
#[derive(Debug, Default)]
pub(crate) struct Grid {
    /// The document it was opened on (`App::doc_gen`): it closes when that is replaced, so it
    /// never shows another model's table of the same file and id.
    doc: u64,
    /// The `.dmn` file and the decision's id.
    file: String,
    decision: String,
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

impl App {
    /// Opens the decision table window on decision `decision` of the `.dmn` file `file`.
    pub(crate) fn open_decision_table(&mut self, file: &str, decision: &str) {
        self.dmn_table = Some(Grid {
            doc: self.doc_gen,
            file: file.to_owned(),
            decision: decision.to_owned(),
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

    /// Draws the decision table window while it is open; it closes when its document is replaced
    /// (a new model, one opened, recovered or dropped in) or its table is gone (an undo that
    /// removed it).
    pub(crate) fn decision_table_window(&mut self, ctx: &egui::Context) {
        let Some(grid) = self.dmn_table.take() else {
            return;
        };
        if grid.doc != self.doc_gen {
            return;
        }
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
        let table = TableDoc::from(&decision.table);
        let (inputs, outputs) = (table.inputs.len(), table.outputs.len());
        let heads: Vec<String> = table
            .inputs
            .iter()
            .map(|i| i.label.clone().unwrap_or_else(|| i.expression.clone()))
            .chain((0..outputs).map(|j| decision.output_name(j)))
            .chain(table.annotations.iter().cloned())
            .collect();
        let mut open = true;
        egui::Window::new(t("Decision table"))
            .id(egui::Id::unique("dmn-table"))
            .open(&mut open)
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.label(theme::strong(decision.label().to_owned()));
                ui.label(theme::weak(ui, grid.file.clone()));
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::weak(ui, t("Hit policy")));
                    ui.label(policy_name(table.hit_policy));
                    if table.hit_policy == HitPolicyDoc::Collect {
                        ui.label(theme::weak(ui, t("Aggregation")));
                        ui.label(aggregation_name(table.aggregation));
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
                        for (r, rule) in table.rules.iter().enumerate() {
                            ui.label((r + 1).to_string());
                            let cells = rule
                                .inputs
                                .iter()
                                .chain(&rule.outputs)
                                .chain(&rule.annotations);
                            for text in cells.take(heads.len()) {
                                ui.label(egui::RichText::new(text).monospace());
                            }
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(6.0);
                ui.label(theme::weak(
                    ui,
                    t("Shown read-only for now: edit the table in its .dmn file, or with the dmn.set command."),
                ));
            });
        if open {
            self.dmn_table = Some(grid);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{App, Platform};

    fn with_table() -> App {
        let mut app = App::new(Platform::DESKTOP);
        let example = stochcraft_engine::COMMANDS
            .iter()
            .find(|c| c.name == "model.set")
            .map(|c| c.example())
            .unwrap_or_default();
        assert!(app.execute("model.set", example).is_some());
        let table = json!({
            "file": "desk.dmn",
            "decision": "priority",
            "table": {
                "inputs": [{ "expression": "size" }],
                "outputs": [{ "name": "priority" }],
                "rules": [{ "inputs": ["< 10"], "outputs": ["1"] }, { "inputs": [">= 10"], "outputs": ["2"] }]
            }
        });
        assert!(app.execute("dmn.set", table).is_some());
        app.open_decision_table("desk.dmn", "priority");
        app
    }

    /// Another document closes the window, even with a table of the same file and id.
    #[test]
    fn the_window_closes_when_its_document_is_replaced() {
        let mut app = with_table();
        app.forget_view();
        assert!(app.dmn_table.is_some());
        app.decision_table_window(&egui::Context::default());
        assert!(app.dmn_table.is_none());
    }

    /// The app writes the model's file only: saving a model with tables says they were not
    /// written, and leaves the model unsaved while a table is not on disk.
    #[test]
    fn a_save_says_the_tables_were_not_written() {
        let mut app = with_table();
        let bytes = app.engine.save_bytes(true).unwrap_or_default();
        app.written("desk.bpmn", &bytes);
        let status = app.status().to_owned();
        assert!(status.starts_with("Saved desk.bpmn. "), "{status}");
        assert!(status.contains(".dmn files"), "{status}");
        assert!(app.unsaved_changes(), "the table is not on disk");
        // A model without tables is just saved.
        let mut plain = App::new(Platform::DESKTOP);
        let bytes = plain.engine.save_bytes(true).unwrap_or_default();
        plain.written("plain.bpmn", &bytes);
        assert_eq!(plain.status(), "Saved plain.bpmn");
        assert!(!plain.unsaved_changes());
    }

    /// The window shows the table and leaves it as it is: nothing it draws changes the model.
    #[test]
    fn the_window_shows_the_table_without_changing_it() {
        let app = with_table();
        let before = app.engine.model().clone();
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui_state(
                |ui, app: &mut App| {
                    let ctx = ui.ctx().clone();
                    app.decision_table_window(&ctx);
                },
                app,
            );
        h.run_steps(3);
        let app = h.state();
        assert!(app.dmn_table.is_some());
        assert!(app.engine.model() == &before);
    }
}
