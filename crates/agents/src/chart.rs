//! A state chart as it is written ([`ChartSpec`]) and as it is run ([`Chart`]).
//!
//! A [`ChartSpec`] is the tree of an SCXML document: states, parallel states, final states and
//! history pseudo-states, each with its entry and exit actions and its transitions, plus the
//! chart's variables. [`Chart::new`] checks it and compiles it: every state gets its number in
//! document order (the `<scxml>` element is 0, then the states as their start tags appear), every
//! target is resolved, and every expression is parsed once.

use std::collections::BTreeMap;
use std::fmt;

use stochcraft_model::expr::{self, Expr};

use crate::data;

/// What kind of state a [`StateSpec`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `<state>`: atomic without children, compound with them.
    State,
    /// `<parallel>`: all its children are active together.
    Parallel,
    /// `<final>`.
    Final,
    /// `<history>`: a pseudo-state that remembers its parent's last active children.
    History(History),
}

/// What a history state remembers (SCXML §3.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum History {
    /// The parent's active children.
    Shallow,
    /// The parent's active atomic descendants.
    Deep,
}

/// One state of a chart, with its children.
#[derive(Debug, Clone, PartialEq)]
pub struct StateSpec {
    pub id: String,
    pub kind: Kind,
    /// The `initial` attribute of a compound state: the states entered by default. Empty: its
    /// first child state in document order.
    pub initial: Vec<String>,
    pub on_entry: Vec<Action>,
    pub on_exit: Vec<Action>,
    /// In document order. A history state has exactly one, eventless and unconditional: its
    /// default, taken when it has no history yet.
    pub transitions: Vec<TransitionSpec>,
    pub children: Vec<StateSpec>,
}

impl StateSpec {
    fn new(id: &str, kind: Kind) -> Self {
        Self {
            id: id.to_string(),
            kind,
            initial: Vec::new(),
            on_entry: Vec::new(),
            on_exit: Vec::new(),
            transitions: Vec::new(),
            children: Vec::new(),
        }
    }

    /// A `<state>` (atomic until it is given children).
    pub fn state(id: &str) -> Self {
        Self::new(id, Kind::State)
    }

    /// A `<parallel>` state.
    pub fn parallel(id: &str) -> Self {
        Self::new(id, Kind::Parallel)
    }

    /// A `<final>` state.
    pub fn final_state(id: &str) -> Self {
        Self::new(id, Kind::Final)
    }

    /// A `<history>` pseudo-state with its default transition.
    pub fn history(id: &str, depth: History, default: TransitionSpec) -> Self {
        let mut s = Self::new(id, Kind::History(depth));
        s.transitions.push(default);
        s
    }

    /// Adds a child (after the ones before, in document order).
    pub fn child(mut self, child: StateSpec) -> Self {
        self.children.push(child);
        self
    }

    /// Sets the `initial` attribute.
    pub fn initial(mut self, ids: &[&str]) -> Self {
        self.initial = ids.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Adds an entry action.
    pub fn on_entry(mut self, action: impl Into<Action>) -> Self {
        self.on_entry.push(action.into());
        self
    }

    /// Adds an exit action.
    pub fn on_exit(mut self, action: impl Into<Action>) -> Self {
        self.on_exit.push(action.into());
        self
    }

    /// Adds a transition (after the ones before, in document order).
    pub fn transition(mut self, t: TransitionSpec) -> Self {
        self.transitions.push(t);
        self
    }
}

/// A transition (SCXML §3.5).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TransitionSpec {
    /// The `event` attribute: event descriptors separated by spaces (§3.12.1). `None`: an
    /// eventless transition, taken as soon as its condition holds.
    pub event: Option<String>,
    /// The `cond` attribute: an expression, true if absent.
    pub cond: Option<String>,
    /// The targets. None: a targetless transition, which runs its actions and leaves the
    /// configuration as it is.
    pub targets: Vec<String>,
    /// `type="internal"`: from a compound state to its own descendants, it does not exit and
    /// re-enter the source (§3.13). Otherwise it is external.
    pub internal: bool,
    /// The transition's executable content.
    pub actions: Vec<Action>,
}

impl TransitionSpec {
    /// A transition on `event` (one or more descriptors separated by spaces).
    pub fn on(event: &str) -> Self {
        Self {
            event: Some(event.to_string()),
            ..Self::default()
        }
    }

    /// An eventless transition.
    pub fn eventless() -> Self {
        Self::default()
    }

    /// Sets the condition.
    pub fn when(mut self, cond: &str) -> Self {
        self.cond = Some(cond.to_string());
        self
    }

    /// Adds a target.
    pub fn to(mut self, target: &str) -> Self {
        self.targets.push(target.to_string());
        self
    }

    /// Makes it an internal transition.
    pub fn internal(mut self) -> Self {
        self.internal = true;
        self
    }

    /// Adds an action to its content.
    pub fn then(mut self, action: impl Into<Action>) -> Self {
        self.actions.push(action.into());
        self
    }
}

/// Executable content (SCXML §4 and §6.2).
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// `<assign location expr>`: the variable `location` (written `x` or `$x`) takes the value
    /// of `expr`.
    Assign { location: String, expr: String },
    /// `<raise event>`: an event on the internal queue.
    Raise { event: String },
    /// `<send>`.
    SendSpec(SendSpec),
    /// `<cancel sendid>`: the pending sends with this id never arrive.
    Cancel { send_id: String },
    /// `<log label expr>`: recorded in the agent's trace.
    Log { label: String, expr: Option<String> },
    /// Creates one agent of `population` (in a [`Model`](crate::Model)): an event at the
    /// current time, after the current one, starts it.
    Create { population: String },
    /// Removes the agent of `population` whose serial number `serial` gives (in a
    /// [`Model`](crate::Model)): an event at the current time, after the current one, ends its
    /// chart (its active states are exited, their exit actions run) and takes it out.
    Remove { population: String, serial: String },
    /// Links the agent, which must be of `population`, to the agent of it whose serial number
    /// `serial` gives, in the population's [`Network`](crate::Network) (a link to itself, to
    /// an agent that is not alive, or one it has already does nothing). It takes effect when
    /// the macrostep ends, in the order the actions ran.
    Connect { population: String, serial: String },
    /// Removes the agent's link to that agent, as [`Action::Connect`] adds it.
    Disconnect { population: String, serial: String },
    /// Moves the agent, which must be of `population`, to the cell at `row` and `column` (two
    /// expressions) of the population's [`Grid`](crate::Grid), wrapped if the grid wraps. If
    /// there is no such cell, or it holds another agent and the grid has one per cell, the agent
    /// stays. It takes effect when the macrostep ends.
    MoveTo {
        population: String,
        row: String,
        column: String,
    },
    /// Moves the agent to one of its neighbouring cells that can take it, picked at random from a
    /// stream of its own (keyed by the action's place in the chart and `pick`); it stays if
    /// there is none. It takes effect when the macrostep ends.
    MoveRandom { population: String },
}

impl Action {
    pub fn assign(location: &str, expr: &str) -> Self {
        Action::Assign {
            location: location.to_string(),
            expr: expr.to_string(),
        }
    }

    pub fn raise(event: &str) -> Self {
        Action::Raise {
            event: event.to_string(),
        }
    }

    pub fn cancel(send_id: &str) -> Self {
        Action::Cancel {
            send_id: send_id.to_string(),
        }
    }

    pub fn log(label: &str) -> Self {
        Action::Log {
            label: label.to_string(),
            expr: None,
        }
    }

    pub fn log_value(label: &str, expr: &str) -> Self {
        Action::Log {
            label: label.to_string(),
            expr: Some(expr.to_string()),
        }
    }

    pub fn create(population: &str) -> Self {
        Action::Create {
            population: population.to_string(),
        }
    }

    pub fn remove(population: &str, serial: &str) -> Self {
        Action::Remove {
            population: population.to_string(),
            serial: serial.to_string(),
        }
    }

    pub fn connect(population: &str, serial: &str) -> Self {
        Action::Connect {
            population: population.to_string(),
            serial: serial.to_string(),
        }
    }

    pub fn disconnect(population: &str, serial: &str) -> Self {
        Action::Disconnect {
            population: population.to_string(),
            serial: serial.to_string(),
        }
    }

    pub fn move_to(population: &str, row: &str, column: &str) -> Self {
        Action::MoveTo {
            population: population.to_string(),
            row: row.to_string(),
            column: column.to_string(),
        }
    }

    pub fn move_random(population: &str) -> Self {
        Action::MoveRandom {
            population: population.to_string(),
        }
    }
}

/// Which other agents a [`SendSpec`] goes to (in a [`Model`](crate::Model)). Each recipient gets
/// the message as an event of its own at the current time, after the current event (ROADMAP
/// D18); a message to no agent (none of that serial, or no other agent to pick) is counted as
/// undelivered, and one that no transition of its recipient takes as dropped
/// ([`Counts`](crate::Counts)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum To {
    /// The agent of `population` whose serial number the expression `serial` gives.
    Agent { population: String, serial: String },
    /// An agent of `population` picked at random, never the sender, from a stream of the
    /// sender's own (keyed by the send's place in the chart and `pick`).
    Random { population: String },
    /// Every agent of `population` but the sender, in serial order.
    All { population: String },
    /// Every agent linked to the sender in `population`'s network, in serial order (the
    /// sender must be of `population`).
    Linked { population: String },
    /// One agent linked to the sender, picked at random as for [`To::Random`].
    LinkedRandom { population: String },
    /// Every agent in the sender's neighbourhood on `population`'s grid (the agents in the
    /// neighbouring cells and the others in its own cell), in serial order.
    Neighbours { population: String },
    /// One agent of the sender's neighbourhood, picked at random as for [`To::Random`].
    NeighbourRandom { population: String },
}

impl To {
    pub fn population(&self) -> &str {
        match self {
            To::Agent { population, .. }
            | To::Random { population }
            | To::All { population }
            | To::Linked { population }
            | To::LinkedRandom { population }
            | To::Neighbours { population }
            | To::NeighbourRandom { population } => population,
        }
    }
}

/// Where a [`SendSpec`] goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Target {
    /// The agent's own external queue (SCXML's default target): an event on the discrete-event
    /// clock, after the delay, handled by a macrostep of its own.
    #[default]
    External,
    /// `#_internal`: the agent's internal queue, at once, within the current macrostep.
    Internal,
}

/// When a [`SendSpec`] arrives.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Delay {
    /// At once: for an external send, an event on the clock at the current time, after the
    /// events already due then.
    #[default]
    None,
    /// The `delay` attribute, in the model's time unit (SCXML writes a CSS2 time; a file
    /// reader converts it).
    Fixed(f64),
    /// `delayexpr`: an expression evaluated when the send runs, which may draw from a
    /// distribution (`exponential(2)`, `uniform(1, 3)`, ...).
    Expr(String),
    /// A rate (an extension, `stochcraft:rate`): the delay is exponential with mean 1 / rate,
    /// drawn again whenever a variable the rate expression reads has changed at the end of a
    /// macrostep, so the event comes at the current rate. A rate of 0 waits until the rate
    /// becomes positive.
    Rate(String),
}

/// `<send>`: an event to the agent itself, at once or later, or a message to other agents.
#[derive(Debug, Clone, PartialEq)]
pub struct SendSpec {
    pub event: String,
    /// The `id` that a [`Action::Cancel`] names.
    pub id: Option<String>,
    pub target: Target,
    pub delay: Delay,
    /// Other agents to send it to; `None`: the agent itself. A message to others is delivered
    /// at once: it takes no delay, id or internal target.
    pub to: Option<To>,
    /// An expression for the number it carries, read as `getProperty('_event.data')`.
    pub data: Option<String>,
}

impl SendSpec {
    /// An external send of `event` with no delay.
    pub fn new(event: &str) -> Self {
        Self {
            event: event.to_string(),
            id: None,
            target: Target::External,
            delay: Delay::None,
            to: None,
            data: None,
        }
    }

    /// To the agent of `population` whose serial number `serial` (an expression) gives.
    pub fn to_agent(mut self, population: &str, serial: &str) -> Self {
        self.to = Some(To::Agent {
            population: population.to_string(),
            serial: serial.to_string(),
        });
        self
    }

    /// To an agent of `population` picked at random, never the sender.
    pub fn to_random(mut self, population: &str) -> Self {
        self.to = Some(To::Random {
            population: population.to_string(),
        });
        self
    }

    /// To every agent of `population` but the sender.
    pub fn to_all(mut self, population: &str) -> Self {
        self.to = Some(To::All {
            population: population.to_string(),
        });
        self
    }

    /// To every agent linked to the sender in `population`'s network.
    pub fn to_linked(mut self, population: &str) -> Self {
        self.to = Some(To::Linked {
            population: population.to_string(),
        });
        self
    }

    /// To one agent linked to the sender, picked at random.
    pub fn to_linked_random(mut self, population: &str) -> Self {
        self.to = Some(To::LinkedRandom {
            population: population.to_string(),
        });
        self
    }

    /// To every agent of the sender's neighbourhood on `population`'s grid.
    pub fn to_neighbours(mut self, population: &str) -> Self {
        self.to = Some(To::Neighbours {
            population: population.to_string(),
        });
        self
    }

    /// To one agent of the sender's neighbourhood, picked at random.
    pub fn to_neighbour_random(mut self, population: &str) -> Self {
        self.to = Some(To::NeighbourRandom {
            population: population.to_string(),
        });
        self
    }

    /// Carrying the number `expr` gives.
    pub fn with_data(mut self, expr: &str) -> Self {
        self.data = Some(expr.to_string());
        self
    }

    pub fn id(mut self, id: &str) -> Self {
        self.id = Some(id.to_string());
        self
    }

    /// To the internal queue (`target="#_internal"`); it takes no delay.
    pub fn internal(mut self) -> Self {
        self.target = Target::Internal;
        self
    }

    /// After a fixed delay.
    pub fn after(mut self, delay: f64) -> Self {
        self.delay = Delay::Fixed(delay);
        self
    }

    /// After the delay an expression gives (`delayexpr`).
    pub fn after_expr(mut self, expr: &str) -> Self {
        self.delay = Delay::Expr(expr.to_string());
        self
    }

    /// At an exponential time with the rate an expression gives, redrawn when the rate's
    /// variables change.
    pub fn at_rate(mut self, expr: &str) -> Self {
        self.delay = Delay::Rate(expr.to_string());
        self
    }
}

impl From<SendSpec> for Action {
    fn from(s: SendSpec) -> Self {
        Action::SendSpec(s)
    }
}

/// A variable of the chart's data model, with the expression of its first value (evaluated at
/// the start, in declaration order: SCXML's early binding).
#[derive(Debug, Clone, PartialEq)]
pub struct Data {
    pub id: String,
    pub expr: String,
}

/// A whole chart: the `<scxml>` element.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartSpec {
    pub name: String,
    /// The states entered at the start. Empty: the first top-level state.
    pub initial: Vec<String>,
    pub data: Vec<Data>,
    pub states: Vec<StateSpec>,
}

impl ChartSpec {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }

    pub fn initial(mut self, ids: &[&str]) -> Self {
        self.initial = ids.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Declares a variable with the expression of its first value.
    pub fn data(mut self, id: &str, expr: &str) -> Self {
        self.data.push(Data {
            id: id.to_string(),
            expr: expr.to_string(),
        });
        self
    }

    /// Adds a top-level state.
    pub fn state(mut self, s: StateSpec) -> Self {
        self.states.push(s);
        self
    }

    /// Checks and compiles the chart ([`Chart::new`]).
    pub fn build(&self) -> Result<Chart, ChartError> {
        Chart::new(self)
    }
}

/// Why a chart cannot be run: where, and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartError {
    /// The element, such as "state `idle`" or "transition 2 of state `idle`".
    pub place: String,
    pub message: String,
}

impl fmt::Display for ChartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.place, self.message)
    }
}

impl std::error::Error for ChartError {}

fn fail<T>(place: &str, message: impl Into<String>) -> Result<T, ChartError> {
    Err(ChartError {
        place: place.to_string(),
        message: message.into(),
    })
}

/// A compiled node: the `<scxml>` element (0) or a state, numbered in document order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Root,
    Atomic,
    Compound,
    Parallel,
    Final,
    History(History),
}

#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
    pub initial: Vec<u32>,
    pub on_entry: Vec<Exec>,
    pub on_exit: Vec<Exec>,
    pub transitions: Vec<u32>,
    pub histories: Vec<u32>,
}

/// A parsed expression and the random-stream purpose its draws come from.
#[derive(Debug, Clone)]
pub(crate) struct Code {
    pub expr: Expr,
    pub site: u32,
    /// The populations it reads through the population functions, sorted, without repeats.
    pub populations: Vec<String>,
    /// Whether it calls `time()`.
    pub reads_time: bool,
}

/// A name in a chart that belongs to a population, checked when a model is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PopRef {
    pub population: String,
    pub what: RefWhat,
    /// Where it is, as [`ChartError::place`] gives it.
    pub place: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RefWhat {
    Population,
    State(String),
    Variable(String),
    /// The population has a network.
    Network,
    /// The population has a grid.
    Grid,
}

#[derive(Debug, Clone)]
pub(crate) struct Transition {
    pub source: u32,
    /// Normalised descriptors (§3.12.1); `None` for an eventless transition.
    pub events: Option<Vec<String>>,
    pub cond: Option<Code>,
    pub targets: Vec<u32>,
    pub internal: bool,
    pub actions: Vec<Exec>,
}

#[derive(Debug, Clone)]
pub(crate) enum Exec {
    Assign {
        var: u32,
        code: Code,
    },
    Raise(String),
    SendSpec(Box<SendExec>),
    Cancel(String),
    Log {
        label: String,
        code: Option<Code>,
    },
    Create(String),
    Remove {
        population: String,
        serial: Code,
    },
    Connect {
        population: String,
        serial: Code,
    },
    Disconnect {
        population: String,
        serial: Code,
    },
    MoveTo {
        population: String,
        row: Code,
        column: Code,
    },
    /// The stream (site) the pick draws from.
    MoveRandom {
        population: String,
        site: u32,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct SendExec {
    pub event: String,
    pub id: Option<String>,
    pub internal: bool,
    pub delay: DelayExec,
    pub to: Option<ToExec>,
    pub data: Option<Code>,
}

#[derive(Debug, Clone)]
pub(crate) enum ToExec {
    Agent {
        population: String,
        serial: Code,
    },
    /// The stream (site) the pick draws from.
    Random {
        population: String,
        site: u32,
    },
    All {
        population: String,
    },
    /// The agents linked to the sender; with a site, one of them picked from that stream.
    Linked {
        population: String,
        site: Option<u32>,
    },
    /// The sender's neighbours on the grid; with a site, one of them picked from that stream.
    Neighbours {
        population: String,
        site: Option<u32>,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum DelayExec {
    None,
    Fixed(f64),
    Expr(Code),
    /// The rate, and the variables it reads.
    Rate {
        code: Code,
        reads: Vec<u32>,
    },
}

/// A checked, compiled state chart, ready to run as any number of agents
/// ([`Agent`](crate::Agent)).
#[derive(Debug, Clone)]
pub struct Chart {
    pub(crate) name: String,
    pub(crate) nodes: Vec<Node>,
    pub(crate) transitions: Vec<Transition>,
    /// The `<scxml>` element's initial transition (an index into `transitions`).
    pub(crate) initial: u32,
    pub(crate) vars: Vec<(String, Code)>,
    pub(crate) var_index: BTreeMap<String, u32>,
    /// The purpose of each random stream, such as `state/4:idle/onentry/1`.
    pub(crate) sites: Vec<String>,
    pub(crate) ids: BTreeMap<String, u32>,
    /// The populations the conditions of its eventless transitions read: when one of them
    /// changes, an agent running this chart is tested again (ROADMAP M4.3).
    pub(crate) waits_on: Vec<String>,
    /// Every population, and population state or variable, the chart names.
    pub(crate) refs: Vec<PopRef>,
}

impl Chart {
    /// Checks `spec` and compiles it. Every state id must be unique and every target and
    /// initial state name a state; states nest at most [`MAX_DEPTH`] deep; the targets of one
    /// transition, and a compound state's or the chart's initial states, must be able to be
    /// active together (no two in one compound state or the chart itself, none inside another:
    /// only different regions of a parallel state); history states sit in a compound or
    /// parallel state with a default transition to its children (shallow) or descendants
    /// (deep), never to a history state; final states have no children or transitions; parallel
    /// states have at least one child; every expression must parse and read only declared
    /// variables (or `getProperty('_event.name')` and `getProperty('_event.data')`, the current
    /// event's name and data); population functions name their population, state and variable
    /// in quotes (checked by a [`Model`](crate::Model)); a rate reads no population and not
    /// `time()`; a message to other agents has no delay; a fixed delay is
    /// a time of 0 or more; an internal send has no delay.
    ///
    /// `In('id')` with a quoted name is checked to name a state. A name the expression computes
    /// (`In(concat('a', 'b'))`) is not checked: one that names no state reads as false.
    pub fn new(spec: &ChartSpec) -> Result<Chart, ChartError> {
        if spec.states.is_empty() {
            return fail(&format!("chart `{}`", spec.name), "it has no states");
        }
        // Number the states in document order, and keep each one's spec.
        let mut nodes = vec![Node {
            id: String::new(),
            kind: NodeKind::Root,
            parent: None,
            children: Vec::new(),
            initial: Vec::new(),
            on_entry: Vec::new(),
            on_exit: Vec::new(),
            transitions: Vec::new(),
            histories: Vec::new(),
        }];
        let mut specs: Vec<Option<&StateSpec>> = vec![None];
        let mut ids = BTreeMap::new();
        for s in &spec.states {
            number(s, 0, 1, &mut nodes, &mut specs, &mut ids)?;
        }
        let mut c = Compiler {
            ids: &ids,
            var_index: BTreeMap::new(),
            sites: Vec::new(),
            refs: Vec::new(),
        };
        // Variables, in declaration order.
        let mut vars = Vec::new();
        for (i, d) in spec.data.iter().enumerate() {
            let place = format!("data `{}`", d.id);
            if !is_name(&d.id) {
                return fail(
                    &place,
                    "a variable's name is letters, digits and `_`, starting with a letter or `_`",
                );
            }
            if c.var_index.insert(d.id.clone(), i as u32).is_some() {
                return fail(&place, "a variable of this name is declared already");
            }
            vars.push(d);
        }
        let mut compiled_vars = Vec::new();
        for d in vars {
            let place = format!("data `{}`", d.id);
            let code = c.code(&d.expr, &place, &format!("data/{}", d.id))?;
            compiled_vars.push((d.id.clone(), code));
        }
        // States: kinds checked, actions and transitions compiled.
        let mut transitions = Vec::new();
        for (n, s) in specs.iter().enumerate() {
            let Some(s) = *s else { continue };
            let place = format!("state `{}`", s.id);
            // The id's length first, so no two states' purposes are the same text.
            let prefix = format!("state/{}:{}", s.id.len(), s.id);
            let parent = nodes[n].parent.unwrap_or(0);
            match s.kind {
                Kind::Final if !s.children.is_empty() || !s.transitions.is_empty() => {
                    return fail(&place, "a final state has no children and no transitions");
                }
                Kind::Parallel if !s.initial.is_empty() => {
                    return fail(&place, "a parallel state has no initial attribute");
                }
                Kind::History(_) => {
                    if !s.children.is_empty() || !s.on_entry.is_empty() || !s.on_exit.is_empty() {
                        return fail(&place, "a history state has no children or actions");
                    }
                    if parent == 0 {
                        return fail(&place, "a history state belongs in a state with children");
                    }
                    match s.transitions.as_slice() {
                        [t] if t.event.is_none() && t.cond.is_none() && !t.targets.is_empty() => {}
                        _ => {
                            return fail(
                                &place,
                                "a history state has exactly one transition, its default: no \
                                 event, no condition, at least one target",
                            );
                        }
                    }
                }
                _ => {}
            }
            let child_states: Vec<u32> = nodes[n]
                .children
                .iter()
                .copied()
                .filter(|&k| !matches!(nodes[k as usize].kind, NodeKind::History(_)))
                .collect();
            if matches!(s.kind, Kind::State | Kind::Parallel)
                && !s.children.is_empty()
                && child_states.is_empty()
            {
                return fail(&place, "its only children are history states");
            }
            if s.kind == Kind::Parallel && child_states.is_empty() {
                return fail(&place, "a parallel state needs at least one child state");
            }
            if s.kind == Kind::State && child_states.is_empty() && !s.initial.is_empty() {
                return fail(&place, "an atomic state has no initial attribute");
            }
            // Initial states of a compound state: given, or the first child state.
            let mut initial = Vec::new();
            for id in &s.initial {
                let t = c.state(id, &place)?;
                if !is_descendant(&nodes, t, n as u32) {
                    return fail(&place, format!("its initial state `{id}` is not inside it"));
                }
                initial.push(t);
            }
            check_together(&nodes, &initial, &place)?;
            if s.kind == Kind::State && initial.is_empty() {
                initial.extend(child_states.first().copied());
            }
            nodes[n].initial = initial;
            nodes[n].on_entry = c.block(&s.on_entry, &place, &format!("{prefix}/onentry"))?;
            nodes[n].on_exit = c.block(&s.on_exit, &place, &format!("{prefix}/onexit"))?;
            for (k, t) in s.transitions.iter().enumerate() {
                let tplace = format!("transition {} of state `{}`", k + 1, s.id);
                let tprefix = format!("{prefix}/transition/{}", k + 1);
                let compiled = c.transition(t, n as u32, &tplace, &tprefix)?;
                check_together(&nodes, &compiled.targets, &tplace)?;
                if let Kind::History(depth) = s.kind {
                    for &target in &compiled.targets {
                        // A default to a history state could lead back to itself; to states
                        // only, the default entry always goes deeper and ends.
                        if matches!(nodes[target as usize].kind, NodeKind::History(_)) {
                            return fail(&place, "its default target is a history state");
                        }
                        let ok = match depth {
                            History::Shallow => nodes[target as usize].parent == Some(parent),
                            History::Deep => is_descendant(&nodes, target, parent),
                        };
                        if !ok {
                            return fail(
                                &place,
                                "its default target must be a child (shallow history) or a \
                                 descendant (deep history) of its parent",
                            );
                        }
                    }
                }
                nodes[n].transitions.push(transitions.len() as u32);
                transitions.push(compiled);
            }
        }
        // The `<scxml>` element's initial transition: to the given states, or the first one.
        // Appendix D enters it through `enterStates`, whose domain (`getTransitionDomain`) would
        // look for a proper ancestor of the `<scxml>` element; it is made an internal transition
        // from the root, and the root counts as compound there, so its domain is the root and
        // the entry set is the targets and their ancestors below it.
        let place = format!("chart `{}`", spec.name);
        let mut targets = Vec::new();
        for id in &spec.initial {
            targets.push(c.state(id, &place)?);
        }
        check_together(&nodes, &targets, &place)?;
        if targets.is_empty() {
            targets.extend(nodes[0].children.first().copied());
        }
        let initial = transitions.len() as u32;
        transitions.push(Transition {
            source: 0,
            events: None,
            cond: None,
            targets,
            internal: true,
            actions: Vec::new(),
        });
        let mut waits_on: Vec<String> = transitions
            .iter()
            .filter(|t| t.events.is_none())
            .filter_map(|t| t.cond.as_ref())
            .flat_map(|c| c.populations.iter().cloned())
            .collect();
        waits_on.sort();
        waits_on.dedup();
        let sites = c.sites;
        let var_index = c.var_index;
        let refs = c.refs;
        Ok(Chart {
            waits_on,
            refs,
            name: spec.name.clone(),
            nodes,
            transitions,
            initial,
            vars: compiled_vars,
            var_index,
            sites,
            ids,
        })
    }

    /// Whether any `<assign>` of the chart writes the variable `name`.
    pub(crate) fn assigns(&self, name: &str) -> bool {
        let Some(&v) = self.var_index.get(name) else {
            return false;
        };
        let hit = |b: &[Exec]| {
            b.iter()
                .any(|e| matches!(e, Exec::Assign { var, .. } if *var == v))
        };
        self.nodes
            .iter()
            .any(|n| hit(&n.on_entry) || hit(&n.on_exit))
            || self.transitions.iter().any(|t| hit(&t.actions))
    }

    /// The chart's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Every state's id, in document order (history states included).
    pub fn state_ids(&self) -> impl Iterator<Item = &str> {
        self.nodes.iter().skip(1).map(|n| n.id.as_str())
    }

    /// The variables, in declaration order.
    pub fn variables(&self) -> impl Iterator<Item = &str> {
        self.vars.iter().map(|(name, _)| name.as_str())
    }

    /// The purposes of the chart's random streams, in the order they were compiled: each
    /// expression draws from its own stream, keyed by where it is in the chart.
    pub fn stream_purposes(&self) -> &[String] {
        &self.sites
    }

    /// The populations whose changes make an agent of this chart test its eventless
    /// transitions again: those their conditions read through the population functions.
    pub fn waits_on(&self) -> &[String] {
        &self.waits_on
    }

    /// Compiles an expression over one agent of this chart, for a statistic: it reads the
    /// chart's variables, `In()`, `serial()`, `time()` and the agent's neighbourhood
    /// ([`data::NEIGHBOURHOOD_FUNCTIONS`]), and draws nothing and reads no whole population.
    /// Returns it with the population names it uses, for the model to check.
    pub(crate) fn agent_expression(
        &self,
        src: &str,
        place: &str,
    ) -> Result<(Code, Vec<PopRef>), ChartError> {
        let mut c = Compiler {
            ids: &self.ids,
            var_index: self.var_index.clone(),
            sites: Vec::new(),
            refs: Vec::new(),
        };
        let code = c.code(src, place, "")?;
        let mut bad = None;
        data::walk(&code.expr, &mut |x| {
            if let Expr::Provided(f, _) = x
                && (data::is_draw(f)
                    || (data::POPULATION_FUNCTIONS.contains(&f.as_str())
                        && !data::NEIGHBOURHOOD_FUNCTIONS.contains(&f.as_str())))
                && bad.is_none()
            {
                bad = Some(f.clone());
            }
        });
        if let Some(f) = bad {
            return fail(
                place,
                format!(
                    "`{src}` calls {f}(), but a statistic reads only the agent itself and its \
                     links and neighbours"
                ),
            );
        }
        if code.expr.references().contains(&data::EVENT_NAME)
            || code.expr.references().contains(&data::EVENT_DATA)
        {
            return fail(
                place,
                format!("`{src}` reads the event, but a statistic has none"),
            );
        }
        Ok((
            Code {
                site: u32::MAX,
                ..code
            },
            c.refs,
        ))
    }

    pub(crate) fn node(&self, i: u32) -> &Node {
        &self.nodes[i as usize]
    }

    pub(crate) fn transition(&self, i: u32) -> &Transition {
        &self.transitions[i as usize]
    }

    pub(crate) fn len(&self) -> u32 {
        self.nodes.len() as u32
    }

    /// Atomic: no child states (final states included).
    pub(crate) fn is_atomic(&self, i: u32) -> bool {
        matches!(self.node(i).kind, NodeKind::Atomic | NodeKind::Final)
    }

    pub(crate) fn is_compound_or_root(&self, i: u32) -> bool {
        matches!(self.node(i).kind, NodeKind::Compound | NodeKind::Root)
    }

    /// Whether `a` is a proper descendant of `b`.
    pub(crate) fn is_descendant(&self, a: u32, b: u32) -> bool {
        is_descendant(&self.nodes, a, b)
    }

    /// The child states of `i` (history pseudo-states left out), in document order.
    pub(crate) fn child_states(&self, i: u32) -> impl Iterator<Item = u32> + '_ {
        self.node(i)
            .children
            .iter()
            .copied()
            .filter(|&c| !matches!(self.node(c).kind, NodeKind::History(_)))
    }

    /// The least common compound ancestor of `list` (SCXML's `findLCCA`): the first proper
    /// ancestor of its head that is a compound state or the `<scxml>` element and holds all the
    /// rest.
    pub(crate) fn lcca(&self, list: &[u32]) -> u32 {
        let Some((&head, tail)) = list.split_first() else {
            return 0;
        };
        let mut anc = self.node(head).parent;
        while let Some(a) = anc {
            if self.is_compound_or_root(a) && tail.iter().all(|&s| self.is_descendant(s, a)) {
                return a;
            }
            anc = self.node(a).parent;
        }
        0
    }
}

fn is_descendant(nodes: &[Node], a: u32, b: u32) -> bool {
    let mut p = nodes.get(a as usize).and_then(|n| n.parent);
    while let Some(x) = p {
        if x == b {
            return true;
        }
        p = nodes.get(x as usize).and_then(|n| n.parent);
    }
    false
}

/// The deepest states may nest (a top-level state is at depth 1): compiling and running a
/// chart recurse once per level.
pub const MAX_DEPTH: u32 = 128;

/// Checks that `targets` can be active together (SCXML §3.11): no two the same or one inside
/// the other, and each two meeting first in a parallel state.
fn check_together(nodes: &[Node], targets: &[u32], place: &str) -> Result<(), ChartError> {
    let id = |s: u32| nodes.get(s as usize).map_or("", |n| n.id.as_str());
    for (k, &a) in targets.iter().enumerate() {
        for &b in &targets[k + 1..] {
            // A history state stands for states inside its parent, chosen only when it is
            // entered (stored or by its default), so no other target may lie in that parent: the
            // two could leave a compound state with two active children.
            for (h, other) in [(a, b), (b, a)] {
                let hist = nodes
                    .get(h as usize)
                    .filter(|n| matches!(n.kind, NodeKind::History(_)));
                if let Some(parent) = hist.and_then(|n| n.parent)
                    && is_descendant(nodes, other, parent)
                {
                    return fail(
                        place,
                        format!(
                            "its state `{}` lies inside the parent of its history state `{}`, \
                             which already decides the states entered there",
                            id(other),
                            id(h)
                        ),
                    );
                }
            }
            if a == b || is_descendant(nodes, a, b) || is_descendant(nodes, b, a) {
                return fail(
                    place,
                    format!(
                        "its states `{}` and `{}` are the same or one is inside the other",
                        id(a),
                        id(b)
                    ),
                );
            }
            // The nearest state holding both.
            let mut meet = nodes.get(a as usize).and_then(|n| n.parent);
            while let Some(m) = meet {
                if is_descendant(nodes, b, m) {
                    break;
                }
                meet = nodes.get(m as usize).and_then(|n| n.parent);
            }
            let parallel = meet
                .and_then(|m| nodes.get(m as usize))
                .is_some_and(|n| n.kind == NodeKind::Parallel);
            if !parallel {
                return fail(
                    place,
                    format!(
                        "its states `{}` and `{}` cannot be active together: only states in \
                         different regions of a parallel state can",
                        id(a),
                        id(b)
                    ),
                );
            }
        }
    }
    Ok(())
}

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Numbers `s` (at `depth`) and its descendants in document order.
fn number<'a>(
    s: &'a StateSpec,
    parent: u32,
    depth: u32,
    nodes: &mut Vec<Node>,
    specs: &mut Vec<Option<&'a StateSpec>>,
    ids: &mut BTreeMap<String, u32>,
) -> Result<(), ChartError> {
    let place = format!("state `{}`", s.id);
    if depth > MAX_DEPTH {
        return fail(&place, format!("states nest more than {MAX_DEPTH} deep"));
    }
    if s.id.is_empty() || s.id.chars().any(char::is_whitespace) {
        return fail(&place, "a state's id is not empty and has no spaces");
    }
    let i = nodes.len() as u32;
    if ids.insert(s.id.clone(), i).is_some() {
        return fail(&place, "another state has this id");
    }
    let kind = match s.kind {
        Kind::State if s.children.is_empty() => NodeKind::Atomic,
        Kind::State => NodeKind::Compound,
        Kind::Parallel => NodeKind::Parallel,
        Kind::Final => NodeKind::Final,
        Kind::History(h) => NodeKind::History(h),
    };
    nodes.push(Node {
        id: s.id.clone(),
        kind,
        parent: Some(parent),
        children: Vec::new(),
        initial: Vec::new(),
        on_entry: Vec::new(),
        on_exit: Vec::new(),
        transitions: Vec::new(),
        histories: Vec::new(),
    });
    specs.push(Some(s));
    if let Some(p) = nodes.get_mut(parent as usize) {
        p.children.push(i);
        if matches!(kind, NodeKind::History(_)) {
            p.histories.push(i);
        }
    }
    for c in &s.children {
        number(c, i, depth + 1, nodes, specs, ids)?;
    }
    Ok(())
}

struct Compiler<'a> {
    ids: &'a BTreeMap<String, u32>,
    var_index: BTreeMap<String, u32>,
    sites: Vec<String>,
    refs: Vec<PopRef>,
}

/// The quoted text of argument `i` of a call, if it is a literal.
fn literal(args: &[Expr], i: usize) -> Option<&str> {
    match args.get(i) {
        Some(Expr::Text(s)) => Some(s),
        _ => None,
    }
}

impl Compiler<'_> {
    fn state(&self, id: &str, place: &str) -> Result<u32, ChartError> {
        match self.ids.get(id) {
            Some(&i) => Ok(i),
            None => fail(place, format!("there is no state `{id}`")),
        }
    }

    /// Parses an expression and gives it a random stream of its own.
    fn code(&mut self, src: &str, place: &str, site: &str) -> Result<Code, ChartError> {
        let parsed = expr::parse_with_functions(src, data::FUNCTIONS);
        let e = match parsed {
            Ok(e) => e,
            Err(e) => return fail(place, format!("`{src}`: {e}")),
        };
        for name in e.references() {
            if name != data::EVENT_NAME
                && name != data::EVENT_DATA
                && !self.var_index.contains_key(name)
            {
                return fail(
                    place,
                    format!("`{src}` reads `{name}`, which is not declared"),
                );
            }
        }
        let mut unknown = None;
        data::walk(&e, &mut |x| {
            if let Expr::Provided(f, args) = x
                && f == "In"
                && let [Expr::Text(id)] = args.as_slice()
                && !self.ids.contains_key(id)
                && unknown.is_none()
            {
                unknown = Some(id.clone());
            }
        });
        if let Some(id) = unknown {
            return fail(
                place,
                format!("`{src}` asks In('{id}'), but there is no such state"),
            );
        }
        // The population functions name their population, state and variable as literals, so
        // what each expression reads is known before the run.
        let mut populations = Vec::new();
        let mut refs = Vec::new();
        let mut bad = None;
        let mut reads_time = false;
        data::walk(&e, &mut |x| {
            let Expr::Provided(f, args) = x else { return };
            if f == "time" {
                reads_time = true;
            }
            if !data::POPULATION_FUNCTIONS.contains(&f.as_str()) {
                return;
            }
            let Some(p) = literal(args, 0) else {
                bad.get_or_insert_with(|| format!("{f}() names its population in quotes"));
                return;
            };
            populations.push(p.to_string());
            let state = |i: usize| literal(args, i).map(|s| RefWhat::State(s.to_string()));
            let var = |i: usize| literal(args, i).map(|v| RefWhat::Variable(v.to_string()));
            let what: Vec<Option<RefWhat>> = match f.as_str() {
                "population-size" => vec![Some(RefWhat::Population)],
                "population-count" => vec![state(1)],
                "population-sum" | "population-mean" => vec![var(1)],
                "degree" => vec![Some(RefWhat::Network)],
                "linked-count" => vec![Some(RefWhat::Network), state(1)],
                "linked-sum" => vec![Some(RefWhat::Network), var(1)],
                "neighbours" => vec![Some(RefWhat::Grid)],
                "neighbour-count" => vec![Some(RefWhat::Grid), state(1)],
                "neighbour-sum" => vec![Some(RefWhat::Grid), var(1)],
                _ => vec![var(2)],
            };
            for w in what {
                match w {
                    Some(what) => refs.push(PopRef {
                        population: p.to_string(),
                        what,
                        place: place.to_string(),
                    }),
                    None => {
                        bad.get_or_insert_with(|| {
                            format!("{f}() names its state or variable in quotes")
                        });
                    }
                }
            }
        });
        if let Some(message) = bad {
            return fail(place, format!("`{src}`: {message}"));
        }
        populations.sort();
        populations.dedup();
        self.refs.extend(refs);
        let index = self.sites.len() as u32;
        self.sites.push(site.to_string());
        Ok(Code {
            expr: e,
            site: index,
            populations,
            reads_time,
        })
    }

    fn population(&mut self, population: &str, place: &str) {
        self.needs(population, place, RefWhat::Population);
    }

    fn needs(&mut self, population: &str, place: &str, what: RefWhat) {
        self.refs.push(PopRef {
            population: population.to_string(),
            what,
            place: place.to_string(),
        });
    }

    /// A stream of its own for a random pick at `site`.
    fn pick_site(&mut self, site: &str) -> u32 {
        let index = self.sites.len() as u32;
        self.sites.push(format!("{site}/pick"));
        index
    }

    fn block(
        &mut self,
        actions: &[Action],
        place: &str,
        prefix: &str,
    ) -> Result<Vec<Exec>, ChartError> {
        actions
            .iter()
            .enumerate()
            .map(|(k, a)| self.action(a, place, &format!("{prefix}/{}", k + 1)))
            .collect()
    }

    fn event_name(event: &str, place: &str) -> Result<String, ChartError> {
        if event.is_empty() || event.chars().any(char::is_whitespace) {
            return fail(
                place,
                format!("`{event}` is not an event name (not empty, no spaces)"),
            );
        }
        Ok(event.to_string())
    }

    fn action(&mut self, a: &Action, place: &str, site: &str) -> Result<Exec, ChartError> {
        Ok(match a {
            Action::Assign { location, expr } => {
                let name = location.strip_prefix('$').unwrap_or(location);
                let Some(&var) = self.var_index.get(name) else {
                    return fail(
                        place,
                        format!("it assigns to `{location}`, which is not a declared variable"),
                    );
                };
                Exec::Assign {
                    var,
                    code: self.code(expr, place, site)?,
                }
            }
            Action::Raise { event } => Exec::Raise(Self::event_name(event, place)?),
            Action::Cancel { send_id } => Exec::Cancel(send_id.clone()),
            Action::Log { label, expr } => Exec::Log {
                label: label.clone(),
                code: match expr {
                    Some(e) => Some(self.code(e, place, site)?),
                    None => None,
                },
            },
            Action::Create { population } => {
                self.population(population, place);
                Exec::Create(population.clone())
            }
            Action::Remove { population, serial } => {
                self.population(population, place);
                Exec::Remove {
                    population: population.clone(),
                    serial: self.code(serial, place, site)?,
                }
            }
            Action::Connect { population, serial } | Action::Disconnect { population, serial } => {
                self.population(population, place);
                self.needs(population, place, RefWhat::Network);
                let population = population.clone();
                let serial = self.code(serial, place, site)?;
                if matches!(a, Action::Connect { .. }) {
                    Exec::Connect { population, serial }
                } else {
                    Exec::Disconnect { population, serial }
                }
            }
            Action::MoveTo {
                population,
                row,
                column,
            } => {
                self.population(population, place);
                self.needs(population, place, RefWhat::Grid);
                Exec::MoveTo {
                    population: population.clone(),
                    row: self.code(row, place, &format!("{site}/row"))?,
                    column: self.code(column, place, &format!("{site}/column"))?,
                }
            }
            Action::MoveRandom { population } => {
                self.population(population, place);
                self.needs(population, place, RefWhat::Grid);
                let index = self.sites.len() as u32;
                self.sites.push(format!("{site}/pick"));
                Exec::MoveRandom {
                    population: population.clone(),
                    site: index,
                }
            }
            Action::SendSpec(s) => {
                let event = Self::event_name(&s.event, place)?;
                let data = match &s.data {
                    Some(d) => Some(self.code(d, place, &format!("{site}/data"))?),
                    None => None,
                };
                if let Some(to) = &s.to {
                    if s.target == Target::Internal || s.delay != Delay::None || s.id.is_some() {
                        return fail(
                            place,
                            "a message to other agents is delivered at once: it takes no delay, \
                             id or internal target",
                        );
                    }
                    self.population(to.population(), place);
                    let to = match to {
                        To::Agent { population, serial } => ToExec::Agent {
                            population: population.clone(),
                            serial: self.code(serial, place, &format!("{site}/to"))?,
                        },
                        To::Random { population } => ToExec::Random {
                            population: population.clone(),
                            site: self.pick_site(site),
                        },
                        To::All { population } => ToExec::All {
                            population: population.clone(),
                        },
                        To::Linked { population } | To::LinkedRandom { population } => {
                            self.needs(population, place, RefWhat::Network);
                            ToExec::Linked {
                                population: population.clone(),
                                site: matches!(to, To::LinkedRandom { .. })
                                    .then(|| self.pick_site(site)),
                            }
                        }
                        To::Neighbours { population } | To::NeighbourRandom { population } => {
                            self.needs(population, place, RefWhat::Grid);
                            ToExec::Neighbours {
                                population: population.clone(),
                                site: matches!(to, To::NeighbourRandom { .. })
                                    .then(|| self.pick_site(site)),
                            }
                        }
                    };
                    return Ok(Exec::SendSpec(Box::new(SendExec {
                        event,
                        id: None,
                        internal: false,
                        delay: DelayExec::None,
                        to: Some(to),
                        data,
                    })));
                }
                let internal = s.target == Target::Internal;
                if internal && data.is_some() {
                    return fail(place, "a send to the internal queue carries no data");
                }
                if internal && s.delay != Delay::None {
                    return fail(place, "a send to the internal queue takes no delay");
                }
                let delay = match &s.delay {
                    Delay::None => DelayExec::None,
                    Delay::Fixed(d) if d.is_finite() && *d >= 0.0 => DelayExec::Fixed(*d),
                    Delay::Fixed(d) => {
                        return fail(place, format!("the delay {d} is not a time of 0 or more"));
                    }
                    Delay::Expr(e) => DelayExec::Expr(self.code(e, place, site)?),
                    Delay::Rate(e) => {
                        let code = self.code(e, place, site)?;
                        if !code.populations.is_empty() || code.reads_time {
                            return fail(
                                place,
                                format!(
                                    "the rate `{e}` reads a population or the time, but a rate \
                                     is drawn again only when the agent's own variables change"
                                ),
                            );
                        }
                        let mut reads: Vec<u32> = code
                            .expr
                            .references()
                            .iter()
                            .filter_map(|n| self.var_index.get(*n).copied())
                            .collect();
                        reads.sort_unstable();
                        reads.dedup();
                        DelayExec::Rate { code, reads }
                    }
                };
                Exec::SendSpec(Box::new(SendExec {
                    event,
                    id: s.id.clone(),
                    internal,
                    delay,
                    to: None,
                    data,
                }))
            }
        })
    }

    fn transition(
        &mut self,
        t: &TransitionSpec,
        source: u32,
        place: &str,
        prefix: &str,
    ) -> Result<Transition, ChartError> {
        let events = match &t.event {
            None => None,
            Some(e) => {
                let ds: Vec<String> = e.split_whitespace().map(data::descriptor).collect();
                if ds.is_empty() || ds.iter().any(String::is_empty) {
                    return fail(place, format!("`{e}` is not a list of event descriptors"));
                }
                Some(ds)
            }
        };
        let cond = match &t.cond {
            Some(c) => Some(self.code(c, place, &format!("{prefix}/cond"))?),
            None => None,
        };
        let mut targets = Vec::new();
        for id in &t.targets {
            targets.push(self.state(id, place)?);
        }
        let actions = self.block(&t.actions, place, prefix)?;
        Ok(Transition {
            source,
            events,
            cond,
            targets,
            internal: t.internal,
            actions,
        })
    }
}
