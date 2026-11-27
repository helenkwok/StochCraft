//! Runs from another tool in the window (M6.0): a CSV of per-replication results opened from the
//! File menu, the browser's picker or dropped on the window, its columns mapped, and the answers
//! shown as the Results tab shows a run's. The tables are the engine's fixtures in `tests/data/`,
//! whose answers `crates/engine/tests/data.rs` works out from the formulas.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
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

/// The table opened, on its mapping step.
fn opened(name: &str, bytes: &[u8]) -> Harness<'static, App> {
    let mut h = window();
    h.state_mut().open_runs(name, bytes);
    settle(&mut h);
    h.get_by_label("What each column holds");
    h
}

fn click(h: &mut Harness<'static, App>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    settle(h);
}

fn show_answers(h: &mut Harness<'static, App>) {
    click(h, Role::Button, "Show the answers");
}

/// Picks `option` from the drop-down labelled `label`.
fn choose(h: &mut Harness<'static, App>, label: &str, option: &str) {
    click(h, Role::ComboBox, label);
    h.get_all_by_label(option)
        .last()
        .unwrap_or_else(|| panic!("no option {option:?}"))
        .click();
    settle(h);
}

fn type_into(h: &mut Harness<'static, App>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label).click();
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    settle(h);
    h.key_press(egui::Key::Enter);
    settle(h);
}

/// How many widgets are labelled exactly `label`.
fn count(h: &Harness<'static, App>, label: &str) -> usize {
    h.get_all_by_label(label).count()
}

/// The File menu asks the shell for a table of runs; the table opens on its mapping step with
/// the roles guessed, then shows its answers: what the runs found first, how sure the numbers
/// are, and each scenario against the baseline with its verdict. Unpaired (Welch), the two
/// desks' wait is no difference detected (−1.6 ± 2.1 holds zero) and the number served differs.
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
    h.get_by_label("What each column holds");
    let wide = h.get_by_role_and_label(Role::RadioButton, "One row per run");
    assert_eq!(
        wide.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    show_answers(&mut h);
    h.get_by_label("What the run found");
    assert!(
        h.query_by_label_contains("served (two desks) rose from")
            .is_some(),
        "the difference in the number served is a finding"
    );
    h.get_by_label("How sure are these numbers?");
    assert!(h.query_by_label_contains("runs would pin it to").is_some());
    // The wait is wide in both scenarios, the number served settled in both: one measure of two.
    assert!(
        h.query_by_label_contains("1 of 2 measures are settled")
            .is_some()
    );
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
    let mut h = opened("runs-wide.csv", WIDE);
    click(
        &mut h,
        Role::CheckBox,
        "These scenarios used common random numbers (paired)",
    );
    show_answers(&mut h);
    h.get_by_label("Difference (paired, 95%)");
    assert_eq!(count(&h, "Differs"), 2, "both measures differ in pairs");
    assert!(
        h.query_by_label_contains("Compared in pairs, as declared")
            .is_some()
    );
    // Back to the mapping, and the choice is kept.
    click(&mut h, Role::Button, "Change the columns");
    let paired = h.get_by_role_and_label(
        Role::CheckBox,
        "These scenarios used common random numbers (paired)",
    );
    assert_eq!(
        paired.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
}

/// A tolerance set in advance: paired, the wait's −1.6 ± 0.68 lies wholly within ±3, so it
/// agrees; against ±1 it straddles the edge, inconclusive.
#[test]
fn a_tolerance_turns_the_verdict_into_agreement_or_inconclusive() {
    let mut h = opened("runs-wide.csv", WIDE);
    click(
        &mut h,
        Role::CheckBox,
        "These scenarios used common random numbers (paired)",
    );
    type_into(&mut h, "wait", "3");
    show_answers(&mut h);
    assert_eq!(count(&h, "Agrees within ±3"), 1);
    assert!(
        h.query_by_label_contains("wait (two desks) agrees with one desk within ±3")
            .is_some()
    );
    click(&mut h, Role::Button, "Change the columns");
    type_into(&mut h, "wait", "1");
    show_answers(&mut h);
    assert_eq!(count(&h, "Inconclusive at current precision"), 1);
}

/// Another baseline: two desks, against which one desk's served is lower.
#[test]
fn the_baseline_can_be_chosen() {
    let mut h = opened("runs-wide.csv", WIDE);
    choose(&mut h, "Baseline", "two desks");
    show_answers(&mut h);
    h.get_by_label("Each scenario against two desks");
    assert!(
        h.query_by_label_contains("served (one desk) fell from")
            .is_some()
    );
}

/// A column the engine takes by its name is still taken: the `scenario` column set to Not used
/// is still the scenario (the mapping shows it as such again), and the scenarios are compared.
#[test]
fn the_mapping_shows_the_columns_the_answer_used() {
    let mut h = opened("runs-wide.csv", WIDE);
    choose(&mut h, "scenario", "Not used");
    h.get_by_role_and_label(Role::ComboBox, "Baseline");
    // A text column set as a measure is an error, said on the mapping step; Show the answers waits.
    choose(&mut h, "note", "Measure");
    assert!(h.query_by_label_contains("not a number").is_some());
    choose(&mut h, "note", "Not used");
    show_answers(&mut h);
    h.get_by_label("Each scenario against one desk");
}

/// A long table, semicolon-delimited with decimal commas, is guessed as long and gives the same
/// verdicts as the wide one.
#[test]
fn a_long_table_gives_the_same_verdicts() {
    let mut h = opened("runs-long.csv", LONG);
    let long = h.get_by_role_and_label(Role::RadioButton, "One row per value");
    assert_eq!(
        long.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    show_answers(&mut h);
    assert_eq!(count(&h, "Differs"), 1);
    assert_eq!(count(&h, "No difference detected"), 1);
}

/// Runs that fall into two outcomes are a finding, and the table says the mean mixes them.
#[test]
fn runs_that_split_are_a_finding() {
    let mut h = opened("runs-split.csv", SPLIT);
    show_answers(&mut h);
    assert!(
        h.query_by_label_contains("backlog (runs-split.csv) split into two outcomes")
            .is_some()
    );
    assert!(h.query_by_label_contains("mixes two outcomes").is_some());
}

/// A time series is guessed by its time column and shows the averaged series with the MSER-5
/// cut: the start-up of ten times ends at time 11.
#[test]
fn a_time_series_shows_its_warm_up() {
    let mut h = opened("series-step.csv", STEP);
    let series = h.get_by_role_and_label(Role::RadioButton, "A time series");
    assert_eq!(
        series.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    show_answers(&mut h);
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
    click(&mut h, Role::Button, "Close");
    assert!(h.query_by_label("Runs from another tool").is_none());
}

/// The browser reads a picked file in the background: a table that arrives opens over the model,
/// unless another file was asked for meanwhile.
#[test]
fn a_table_read_in_the_background_opens_when_it_arrives() {
    let mut h = window();
    let stale = h.state_mut().begin_open();
    let ticket = h.state_mut().begin_open();
    h.state_mut()
        .finish_open(stale, "runs-wide.csv", WIDE.to_vec(), false);
    settle(&mut h);
    assert!(h.query_by_label("Runs from another tool").is_none());
    h.state_mut()
        .finish_open(ticket, "runs-wide.csv", WIDE.to_vec(), false);
    settle(&mut h);
    h.get_by_label("Runs from another tool");
}
