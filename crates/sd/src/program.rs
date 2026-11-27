//! A model made ready to run: every equation read and its names resolved, the order variables
//! are worked out in, delays and smooths turned into hidden stocks, and the checks that need no
//! run (algebraic loops, units).
//!
//! **Order.** Stocks are state: their values are known before anything is worked out. The other
//! variables (flows and auxiliaries) are worked out in topological order of what their equations
//! name, ties broken by canonical name, so the order does not depend on the file. The input and
//! initial value of a `DELAY` or `SMTH` are not dependencies (the output of one is a state), so
//! a feedback loop through one is not an algebraic loop. A flow that leaves a stock that cannot
//! go negative also depends on the flows before it in that stock's list of outflows, because it
//! is limited by what they left.

use std::collections::{BTreeMap, BTreeSet};

use crate::expr::{Ast, BinOp, parse};
use crate::model::{GfKind, Method, SdModel, StopWhen, VarKind, canonical};
use crate::units::{Units, time_unit};
use crate::{ErrorKind, SdError};

/// Functions with no state of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Func {
    Min,
    Max,
    Abs,
    Sqrt,
    Exp,
    Ln,
    Step,
    Ramp,
    Pulse,
}

/// A resolved equation.
#[derive(Debug, Clone)]
pub(crate) enum Ex {
    Num(f64),
    /// The value of a variable.
    Var(usize),
    Time,
    Dt,
    Start,
    Stop,
    Neg(Box<Ex>),
    Not(Box<Ex>),
    Bin(BinOp, Box<Ex>, Box<Ex>),
    If(Box<Ex>, Box<Ex>, Box<Ex>),
    Fn(Func, Vec<Ex>),
    /// A graphical function (the variable that owns the table) applied to a value.
    Lookup(usize, Box<Ex>),
    /// The output of a delay or smooth.
    Site(usize),
    /// The current draw of a random function.
    Random(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SiteKind {
    Delay1,
    Delay3,
    Smth1,
    Smth3,
}

impl SiteKind {
    pub fn stages(self) -> usize {
        match self {
            SiteKind::Delay1 | SiteKind::Smth1 => 1,
            SiteKind::Delay3 | SiteKind::Smth3 => 3,
        }
    }

    pub fn is_delay(self) -> bool {
        matches!(self, SiteKind::Delay1 | SiteKind::Delay3)
    }
}

/// One call of DELAY1, DELAY3, SMTH1 or SMTH3: one or three hidden stocks.
#[derive(Debug, Clone)]
pub(crate) struct Site {
    pub kind: SiteKind,
    pub input: Ex,
    pub time: Ex,
    pub init: Option<Ex>,
    /// Where its stocks begin among the hidden ones.
    pub offset: usize,
}

/// One call of NORMAL or RANDOM (UNIFORM).
#[derive(Debug, Clone)]
pub(crate) struct RandomSite {
    pub normal: bool,
    pub a: Ex,
    pub b: Ex,
    /// The key of its stream: the variable, which call in it, and the seed it was given.
    pub key: String,
    /// The interval between draws, in the model's time unit; the file's step if not given.
    pub interval: Option<f64>,
}

/// A graphical function, its points checked.
#[derive(Debug, Clone)]
pub(crate) struct Table {
    pub kind: GfKind,
    pub pts: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Stock,
    Flow,
    Aux,
}

#[derive(Debug, Clone)]
pub(crate) struct VarInfo {
    pub name: String,
    pub role: Role,
    pub eqn: Option<Ex>,
    pub table: Option<Table>,
    /// A flow that cannot be negative.
    pub flow_non_negative: bool,
    /// For a flow: the positions (in the state) of the non-negative stocks it leaves, which
    /// limit it.
    pub limited_by: Vec<usize>,
    /// For a flow: the positions of the non-negative stocks it enters. When the flow comes out
    /// negative they are the ones it takes from, so they limit it in that direction.
    pub limited_by_rev: Vec<usize>,
    pub units: Option<String>,
}

/// A model ready to run.
#[derive(Debug, Clone)]
pub struct Program {
    pub(crate) name: String,
    pub(crate) vars: Vec<VarInfo>,
    /// Flows and auxiliaries in the order they are worked out.
    pub(crate) order: Vec<usize>,
    /// The variable of each stock, which is also its place in the state.
    pub(crate) stocks: Vec<usize>,
    /// For each stock: its inflows and outflows (variable indices). A non-negative stock's
    /// limit is on its outflows' `limited_by`.
    pub(crate) inflows: Vec<Vec<usize>>,
    pub(crate) outflows: Vec<Vec<usize>>,
    pub(crate) sites: Vec<Site>,
    pub(crate) hidden: usize,
    pub(crate) randoms: Vec<RandomSite>,
    pub(crate) start: f64,
    pub(crate) stop: f64,
    pub(crate) dt: f64,
    pub(crate) method: Method,
    pub(crate) time_units: String,
    pub(crate) save_step: Option<f64>,
    /// The stop condition: the variable's index, and the condition with its name as the model
    /// writes it.
    pub(crate) stop_when: Option<(usize, StopWhen)>,
    /// What the checks found that does not stop a run.
    pub warnings: Vec<String>,
}

impl Program {
    /// Whether the model has a random function, so that a run is one of many.
    pub fn is_random(&self) -> bool {
        !self.randoms.is_empty()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn start(&self) -> f64 {
        self.start
    }

    pub fn stop(&self) -> f64 {
        self.stop
    }

    pub fn dt(&self) -> f64 {
        self.dt
    }

    pub fn method(&self) -> Method {
        self.method
    }

    pub fn time_units(&self) -> &str {
        &self.time_units
    }

    /// The stop condition, its variable named as the model names it.
    pub fn stop_when(&self) -> Option<&StopWhen> {
        self.stop_when.as_ref().map(|(_, w)| w)
    }

    /// The names of the variables, in the file's order, with their kinds.
    pub fn variables(&self) -> Vec<(&str, &'static str)> {
        self.vars
            .iter()
            .map(|v| {
                (
                    v.name.as_str(),
                    match v.role {
                        Role::Stock => "stock",
                        Role::Flow => "flow",
                        Role::Aux => "aux",
                    },
                )
            })
            .collect()
    }
}

fn err(kind: ErrorKind, message: String) -> SdError {
    SdError { kind, message }
}

/// Function names that XMILE has and this version does not run.
fn unsupported_function(name: &str) -> bool {
    matches!(
        name,
        "sum"
            | "mean"
            | "stddev"
            | "size"
            | "rank"
            | "prod"
            | "init"
            | "previous"
            | "forcst"
            | "trend"
            | "npv"
            | "safediv"
            | "inf"
            | "pi"
            | "int"
            | "sin"
            | "cos"
            | "tan"
            | "arcsin"
            | "arccos"
            | "arctan"
            | "log10"
            | "exprnd"
            | "lognormal"
            | "poisson"
            | "delay"
            | "lookup"
            | "history"
            | "self"
            | "sign"
            | "ceiling"
            | "floor"
            | "round"
    )
}

struct Resolver<'a> {
    names: &'a BTreeMap<String, usize>,
    tables: &'a [bool],
    sites: Vec<Site>,
    randoms: Vec<RandomSite>,
    hidden: usize,
    /// The variable being resolved, and how many random calls it has had.
    owner: String,
    random_calls: usize,
}

fn literal(a: &Ast, what: &str, func: &str) -> Result<f64, String> {
    match a {
        Ast::Num(v) => Ok(*v),
        _ => Err(format!(
            "the {what} of {func} must be a number written in the equation"
        )),
    }
}

impl Resolver<'_> {
    fn args(&mut self, args: &[Ast]) -> Result<Vec<Ex>, String> {
        args.iter().map(|a| self.resolve(a)).collect()
    }

    fn resolve(&mut self, a: &Ast) -> Result<Ex, String> {
        Ok(match a {
            Ast::Num(v) => Ex::Num(*v),
            Ast::Name(n) => match n.as_str() {
                "time" => Ex::Time,
                "dt" => Ex::Dt,
                "starttime" => Ex::Start,
                "stoptime" => Ex::Stop,
                _ => match self.names.get(n) {
                    Some(i) => Ex::Var(*i),
                    None => return Err(format!("`{n}` is not a variable of the model")),
                },
            },
            Ast::Neg(x) => Ex::Neg(Box::new(self.resolve(x)?)),
            Ast::Not(x) => Ex::Not(Box::new(self.resolve(x)?)),
            Ast::Bin(op, l, r) => {
                Ex::Bin(*op, Box::new(self.resolve(l)?), Box::new(self.resolve(r)?))
            }
            Ast::If(c, t, e) => Ex::If(
                Box::new(self.resolve(c)?),
                Box::new(self.resolve(t)?),
                Box::new(self.resolve(e)?),
            ),
            Ast::Call(f, args) => self.call(f, args)?,
        })
    }

    fn call(&mut self, f: &str, args: &[Ast]) -> Result<Ex, String> {
        let upper = f.to_uppercase();
        let count = |lo: usize, hi: usize| -> Result<(), String> {
            if args.len() < lo || args.len() > hi {
                let want = if lo == hi {
                    format!("{lo}")
                } else {
                    format!("{lo} to {hi}")
                };
                Err(format!(
                    "{upper} takes {want} values, but is given {}",
                    args.len()
                ))
            } else {
                Ok(())
            }
        };
        match f {
            "if_then_else" => {
                count(3, 3)?;
                let mut it = self.args(args)?.into_iter();
                match (it.next(), it.next(), it.next()) {
                    (Some(c), Some(t), Some(e)) => {
                        Ok(Ex::If(Box::new(c), Box::new(t), Box::new(e)))
                    }
                    _ => Err("IF_THEN_ELSE needs three values".to_owned()),
                }
            }
            "min" | "max" => {
                if args.len() < 2 {
                    return Err(format!(
                        "{upper} takes at least 2 values, but is given {}",
                        args.len()
                    ));
                }
                let func = if f == "min" { Func::Min } else { Func::Max };
                Ok(Ex::Fn(func, self.args(args)?))
            }
            "abs" | "sqrt" | "exp" | "ln" => {
                count(1, 1)?;
                let func = match f {
                    "abs" => Func::Abs,
                    "sqrt" => Func::Sqrt,
                    "exp" => Func::Exp,
                    _ => Func::Ln,
                };
                Ok(Ex::Fn(func, self.args(args)?))
            }
            "step" => {
                count(2, 2)?;
                Ok(Ex::Fn(Func::Step, self.args(args)?))
            }
            "ramp" => {
                count(2, 3)?;
                Ok(Ex::Fn(Func::Ramp, self.args(args)?))
            }
            "pulse" => {
                count(2, 3)?;
                Ok(Ex::Fn(Func::Pulse, self.args(args)?))
            }
            "delay1" | "delay3" | "smth1" | "smth3" | "smooth" | "smooth3" => {
                count(2, 3)?;
                let kind = match f {
                    "delay1" => SiteKind::Delay1,
                    "delay3" => SiteKind::Delay3,
                    "smth1" | "smooth" => SiteKind::Smth1,
                    _ => SiteKind::Smth3,
                };
                let mut v = self.args(args)?.into_iter();
                let (Some(input), Some(time)) = (v.next(), v.next()) else {
                    return Err(format!("{upper} needs an input and a time"));
                };
                let init = v.next();
                let offset = self.hidden;
                self.hidden += kind.stages();
                self.sites.push(Site {
                    kind,
                    input,
                    time,
                    init,
                    offset,
                });
                Ok(Ex::Site(self.sites.len() - 1))
            }
            "normal" | "random" | "uniform" => {
                count(2, 4)?;
                let normal = f == "normal";
                let seed = match args.get(2) {
                    Some(a) => {
                        let s = literal(a, "seed", &upper)?;
                        if !((0.0..4_294_967_296.0).contains(&s) && s.fract().abs() < 1e-12) {
                            return Err(format!(
                                "the seed of {upper} must be a whole number from 0 to 2^32 - 1"
                            ));
                        }
                        s as u64
                    }
                    None => 0,
                };
                let interval = match args.get(3) {
                    Some(a) => {
                        let v = literal(a, "interval", &upper)?;
                        if !(v.is_finite() && v > 0.0) {
                            return Err(format!("the interval of {upper} must be above 0"));
                        }
                        Some(v)
                    }
                    None => None,
                };
                let first = args.get(..2).unwrap_or_default();
                let mut v = self.args(first)?.into_iter();
                let (Some(a), Some(b)) = (v.next(), v.next()) else {
                    return Err(format!("{upper} needs two values"));
                };
                self.random_calls += 1;
                self.randoms.push(RandomSite {
                    normal,
                    a,
                    b,
                    key: format!("{}#{}#{seed}", self.owner, self.random_calls),
                    interval,
                });
                Ok(Ex::Random(self.randoms.len() - 1))
            }
            _ => {
                // A graphical function used as a function.
                if let Some(&i) = self.names.get(f) {
                    if self.tables.get(i).copied().unwrap_or(false) {
                        count(1, 1)?;
                        let mut v = self.args(args)?.into_iter();
                        return match v.next() {
                            Some(x) => Ok(Ex::Lookup(i, Box::new(x))),
                            None => Err(format!("{f} needs a value")),
                        };
                    }
                    return Err(format!("`{f}` is a variable, not a function"));
                }
                if unsupported_function(f) {
                    Err(format!(
                        "the function {upper} is not supported in this version"
                    ))
                } else {
                    Err(format!(
                        "`{f}` is not a function StochCraft knows, or a graphical function of the model"
                    ))
                }
            }
        }
    }
}

/// The variables an expression needs the values of before it can be worked out.
fn deps(
    e: &Ex,
    sites: &[Site],
    randoms: &[RandomSite],
    is_stock: &dyn Fn(usize) -> bool,
    out: &mut BTreeSet<usize>,
) {
    match e {
        Ex::Num(_) | Ex::Time | Ex::Dt | Ex::Start | Ex::Stop => {}
        Ex::Var(i) => {
            if !is_stock(*i) {
                out.insert(*i);
            }
        }
        Ex::Neg(x) | Ex::Not(x) | Ex::Lookup(_, x) => deps(x, sites, randoms, is_stock, out),
        Ex::Bin(_, l, r) => {
            deps(l, sites, randoms, is_stock, out);
            deps(r, sites, randoms, is_stock, out);
        }
        Ex::If(c, t, f) => {
            deps(c, sites, randoms, is_stock, out);
            deps(t, sites, randoms, is_stock, out);
            deps(f, sites, randoms, is_stock, out);
        }
        Ex::Fn(_, args) => {
            for a in args {
                deps(a, sites, randoms, is_stock, out);
            }
        }
        Ex::Site(s) => {
            if let Some(site) = sites.get(*s)
                && site.kind.is_delay()
            {
                deps(&site.time, sites, randoms, is_stock, out);
            }
        }
        Ex::Random(r) => {
            if let Some(site) = randoms.get(*r) {
                deps(&site.a, sites, randoms, is_stock, out);
                deps(&site.b, sites, randoms, is_stock, out);
            }
        }
    }
}

/// Reads and checks `model`, ready to run. Errors are what makes a run impossible; the
/// program's `warnings` are what does not.
pub fn compile(model: &SdModel) -> Result<Program, SdError> {
    let s = &model.specs;
    let dt = s.dt.dt();
    if !(s.start.is_finite() && s.stop.is_finite() && s.stop > s.start) {
        return Err(err(
            ErrorKind::Model,
            format!(
                "the run must stop after it starts (start {}, stop {})",
                s.start, s.stop
            ),
        ));
    }
    if !(dt.is_finite() && dt > 0.0) {
        return Err(err(
            ErrorKind::Model,
            format!("the time step must be above 0 (it is {dt})"),
        ));
    }
    if let Some(v) = s.save_step
        && !(v.is_finite() && v > 0.0)
    {
        return Err(err(
            ErrorKind::Model,
            format!("the save step must be above 0 (it is {v})"),
        ));
    }
    // Names.
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    for (i, v) in model.vars.iter().enumerate() {
        let c = canonical(&v.name);
        if c.is_empty() {
            return Err(err(ErrorKind::Model, "a variable has no name".to_owned()));
        }
        if matches!(
            c.as_str(),
            "time"
                | "dt"
                | "starttime"
                | "stoptime"
                | "if"
                | "then"
                | "else"
                | "and"
                | "or"
                | "not"
                | "mod"
        ) {
            return Err(err(
                ErrorKind::Model,
                format!("`{}` is a reserved word, not a name for a variable", v.name),
            ));
        }
        if names.insert(c, i).is_some() {
            return Err(err(
                ErrorKind::Model,
                format!(
                    "two variables are called `{}` (names are not case-sensitive, and spaces and underscores are the same)",
                    v.name
                ),
            ));
        }
    }
    let stop_when = match &s.stop_when {
        Some(w) => {
            let Some(&i) = names.get(&canonical(&w.variable)) else {
                return Err(err(
                    ErrorKind::Model,
                    format!(
                        "the stop condition names `{}`, which is not a variable of the model",
                        w.variable
                    ),
                ));
            };
            if !w.value.is_finite() {
                return Err(err(
                    ErrorKind::Model,
                    format!(
                        "the stop condition's value must be a number (it is {})",
                        w.value
                    ),
                ));
            }
            let variable = model
                .vars
                .get(i)
                .map_or_else(|| w.variable.clone(), |v| v.name.clone());
            Some((
                i,
                StopWhen {
                    variable,
                    ..w.clone()
                },
            ))
        }
        None => None,
    };
    let tables: Vec<bool> = model.vars.iter().map(|v| v.gf.is_some()).collect();
    let mut resolver = Resolver {
        names: &names,
        tables: &tables,
        sites: Vec::new(),
        randoms: Vec::new(),
        hidden: 0,
        owner: String::new(),
        random_calls: 0,
    };
    let mut vars: Vec<VarInfo> = Vec::new();
    let mut stocks: Vec<usize> = Vec::new();
    let mut stock_pos: Vec<Option<usize>> = Vec::new();
    for (i, v) in model.vars.iter().enumerate() {
        let role = match v.kind {
            VarKind::Stock { .. } => Role::Stock,
            VarKind::Flow { .. } => Role::Flow,
            VarKind::Aux => Role::Aux,
        };
        resolver.owner = canonical(&v.name);
        resolver.random_calls = 0;
        let eqn = match &v.eqn {
            Some(text) => {
                let ast: Ast = parse(text).map_err(|m| {
                    err(
                        ErrorKind::Equation,
                        format!("the equation of `{}` ({text}): {m}", v.name),
                    )
                })?;
                Some(resolver.resolve(&ast).map_err(|m| {
                    err(
                        ErrorKind::Equation,
                        format!("the equation of `{}` ({text}): {m}", v.name),
                    )
                })?)
            }
            None => None,
        };
        if eqn.is_none() && v.gf.is_none() {
            return Err(err(
                ErrorKind::Model,
                format!("`{}` has no equation", v.name),
            ));
        }
        if eqn.is_none() && role != Role::Aux {
            return Err(err(
                ErrorKind::Model,
                format!("the {} `{}` has no equation", v.kind.name(), v.name),
            ));
        }
        let table = match &v.gf {
            Some(g) => {
                let pts = g.points().map_err(|m| {
                    err(
                        ErrorKind::Model,
                        format!("the graphical function of `{}` {m}", v.name),
                    )
                })?;
                Some(Table { kind: g.kind, pts })
            }
            None => None,
        };
        if role == Role::Stock {
            stock_pos.push(Some(stocks.len()));
            stocks.push(i);
        } else {
            stock_pos.push(None);
        }
        vars.push(VarInfo {
            name: v.name.clone(),
            role,
            eqn,
            table,
            flow_non_negative: matches!(v.kind, VarKind::Flow { non_negative: true }),
            limited_by: Vec::new(),
            limited_by_rev: Vec::new(),
            units: v.units.clone(),
        });
    }
    let sites = resolver.sites;
    let randoms = resolver.randoms;
    let hidden = resolver.hidden;

    // Stocks: flows in and out.
    let mut inflows: Vec<Vec<usize>> = Vec::new();
    let mut outflows: Vec<Vec<usize>> = Vec::new();
    let mut non_negative: Vec<bool> = Vec::new();
    for &si in &stocks {
        let Some(v) = model.vars.get(si) else {
            continue;
        };
        let VarKind::Stock {
            inflows: ins,
            outflows: outs,
            non_negative: nn,
        } = &v.kind
        else {
            continue;
        };
        let lookup = |list: &[String], what: &str| -> Result<Vec<usize>, SdError> {
            list.iter()
                .map(|n| {
                    let idx = names.get(&canonical(n)).copied();
                    match idx.and_then(|i| vars.get(i).map(|x| (i, x.role))) {
                        Some((i, Role::Flow)) => Ok(i),
                        Some(_) => Err(err(
                            ErrorKind::Model,
                            format!(
                                "the stock `{}` lists `{n}` as an {what}, but it is not a flow",
                                v.name
                            ),
                        )),
                        None => Err(err(
                            ErrorKind::Model,
                            format!(
                                "the stock `{}` lists `{n}` as an {what}, but there is no such variable",
                                v.name
                            ),
                        )),
                    }
                })
                .collect()
        };
        let ins = lookup(ins, "inflow")?;
        let outs = lookup(outs, "outflow")?;
        let mut seen = BTreeSet::new();
        for &f in ins.iter().chain(outs.iter()) {
            if !seen.insert(f) {
                return Err(err(
                    ErrorKind::Model,
                    format!(
                        "the stock `{}` lists the flow `{}` more than once (as an inflow or an outflow): a flow can be listed once at a stock",
                        v.name,
                        vars.get(f).map(|x| x.name.as_str()).unwrap_or_default()
                    ),
                ));
            }
        }
        inflows.push(ins);
        outflows.push(outs);
        non_negative.push(*nn);
    }
    // A flow that leaves a non-negative stock is limited by it, after the flows before it. A flow
    // that enters one is limited by it too when it comes out negative (it then takes from that
    // stock); those are limited in the order the flows are worked out.
    let mut extra_deps: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (pos, ins) in inflows.iter().enumerate() {
        if !non_negative.get(pos).copied().unwrap_or(false) {
            continue;
        }
        for &f in ins {
            if let Some(info) = vars.get_mut(f) {
                info.limited_by_rev.push(pos);
            }
        }
    }
    for (pos, outs) in outflows.iter().enumerate() {
        if !non_negative.get(pos).copied().unwrap_or(false) {
            continue;
        }
        for (k, &f) in outs.iter().enumerate() {
            if let Some(info) = vars.get_mut(f) {
                info.limited_by.push(pos);
            }
            if k > 0
                && let Some(&prev) = outs.get(k - 1)
            {
                extra_deps.entry(f).or_default().insert(prev);
            }
        }
    }

    // Order of working out.
    let is_stock = |i: usize| stock_pos.get(i).copied().flatten().is_some();
    let mut deps_of: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (i, info) in vars.iter().enumerate() {
        if info.role == Role::Stock {
            continue;
        }
        let mut d = BTreeSet::new();
        if let Some(e) = &info.eqn {
            deps(e, &sites, &randoms, &is_stock, &mut d);
        }
        if let Some(x) = extra_deps.get(&i) {
            d.extend(x.iter().copied());
        }
        deps_of.insert(i, d);
    }
    let canon_of = |i: usize| {
        model
            .vars
            .get(i)
            .map(|v| canonical(&v.name))
            .unwrap_or_default()
    };
    let mut placed: BTreeSet<usize> = BTreeSet::new();
    let mut order: Vec<usize> = Vec::new();
    let mut remaining: BTreeSet<(String, usize)> =
        deps_of.keys().map(|&i| (canon_of(i), i)).collect();
    loop {
        let ready = remaining
            .iter()
            .find(|(_, i)| {
                deps_of
                    .get(i)
                    .is_some_and(|d| d.iter().all(|x| placed.contains(x)))
            })
            .cloned();
        match ready {
            Some(key) => {
                remaining.remove(&key);
                placed.insert(key.1);
                order.push(key.1);
            }
            None => break,
        }
    }
    if let Some((_, start)) = remaining.iter().next().cloned() {
        let left: BTreeSet<usize> = remaining.iter().map(|(_, i)| *i).collect();
        let mut path = vec![start];
        let mut cur = start;
        let cycle = loop {
            let next = deps_of
                .get(&cur)
                .and_then(|d| d.iter().find(|x| left.contains(x)).copied());
            let Some(next) = next else {
                break vec![start];
            };
            if let Some(p) = path.iter().position(|x| *x == next) {
                let mut c: Vec<usize> = path.get(p..).unwrap_or_default().to_vec();
                c.push(next);
                break c;
            }
            path.push(next);
            cur = next;
        };
        let text: Vec<String> = cycle
            .iter()
            .map(|i| {
                model
                    .vars
                    .get(*i)
                    .map(|v| v.name.clone())
                    .unwrap_or_default()
            })
            .collect();
        return Err(err(
            ErrorKind::Loop,
            format!(
                "algebraic loop: {} (a stock, DELAY or SMTH on the loop breaks it)",
                text.join(" -> ")
            ),
        ));
    }

    let mut program = Program {
        name: model.name.clone(),
        vars,
        order,
        stocks,
        inflows,
        outflows,
        sites,
        hidden,
        randoms,
        start: s.start,
        stop: s.stop,
        dt,
        method: s.method,
        time_units: s.time_units.clone(),
        save_step: s.save_step,
        stop_when,
        warnings: Vec::new(),
    };
    program.warnings = lint(model, &program);
    Ok(program)
}

/// What a unit check made of a part of an equation.
#[derive(Debug, Clone, PartialEq)]
enum UInfo {
    /// Nothing is known (a variable with no units).
    Unknown,
    /// A plain number, which goes with any units.
    Literal,
    Known(Units),
}

struct UnitCheck<'a> {
    program: &'a Program,
    units: Vec<Option<Units>>,
    time: Option<Units>,
    notes: Vec<String>,
    at: String,
}

impl UnitCheck<'_> {
    fn same(&mut self, a: UInfo, b: UInfo, what: &str) -> UInfo {
        match (a, b) {
            (UInfo::Known(x), UInfo::Known(y)) => {
                if x != y {
                    self.notes.push(format!(
                        "in the equation of `{}`, {what} has {x} on one side and {y} on the other",
                        self.at
                    ));
                }
                UInfo::Known(x)
            }
            (UInfo::Known(x), UInfo::Literal) | (UInfo::Literal, UInfo::Known(x)) => {
                UInfo::Known(x)
            }
            (UInfo::Literal, UInfo::Literal) => UInfo::Literal,
            _ => UInfo::Unknown,
        }
    }

    fn var_units(&self, i: usize) -> UInfo {
        match self.units.get(i).cloned().flatten() {
            Some(u) => UInfo::Known(u),
            None => UInfo::Unknown,
        }
    }

    fn infer(&mut self, e: &Ex) -> UInfo {
        let program = self.program;
        match e {
            Ex::Num(_) => UInfo::Literal,
            Ex::Var(i) => self.var_units(*i),
            Ex::Time | Ex::Dt | Ex::Start | Ex::Stop => match &self.time {
                Some(t) => UInfo::Known(t.clone()),
                None => UInfo::Unknown,
            },
            Ex::Neg(x) => self.infer(x),
            Ex::Not(x) => {
                self.infer(x);
                UInfo::Literal
            }
            Ex::Bin(op, l, r) => {
                let a = self.infer(l);
                let b = self.infer(r);
                match op {
                    BinOp::Add | BinOp::Sub => self.same(a, b, "a sum"),
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => {
                        self.same(a, b, "a comparison");
                        UInfo::Literal
                    }
                    BinOp::And | BinOp::Or => UInfo::Literal,
                    BinOp::Mul | BinOp::Div => {
                        let times = *op == BinOp::Mul;
                        match (a, b) {
                            (UInfo::Known(x), UInfo::Known(y)) => {
                                UInfo::Known(if times { x.mul(&y) } else { x.div(&y) })
                            }
                            (UInfo::Known(x), UInfo::Literal) => UInfo::Known(x),
                            (UInfo::Literal, UInfo::Known(y)) => {
                                UInfo::Known(if times { y } else { y.inverse() })
                            }
                            (UInfo::Literal, UInfo::Literal) => UInfo::Literal,
                            _ => UInfo::Unknown,
                        }
                    }
                    BinOp::Mod => a,
                    BinOp::Pow => match (a, r.as_ref()) {
                        (UInfo::Literal, _) => UInfo::Literal,
                        (UInfo::Known(x), Ex::Num(n))
                            if n.fract().abs() < 1e-12 && n.abs() < 1000.0 =>
                        {
                            UInfo::Known(x.pow(*n as i32))
                        }
                        (UInfo::Known(x), _) if x.is_dimensionless() => UInfo::Known(x),
                        _ => UInfo::Unknown,
                    },
                }
            }
            Ex::If(c, t, f) => {
                self.infer(c);
                let a = self.infer(t);
                let b = self.infer(f);
                self.same(a, b, "the two results of an IF")
            }
            Ex::Fn(func, args) => {
                let infos: Vec<UInfo> = args.iter().map(|a| self.infer(a)).collect();
                let first = infos.first().cloned().unwrap_or(UInfo::Unknown);
                match func {
                    Func::Min | Func::Max => {
                        let mut acc = first;
                        for x in infos.into_iter().skip(1) {
                            acc = self.same(acc, x, "MIN or MAX");
                        }
                        acc
                    }
                    Func::Abs | Func::Step => first,
                    Func::Sqrt => match first {
                        UInfo::Known(u) => u.sqrt().map_or(UInfo::Unknown, UInfo::Known),
                        other => other,
                    },
                    Func::Exp | Func::Ln => {
                        if let UInfo::Known(u) = &first
                            && !u.is_dimensionless()
                        {
                            self.notes.push(format!(
                                "in the equation of `{}`, EXP or LN is given {u}, not a plain number",
                                self.at
                            ));
                        }
                        UInfo::Known(Units::dimensionless())
                    }
                    Func::Pulse => match (first, &self.time) {
                        (UInfo::Known(u), Some(t)) => UInfo::Known(u.div(t)),
                        _ => UInfo::Unknown,
                    },
                    Func::Ramp => UInfo::Unknown,
                }
            }
            Ex::Lookup(i, x) => {
                self.infer(x);
                self.var_units(*i)
            }
            Ex::Site(s) => match program.sites.get(*s) {
                Some(site) => {
                    let out = self.infer(&site.input);
                    let t = self.infer(&site.time);
                    if let (UInfo::Known(u), Some(tu)) = (&t, &self.time)
                        && u != tu
                    {
                        self.notes.push(format!(
                            "in the equation of `{}`, a delay or smoothing time is in {u}, but the model's time unit is {tu}",
                            self.at
                        ));
                    }
                    out
                }
                None => UInfo::Unknown,
            },
            Ex::Random(r) => match program.randoms.get(*r) {
                Some(site) => {
                    let x = self.infer(&site.a);
                    let y = self.infer(&site.b);
                    self.same(x, y, "the two values of a random function")
                }
                None => UInfo::Unknown,
            },
        }
    }
}

/// Unit checks and the other advice that needs no run. Mismatches are warnings, not errors.
fn lint(model: &SdModel, program: &Program) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut units: Vec<Option<Units>> = Vec::new();
    for v in &model.vars {
        units.push(match &v.units {
            Some(text) => match Units::parse(text) {
                Ok(u) => Some(u),
                Err(m) => {
                    warnings.push(format!(
                        "the units of `{}` ({text}) cannot be read: {m}",
                        v.name
                    ));
                    None
                }
            },
            None => None,
        });
    }
    let time = if program.time_units.trim().is_empty() {
        None
    } else {
        if time_unit(&program.time_units).is_none() {
            warnings.push(format!(
                "the model's time unit `{}` is not one StochCraft knows (seconds to years), so a run cannot be reported in another",
                program.time_units
            ));
        }
        Some(Units::base(&program.time_units))
    };
    let mut check = UnitCheck {
        program,
        units: units.clone(),
        time: time.clone(),
        notes: Vec::new(),
        at: String::new(),
    };
    for (i, info) in program.vars.iter().enumerate() {
        let Some(eqn) = &info.eqn else {
            continue;
        };
        check.at = info.name.clone();
        let got = check.infer(eqn);
        // A graphical function turns the equation's value into another quantity.
        if info.table.is_some() {
            continue;
        }
        if let (UInfo::Known(g), Some(Some(want))) = (&got, units.get(i))
            && g != want
        {
            check.notes.push(format!(
                "the equation of `{}` is in {g}, but its units are {want}",
                info.name
            ));
        }
    }
    warnings.append(&mut check.notes);
    // A flow into or out of a stock is the stock's units per time unit.
    if let Some(t) = &time {
        for (pos, &si) in program.stocks.iter().enumerate() {
            let Some(Some(su)) = units.get(si) else {
                continue;
            };
            let want = su.div(t);
            let flows = program
                .inflows
                .get(pos)
                .into_iter()
                .flatten()
                .chain(program.outflows.get(pos).into_iter().flatten());
            for &f in flows {
                if let Some(Some(fu)) = units.get(f)
                    && *fu != want
                {
                    let sn = program
                        .vars
                        .get(si)
                        .map(|v| v.name.as_str())
                        .unwrap_or_default();
                    let fname = program
                        .vars
                        .get(f)
                        .map(|v| v.name.as_str())
                        .unwrap_or_default();
                    warnings.push(format!(
                        "the flow `{fname}` is in {fu}, but `{sn}` is in {su} and the model's time unit is {t}, so its flows should be in {want}"
                    ));
                }
            }
        }
    }
    // A step that is large against the shortest time constant of a delay or smooth.
    let mut shortest: Option<f64> = None;
    for site in &program.sites {
        if let Ex::Num(d) = &site.time
            && *d > 0.0
        {
            let stage = *d / site.kind.stages() as f64;
            shortest = Some(shortest.map_or(stage, |s: f64| s.min(stage)));
        }
    }
    if let Some(s) = shortest
        && program.dt > s / 4.0
    {
        warnings.push(format!(
            "the time step {} is more than a quarter of the shortest time constant of a delay or smooth ({s}); results may depend on the step (see dt_check)",
            program.dt
        ));
    }
    warnings
}
