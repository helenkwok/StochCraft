//! System dynamics (ROADMAP M4.2): stocks and flows over time.
//!
//! A model is read from an XMILE 1.0 file ([`read_xmile`]) or built from [`Variable`]s, made
//! ready with [`compile`] (which reads every equation, orders the variables, and checks for
//! algebraic loops and units) and run with [`run`]:
//!
//! ```
//! use stochcraft_sd::{RunOptions, SdModel, Variable, compile, run};
//!
//! let mut model = SdModel::default();
//! model.specs.stop = 10.0;
//! model.specs.dt = stochcraft_sd::Step::Reciprocal(64.0);
//! model.vars = vec![
//!     Variable::stock("population", "100", &["growth"], &[]),
//!     Variable::flow("growth", "0.1 * population"),
//! ];
//! let program = compile(&model)?;
//! let results = run(&program, &RunOptions::default())?;
//! let last = results.series("population").and_then(|s| s.mean.last().copied());
//! // 100 e^(0.1 * 10) = 271.8; Euler at 1/64 is a little under it.
//! assert!(last.is_some_and(|v| (v - 271.83).abs() < 1.0));
//! # Ok::<(), stochcraft_sd::SdError>(())
//! ```
//!
//! **Numerics.** Euler (the default) or classical fourth-order Runge-Kutta, as the file asks.
//! Time is `start + n * DT` from an integer counter. Variables are worked out in topological
//! order, ties broken by name; an algebraic loop is an error naming the cycle. A stock that
//! cannot go negative limits its outflows to what it holds (and a flow that runs backwards into it,
//! to what it holds too), so material is conserved. Delays
//! and smooths are hidden stocks; a graphical function is clamped at its ends, with a warning.
//! See [`run`](mod@run) for the rest.
//!
//! **Replications.** A model with no random function runs once. One with `NORMAL` or `RANDOM`
//! (`UNIFORM`) runs in replications, each drawing from streams keyed by variable name under the
//! seed, and its results give the mean, the median and the 5% to 95% band at each saved time.
//!
//! **Diagram.** A model keeps where each variable is drawn ([`Layout`]): read from and written to
//! the file's `views`, and made by [`Layout::auto`] for a model that has none. [`rename_variable`]
//! and [`remove_variable`] change a model beyond one variable, keeping the equations, flow lists
//! and diagram consistent.
//!
//! **Units** are checked on both sides of each equation (compound units and the plain number
//! `1` included) and a mismatch is a warning, not an error.

mod edit;
mod expr;
mod layout;
mod model;
mod program;
pub mod run;
mod units;
mod xmile;
mod xml;

pub use edit::{Users, find_variable, remove_variable, rename_variable, users_of};
pub use layout::{AUX_SIZE, Connector, Layout, STOCK_SIZE, Shape, VALVE_SIZE, flow_ends};
pub use model::{Gf, GfKind, GfX, Method, SdModel, SimSpecs, Step, VarKind, Variable, canonical};
pub use program::{Program, compile};
pub use run::{
    DEFAULT_REPLICATIONS, DT_CHECK_THRESHOLD, DtCheck, RunOptions, SdJob, SdResults, Series,
    StockCheck, run,
};
pub use units::{Units, time_unit};
pub use xmile::{
    EXTENSION_NAMESPACE, NAMESPACE, XmileFile, looks_like_xmile, model_text, read_xmile,
    write_xmile,
};

/// What kind of thing is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The file is not well-formed XMILE.
    Xml,
    /// The file uses something this version does not support; the message names it.
    Unsupported,
    /// An equation cannot be read or names something that is not there.
    Equation,
    /// An algebraic loop; the message names the cycle.
    Loop,
    /// The model is not complete or consistent.
    Model,
    /// A run cannot be made (too large, or a setting out of range).
    Run,
}

impl ErrorKind {
    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::Xml => "xml",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Equation => "equation",
            ErrorKind::Loop => "algebraic_loop",
            ErrorKind::Model => "model",
            ErrorKind::Run => "run",
        }
    }
}

/// Why a file cannot be read, a model cannot be made ready, or a run cannot be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdError {
    pub kind: ErrorKind,
    pub message: String,
}

impl std::fmt::Display for SdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SdError {}
