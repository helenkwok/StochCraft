//! Runs from another tool in the window (M6.0): a CSV of per-replication results opened from the
//! File menu or dropped on the window, its columns mapped, and the answers shown as the Results
//! tab shows a run's. The tables are the engine's fixtures in `tests/data/`, whose answers
//! `crates/engine/tests/data.rs` works out from the formulas.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use stochcraft_ui_egui::commands::{self, CommandRef};
use stochcraft_ui_egui::{App, EXAMPLES, Platform, Request};

const WIDE: &[u8] = include_bytes!("../../../tests/data/runs-wide.csv");
const LONG: &[u8] = include_bytes!("../../../tests/data/runs-long.csv");
const SPLIT: &[u8] = include_bytes!("../../../tests/data/runs-split.csv");
const STEP: &[u8] = include_bytes!("../../../tests/data/series-step.csv");

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

fn window() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 1600.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    settle(&mut h);
    h
}

/// How many widgets are labelled exactly `label`.
fn count(h: &Harness<'static, App>, label: &str) -> usize {
    h.get_all_by_label(label).count()
}

/// The File menu asks the shell for a table of runs; the table opens straight to its answers:
/// what the runs found first, how sure the numbers are, and each scenario against the baseline
/// with its verdict. Unpaired (Welch), the two desks' wait is no difference detected (−1.6 ± 2.1
/// holds zero) and the number served differs.
#[test]
fn a_table_of_runs_opens_from_the_file_menu_to_its_answers() {
    let mut h = window();
    h.state_mut()
        .run_command(&CommandRef::new(commands::ANALYSE_RUNS));
    assert_eq!(h.state_mut().take_requests(), vec![Request::PickRuns]);
    h.state_mut().open_runs("runs-wide.csv", WIDE);
    settle(&mut h);
    assert_eq!(h.state().status(), "Opened runs-wide.csv");
    h.get_by_label("Runs from another tool");
    h.get_by_label("What the run found");
    assert!(
        h.query_by_label_contains("served (two desks) rose from")
            .is_some(),
        "the difference in the number served is a finding"
    );
    h.get_by_label("How sure are these numbers?");
    assert!(h.query_by_label_contains("runs would pin it to").is_some());
    h.get_by_label("Each scenario against one desk");
    assert_eq!(count(&h, "Differs"), 1, "served differs");
    assert_eq!(
        count(&h, "No difference detected"),
        1,
        "the wait could be chance"
    );
    // Same ids in both scenarios: pairing is offered, never assumed.
    assert!(
        h.query_by_label_contains("Every scenario has the same run numbers")
            .is_some()
    );
}

/// Declared paired, the wait is compared run by run (−1.6 ± 0.7), which leaves out zero: both
/// measures differ, and the table says the difference is paired.
#[test]
fn declaring_pairs_tightens_the_comparison() {
    let mut h = window();
    h.state_mut().open_runs("runs-wide.csv", WIDE);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Change the columns")
        .click();
    settle(&mut h);
    h.get_by_label("What each column holds");
    h.get_by_role_and_label(
        Role::CheckBox,
        "These scenarios used common random numbers (paired)",
    )
    .click();
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Show the answers")
        .click();
    settle(&mut h);
    h.get_by_label("Difference (paired, 95%)");
    assert_eq!(count(&h, "Differs"), 2, "both measures differ in pairs");
    assert!(
        h.query_by_label_contains("Compared in pairs, as declared")
            .is_some()
    );
}

/// A long table, semicolon-delimited with decimal commas, is guessed as long and gives the same
/// verdicts as the wide one.
#[test]
fn a_long_table_gives_the_same_verdicts() {
    let mut h = window();
    h.state_mut().open_runs("runs-long.csv", LONG);
    settle(&mut h);
    assert_eq!(count(&h, "Differs"), 1);
    assert_eq!(count(&h, "No difference detected"), 1);
    h.get_by_role_and_label(Role::Button, "Change the columns")
        .click();
    settle(&mut h);
    let long = h.get_by_role_and_label(Role::RadioButton, "One row per value");
    assert_eq!(
        long.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
}

/// Runs that fall into two outcomes are a finding, and the table says the mean mixes them.
#[test]
fn runs_that_split_are_a_finding() {
    let mut h = window();
    h.state_mut().open_runs("runs-split.csv", SPLIT);
    settle(&mut h);
    assert!(
        h.query_by_label_contains("backlog (runs-split.csv) split into two outcomes")
            .is_some()
    );
    assert!(h.query_by_label_contains("mixes two outcomes").is_some());
}

/// A time series shows the averaged series with the MSER-5 cut: the start-up of ten times ends
/// at time 11.
#[test]
fn a_time_series_shows_its_warm_up() {
    let mut h = window();
    h.state_mut().open_runs("series-step.csv", STEP);
    settle(&mut h);
    h.get_by_label("Warm-up");
    h.get_by_label("Averaged over the runs at each time, with the warm-up cut marked");
    assert!(
        h.query_by_label_contains("MSER-5 suggests a warm-up of 11:")
            .is_some()
    );
}

/// A table dropped on the window opens without asking about the model's unsaved changes, and
/// leaves the model as it was; Close goes back to it.
#[test]
fn a_dropped_table_leaves_the_model_alone() {
    let mut h = window();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    h.state_mut().execute(
        "block.set",
        serde_json::json!({"id": "serve", "capacity": 2}),
    );
    settle(&mut h);
    assert!(h.state().has_unsaved_changes());
    h.state_mut().drop_file("runs-wide.csv", WIDE.to_vec());
    settle(&mut h);
    assert!(h.query_by_label_contains("Save changes to").is_none());
    h.get_by_label("Runs from another tool");
    assert!(h.state().has_unsaved_changes(), "the model is still there");
    h.get_by_role_and_label(Role::Button, "Close").click();
    settle(&mut h);
    assert!(h.query_by_label("Runs from another tool").is_none());
}
