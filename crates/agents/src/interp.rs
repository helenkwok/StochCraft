//! One running chart: an [`Agent`], interpreted by the algorithm of SCXML's Appendix D.
//!
//! The procedures below keep Appendix D's names in their doc comments (`selectTransitions`,
//! `removeConflictingTransitions`, `microstep`, `exitStates`, `enterStates`, ...) and follow its
//! steps in its order. Sets that Appendix D keeps as ordered sets are vectors here, kept in the
//! order the algorithm adds to them; states are numbered in document order, so entry order is
//! ascending numbers and exit order descending.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::Arc;

use stochcraft_model::expr::Value;
use stochcraft_random::{Rng, math};

use crate::chart::{Chart, Code, DelayExec, Exec, History, NodeKind, ToExec};
use crate::data;

/// The microsteps a macrostep may take before the run is stopped as looping.
pub const DEFAULT_MICROSTEP_LIMIT: u64 = 10_000;

/// How many of the latest transitions a [`RunError::MicrostepLimit`] names.
const PATH_LEN: usize = 12;

/// Something an agent did, as its trace records it.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// An event was taken off a queue to be handled (whether or not a transition took it).
    Event { name: String, internal: bool },
    /// A state was entered (before its entry actions run).
    Enter(String),
    /// A state was exited (before its exit actions run).
    Exit(String),
    /// A `<log>`: its label, then `: ` and its value if it has an expression.
    Log(String),
    /// An action failed; `error.execution` was put on the internal queue and the rest of its
    /// block skipped (SCXML §4.9).
    Error(String),
    /// A top-level final state was reached and the chart has ended.
    Done,
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Step::Event { name, .. } => write!(f, "event {name}"),
            Step::Enter(s) => write!(f, "enter {s}"),
            Step::Exit(s) => write!(f, "exit {s}"),
            Step::Log(s) => write!(f, "log {s}"),
            Step::Error(s) => write!(f, "error {s}"),
            Step::Done => f.write_str("done"),
        }
    }
}

/// A [`Step`] and the time it happened.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub time: f64,
    pub step: Step,
}

/// An event an agent has sent itself, for its driver to put on the clock: deliver `token` back
/// with [`Agent::fire`] `delay` time units from now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scheduled {
    pub token: u64,
    pub delay: f64,
}

/// Why a run stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum RunError {
    /// A macrostep took more microsteps than the limit: the chart loops without waiting for an
    /// event. `path` is the latest transitions, oldest first, as `source -> targets on event`.
    /// The agent has stopped, its pending sends and internal events dropped.
    MicrostepLimit {
        chart: String,
        agent: String,
        time: f64,
        limit: u64,
        path: Vec<String>,
    },
    /// The clock refused an event (its time would not be a finite number). The agent has
    /// stopped, its other pending sends dropped.
    Schedule {
        agent: String,
        time: f64,
        delay: f64,
    },
    /// After one event in a [`Model`](crate::Model), more than `limit` re-tests of conditions
    /// on populations took a transition: agents keep changing each other without waiting for
    /// an event.
    RetestLimit { time: f64, limit: u64 },
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::MicrostepLimit {
                chart,
                agent,
                time,
                limit,
                path,
            } => write!(
                f,
                "agent `{agent}` (state chart `{chart}`) took more than {limit} transitions at \
                 time {time} without waiting for an event, so it loops; the last ones: {}",
                path.join(", ")
            ),
            RunError::Schedule { agent, time, delay } => write!(
                f,
                "agent `{agent}` sent an event {delay} after time {time}, which is not a time"
            ),
            RunError::RetestLimit { time, limit } => write!(
                f,
                "at time {time}, agents whose conditions read populations changed each other \
                 more than {limit} times without waiting for an event, so they loop"
            ),
        }
    }
}

impl std::error::Error for RunError {}

/// What an agent's expressions see beyond the agent itself: the populations of a model. It
/// answers `serial()` and the population functions ([`data::POPULATION_FUNCTIONS`]); `me` is the
/// agent evaluating, which its model has taken out of its population while it runs.
pub(crate) trait World {
    fn call(&self, name: &str, args: &[Value], me: &Agent) -> Value;
}

/// No model: the population functions read as empty and `serial()` as NaN.
pub(crate) struct NoWorld;

impl World for NoWorld {
    fn call(&self, name: &str, _: &[Value], _: &Agent) -> Value {
        if name == "serial" {
            Value::Number(f64::NAN)
        } else {
            Value::Text(String::new())
        }
    }
}

/// Who a message goes to, as the sender resolved it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Recipient {
    /// The agent of this serial number (as the expression gave it: not yet checked to be one).
    One { population: String, serial: f64 },
    /// One agent other than the sender, picked by this draw (64 random bits).
    Random { population: String, draw: u64 },
    /// Every agent but the sender.
    All { population: String },
}

/// Something a macrostep sent out, in the order its actions ran.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Out {
    /// An event to the agent itself.
    Own(Scheduled),
    Message {
        to: Recipient,
        event: String,
        data: Option<f64>,
    },
    Create(String),
    Remove {
        population: String,
        serial: f64,
    },
}

/// A send that has not arrived yet.
#[derive(Debug, Clone)]
struct Pending {
    event: String,
    send_id: Option<String>,
    rate: Option<RateWatch>,
    data: Option<f64>,
}

/// A rate send's expression and the values of the variables it read when it was drawn.
#[derive(Debug, Clone)]
struct RateWatch {
    code: Code,
    reads: Vec<(u32, Value)>,
}

/// One state chart running: its configuration, its variables, its queues and its random
/// streams.
///
/// Its streams are `stochcraft_random::Rng::keyed(seed, "<n>:<key>/<purpose>")`, where `<n>` is
/// the length of the key in bytes, one for each
/// expression in the chart ([`Chart::stream_purposes`]), made when first drawn from: they
/// depend only on the seed, the agent's key and the expression, never on other agents or on
/// the order agents are made.
#[derive(Debug, Clone)]
pub struct Agent {
    chart: Arc<Chart>,
    key: String,
    seed: u64,
    streams: BTreeMap<u32, Rng>,
    active: Vec<bool>,
    values: Vec<Value>,
    history: BTreeMap<u32, Vec<u32>>,
    internal: VecDeque<String>,
    event: String,
    event_data: Option<f64>,
    now: f64,
    started: bool,
    running: bool,
    limit: u64,
    microsteps: u64,
    path: VecDeque<String>,
    pending: BTreeMap<u64, Pending>,
    next_token: u64,
    outbox: Vec<Out>,
    trace: Option<Vec<Record>>,
}

impl Agent {
    /// An agent running `chart`, its streams under `seed` and named by `key` (such as
    /// `population/0/agent/17`). It starts with [`Agent::start`].
    pub fn new(chart: Arc<Chart>, seed: u64, key: impl Into<String>) -> Self {
        let n = chart.nodes.len();
        let vars = chart.vars.len();
        Self {
            chart,
            key: key.into(),
            seed,
            streams: BTreeMap::new(),
            active: vec![false; n],
            values: vec![Value::Text(String::new()); vars],
            history: BTreeMap::new(),
            internal: VecDeque::new(),
            event: String::new(),
            event_data: None,
            now: 0.0,
            started: false,
            running: false,
            limit: DEFAULT_MICROSTEP_LIMIT,
            microsteps: 0,
            path: VecDeque::new(),
            pending: BTreeMap::new(),
            next_token: 0,
            outbox: Vec::new(),
            trace: None,
        }
    }

    /// Sets the most microsteps one macrostep may take ([`DEFAULT_MICROSTEP_LIMIT`]).
    pub fn with_microstep_limit(mut self, limit: u64) -> Self {
        self.limit = limit;
        self
    }

    /// Records what the agent does ([`Agent::trace`]).
    pub fn with_trace(mut self) -> Self {
        self.trace = Some(Vec::new());
        self
    }

    pub fn chart(&self) -> &Chart {
        &self.chart
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// Whether it has started and not ended (a top-level final state ends it, and so does a
    /// [`RunError`]).
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Whether the state `id` is active.
    pub fn is_in(&self, id: &str) -> bool {
        self.chart
            .ids
            .get(id)
            .is_some_and(|&i| self.active[i as usize])
    }

    /// The active states, in document order.
    pub fn active_states(&self) -> Vec<&str> {
        (0..self.chart.len())
            .filter(|&i| self.active[i as usize])
            .map(|i| self.chart.node(i).id.as_str())
            .collect()
    }

    /// A variable's value.
    pub fn value(&self, name: &str) -> Option<&Value> {
        let i = *self.chart.var_index.get(name)?;
        self.values.get(i as usize)
    }

    /// What it has done, if it was made [`Agent::with_trace`].
    pub fn trace(&self) -> &[Record] {
        self.trace.as_deref().unwrap_or(&[])
    }

    /// How many sends have not arrived yet (cancelled ones not counted).
    pub fn pending_sends(&self) -> usize {
        self.pending.len()
    }

    /// The events it has sent itself since the last call, for the clock. A send cancelled or
    /// redrawn since it was made is left out, and so are messages to other agents and the
    /// agents it creates or removes, which only a [`Model`](crate::Model) delivers.
    pub fn take_sends(&mut self) -> Vec<Scheduled> {
        self.take_out()
            .into_iter()
            .filter_map(|o| match o {
                Out::Own(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// Everything it has sent out since the last call, in the order its actions ran (its own
    /// sends cancelled or redrawn since left out).
    pub(crate) fn take_out(&mut self) -> Vec<Out> {
        let outbox = std::mem::take(&mut self.outbox);
        outbox
            .into_iter()
            .filter(|o| match o {
                Out::Own(s) => self.pending.contains_key(&s.token),
                _ => true,
            })
            .collect()
    }

    /// Stops the agent after a [`RunError`]: it records the reason, and drops its pending sends
    /// (so the ones already on a clock are skipped when they arrive) and its internal events.
    /// Its states stay as they were, without exit actions, and no [`Step::Done`] is recorded.
    pub(crate) fn halt(&mut self, reason: String) {
        self.running = false;
        self.record(Step::Error(reason));
        self.pending.clear();
        self.outbox.clear();
        self.internal.clear();
    }

    /// Starts the chart at time `now`: its variables take their first values in declaration
    /// order, its initial states are entered, and it runs until it waits for an event (one
    /// macrostep).
    pub fn start(&mut self, now: f64) -> Result<(), RunError> {
        self.start_in(now, &NoWorld)
    }

    pub(crate) fn start_in(&mut self, now: f64, w: &dyn World) -> Result<(), RunError> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        self.running = true;
        self.now = now;
        self.begin_macrostep();
        let chart = Arc::clone(&self.chart);
        for (i, (_, code)) in chart.vars.iter().enumerate() {
            let v = self.eval(&chart, code, w);
            if let Some(slot) = self.values.get_mut(i) {
                *slot = v;
            }
        }
        self.enter_states(&chart, &[chart.initial], w);
        self.settle(&chart, w)
    }

    /// Handles the external event `name` at time `now`: one macrostep (the transitions it
    /// enables, then every eventless transition and internal event that follows). Ignored if
    /// the chart is not running.
    pub fn handle(&mut self, name: &str, now: f64) -> Result<(), RunError> {
        self.handle_in(name, None, now, &NoWorld).map(|_| ())
    }

    /// [`Agent::handle`] with the event's data and a world; returns whether a transition took
    /// the event (false also if the chart is not running).
    pub(crate) fn handle_in(
        &mut self,
        name: &str,
        data: Option<f64>,
        now: f64,
        w: &dyn World,
    ) -> Result<bool, RunError> {
        if !self.running {
            return Ok(false);
        }
        self.now = now;
        self.begin_macrostep();
        self.record(Step::Event {
            name: name.to_string(),
            internal: false,
        });
        self.event = name.to_string();
        self.event_data = data;
        let chart = Arc::clone(&self.chart);
        let enabled = self.select(&chart, Some(name), w);
        let taken = !enabled.is_empty();
        if taken {
            self.microstep(&chart, &enabled, w)?;
        }
        self.settle(&chart, w)?;
        Ok(taken)
    }

    /// Tests its eventless transitions again at time `now`, as a macrostep with no event (for a
    /// condition that reads a population that has changed). Returns whether it took a
    /// transition.
    pub(crate) fn retest(&mut self, now: f64, w: &dyn World) -> Result<bool, RunError> {
        if !self.running {
            return Ok(false);
        }
        self.now = now;
        self.begin_macrostep();
        let chart = Arc::clone(&self.chart);
        self.settle(&chart, w)?;
        Ok(self.microsteps > 0)
    }

    /// Ends the chart at time `now` as SCXML's `exitInterpreter` does: every active state is
    /// exited, its exit actions run (their messages still go out), and the pending sends are
    /// dropped.
    pub(crate) fn stop(&mut self, now: f64, w: &dyn World) {
        if !self.running {
            return;
        }
        self.now = now;
        self.begin_macrostep();
        self.running = false;
        let chart = Arc::clone(&self.chart);
        self.exit_interpreter(&chart, w);
    }

    /// Whether its last macrostep took a transition.
    pub(crate) fn took_steps(&self) -> bool {
        self.microsteps > 0
    }

    /// Evaluates an expression over the agent as it is now, drawing nothing (a statistic's
    /// expression, which [`Chart`]'s `agent_expression` checked).
    pub(crate) fn evaluate(&self, code: &Code, w: &dyn World) -> Value {
        let chart = &self.chart;
        let property = |name: &str| self.lookup(chart, name);
        let provided = |name: &str, args: &[Value]| self.provided(chart, name, args, None, w);
        code.expr.eval_with(&property, &provided)
    }

    /// Sends the agent the external event `name`, to arrive `delay` from now (through
    /// [`Agent::take_sends`]). Returns its token, or `None` if the agent has ended or the delay
    /// is not a time of 0 or more.
    pub fn post(&mut self, name: &str, delay: f64) -> Option<u64> {
        if (self.started && !self.running) || !(delay.is_finite() && delay >= 0.0) {
            return None;
        }
        let pending = Pending {
            event: name.to_string(),
            send_id: None,
            rate: None,
            data: None,
        };
        Some(self.add_pending(pending, Some(delay)))
    }

    /// Delivers the send `token` at time `now`, handling its event. Returns false (and does
    /// nothing) if the send was cancelled or redrawn since, or the chart has ended.
    pub fn fire(&mut self, token: u64, now: f64) -> Result<bool, RunError> {
        self.fire_in(token, now, &NoWorld)
    }

    pub(crate) fn fire_in(
        &mut self,
        token: u64,
        now: f64,
        w: &dyn World,
    ) -> Result<bool, RunError> {
        let Some(p) = self.pending.remove(&token) else {
            return Ok(false);
        };
        if !self.running {
            return Ok(false);
        }
        self.handle_in(&p.event, p.data, now, w)?;
        Ok(true)
    }

    fn record(&mut self, step: Step) {
        let time = self.now;
        if let Some(t) = &mut self.trace {
            t.push(Record { time, step });
        }
    }

    fn begin_macrostep(&mut self) {
        self.microsteps = 0;
        self.path.clear();
    }

    fn add_pending(&mut self, pending: Pending, delay: Option<f64>) -> u64 {
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);
        self.pending.insert(token, pending);
        if let Some(delay) = delay {
            self.outbox.push(Out::Own(Scheduled { token, delay }));
        }
        token
    }

    /// The inner loop of `mainEventLoop`: eventless transitions first, then internal events,
    /// until neither enables anything; then rate sends whose variables changed are redrawn
    /// (which may raise `error.execution`, handled in turn). Ends the chart (`exitInterpreter`)
    /// if a top-level final state was entered.
    fn settle(&mut self, chart: &Chart, w: &dyn World) -> Result<(), RunError> {
        loop {
            while self.running {
                let mut enabled = self.select(chart, None, w);
                if enabled.is_empty() {
                    let Some(ev) = self.internal.pop_front() else {
                        break;
                    };
                    self.record(Step::Event {
                        name: ev.clone(),
                        internal: true,
                    });
                    // SCXML sets `_event` before selecting, so conditions read it.
                    self.event.clone_from(&ev);
                    self.event_data = None;
                    enabled = self.select(chart, Some(&ev), w);
                }
                if !enabled.is_empty() {
                    self.microstep(chart, &enabled, w)?;
                }
            }
            if !self.running {
                break;
            }
            self.redraw_rates(chart, w);
            if self.internal.is_empty() {
                break;
            }
        }
        if !self.running {
            self.exit_interpreter(chart, w);
        }
        Ok(())
    }

    /// `exitInterpreter`: exits every active state in exit order, running its exit actions, and
    /// drops the sends to itself still pending (its messages to other agents still go out).
    fn exit_interpreter(&mut self, chart: &Chart, w: &dyn World) {
        let mut states: Vec<u32> = (0..chart.len())
            .filter(|&i| self.active[i as usize])
            .collect();
        states.reverse();
        for s in states {
            self.record(Step::Exit(chart.node(s).id.clone()));
            self.execute(chart, &chart.node(s).on_exit, w);
            self.active[s as usize] = false;
        }
        self.pending.clear();
        self.outbox.retain(|o| !matches!(o, Out::Own(_)));
        self.internal.clear();
        self.record(Step::Done);
    }

    /// `selectEventlessTransitions` (with `event` None) and `selectTransitions`: for each
    /// active atomic state in document order, the first transition in document order of it or
    /// its nearest ancestor that matches and whose condition holds; then
    /// `removeConflictingTransitions`.
    fn select(&mut self, chart: &Chart, event: Option<&str>, w: &dyn World) -> Vec<u32> {
        let mut enabled: Vec<u32> = Vec::new();
        for state in 0..chart.len() {
            if !self.active[state as usize] || !chart.is_atomic(state) {
                continue;
            }
            let mut s = Some(state);
            'chain: while let Some(cur) = s {
                for &t in &chart.node(cur).transitions {
                    let tr = chart.transition(t);
                    let matched = match (event, &tr.events) {
                        (None, None) => true,
                        (Some(name), Some(ds)) => ds.iter().any(|d| data::matches(d, name)),
                        _ => false,
                    };
                    if !matched {
                        continue;
                    }
                    let holds = match &tr.cond {
                        None => true,
                        Some(c) => self.eval(chart, c, w).boolean(),
                    };
                    if holds {
                        if !enabled.contains(&t) {
                            enabled.push(t);
                        }
                        break 'chain;
                    }
                }
                s = chart.node(cur).parent;
            }
        }
        self.remove_conflicting(chart, enabled)
    }

    /// `removeConflictingTransitions`: of two transitions whose exit sets meet, the one whose
    /// source is a descendant of the other's wins; otherwise the one selected first (by the
    /// earlier state in document order).
    fn remove_conflicting(&self, chart: &Chart, enabled: Vec<u32>) -> Vec<u32> {
        let mut filtered: Vec<u32> = Vec::new();
        for t1 in enabled {
            let x1 = self.exit_set(chart, &[t1]);
            let mut preempted = false;
            let mut remove = Vec::new();
            for &t2 in &filtered {
                let x2 = self.exit_set(chart, &[t2]);
                if x1.iter().any(|s| x2.contains(s)) {
                    if chart.is_descendant(chart.transition(t1).source, chart.transition(t2).source)
                    {
                        remove.push(t2);
                    } else {
                        preempted = true;
                        break;
                    }
                }
            }
            if !preempted {
                filtered.retain(|t| !remove.contains(t));
                filtered.push(t1);
            }
        }
        filtered
    }

    /// `getEffectiveTargetStates`: the targets, with each history state replaced by its
    /// history, or if it has none, by its default transition's targets.
    fn effective_targets(&self, chart: &Chart, t: u32) -> Vec<u32> {
        let mut out = Vec::new();
        self.effective_into(chart, t, &mut out);
        out
    }

    fn effective_into(&self, chart: &Chart, t: u32, out: &mut Vec<u32>) {
        for &s in &chart.transition(t).targets {
            if matches!(chart.node(s).kind, NodeKind::History(_)) {
                match self.history.get(&s) {
                    Some(h) => {
                        for &x in h {
                            if !out.contains(&x) {
                                out.push(x);
                            }
                        }
                    }
                    None => {
                        if let Some(&d) = chart.node(s).transitions.first() {
                            self.effective_into(chart, d, out);
                        }
                    }
                }
            } else if !out.contains(&s) {
                out.push(s);
            }
        }
    }

    /// `getTransitionDomain`: none for a targetless transition; the source for an internal
    /// transition from a compound state to its own descendants; otherwise the least common
    /// compound ancestor of the source and the targets.
    fn domain(&self, chart: &Chart, t: u32) -> Option<u32> {
        let targets = self.effective_targets(chart, t);
        if targets.is_empty() {
            return None;
        }
        let tr = chart.transition(t);
        if tr.internal
            && chart.is_compound_or_root(tr.source)
            && targets.iter().all(|&s| chart.is_descendant(s, tr.source))
        {
            return Some(tr.source);
        }
        let mut list = vec![tr.source];
        list.extend(targets);
        Some(chart.lcca(&list))
    }

    /// `computeExitSet`: the active states inside each targeted transition's domain.
    fn exit_set(&self, chart: &Chart, transitions: &[u32]) -> Vec<u32> {
        let mut out = Vec::new();
        for &t in transitions {
            if chart.transition(t).targets.is_empty() {
                continue;
            }
            let Some(domain) = self.domain(chart, t) else {
                continue;
            };
            for s in 0..chart.len() {
                if self.active[s as usize] && chart.is_descendant(s, domain) && !out.contains(&s) {
                    out.push(s);
                }
            }
        }
        out
    }

    /// `microstep`: exit the sources' states, run the transitions' content, enter the targets.
    ///
    /// The content runs in document order of the transitions, as SCXML §3.13 (normative) says:
    /// "It MUST then execute the executable content contained in the transitions in document
    /// order". Appendix D (informative) runs it in the order the atomic states selected the
    /// transitions; the two differ only when a state's transition and an ancestor's are both
    /// taken (targetless, or not conflicting). A state's transitions count as written before
    /// its children's.
    fn microstep(&mut self, chart: &Chart, enabled: &[u32], w: &dyn World) -> Result<(), RunError> {
        if self.microsteps >= self.limit {
            self.halt(format!(
                "more than {} transitions in one macrostep",
                self.limit
            ));
            return Err(RunError::MicrostepLimit {
                chart: chart.name.clone(),
                agent: self.key.clone(),
                time: self.now,
                limit: self.limit,
                path: self.path.iter().cloned().collect(),
            });
        }
        self.microsteps += 1;
        for &t in enabled {
            let d = self.describe(chart, t);
            self.path.push_back(d);
            if self.path.len() > PATH_LEN {
                self.path.pop_front();
            }
        }
        self.exit_states(chart, enabled, w);
        let mut in_document_order = enabled.to_vec();
        in_document_order.sort_unstable();
        for t in in_document_order {
            self.execute(chart, &chart.transition(t).actions, w);
        }
        self.enter_states(chart, enabled, w);
        Ok(())
    }

    fn describe(&self, chart: &Chart, t: u32) -> String {
        let tr = chart.transition(t);
        let source = &chart.node(tr.source).id;
        let targets: Vec<&str> = tr
            .targets
            .iter()
            .map(|&s| chart.node(s).id.as_str())
            .collect();
        let to = if targets.is_empty() {
            format!("{source} (no target)")
        } else {
            format!("{source} -> {}", targets.join(" "))
        };
        match tr.events {
            Some(_) => format!("{to} on {}", self.event),
            None => to,
        }
    }

    /// `exitStates`: the exit set in exit order (innermost first, then reverse document order);
    /// every history of those states is recorded first, then each state's exit actions run and
    /// it leaves the configuration.
    fn exit_states(&mut self, chart: &Chart, enabled: &[u32], w: &dyn World) {
        let mut exits = self.exit_set(chart, enabled);
        exits.sort_unstable();
        exits.reverse();
        for &s in &exits {
            for &h in &chart.node(s).histories {
                let deep = matches!(chart.node(h).kind, NodeKind::History(History::Deep));
                let value: Vec<u32> = (0..chart.len())
                    .filter(|&x| {
                        self.active[x as usize]
                            && if deep {
                                chart.is_atomic(x) && chart.is_descendant(x, s)
                            } else {
                                chart.node(x).parent == Some(s)
                            }
                    })
                    .collect();
                self.history.insert(h, value);
            }
        }
        for s in exits {
            self.record(Step::Exit(chart.node(s).id.clone()));
            self.execute(chart, &chart.node(s).on_exit, w);
            self.active[s as usize] = false;
        }
    }

    /// `enterStates`: the entry set (`computeEntrySet`) in entry order (document order); each
    /// state joins the configuration and runs its entry actions, then a history's default
    /// content if it was entered through one (§3.10: after the parent's entry actions; when the
    /// parent stays active, as for an internal transition to its own history, before its first
    /// descendant is entered); entering a final state raises `done.state.<id>`
    /// of its parent, and of the parallel grandparent once all of that one's children are
    /// final, or ends the chart if it is top-level.
    fn enter_states(&mut self, chart: &Chart, enabled: &[u32], w: &dyn World) {
        let mut to_enter = Vec::new();
        let mut default_history: BTreeMap<u32, u32> = BTreeMap::new();
        for &t in enabled {
            for &s in &chart.transition(t).targets {
                self.add_descendants(chart, s, &mut to_enter, &mut default_history);
            }
            let ancestor = self.domain(chart, t).unwrap_or(0);
            for s in self.effective_targets(chart, t) {
                self.add_ancestors(chart, s, ancestor, &mut to_enter, &mut default_history);
            }
        }
        to_enter.sort_unstable();
        // Default histories whose parent is already active and not entered again.
        let mut waiting: Vec<(u32, u32)> = default_history
            .iter()
            .filter(|(p, _)| !to_enter.contains(p))
            .map(|(&p, &h)| (p, h))
            .collect();
        for s in to_enter {
            let mut k = 0;
            while k < waiting.len() {
                let (p, h) = waiting[k];
                if chart.is_descendant(s, p) {
                    waiting.remove(k);
                    self.history_default_content(chart, h, w);
                } else {
                    k += 1;
                }
            }
            self.active[s as usize] = true;
            self.record(Step::Enter(chart.node(s).id.clone()));
            self.execute(chart, &chart.node(s).on_entry, w);
            // A compound state's default entry would run its <initial> transition's content
            // here; initial states are given by the `initial` attribute, which has none.
            if let Some(&h) = default_history.get(&s) {
                self.history_default_content(chart, h, w);
            }
            if chart.node(s).kind != NodeKind::Final {
                continue;
            }
            match chart.node(s).parent {
                None | Some(0) => self.running = false,
                Some(parent) => {
                    self.internal
                        .push_back(format!("done.state.{}", chart.node(parent).id));
                    if let Some(g) = chart.node(parent).parent
                        && chart.node(g).kind == NodeKind::Parallel
                        && chart.child_states(g).all(|c| self.in_final(chart, c))
                    {
                        self.internal
                            .push_back(format!("done.state.{}", chart.node(g).id));
                    }
                }
            }
        }
        for (_, h) in waiting {
            self.history_default_content(chart, h, w);
        }
    }

    /// Runs the content of history state `h`'s default transition.
    fn history_default_content(&mut self, chart: &Chart, h: u32, w: &dyn World) {
        if let Some(&d) = chart.node(h).transitions.first() {
            self.execute(chart, &chart.transition(d).actions, w);
        }
    }

    /// `addDescendantStatesToEnter`.
    fn add_descendants(
        &self,
        chart: &Chart,
        s: u32,
        to_enter: &mut Vec<u32>,
        default_history: &mut BTreeMap<u32, u32>,
    ) {
        let node = chart.node(s);
        if matches!(node.kind, NodeKind::History(_)) {
            let parent = node.parent.unwrap_or(0);
            let targets = match self.history.get(&s) {
                Some(h) => h.clone(),
                None => {
                    default_history.insert(parent, s);
                    match node.transitions.first() {
                        Some(&d) => chart.transition(d).targets.clone(),
                        None => Vec::new(),
                    }
                }
            };
            for &x in &targets {
                self.add_descendants(chart, x, to_enter, default_history);
            }
            for &x in &targets {
                self.add_ancestors(chart, x, parent, to_enter, default_history);
            }
            return;
        }
        if !to_enter.contains(&s) {
            to_enter.push(s);
        }
        match node.kind {
            NodeKind::Compound => {
                for &x in &node.initial {
                    self.add_descendants(chart, x, to_enter, default_history);
                }
                for &x in &node.initial {
                    self.add_ancestors(chart, x, s, to_enter, default_history);
                }
            }
            NodeKind::Parallel => {
                for child in chart.child_states(s) {
                    if !to_enter.iter().any(|&x| chart.is_descendant(x, child)) {
                        self.add_descendants(chart, child, to_enter, default_history);
                    }
                }
            }
            _ => {}
        }
    }

    /// `addAncestorStatesToEnter`: the proper ancestors of `s` below `ancestor`, and the
    /// regions of any parallel one among them.
    fn add_ancestors(
        &self,
        chart: &Chart,
        s: u32,
        ancestor: u32,
        to_enter: &mut Vec<u32>,
        default_history: &mut BTreeMap<u32, u32>,
    ) {
        let mut anc = chart.node(s).parent;
        while let Some(a) = anc {
            if a == ancestor || a == 0 {
                break;
            }
            if !to_enter.contains(&a) {
                to_enter.push(a);
            }
            if chart.node(a).kind == NodeKind::Parallel {
                for child in chart.child_states(a) {
                    if !to_enter.iter().any(|&x| chart.is_descendant(x, child)) {
                        self.add_descendants(chart, child, to_enter, default_history);
                    }
                }
            }
            anc = chart.node(a).parent;
        }
    }

    /// `isInFinalState`.
    fn in_final(&self, chart: &Chart, s: u32) -> bool {
        match chart.node(s).kind {
            NodeKind::Compound => chart
                .child_states(s)
                .any(|c| chart.node(c).kind == NodeKind::Final && self.active[c as usize]),
            NodeKind::Parallel => chart.child_states(s).all(|c| self.in_final(chart, c)),
            _ => false,
        }
    }

    /// Runs a block of executable content; the first action that fails puts
    /// `error.execution` on the internal queue and skips the rest (SCXML §4.9).
    fn execute(&mut self, chart: &Chart, block: &[Exec], w: &dyn World) {
        for e in block {
            if let Err(message) = self.run(chart, e, w) {
                self.record(Step::Error(message));
                self.internal.push_back("error.execution".to_string());
                break;
            }
        }
    }

    fn run(&mut self, chart: &Chart, e: &Exec, w: &dyn World) -> Result<(), String> {
        match e {
            Exec::Assign { var, code } => {
                let v = self.eval(chart, code, w);
                if let Some(slot) = self.values.get_mut(*var as usize) {
                    *slot = v;
                }
            }
            Exec::Raise(event) => self.internal.push_back(event.clone()),
            Exec::Cancel(id) => self.pending.retain(|_, p| p.send_id.as_ref() != Some(id)),
            Exec::Log { label, code } => {
                let text = match code {
                    Some(c) => format!("{label}: {}", self.eval(chart, c, w).text()),
                    None => label.clone(),
                };
                self.record(Step::Log(text));
            }
            Exec::Create(population) => self.outbox.push(Out::Create(population.clone())),
            Exec::Remove { population, serial } => {
                let serial = self.eval(chart, serial, w).number();
                self.outbox.push(Out::Remove {
                    population: population.clone(),
                    serial,
                });
            }
            Exec::SendSpec(s) => {
                if s.internal {
                    self.internal.push_back(s.event.clone());
                    return Ok(());
                }
                let data = s.data.as_ref().map(|c| self.eval(chart, c, w).number());
                if let Some(to) = &s.to {
                    let to = match to {
                        ToExec::Agent { population, serial } => Recipient::One {
                            population: population.clone(),
                            serial: self.eval(chart, serial, w).number(),
                        },
                        ToExec::Random { population, site } => {
                            let mut rng = self.take_stream(chart, *site);
                            let draw = rng.next_u64();
                            self.streams.insert(*site, rng);
                            Recipient::Random {
                                population: population.clone(),
                                draw,
                            }
                        }
                        ToExec::All { population } => Recipient::All {
                            population: population.clone(),
                        },
                    };
                    self.outbox.push(Out::Message {
                        to,
                        event: s.event.clone(),
                        data,
                    });
                    return Ok(());
                }
                let mut pending = Pending {
                    event: s.event.clone(),
                    send_id: s.id.clone(),
                    rate: None,
                    data,
                };
                let delay = match &s.delay {
                    DelayExec::None => Some(0.0),
                    DelayExec::Fixed(d) => Some(*d),
                    DelayExec::Expr(c) => {
                        let d = self.eval(chart, c, w).number();
                        if !(d.is_finite() && d >= 0.0) {
                            return Err(format!(
                                "the delay of the send of `{}` is {d}, not a time of 0 or more",
                                s.event
                            ));
                        }
                        Some(d)
                    }
                    DelayExec::Rate { code, reads } => {
                        let (watch, delay) = self.draw_rate(chart, code, reads, w)?;
                        pending.rate = Some(watch);
                        delay
                    }
                };
                self.add_pending(pending, delay);
            }
        }
        Ok(())
    }

    /// Evaluates a rate and draws its exponential delay (none while the rate is 0).
    fn draw_rate(
        &mut self,
        chart: &Chart,
        code: &Code,
        reads: &[u32],
        w: &dyn World,
    ) -> Result<(RateWatch, Option<f64>), String> {
        let rate = self.eval(chart, code, w).number();
        if !(rate.is_finite() && rate >= 0.0) {
            return Err(format!("the rate is {rate}, not a number of 0 or more"));
        }
        let delay = if rate > 0.0 {
            // −ln(U) / rate, not an exponential of mean 1 / rate: a tiny rate's reciprocal
            // can be infinite where the delay is not.
            let mut rng = self.take_stream(chart, code.site);
            let d = (0.0 - math::ln(rng.uniform_open0())) / rate;
            self.streams.insert(code.site, rng);
            if !d.is_finite() {
                return Err(format!(
                    "the rate {rate} gives a delay too long to be a time"
                ));
            }
            Some(d)
        } else {
            None
        };
        let reads = reads
            .iter()
            .map(|&v| {
                let value = self
                    .values
                    .get(v as usize)
                    .cloned()
                    .unwrap_or(Value::Text(String::new()));
                (v, value)
            })
            .collect();
        let watch = RateWatch {
            code: code.clone(),
            reads,
        };
        Ok((watch, delay))
    }

    /// Redraws every pending rate send one of whose variables has changed since it was drawn
    /// (the exponential forgets the time already waited, so the event comes at the new rate).
    fn redraw_rates(&mut self, chart: &Chart, w: &dyn World) {
        let changed: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, p)| {
                p.rate.as_ref().is_some_and(|r| {
                    r.reads.iter().any(|(v, old)| {
                        self.values
                            .get(*v as usize)
                            .is_some_and(|now| !same(now, old))
                    })
                })
            })
            .map(|(&t, _)| t)
            .collect();
        for token in changed {
            let Some(mut p) = self.pending.remove(&token) else {
                continue;
            };
            let Some(old) = p.rate.take() else {
                continue;
            };
            let reads: Vec<u32> = old.reads.iter().map(|(v, _)| *v).collect();
            match self.draw_rate(chart, &old.code, &reads, w) {
                Ok((watch, delay)) => {
                    p.rate = Some(watch);
                    self.add_pending(p, delay);
                }
                Err(message) => {
                    self.record(Step::Error(message));
                    self.internal.push_back("error.execution".to_string());
                }
            }
        }
    }

    fn take_stream(&mut self, chart: &Chart, site: u32) -> Rng {
        self.streams.remove(&site).unwrap_or_else(|| {
            let purpose = chart.sites.get(site as usize).map_or("", String::as_str);
            Rng::keyed(self.seed, &stream_key(&self.key, purpose))
        })
    }

    /// Evaluates an expression, its draws from its own stream.
    fn eval(&mut self, chart: &Chart, code: &Code, w: &dyn World) -> Value {
        let cell = RefCell::new(self.take_stream(chart, code.site));
        let value = {
            let property = |name: &str| self.lookup(chart, name);
            let provided =
                |name: &str, args: &[Value]| self.provided(chart, name, args, Some(&cell), w);
            code.expr.eval_with(&property, &provided)
        };
        self.streams.insert(code.site, cell.into_inner());
        value
    }

    fn lookup(&self, chart: &Chart, name: &str) -> Option<Value> {
        if name == data::EVENT_NAME {
            return Some(Value::Text(self.event.clone()));
        }
        if name == data::EVENT_DATA {
            return self.event_data.map(Value::Number);
        }
        let i = *chart.var_index.get(name)?;
        self.values.get(i as usize).cloned()
    }

    fn provided(
        &self,
        chart: &Chart,
        name: &str,
        args: &[Value],
        rng: Option<&RefCell<Rng>>,
        w: &dyn World,
    ) -> Value {
        match name {
            "In" => {
                let active = args
                    .first()
                    .and_then(|a| chart.ids.get(&a.text()))
                    .is_some_and(|&i| {
                        !matches!(chart.node(i).kind, NodeKind::History(_))
                            && self.active[i as usize]
                    });
                Value::Boolean(active)
            }
            "time" => Value::Number(self.now),
            "serial" => w.call(name, args, self),
            _ if data::POPULATION_FUNCTIONS.contains(&name) => w.call(name, args, self),
            _ => rng
                .and_then(|rng| data::draw(name, args, rng))
                .unwrap_or(Value::Text(String::new())),
        }
    }
}

/// The key of an agent's stream: the agent key's length in bytes, `:`, the agent key, `/`, the
/// purpose, so no two pairs of agent key and purpose give the same key.
fn stream_key(agent: &str, purpose: &str) -> String {
    format!("{}:{agent}/{purpose}", agent.len())
}

/// Equal values, NaN equal to NaN (a variable that stays NaN has not changed).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}
