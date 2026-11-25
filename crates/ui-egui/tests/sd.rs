//! System dynamics models in the window, as a user would use them: the example opened from the
//! start page, its variables listed on the Model tab, a run, the chart and the table on the
//! Results tab; a model with a random function run in replications, with the band; a model
//! changed by a command, which is unsaved, saves as XMILE that reopens to the change, and whose
//! results are marked out of date (egui_kittest, through the accessibility tree; nothing is rendered).

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
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

/// Runs the model on screen: the frame after Run is pressed paints "Running…", the ones after
/// make the replications, a few a frame; this steps until the run is over.
fn run(h: &mut Harness<'static, App>) {
    h.state_mut().run();
    h.step();
    for _ in 0..20_000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    assert!(!h.state().is_running(), "the run did not finish");
    settle(h);
}

/// The equation of variable `name` in the model that the XMILE `bytes` hold.
fn equation_in(bytes: &[u8], name: &str) -> String {
    let mut engine = Engine::new();
    engine
        .open_bytes("saved.xmile", bytes, None)
        .expect("the saved bytes open as a model");
    let info = engine.execute("sd.get", json!({})).expect("sd.get");
    info["variables"]
        .as_array()
        .and_then(|vs| vs.iter().find(|v| v["name"] == name))
        .and_then(|v| v["equation"].as_str())
        .unwrap_or("")
        .to_owned()
}

/// Changes the adoption rate of the open model, as an agent's command does.
fn change_innovation(app: &mut App) {
    app.control(
        "sd.variable.set",
        json!({ "name": "innovation", "equation": "0.05" }),
    )
    .expect("the edit applies");
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
    // A column that fell short would end the comparison below early, unseen.
    assert_eq!(potential.len(), times.len());
    assert_eq!(customers.len(), times.len());
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
    // It says where the model can be changed: on the Diagram tab, or by commands.
    h.get_by_label_contains("change this model on the Diagram tab");
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

    // The answer in words gives the median and the percentiles it ends between; the chart has a
    // band and says what it is.
    h.get_by_label_contains("w ends near");
    h.get_by_label_contains("(median); its 5th to 95th percentiles at the end are");
    h.get_by_label_contains("Shaded band: 5th to 95th percentile, at each saved time.");
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
    // Another seed gives other runs, and says which seed they are of; the first seed again gives
    // the first runs again.
    let first = h.state().sd_results().cloned().expect("the first results");
    assert_eq!(first["seed"], 1);
    type_into(&mut h, Role::TextInput, "Seed", "7");
    click(&mut h, Role::Button, "Run again");
    let other = h.state().sd_results().cloned().expect("results of seed 7");
    assert_eq!(other["seed"], 7);
    assert_ne!(other["variables"], first["variables"]);
    type_into(&mut h, Role::TextInput, "Seed", "1");
    click(&mut h, Role::Button, "Run again");
    let again = h.state().sd_results().cloned().expect("results of seed 1");
    assert_eq!(again["seed"], 1);
    assert_eq!(again["variables"], first["variables"]);
}

/// A model changed by a command is unsaved; Save writes XMILE (the engine's), which reopens to
/// the change; once the shell has written it, nothing is unsaved. In the browser the bytes are
/// offered as a download under the file's name, and the model stays marked unsaved, as it is
/// not known whether the user kept the file.
#[test]
fn a_changed_model_saves_as_xmile_that_reopens_to_the_change() {
    for platform in [Platform::DESKTOP, Platform::BROWSER] {
        let desktop = platform.file_system;
        let mut app = english(App::new(platform));
        let e = adoption();
        app.open_path("/models/adoption.xmile", e.bytes);
        assert!(app.is_system_dynamics());
        assert!(app.status().starts_with("Opened adoption.xmile"));
        assert!(!app.has_unsaved_changes());
        assert!(!app.title().contains('•'));

        change_innovation(&mut app);
        assert!(app.has_unsaved_changes());
        assert!(app.title().contains('•'));
        assert!(!matches!(app.autosave(), Autosave::Write(_)));

        app.save();
        let requests = app.take_requests();
        let bytes = match requests.as_slice() {
            [Request::Write { path, bytes }] if desktop => {
                assert_eq!(path, "/models/adoption.xmile");
                bytes
            }
            [Request::Download { name, bytes }] if !desktop => {
                assert_eq!(name, "/models/adoption.xmile");
                bytes
            }
            other => panic!("expected one write or download, got {other:?}"),
        };
        let text = String::from_utf8_lossy(bytes);
        assert!(text.contains("<xmile"), "not XMILE: {text}");
        assert_eq!(equation_in(bytes, "innovation"), "0.05");
        assert_eq!(equation_in(e.bytes, "innovation"), "0.03");

        // Nothing is saved until the shell says so.
        assert!(app.has_unsaved_changes());
        if desktop {
            app.written("/models/adoption.xmile", bytes);
            assert_eq!(app.status(), "Saved adoption.xmile");
            assert!(!app.has_unsaved_changes());
            assert!(!app.title().contains('•'));
            assert!(app.close_requested());
        } else {
            app.downloaded("/models/adoption.xmile");
            assert!(app.has_unsaved_changes());
        }
        assert!(app.is_system_dynamics(), "the model is still open");

        // Edited again, it is unsaved again.
        app.control(
            "sd.variable.set",
            json!({ "name": "innovation", "equation": "0.06" }),
        )
        .expect("the second edit applies");
        assert!(app.has_unsaved_changes());
    }
}

/// Save As suggests, and writes, an XMILE file's name.
#[test]
fn save_as_names_an_xmile_file() {
    let mut app = english(App::new(Platform::DESKTOP));
    let e = adoption();
    app.open_file(e.file, e.bytes);
    change_innovation(&mut app);
    app.save_as();
    let requests = app.take_requests();
    assert!(
        matches!(
            requests.as_slice(),
            [Request::PickSave { name, xpdl: false }] if name == "adoption.xmile"
        ),
        "{requests:?}"
    );
    // A name that is not an XMILE file's is refused, never rewritten: the dialog confirmed that
    // file and no other, so writing `copy.xmile` could replace one nobody agreed to replace.
    for name in ["/models/copy.bpmn", "/models/copy"] {
        app.save_to(name);
        assert_eq!(app.take_requests(), vec![], "{name}: nothing is written");
        assert!(
            app.status()
                .contains("saved as XMILE; choose a name ending in .xmile"),
            "{}",
            app.status()
        );
        assert!(app.has_unsaved_changes());
    }
    // A name ending .xmile (or .stmx), in any case, is written as chosen.
    app.save_to("/models/Copy.XMILE");
    let requests = app.take_requests();
    let [Request::Write { path, bytes }] = requests.as_slice() else {
        panic!("expected one write, got {requests:?}");
    };
    assert_eq!(path, "/models/Copy.XMILE");
    assert_eq!(equation_in(bytes, "innovation"), "0.05");
    app.written(path, bytes);
    assert!(!app.has_unsaved_changes());
    assert!(app.title().starts_with("Copy.XMILE"), "{}", app.title());
    app.save_to("/models/copy.stmx");
    assert!(matches!(
        app.take_requests().as_slice(),
        [Request::Write { path, .. }] if path == "/models/copy.stmx"
    ));
}

/// A model whose file has a second diagram, which the reader does not keep.
const TWO_VIEWS: &str = "<xmile version=\"1.0\" xmlns=\"http://docs.oasis-open.org/xmile/ns/XMILE/v1.0\"><header><name>Two</name></header><sim_specs><start>0</start><stop>10</stop><dt>1</dt></sim_specs><model><variables><stock name=\"s\"><eqn>1</eqn><inflow>f</inflow></stock><flow name=\"f\"><eqn>0.1</eqn></flow></variables><views><view type=\"stock_flow\"><stock name=\"s\" x=\"100\" y=\"100\"/></view><view type=\"stock_flow\"><stock name=\"s\" x=\"100\" y=\"100\"/></view></views></model></xmile>";

/// Save into a file that has more than the model keeps asks first, naming what would be lost,
/// and writes nothing until it is answered; Replace writes, Save As keeps the original, Cancel
/// writes nothing. A file that lost nothing saves at once, and so does a replaced one.
#[test]
fn saving_into_a_file_that_lost_content_asks_first() {
    let mut h = harness();
    h.state_mut()
        .open_path("/models/two.xmile", TWO_VIEWS.as_bytes());
    // Changed, so saving writes the model and leaves out the second diagram (an unchanged save
    // writes the file's own bytes and loses nothing).
    h.state_mut()
        .control("sd.variable.set", json!({ "name": "f", "equation": "0.2" }))
        .expect("an edit");
    assert!(h.state().is_system_dynamics());
    assert!(h.state().has_unsaved_changes());

    // Replace the file anyway.
    h.state_mut().save();
    assert_eq!(h.state_mut().take_requests(), vec![], "asked first");
    settle(&mut h);
    h.get_by_label("Replace the XMILE file?");
    assert!(
        // The reason, in the question (the window lists the same note on its own as well).
        h.query_by_label_contains(
            "Saving into two.xmile would leave out what StochCraft does not keep of it: \
             only the first diagram of the model is kept"
        )
        .is_some()
    );
    assert_eq!(
        h.state_mut().take_requests(),
        vec![],
        "nothing before the answer"
    );
    click(&mut h, Role::Button, "Replace");
    let requests = h.state_mut().take_requests();
    let [Request::Write { path, bytes }] = requests.as_slice() else {
        panic!("expected one write, got {requests:?}");
    };
    assert_eq!(path, "/models/two.xmile");
    h.state_mut().written(path, bytes);
    // What the file now holds is all there is: the next Save is not asked about.
    h.state_mut().save();
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::Write { .. }]
    ));

    // Save As keeps the original.
    h.state_mut()
        .open_path("/models/two.xmile", TWO_VIEWS.as_bytes());
    // Changed, so saving writes the model and leaves out the second diagram (an unchanged save
    // writes the file's own bytes and loses nothing).
    h.state_mut()
        .control("sd.variable.set", json!({ "name": "f", "equation": "0.2" }))
        .expect("an edit");
    h.state_mut().save();
    settle(&mut h);
    click(&mut h, Role::Button, "Save As…");
    let requests = h.state_mut().take_requests();
    assert!(
        matches!(
            requests.as_slice(),
            [Request::PickSave { name, xpdl: false }] if name.ends_with(".xmile")
        ),
        "{requests:?}"
    );
    h.state_mut().save_to("/models/copy.xmile");
    let requests = h.state_mut().take_requests();
    assert!(
        matches!(
            requests.as_slice(),
            [Request::Write { path, .. }] if path == "/models/copy.xmile"
        ),
        "{requests:?}"
    );

    // Cancel writes nothing.
    h.state_mut()
        .open_path("/models/two.xmile", TWO_VIEWS.as_bytes());
    // Changed, so saving writes the model and leaves out the second diagram (an unchanged save
    // writes the file's own bytes and loses nothing).
    h.state_mut()
        .control("sd.variable.set", json!({ "name": "f", "equation": "0.2" }))
        .expect("an edit");
    h.state_mut().save();
    settle(&mut h);
    click(&mut h, Role::Button, "Cancel");
    assert_eq!(h.state_mut().take_requests(), vec![]);
    assert!(h.query_by_label("Replace the XMILE file?").is_none());

    // Save As naming the opened file replaces it as well, so it is asked about too.
    h.state_mut()
        .open_path("/models/two.xmile", TWO_VIEWS.as_bytes());
    // Changed, so saving writes the model and leaves out the second diagram (an unchanged save
    // writes the file's own bytes and loses nothing).
    h.state_mut()
        .control("sd.variable.set", json!({ "name": "f", "equation": "0.2" }))
        .expect("an edit");
    h.state_mut().save_to("/models/two.xmile");
    assert_eq!(h.state_mut().take_requests(), vec![], "asked first");
    settle(&mut h);
    h.get_by_label("Replace the XMILE file?");
    click(&mut h, Role::Button, "Cancel");
    assert_eq!(h.state_mut().take_requests(), vec![]);

    // The model is replaced while the question is open (an agent opens another file): the
    // answer would write the new model over the file asked about, so the question is dropped.
    h.state_mut()
        .open_path("/models/two.xmile", TWO_VIEWS.as_bytes());
    // Changed, so saving writes the model and leaves out the second diagram (an unchanged save
    // writes the file's own bytes and loses nothing).
    h.state_mut()
        .control("sd.variable.set", json!({ "name": "f", "equation": "0.2" }))
        .expect("an edit");
    h.state_mut().save();
    settle(&mut h);
    h.get_by_label("Replace the XMILE file?");
    h.state_mut()
        .open_path("/models/noise.xmile", NOISE.as_bytes());
    settle(&mut h);
    assert!(h.query_by_label("Replace the XMILE file?").is_none());
    assert_eq!(h.state_mut().take_requests(), vec![], "nothing written");
    assert!(
        h.state().status().contains("The model changed"),
        "{}",
        h.state().status()
    );

    // A file with nothing left out saves at once.
    h.state_mut()
        .open_path("/models/noise.xmile", NOISE.as_bytes());
    h.state_mut().save();
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::Write { path, .. }] if path == "/models/noise.xmile"
    ));
}

/// A run, then a change: the Results tab says the results are out of date, as a process model's
/// do, until it is run again.
#[test]
fn results_are_marked_out_of_date_once_the_model_is_changed() {
    let mut h = harness();
    let e = adoption();
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    click(&mut h, Role::Tab, "Results");
    run(&mut h);
    assert!(h.state().sd_results().is_some());
    assert!(h.query_by_label("Out of date:").is_none());
    assert!(h.query_by_label("· out of date").is_none());

    change_innovation(h.state_mut());
    settle(&mut h);
    h.get_by_label("Out of date:");
    h.get_by_label("· out of date");
    // The old results stay on show, marked.
    h.get_by_label_contains("Customers ends at 9,087");

    click(&mut h, Role::Button, "Run again");
    assert!(h.query_by_label("Out of date:").is_none());
    assert!(h.query_by_label("· out of date").is_none());
    assert!(
        h.state()
            .sd_results()
            .is_some_and(|r| variable(r, "Customers")["final"].as_f64().unwrap_or(0.0) > 9_087.0),
        "the new run is of the changed model"
    );
}

#[test]
fn the_start_page_offers_the_example_among_25() {
    assert_eq!(EXAMPLES.len(), 25);
    let mut h = harness();
    h.get_by_label("See all 25 examples, including reference models")
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
    // The engine's own text, naming the file and what it cannot read.
    assert!(
        app.status().contains(
            "arrays.xmile: the file uses arrays (<uses_arrays>), which are not supported in this version"
        ),
        "{}",
        app.status()
    );
    // What was open stays: the example, by its name, with nothing changed in it.
    assert!(app.is_system_dynamics());
    assert_eq!(app.engine().system_dynamics_path(), Some("adoption.xmile"));
    assert!(app.title().starts_with("adoption.xmile"), "{}", app.title());
    assert!(!app.has_unsaved_changes());
}

/// Export stays on while a run is on its way, and what it writes is still the last run's.
#[test]
fn a_table_exported_while_a_run_is_pending_is_labelled_a_prior_run() {
    let mut h = harness();
    let e = adoption();
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    run(&mut h);
    change_innovation(h.state_mut());
    settle(&mut h);
    // Run is asked for and has not been made: the old results are still on show.
    h.state_mut().run();
    h.state_mut().export_csv();
    let requests = h.state_mut().take_requests();
    let [Request::Export { name, bytes }] = requests.as_slice() else {
        panic!("expected one export, got {requests:?}");
    };
    assert_eq!(name, "adoption-results-prior-run.csv");
    let csv = String::from_utf8_lossy(bytes);
    let header = csv.lines().next().unwrap_or("");
    assert!(header.contains("run_status"), "{header}");
    for row in csv.lines().skip(1) {
        assert!(
            row.contains("prior run: the model has changed since"),
            "{row}"
        );
        // The run's numerical settings go with it: the integration method and its steps.
        for key in ["method=", "dt=", "save_step="] {
            assert!(row.contains(key), "{key} in {row}");
        }
    }

    // Once the run is made its results are the current ones, and the export is unlabelled.
    h.step();
    h.step();
    settle(&mut h);
    h.state_mut().export_csv();
    let requests = h.state_mut().take_requests();
    let [Request::Export { name, bytes }] = requests.as_slice() else {
        panic!("expected one export, got {requests:?}");
    };
    assert_eq!(name, "adoption-results.csv");
    assert!(!String::from_utf8_lossy(bytes).contains("run_status"));
}

/// The runs "n of N runs finished" says are made, if the page says so now.
fn runs_made(h: &Harness<'static, App>) -> Option<u64> {
    let node = h.query_by_label_contains("runs finished")?;
    // A plain label's text is the node's value; a button's is its label.
    let node = node.accesskit_node();
    let text = node.label().or_else(|| node.value())?;
    text.split(' ').next()?.parse().ok()
}

/// Opens the noise model on its Results tab with `runs` runs asked for.
fn noise_with_runs(runs: &str) -> Harness<'static, App> {
    let mut h = harness();
    h.state_mut()
        .control("sd.open", json!({ "text": NOISE, "name": "noise.xmile" }))
        .expect("it opens");
    settle(&mut h);
    click(&mut h, Role::Tab, "Results");
    type_into(&mut h, Role::TextInput, "Runs", runs);
    h
}

/// Steps a started run for up to `frames` frames, or until it ends.
fn step_while(h: &mut Harness<'static, App>, frames: usize) {
    for _ in 0..frames {
        if !h.state().is_running() {
            return;
        }
        h.step();
    }
}

#[test]
fn a_long_run_is_made_a_few_runs_a_frame_and_ends_with_sd_runs_numbers() {
    let mut h = noise_with_runs("20000");
    h.state_mut().run();
    assert!(h.state().is_running());
    h.step();
    let (mut seen, mut frames) = (Vec::new(), 0);
    // What the page said about the run on the first frames, to tell why if no progress is seen.
    let mut said: Vec<String> = Vec::new();
    while h.state().is_running() && frames < 20_000 {
        h.step();
        frames += 1;
        if frames <= 3 {
            said.extend(
                h.get_all_by_label_contains("run")
                    .filter_map(|n| {
                        let n = n.accesskit_node();
                        n.label().or_else(|| n.value())
                    })
                    .map(|l| format!("{frames}: {l}")),
            );
        }
        if let Some(n) = runs_made(&h)
            && seen.last() != Some(&n)
        {
            seen.push(n);
        }
    }
    assert!(!h.state().is_running(), "the run did not finish");
    // Progress was shown over several frames, and only grew.
    assert!(
        seen.len() >= 3,
        "progress seen: {seen:?} over {frames} frames; the page said {said:?}"
    );
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
    assert!(seen.iter().all(|n| *n < 20_000), "{seen:?}");
    settle(&mut h);
    let status = h.state().status().to_owned();
    let results = h.state().sd_results().cloned().expect(&status);
    assert_eq!(results["replications"], 20_000);
    assert_eq!(status.replace(',', ""), "Finished 20000 runs");
    // The same numbers as sd.run gives, which is the journal's one step.
    let mut e = Engine::new();
    e.execute("sd.open", json!({ "text": NOISE }))
        .expect("it opens");
    let whole = e
        .execute("sd.run", json!({ "replications": 20_000, "seed": 1 }))
        .expect("it runs");
    assert_eq!(results["variables"], whole["variables"]);
    let journal = h.state().engine().journal();
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0].command, "sd.run");
}

#[test]
fn cancelling_a_run_discards_what_it_made() {
    let mut h = noise_with_runs("20");
    run(&mut h);
    let first = h.state().sd_results().cloned().expect("the first results");
    type_into(&mut h, Role::TextInput, "Runs", "20000");
    h.state_mut().run();
    h.step();
    step_while(&mut h, 3);
    assert!(h.state().is_running());
    click(&mut h, Role::Button, "Cancel run");
    assert!(!h.state().is_running());
    assert_eq!(h.state().status(), "Run cancelled");
    // The results on show are still the first run's, and the cancelled run is not journalled.
    assert_eq!(h.state().sd_results(), Some(&first));
    assert_eq!(h.state().engine().journal().len(), 1);
    // The page is not left running.
    h.get_by_label("Run again");
}

#[test]
fn changing_the_model_cancels_a_run_in_progress() {
    let mut h = noise_with_runs("20000");
    h.state_mut().run();
    h.step();
    step_while(&mut h, 3);
    assert!(h.state().is_running());
    h.state_mut()
        .control("sd.specs.set", json!({ "stop": 40.0 }))
        .expect("it changes");
    step_while(&mut h, 3);
    settle(&mut h);
    assert!(!h.state().is_running());
    assert_eq!(h.state().status(), "Run cancelled");
    assert!(h.state().sd_results().is_none());
    assert!(
        h.state()
            .engine()
            .journal()
            .iter()
            .all(|j| j.command != "sd.run")
    );
}

#[test]
fn opening_another_model_cancels_a_run_in_progress() {
    let mut h = noise_with_runs("20000");
    h.state_mut().run();
    h.step();
    step_while(&mut h, 3);
    assert!(h.state().is_running());
    let e = adoption();
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    assert!(!h.state().is_running());
    assert!(h.state().sd_results().is_none());
}
