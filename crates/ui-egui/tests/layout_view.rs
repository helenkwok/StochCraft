//! The Layout view: its tab shows the sorter example's plan to scale, with a scale bar, the
//! blocks' names and the note about where positions come from. What is drawn is checked by the
//! unit tests in `layout_view.rs`.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use stochcraft_ui_egui::{App, Platform};

const VIEW: &str = "Layout of the model, to scale";

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

fn sorter() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.state_mut().open_file(
        "sorter.bpmn",
        include_bytes!("../../../tests/models/sorter.bpmn"),
    );
    settle(&mut h);
    h
}

fn has(h: &Harness<'static, App>, label: &str) -> bool {
    !h.query_all_by_label(label).collect::<Vec<_>>().is_empty()
}

#[test]
fn the_layout_tab_shows_the_scale_bar_and_the_blocks() {
    let mut h = sorter();
    assert!(!has(&h, VIEW), "the view is on its own tab");
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    settle(&mut h);
    assert!(has(&h, VIEW));
    assert!(has(&h, "Positions from the diagram"));
    assert!(has(&h, "Unplaced"));
    assert!(
        !h.query_all_by_label_contains("Scale bar: ")
            .collect::<Vec<_>>()
            .is_empty()
    );
    // A block's name, written over its footprint.
    assert!(
        !h.query_all_by_label_contains("Infeed")
            .collect::<Vec<_>>()
            .is_empty()
    );
    h.get_by_role_and_label(Role::Button, "Fit").click();
    settle(&mut h);
    assert!(has(&h, VIEW));
    // Back to the model, and across to the 3D view, which keeps its own tab.
    h.get_by_role_and_label(Role::Tab, "3D").click();
    settle(&mut h);
    assert!(!has(&h, VIEW));
    assert!(has(&h, "3D view of the model"));
}
