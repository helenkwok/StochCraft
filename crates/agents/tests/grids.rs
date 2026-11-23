//! Grids (ROADMAP M4.3, stage A3): neighbourhoods worked out by hand, moves, messages to
//! neighbours, and Conway's Game of Life, whose glider is exact.
#![allow(clippy::panic)] // A model that fails to build or run fails the test.

use std::collections::BTreeSet;
use std::sync::Arc;

use stochcraft_agents::{
    Action, Chart, ChartSpec, Grid, Model, ModelError, Population, SendSpec, Simulation, StateSpec,
    Statistic, Stepping, TransitionSpec,
};

fn build(spec: &ChartSpec) -> Arc<Chart> {
    match spec.build() {
        Ok(c) => Arc::new(c),
        Err(e) => panic!("{e}"),
    }
}

fn start(model: &Model, seed: u64) -> Simulation {
    match Simulation::new(model, seed) {
        Ok(s) => s,
        Err(e) => panic!("{e}"),
    }
}

fn advance(sim: &mut Simulation, to: f64) {
    if let Err(e) = sim.run_until(to) {
        panic!("{e}");
    }
}

fn value(sim: &Simulation, population: &str, serial: u64, var: &str) -> f64 {
    sim.agent(population, serial)
        .and_then(|a| a.value(var))
        .map_or(f64::NAN, |v| v.number())
}

fn cell_chart() -> Arc<Chart> {
    build(
        &ChartSpec::new("cell")
            .data("row", "-1")
            .data("column", "-1")
            .state(StateSpec::state("here")),
    )
}

/// The neighbourhood size of agents 0 (a corner) and 4 (the centre) of a full 3 × 3 grid.
fn corner_and_centre(grid: Grid) -> (f64, f64) {
    let model = Model::new()
        .population(Population::new("g", cell_chart()).count(9).grid(grid))
        .statistic(Statistic::sum("corner", "g", "neighbours('g')").when("serial() = 0"))
        .statistic(Statistic::sum("centre", "g", "neighbours('g')").when("serial() = 4"));
    match model.run(1, 0.0) {
        Ok(r) => (r.series[0].values[0], r.series[1].values[0]),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn neighbourhoods_are_moore_or_von_neumann_wrapped_or_not() {
    assert_eq!(corner_and_centre(Grid::new(3, 3)), (3.0, 8.0));
    assert_eq!(corner_and_centre(Grid::new(3, 3).von_neumann()), (2.0, 4.0));
    assert_eq!(corner_and_centre(Grid::new(3, 3).wrapped()), (8.0, 8.0));
    assert_eq!(
        corner_and_centre(Grid::new(3, 3).von_neumann().wrapped()),
        (4.0, 4.0)
    );
    // Placed in order, row by row, the cell in the agent's variables.
    let model = Model::new().population(
        Population::new("g", cell_chart())
            .count(9)
            .grid(Grid::new(3, 3)),
    );
    let sim = start(&model, 1);
    for s in 0..9 {
        assert_eq!(sim.cell("g", s), Some((s / 3, s % 3)));
        assert_eq!(value(&sim, "g", s, "row"), (s / 3) as f64);
        assert_eq!(value(&sim, "g", s, "column"), (s % 3) as f64);
    }
    assert_eq!(sim.neighbours("g", 0), [1, 3, 4]);
    // Many to a cell: the others in the same cell are neighbours too.
    let model = Model::new().population(
        Population::new("g", cell_chart())
            .count(6)
            .grid(Grid::new(1, 3).many_per_cell()),
    );
    let sim = start(&model, 1);
    assert_eq!(sim.cell("g", 3), Some((0, 0)));
    assert_eq!(sim.neighbours("g", 0), [1, 3, 4]);
    // Too many agents for one per cell, or no `row` and `column`, are refused.
    let full = Model::new().population(
        Population::new("g", cell_chart())
            .count(10)
            .grid(Grid::new(3, 3)),
    );
    assert!(matches!(
        Simulation::new(&full, 1),
        Err(ModelError::Invalid(_))
    ));
    let bare = build(&ChartSpec::new("bare").state(StateSpec::state("here")));
    let bare = Model::new().population(Population::new("g", bare).count(2).grid(Grid::new(3, 3)));
    assert!(matches!(
        Simulation::new(&bare, 1),
        Err(ModelError::Invalid(_))
    ));
}

/// Random placement puts every agent in its own cell.
#[test]
fn random_placement_fills_distinct_cells() {
    let model = Model::new().population(
        Population::new("g", cell_chart())
            .count(30)
            .grid(Grid::new(6, 7).random_placement()),
    );
    let sim = start(&model, 2);
    let cells: BTreeSet<(u64, u64)> = (0..30).filter_map(|s| sim.cell("g", s)).collect();
    assert_eq!(cells.len(), 30);
    let other = start(&model, 3);
    let moved = (0..30).any(|s| sim.cell("g", s) != other.cell("g", s));
    assert!(moved, "another seed places them elsewhere");
}

/// The corner agent tells its neighbours, which arrive in serial order; the centre agent picks
/// 400 neighbours at random, never itself.
#[test]
fn messages_go_to_grid_neighbours() {
    let chart = build(
        &ChartSpec::new("talker")
            .data("row", "-1")
            .data("column", "-1")
            .data("rank", "-1")
            .data("picked", "0")
            .data("n", "0")
            .state(
                StateSpec::state("life")
                    .on_entry(SendSpec::new("go").after(1.0))
                    .transition(
                        TransitionSpec::on("go")
                            .when("serial() = 0")
                            .then(SendSpec::new("hi").to_neighbours("g")),
                    )
                    .transition(
                        TransitionSpec::on("go")
                            .when("serial() = 4 and $n < 400")
                            .then(Action::assign("n", "$n + 1"))
                            .then(SendSpec::new("pick").to_neighbour_random("g"))
                            .then(SendSpec::new("go")),
                    )
                    .transition(
                        TransitionSpec::on("pick").then(Action::assign("picked", "$picked + 1")),
                    )
                    .child(
                        StateSpec::state("listen").transition(
                            TransitionSpec::on("hi")
                                .to("heard")
                                .then(Action::assign("rank", "population-count('g', 'heard')")),
                        ),
                    )
                    .child(StateSpec::state("heard")),
            ),
    );
    let model = Model::new().population(Population::new("g", chart).count(9).grid(Grid::new(3, 3)));
    let mut sim = start(&model, 5);
    advance(&mut sim, 2.0);
    let ranks: Vec<f64> = (0..9).map(|s| value(&sim, "g", s, "rank")).collect();
    assert_eq!(ranks, [-1.0, 0.0, -1.0, 1.0, 2.0, -1.0, -1.0, -1.0, -1.0]);
    let picked: Vec<f64> = (0..9).map(|s| value(&sim, "g", s, "picked")).collect();
    assert_eq!(picked[4], 0.0);
    assert_eq!(picked.iter().sum::<f64>(), 400.0);
    // Uniform over 8: mean 50, standard deviation about 6.6.
    for (s, &p) in picked.iter().enumerate() {
        if s != 4 {
            assert!((p - 50.0).abs() < 30.0, "agent {s}: {picked:?}");
        }
    }
}

/// On a 1 × 4 grid, agent 0 tries the cell of agent 1 (taken: it stays), then the last cell,
/// then one that does not exist (it stays). Agent 1 waits until it has no neighbours, which it
/// finds at exactly 2, when agent 0 moves away. A lone walker with von Neumann neighbours
/// always steps to a cell beside its last one.
#[test]
fn agents_move_to_cells_and_at_random() {
    let mover = build(
        &ChartSpec::new("mover")
            .data("row", "-1")
            .data("column", "-1")
            .data("alone_at", "-1")
            .state(
                StateSpec::state("s")
                    .on_entry(SendSpec::new("a").after(1.0))
                    .on_entry(SendSpec::new("b").after(2.0))
                    .on_entry(SendSpec::new("c").after(3.0))
                    .transition(
                        TransitionSpec::on("a")
                            .when("serial() = 0")
                            .then(Action::move_to("g", "0", "1")),
                    )
                    .transition(
                        TransitionSpec::on("b")
                            .when("serial() = 0")
                            .then(Action::move_to("g", "0", "$column + 3")),
                    )
                    .transition(
                        TransitionSpec::on("c")
                            .when("serial() = 0")
                            .then(Action::move_to("g", "5", "5")),
                    )
                    .transition(
                        TransitionSpec::eventless()
                            .when("serial() = 1 and neighbours('g') = 0")
                            .to("alone")
                            .then(Action::assign("alone_at", "time()")),
                    ),
            )
            .state(StateSpec::state("alone")),
    );
    let model = Model::new().population(Population::new("g", mover).count(2).grid(Grid::new(1, 4)));
    let mut sim = start(&model, 1);
    advance(&mut sim, 1.5);
    assert_eq!(sim.cell("g", 0), Some((0, 0)), "the cell was taken");
    advance(&mut sim, 2.5);
    assert_eq!(sim.cell("g", 0), Some((0, 3)));
    assert_eq!(value(&sim, "g", 0, "column"), 3.0);
    assert_eq!(value(&sim, "g", 1, "alone_at"), 2.0);
    advance(&mut sim, 5.0);
    assert_eq!(sim.cell("g", 0), Some((0, 3)), "there is no cell (5, 5)");

    let walker = build(
        &ChartSpec::new("walker")
            .data("row", "-1")
            .data("column", "-1")
            .state(
                StateSpec::state("s")
                    .on_entry(SendSpec::new("step").after(1.0))
                    .transition(
                        TransitionSpec::on("step")
                            .then(Action::move_random("g"))
                            .then(SendSpec::new("step").after(1.0)),
                    ),
            ),
    );
    let model = Model::new().population(
        Population::new("w", walker)
            .count(1)
            .grid(Grid::new(3, 3).von_neumann()),
    );
    // The population is `w`, so `move_random('g')` names no population of the model.
    assert!(matches!(
        Simulation::new(&model, 1),
        Err(ModelError::Invalid(_))
    ));
    let walker = build(
        &ChartSpec::new("walker")
            .data("row", "-1")
            .data("column", "-1")
            .state(
                StateSpec::state("s")
                    .on_entry(SendSpec::new("step").after(1.0))
                    .transition(
                        TransitionSpec::on("step")
                            .then(Action::move_random("w"))
                            .then(SendSpec::new("step").after(1.0)),
                    ),
            ),
    );
    let model = Model::new().population(
        Population::new("w", walker)
            .count(1)
            .grid(Grid::new(3, 3).von_neumann()),
    );
    let mut sim = start(&model, 7);
    let mut last = (0u64, 0u64);
    let mut seen = BTreeSet::new();
    for t in 1..=200 {
        advance(&mut sim, t as f64 + 0.5);
        let Some(now) = sim.cell("w", 0) else {
            panic!("the walker has no cell at {t}");
        };
        let step = now.0.abs_diff(last.0) + now.1.abs_diff(last.1);
        assert_eq!(step, 1, "from {last:?} to {now:?} at {t}");
        assert_eq!(value(&sim, "w", 0, "row"), now.0 as f64);
        seen.insert(now);
        last = now;
    }
    assert_eq!(seen.len(), 9, "it visits every cell");
}

/// The Game of Life on a 10 × 10 wrapped grid in the stepped mode, updated synchronously: each
/// cell's before-step works out its next state from its neighbours' current one, and its step
/// copies it in. A glider keeps its 5 cells and every 4 steps is the same shape moved one cell
/// down and one right, exactly, for 24 steps (once round the grid and more).
#[test]
fn a_glider_moves_one_cell_diagonally_every_four_steps() {
    // (0, 1), (1, 2), (2, 0), (2, 1), (2, 2): a glider heading down and right.
    let glider: [(u64, u64); 5] = [(0, 1), (1, 2), (2, 0), (2, 1), (2, 2)];
    let start_alive = glider
        .iter()
        .map(|(r, c)| format!("serial() = {}", r * 10 + c))
        .collect::<Vec<_>>()
        .join(" or ");
    let life = build(
        &ChartSpec::new("life")
            .data("row", "-1")
            .data("column", "-1")
            .data("alive", &format!("if({start_alive}, 1, 0)"))
            .data("n", "0")
            .data("next", "0")
            .state(
                StateSpec::state("cell")
                    .transition(
                        TransitionSpec::on("before-step")
                            .then(Action::assign("n", "neighbour-sum('cells', 'alive')"))
                            .then(Action::assign(
                                "next",
                                "if($n = 3 or ($alive = 1 and $n = 2), 1, 0)",
                            )),
                    )
                    .transition(TransitionSpec::on("step").then(Action::assign("alive", "$next"))),
            ),
    );
    let model = Model::new()
        .population(
            Population::new("cells", life)
                .count(100)
                .grid(Grid::new(10, 10).wrapped()),
        )
        .stepped(Stepping::every(1.0));
    let mut sim = start(&model, 1);
    let alive = |sim: &Simulation| -> BTreeSet<(u64, u64)> {
        (0..100)
            .filter(|&s| value(sim, "cells", s, "alive") == 1.0)
            .filter_map(|s| sim.cell("cells", s))
            .collect()
    };
    let shape = |k: u64| -> BTreeSet<(u64, u64)> {
        glider
            .iter()
            .map(|&(r, c)| ((r + k) % 10, (c + k) % 10))
            .collect()
    };
    assert_eq!(alive(&sim), shape(0));
    for step in 1..=24u64 {
        advance(&mut sim, step as f64);
        let now = alive(&sim);
        assert_eq!(now.len(), 5, "step {step}: {now:?}");
        if step % 4 == 0 {
            assert_eq!(now, shape(step / 4), "step {step}");
        } else {
            assert_ne!(now, shape(step / 4), "step {step} is between shapes");
        }
    }
}
