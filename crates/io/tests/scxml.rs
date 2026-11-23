//! Agents' state charts as SCXML 1.0 files (ROADMAP M4.3, stage A4): every feature a chart has
//! written and read back exactly, what cannot be read or written refused by name, a chart read
//! back running as the original did, and every chart written valid against W3C's SCXML 1.0 XML
//! Schema (fetched by `cargo xtask fetch-scxml`, never committed) with StochCraft's extension
//! schema beside it.

#![allow(clippy::panic)] // A chart that fails to be written or read fails the test.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use stochcraft_agents::{
    Action, ChartSpec, History, Model, Population, SendSpec, StateSpec, Statistic, TransitionSpec,
};
use stochcraft_io::scxml::{read_scxml, write_scxml};

fn write(chart: &ChartSpec, seconds: f64) -> String {
    match write_scxml(chart, seconds) {
        Ok(t) => t,
        Err(e) => panic!("{e}"),
    }
}

fn read(text: &str, seconds: f64) -> ChartSpec {
    match read_scxml(text.as_bytes(), seconds) {
        Ok(c) => c,
        Err(e) => panic!("{e}\n{text}"),
    }
}

/// A chart with every element and attribute a chart can have.
fn everything() -> ChartSpec {
    ChartSpec::new("person")
        .initial(&["life"])
        .data("age", "uniform(20, 60)")
        .data("contacts", "0")
        .data("note", "'it''s \"quoted\" & <odd>\n\tand on two lines'")
        .state(
            StateSpec::state("life")
                .initial(&["healthy"])
                .on_entry(Action::log("born"))
                .on_exit(Action::log_value("died at", "time()"))
                .transition(
                    TransitionSpec::on("cull.* stop")
                        .when("$age > 80 and In('sick')")
                        .to("gone"),
                )
                .child(
                    StateSpec::state("healthy")
                        .on_entry(SendSpec::new("tick").id("tick_send").after(0.1))
                        .on_entry(SendSpec::new("third").id("third_send").after(1.0 / 3.0))
                        .on_entry(
                            SendSpec::new("catch")
                                .id("catch_send")
                                .at_rate("0.5 * $contacts + 0.01"),
                        )
                        .on_exit(Action::cancel("tick_send"))
                        .on_exit(Action::cancel("catch_send"))
                        .on_exit(Action::cancel("never sent"))
                        .transition(TransitionSpec::on("catch").to("sick"))
                        .transition(
                            TransitionSpec::on("tick")
                                .internal()
                                .then(Action::assign("contacts", "$contacts + 1"))
                                .then(SendSpec::new("hello").to_random("people").with_data("$age"))
                                .then(SendSpec::new("all").to_all("people"))
                                .then(SendSpec::new("one").to_agent("people", "serial() + 1"))
                                .then(SendSpec::new("near").to_linked("people"))
                                .then(SendSpec::new("near1").to_linked_random("people"))
                                .then(SendSpec::new("cell").to_neighbours("people"))
                                .then(SendSpec::new("cell1").to_neighbour_random("people"))
                                .then(SendSpec::new("inner").internal())
                                .then(SendSpec::new("later").after_expr("exponential(2)"))
                                .then(SendSpec::new("now"))
                                .then(Action::raise("raised.one")),
                        ),
                )
                .child(
                    StateSpec::state("sick")
                        .on_entry(Action::create("people"))
                        .on_entry(Action::remove("people", "serial() - 1"))
                        .on_entry(Action::connect("people", "0"))
                        .on_entry(Action::disconnect("people", "1"))
                        .on_entry(Action::move_to("people", "1", "2"))
                        .on_entry(Action::move_random("people"))
                        .transition(
                            TransitionSpec::eventless()
                                .when("$contacts > 3")
                                .to("treated"),
                        )
                        .child(StateSpec::state("mild"))
                        .child(StateSpec::state("severe")),
                )
                .child(
                    StateSpec::parallel("treated")
                        .child(
                            StateSpec::state("drug")
                                .child(StateSpec::history(
                                    "drug_history",
                                    History::Deep,
                                    TransitionSpec::eventless()
                                        .to("dose")
                                        .then(Action::log("no history yet")),
                                ))
                                .child(StateSpec::state("dose").child(StateSpec::state("dose_a"))),
                        )
                        .child(
                            StateSpec::state("rest")
                                .child(StateSpec::history(
                                    "rest_history",
                                    History::Shallow,
                                    TransitionSpec::eventless().to("bed"),
                                ))
                                .child(StateSpec::state("bed")),
                        ),
                ),
        )
        .state(StateSpec::final_state("gone").on_entry(Action::log("gone")))
}

#[test]
fn every_feature_reads_back_exactly() {
    for seconds in [1.0, 60.0, 3600.0, 86_400.0, 0.001] {
        let chart = everything();
        let text = write(&chart, seconds);
        assert_eq!(read(&text, seconds), chart, "{seconds} s per unit:\n{text}");
        // Written again from what was read, the same text.
        assert_eq!(write(&read(&text, seconds), seconds), text);
    }
}

#[test]
fn a_fixed_delay_is_seconds_and_exact_where_seconds_would_not_be() {
    let chart = |d: f64| {
        ChartSpec::new("t").state(StateSpec::state("a").on_entry(SendSpec::new("e").after(d)))
    };
    // Two minutes: 120 seconds, and nothing else needed.
    let text = write(&chart(2.0), 60.0);
    assert!(text.contains("delay=\"120s\""), "{text}");
    assert!(!text.contains("stochcraft:delay"), "{text}");
    // A third of a minute comes back from its 20 seconds exactly: nothing else needed.
    let text = write(&chart(1.0 / 3.0), 60.0);
    assert!(text.contains("delay=\"20s\""), "{text}");
    assert!(!text.contains("stochcraft:delay"), "{text}");
    // A delay whose seconds do not convert back to the same number keeps it exactly too.
    let inexact = (1..10_000)
        .map(|k| f64::from(k) / 97.0)
        .find(|d| (d * 60.0) / 60.0 != *d);
    let Some(d) = inexact else {
        panic!("every k / 97 minutes converts back from its seconds")
    };
    let text = write(&chart(d), 60.0);
    assert!(
        text.contains(&format!("stochcraft:delay=\"{d}\"")),
        "{text}"
    );
    assert_eq!(read(&text, 60.0), chart(d));
    // A delay below zero is no CSS2 time: only StochCraft's attribute.
    let text = write(&chart(-1.0), 60.0);
    assert!(!text.contains(" delay="), "{text}");
    for d in [0.0, 0.1, 1.0 / 3.0, 1e-9, 12_345.678, 1e15, -1.0, -0.0] {
        for seconds in [1.0, 60.0, 3600.0, 0.001] {
            let back = read(&write(&chart(d), seconds), seconds);
            assert_eq!(back, chart(d), "{d} at {seconds} s per unit");
        }
    }
    // Other tools' units are read.
    let other = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="stochcraft">
  <state id="a"><onentry><send event="e" delay="1.5m"/><send event="f" delay="250ms"/></onentry></state>
</scxml>"#;
    let got = read(other, 60.0);
    let want = ChartSpec::new("").state(
        StateSpec::state("a")
            .on_entry(SendSpec::new("e").after(1.5))
            .on_entry(SendSpec::new("f").after(0.25 / 60.0)),
    );
    assert_eq!(got, want);
}

#[test]
fn a_rate_is_also_a_delay_other_tools_can_read() {
    let chart = ChartSpec::new("r")
        .state(StateSpec::state("a").on_entry(SendSpec::new("e").at_rate("2 * $k")))
        .data("k", "1");
    let text = write(&chart, 1.0);
    assert!(
        text.contains("delayexpr=\"exponential(1 div (2 * $k))\""),
        "{text}"
    );
    assert!(text.contains("stochcraft:rate=\"2 * $k\""), "{text}");
}

#[test]
fn messages_to_other_agents_name_stochcrafts_event_processor() {
    let chart = ChartSpec::new("m").state(
        StateSpec::state("a").on_entry(SendSpec::new("hi").to_random("people").with_data("1")),
    );
    let text = write(&chart, 1.0);
    assert!(
        text.contains("type=\"https://helenkwok.github.io/StochCraft/scxml/1#agents\""),
        "{text}"
    );
    assert!(text.contains("<content expr=\"1\"/>"), "{text}");
}

#[test]
fn another_prefix_and_initial_elements_read_the_same() {
    let text = r#"<?xml version="1.0"?>
<s:scxml xmlns:s="http://www.w3.org/2005/07/scxml" xmlns:x="https://helenkwok.github.io/StochCraft/scxml/1" version="1.0" datamodel="stochcraft" name="p">
  <s:state id="a">
    <s:initial><s:transition target="b"/></s:initial>
    <s:onentry><s:send event="e" x:rate="3"/><x:create population="people"/></s:onentry>
    <s:onentry><s:cancel sendidexpr="'e1'"/></s:onentry>
    <s:state id="b"/>
  </s:state>
</s:scxml>"#;
    let want = ChartSpec::new("p").state(
        StateSpec::state("a")
            .initial(&["b"])
            .on_entry(SendSpec::new("e").at_rate("3"))
            .on_entry(Action::create("people"))
            .on_entry(Action::cancel("e1"))
            .child(StateSpec::state("b")),
    );
    assert_eq!(read(text, 1.0), want);
}

#[test]
fn what_a_chart_cannot_hold_is_refused_by_name() {
    let wrap = |inner: &str| {
        format!(
            r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="stochcraft"><state id="a">{inner}</state></scxml>"#
        )
    };
    let cases = [
        (wrap("<onentry><if cond=\"true\"><log label=\"x\"/></if></onentry>"), "<if>"),
        (wrap("<onentry><foreach array=\"$a\" item=\"i\"/></onentry>"), "<foreach>"),
        (wrap("<onentry><script>x = 1</script></onentry>"), "<script>"),
        (wrap("<invoke src=\"x.scxml\"/>"), "<invoke>"),
        (wrap("<onentry><send event=\"e\"><param name=\"a\" expr=\"1\"/></send></onentry>"), "<param>"),
        (wrap("<datamodel><data id=\"x\" expr=\"1\"/></datamodel>"), "<datamodel>"),
        (wrap("<onentry><send eventexpr=\"'e'\"/></onentry>"), "eventexpr"),
        (wrap("<onentry><send event=\"e\" target=\"#_parent\"/></onentry>"), "#_parent"),
        (wrap("<onentry><send event=\"e\" idlocation=\"x\"/></onentry>"), "idlocation"),
        (wrap("<onentry><send event=\"e\" namelist=\"x\"/></onentry>"), "namelist"),
        (wrap("<onentry><cancel sendidexpr=\"$id\"/></onentry>"), "computed send id"),
        (wrap("<onentry><other xmlns=\"urn:x\"/></onentry>"), "<other>"),
        (
            r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript"/>"#.to_owned(),
            "ecmascript",
        ),
        (
            r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" binding="late"/>"#.to_owned(),
            "late",
        ),
        (r#"<scxml xmlns="http://www.w3.org/2005/07/scxml"/>"#.to_owned(), "version"),
        (r#"<scxml version="1.0"/>"#.to_owned(), "SCXML's"),
    ];
    for (text, word) in cases {
        match read_scxml(text.as_bytes(), 1.0) {
            Ok(c) => panic!("read {text} as {c:?}"),
            Err(e) => assert!(e.to_string().contains(word), "{word}: {e}"),
        }
    }
}

#[test]
fn what_scxml_cannot_say_is_not_written() {
    let refused = |chart: ChartSpec, word: &str| match write_scxml(&chart, 1.0) {
        Ok(t) => panic!("wrote {t}"),
        Err(e) => assert!(e.to_string().contains(word), "{word}: {e}"),
    };
    // A variable and a state with one id: SCXML gives them one set of ids.
    refused(
        ChartSpec::new("c")
            .data("sick", "0")
            .state(StateSpec::state("sick")),
        "also variable `sick`",
    );
    // Two sends with one id.
    refused(
        ChartSpec::new("c")
            .state(StateSpec::state("a").on_entry(SendSpec::new("e").id("t")))
            .state(StateSpec::state("b").on_entry(SendSpec::new("e").id("t"))),
        "the id `t`",
    );
    refused(
        ChartSpec::new("c").state(StateSpec::state("1st")),
        "not an XML name",
    );
    refused(
        ChartSpec::new("two words").state(StateSpec::state("a")),
        "name token",
    );
    refused(
        ChartSpec::new("c").state(StateSpec::state("a").on_entry(Action::raise("a b"))),
        "event name",
    );
    refused(
        ChartSpec::new("c").state(StateSpec::state("a").transition(TransitionSpec::on("a..b"))),
        "event descriptor",
    );
}

/// A Bass diffusion chart, as A2 checks it.
fn bass() -> ChartSpec {
    ChartSpec::new("person")
        .data("adopted_at", "-1")
        .state(
            StateSpec::state("potential")
                .on_entry(SendSpec::new("advert").id("ad").at_rate("0.011"))
                .on_exit(Action::cancel("ad"))
                .transition(TransitionSpec::on("advert wom").to("adopted")),
        )
        .state(
            StateSpec::state("adopted")
                .on_entry(Action::assign("adopted_at", "time()"))
                .on_entry(SendSpec::new("talk").at_rate("1.5"))
                .transition(
                    TransitionSpec::on("talk")
                        .then(SendSpec::new("wom").to_random("people"))
                        .then(SendSpec::new("talk").at_rate("1.5")),
                ),
        )
}

fn bass_model(chart: &ChartSpec) -> Model {
    let chart = match chart.build() {
        Ok(c) => Arc::new(c),
        Err(e) => panic!("{e}"),
    };
    Model::new()
        .population(Population::new("people", chart).count(200))
        .statistic(Statistic::count("adopters", "people").when("In('adopted')"))
        .save_every(0.5)
}

#[test]
fn a_chart_read_back_runs_as_the_original_did() {
    let original = bass();
    let back = read(&write(&original, 86_400.0), 86_400.0);
    for seed in [1, 2, 3] {
        let a = bass_model(&original).run(seed, 10.0);
        let b = bass_model(&back).run(seed, 10.0);
        assert_eq!(a.ok(), b.ok(), "seed {seed}");
    }
}

// ---------------------------------------------------------------------------------------------
// W3C's XML Schema

fn required() -> bool {
    std::env::var("STOCHCRAFT_REQUIRE_BPSIM").is_ok_and(|v| v == "1")
}

fn missing(what: &str) {
    assert!(
        !required(),
        "{what} (STOCHCRAFT_REQUIRE_BPSIM=1); run `cargo xtask fetch-scxml`"
    );
    eprintln!("skipped: {what}; run `cargo xtask fetch-scxml` to include this test");
}

fn schema_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scxml")
}

/// A `file:` URI for an absolute path, as `schemaLocation` needs.
fn file_uri(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from(if path.starts_with('/') {
        "file://"
    } else {
        "file:///"
    });
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            uri.push(char::from(b));
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// What xmllint says is wrong with `text` against W3C's SCXML schema (with its modules and the
/// XML namespace's schema read from `dir`, never the network) and StochCraft's extension schema.
fn schema_errors(text: &str, label: &str, dir: &Path) -> Option<String> {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let stem = format!(
        "stochcraft-scxml-{}-{}-{label}",
        std::process::id(),
        CALLS.fetch_add(1, Ordering::Relaxed)
    );
    let tmp = std::env::temp_dir();
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let driver = std::fs::read_to_string(dir.join("scxml.xsd"))
        .unwrap_or_else(|e| panic!("scxml.xsd: {e}"))
        .replace(
            "schemaLocation=\"http://www.w3.org/2001/xml.xsd\"",
            &format!("schemaLocation=\"{}\"", file_uri(&dir.join("xml.xsd"))),
        )
        .replace(
            "schemaLocation=\"scxml-",
            &format!("schemaLocation=\"{}/scxml-", file_uri(&dir)),
        );
    let driver_path = tmp.join(format!("{stem}-scxml.xsd"));
    std::fs::write(&driver_path, driver).unwrap_or_else(|e| panic!("{e}"));
    let ours = Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/stochcraft-scxml-1.xsd");
    let wrapper = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:import namespace="http://www.w3.org/2005/07/scxml" schemaLocation="{}"/>
  <xs:import namespace="https://helenkwok.github.io/StochCraft/scxml/1" schemaLocation="{}"/>
</xs:schema>
"#,
        file_uri(&driver_path),
        file_uri(&ours)
    );
    let wrapper_path = tmp.join(format!("{stem}-schemas.xsd"));
    std::fs::write(&wrapper_path, wrapper).unwrap_or_else(|e| panic!("{e}"));
    let doc = tmp.join(format!("{stem}.scxml"));
    std::fs::write(&doc, text).unwrap_or_else(|e| panic!("{e}"));
    let result = Command::new("xmllint")
        .arg("--noout")
        .arg("--nonet")
        .arg("--schema")
        .arg(&wrapper_path)
        .arg(&doc)
        .output();
    for p in [&driver_path, &wrapper_path, &doc] {
        let _ = std::fs::remove_file(p);
    }
    let result = result.unwrap_or_else(|e| panic!("xmllint: {e}"));
    (!result.status.success()).then(|| {
        format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )
    })
}

#[test]
fn every_chart_written_is_valid_scxml() {
    let dir = schema_dir();
    if !dir.join("scxml.xsd").exists() || !dir.join("xml.xsd").exists() {
        return missing("W3C's SCXML 1.0 XML Schema is not in target/scxml");
    }
    if Command::new("xmllint").arg("--version").output().is_err() {
        return missing("xmllint is not installed (libxml2-utils)");
    }
    for (label, chart, seconds) in [
        ("everything", everything(), 60.0),
        ("everything_seconds", everything(), 1.0),
        ("bass", bass(), 86_400.0),
    ] {
        let text = write(&chart, seconds);
        if let Some(errors) = schema_errors(&text, label, &dir) {
            panic!("{label} is not valid SCXML 1.0:\n{errors}\n{text}");
        }
    }
    // The check itself: a chart the schema refuses is caught.
    let bad = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"><state id="a"><transition type="sideways"/></state></scxml>"#;
    assert!(schema_errors(bad, "bad", &dir).is_some());
}
