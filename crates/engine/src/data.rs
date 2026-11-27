//! Results made elsewhere, analysed without a model (`data.analyse`, `data.warmup`): a table of
//! per-replication results written by any simulation tool, spreadsheet or script that can write
//! CSV, given the same output analysis as StochCraft's own runs.
//!
//! `data.analyse` reads one of two shapes of table ([`crate::table`] reads the CSV):
//!
//! - wide: one row per replication and one column per measure, with an optional scenario
//!   column and an optional replication column;
//! - long: one row per value, with a measure column (the measure's name) and a value column,
//!   and the same optional scenario and replication columns.
//!
//! An empty cell is a replication with no value; any other cell of a measure that is not a
//! finite number is an error naming its line and column. For each scenario and measure it gives
//! n, the mean with its 95% Student-t half-width, the minimum and maximum, how precisely the
//! replications pin the mean down against a target ([`stochcraft_stats::precision`]: relative
//! ±5% by default) with about how many replications would settle it, and the two outcomes
//! where the replications split ([`stochcraft_stats::split::two_outcomes`]). Each scenario but
//! the baseline is compared with it, measure by measure: the difference with its 95% interval,
//! as `results.compare` computes it (paired-t when the scenarios are declared paired, Welch's
//! two-sample t otherwise), judged as the check against observed data judges a difference
//! ([`stochcraft_stats::observed::judge`] against 0): `differs`, `no_difference_detected`,
//! `agrees` (within a tolerance set in advance) or `inconclusive`. Means and half-widths come
//! with the decimal places the interval supports ([`stochcraft_stats::round`]).
//!
//! Pairing must be declared (`paired`): only the tool that made the runs knows whether its
//! scenarios used common random numbers. Pairs are matched by replication id, so pairing needs
//! a replication column and the same ids in every scenario.
//!
//! `data.warmup` reads a time series per replication (a time column, a replication column, one
//! or more measures, an optional scenario), averages each measure across the replications at
//! each time (Welch's plot) and applies MSER-5 ([`stochcraft_stats::warmup::mser5`]) to the
//! average: the suggested warm-up is the first time kept.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_stats::observed::{self, Check, Outcome};
use stochcraft_stats::precision::{self as rule, SETTLED, Scale, Verdict};
use stochcraft_stats::warmup::{self, MSER_BATCH, MSER_MIN_BATCHES, Truncation};
use stochcraft_stats::{Difference, Summary, Tally, round, split};

use crate::compare::{difference_of, interval_name};
use crate::table::{self, Record, Table};
use crate::{CommandError, ErrorCode};

/// Header names taken for a role when the request names no column for it (case ignored).
const SCENARIO_NAMES: &[&str] = &["scenario"];
const REPLICATION_NAMES: &[&str] = &["replication", "rep", "run"];
const MEASURE_NAMES: &[&str] = &["measure", "metric"];
const VALUE_NAMES: &[&str] = &["value"];
const TIME_NAMES: &[&str] = &["time", "t"];

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParams, message)
}

/// The precision a measure's 95% half-width is judged against.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// A share of the mean: 0.05 is ±5% (the default).
    Relative(f64),
    /// Percentage points, for a measure that is a share between 0 and 1: 1.0 is ±1 point.
    Points(f64),
    /// A half-width in the measure's own unit.
    HalfWidth(f64),
}

impl Target {
    fn scale(self) -> Scale {
        match self {
            Target::Relative(x) => Scale::Relative(x),
            Target::Points(x) => Scale::Points(x),
            Target::HalfWidth(x) => Scale::Absolute(x),
        }
    }

    fn size(self) -> f64 {
        match self {
            Target::Relative(x) | Target::Points(x) | Target::HalfWidth(x) => x,
        }
    }

    fn to_json(self) -> Value {
        match self {
            Target::Relative(x) => json!({ "relative": x }),
            Target::Points(x) => json!({ "points": x }),
            Target::HalfWidth(x) => json!({ "half_width": x }),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyseParams {
    /// The table as CSV text (comma, semicolon or tab delimited; the first row is the header).
    pub csv: String,
    /// The column naming each row's scenario; by default one headed `scenario`, if any. Without
    /// one, the table is a single scenario.
    #[serde(default)]
    pub scenario: Option<String>,
    /// The column giving each row's replication id; by default one headed `replication`, `rep`
    /// or `run`, if any. Without one, rows are numbered in order within each scenario.
    #[serde(default)]
    pub replication: Option<String>,
    /// A long table's column naming the measure of each row; by default one headed `measure` or
    /// `metric` (with a `value` column, the table is then read as long).
    #[serde(default)]
    pub measure: Option<String>,
    /// A long table's column holding the values; by default one headed `value`.
    #[serde(default)]
    pub value: Option<String>,
    /// The measures to analyse: a wide table's columns, or a long table's measure names. By
    /// default every column with numbers (wide), or every measure named (long).
    #[serde(default)]
    pub measures: Option<Vec<String>>,
    /// The scenario the others are compared with; by default the first.
    #[serde(default)]
    pub baseline: Option<String>,
    /// The scenarios used common random numbers, so that replication i of each met the same
    /// random numbers: compare them in pairs, matched by replication id (which needs a
    /// replication column and the same ids in every scenario). StochCraft cannot check this:
    /// declare it only when the tool that made the runs did so.
    #[serde(default)]
    pub paired: bool,
    /// Per measure, a tolerance set in advance: a difference whose whole 95% interval lies
    /// within ± it agrees, one wholly outside it differs.
    #[serde(default)]
    pub tolerance: BTreeMap<String, f64>,
    /// The precision every measure is judged against; by default `{"relative": 0.05}`.
    #[serde(default)]
    pub target: Option<Target>,
    /// Per measure, a precision target in place of `target`.
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WarmupParams {
    /// The time series as CSV text: one row per replication and time (the first row is the
    /// header).
    pub csv: String,
    /// The time column; by default one headed `time` or `t`.
    #[serde(default)]
    pub time: Option<String>,
    /// The replication column; by default one headed `replication`, `rep` or `run`.
    #[serde(default)]
    pub replication: Option<String>,
    /// The scenario column, if any; by default one headed `scenario`, if any.
    #[serde(default)]
    pub scenario: Option<String>,
    /// The measure columns; by default every other column with numbers.
    #[serde(default)]
    pub measures: Option<Vec<String>>,
}

/// A table read for analysis: the header and the rows under it.
struct Sheet {
    /// The table with its records taken out (for its delimiter and numbers).
    table: Table,
    header: Vec<String>,
    rows: Vec<Record>,
}

impl Sheet {
    fn read(csv: &str) -> Result<Self, CommandError> {
        let mut table = table::read(csv).map_err(invalid)?;
        let mut records = std::mem::take(&mut table.records).into_iter();
        let first = records.next().ok_or_else(|| {
            invalid("the table is empty: give a header row and a row per replication")
        })?;
        let header: Vec<String> = first
            .cells
            .iter()
            .enumerate()
            .map(|(i, h)| {
                if h.is_empty() {
                    format!("column {}", i + 1)
                } else {
                    h.clone()
                }
            })
            .collect();
        let mut seen = BTreeSet::new();
        for h in &header {
            if !seen.insert(h.as_str()) {
                return Err(invalid(format!(
                    "two columns are headed `{h}`: give each column its own name"
                )));
            }
        }
        let rows: Vec<Record> = records.collect();
        if rows.is_empty() {
            return Err(invalid("the table has a header but no rows"));
        }
        for r in &rows {
            if r.cells.len() > header.len() && r.cells[header.len()..].iter().any(|c| !c.is_empty())
            {
                return Err(invalid(format!(
                    "line {}: {} cells, but the header has {}",
                    r.line,
                    r.cells.len(),
                    header.len()
                )));
            }
        }
        Ok(Self {
            table,
            header,
            rows,
        })
    }

    fn name(&self, col: usize) -> &str {
        self.header.get(col).map_or("", String::as_str)
    }

    fn list(&self) -> String {
        self.header
            .iter()
            .map(|h| format!("`{h}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The column headed `name` (exactly, else ignoring case).
    fn find(&self, name: &str, role: &str) -> Result<usize, CommandError> {
        self.header
            .iter()
            .position(|h| h == name)
            .or_else(|| {
                self.header
                    .iter()
                    .position(|h| h.eq_ignore_ascii_case(name))
            })
            .ok_or_else(|| {
                invalid(format!(
                    "no column `{name}` for the {role}: the columns are {}",
                    self.list()
                ))
            })
    }

    /// The column for a role: the one named, or else the first headed one of `names`.
    fn role(
        &self,
        given: Option<&str>,
        names: &[&str],
        role: &str,
    ) -> Result<Option<usize>, CommandError> {
        match given {
            Some(name) => self.find(name, role).map(Some),
            None => Ok(self
                .header
                .iter()
                .position(|h| names.iter().any(|n| h.eq_ignore_ascii_case(n)))),
        }
    }

    /// The text of row `r`'s cell in column `col` ("" where the row is short).
    fn cell<'a>(r: &'a Record, col: usize) -> &'a str {
        r.cells.get(col).map_or("", String::as_str)
    }

    /// A cell that must not be empty (a scenario, a replication, a measure's name).
    fn label(&self, r: &Record, col: usize) -> Result<String, CommandError> {
        let c = Self::cell(r, col);
        if c.is_empty() {
            return Err(invalid(format!(
                "line {}: the `{}` cell is empty",
                r.line,
                self.name(col)
            )));
        }
        Ok(c.to_owned())
    }

    /// A measure's value: `None` for an empty cell (no value in that replication), an error for
    /// anything else that is not a finite number.
    fn value(&self, r: &Record, col: usize) -> Result<Option<f64>, CommandError> {
        let c = Self::cell(r, col);
        if c.is_empty() {
            return Ok(None);
        }
        match self.table.number(c) {
            Some(v) if v.is_finite() => Ok(Some(v)),
            _ => Err(invalid(format!(
                "line {}, column `{}`: `{c}` is not a number (leave the cell empty for a replication with no value)",
                r.line,
                self.name(col)
            ))),
        }
    }

    /// The columns other than `roles` with at least one number in them, and those without any.
    fn numeric_columns(&self, roles: &[Option<usize>]) -> (Vec<usize>, Vec<usize>) {
        (0..self.header.len())
            .filter(|c| !roles.contains(&Some(*c)))
            .partition(|&c| {
                self.rows
                    .iter()
                    .any(|r| self.table.number(Self::cell(r, c)).is_some())
            })
    }

    /// The named columns, none of them a role's.
    fn columns(
        &self,
        names: &[String],
        roles: &[Option<usize>],
    ) -> Result<Vec<usize>, CommandError> {
        let mut out = Vec::new();
        for n in names {
            let c = self.find(n, "measure")?;
            if roles.contains(&Some(c)) {
                return Err(invalid(format!(
                    "`{n}` is the scenario, replication or time column, not a measure"
                )));
            }
            if out.contains(&c) {
                return Err(invalid(format!("the measure `{n}` is named twice")));
            }
            out.push(c);
        }
        Ok(out)
    }
}

/// The values read: per scenario and measure, each replication's id and value in row order.
struct Runs {
    /// The scenarios in the order they first appear; one, unnamed, without a scenario column.
    scenarios: Vec<Option<String>>,
    measures: Vec<String>,
    series: BTreeMap<(usize, usize), Vec<(String, Option<f64>)>>,
}

impl Runs {
    fn scenario_index(&mut self, name: Option<String>) -> usize {
        match self.scenarios.iter().position(|s| *s == name) {
            Some(i) => i,
            None => {
                self.scenarios.push(name);
                self.scenarios.len() - 1
            }
        }
    }
}

/// A wide table: one row per replication, one column per measure.
fn read_wide(
    s: &Sheet,
    scenario: Option<usize>,
    replication: Option<usize>,
    measures: &[usize],
) -> Result<Runs, CommandError> {
    let mut runs = Runs {
        scenarios: Vec::new(),
        measures: measures.iter().map(|&c| s.name(c).to_owned()).collect(),
        series: BTreeMap::new(),
    };
    let mut seen: BTreeSet<(usize, String)> = BTreeSet::new();
    let mut counts: BTreeMap<usize, u64> = BTreeMap::new();
    for r in &s.rows {
        let name = scenario.map(|c| s.label(r, c)).transpose()?;
        let si = runs.scenario_index(name);
        let count = counts.entry(si).or_default();
        *count += 1;
        let id = match replication {
            Some(c) => s.label(r, c)?,
            None => count.to_string(),
        };
        if !seen.insert((si, id.clone())) {
            return Err(invalid(format!(
                "line {}: replication `{id}` appears twice{}",
                r.line,
                in_scenario(runs.scenarios[si].as_deref())
            )));
        }
        for (mi, &c) in measures.iter().enumerate() {
            let v = s.value(r, c)?;
            runs.series
                .entry((si, mi))
                .or_default()
                .push((id.clone(), v));
        }
    }
    Ok(runs)
}

/// A long table: one row per value, with the measure's name in a column of its own.
fn read_long(
    s: &Sheet,
    scenario: Option<usize>,
    replication: Option<usize>,
    measure: usize,
    value: usize,
    only: Option<&[String]>,
) -> Result<Runs, CommandError> {
    let mut runs = Runs {
        scenarios: Vec::new(),
        measures: Vec::new(),
        series: BTreeMap::new(),
    };
    let mut seen: BTreeSet<(usize, usize, String)> = BTreeSet::new();
    let mut named: BTreeSet<String> = BTreeSet::new();
    for r in &s.rows {
        let name = scenario.map(|c| s.label(r, c)).transpose()?;
        let m = s.label(r, measure)?;
        named.insert(m.clone());
        if only.is_some_and(|o| !o.contains(&m)) {
            continue;
        }
        let si = runs.scenario_index(name);
        let mi = match runs.measures.iter().position(|x| *x == m) {
            Some(i) => i,
            None => {
                runs.measures.push(m.clone());
                runs.measures.len() - 1
            }
        };
        let list = runs.series.entry((si, mi)).or_default();
        let id = match replication {
            Some(c) => s.label(r, c)?,
            None => (list.len() + 1).to_string(),
        };
        if !seen.insert((si, mi, id.clone())) {
            return Err(invalid(format!(
                "line {}: replication `{id}` has a second value for `{m}`{}",
                r.line,
                in_scenario(runs.scenarios[si].as_deref())
            )));
        }
        list.push((id, s.value(r, value)?));
    }
    if let Some(o) = only {
        for m in o {
            if !named.contains(m) {
                let all: Vec<String> = named.iter().map(|n| format!("`{n}`")).collect();
                return Err(invalid(format!(
                    "no measure `{m}` in the `{}` column: the measures are {}",
                    s.name(measure),
                    all.join(", ")
                )));
            }
        }
    }
    Ok(runs)
}

fn in_scenario(name: Option<&str>) -> String {
    name.map_or_else(String::new, |n| format!(" in scenario `{n}`"))
}

/// The verdict on a difference b − a, judged as the check against observed data judges a
/// simulated result against an observed value: here the observed value is 0 and the "simulated"
/// interval is the difference's. Without a tolerance an interval that excludes 0 `differs` and
/// one that holds it is `no_difference_detected` (the same as `results.compare`'s
/// `significant`, true and false); with one, an interval wholly within ± it `agrees`, wholly
/// outside it `differs`, anything else is `inconclusive`, as is a difference without a finite
/// interval.
fn judged(d: Option<Difference>, tolerance: Option<f64>) -> Outcome {
    let Some(d) = d else {
        return Outcome::Inconclusive;
    };
    let check = Check::Value {
        value: 0.0,
        half_width: None,
        tolerance,
    };
    observed::judge(check, d.mean, d.half_width, d.n_a.min(d.n_b)).outcome
}

/// A mean and half-width as text, rounded together; `None` without an interval.
fn shown(mean: Option<f64>, half_width: Option<f64>) -> (Option<i32>, Option<String>) {
    match (mean, half_width) {
        (Some(m), Some(h)) => (
            round::places(h),
            round::interval(m, h).map(|(m, h)| format!("{m} ± {h}")),
        ),
        _ => (None, None),
    }
}

/// One scenario's measure: its values summarised, judged for precision and checked for a split.
fn describe(values: &[Option<f64>], target: Target) -> Value {
    let xs: Vec<f64> = values.iter().flatten().copied().collect();
    let n = xs.len() as u64;
    let s = Summary::of(&xs);
    let mut tally = Tally::new();
    xs.iter().for_each(|x| tally.add(*x));
    let hw = s.map(|s| s.half_width).filter(|h| h.is_finite());
    let verdict = rule::verdict(
        n,
        s.map(|s| s.mean),
        s.map(|s| s.half_width),
        s.map(|s| s.std_dev),
        target.scale(),
    );
    let runs_needed = match verdict {
        Verdict::Wide(score) => Some(rule::runs_needed(n, score)),
        _ => None,
    };
    let group = |g: split::Group| json!({ "n": g.n, "mean": g.mean, "std_dev": g.std_dev });
    let (decimals, text) = shown(s.map(|s| s.mean), hw);
    json!({
        "n": n,
        "missing": values.len() as u64 - n,
        "mean": s.map(|s| s.mean),
        "std_dev": s.filter(|s| s.n > 1).map(|s| s.std_dev),
        "half_width": hw,
        "min": tally.min(),
        "max": tally.max(),
        "decimals": decimals,
        "text": text,
        "precision": {
            "target": target.to_json(),
            "status": verdict.name(),
            "score": verdict.score(),
            "runs_needed": runs_needed,
        },
        "split": split::two_outcomes(&xs).map(|sp| json!({
            "cut": sp.cut,
            "low": group(sp.low),
            "high": group(sp.high),
        })),
    })
}

const ANALYSE_METHOD: &str = "each scenario's measure: n replications with a value (missing: those with an empty cell), the mean with half_width its 95% Student-t half-width (n − 1 degrees of freedom), the minimum and maximum. precision: the half-width against the target (by default ±5% of the mean); status settled, wide (with runs_needed, about how many replications would settle it: the interval narrows with the square root of their number), no_spread (every replication gave the same value: not settled, a rare event may not have been seen), mean_near_zero (a share of a mean at or near zero cannot be met), no_interval (fewer than two values). split: the replications fall into two clearly separate outcomes, so the mean describes no actual run. Each scenario against the baseline: difference is the scenario's mean minus the baseline's, with half_width its 95% interval: paired-t over the replications with the same id in both (pairs − 1 degrees of freedom) when the scenarios are declared paired (common random numbers, which StochCraft cannot check), Welch's two-sample t otherwise (Welch–Satterthwaite degrees of freedom, rounded down); verdict as the check against observed data judges a difference: differs (the interval excludes 0, or lies wholly outside the tolerance), no_difference_detected (it holds 0, with no tolerance: not evidence of agreement), agrees (wholly within the tolerance), inconclusive (neither, or fewer than two values). decimals and text: the mean and half-width rounded together, the half-width to one significant figure (two where it starts with 1)";

/// `data.analyse`.
pub fn analyse(p: AnalyseParams) -> Result<Value, CommandError> {
    let s = Sheet::read(&p.csv)?;
    let scenario = s.role(p.scenario.as_deref(), SCENARIO_NAMES, "scenario")?;
    let replication = s.role(p.replication.as_deref(), REPLICATION_NAMES, "replication")?;
    let measure = s.role(p.measure.as_deref(), MEASURE_NAMES, "measure name")?;
    let value = s.role(p.value.as_deref(), VALUE_NAMES, "value")?;
    let long = p.measure.is_some() || p.value.is_some() || (measure.is_some() && value.is_some());
    let roles = [scenario, replication];
    let (runs, ignored) = if long {
        let (Some(m), Some(v)) = (measure, value) else {
            return Err(invalid(
                "a long table needs a column naming the measure (`measure`) and a column of values (`value`)",
            ));
        };
        if roles.contains(&Some(m)) || roles.contains(&Some(v)) || m == v {
            return Err(invalid(
                "the measure, value, scenario and replication columns must be different columns",
            ));
        }
        let runs = read_long(&s, scenario, replication, m, v, p.measures.as_deref())?;
        (runs, Vec::new())
    } else {
        if scenario.is_some() && scenario == replication {
            return Err(invalid(
                "the scenario and replication columns must be different columns",
            ));
        }
        let (cols, ignored) = match &p.measures {
            Some(names) => (s.columns(names, &roles)?, Vec::new()),
            None => s.numeric_columns(&roles),
        };
        if cols.is_empty() {
            return Err(invalid(format!(
                "no column of numbers to analyse: the columns are {}",
                s.list()
            )));
        }
        (read_wide(&s, scenario, replication, &cols)?, ignored)
    };
    if runs.measures.is_empty() {
        return Err(invalid("no measures to analyse"));
    }
    let measure_list = || {
        runs.measures
            .iter()
            .map(|m| format!("`{m}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    for (what, keys) in [
        ("a tolerance", p.tolerance.keys().collect::<Vec<_>>()),
        ("a target", p.targets.keys().collect::<Vec<_>>()),
    ] {
        if let Some(k) = keys.into_iter().find(|k| !runs.measures.contains(*k)) {
            return Err(invalid(format!(
                "{what} for `{k}`, which is not a measure: the measures are {}",
                measure_list()
            )));
        }
    }
    if let Some((k, t)) = p
        .tolerance
        .iter()
        .find(|(_, t)| !(t.is_finite() && **t > 0.0))
    {
        return Err(invalid(format!(
            "the tolerance for `{k}` is {t}: give a number above 0"
        )));
    }
    let targets: Vec<Target> = runs
        .measures
        .iter()
        .map(|m| {
            p.targets
                .get(m)
                .copied()
                .or(p.target)
                .unwrap_or(Target::Relative(SETTLED))
        })
        .collect();
    if let Some(t) = targets
        .iter()
        .find(|t| !(t.size().is_finite() && t.size() > 0.0))
    {
        return Err(invalid(format!(
            "the precision target {} is not above 0",
            t.to_json()
        )));
    }
    let names: Vec<Option<&str>> = runs.scenarios.iter().map(Option::as_deref).collect();
    let base = match &p.baseline {
        Some(b) => names
            .iter()
            .position(|n| *n == Some(b.as_str()))
            .ok_or_else(|| {
                let all: Vec<String> = names.iter().flatten().map(|n| format!("`{n}`")).collect();
                invalid(if all.is_empty() {
                    format!("no scenario `{b}`: the table has no scenario column")
                } else {
                    format!("no scenario `{b}`: the scenarios are {}", all.join(", "))
                })
            })?,
        None => 0,
    };
    let ids = |si: usize, mi: usize| {
        runs.series
            .get(&(si, mi))
            .into_iter()
            .flatten()
            .map(|(id, _)| id.as_str())
            .collect::<BTreeSet<&str>>()
    };
    let same_ids = (0..runs.scenarios.len()).all(|si| {
        (0..runs.measures.len()).all(|mi| {
            !runs.series.contains_key(&(si, mi))
                || !runs.series.contains_key(&(base, mi))
                || ids(si, mi) == ids(base, mi)
        })
    });
    if p.paired {
        if replication.is_none() {
            return Err(invalid(
                "`paired` needs a replication column: the replications are paired by their ids, so that replication i of each scenario is the one that met the same random numbers (give `replication`)",
            ));
        }
        for si in (0..runs.scenarios.len()).filter(|si| *si != base) {
            for (mi, m) in runs.measures.iter().enumerate() {
                if !runs.series.contains_key(&(si, mi)) || !runs.series.contains_key(&(base, mi)) {
                    continue;
                }
                let (a, b) = (ids(base, mi), ids(si, mi));
                if let Some(odd) = a.symmetric_difference(&b).next() {
                    return Err(invalid(format!(
                        "`paired` is set, but replication `{odd}` of `{m}` is in only one of scenario `{}` and the baseline `{}`: pairing needs the same replication ids in every scenario",
                        names[si].unwrap_or(""),
                        names[base].unwrap_or("")
                    )));
                }
            }
        }
    }
    let mut results = Vec::new();
    for si in 0..runs.scenarios.len() {
        for (mi, m) in runs.measures.iter().enumerate() {
            let Some(list) = runs.series.get(&(si, mi)) else {
                continue;
            };
            let values: Vec<Option<f64>> = list.iter().map(|(_, v)| *v).collect();
            let mut r = describe(&values, targets[mi]);
            r["scenario"] = json!(names[si]);
            r["measure"] = json!(m);
            results.push(r);
        }
    }
    let mut comparisons = Vec::new();
    for si in (0..runs.scenarios.len()).filter(|si| *si != base) {
        for (mi, m) in runs.measures.iter().enumerate() {
            let (Some(a), Some(b)) = (runs.series.get(&(base, mi)), runs.series.get(&(si, mi)))
            else {
                continue;
            };
            let xa: Vec<Option<f64>> = a.iter().map(|(_, v)| *v).collect();
            // Paired: the scenario's values in the baseline's order of ids.
            let xb: Vec<Option<f64>> = if p.paired {
                let by_id: BTreeMap<&str, Option<f64>> =
                    b.iter().map(|(id, v)| (id.as_str(), *v)).collect();
                a.iter()
                    .map(|(id, _)| by_id.get(id.as_str()).copied().flatten())
                    .collect()
            } else {
                b.iter().map(|(_, v)| *v).collect()
            };
            let d = difference_of(&xa, &xb, p.paired);
            let tolerance = p.tolerance.get(m).copied();
            let hw = d.map(|d| d.half_width).filter(|h| h.is_finite());
            let (decimals, text) = shown(d.map(|d| d.mean), hw);
            let mut c = json!({
                "scenario": names[si],
                "baseline": names[base],
                "measure": m,
                "difference": d.map(|d| d.mean),
                "half_width": hw,
                "df": d.and_then(|d| d.df),
                "interval": interval_name(p.paired),
                "n_baseline": d.map_or(0, |d| d.n_a),
                "n_scenario": d.map_or(0, |d| d.n_b),
                "tolerance": tolerance,
                "verdict": judged(d, tolerance).name(),
                "decimals": decimals,
                "text": text,
            });
            if p.paired {
                c["pairs"] = json!(d.map_or(0, |d| d.n_a));
            }
            comparisons.push(c);
        }
    }
    let mut notes = Vec::new();
    if p.paired {
        notes.push("the scenarios are compared in pairs, as declared: StochCraft cannot check that they used common random numbers, and if they did not, the paired intervals are too narrow".to_owned());
    } else if runs.scenarios.len() > 1 && replication.is_some() && same_ids {
        notes.push("every scenario has the same replication ids: if the tool that made them gave replication i of each scenario the same random numbers (common random numbers), set `paired` for a comparison in pairs, usually much tighter".to_owned());
    }
    let col = |c: Option<usize>| c.map(|c| s.name(c).to_owned());
    Ok(json!({
        "shape": if long { "long" } else { "wide" },
        "delimiter": s.table.delimiter.name(),
        "decimal_comma": s.table.decimal_comma(),
        "rows": s.rows.len() as u64,
        "columns": {
            "scenario": col(scenario),
            "replication": col(replication),
            "measure": if long { col(measure) } else { None },
            "value": if long { col(value) } else { None },
            "ignored": ignored.iter().map(|&c| s.name(c)).collect::<Vec<_>>(),
        },
        "measures": runs.measures,
        "scenarios": names,
        "baseline": names[base],
        "paired": p.paired,
        "results": results,
        "comparisons": comparisons,
        "notes": notes,
        "method": ANALYSE_METHOD,
    }))
}

/// One replication's series in a time series table.
struct Track {
    scenario: usize,
    id: String,
    /// Each row's line, time and measure values.
    rows: Vec<(u64, f64, Vec<f64>)>,
}

const WARMUP_METHOD: &str = "each measure averaged across the replications at each time (Welch's plot), then MSER-5 on that average: the means of consecutive batches of 5 times, and the number of batches d (from none to half of them) to delete that minimises the squared standard error of the mean of the rest; status found (warmup.time is the first time kept, observations the times deleted before it), too_short (the statistic was still falling at the middle of the series: the series is too short to show where the start-up ends; record longer runs), too_few (fewer than 20 times, 4 batches of 5)";

/// `data.warmup`.
pub fn warmup(p: WarmupParams) -> Result<Value, CommandError> {
    let s = Sheet::read(&p.csv)?;
    let time = s
        .role(p.time.as_deref(), TIME_NAMES, "time")?
        .ok_or_else(|| invalid("which column is the time? give `time`"))?;
    let replication = s
        .role(p.replication.as_deref(), REPLICATION_NAMES, "replication")?
        .ok_or_else(|| invalid("which column is the replication? give `replication`"))?;
    let scenario = s.role(p.scenario.as_deref(), SCENARIO_NAMES, "scenario")?;
    if time == replication || scenario == Some(time) || scenario == Some(replication) {
        return Err(invalid(
            "the time, replication and scenario columns must be different columns",
        ));
    }
    let roles = [Some(time), Some(replication), scenario];
    let cols = match &p.measures {
        Some(names) => s.columns(names, &roles)?,
        None => s.numeric_columns(&roles).0,
    };
    if cols.is_empty() {
        return Err(invalid(format!(
            "no column of numbers to average: the columns are {}",
            s.list()
        )));
    }
    let mut scenarios: Vec<Option<String>> = Vec::new();
    let mut tracks: Vec<Track> = Vec::new();
    let mut track_of: BTreeMap<(usize, String), usize> = BTreeMap::new();
    for r in &s.rows {
        let name = scenario.map(|c| s.label(r, c)).transpose()?;
        let si = match scenarios.iter().position(|x| *x == name) {
            Some(i) => i,
            None => {
                scenarios.push(name);
                scenarios.len() - 1
            }
        };
        let id = s.label(r, replication)?;
        let t = s.value(r, time)?.ok_or_else(|| {
            invalid(format!(
                "line {}: the `{}` cell is empty",
                r.line,
                s.name(time)
            ))
        })?;
        let mut values = Vec::with_capacity(cols.len());
        for &c in &cols {
            values.push(s.value(r, c)?.ok_or_else(|| {
                invalid(format!(
                    "line {}, column `{}`: the cell is empty: every replication needs a value at every time",
                    r.line,
                    s.name(c)
                ))
            })?);
        }
        let k = *track_of.entry((si, id.clone())).or_insert_with(|| {
            tracks.push(Track {
                scenario: si,
                id: id.clone(),
                rows: Vec::new(),
            });
            tracks.len() - 1
        });
        if let Some(track) = tracks.get_mut(k) {
            if let Some(&(_, last, _)) = track.rows.last()
                && t <= last
            {
                return Err(invalid(format!(
                    "line {}: replication `{id}` goes from time {last} to {t}: each replication's rows must be in time order, one row per time",
                    r.line
                )));
            }
            track.rows.push((r.line, t, values));
        }
    }
    let mut series = Vec::new();
    for (si, name) in scenarios.iter().enumerate() {
        let mine: Vec<&Track> = tracks.iter().filter(|x| x.scenario == si).collect();
        let Some(first) = mine.first() else {
            continue;
        };
        let times: Vec<f64> = first.rows.iter().map(|(_, t, _)| *t).collect();
        for other in &mine[1..] {
            let theirs: Vec<f64> = other.rows.iter().map(|(_, t, _)| *t).collect();
            if theirs != times {
                let line = other
                    .rows
                    .iter()
                    .zip(&times)
                    .find(|((_, t, _), u)| t != *u)
                    .or_else(|| other.rows.get(times.len()).map(|r| (r, &0.0)))
                    .map_or_else(
                        || other.rows.last().map_or(0, |r| r.0),
                        |((line, _, _), _)| *line,
                    );
                return Err(invalid(format!(
                    "replication `{}`{} is recorded at other times than replication `{}` ({} times against {}, first different at line {line}): every replication must be recorded at the same times",
                    other.id,
                    in_scenario(name.as_deref()),
                    first.id,
                    theirs.len(),
                    times.len()
                )));
            }
        }
        let n = mine.len() as f64;
        for (mi, &c) in cols.iter().enumerate() {
            let average: Vec<f64> = (0..times.len())
                .map(|k| {
                    mine.iter()
                        .map(|x| {
                            x.rows
                                .get(k)
                                .and_then(|r| r.2.get(mi))
                                .copied()
                                .unwrap_or(0.0)
                        })
                        .sum::<f64>()
                        / n
                })
                .collect();
            let found = match warmup::mser5(&average) {
                None => json!({ "status": "too_few" }),
                Some(Truncation::TooShort) => json!({ "status": "too_short" }),
                Some(Truncation::At { observations }) => json!({
                    "status": "found",
                    "observations": observations as u64,
                    "time": times.get(observations),
                }),
            };
            series.push(json!({
                "scenario": name,
                "measure": s.name(c),
                "replications": mine.len() as u64,
                "times": times,
                "average": average,
                "warmup": found,
            }));
        }
    }
    Ok(json!({
        "delimiter": s.table.delimiter.name(),
        "decimal_comma": s.table.decimal_comma(),
        "columns": {
            "time": s.name(time),
            "replication": s.name(replication),
            "scenario": scenario.map(|c| s.name(c)),
            "measures": cols.iter().map(|&c| s.name(c)).collect::<Vec<_>>(),
        },
        "batch": MSER_BATCH as u64,
        "min_times": (MSER_BATCH * MSER_MIN_BATCHES) as u64,
        "series": series,
        "method": WARMUP_METHOD,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stochcraft_stats::difference;

    /// At the boundary |d| = half-width the interval just reaches 0: no difference detected, as
    /// `results.compare`'s `significant` (false there) says; a hair further out, it differs.
    #[test]
    fn the_verdict_at_the_edge_of_zero_agrees_with_significant() {
        let at = |mean: f64, half_width: f64| Difference {
            n_a: 5,
            n_b: 5,
            mean,
            half_width,
            df: Some(4),
        };
        let edge = at(-0.75, 0.75);
        assert_eq!(edge.significant(), Some(false));
        assert_eq!(judged(Some(edge), None), Outcome::NoDifferenceDetected);
        let out = at(-0.75 - 1e-9, 0.75);
        assert_eq!(out.significant(), Some(true));
        assert_eq!(judged(Some(out), None), Outcome::Differs);
        // With a tolerance, the edges of ± it count as inside.
        assert_eq!(judged(Some(at(0.5, 0.5)), Some(1.0)), Outcome::Agrees);
        assert_eq!(judged(Some(at(1.5, 0.5)), Some(1.0)), Outcome::Inconclusive);
        assert_eq!(judged(Some(at(1.6, 0.5)), Some(1.0)), Outcome::Differs);
        assert_eq!(judged(None, None), Outcome::Inconclusive);
    }

    /// One replication on a side gives Welch's interval an infinite half-width: inconclusive,
    /// whatever the tolerance.
    #[test]
    fn an_infinite_welch_half_width_is_inconclusive() {
        let d = difference::welch(&[1.0], &[2.0, 3.0, 4.0]);
        assert!(d.is_some_and(|d| d.half_width.is_infinite() && d.significant().is_none()));
        assert_eq!(judged(d, None), Outcome::Inconclusive);
        assert_eq!(judged(d, Some(100.0)), Outcome::Inconclusive);
    }
}
