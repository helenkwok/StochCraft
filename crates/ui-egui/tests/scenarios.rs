//! A file's scenarios: the Scenario choice on the model page and in the sidebar reopens the
//! file in another of them, the values it changes show in bold, the status bar names it, and
//! unsaved changes are asked about beside the choice first.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use serde_json::{Value, json};
use stochcraft_ui_egui::{App, EXAMPLES, Platform, Request};

fn harness() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    // English words and numbers (`1,234.5`), whatever this machine's region and language.
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h
}

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The bank example, whose file holds two scenarios: "Five cashiers" (its first) and "Six
/// cashiers", which inherits the rest from it.
fn bank() -> Harness<'static, App> {
    let mut h = harness();
    let e = EXAMPLES
        .iter()
        .find(|e| e.file == "bank.bpmn")
        .expect("the bank example");
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h
}

/// The id of the open scenario.
fn open(h: &Harness<'static, App>) -> Value {
    h.state().engine().scenarios()["open"].clone()
}

fn cashiers(h: &Harness<'static, App>) -> Value {
    stochcraft_engine::wire::model(h.state().engine().model())["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|e| e["id"] == "cashiers"))
        .map(|e| e["capacity"].clone())
        .unwrap_or_default()
}

/// Opens the Scenario choice (the model page's or the sidebar's: they are the same) and picks
/// `name`.
fn choose(h: &mut Harness<'static, App>, name: &str) {
    h.get_all_by_role_and_label(Role::ComboBox, "Scenario")
        .next()
        .expect("a Scenario choice")
        .click();
    settle(h);
    h.get_by_role_and_label(Role::Button, name).click();
    settle(h);
}

/// What a field's label says to a screen reader about why it is in bold, if it is.
fn changed_note(h: &Harness<'static, App>, label: &str) -> Option<String> {
    h.query_all(By::new().role(Role::Label).label(label))
        .find_map(|n| n.accesskit_node().description())
        .and_then(|d| {
            let why = d.split_once(". ").map_or(d.as_str(), |(why, _)| why);
            why.starts_with("Changed from").then(|| why.to_owned())
        })
}

#[test]
fn the_bank_switches_to_six_cashiers_and_shows_what_changed() {
    let mut h = bank();
    // A choice on the model page and one in the sidebar's experiment summary, and the status
    // bar names the scenario.
    assert_eq!(
        h.get_all_by_role_and_label(Role::ComboBox, "Scenario")
            .count(),
        2
    );
    h.get_by_label("Scenario: Five cashiers");
    assert_eq!(open(&h), "scenario");
    assert_eq!(cashiers(&h), json!(5));

    choose(&mut h, "Six cashiers");
    assert_eq!(open(&h), "six-cashiers");
    h.get_by_label("Scenario: Six cashiers");
    assert!(h.query_by_label("Scenario: Five cashiers").is_none());
    assert!(!h.state().has_unsaved_changes());
    assert!(h.state().status().starts_with("Opened bank.bpmn"));

    // The sixth cashier is the scenario's own value, in bold against the one it inherits.
    h.get_by_role_and_label(Role::ListBoxOption, "Cashiers")
        .click();
    settle(&mut h);
    assert_eq!(cashiers(&h), json!(6));
    assert_eq!(
        changed_note(&h, "Capacity").as_deref(),
        Some("Changed from the inherited scenario")
    );
}

#[test]
fn results_of_one_scenario_are_not_shown_for_another() {
    let mut h = bank();
    h.state_mut().run();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(&mut h);
    assert!(h.state().results().is_some());
    choose(&mut h, "Six cashiers");
    assert!(
        h.state().results().is_none(),
        "the five cashiers' results went"
    );
}

#[test]
fn unsaved_changes_are_asked_about_beside_the_choice() {
    let mut h = bank();
    h.state_mut()
        .execute("experiment.set", json!({ "replications": 3 }));
    settle(&mut h);
    let question = "Opening Six cashiers drops the unsaved changes to bank.bpmn. Save them first?";

    // Cancel keeps the scenario and the changes.
    choose(&mut h, "Six cashiers");
    h.get_by_label(question);
    assert_eq!(open(&h), "scenario");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    settle(&mut h);
    assert!(h.query_by_label(question).is_none());
    assert_eq!(h.state().engine().model().experiment.replications, 3);

    // Save writes the changes into the file, then opens the other scenario of it, which
    // inherits them.
    choose(&mut h, "Six cashiers");
    h.get_by_role_and_label(Role::Button, "Save").click();
    settle(&mut h);
    let requests = h.state_mut().take_requests();
    let [Request::Write { path, bytes }] = requests.as_slice() else {
        panic!("expected one write, got {requests:?}");
    };
    h.state_mut().written(path, bytes);
    settle(&mut h);
    assert_eq!(open(&h), "six-cashiers");
    assert_eq!(cashiers(&h), json!(6));
    assert_eq!(h.state().engine().model().experiment.replications, 3);

    // Don't Save drops the changes and opens it.
    h.state_mut()
        .execute("experiment.set", json!({ "replications": 7 }));
    settle(&mut h);
    choose(&mut h, "Five cashiers");
    h.get_by_role_and_label(Role::Button, "Don't Save").click();
    settle(&mut h);
    assert_eq!(open(&h), "scenario");
    assert_eq!(h.state().engine().model().experiment.replications, 3);
    assert!(!h.state().has_unsaved_changes());
}

/// A Save before another scenario opens that can only write a new file, which holds only the
/// open scenario, saves it and does not open the other one, saying why.
#[test]
fn a_save_as_a_new_file_does_not_switch_to_a_scenario_it_does_not_hold() {
    let mut h = bank();
    // A pool and two tasks that share it: a change the bank's file can't take.
    {
        let app = h.state_mut();
        app.execute("resource.add", json!({ "id": "agents", "quantity": 2 }));
        for id in ["triage", "callback"] {
            app.execute(
                "block.add",
                json!({ "kind": "shared_task", "id": id, "needs": [{ "resource": "agents" }] }),
            );
        }
        assert_eq!(app.engine().model().resources.len(), 1, "{}", app.status());
    }
    settle(&mut h);

    choose(&mut h, "Six cashiers");
    h.get_by_role_and_label(Role::Button, "Save").click();
    settle(&mut h);
    h.get_by_label("Save as a new BPMN file?");
    // The question's own button (the last drawn, over the window).
    h.query_all_by_role_and_label(Role::Button, "Save As…")
        .last()
        .expect("the question's Save As…")
        .click();
    settle(&mut h);
    let requests = h.state_mut().take_requests();
    assert!(
        matches!(requests.as_slice(), [Request::PickSave { .. }]),
        "{requests:?}"
    );
    h.state_mut().save_to("/tmp/bank-with-a-pool.bpmn");
    let requests = h.state_mut().take_requests();
    let [Request::Write { path, bytes }] = requests.as_slice() else {
        panic!("expected one write, got {requests:?}");
    };
    h.state_mut().written(path, bytes);
    settle(&mut h);

    // Saved, and still the five cashiers with the pool: the new file does not hold the six.
    assert!(!h.state().has_unsaved_changes());
    assert_eq!(cashiers(&h), json!(5));
    assert_eq!(h.state().engine().model().resources.len(), 1);
    let listed = h.state().engine().scenarios();
    assert!(
        listed["scenarios"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|s| s["id"] != "six-cashiers"),
        "{listed}"
    );
    assert_eq!(
        h.state().status(),
        "Saved bank-with-a-pool.bpmn. It was saved as a new file that holds only the open scenario, so the switch to Six cashiers was not made."
    );
}

/// The budget example, a Monte Carlo model, with a second scenario: the first with 1,000
/// trials, not 10,000.
fn budget_with_two_scenarios() -> Harness<'static, App> {
    let e = EXAMPLES
        .iter()
        .find(|e| e.file == "budget.bpmn")
        .expect("the budget example");
    let text = std::str::from_utf8(e.bytes).expect("UTF-8");
    let close = "</bpsim:Scenario>";
    let start = text.find("<bpsim:Scenario ").expect("a scenario");
    let end = text.find(close).expect("its end") + close.len();
    let fewer = text
        .get(start..end)
        .expect("the scenario")
        .replace(
            "id=\"base\" name=\"Project budget\"",
            "id=\"fewer\" name=\"Fewer trials\"",
        )
        .replace("replication=\"10000\"", "replication=\"1000\"");
    assert!(fewer.contains("id=\"fewer\"") && fewer.contains("replication=\"1000\""));
    let file = format!(
        "{}{fewer}{}",
        text.get(..end).expect("before"),
        text.get(end..).expect("after")
    );
    let mut h = harness();
    h.state_mut()
        .open_file("budget-scenarios.bpmn", file.as_bytes());
    settle(&mut h);
    h
}

#[test]
fn a_monte_carlo_file_with_scenarios_has_the_choice_with_its_trials() {
    let mut h = budget_with_two_scenarios();
    assert_eq!(open(&h), "base");
    // The sidebar's, with the trials, and the Model tab's, under Trials and sampling.
    assert_eq!(
        h.get_all_by_role_and_label(Role::ComboBox, "Scenario")
            .count(),
        2
    );
    h.get_by_label("Scenario: Project budget");
    // In the sidebar it shows above the trials line, over neither it nor the heading.
    let sidebar = h
        .get_all_by_role_and_label(Role::ComboBox, "Scenario")
        .map(|n| n.rect())
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .expect("the sidebar's Scenario choice");
    let trials = h.get_by_label_contains("trials · seed 1").rect();
    assert!(
        sidebar.bottom() <= trials.top() + 0.5,
        "{sidebar:?} over the trials {trials:?}"
    );
    let heading = h
        .get_all_by_label("Trials")
        .map(|n| n.rect())
        .find(|r| r.left() < sidebar.right())
        .expect("the sidebar's Trials heading");
    assert!(
        heading.bottom() <= sidebar.top() + 0.5,
        "{heading:?} over the choice {sidebar:?}"
    );

    choose(&mut h, "Fewer trials");
    assert_eq!(open(&h), "fewer");
    assert_eq!(h.state().engine().model().experiment.replications, 1000);
    h.get_by_label("Scenario: Fewer trials");
    assert!(!h.state().has_unsaved_changes());
}

#[test]
fn a_file_with_one_scenario_has_no_choice() {
    let mut h = harness();
    let e = EXAMPLES
        .iter()
        .find(|e| e.file == "call-centre.bpmn")
        .expect("the call centre example");
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    assert!(
        h.query_by_role_and_label(Role::ComboBox, "Scenario")
            .is_none()
    );
    assert!(h.query_by_label_contains("Scenario:").is_none());
}

#[test]
fn the_start_page_says_which_examples_have_scenarios() {
    let h = harness();
    // The M/M/1 queue's card, which is featured, counts its file's scenarios.
    let n = stochcraft_engine::file_scenarios(EXAMPLES[0].bytes).len();
    assert!(n > 1);
    assert!(
        h.query_all_by_label(&format!("{n} scenarios to compare"))
            .count()
            >= 1
    );
    assert!(h.query_by_label("1 scenario to compare").is_none());
}

/// The sidebar's question before unsaved changes are dropped takes its own room, under the
/// sidebar's Scenario choice and above the experiment summary, overlapping neither, in a window
/// `width` wide and in `language` (a locale code).
fn the_sidebar_question_has_room(width: f32, language: Option<&str>) {
    use stochcraft_ui_egui::i18n::{t, tf, tnf};
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_language(language);
    let mut h = Harness::builder()
        .with_size(egui::vec2(width, 800.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    let e = EXAMPLES
        .iter()
        .find(|e| e.file == "bank.bpmn")
        .expect("the bank example");
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h.state_mut()
        .execute("experiment.set", json!({ "replications": 3 }));
    settle(&mut h);
    let scenario = t("Scenario");
    // The sidebar's choice is the one further left.
    let sidebar = |h: &Harness<'static, App>| {
        h.get_all_by_role_and_label(Role::ComboBox, scenario)
            .map(|n| n.rect())
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .expect("the sidebar's Scenario choice")
    };
    let at = sidebar(&h).center();
    h.get_all_by_role_and_label(Role::ComboBox, scenario)
        .find(|n| n.rect().center() == at)
        .expect("the sidebar's Scenario choice")
        .click();
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Six cashiers")
        .click();
    settle(&mut h);

    let question = tf(
        "Opening {scenario} drops the unsaved changes to {name}. Save them first?",
        &[("scenario", "Six cashiers"), ("name", "bank.bpmn")],
    );
    let summary = tnf(
        3,
        "{n} run · seed {seed}",
        "{n} runs · seed {seed}",
        &[("seed", "1")],
    );
    let rect = |h: &Harness<'static, App>, role: Role, label: &str| {
        h.get_by_role_and_label(role, label).rect()
    };
    let choice = sidebar(&h);
    // A button of the sidebar's question (the toolbar may have one of the same name).
    let button = |h: &Harness<'static, App>, label: &str| {
        h.get_all_by_role_and_label(Role::Button, label)
            .map(|n| n.rect())
            .find(|r| r.left() < choice.right())
            .unwrap_or_else(|| panic!("no {label} in the sidebar"))
    };
    let summary = rect(&h, Role::Label, &summary);
    let mut drawn = vec![(question.clone(), rect(&h, Role::Label, &question))];
    for b in [t("Save"), t("Don't Save"), t("Cancel")] {
        drawn.push((b.to_owned(), button(&h, b)));
    }
    for (i, (name, r)) in drawn.iter().enumerate() {
        assert!(
            !r.intersects(summary),
            "{name} {r:?} over the summary {summary:?}"
        );
        assert!(
            r.bottom() <= summary.top() + 0.5,
            "{name} {r:?} below the summary {summary:?}"
        );
        assert!(
            r.top() >= choice.bottom() - 0.5,
            "{name} {r:?} above the choice {choice:?}"
        );
        for (other, o) in &drawn[i + 1..] {
            assert!(!r.intersects(*o), "{name} {r:?} over {other} {o:?}");
        }
    }
    // It still answers: Cancel keeps the scenario and the changes.
    let cancel = button(&h, t("Cancel")).center();
    h.get_all_by_role_and_label(Role::Button, t("Cancel"))
        .find(|n| n.rect().center() == cancel)
        .expect("the sidebar's Cancel")
        .click();
    settle(&mut h);
    assert!(h.query_by_label(&question).is_none());
    assert_eq!(open(&h), "scenario");
    assert_eq!(h.state().engine().model().experiment.replications, 3);
}

#[test]
fn the_sidebar_question_has_room_in_english() {
    the_sidebar_question_has_room(1200.0, Some("en"));
}

#[test]
fn the_sidebar_question_has_room_in_a_narrow_window() {
    the_sidebar_question_has_room(760.0, Some("en"));
}

#[test]
fn the_sidebar_question_has_room_in_finnish_and_german() {
    the_sidebar_question_has_room(900.0, Some("fi"));
    the_sidebar_question_has_room(900.0, Some("de"));
}
