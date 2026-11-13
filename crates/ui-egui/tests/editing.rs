//! Editing a model in the window, as a user would: selecting blocks in the process list, the
//! inspector's forms (generated from the engine's command schemas), connections, the experiment,
//! the keyboard, and that each edit is one undo step.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

/// The app with English numbers (`1,234.5`), so the tests read the same in every region.
fn english(mut app: App) -> App {
    app.set_number_region("en");
    // The interface in English too, whatever this machine's language.
    app.set_lang(craft_i18n::Lang::EN);
    app
}

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
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

/// Opens the M/M/1 example (arrive → serve → leave).
fn mm1() -> Harness<'static, App> {
    let mut h = harness();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h
}

fn model(h: &Harness<'static, App>) -> Value {
    stochcraft_engine::wire::model(h.state().engine().model())
}

fn element(h: &Harness<'static, App>, id: &str) -> Value {
    model(h)["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|e| e["id"] == id).cloned())
        .unwrap_or_else(|| panic!("no element {id}"))
}

/// Whether a widget is outside the inspector's visible part (below the tabs, above the status
/// bar, in the 1200 × 900 window), scrolled out of sight. (Scrolling to one in sight would close
/// an open drop-down.)
fn off_screen(r: egui::Rect) -> bool {
    r.top() < 110.0 || r.bottom() > 820.0
}

/// Clicks the last widget with `role` and `label`, scrolled into view first (the inspector
/// beside the canvas scrolls).
fn click(h: &mut Harness<'static, App>, role: Role, label: &str) {
    fn last<'a>(
        h: &'a Harness<'static, App>,
        role: Role,
        label: &'a str,
    ) -> egui_kittest::Node<'a> {
        h.get_all_by_role_and_label(role, label)
            .last()
            .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
    }
    if off_screen(last(h, role, label).rect()) {
        last(h, role, label).scroll_to_me();
        settle(h);
    }
    last(h, role, label).click();
    settle(h);
}

/// Replaces the text of the field labelled `label` and presses Enter.
fn type_into(h: &mut Harness<'static, App>, label: &str, text: &str) {
    if off_screen(h.get_by_role_and_label(Role::TextInput, label).rect()) {
        h.get_by_role_and_label(Role::TextInput, label)
            .scroll_to_me();
        settle(h);
    }
    h.get_by_role_and_label(Role::TextInput, label).click();
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    settle(h);
    h.key_press(egui::Key::Enter);
    settle(h);
}

#[test]
fn a_parameter_typed_in_the_inspector_is_one_undo_step() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    h.get_by_label("Parameters");
    type_into(&mut h, "Mean", "1.25");
    assert_eq!(
        element(&h, "serve")["service"],
        json!({"type": "exponential", "mean": 1.25})
    );
    assert!(h.state().has_unsaved_changes());

    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Change Block");
    assert_eq!(element(&h, "serve")["service"]["mean"], json!(1.0));
}

#[test]
fn text_that_is_not_a_number_stays_in_its_field_until_fixed() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    type_into(&mut h, "Capacity", "two");
    // Not committed (craft-ui's NumberField keeps the text, outlined, with the reason).
    assert_eq!(element(&h, "serve")["capacity"], json!(1));
    assert!(!h.state().has_unsaved_changes());
    type_into(&mut h, "Capacity", "2");
    assert_eq!(element(&h, "serve")["capacity"], json!(2));
}

#[test]
fn numbers_are_read_as_the_users_region_writes_them() {
    let mut h = mm1();
    h.state_mut().set_number_region("de-DE");
    click(&mut h, Role::ListBoxOption, "Serve customer");
    // German: a decimal comma.
    type_into(&mut h, "Mean", "1,5");
    assert_eq!(element(&h, "serve")["service"]["mean"], json!(1.5));
}

#[test]
fn choosing_another_distribution_keeps_its_scale() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    click(&mut h, Role::ComboBox, "Service time");
    click(&mut h, Role::Button, "Triangular");
    let service = element(&h, "serve")["service"].clone();
    assert_eq!(service["type"], "triangular");
    assert_eq!(service["mode"], json!(1.0), "{service}");
    assert!(service["min"].as_f64() < service["mode"].as_f64());
    // The new kind's fields are on screen with the time unit.
    h.get_by_role_and_label(Role::TextInput, "Most likely");
}

#[test]
fn a_model_is_built_from_blocks_connected_and_run() {
    let mut h = harness();
    h.state_mut()
        .execute("model.new", json!({"name": "Clinic"}));
    settle(&mut h);
    for kind in ["Arrivals", "Task with a resource", "Departures"] {
        click(&mut h, Role::Button, "+ Add block");
        click(&mut h, Role::Button, kind);
    }
    let ids: Vec<String> = model(&h)["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["id"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(ids, ["source-1", "server-1", "sink-1"]);

    // Join them from the model overview's connections table.
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    for (from, to) in [("source-1", "server-1"), ("server-1", "sink-1")] {
        click(&mut h, Role::ComboBox, "From");
        click(&mut h, Role::Button, from);
        click(&mut h, Role::ComboBox, "To");
        click(&mut h, Role::Button, to);
        click(&mut h, Role::Button, "Connect");
    }
    assert_eq!(model(&h)["connections"].as_array().map(Vec::len), Some(2));
    h.get_by_label("None found. The model is ready to run.");

    h.state_mut().run();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(&mut h);
    assert!(h.state().results().is_some(), "{}", h.state().status());
}

#[test]
fn the_keyboard_moves_through_blocks_and_deletes_one() {
    let mut h = mm1();
    h.key_press(egui::Key::ArrowDown);
    settle(&mut h);
    h.key_press(egui::Key::ArrowDown);
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "Capacity"); // the server's inspector
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    assert!(
        model(&h)["elements"]
            .as_array()
            .is_some_and(|e| e.iter().all(|e| e["id"] != "serve"))
    );
    // Removing it left the other two unconnected: the overview says so, and the block is back
    // with one undo.
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    assert!(
        h.query_by_label("None found. The model is ready to run.")
            .is_none()
    );
    h.state_mut().undo();
    settle(&mut h);
    element(&h, "serve");
}

#[test]
fn the_experiment_and_flows_are_edited_from_their_forms() {
    let mut h = mm1();
    type_into(&mut h, "Replications", "32");
    assert_eq!(model(&h)["experiment"]["replications"], json!(32));
    click(&mut h, Role::ComboBox, "Time unit");
    click(&mut h, Role::Button, "hours");
    assert_eq!(model(&h)["experiment"]["time_unit"], json!("hour"));

    click(&mut h, Role::ListBoxOption, "Serve customer");
    // The inspector lists the flows; removing one is an edit like any other.
    // (The last Remove is a flow's, below the parameters: scrolled into view first.)
    click(&mut h, Role::Button, "Remove");
    assert_eq!(model(&h)["connections"].as_array().map(Vec::len), Some(1));
    click(&mut h, Role::ComboBox, "Add a flow to…");
    click(&mut h, Role::Button, "Customer leaves");
    assert_eq!(model(&h)["connections"].as_array().map(Vec::len), Some(2));
}

#[test]
fn a_property_is_added_typed_given_an_expression_and_removed() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    h.get_by_role_and_label(Role::TextInput, "New property name")
        .click();
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "New property name")
        .type_text("visits");
    settle(&mut h);
    click(&mut h, Role::Button, "Add property");
    assert_eq!(
        element(&h, "serve")["properties"],
        json!([{"name": "visits", "value": {"number": 0.0}}])
    );

    click(&mut h, Role::ComboBox, "Type");
    click(&mut h, Role::Button, "Whole number");
    assert_eq!(element(&h, "serve")["properties"][0]["type"], "long");

    click(&mut h, Role::ComboBox, "Value");
    click(&mut h, Role::Button, "Expression");
    type_into(&mut h, "Expression", "getProperty('visits') + 1");
    assert_eq!(
        element(&h, "serve")["properties"][0]["value"],
        json!({"expression": "getProperty('visits') + 1"})
    );
    assert!(
        h.query_by_label("Error").is_none(),
        "no error: {}",
        h.state().status()
    );

    // A malformed expression is the model's problem, shown on the block, not a failure.
    type_into(&mut h, "Expression", "getProperty('visits' +");
    h.get_by_label_contains("the expression ends too early (at character 23)");
    assert!(h.query_by_label("Error").is_some());

    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Set Property");
    let remove = h
        .get_all_by_role_and_label(Role::Button, "Remove")
        .next()
        .expect("the property's Remove")
        .rect();
    if off_screen(remove) {
        h.get_all_by_role_and_label(Role::Button, "Remove")
            .next()
            .expect("the property's Remove")
            .scroll_to_me();
        settle(&mut h);
    }
    h.get_all_by_role_and_label(Role::Button, "Remove")
        .next()
        .expect("the property's Remove")
        .click();
    settle(&mut h);
    assert!(element(&h, "serve")["properties"].is_null());
}

#[test]
fn a_decision_routes_by_probability_and_condition() {
    let mut h = harness();
    h.state_mut()
        .execute("model.new", json!({"name": "Triage"}));
    settle(&mut h);
    for kind in [
        "Arrivals",
        "Decision (one way)",
        "Task",
        "Task",
        "Departures",
    ] {
        click(&mut h, Role::Button, "+ Add block");
        click(&mut h, Role::Button, kind);
    }
    let gw = "exclusive_gateway-1";
    assert_eq!(element(&h, gw)["kind"], "exclusive_gateway");
    for (from, to) in [
        ("source-1", gw),
        (gw, "delay-1"),
        (gw, "delay-2"),
        ("delay-1", "sink-1"),
        ("delay-2", "sink-1"),
    ] {
        h.state_mut()
            .execute("connect", json!({ "from": from, "to": to }));
    }
    settle(&mut h);
    let flow_to = |h: &Harness<'static, App>, to: &str| {
        model(h)["connections"]
            .as_array()
            .and_then(|cs| {
                cs.iter()
                    .find(|c| c["from"] == gw && c["to"] == to)
                    .and_then(|c| c["id"].as_str().map(str::to_owned))
            })
            .expect("a flow out of the decision")
    };
    let first = flow_to(&h, "delay-1");

    // The decision's inspector: a probability on one way out, blank on the other.
    click(&mut h, Role::ListBoxOption, gw);
    type_into(&mut h, &format!("Probability of {first}"), "0.3");
    let conn = |h: &Harness<'static, App>, id: &str| {
        model(h)["connections"]
            .as_array()
            .and_then(|cs| cs.iter().find(|c| c["id"] == id).cloned())
            .unwrap_or_default()
    };
    assert_eq!(conn(&h, &first)["probability"], json!(0.3));
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Change Flow");
    h.state_mut().redo();
    settle(&mut h);

    // A default way out.
    click(&mut h, Role::ComboBox, "Default flow");
    click(&mut h, Role::Button, "to delay-2");
    assert_eq!(element(&h, gw)["default"], json!(flow_to(&h, "delay-2")));

    // A condition that can't be read is the decision's problem, shown on it.
    type_into(
        &mut h,
        &format!("Condition of {first}"),
        "getProperty('x' >",
    );
    assert_eq!(conn(&h, &first)["condition"], json!("getProperty('x' >"));
    assert!(h.query_by_label("Error").is_some());
    // Cleared, the condition goes.
    let field = format!("Condition of {first}");
    h.get_by_role_and_label(Role::TextInput, &field).click();
    settle(&mut h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert!(conn(&h, &first)["condition"].is_null());

    h.state_mut().run();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(&mut h);
    assert!(h.state().results().is_some(), "{}", h.state().status());
}

#[test]
fn a_waiting_room_is_set_on_a_task_and_its_losses_reported() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    // No limit at first: the queue may grow without bound.
    assert!(element(&h, "serve")["queue_capacity"].is_null());
    h.get_by_label("unlimited");

    // Ticking the box gives room for ten; then type the size wanted.
    click(&mut h, Role::CheckBox, "Waiting room");
    assert_eq!(element(&h, "serve")["queue_capacity"], json!(10));
    type_into(&mut h, "Waiting room", "2");
    assert_eq!(element(&h, "serve")["queue_capacity"], json!(2));

    // A run reports the arrivals turned away at the full room, in words and as a chart.
    h.state_mut().run();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(&mut h);
    let results = h.state().results().expect("results").clone();
    let lost = results["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|e| e["id"] == "serve"))
        .map(|e| e["metrics"]["loss_probability"]["mean"].clone())
        .unwrap_or_default();
    assert!(lost.as_f64().is_some_and(|p| p > 0.0), "{results}");
    click(&mut h, Role::Tab, "Results");
    h.get_by_label("Share of arrivals lost");

    // Unticking removes the limit again, as one undoable step.
    click(&mut h, Role::Tab, "Model");
    click(&mut h, Role::ListBoxOption, "Serve customer");
    click(&mut h, Role::CheckBox, "Waiting room");
    assert!(element(&h, "serve")["queue_capacity"].is_null());
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(element(&h, "serve")["queue_capacity"], json!(2));
}

#[test]
fn results_are_written_as_the_users_region_writes_numbers() {
    let mut h = mm1();
    h.state_mut().set_number_region("de-DE");
    h.state_mut().run();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(&mut h);
    // About 14,990 arrivals in 20,000 minutes, grouped with a point in German.
    assert!(
        !h.get_all_by_label("14.990").collect::<Vec<_>>().is_empty(),
        "no German grouping in the results"
    );
    assert!(h.query_by_label("14,990").is_none());
}

#[test]
fn pools_are_added_and_a_shared_task_needs_them_by_pool_or_role() {
    let mut h = mm1();
    // The overview: add a pool, give it 3 units and a role.
    click(&mut h, Role::Button, "Add pool");
    let pools = |h: &Harness<'static, App>| model(h)["resources"].clone();
    assert_eq!(pools(&h)[0]["id"], "resource-1", "{}", pools(&h));
    type_into(&mut h, "Units", "3");
    type_into(&mut h, "Roles", "agent, senior");
    assert_eq!(pools(&h)[0]["quantity"], json!(3));
    assert_eq!(pools(&h)[0]["roles"], json!(["agent", "senior"]));

    // A task with shared resources starts out needing one unit of the first pool.
    click(&mut h, Role::Button, "+ Add block");
    click(&mut h, Role::Button, "Task with shared resources");
    let task = element(&h, "shared_task-1");
    assert_eq!(
        task["needs"],
        json!([{"resource": "resource-1", "quantity": 1}])
    );

    // Needed by role instead, then a second alternative.
    click(&mut h, Role::ComboBox, "Alternative 1");
    click(&mut h, Role::Button, "By role");
    type_into(&mut h, "Roles", "agent");
    type_into(&mut h, "Units", "2");
    assert_eq!(
        element(&h, "shared_task-1")["needs"],
        json!([{"roles": ["agent"], "quantity": 2}])
    );
    click(&mut h, Role::Button, "Add alternative");
    assert_eq!(
        element(&h, "shared_task-1")["needs"][1],
        json!({"resource": "resource-1", "quantity": 1})
    );
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(
        element(&h, "shared_task-1")["needs"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn a_servers_transfer_breakdowns_and_availability_are_edited_in_its_inspector() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");

    // A transfer time: none at first, then a constant one, whose sender is then freed at once.
    click(&mut h, Role::ComboBox, "Transfer time");
    click(&mut h, Role::Button, "Constant");
    assert_eq!(
        element(&h, "serve")["transfer"],
        json!({"type": "constant", "value": 1.0})
    );
    click(&mut h, Role::CheckBox, "Sender waits during transfer");
    assert_eq!(element(&h, "serve")["transfer_holds_sender"], json!(false));
    click(&mut h, Role::ComboBox, "Transfer time");
    click(&mut h, Role::Button, "None");
    assert!(element(&h, "serve").get("transfer").is_none());
    assert!(
        h.query_by_label("Sender waits during transfer").is_none(),
        "shown only with a transfer"
    );

    // Breakdowns and availability start closed on a server with neither.
    assert!(h.query_by_label("Breakdowns").is_none());
    click(
        &mut h,
        Role::DisclosureTriangle,
        "Breakdowns and availability",
    );
    h.get_by_label("The model has no calendars yet.");
    click(&mut h, Role::CheckBox, "Breakdowns");
    assert_eq!(
        element(&h, "serve")["breakdown"],
        json!({
            "time_to_failure": {"type": "exponential", "mean": 1.0},
            "repair": {"type": "exponential", "mean": 1.0},
            "interruptible": false,
        })
    );
    // The repair time's mean: the last "Mean" (service, time to failure, repair).
    let repair_mean = |h: &Harness<'static, App>| {
        h.get_all_by_role_and_label(Role::TextInput, "Mean")
            .last()
            .expect("the repair's mean")
            .rect()
    };
    if off_screen(repair_mean(&h)) {
        h.get_all_by_role_and_label(Role::TextInput, "Mean")
            .last()
            .expect("the repair's mean")
            .scroll_to_me();
        settle(&mut h);
    }
    h.get_all_by_role_and_label(Role::TextInput, "Mean")
        .last()
        .expect("the repair's mean")
        .click();
    settle(&mut h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_all_by_role_and_label(Role::TextInput, "Mean")
        .last()
        .expect("the repair's mean")
        .type_text("5");
    settle(&mut h);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert_eq!(
        element(&h, "serve")["breakdown"]["repair"],
        json!({"type": "exponential", "mean": 5.0})
    );
    // Hands on finished parts while down is true when left out of the file.
    assert_eq!(
        h.get_by_role_and_label(Role::CheckBox, "Hands on finished parts while down")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    click(&mut h, Role::CheckBox, "Failure interrupts the job");
    assert_eq!(
        element(&h, "serve")["breakdown"]["interruptible"],
        json!(true)
    );

    // With a calendar in the model, the server is off shift while it is active.
    h.state_mut()
        .execute(
            "calendar.add",
            json!({
                "id": "lunch",
                "name": "Lunch",
                "events": [{"start": "2026-01-01T12:00:00", "end": "2026-01-01T13:00:00", "repeat": "FREQ=DAILY"}],
            }),
        )
        .expect("a calendar");
    settle(&mut h);
    click(&mut h, Role::Button, "Add calendar entry");
    assert_eq!(
        element(&h, "serve")["availability"],
        json!([{"calendar": "lunch", "available": false}])
    );
    h.get_by_role_and_label(Role::ComboBox, "Calendar 1");
    // The first active entry applies, so a calendar has one entry: none left to add.
    assert!(h.query_by_label("Add calendar entry").is_none());
    click(&mut h, Role::CheckBox, "Available outside calendars");
    assert_eq!(element(&h, "serve")["available"], json!(false));
    // Each edit is one undo step.
    h.state_mut().undo();
    settle(&mut h);
    assert!(element(&h, "serve").get("available").is_none());
    assert_eq!(
        element(&h, "serve")["availability"],
        json!([{"calendar": "lunch", "available": false}])
    );
}

#[test]
fn calendars_are_added_named_and_given_periods_on_the_model_page() {
    let mut h = mm1();
    click(&mut h, Role::Button, "Add calendar");
    let calendars = |h: &Harness<'static, App>| model(h)["calendars"].clone();
    assert_eq!(calendars(&h)[0]["id"], "calendar-1", "{}", calendars(&h));
    type_into(&mut h, "Calendar name", "Day shift");
    assert_eq!(calendars(&h)[0]["name"], json!("Day shift"));

    // A new period: 9 to 5 every day, on the day of the model's first period (none yet).
    click(&mut h, Role::Button, "Add period");
    assert_eq!(
        calendars(&h)[0]["events"],
        json!([{"start": "2026-01-01T09:00:00", "end": "2026-01-01T17:00:00", "repeat": "FREQ=DAILY"}])
    );
    type_into(&mut h, "Period 1 ends", "2026-01-01T13:00:00");
    assert_eq!(
        calendars(&h)[0]["events"][0]["end"],
        json!("2026-01-01T13:00:00")
    );
    // Text that is not a date is refused, and the period keeps its time.
    type_into(&mut h, "Period 1 starts", "tomorrow");
    assert_eq!(
        calendars(&h)[0]["events"][0]["start"],
        json!("2026-01-01T09:00:00")
    );

    // Weekly on Monday and Friday.
    click(&mut h, Role::ComboBox, "Period 1 repeats");
    click(&mut h, Role::Button, "Weekly");
    click(&mut h, Role::Button, "Fri");
    click(&mut h, Role::Button, "Mon");
    assert_eq!(
        calendars(&h)[0]["events"][0]["repeat"],
        json!("FREQ=WEEKLY;BYDAY=MO,FR")
    );

    // Model time 0 placed on a date.
    type_into(&mut h, "Model time 0 is", "2026-01-05T00:00:00");
    assert_eq!(
        model(&h)["experiment"]["start"],
        json!("2026-01-05T00:00:00")
    );

    // The period's Remove (the last one) removes it; then the calendar's is the last.
    click(&mut h, Role::Button, "Remove");
    assert_eq!(calendars(&h)[0]["events"], json!([]));

    // A calendar in use is not removed; once unused, it is.
    h.state_mut()
        .execute(
            "block.set",
            json!({"id": "serve", "availability": [{"calendar": "calendar-1", "available": false}]}),
        )
        .expect("availability set");
    settle(&mut h);
    click(&mut h, Role::Button, "Remove");
    assert_eq!(calendars(&h).as_array().map(Vec::len), Some(1));
    h.state_mut()
        .execute("block.set", json!({"id": "serve", "availability": []}))
        .expect("availability cleared");
    settle(&mut h);
    click(&mut h, Role::Button, "Remove");
    assert_eq!(calendars(&h), Value::Null);
}

#[test]
fn a_product_mix_is_set_on_the_arrivals() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Customer arrives");
    h.get_by_role_and_label(Role::TextInput, "New property name")
        .click();
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "New property name")
        .type_text("type");
    settle(&mut h);
    click(&mut h, Role::Button, "Add property");

    // A mix: two values, half each, then a third value and other shares.
    click(&mut h, Role::ComboBox, "Value");
    click(&mut h, Role::Button, "Mix");
    let value = |h: &Harness<'static, App>| element(h, "arrive")["properties"][0]["value"].clone();
    assert_eq!(
        value(&h),
        json!({"mix": [{"value": "A", "share": 1.0}, {"value": "B", "share": 1.0}]})
    );
    click(&mut h, Role::Button, "Add value");
    type_into(&mut h, "Value 3", "C");
    type_into(&mut h, "Share 1", "3");
    assert_eq!(
        value(&h),
        json!({"mix": [
            {"value": "A", "share": 3.0},
            {"value": "B", "share": 1.0},
            {"value": "C", "share": 1.0},
        ]})
    );
    // Typed as text, so a decision can read it with getProperty('type').
    assert!(
        h.query_by_label("Error").is_none(),
        "no error: {}",
        h.state().status()
    );
}

#[test]
fn a_setup_and_a_changeover_are_set_in_the_inspector() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    click(&mut h, Role::CheckBox, "Setup");
    assert_eq!(
        element(&h, "serve")["setup"],
        json!({"time": {"type": "exponential", "mean": 1.0}})
    );
    // A changeover: only when the type changes, 3 time units from A to B.
    type_into(&mut h, "Only when this property changes", "type");
    assert_eq!(element(&h, "serve")["setup"]["on_change_of"], json!("type"));
    click(&mut h, Role::Button, "Add row");
    type_into(&mut h, "From", "A");
    type_into(&mut h, "To", "B");
    assert_eq!(
        element(&h, "serve")["setup"]["by_change"],
        json!([{"from": "A", "to": "B", "time": {"type": "exponential", "mean": 1.0}}])
    );
    // No element sets `type` yet: the model says so.
    h.get_by_label_contains("reads property `type`, which no element sets");
    // Removing the setup is one step.
    click(&mut h, Role::CheckBox, "Setup");
    assert!(element(&h, "serve").get("setup").is_none());
}

/// A planned run time per item, for OEE, is set and removed in the inspector like a transfer
/// time.
#[test]
fn a_planned_run_time_is_set_in_the_inspector() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    click(&mut h, Role::ComboBox, "Planned run time per item");
    click(&mut h, Role::Button, "Constant");
    assert_eq!(
        element(&h, "serve")["planned_run_time"],
        json!({"type": "constant", "value": 1.0})
    );
    click(&mut h, Role::ComboBox, "Planned run time per item");
    click(&mut h, Role::Button, "None");
    assert!(element(&h, "serve").get("planned_run_time").is_none());
}
