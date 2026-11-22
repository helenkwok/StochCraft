//! Agents (ROADMAP M4.3): state charts with the semantics of W3C SCXML 1.0.
//!
//! This first stage is the state chart alone, with no populations, messages between agents or
//! files yet. A chart is written as a [`ChartSpec`] (the tree of an SCXML document), checked and
//! compiled by [`Chart::new`], and run as an [`Agent`]; a [`Driver`] runs agents on the
//! discrete-event clock (`stochcraft_des::EventQueue`).
//!
//! ```
//! use std::sync::Arc;
//! use stochcraft_agents::{Action, Agent, ChartSpec, Driver, Send, StateSpec, TransitionSpec};
//!
//! // A machine that works for a fixed 2, then waits for a repair.
//! let chart = ChartSpec::new("machine")
//!     .data("jobs", "0")
//!     .state(
//!         StateSpec::state("working")
//!             .on_entry(Send::new("done").after(2.0))
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
//! an assignment to one, a target or `In()` naming no state, are refused by [`Chart::new`]. An
//! action that fails as it runs (a delay or rate that is negative or not a number) puts
//! `error.execution` on the internal queue, records [`Step::Error`] and skips the rest of its
//! block, as SCXML §4.9 says.
//!
//! **Time** (ROADMAP D18): a send to the agent itself with a delay, or with none, becomes an
//! event on the clock; [`Action::Cancel`] makes the pending sends with its id never arrive. A
//! rate send ([`Send::at_rate`]) waits an exponential time with mean 1 / rate, and is drawn again
//! whenever, at the end of a macrostep, a variable its rate reads has a new value: by the
//! exponential's lack of memory the event then comes at the new rate. A send to the internal
//! queue ([`Send::internal`]) acts as `<raise>`.
//!
//! **Random streams** (D18): each expression of the chart draws from a stream of its own,
//! `stochcraft_random::Rng::keyed(seed, "<agent key>/<purpose>")`, where the purpose is where it
//! is in the chart ([`Chart::stream_purposes`]), so two agents never share a stream and making
//! another agent never changes an agent's numbers.
//!
//! **Not here yet:** `<if>`, `<foreach>`, `<script>` (never: D17), `<invoke>`, `<donedata>`,
//! event data and `<param>`, `<send>` to other agents or with a target or type other than the
//! agent itself, `<initial>` elements with content (initial states are the `initial`
//! attribute), late data binding, and reading and writing SCXML files (a later stage, with
//! populations and messages before it).

mod chart;
mod data;
mod driver;
mod interp;

pub use chart::{
    Action, Chart, ChartError, ChartSpec, Data, Delay, History, Kind, Send, StateSpec, Target,
    TransitionSpec,
};
pub use data::FUNCTIONS;
pub use driver::Driver;
pub use interp::{Agent, DEFAULT_MICROSTEP_LIMIT, Record, RunError, Scheduled, Step};
