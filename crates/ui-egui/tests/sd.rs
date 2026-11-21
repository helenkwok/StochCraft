//! System dynamics models in the window, as a user would use them: the example opened from the
//! start page, its variables listed on the Model tab, a run, the chart and the table on the
//! Results tab; a model with a random function run in replications, with the band; and Save
//! refused, so an XMILE file is never overwritten with the engine's empty process model
//! (egui_kittest, through the accessibility tree; nothing is rendered).

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use stochcraft_engine::Engine;
use stochcraft_ui_egui::{App, Autosave, EXAMPLES, Example, Platform, Request};

/// The app with English numbers and words, so the tests read the same in every region.
fn english(mut app: App) -> App {
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    app
}

/// Tall enough that a whole page is on screen.
fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 3000.0))
        .build_ui_state(
            |ui, app: &mut App| app.ui(ui),
            english(App::new(Platform::DESKTOP)),
        );
    h.run();
    h
}

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// Clicks the last widget labelled `label` with `role`.
fn click(h: &mut Harness<'static, App>, role: Role, label: &str) {
    h.get_all_by_role_and_label(role, label)
        .last()
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
        .click();
    settle(h);
}

/// Replaces the text of the first field labelled `label` and presses Enter.
fn type_into(h: &mut Harness<'static, App>, role: Role, label: &str, text: &str) {
    let first =
        |h: &Harness<'static, App>| h.get_all_by_role_and_label(role, label).next().is_some();
    assert!(first(h), "no {role:?} labelled {label:?}");
    if let Some(n) = h.get_all_by_role_and_label(role, label).next() {
        n.click();
    }
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    if let Some(n) = h.get_all_by_role_and_label(role, label).next() {
        n.type_text(text);
    }
    settle(h);
    h.key_press(egui::Key::Enter);
    settle(h);
}

/// The built-in system dynamics example.
fn adoption() -> &'static Example {
    EXAMPLES
        .iter()
        .find(|e| e.file == "adoption.xmile")
        .expect("the adoption example")
}

/// A model with a random function: a stock that adds a standard normal draw at every step.
const NOISE: &str = "<xmile version=\"1.0\"><sim_specs><start>0</start><stop>50</stop><dt>1</dt></sim_specs><model><variables><stock name=\"w\"><eqn>0</eqn><inflow>n</inflow></stock><flow name=\"n\"><eqn>NORMAL(0, 1)</eqn></flow></variables></model></xmile>";

fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect()
}

static NULL: Value = Value::Null;

fn variable<'a>(results: &'a Value, name: &str) -> &'a Value {
    results["variables"]
        .as_array()
        .and_then(|vs| vs.iter().find(|v| v["name"] == name))
        .unwrap_or(&NULL)
}

/// Runs the model on screen: Run is made on the frame after it is pressed.
fn run(h: &mut Harness<'static, App>) {
    h.state_mut().run();
    h.step();
    h.step();
    settle(h);
}

/// The bundled adoption model follows the exact solution of its equations: a potential customer
/// buys on their own at the rate p, or after meeting a customer at the rate q, so with no one
/// buying at the start the customers are N (1 - e^-(p+q)t) / (1 + (q/p) e^-(p+q)t).
#[test]
fn the_adoption_example_follows_its_exact_solution() {
    let e = adoption();
    let mut engine = Engine::new();
    let opened = engine.open_bytes(e.file, e.bytes, None).expect("it opens");
    assert_eq!(opened["kind"], "system_dynamics");
    // Nothing the checks or the reader have to say, so the Model tab has nothing to warn of.
    assert_eq!(opened["diagnostics"], json!([]));
    assert_eq!(opened["notes"], json!([]));
    let r = engine.execute("sd.run", json!({})).expect("it runs");
    assert_eq!(r["random"], false);
    assert_eq!(r["replications"], 1);

    let (p, q, n) = (0.03, 0.38, 10_000.0);
    let times = floats(&r["times"]);
    let customers = floats(&variable(&r, "Customers")["mean"]);
    let potential = floats(&variable(&r, "Potential customers")["mean"]);
    assert_eq!(times.len(), 49);
    assert_eq!(times.len(), customers.len());
    for ((t, a), left) in times.iter().zip(&customers).zip(&potential) {
        let decay = stochcraft_random::math::exp(-(p + q) * t);
        let exact = n * (1.0 - decay) / (1.0 + (q / p) * decay);
        assert!((a - exact).abs() < 0.01, "at {t}: {a} against {exact}");
        // Whoever has not bought is still waiting: nobody is lost.
        assert!((a + left - n).abs() < 1e-6, "at {t}: {a} + {left}");
    }
    assert_eq!(times.last().copied(), Some(12.0));
}

#[test]
fn the_example_opens_runs_and_charts_its_stocks() {
    let mut h = harness();
    let e = adoption();
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    assert!(h.state().is_system_dynamics());
    assert!(h.state().status().starts_with("Opened adoption.xmile"));

    // Its own tabs: the model and its results, and none of a process model's views.
    assert!(h.query_by_role_and_label(Role::Tab, "Model").is_some());
    assert!(h.query_by_role_and_label(Role::Tab, "Results").is_some());
    assert!(h.query_by_role_and_label(Role::Tab, "3D").is_none());
    assert!(h.query_all_by_label("System dynamics model").count() >= 1);
    // Read-only for now, and says so.
    h.get_by_label_contains("Editing comes next");
    // Its settings, in a sentence, and its variables in a table, with a stock's flows.
    h.get_by_label_contains(
        "From 0 Years to 12 Years, in steps of 0.125 Years, by the RK4 method.",
    );
    for name in [
        "Potential customers",
        "Customers",
        "adoption",
        "innovation",
        "imitation",
    ] {
        assert!(h.query_all_by_label(name).count() >= 1, "no row for {name}");
    }
    assert!(h.query_all_by_label("Equation").count() >= 1);
    h.get_by_label_contains("starts at 10000");
    h.get_by_label_contains("Out: adoption");
    h.get_by_label_contains("In: adoption");
    h.get_by_label("None found. The model is ready to run.");

    // Run: a model with no random function runs once, and shows at once.
    click(&mut h, Role::Tab, "Results");
    h.get_by_label("No results yet");
    assert!(h.query_by_role_and_label(Role::TextInput, "Runs").is_none());
    run(&mut h);
    let status = h.state().status().to_owned();
    assert_eq!(status, "Finished 1 run");
    let results = h.state().sd_results().cloned().expect(&status);
    assert_eq!(results["replications"], 1);

    // The answer in words, then a chart for each stock, drawn from the values the app keeps.
    h.get_by_label_contains("Customers ends at 9,087");
    assert!(h.query_all_by_label("Customers over time").count() >= 1);
    assert!(
        h.query_all_by_label("Potential customers over time")
            .count()
            >= 1
    );
    // Flows and auxiliaries are there to pick.
    assert!(h.query_all_by_label("adoption over time").count() == 0);
    click(&mut h, Role::CheckBox, "adoption");
    assert!(h.query_all_by_label("adoption over time").count() >= 1);

    // What the chart plots at the stop time is `sd.run`'s value there.
    let plotted = h.state().sd_plotted("Customers").expect("a plot");
    let (at, value) = plotted.last().copied().expect("a last point");
    let again = h
        .state_mut()
        .execute("sd.run", json!({}))
        .expect("sd.run again");
    assert_eq!(at, again["stop"].as_f64().unwrap_or(f64::NAN));
    let end = variable(&again, "Customers")["final"]
        .as_f64()
        .expect("a final value");
    assert!((value - end).abs() < 1e-9, "{value} against {end}");
    assert!(h.state().sd_plotted_band("Customers").is_none());

    // The same values as a table, and as CSV: the variables shown.
    click(&mut h, Role::Button, "Table");
    h.get_by_label("Time (Years)");
    assert!(h.query_all_by_label("Customers").count() >= 1);
    click(&mut h, Role::Button, "Export table (CSV)");
    let requests = h.state_mut().take_requests();
    let [Request::Export { name, bytes }] = requests.as_slice() else {
        panic!("expected one export, got {requests:?}");
    };
    assert_eq!(name, "adoption-results.csv");
    let csv = String::from_utf8_lossy(bytes);
    let header = csv.lines().next().unwrap_or("");
    assert!(header.starts_with("time (Years),"), "{header}");
    assert!(header.contains("Customers") && header.contains("adoption"));
    assert_eq!(csv.lines().count(), 50, "a row for each saved time");
}

#[test]
fn a_model_with_a_random_function_runs_in_replications_with_a_band() {
    let mut h = harness();
    h.state_mut()
        .control("sd.open", json!({ "text": NOISE, "name": "noise.xmile" }))
        .expect("it opens");
    settle(&mut h);
    assert!(h.state().is_system_dynamics());
    click(&mut h, Role::Tab, "Results");
    h.get_by_label_contains("every run differs");
    // Runs and a seed to choose, for a model that has a random function.
    type_into(&mut h, Role::TextInput, "Runs", "40");
    assert!(h.query_by_role_and_label(Role::TextInput, "Seed").is_some());
    click(&mut h, Role::Button, "Run");
    let status = h.state().status().to_owned();
    let results = h.state().sd_results().cloned().expect(&status);
    assert_eq!(results["random"], true);
    assert_eq!(results["replications"], 40);
    assert_eq!(status, "Finished 40 runs");

    // The answer in words says where 9 runs in 10 end; the chart has a band and says what it is.
    h.get_by_label_contains("w ends near");
    h.get_by_label_contains("9 runs in 10 end between");
    h.get_by_label_contains("9 runs in 10 fell inside it");
    assert!(h.query_all_by_label("w over time").count() >= 1);
    let line = h.state().sd_plotted("w").expect("a line");
    let band = h.state().sd_plotted_band("w").expect("a band");
    assert_eq!(line.len(), 51);
    assert_eq!(band.len(), 51);
    // The band holds the median line, from the 5th to the 95th percentile.
    for ((_, middle), (low, high)) in line.iter().zip(&band) {
        assert!(low <= middle && middle <= high, "{low} {middle} {high}");
    }
    assert!(band.iter().skip(1).any(|(low, high)| high > low));
    // The same seed gives the same runs.
    let first = h.state().sd_results().cloned();
    click(&mut h, Role::Button, "Run again");
    assert_eq!(
        h.state().sd_results().map(|r| &r["variables"]),
        first.as_ref().map(|r| &r["variables"])
    );
}

/// Saving would write the engine's empty process model over the XMILE file, so every way of
/// saving is refused with the reason, and nothing is asked of the shell.
#[test]
fn save_is_refused_and_writes_nothing() {
    let message = "Saving system dynamics models comes with editing.";
    for platform in [Platform::DESKTOP, Platform::BROWSER] {
        let mut app = english(App::new(platform));
        let e = adoption();
        app.open_path("/models/adoption.xmile", e.bytes);
        assert!(app.is_system_dynamics());
        assert!(app.status().starts_with("Opened adoption.xmile"));

        app.save();
        assert!(app.take_requests().is_empty(), "Save asked the shell");
        assert_eq!(app.status(), message);
        app.report_error("reset");
        app.save_as();
        assert!(app.take_requests().is_empty(), "Save As asked the shell");
        assert_eq!(app.status(), message);
        app.report_error("reset");
        // An answer from the Save dialog, or from a write that was under way.
        app.save_to("/models/adoption.xmile");
        assert!(app.take_requests().is_empty());
        assert_eq!(app.status(), message);
        app.report_error("reset");
        app.written("/models/adoption.xmile", b"<xmile/>");
        assert_eq!(app.status(), message);
        assert!(app.is_system_dynamics(), "the model is still open");
        assert!(app.take_requests().is_empty());

        // Even with the empty process model beside it changed, there is nothing to save: no
        // unsaved-changes question when closing, and no recovery copy to keep.
        assert!(
            app.execute("model.rename", json!({ "name": "Renamed" }))
                .is_some()
        );
        assert!(app.engine().is_dirty());
        assert!(!app.has_unsaved_changes());
        assert!(!matches!(app.autosave(), Autosave::Write(_)));
        assert!(app.close_requested());
    }
}

#[test]
fn the_start_page_offers_the_example_among_23() {
    assert_eq!(EXAMPLES.len(), 23);
    let mut h = harness();
    h.get_by_label("See all 23 examples, including reference models")
        .click_accesskit();
    settle(&mut h);
    click(&mut h, Role::Button, "How fast does a new product spread?");
    assert!(h.state().is_system_dynamics());
    assert!(h.state().status().starts_with("Opened adoption.xmile"));
    assert!(h.query_all_by_label("System dynamics model").count() >= 1);
}

/// A file with arrays or modules is refused with the engine's message, and what was open stays.
#[test]
fn a_file_that_cannot_be_read_is_refused_with_the_engines_message() {
    let mut app = english(App::new(Platform::DESKTOP));
    let e = adoption();
    app.open_file(e.file, e.bytes);
    let arrays = "<xmile version=\"1.0\"><header><options><uses_arrays/></options></header><sim_specs><start>0</start><stop>1</stop></sim_specs><model><variables/></model></xmile>";
    app.open_file("arrays.xmile", arrays.as_bytes());
    assert!(app.status().contains("arrays"), "{}", app.status());
    assert!(app.is_system_dynamics());
}
