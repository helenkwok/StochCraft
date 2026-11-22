//! The order of actions, by hand: each chart's expected steps are worked out from SCXML's
//! Appendix D (exit set innermost first and in reverse document order, the transitions'
//! content, the entry set outermost first and in document order).
#![allow(clippy::panic)] // A chart that fails to build or run fails the test.

use std::sync::Arc;

use stochcraft_agents::{
    Action, Agent, ChartSpec, History, SendSpec, StateSpec, Step, TransitionSpec,
};

fn start(spec: &ChartSpec) -> Agent {
    let chart = match spec.build() {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    };
    let mut a = Agent::new(Arc::new(chart), 1, "agent").with_trace();
    if let Err(e) = a.start(0.0) {
        panic!("{e}");
    }
    a
}

fn steps(a: &Agent) -> Vec<String> {
    a.trace().iter().map(|r| r.step.to_string()).collect()
}

/// The steps of handling one external event.
fn handle(a: &mut Agent, event: &str) -> Vec<String> {
    let from = a.trace().len();
    if let Err(e) = a.handle(event, 0.0) {
        panic!("{e}");
    }
    a.trace()[from..]
        .iter()
        .map(|r| r.step.to_string())
        .collect()
}

#[test]
fn a_transition_between_cousins_exits_and_enters_up_to_their_common_ancestor() {
    let spec = ChartSpec::new("cousins").state(
        StateSpec::state("P")
            .child(
                StateSpec::state("A").on_exit(Action::log("leave A")).child(
                    StateSpec::state("A1").child(
                        StateSpec::state("A11")
                            .on_exit(Action::log("leave A11"))
                            .transition(TransitionSpec::on("go").to("B11").then(Action::log("t"))),
                    ),
                ),
            )
            .child(
                StateSpec::state("B")
                    .on_entry(Action::log("reach B"))
                    .child(
                        StateSpec::state("B1")
                            .child(StateSpec::state("B11").on_entry(Action::log("reach B11"))),
                    ),
            ),
    );
    let mut a = start(&spec);
    assert_eq!(steps(&a), ["enter P", "enter A", "enter A1", "enter A11"]);
    assert_eq!(
        handle(&mut a, "go"),
        [
            "event go",
            "exit A11",
            "log leave A11",
            "exit A1",
            "exit A",
            "log leave A",
            "log t",
            "enter B",
            "log reach B",
            "enter B1",
            "enter B11",
            "log reach B11",
        ]
    );
    assert_eq!(a.active_states(), ["P", "B", "B1", "B11"]);
}

fn branch(x: &str) -> Agent {
    let spec = ChartSpec::new("branch")
        .data("x", x)
        .state(
            StateSpec::state("check")
                .transition(TransitionSpec::eventless().when("$x > 5").to("big"))
                .transition(TransitionSpec::eventless().when("$x > 2").to("mid"))
                .transition(TransitionSpec::eventless().to("small")),
        )
        .state(StateSpec::state("big"))
        .state(StateSpec::state("mid"))
        .state(StateSpec::state("small"));
    start(&spec)
}

#[test]
fn a_branch_takes_its_first_true_condition_or_its_default() {
    for (x, to) in [("9", "big"), ("3", "mid"), ("1", "small"), ("5", "mid")] {
        let a = branch(x);
        assert_eq!(
            steps(&a),
            vec![
                "enter check".to_string(),
                "exit check".to_string(),
                format!("enter {to}")
            ],
            "x = {x}"
        );
        assert_eq!(a.active_states(), [to]);
    }
}

fn history_chart(depth: History) -> ChartSpec {
    ChartSpec::new("history")
        .initial(&["H"])
        .state(
            StateSpec::state("P")
                .child(StateSpec::history(
                    "H",
                    depth,
                    TransitionSpec::eventless()
                        .to("R")
                        .then(Action::log("no history yet")),
                ))
                .child(
                    StateSpec::state("Q")
                        .child(
                            StateSpec::state("Q1").transition(TransitionSpec::on("next").to("Q2")),
                        )
                        .child(StateSpec::state("Q2")),
                )
                .child(StateSpec::state("R").transition(TransitionSpec::on("toQ").to("Q")))
                .transition(TransitionSpec::on("out").to("Out")),
        )
        .state(StateSpec::state("Out").transition(TransitionSpec::on("back").to("H")))
}

#[test]
fn shallow_and_deep_history() {
    for depth in [History::Shallow, History::Deep] {
        let mut a = start(&history_chart(depth));
        // No history yet: the default transition, its content run after the parent's entry.
        assert_eq!(steps(&a), ["enter P", "log no history yet", "enter R"]);
        assert_eq!(
            handle(&mut a, "toQ"),
            ["event toQ", "exit R", "enter Q", "enter Q1"]
        );
        assert_eq!(
            handle(&mut a, "next"),
            ["event next", "exit Q1", "enter Q2"]
        );
        assert_eq!(
            handle(&mut a, "out"),
            ["event out", "exit Q2", "exit Q", "exit P", "enter Out"]
        );
        let back = handle(&mut a, "back");
        match depth {
            // P's last active child was Q, which is entered by its default, Q1.
            History::Shallow => assert_eq!(
                back,
                ["event back", "exit Out", "enter P", "enter Q", "enter Q1"]
            ),
            // P's last active atomic descendant was Q2.
            History::Deep => assert_eq!(
                back,
                ["event back", "exit Out", "enter P", "enter Q", "enter Q2"]
            ),
        }
    }
}

#[test]
fn both_regions_of_a_parallel_state_react_to_one_event() {
    let region = |r: &str, a: &str, b: &str, log: &str| {
        StateSpec::state(r)
            .child(
                StateSpec::state(a)
                    .transition(TransitionSpec::on("e").to(b).then(Action::log(log))),
            )
            .child(StateSpec::final_state(b))
    };
    let spec = ChartSpec::new("parallel")
        .state(
            StateSpec::parallel("P")
                .child(region("R1", "a1", "b1", "t1"))
                .child(region("R2", "a2", "b2", "t2"))
                .transition(TransitionSpec::on("done.state.P").to("End")),
        )
        .state(StateSpec::state("End"));
    let mut a = start(&spec);
    assert_eq!(
        steps(&a),
        ["enter P", "enter R1", "enter a1", "enter R2", "enter a2"]
    );
    assert_eq!(
        handle(&mut a, "e"),
        [
            "event e",
            // One microstep: both sources exited (reverse document order), both contents in
            // the order their states selected them, both targets entered.
            "exit a2",
            "exit a1",
            "log t1",
            "log t2",
            "enter b1",
            "enter b2",
            // Each region's final state raises its done event; the second makes P done.
            "event done.state.R1",
            "event done.state.R2",
            "event done.state.P",
            "exit b2",
            "exit R2",
            "exit b1",
            "exit R1",
            "exit P",
            "enter End",
        ]
    );
}

/// A transition from outside to a state deep in one region of a parallel state enters the
/// parallel state, that region down to the target, and every other region at its initial state,
/// in document order.
#[test]
fn entering_one_region_of_a_parallel_state_enters_the_others_at_their_initial_states() {
    let spec = ChartSpec::new("into-parallel")
        .state(StateSpec::state("A").transition(TransitionSpec::on("go").to("a2")))
        .state(
            StateSpec::parallel("P")
                .child(
                    StateSpec::state("R1")
                        .child(StateSpec::state("a1"))
                        .child(StateSpec::state("a2")),
                )
                .child(
                    StateSpec::state("R2")
                        .child(StateSpec::state("b1"))
                        .child(StateSpec::state("b2")),
                ),
        );
    let mut a = start(&spec);
    assert_eq!(steps(&a), ["enter A"]);
    assert_eq!(
        handle(&mut a, "go"),
        [
            "event go", "exit A", "enter P", "enter R1", "enter a2", "enter R2", "enter b1",
        ]
    );
    assert_eq!(a.active_states(), ["P", "R1", "a2", "R2", "b1"]);
}

#[test]
fn an_internal_transition_does_not_exit_its_source() {
    let spec = ChartSpec::new("internal").state(
        StateSpec::state("S")
            .on_entry(Action::log("S in"))
            .on_exit(Action::log("S out"))
            .child(StateSpec::state("S1"))
            .child(StateSpec::state("S2"))
            .transition(TransitionSpec::on("e").to("S2").internal())
            .transition(TransitionSpec::on("f").to("S2"))
            .transition(TransitionSpec::on("t").then(Action::log("no target"))),
    );
    let mut a = start(&spec);
    assert_eq!(steps(&a), ["enter S", "log S in", "enter S1"]);
    assert_eq!(handle(&mut a, "e"), ["event e", "exit S1", "enter S2"]);
    // The same transition, external: S is exited and entered again.
    assert_eq!(
        handle(&mut a, "f"),
        [
            "event f",
            "exit S2",
            "exit S",
            "log S out",
            "enter S",
            "log S in",
            "enter S2"
        ]
    );
    // Targetless: only its content.
    assert_eq!(handle(&mut a, "t"), ["event t", "log no target"]);
}

#[test]
fn conflicting_transitions_go_by_document_order() {
    let spec = ChartSpec::new("conflict")
        .state(
            StateSpec::parallel("P")
                .child(
                    StateSpec::state("R1")
                        .child(StateSpec::state("a").transition(TransitionSpec::on("e").to("X"))),
                )
                .child(
                    StateSpec::state("R2")
                        .child(StateSpec::state("b").transition(TransitionSpec::on("e").to("Y"))),
                ),
        )
        .state(StateSpec::state("X"))
        .state(StateSpec::state("Y"));
    let mut a = start(&spec);
    // Both leave P, so their exit sets meet; neither source is inside the other, so a's,
    // selected first, wins.
    assert_eq!(
        handle(&mut a, "e"),
        [
            "event e", "exit b", "exit R2", "exit a", "exit R1", "exit P", "enter X"
        ]
    );
}

#[test]
fn conflicting_transitions_prefer_the_descendant() {
    let spec =
        ChartSpec::new("descendant")
            .state(
                StateSpec::parallel("P")
                    .child(StateSpec::state("R1").child(StateSpec::state("c")))
                    .child(StateSpec::state("R2").child(
                        StateSpec::state("d").transition(
                            TransitionSpec::on("e").to("X").then(Action::log("from d")),
                        ),
                    ))
                    .transition(TransitionSpec::on("e").to("Y").then(Action::log("from P"))),
            )
            .state(StateSpec::state("X"))
            .state(StateSpec::state("Y"));
    let mut a = start(&spec);
    // c selects P's transition first (its own state and region have none); d then selects its
    // own, and wins because d is a descendant of P.
    assert_eq!(
        handle(&mut a, "e"),
        [
            "event e",
            "exit d",
            "exit R2",
            "exit c",
            "exit R1",
            "exit P",
            "log from d",
            "enter X"
        ]
    );
}

#[test]
fn a_state_s_own_transition_comes_before_its_ancestor_s() {
    let spec = ChartSpec::new("nearest")
        .state(
            StateSpec::state("S")
                .child(StateSpec::state("s1").transition(TransitionSpec::on("e").to("A")))
                .transition(TransitionSpec::on("e").to("B")),
        )
        .state(StateSpec::state("A"))
        .state(StateSpec::state("B"));
    let mut a = start(&spec);
    assert_eq!(
        handle(&mut a, "e"),
        ["event e", "exit s1", "exit S", "enter A"]
    );
}

#[test]
fn events_match_by_whole_tokens() {
    let spec = ChartSpec::new("tokens")
        .state(StateSpec::state("w").transition(TransitionSpec::on("sell buy").to("b")))
        .state(StateSpec::state("b").transition(TransitionSpec::on("*").to("w")));
    let mut a = start(&spec);
    assert_eq!(handle(&mut a, "buyer"), ["event buyer"]);
    assert_eq!(
        handle(&mut a, "buy.now"),
        ["event buy.now", "exit w", "enter b"]
    );
    assert_eq!(handle(&mut a, "x.y"), ["event x.y", "exit b", "enter w"]);
    assert_eq!(handle(&mut a, "sell"), ["event sell", "exit w", "enter b"]);
}

#[test]
fn conditions_read_in_and_the_event_and_raise_runs_first() {
    let spec = ChartSpec::new("in").data("seen", "''").state(
        StateSpec::parallel("P")
            .child(
                StateSpec::state("R1")
                    .child(
                        StateSpec::state("off").transition(
                            TransitionSpec::on("flip")
                                .to("on")
                                .then(Action::raise("flipped")),
                        ),
                    )
                    .child(StateSpec::state("on")),
            )
            .child(
                StateSpec::state("R2")
                    .child(
                        StateSpec::state("watch")
                            .transition(TransitionSpec::eventless().when("In('on')").to("seen")),
                    )
                    .child(
                        StateSpec::state("seen").transition(
                            TransitionSpec::on("flipped")
                                .then(Action::assign("seen", "getProperty('_event.name')")),
                        ),
                    ),
            ),
    );
    let mut a = start(&spec);
    // The eventless transition is taken before the raised event is handled.
    assert_eq!(
        handle(&mut a, "flip"),
        [
            "event flip",
            "exit off",
            "enter on",
            "exit watch",
            "enter seen",
            "event flipped"
        ]
    );
    assert_eq!(
        a.value("seen").map(|v| v.text()),
        Some("flipped".to_string())
    );
}

#[test]
fn a_failed_action_raises_error_execution() {
    let spec = ChartSpec::new("error")
        .state(
            StateSpec::state("s")
                .on_entry(SendSpec::new("later").after_expr("-1"))
                .on_entry(Action::log("skipped"))
                .transition(TransitionSpec::on("error").to("failed")),
        )
        .state(StateSpec::state("failed"));
    let a = start(&spec);
    let s = steps(&a);
    assert_eq!(s.first().map(String::as_str), Some("enter s"));
    assert!(
        matches!(a.trace().get(1).map(|r| &r.step), Some(Step::Error(_))),
        "{s:?}"
    );
    // The rest of the block is skipped, and the error is handled as an internal event.
    assert_eq!(s[2..], ["event error.execution", "exit s", "enter failed"]);
    assert_eq!(a.pending_sends(), 0);
}

#[test]
fn a_top_level_final_state_ends_the_chart() {
    let spec = ChartSpec::new("end")
        .state(
            StateSpec::state("s")
                .on_exit(Action::log("bye"))
                .transition(TransitionSpec::on("stop").to("end")),
        )
        .state(StateSpec::final_state("end").on_exit(Action::log("end exits")));
    let mut a = start(&spec);
    assert!(a.is_running());
    assert_eq!(
        handle(&mut a, "stop"),
        [
            "event stop",
            "exit s",
            "log bye",
            "enter end",
            "exit end",
            "log end exits",
            "done"
        ]
    );
    assert!(!a.is_running());
    assert!(
        handle(&mut a, "stop").is_empty(),
        "an ended chart ignores events"
    );
}

#[test]
fn charts_with_mistakes_are_refused() {
    let one = |s: StateSpec| ChartSpec::new("bad").data("x", "0").state(s);
    let refused = [
        (
            one(StateSpec::state("a").transition(TransitionSpec::on("e").to("nowhere"))),
            "no state `nowhere`",
        ),
        (
            ChartSpec::new("bad")
                .state(StateSpec::state("a"))
                .state(StateSpec::state("a")),
            "another state has this id",
        ),
        (
            one(StateSpec::state("a").on_entry(Action::assign("y", "1"))),
            "not a declared variable",
        ),
        (
            one(StateSpec::state("a").on_entry(Action::log_value("v", "$y + 1"))),
            "not declared",
        ),
        (
            one(StateSpec::state("a").transition(TransitionSpec::eventless().when("In('b')"))),
            "no such state",
        ),
        (
            one(StateSpec::history(
                "h",
                History::Shallow,
                TransitionSpec::eventless().to("h"),
            )),
            "belongs in a state",
        ),
        (
            one(StateSpec::final_state("f").child(StateSpec::state("g"))),
            "final state",
        ),
        (
            one(StateSpec::state("a").on_entry(SendSpec::new("e").after(-1.0))),
            "not a time",
        ),
        (
            one(StateSpec::state("a").on_entry(SendSpec::new("e").internal().after(1.0))),
            "no delay",
        ),
        (
            one(StateSpec::state("a").on_entry(Action::log_value("v", "nosuch(1)"))),
            "not supported",
        ),
        (
            one(StateSpec::state("a").on_entry(Action::raise("two words"))),
            "not an event name",
        ),
        (ChartSpec::new("empty"), "no states"),
    ];
    for (spec, words) in refused {
        match spec.build() {
            Ok(_) => panic!("{spec:?} was accepted"),
            Err(e) => assert!(e.to_string().contains(words), "{e} (wanted `{words}`)"),
        }
    }
}
