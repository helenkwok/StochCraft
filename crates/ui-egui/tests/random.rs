//! Experiment settings ▸ Random numbers: the modes, a seed drawn from a source (a scripted fetch
//! answering with a recorded beacon pulse, never the network) and shown, the estimate, and the
//! source manager keeping a key without showing it.

#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)] // A missing widget fails the test.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use stochcraft_qrng::MemoryKeys;
use stochcraft_qrng::testing::ScriptedFetch;
use stochcraft_ui_egui::{App, Platform};

const PULSE: &[u8] = include_bytes!("../../qrng/tests/fixtures/nist-beacon-pulse.json");

fn harness() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    app.set_random_net(
        Box::new(ScriptedFetch::always(PULSE.to_vec())),
        Box::new(MemoryKeys::default()),
    );
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 2400.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run_steps(2);
    for label in ["File", "Open Example", "M/M/1 queue"] {
        let item = h
            .query_all_by_label(label)
            .last()
            .map(|n| n.click())
            .is_some();
        assert!(item, "{label}");
        h.run_steps(2);
    }
    h
}

fn click(h: &mut Harness<'static, App>, label: &str) {
    h.query_all_by_label(label)
        .last()
        .unwrap_or_else(|| panic!("no {label}"))
        .click();
    h.run_steps(3);
}

#[test]
fn a_seed_is_drawn_from_the_beacon_and_shown() {
    let mut h = harness();
    click(&mut h, "Seed from a source");
    let before = h.get_by_label("Random numbers").rect();
    click(&mut h, "Draw a seed");
    for _ in 0..10 {
        h.run_steps(2);
    }
    // The source by its name: its address, one long word, would widen the panel.
    h.get_by_label_contains("taken from NIST Randomness Beacon");
    assert!(h.query_by_label_contains("beacon.nist.gov").is_none());
    let after = h.get_by_label("Random numbers").rect();
    assert_eq!(before, after, "the panel kept its width and place");
    assert!(
        h.state()
            .engine()
            .journal()
            .iter()
            .all(|e| !e.command.starts_with("random."))
    );
}

#[test]
fn file_and_live_modes_show_their_fields_and_estimate() {
    let mut h = harness();
    click(&mut h, "File of random bytes");
    h.get_by_label("Bytes per run");
    h.get_by_label("From byte");
    click(&mut h, "Live from a source");
    h.get_by_label_contains("ANU's free tier (100 requests a month) covers seeds");
}

#[test]
fn the_source_manager_keeps_a_key_without_showing_it() {
    let mut h = harness();
    click(&mut h, "Sources and keys…");
    h.get_by_label("Add a source");
    assert!(h.query_by_label("Key kept").is_none());
    h.state_mut().set_random_key("anu", "SECRET-KEY-1").unwrap();
    h.run_steps(3);
    h.get_by_label("Key kept");
    assert!(h.query_by_label_contains("SECRET-KEY").is_none());
}
