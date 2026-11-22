//! The distribution functions: counts as large as a `u64` holds are drawn, in time that does
//! not grow with them, and counts that are not whole numbers give NaN.
#![allow(clippy::panic)] // A chart that fails to build or run fails the test.

use std::sync::Arc;

use stochcraft_agents::{Action, Agent, ChartSpec, StateSpec};

fn draw(expr: &str) -> f64 {
    let spec = ChartSpec::new("draws")
        .data("x", "0")
        .state(StateSpec::state("s").on_entry(Action::assign("x", expr)));
    let chart = match spec.build() {
        Ok(c) => Arc::new(c),
        Err(e) => panic!("{e}"),
    };
    let mut a = Agent::new(chart, 11, "a");
    if let Err(e) = a.start(0.0) {
        panic!("{e}");
    }
    a.value("x").map_or(f64::NAN, |v| v.number())
}

#[test]
fn large_counts_are_drawn() {
    // 10^15 + 1 trials, beyond the 10^15 the first version allowed.
    let n = 1_000_000_000_000_001.0;
    let b = draw("binomial(0.5, 1000000000000001)");
    assert!(b.is_finite() && (0.0..=n).contains(&b), "{b}");
    assert!((b - n / 2.0).abs() < 10.0 * (n / 4.0_f64).sqrt(), "{b}");
    // An Erlang of 10^9 phases with mean 1: a gamma draw, its standard deviation 10^-4.5.
    let e = draw("erlang(1, 1000000000)");
    assert!((e - 1.0).abs() < 0.001, "{e}");
}

#[test]
fn counts_that_are_not_whole_numbers_give_nan() {
    for expr in [
        "binomial(0.5, 2.5)",
        "binomial(0.5, -1)",
        "binomial(0.5, 1 div 0)",
        "erlang(1, 0)",
        "erlang(1, 1.5)",
    ] {
        assert!(draw(expr).is_nan(), "{expr}");
    }
}
