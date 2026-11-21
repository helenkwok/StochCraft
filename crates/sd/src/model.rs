//! The model as a file holds it: variables with their equations as text, a graphical function
//! where there is one, units, and the simulation settings. Nothing here is compiled or
//! checked; [`crate::compile`] does that.

use crate::layout::Layout;

/// How a model is integrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Method {
    /// Euler's method: one derivative per step.
    #[default]
    Euler,
    /// The classical fourth-order Runge-Kutta method: four derivatives per step.
    Rk4,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::Euler => "euler",
            Method::Rk4 => "rk4",
        }
    }
}

/// The time step as the file writes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// `<dt>0.25</dt>`.
    Value(f64),
    /// `<dt reciprocal="true">4</dt>`: a step of 1/4.
    Reciprocal(f64),
}

impl Step {
    /// The step itself.
    pub fn dt(self) -> f64 {
        match self {
            Step::Value(v) => v,
            Step::Reciprocal(r) => 1.0 / r,
        }
    }
}

/// The simulation settings (`<sim_specs>`).
#[derive(Debug, Clone, PartialEq)]
pub struct SimSpecs {
    pub start: f64,
    pub stop: f64,
    pub dt: Step,
    pub method: Method,
    /// The model's time unit, as written (`Years`); empty if the file names none.
    pub time_units: String,
    /// The interval results are saved at, in the model's time unit; by default the step. XMILE
    /// has no such setting, so it is written as a `stochcraft:save_step` extension.
    pub save_step: Option<f64>,
}

impl Default for SimSpecs {
    fn default() -> Self {
        Self {
            start: 0.0,
            stop: 100.0,
            dt: Step::Value(1.0),
            method: Method::Euler,
            time_units: String::new(),
            save_step: None,
        }
    }
}

/// How a graphical function behaves between and beyond its points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GfKind {
    /// Linear between points, the end values beyond them.
    #[default]
    Continuous,
    /// Linear between points, extended beyond them from the last two points at each end.
    Extrapolate,
    /// A step: the value of the point at or below the input.
    Discrete,
}

impl GfKind {
    pub fn name(self) -> &'static str {
        match self {
            GfKind::Continuous => "continuous",
            GfKind::Extrapolate => "extrapolate",
            GfKind::Discrete => "discrete",
        }
    }
}

/// Where a graphical function's points are along x.
#[derive(Debug, Clone, PartialEq)]
pub enum GfX {
    /// Evenly spaced from `min` to `max` (`<xscale>`).
    Scale { min: f64, max: f64 },
    /// Given (`<xpts>`), ascending.
    Points(Vec<f64>),
}

/// A graphical function (a lookup table).
#[derive(Debug, Clone, PartialEq)]
pub struct Gf {
    pub kind: GfKind,
    pub x: GfX,
    pub y: Vec<f64>,
}

impl Gf {
    /// The points (x, y), or why the table is not usable.
    pub fn points(&self) -> Result<Vec<(f64, f64)>, String> {
        if self.y.is_empty() {
            return Err("has no points".to_owned());
        }
        let xs: Vec<f64> = match &self.x {
            GfX::Points(p) => {
                if p.len() != self.y.len() {
                    return Err(format!(
                        "has {} x values and {} y values",
                        p.len(),
                        self.y.len()
                    ));
                }
                p.clone()
            }
            GfX::Scale { min, max } => {
                let n = self.y.len();
                if n == 1 {
                    vec![*min]
                } else {
                    (0..n)
                        .map(|i| min + (max - min) * i as f64 / (n - 1) as f64)
                        .collect()
                }
            }
        };
        if xs.iter().chain(self.y.iter()).any(|v| !v.is_finite()) {
            return Err("has a value that is not a number".to_owned());
        }
        if xs.windows(2).any(|w| w.first() >= w.get(1)) {
            return Err("has x values that do not increase".to_owned());
        }
        Ok(xs.into_iter().zip(self.y.iter().copied()).collect())
    }
}

/// What a variable is.
#[derive(Debug, Clone, PartialEq)]
pub enum VarKind {
    /// A level that accumulates its flows. `eqn` is its initial value.
    Stock {
        /// Names of the flows in, in the file's order.
        inflows: Vec<String>,
        /// Names of the flows out, in the file's order, which is the order they are limited in
        /// when the stock cannot be negative.
        outflows: Vec<String>,
        /// Outflows are limited to what the stock holds.
        non_negative: bool,
    },
    /// A rate of change.
    Flow {
        /// The flow cannot be negative.
        non_negative: bool,
    },
    /// An auxiliary, which includes a constant (an equation that is a number).
    Aux,
}

impl VarKind {
    pub fn name(&self) -> &'static str {
        match self {
            VarKind::Stock { .. } => "stock",
            VarKind::Flow { .. } => "flow",
            VarKind::Aux => "aux",
        }
    }
}

/// One variable.
#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    /// As written in the file (spaces allowed); equations name it with underscores for spaces,
    /// in any case.
    pub name: String,
    pub kind: VarKind,
    /// The equation as text (a stock's is its initial value); `None` for a graphical function
    /// with no equation, which is a function of time.
    pub eqn: Option<String>,
    pub gf: Option<Gf>,
    /// The units as written (`people/year`).
    pub units: Option<String>,
    pub doc: Option<String>,
}

impl Variable {
    pub fn new(name: &str, kind: VarKind, eqn: &str) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            eqn: Some(eqn.to_owned()),
            gf: None,
            units: None,
            doc: None,
        }
    }

    pub fn stock(name: &str, initial: &str, inflows: &[&str], outflows: &[&str]) -> Self {
        Self::new(
            name,
            VarKind::Stock {
                inflows: inflows.iter().map(|s| (*s).to_owned()).collect(),
                outflows: outflows.iter().map(|s| (*s).to_owned()).collect(),
                non_negative: false,
            },
            initial,
        )
    }

    pub fn flow(name: &str, eqn: &str) -> Self {
        Self::new(
            name,
            VarKind::Flow {
                non_negative: false,
            },
            eqn,
        )
    }

    pub fn aux(name: &str, eqn: &str) -> Self {
        Self::new(name, VarKind::Aux, eqn)
    }

    pub fn with_units(mut self, units: &str) -> Self {
        self.units = Some(units.to_owned());
        self
    }
}

/// A system dynamics model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SdModel {
    pub name: String,
    pub vendor: String,
    pub product: String,
    pub product_version: String,
    pub specs: SimSpecs,
    pub vars: Vec<Variable>,
    /// The diagram: where each variable is drawn. A model read from a file has one for every
    /// variable (see [`Layout`]); one built in code may have none, and is given one when written.
    pub layout: Layout,
}

/// The name an equation uses for a variable: lower case, with each run of spaces, underscores
/// and line breaks one underscore (XMILE 1.0, 3.2.2).
pub fn canonical(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut gap = false;
    for c in name.trim().chars() {
        if c.is_whitespace() || c == '_' || c == '\u{a0}' {
            gap = true;
        } else {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.extend(c.to_lowercase());
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn names_are_compared_by_their_canonical_form() {
        assert_eq!(canonical("Birth  Rate"), "birth_rate");
        assert_eq!(canonical("birth_rate"), "birth_rate");
        assert_eq!(canonical(" Birth\nrate "), "birth_rate");
    }

    #[test]
    fn a_table_on_a_scale_has_even_points() {
        let g = Gf {
            kind: GfKind::Continuous,
            x: GfX::Scale { min: 0.0, max: 1.0 },
            y: vec![0.0, 1.0, 4.0],
        };
        assert_eq!(g.points(), Ok(vec![(0.0, 0.0), (0.5, 1.0), (1.0, 4.0)]));
    }
}
