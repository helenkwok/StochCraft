//! Agents (ROADMAP M4.3): state charts with the semantics of W3C SCXML 1.0.
//!
//! The state chart comes first (stage A1), then populations of agents with messages between them
//! (stage A2), with no files yet. A chart is written as a [`ChartSpec`] (the tree of an SCXML document), checked and
//! compiled by [`Chart::new`], and run as an [`Agent`]; a [`Driver`] runs agents on the
//! discrete-event clock (`stochcraft_des::EventQueue`).
//!
//! ```
//! use std::sync::Arc;
//! use stochcraft_agents::{Action, Agent, ChartSpec, Driver, SendSpec, StateSpec, TransitionSpec};
//!
//! // A machine that works for a fixed 2, then waits for a repair.
//! let chart = ChartSpec::new("machine")
//!     .data("jobs", "0")
//!     .state(
//!         StateSpec::state("working")
//!             .on_entry(SendSpec::new("done").after(2.0))
//!             .transition(
//!                 TransitionSpec::on("done")
//!                     .to("idle")
//!                     .then(Action::assign("jobs", "$jobs + 1")),
//!             ),
//!     )
//!     .state(StateSpec::state("idle"))
//!     .build()?;
//! let mut driver = Driver::new(0.0);
//! let m = driver.add(Agent::new(Arc::new(chart), 1, "machine/0"))?;
//! driver.run_until(10.0)?;
//! let agent = driver.agent(m);
//! assert!(agent.is_some_and(|a| a.is_in("idle")));
//! assert_eq!(agent.and_then(|a| a.value("jobs")).map(|v| v.number()), Some(1.0));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! **The chart.** States are atomic, compound (with an initial child: the `initial`
//! attribute, or the first child), parallel or final, and history pseudo-states are shallow or
//! deep (with a default transition for when there is no history yet). Transitions have an
//! optional event (event descriptors matched as SCXML §3.12.1 says: `buy` matches `buy` and
//! `buy.now`, not `buyer`; `*` matches any event), an optional condition, zero or more targets,
//! and the type external or internal. A transient state, a branch, is a state whose
//! transitions are eventless with conditions. The executable content is `<assign>`, `<raise>`,
//! `<send>` (to the agent itself, at once or after a `delay`, a `delayexpr`, or a rate),
//! `<cancel>` and `<log>`, in each state's `onentry` and `onexit` and in each transition.
//!
//! **The algorithm** is SCXML's Appendix D ([`Agent`]): an event is handled by a macrostep, a
//! series of microsteps; each microstep takes the transitions enabled together (eventless
//! transitions first, then the next internal event; an external event only once neither enables
//! anything), with conflicts resolved by `removeConflictingTransitions` (a descendant's
//! transition wins; otherwise the earlier state in document order); exits the exit set
//! innermost first, recording history; runs the transitions' content in order; enters the entry
//! set outermost first. Entering a final state raises `done.state.<parent>` (and the parallel
//! grandparent's once all its regions are final); a top-level final state ends the chart. A
//! macrostep that takes more than [`DEFAULT_MICROSTEP_LIMIT`] microsteps (or the agent's own
//! limit) stops with [`RunError::MicrostepLimit`], which names the latest transitions.
//!
//! **Data** (ROADMAP D17): the variables of [`ChartSpec::data`] take their first values at the
//! start, in order (early binding). Conditions, values and delays are expressions in BPSim's
//! expression language (`stochcraft_model::expr`), reading variables as `$x` or
//! `getProperty('x')`, with `In('state')` and random draws such as `exponential(2)` beside the
//! language's own functions ([`FUNCTIONS`]). An expression that reads an undeclared variable,
//! an assignment to one, a target or a quoted `In('id')` naming no state, targets that cannot
//! be active together, and states nested more than [`MAX_DEPTH`] deep are refused by
//! [`Chart::new`]; a computed `In()` name is not checked, and one that names no state reads as
//! false. An action that fails as it runs (a delay or rate that is negative, not a number, or
//! too long to be a time) puts
//! `error.execution` on the internal queue, records [`Step::Error`] and skips the rest of its
//! block, as SCXML §4.9 says.
//!
//! **Time** (ROADMAP D18): a send to the agent itself with a delay, or with none, becomes an
//! event on the clock; [`Action::Cancel`] makes the pending sends with its id never arrive. A
//! rate send ([`SendSpec::at_rate`]) waits an exponential time with mean 1 / rate, and is drawn again
//! whenever, at the end of a macrostep, a variable its rate reads has a new value: by the
//! exponential's lack of memory the event then comes at the new rate. A send to the internal
//! queue ([`SendSpec::internal`]) acts as `<raise>`.
//!
//! **Random streams** (D18): each expression of the chart draws from a stream of its own,
//! `stochcraft_random::Rng::keyed(seed, "<n>:<agent key>/<purpose>")`, where `<n>` is the agent
//! key's length in bytes and the purpose is where the expression is in the chart, with each
//! state id written after its length ([`Chart::stream_purposes`], such as
//! `state/4:idle/onentry/1`), so two agents or two expressions never share a stream and making
//! another agent never changes an agent's numbers.
//!
//! **Populations** (stage A2, [`population`]): a [`Model`] holds [`Population`]s of agents of one
//! chart each, created at the start, by arrivals at a rate or by other agents
//! ([`Action::Create`]), and removed when their chart ends or by other agents
//! ([`Action::Remove`]); messages to one agent, a random other agent or all others of a
//! population ([`SendSpec::to_agent`], [`SendSpec::to_random`], [`SendSpec::to_all`]), with a
//! number as data (`getProperty('_event.data')`); conditions on populations
//! (`population-count('p', 's')` and the other functions of [`FUNCTIONS`]) tested again when
//! what they read changes; [`Statistic`]s over time and their [`bands`] across replications;
//! and a stepped mode ([`Stepping`]). A [`Simulation`] runs one replication from a seed.
//!
//! **Networks and grids** (stage A3, [`network`]): a population can have a [`Network`] (random,
//! ring lattice, small world, scale free, within a distance, or a list of links, built at the
//! start from the population's own stream) and a [`Grid`] (Moore or von Neumann neighbours,
//! wrapped or not, one agent per cell or many); messages go along them
//! ([`SendSpec::to_linked`], [`SendSpec::to_linked_random`], [`SendSpec::to_neighbours`],
//! [`SendSpec::to_neighbour_random`]); actions link, unlink and move agents
//! ([`Action::connect`], [`Action::disconnect`], [`Action::move_to`], [`Action::move_random`]);
//! and `degree('p')`, `linked-count('p', 's')`, `neighbour-count('p', 's')` and the others of
//! [`FUNCTIONS`] read an agent's neighbourhood.
//!
//! **Errors.** A [`RunError`] stops its agent for good: its pending sends are dropped (those
//! already on the clock are skipped when they come up) and no [`Step::Done`] is recorded; in a
//! [`Model`] it stops the run.
//!
//! **Not here yet:** `<if>`, `<foreach>`, `<script>` (never: D17), `<invoke>`, `<donedata>`,
//! `<param>`, `<send>` with a delay to other agents, `<initial>` elements with content (initial
//! states are the `initial` attribute), late data binding, and reading and writing SCXML files
//! (stage A4).

mod chart;
mod data;
mod driver;
mod interp;
pub mod network;
pub mod population;

pub use chart::{
    Action, Chart, ChartError, ChartSpec, Data, Delay, History, Kind, MAX_DEPTH, SendSpec,
    StateSpec, Target, To, TransitionSpec,
};
pub use data::FUNCTIONS;
pub use driver::Driver;
pub use interp::{Agent, DEFAULT_MICROSTEP_LIMIT, Record, RunError, Scheduled, Step};
pub use network::{Grid, LinkError, Neighbourhood, Network, Placement, read_links, write_links};
pub use population::{
    Band, Bands, Counts, DEFAULT_RETEST_LIMIT, Measure, Model, ModelError, Order, Population, Run,
    Series, Simulation, Statistic, Stepping, bands,
};
