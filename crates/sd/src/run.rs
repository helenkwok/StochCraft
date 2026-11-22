//! Running a [`Program`]: Euler or fourth-order Runge-Kutta, replications of a model with random
//! functions, and results over time.
//!
//! **Time** is `start + n * DT` from an integer step counter `n`, never added up, so the same
//! step is at the same time however long the run.
//!
//! **Each step** works out every flow and auxiliary in the program's order from the stocks, then
//! the rate of change of every stock (and of the hidden stocks of delays and smooths). Euler
//! takes one such pass per step; Runge-Kutta four, at the start, twice at the middle and at the
//! end of the step.
//!
//! **Stocks that cannot go negative** limit their outflows, as each flow is worked out: a flow
//! cannot take more than the stock held at the start of the pass, less what the flows before
//! it in the stock's list of outflows have taken. The limited value is the flow's value for
//! everything that reads it, including the stock that receives it, so material is conserved.
//! Under Runge-Kutta the limit is applied at each of the four passes; their weighted sum can
//! leave a stock a little below 0 (by no more than the method's truncation error).
//!
//! **Functions of the step.** `PULSE` is a one-step event: it fires in the step whose number it
//! falls on and is the same in every pass of that step, so its volume is delivered whole under
//! either method. `STEP` and `RAMP` are functions of the time of the pass.
//!
//! **Discontinuities under Runge-Kutta.** The method assumes the rates are smooth over a step.
//! `STEP` is the one jump the stepping handles: it is continuous from the right, and the last of
//! a step's four evaluations is at the end of the step, where the integral over the step wants
//! the value from the left, so that evaluation sees the value just before the jump. A `STEP`
//! whose time falls on a step boundary is then integrated as Euler does it (a jump at 1 adds
//! nothing to the integral up to 1). A jump inside a step, or one made another way (`IF` on
//! `TIME`, a comparison, `MIN`/`MAX` of rates), makes the step's four evaluations disagree about
//! the rate and the method falls to first order there: put such jumps on the step boundaries,
//! use Euler or lower the step (the halved-step check shows the dependence).
//!
//! **Random functions** hold one value for an interval (the file's step unless the equation
//! gives one, in whole steps of the run) and draw it from a stream keyed by the variable, the
//! call and the draw's number under the replication's stream (`Rng::keyed`), so a draw depends
//! on nothing but those: adding a variable, halving the step or skipping an `IF` branch leaves
//! every other draw as it was.

use std::collections::BTreeSet;

use stochcraft_random::math::{exp, ln, powf};
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::Summary;
use stochcraft_stats::spread::quantile;

use crate::expr::BinOp;
use crate::program::{Ex, Func, Program, Role, SiteKind};
use crate::units::time_unit;
use crate::{ErrorKind, SdError};

/// The most steps one run may take.
pub const MAX_STEPS: u64 = 20_000_000;
/// The most replications.
pub const MAX_REPLICATIONS: u64 = 100_000;
/// The most steps over all replications.
const MAX_WORK: u64 = 100_000_000;
/// The most numbers a run may record (variables times saved times times replications).
const MAX_RECORDED: u64 = 50_000_000;
/// Replications of a model with random functions, if none are asked for.
pub const DEFAULT_REPLICATIONS: u64 = 100;
/// Where the halved-step check calls the stepping insensitive: the largest relative difference
/// of a stock's mean path.
pub const DT_CHECK_THRESHOLD: f64 = 0.01;

fn run_err(message: String) -> SdError {
    SdError {
        kind: ErrorKind::Run,
        message,
    }
}

/// What to run.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Replications, for a model with random functions (it is run once otherwise).
    pub replications: Option<u64>,
    /// The seed of the random streams.
    pub seed: u64,
    /// A time step in place of the file's.
    pub dt: Option<f64>,
    /// Also run at half the step and report the largest relative difference of each stock.
    pub dt_check: bool,
    /// Report times in this unit instead of the model's.
    pub time_unit: Option<String>,
}

/// One variable over time.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub name: String,
    pub kind: &'static str,
    pub units: Option<String>,
    /// The mean across replications (the value itself for one run) at each saved time.
    pub mean: Vec<f64>,
    /// The median, and the 5th and 95th percentiles, across replications; empty for one run.
    pub median: Vec<f64>,
    pub low: Vec<f64>,
    pub high: Vec<f64>,
}

/// One stock's answer to the halved-step check.
#[derive(Debug, Clone, PartialEq)]
pub struct StockCheck {
    pub name: String,
    /// The largest, over the saved times, of the difference between the mean path at the step
    /// and at half the step, relative to the larger of the value at half the step and 0.1% of
    /// the stock's largest value.
    pub max_relative_difference: f64,
    /// The time it is largest at (in the results' time unit).
    pub at_time: f64,
    /// How many saved times after the start the two runs share and were compared at.
    pub compared: usize,
    /// Whether the check cannot say for this stock: no time to compare, or a value at one that
    /// is missing or not a number. Its difference is then not a measure of anything.
    pub inconclusive: bool,
}

/// The halved-step check.
#[derive(Debug, Clone, PartialEq)]
pub struct DtCheck {
    pub dt: f64,
    pub half_dt: f64,
    pub stocks: Vec<StockCheck>,
    /// The largest of the stocks' differences.
    pub max_relative_difference: f64,
    /// Whether it is within [`DT_CHECK_THRESHOLD`]. Never true when the check is inconclusive.
    pub within_threshold: bool,
    /// Whether the check cannot say: some stock is inconclusive (or there is no stock).
    pub inconclusive: bool,
    /// Whether the two runs use the same random draws: true for a model with no random function,
    /// and for one whose every random function draws at the same times at both steps.
    pub draws_matched: bool,
    /// What a reader needs to know to take the answer: different end times, inconclusive stocks,
    /// draws that are not matched.
    pub notes: Vec<String>,
}

/// The results of a run.
#[derive(Debug, Clone, PartialEq)]
pub struct SdResults {
    pub name: String,
    pub method: &'static str,
    pub dt: f64,
    pub start: f64,
    pub stop: f64,
    /// The time unit of `times`.
    pub time_unit: String,
    pub save_step: f64,
    pub times: Vec<f64>,
    pub series: Vec<Series>,
    pub replications: u64,
    /// Whether the model has random functions.
    pub random: bool,
    pub seed: u64,
    /// Steps in each run.
    pub steps: u64,
    pub warnings: Vec<String>,
    pub dt_check: Option<DtCheck>,
}

impl SdResults {
    pub fn series(&self, name: &str) -> Option<&Series> {
        let want = crate::model::canonical(name);
        self.series
            .iter()
            .find(|s| crate::model::canonical(&s.name) == want)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visit {
    Var(usize),
    Site(usize),
}

/// The state of one run.
struct Machine<'a> {
    p: &'a Program,
    dt: f64,
    n: u64,
    time: f64,
    /// Whether the pass is the one at the end of a Runge-Kutta step, where `STEP` takes the
    /// value it has just before its time (see the module's note on discontinuities).
    end_stage: bool,
    values: Vec<f64>,
    /// The state the pass works from: stocks, then the hidden stocks of delays and smooths.
    x: Vec<f64>,
    avail: Vec<f64>,
    init: bool,
    memo_var: Vec<Option<f64>>,
    memo_site: Vec<Option<f64>>,
    stack: Vec<Visit>,
    rep_base: u64,
    draw_every: Vec<u64>,
    rand_k: Vec<Option<u64>>,
    rand_val: Vec<f64>,
    /// A digest of when each fresh draw was made: the random function, its draw number and the
    /// time of the stage that made it. Two runs that agree on it shared every random value.
    draw_digest: u64,
    gf_warned: Vec<bool>,
    warnings: BTreeSet<String>,
}

/// One run's saved values.
struct Run {
    times: Vec<f64>,
    /// For each variable, its value at each saved time.
    values: Vec<Vec<f64>>,
    warnings: BTreeSet<String>,
    /// See [`Machine::draw_digest`].
    draw_digest: u64,
}

fn integer_power(x: f64, n: i64) -> f64 {
    let mut result = 1.0;
    let mut base = x;
    let mut k = n.unsigned_abs();
    while k > 0 {
        if k & 1 == 1 {
            result *= base;
        }
        base *= base;
        k >>= 1;
    }
    if n < 0 { 1.0 / result } else { result }
}

fn power(x: f64, y: f64) -> f64 {
    if y.fract().abs() < f64::EPSILON && y.abs() <= 64.0 {
        integer_power(x, y as i64)
    } else {
        powf(x, y)
    }
}

fn truth(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

impl<'a> Machine<'a> {
    fn new(p: &'a Program, dt: f64, rep_base: u64) -> Self {
        let mut warnings = BTreeSet::new();
        let draw_every = p
            .randoms
            .iter()
            .map(|r| {
                let interval = r.interval.unwrap_or(p.dt);
                let steps = (interval / dt).round();
                if steps < 1.0 {
                    warnings.insert(format!(
                        "a random function draws every {interval}, shorter than the time step {dt}: it draws once per step"
                    ));
                    1
                } else {
                    steps as u64
                }
            })
            .collect();
        Self {
            p,
            dt,
            n: 0,
            time: p.start,
            end_stage: false,
            values: vec![0.0; p.vars.len()],
            x: vec![0.0; p.stocks.len() + p.hidden],
            avail: vec![0.0; p.stocks.len()],
            init: false,
            memo_var: vec![None; p.vars.len()],
            memo_site: vec![None; p.sites.len()],
            stack: Vec::new(),
            rep_base,
            draw_every,
            rand_k: vec![None; p.randoms.len()],
            rand_val: vec![0.0; p.randoms.len()],
            draw_digest: 0,
            gf_warned: vec![false; p.vars.len()],
            warnings,
        }
    }

    fn loop_error(&self) -> SdError {
        let names: Vec<&str> = self
            .stack
            .iter()
            .filter_map(|v| match v {
                Visit::Var(i) => self.p.vars.get(*i).map(|x| x.name.as_str()),
                Visit::Site(_) => None,
            })
            .collect();
        SdError {
            kind: ErrorKind::Loop,
            message: format!(
                "algebraic loop in the initial values: {} (a DELAY or SMTH with an initial value written out breaks it)",
                names.join(" -> ")
            ),
        }
    }

    /// The value of variable `i` at the start of the run.
    fn init_var(&mut self, i: usize) -> Result<f64, SdError> {
        if let Some(Some(v)) = self.memo_var.get(i) {
            return Ok(*v);
        }
        if self.stack.contains(&Visit::Var(i)) {
            self.stack.push(Visit::Var(i));
            return Err(self.loop_error());
        }
        self.stack.push(Visit::Var(i));
        let p = self.p;
        let v = match p.vars.get(i) {
            Some(info) => self.variable_value(i, info.eqn.as_ref())?,
            None => 0.0,
        };
        self.stack.pop();
        if let Some(m) = self.memo_var.get_mut(i) {
            *m = Some(v);
        }
        Ok(v)
    }

    /// The value of a variable from its equation (through its table, if it has one).
    fn variable_value(&mut self, i: usize, eqn: Option<&Ex>) -> Result<f64, SdError> {
        let p = self.p;
        let has_table = p.vars.get(i).is_some_and(|v| v.table.is_some());
        let base = match eqn {
            Some(e) => self.eval(e)?,
            None => self.time,
        };
        Ok(if has_table { self.table(i, base) } else { base })
    }

    fn site_init(&mut self, s: usize) -> Result<f64, SdError> {
        if let Some(Some(v)) = self.memo_site.get(s) {
            return Ok(*v);
        }
        if self.stack.contains(&Visit::Site(s)) {
            return Err(self.loop_error());
        }
        self.stack.push(Visit::Site(s));
        let p = self.p;
        let v = match p.sites.get(s) {
            Some(site) => match &site.init {
                Some(e) => self.eval(e)?,
                None => self.eval(&site.input)?,
            },
            None => 0.0,
        };
        self.stack.pop();
        if let Some(m) = self.memo_site.get_mut(s) {
            *m = Some(v);
        }
        Ok(v)
    }

    /// The time of one stage of a delay or smooth: the delay time over the stages, and never
    /// less than the step, which a smaller one cannot be followed by.
    fn stage_time(&mut self, kind: SiteKind, time: &Ex) -> Result<f64, SdError> {
        let d = self.eval(time)?;
        let stage = d / kind.stages() as f64;
        if stage < self.dt {
            self.warnings.insert(format!(
                "a delay or smoothing time ({d}) is shorter than the time step {} times the number of its stages: it is taken as that",
                self.dt
            ));
            return Ok(self.dt);
        }
        Ok(stage)
    }

    fn init_all(&mut self) -> Result<(), SdError> {
        self.init = true;
        self.time = self.p.start;
        self.n = 0;
        let p = self.p;
        let mut x = vec![0.0; p.stocks.len() + p.hidden];
        for (pos, &vi) in p.stocks.iter().enumerate() {
            let v = self.init_var(vi)?;
            if let Some(slot) = x.get_mut(pos) {
                *slot = v;
            }
        }
        for (si, site) in p.sites.iter().enumerate() {
            let init = self.site_init(si)?;
            let level = if site.kind.is_delay() {
                init * self.stage_time(site.kind, &site.time)?
            } else {
                init
            };
            for k in 0..site.kind.stages() {
                if let Some(slot) = x.get_mut(p.stocks.len() + site.offset + k) {
                    *slot = level;
                }
            }
        }
        self.init = false;
        self.x = x;
        Ok(())
    }

    /// A graphical function of variable `i` at `x`.
    fn table(&mut self, i: usize, x: f64) -> f64 {
        let p = self.p;
        let Some(t) = p.vars.get(i).and_then(|v| v.table.as_ref()) else {
            return x;
        };
        let pts = &t.pts;
        let (Some(first), Some(last)) = (pts.first(), pts.last()) else {
            return f64::NAN;
        };
        if x.is_nan() {
            return f64::NAN;
        }
        let outside = x < first.0 || x > last.0;
        if outside && t.kind != crate::model::GfKind::Extrapolate {
            let warned = self.gf_warned.get(i).copied().unwrap_or(true);
            if !warned {
                if let Some(w) = self.gf_warned.get_mut(i) {
                    *w = true;
                }
                self.warnings.insert(format!(
                    "the graphical function of `{}` was given {x}, outside its points ({} to {}): the end value is used",
                    p.vars.get(i).map(|v| v.name.as_str()).unwrap_or_default(),
                    first.0,
                    last.0
                ));
            }
        }
        let line = |a: &(f64, f64), b: &(f64, f64)| {
            if b.0 > a.0 {
                a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0)
            } else {
                a.1
            }
        };
        let at_or_below = pts.partition_point(|q| q.0 <= x);
        if x < first.0 {
            return match t.kind {
                crate::model::GfKind::Extrapolate => match pts.get(1) {
                    Some(second) => line(first, second),
                    None => first.1,
                },
                _ => first.1,
            };
        }
        if x >= last.0 {
            return match t.kind {
                crate::model::GfKind::Extrapolate if x > last.0 && pts.len() >= 2 => {
                    match pts.get(pts.len() - 2) {
                        Some(before) => line(before, last),
                        None => last.1,
                    }
                }
                _ => last.1,
            };
        }
        let (Some(lo), Some(hi)) = (
            at_or_below.checked_sub(1).and_then(|k| pts.get(k)),
            pts.get(at_or_below),
        ) else {
            return first.1;
        };
        match t.kind {
            crate::model::GfKind::Discrete => lo.1,
            _ => line(lo, hi),
        }
    }

    fn draw(&mut self, r: usize) -> Result<f64, SdError> {
        let p = self.p;
        let Some(site) = p.randoms.get(r) else {
            return Ok(0.0);
        };
        let every = self.draw_every.get(r).copied().unwrap_or(1).max(1);
        let k = self.n / every;
        if self.rand_k.get(r).copied().flatten() == Some(k) {
            return Ok(self.rand_val.get(r).copied().unwrap_or(0.0));
        }
        let a = self.eval(&site.a)?;
        let b = self.eval(&site.b)?;
        let mut rng = Rng::keyed(self.rep_base, &format!("{}/{k}", site.key));
        let v = if site.normal {
            if b > 0.0 && a.is_finite() && b.is_finite() {
                Dist::Normal {
                    mean: a,
                    std_dev: b,
                }
                .sample(&mut rng)
            } else {
                a
            }
        } else {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            if hi > lo && lo.is_finite() && hi.is_finite() {
                rng.uniform_range(lo, hi)
            } else {
                lo
            }
        };
        if let Some(s) = self.rand_k.get_mut(r) {
            *s = Some(k);
        }
        for word in [r as u64, k, self.time.to_bits()] {
            self.draw_digest = Rng::keyed(self.draw_digest ^ word, "draw-digest").next_u64();
        }
        if let Some(s) = self.rand_val.get_mut(r) {
            *s = v;
        }
        Ok(v)
    }

    fn site_output(&mut self, s: usize) -> Result<f64, SdError> {
        if self.init {
            return self.site_init(s);
        }
        let p = self.p;
        let Some(site) = p.sites.get(s) else {
            return Ok(0.0);
        };
        let base = p.stocks.len() + site.offset;
        let at = |m: &Self, k: usize| m.x.get(base + k).copied().unwrap_or(0.0);
        Ok(match site.kind {
            SiteKind::Smth1 => at(self, 0),
            SiteKind::Smth3 => at(self, 2),
            SiteKind::Delay1 => at(self, 0) / self.stage_time(site.kind, &site.time)?,
            SiteKind::Delay3 => at(self, 2) / self.stage_time(site.kind, &site.time)?,
        })
    }

    fn eval(&mut self, e: &Ex) -> Result<f64, SdError> {
        Ok(match e {
            Ex::Num(v) => *v,
            Ex::Var(i) => {
                if self.init {
                    self.init_var(*i)?
                } else {
                    self.values.get(*i).copied().unwrap_or(0.0)
                }
            }
            Ex::Time => self.time,
            Ex::Dt => self.dt,
            Ex::Start => self.p.start,
            Ex::Stop => self.p.stop,
            Ex::Neg(x) => -self.eval(x)?,
            Ex::Not(x) => truth(self.eval(x)? == 0.0),
            Ex::If(c, t, f) => {
                if self.eval(c)? != 0.0 {
                    self.eval(t)?
                } else {
                    self.eval(f)?
                }
            }
            Ex::Bin(op, l, r) => {
                let a = self.eval(l)?;
                match op {
                    BinOp::And => truth(a != 0.0 && self.eval(r)? != 0.0),
                    BinOp::Or => truth(a != 0.0 || self.eval(r)? != 0.0),
                    _ => {
                        let b = self.eval(r)?;
                        match op {
                            BinOp::Add => a + b,
                            BinOp::Sub => a - b,
                            BinOp::Mul => a * b,
                            BinOp::Div => a / b,
                            BinOp::Pow => power(a, b),
                            BinOp::Mod => a - b * (a / b).floor(),
                            BinOp::Lt => truth(a < b),
                            BinOp::Le => truth(a <= b),
                            BinOp::Gt => truth(a > b),
                            BinOp::Ge => truth(a >= b),
                            BinOp::Eq => {
                                truth(a.partial_cmp(&b) == Some(std::cmp::Ordering::Equal))
                            }
                            BinOp::Ne => {
                                truth(a.partial_cmp(&b) != Some(std::cmp::Ordering::Equal))
                            }
                            BinOp::And | BinOp::Or => 0.0,
                        }
                    }
                }
            }
            Ex::Fn(Func::Step, args) if self.end_stage => {
                // At the end of a step a STEP is taken at its left limit: the whole call, its
                // arguments too, a hair before the step's end. A step that switches on exactly
                // there is not yet on, while `STEP(6, TIME)` stays on, as it is at every stage.
                let (time, tol) = (self.time, 1e-9 * self.dt);
                self.time = time - 2.0 * tol;
                self.end_stage = false;
                let v = args
                    .iter()
                    .map(|a| self.eval(a))
                    .collect::<Result<Vec<_>, _>>();
                self.time = time;
                self.end_stage = true;
                self.function(Func::Step, &v?)
            }
            Ex::Fn(f, args) => {
                let mut v = Vec::with_capacity(args.len());
                for a in args {
                    v.push(self.eval(a)?);
                }
                self.function(*f, &v)
            }
            Ex::Lookup(i, x) => {
                let v = self.eval(x)?;
                self.table(*i, v)
            }
            Ex::Site(s) => self.site_output(*s)?,
            Ex::Random(r) => self.draw(*r)?,
        })
    }

    fn function(&mut self, f: Func, v: &[f64]) -> f64 {
        let arg = |k: usize| v.get(k).copied().unwrap_or(0.0);
        match f {
            Func::Min => v.iter().copied().fold(f64::INFINITY, f64::min),
            Func::Max => v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            Func::Abs => arg(0).abs(),
            Func::Sqrt => {
                if arg(0) < 0.0 {
                    f64::NAN
                } else {
                    arg(0).sqrt()
                }
            }
            Func::Exp => exp(arg(0)),
            Func::Ln => {
                if arg(0) > 0.0 {
                    ln(arg(0))
                } else {
                    f64::NAN
                }
            }
            Func::Step => {
                if self.time >= arg(1) - 1e-9 * self.dt {
                    arg(0)
                } else {
                    0.0
                }
            }
            Func::Ramp => {
                let end = v.get(2).copied().unwrap_or(f64::INFINITY);
                if self.time <= arg(1) {
                    0.0
                } else {
                    arg(0) * (self.time.min(end) - arg(1))
                }
            }
            Func::Pulse => {
                // The step the pulse falls in, counted from the start of the run.
                let first = ((arg(1) - self.p.start) / self.dt - 1e-9).ceil();
                let rel = self.n as f64 - first;
                let fires = if rel < -0.5 {
                    false
                } else if arg(2) > 0.0 {
                    let every = (arg(2) / self.dt).round().max(1.0);
                    (rel % every).abs() < 0.5
                } else {
                    rel.abs() < 0.5
                };
                if fires { arg(0) / self.dt } else { 0.0 }
            }
        }
    }

    /// Works out every variable from the state at `frac` of the way through step `n`, and gives
    /// the rate of change of every state.
    fn pass(&mut self, frac: f64) -> Result<Vec<f64>, SdError> {
        let p = self.p;
        self.time = p.start + (self.n as f64 + frac) * self.dt;
        self.end_stage = frac >= 1.0;
        for (pos, &vi) in p.stocks.iter().enumerate() {
            let level = self.x.get(pos).copied().unwrap_or(0.0);
            if let Some(v) = self.values.get_mut(vi) {
                *v = level;
            }
            if let Some(a) = self.avail.get_mut(pos) {
                *a = level.max(0.0) / self.dt;
            }
        }
        for &vi in &p.order {
            let Some(info) = p.vars.get(vi) else { continue };
            let mut v = self.variable_value(vi, info.eqn.as_ref())?;
            if info.role == Role::Flow && v.is_finite() {
                // A rate that is not a number is not limited: limiting it (`NaN.min(room)` is
                // `room`) would turn it into a real transfer. It stays as it is, is reported
                // below, and carries through to the stocks it touches.
                if info.flow_non_negative {
                    v = v.max(0.0);
                }
                if v > 0.0 {
                    for &pos in &info.limited_by {
                        let room = self.avail.get(pos).copied().unwrap_or(0.0).max(0.0);
                        v = v.min(room);
                    }
                    if v > 0.0 {
                        for &pos in &info.limited_by {
                            if let Some(a) = self.avail.get_mut(pos) {
                                *a -= v;
                            }
                        }
                    }
                } else if v < 0.0 {
                    // Negative: the flow runs backwards, so the stocks it enters are the ones
                    // it takes from, and they limit it.
                    let mut take = -v;
                    for &pos in &info.limited_by_rev {
                        let room = self.avail.get(pos).copied().unwrap_or(0.0).max(0.0);
                        take = take.min(room);
                    }
                    if take > 0.0 {
                        for &pos in &info.limited_by_rev {
                            if let Some(a) = self.avail.get_mut(pos) {
                                *a -= take;
                            }
                        }
                    }
                    v = -take;
                }
            }
            if !v.is_finite() {
                self.warnings.insert(format!(
                    "`{}` is not a finite number (a division by 0, or the square root or logarithm of a number it cannot take)",
                    info.name
                ));
            }
            if let Some(slot) = self.values.get_mut(vi) {
                *slot = v;
            }
        }
        let mut d = vec![0.0; self.x.len()];
        for pos in 0..p.stocks.len() {
            let sum = |list: Option<&Vec<usize>>| -> f64 {
                list.map_or(0.0, |l| {
                    l.iter()
                        .map(|f| self.values.get(*f).copied().unwrap_or(0.0))
                        .sum()
                })
            };
            let rate = sum(p.inflows.get(pos)) - sum(p.outflows.get(pos));
            if let Some(slot) = d.get_mut(pos) {
                *slot = rate;
            }
        }
        for site in &p.sites {
            let input = self.eval(&site.input)?;
            let tau = self.stage_time(site.kind, &site.time)?;
            let base = p.stocks.len() + site.offset;
            let level = |m: &Self, k: usize| m.x.get(base + k).copied().unwrap_or(0.0);
            let rates: Vec<f64> = match site.kind {
                SiteKind::Delay1 => vec![input - level(self, 0) / tau],
                SiteKind::Delay3 => {
                    let r: Vec<f64> = (0..3).map(|k| level(self, k) / tau).collect();
                    vec![
                        input - r.first().copied().unwrap_or(0.0),
                        r.first().copied().unwrap_or(0.0) - r.get(1).copied().unwrap_or(0.0),
                        r.get(1).copied().unwrap_or(0.0) - r.get(2).copied().unwrap_or(0.0),
                    ]
                }
                SiteKind::Smth1 => vec![(input - level(self, 0)) / tau],
                SiteKind::Smth3 => vec![
                    (input - level(self, 0)) / tau,
                    (level(self, 0) - level(self, 1)) / tau,
                    (level(self, 1) - level(self, 2)) / tau,
                ],
            };
            for (k, r) in rates.into_iter().enumerate() {
                if let Some(slot) = d.get_mut(base + k) {
                    *slot = r;
                }
            }
        }
        Ok(d)
    }

    /// Moves the state on by one step from the pass at its start (`k1`).
    fn advance(&mut self, k1: &[f64]) -> Result<(), SdError> {
        let dt = self.dt;
        match self.p.method {
            crate::model::Method::Euler => {
                for (x, d) in self.x.iter_mut().zip(k1) {
                    *x += dt * d;
                }
            }
            crate::model::Method::Rk4 => {
                let x0 = self.x.clone();
                let shifted = |k: &[f64], h: f64| -> Vec<f64> {
                    x0.iter().zip(k).map(|(x, d)| x + h * d).collect()
                };
                self.x = shifted(k1, dt / 2.0);
                let k2 = self.pass(0.5)?;
                self.x = shifted(&k2, dt / 2.0);
                let k3 = self.pass(0.5)?;
                self.x = shifted(&k3, dt);
                let k4 = self.pass(1.0)?;
                self.x = x0
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        let g = |k: &[f64]| k.get(i).copied().unwrap_or(0.0);
                        x + dt / 6.0 * (g(k1) + 2.0 * g(&k2) + 2.0 * g(&k3) + g(&k4))
                    })
                    .collect();
            }
        }
        Ok(())
    }
}

/// The number of steps from the start to the stop at step `dt`, and a warning if the run is not
/// a whole number of steps long.
fn step_count(p: &Program, dt: f64) -> Result<(u64, Option<String>), SdError> {
    let span = p.stop - p.start;
    let exact = span / dt;
    let n = exact.round();
    if !(1.0..=MAX_STEPS as f64).contains(&n) {
        return Err(run_err(format!(
            "the run would take {exact:.0} steps (the limit is {MAX_STEPS}): lower the length or raise the time step"
        )));
    }
    let note = ((exact - n).abs() > 1e-6).then(|| {
        format!(
            "the run is not a whole number of steps of {dt} long, so it stops at {}",
            p.start + n * dt
        )
    });
    Ok((n as u64, note))
}

fn simulate(
    p: &Program,
    dt: f64,
    steps: u64,
    save_every: u64,
    rep_base: u64,
) -> Result<Run, SdError> {
    let mut m = Machine::new(p, dt, rep_base);
    m.init_all()?;
    let mut times = Vec::new();
    let mut values: Vec<Vec<f64>> = vec![Vec::new(); p.vars.len()];
    for n in 0..=steps {
        m.n = n;
        let k1 = m.pass(0.0)?;
        if n.is_multiple_of(save_every) || n == steps {
            times.push(p.start + n as f64 * dt);
            for (col, v) in values.iter_mut().zip(m.values.iter()) {
                col.push(*v);
            }
        }
        if n == steps {
            break;
        }
        m.advance(&k1)?;
    }
    Ok(Run {
        times,
        values,
        warnings: m.warnings,
        draw_digest: m.draw_digest,
    })
}

/// The replications' base for the streams: depends only on the seed and the replication.
fn replication_base(seed: u64, r: u64) -> u64 {
    Rng::keyed(seed, &format!("replication/{r}")).next_u64()
}

/// The mean, median and 5th and 95th percentiles at each saved time across `runs`.
fn aggregate(p: &Program, runs: &[Run], saved: usize) -> Vec<Series> {
    let many = runs.len() > 1;
    p.vars
        .iter()
        .enumerate()
        .map(|(vi, info)| {
            let mut s = Series {
                name: info.name.clone(),
                kind: match info.role {
                    Role::Stock => "stock",
                    Role::Flow => "flow",
                    Role::Aux => "aux",
                },
                units: info.units.clone(),
                mean: Vec::with_capacity(saved),
                median: Vec::new(),
                low: Vec::new(),
                high: Vec::new(),
            };
            for k in 0..saved {
                let mut at: Vec<f64> = runs
                    .iter()
                    .filter_map(|r| r.values.get(vi).and_then(|c| c.get(k)).copied())
                    .collect();
                if many {
                    s.mean.push(Summary::of(&at).map_or(f64::NAN, |x| x.mean));
                    at.retain(|v| v.is_finite());
                    at.sort_by(f64::total_cmp);
                    s.median.push(quantile(&at, 0.5).unwrap_or(f64::NAN));
                    s.low.push(quantile(&at, 0.05).unwrap_or(f64::NAN));
                    s.high.push(quantile(&at, 0.95).unwrap_or(f64::NAN));
                } else {
                    s.mean.push(at.first().copied().unwrap_or(f64::NAN));
                }
            }
            s
        })
        .collect()
}

/// What one pass of replications makes: the step it is made at and the steps and saving of each.
#[derive(Debug, Clone, Copy)]
struct Plan {
    dt: f64,
    steps: u64,
    save_every: u64,
}

/// A run made a few replications at a time, so a window can draw between them and a long run
/// never holds up the page: [`SdJob::new`] checks the run as [`run`] does, each
/// [`SdJob::step`] makes some replications, [`SdJob::progress`] says how far it has got and
/// [`SdJob::finish`] gives the results. [`run`] is the same job with every replication made at
/// once, so the results are the same however the replications are spread over steps: each one's
/// streams are keyed by the seed and its number alone.
///
/// The job is stepped by whole replications; one replication is not interrupted, so a single
/// huge one (a model run once, or a very long run) is made in one step.
pub struct SdJob {
    program: Program,
    opts: RunOptions,
    plan: Plan,
    /// The plan of the halved-step check, and its runs.
    half: Option<Plan>,
    reps: u64,
    warnings: BTreeSet<String>,
    runs: Vec<Run>,
    half_runs: Vec<Run>,
}

impl SdJob {
    /// Checks the run `opts` asks of `p`, with the same refusals as [`run`], and gets it ready
    /// without making any replication.
    pub fn new(p: &Program, opts: &RunOptions) -> Result<Self, SdError> {
        let dt = opts.dt.unwrap_or(p.dt);
        if !(dt.is_finite() && dt > 0.0) {
            return Err(run_err(format!(
                "the time step must be above 0 (it is {dt})"
            )));
        }
        let (steps, length_note) = step_count(p, dt)?;
        let reps = if p.is_random() {
            let r = opts.replications.unwrap_or(DEFAULT_REPLICATIONS);
            if r == 0 || r > MAX_REPLICATIONS {
                return Err(run_err(format!(
                    "replications must be from 1 to {MAX_REPLICATIONS} (it is {r})"
                )));
            }
            r
        } else {
            1
        };
        if steps.saturating_mul(reps) > MAX_WORK {
            return Err(run_err(format!(
                "{reps} replications of {steps} steps is more than the limit of {MAX_WORK} steps: lower one of them"
            )));
        }
        let save_step = p.save_step.unwrap_or(p.dt);
        let every = (save_step / dt).round();
        let save_every = if every >= 1.0 { every as u64 } else { 1 };
        let saved = steps / save_every + 2;
        if saved
            .saturating_mul(p.vars.len() as u64)
            .saturating_mul(reps)
            > MAX_RECORDED
        {
            return Err(run_err(
                "the results would be too large: raise the save step or lower the replications"
                    .to_owned(),
            ));
        }
        let mut warnings: BTreeSet<String> = p.warnings.iter().cloned().collect();
        warnings.extend(length_note);
        if ((save_step / dt) - every).abs() > 1e-6 || every < 1.0 {
            warnings.insert(format!(
                "the save step {save_step} is not a whole number of time steps of {dt}: results are saved every {} instead",
                save_every as f64 * dt
            ));
        }
        if !p.is_random() && opts.replications.is_some_and(|r| r > 1) {
            warnings.insert(
                "the model has no random function, so it was run once, not in replications"
                    .to_owned(),
            );
        }
        let half = if opts.dt_check {
            let half_dt = dt / 2.0;
            // Twice the steps, so both runs end at the same time, the one the run stops at.
            let half_steps = steps.saturating_mul(2);
            if half_steps > MAX_STEPS {
                return Err(run_err(format!(
                    "the halved-step check would take {half_steps} steps (the limit is {MAX_STEPS}): lower the length or raise the time step"
                )));
            }
            Some(Plan {
                dt: half_dt,
                steps: half_steps,
                save_every: save_every * 2,
            })
        } else {
            None
        };
        Ok(Self {
            program: p.clone(),
            opts: opts.clone(),
            plan: Plan {
                dt,
                steps,
                save_every,
            },
            half,
            reps,
            warnings,
            runs: Vec::new(),
            half_runs: Vec::new(),
        })
    }

    /// Replications made, and replications to make (those of the halved-step check included).
    pub fn progress(&self) -> (u64, u64) {
        let each = if self.half.is_some() { 2 } else { 1 };
        (
            (self.runs.len() + self.half_runs.len()) as u64,
            self.reps * each,
        )
    }

    /// Whether every replication is made.
    pub fn is_done(&self) -> bool {
        let (done, total) = self.progress();
        done >= total
    }

    /// Makes up to `replications` (at least one) of the replications still to make. Stops at the
    /// first error, which the job keeps hitting if stepped again.
    pub fn step(&mut self, replications: usize) -> Result<(), SdError> {
        let mut left = replications.max(1);
        while left > 0 && !self.is_done() {
            let seed = self.opts.seed;
            if (self.runs.len() as u64) < self.reps {
                let r = self.runs.len() as u64;
                let run = simulate(
                    &self.program,
                    self.plan.dt,
                    self.plan.steps,
                    self.plan.save_every,
                    replication_base(seed, r),
                )?;
                self.runs.push(run);
            } else if let Some(half) = self.half {
                let r = self.half_runs.len() as u64;
                let run = simulate(
                    &self.program,
                    half.dt,
                    half.steps,
                    half.save_every,
                    replication_base(seed, r),
                )?;
                self.half_runs.push(run);
            }
            left -= 1;
        }
        Ok(())
    }

    /// The results. Makes any replication not yet made first.
    pub fn finish(mut self) -> Result<SdResults, SdError> {
        self.step(usize::MAX)?;
        let p = &self.program;
        let dt = self.plan.dt;
        let (mut times, series, mut warnings) = collect(p, &self.runs, self.warnings);

        // The halved-step check, on the mean paths of the stocks.
        let mut dt_check = None;
        if let Some(half) = self.half {
            let (half_times, half_series, w) = collect(p, &self.half_runs, BTreeSet::new());
            warnings.extend(w);
            // A random function drawn under a condition can be first called at a time one run
            // reaches and the other does not; equal intervals alone do not show that.
            let same_draw_times = self.runs.len() == self.half_runs.len()
                && self
                    .runs
                    .iter()
                    .zip(&self.half_runs)
                    .all(|(a, b)| a.draw_digest == b.draw_digest);
            dt_check = Some(compare_steps(
                p,
                &series,
                &times,
                &half_series,
                &half_times,
                (dt, half.dt),
                same_draw_times,
            ));
        }

        // Times in the unit asked for.
        let model_unit = p.time_units.trim().to_owned();
        let mut unit = model_unit.clone();
        let mut factor = 1.0;
        if let Some(want) = self
            .opts
            .time_unit
            .as_deref()
            .filter(|w| !w.trim().is_empty())
        {
            match (time_unit(&model_unit), time_unit(want)) {
                (Some((from_name, from)), Some((to_name, to))) => {
                    if from_name != to_name {
                        factor = from / to;
                        unit = to_name.to_owned();
                    }
                }
                _ => {
                    warnings.insert(format!(
                        "the model's time unit ({}) and the unit asked for ({want}) are not both ones StochCraft knows, so times are given in the model's",
                        if model_unit.is_empty() { "none" } else { model_unit.as_str() }
                    ));
                }
            }
        }
        if (factor - 1.0).abs() > 0.0 {
            times.iter_mut().for_each(|t| *t *= factor);
            if let Some(check) = dt_check.as_mut() {
                check.stocks.iter_mut().for_each(|s| s.at_time *= factor);
                check.dt *= factor;
                check.half_dt *= factor;
            }
        }
        Ok(SdResults {
            name: p.name.clone(),
            method: p.method.name(),
            dt: dt * factor,
            start: p.start * factor,
            stop: p.stop * factor,
            time_unit: unit,
            save_step: self.plan.save_every as f64 * dt * factor,
            times,
            series,
            replications: self.reps,
            random: p.is_random(),
            seed: self.opts.seed,
            steps: self.plan.steps,
            warnings: warnings.into_iter().collect(),
            dt_check,
        })
    }
}

/// The time between draws of a random function at step `dt`: a whole number of steps.
fn draw_interval(p: &Program, r: usize, dt: f64) -> f64 {
    let interval = p.randoms.get(r).and_then(|x| x.interval).unwrap_or(p.dt);
    (interval / dt).round().max(1.0) * dt
}

/// The halved-step check: for each stock, the largest relative difference between its mean path
/// at the step and at half the step, compared at the times both saved (by time, not by position
/// in the saved lists). A stock with no time to compare after the start, or whose value at a
/// shared time is missing or not a number, is inconclusive, not a difference of 0.
fn compare_steps(
    p: &Program,
    coarse: &[Series],
    times: &[f64],
    fine: &[Series],
    fine_times: &[f64],
    steps: (f64, f64),
    same_draw_times: bool,
) -> DtCheck {
    let (dt, half_dt) = steps;
    let tol = 1e-9 * dt;
    // The index pairs of the saved times the two runs share, after the start.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut j = 0;
    for (i, t) in times.iter().enumerate() {
        while fine_times.get(j).is_some_and(|f| *f < t - tol) {
            j += 1;
        }
        if fine_times.get(j).is_some_and(|f| (f - t).abs() <= tol) && *t > p.start + tol {
            pairs.push((i, j));
        }
    }
    let mut notes = Vec::new();
    let (end, fine_end) = (
        times.last().copied().unwrap_or(p.start),
        fine_times.last().copied().unwrap_or(p.start),
    );
    if (end - fine_end).abs() > tol {
        notes.push(format!(
            "the runs end at different times ({end} at the step, {fine_end} at half of it): compared at the times they share"
        ));
    }
    let mut stocks = Vec::new();
    for (a, b) in coarse.iter().zip(fine.iter()) {
        if a.kind != "stock" {
            continue;
        }
        let peak = b
            .mean
            .iter()
            .filter(|v| v.is_finite())
            .fold(0.0_f64, |m, v| m.max(v.abs()));
        let floor = (1e-3 * peak).max(f64::MIN_POSITIVE);
        let mut worst = 0.0_f64;
        let mut at = 0.0;
        let mut inconclusive = pairs.is_empty();
        for &(i, j) in &pairs {
            let (Some(x), Some(y)) = (a.mean.get(i), b.mean.get(j)) else {
                inconclusive = true;
                continue;
            };
            if !(x.is_finite() && y.is_finite()) {
                inconclusive = true;
                continue;
            }
            let rel = (x - y).abs() / y.abs().max(floor);
            if rel > worst {
                worst = rel;
                at = times.get(i).copied().unwrap_or(0.0);
            }
        }
        if pairs.is_empty() {
            notes.push(format!(
                "`{}`: the two runs share no saved time after the start, so there is nothing to compare",
                a.name
            ));
        } else if inconclusive {
            notes.push(format!(
                "`{}`: a value is missing or not a number at a time compared, so the check cannot say",
                a.name
            ));
        }
        stocks.push(StockCheck {
            name: a.name.clone(),
            max_relative_difference: worst,
            at_time: at,
            compared: pairs.len(),
            inconclusive,
        });
    }
    let inconclusive = stocks.is_empty() || stocks.iter().any(|s| s.inconclusive);
    let max = stocks
        .iter()
        .map(|s| s.max_relative_difference)
        .fold(0.0_f64, f64::max);
    let draws_matched = same_draw_times
        && (0..p.randoms.len())
            .all(|r| (draw_interval(p, r, dt) - draw_interval(p, r, half_dt)).abs() <= tol);
    if !draws_matched {
        notes.push(
            "a random function draws at different times at the two steps, so the two runs do not share their random values and part of the difference is sampling noise"
                .to_owned(),
        );
    }
    DtCheck {
        dt,
        half_dt,
        stocks,
        max_relative_difference: max,
        within_threshold: !inconclusive && max <= DT_CHECK_THRESHOLD,
        inconclusive,
        draws_matched,
        notes,
    }
}

/// The saved times, the series, and `warnings` with those of the replications `runs`.
fn collect(
    p: &Program,
    runs: &[Run],
    mut warnings: BTreeSet<String>,
) -> (Vec<f64>, Vec<Series>, BTreeSet<String>) {
    let saved = runs.first().map_or(0, |r| r.times.len());
    let times = runs.first().map(|r| r.times.clone()).unwrap_or_default();
    let series = aggregate(p, runs, saved);
    for r in runs {
        warnings.extend(r.warnings.iter().cloned());
    }
    (times, series, warnings)
}

/// Runs `p`: once, or, for a model with random functions, in as many replications as asked.
/// The one-call form of [`SdJob`].
pub fn run(p: &Program, opts: &RunOptions) -> Result<SdResults, SdError> {
    SdJob::new(p, opts)?.finish()
}
