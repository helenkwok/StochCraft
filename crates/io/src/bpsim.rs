//! Reading BPSim 2.0 and 1.0 data: scenarios, scenario parameters and element parameters.
//!
//! Reading is lenient (ROADMAP.md): parameter groups and parameters are found by name in any
//! order, the 1.0 and 2.0 namespaces are both accepted, and values in other time units are
//! converted to the scenario's base unit. What StochCraft cannot use yet becomes a [`Note`].

use std::collections::BTreeMap;

use stochcraft_model::{Dist, TimeUnit};

use crate::Note;
use crate::duration;
use crate::xml::Element;

pub const NS2: &str = "http://www.bpsim.org/schemas/2.0";
pub const NS1: &str = "http://www.bpsim.org/schemas/1.0";
/// Both versions, for lenient reading.
pub const NSS: [&str; 2] = [NS2, NS1];

/// The namespace of StochCraft's vendor extensions' values.
pub const STOCHCRAFT_NS: &str = "https://helenkwok.github.io/StochCraft/bpsim/1";
/// The prefix StochCraft's vendor extension names start with.
pub const STOCHCRAFT: &str = "stochcraft:";
/// A server's finite buffer: `<bpsim:VendorExtension name="stochcraft:queueCapacity"
/// xmlns:stochcraft="…" stochcraft:value="3"/>` in the task's `ElementParameters`.
pub const QUEUE_CAPACITY: &str = "stochcraft:queueCapacity";

/// A shared pool: `stochcraft:pool` with the value `true` on a resource's `ElementParameters`.
/// BPMN with BPSim cannot otherwise tell a pool that one task uses, or none, from a task's own
/// resource; a resource with this marker is always read as a pool.
pub const POOL: &str = "stochcraft:pool";

/// The vendor extension for a server's breakdowns: a `stochcraft:breakdown` element holding
/// `stochcraft:timeToFailure` and `stochcraft:repairTime`, each a BPSim distribution, in the
/// task's `ElementParameters`. Whether a failure interrupts the job is BPSim's own
/// `PriorityParameters/Interruptible`.
pub const BREAKDOWN: &str = "stochcraft:breakdown";

/// The vendor extension for the time a part takes to reach a server: a
/// `stochcraft:transferTime` element holding one BPSim distribution or constant, in the
/// receiving task's `ElementParameters`; with the attribute `holdsSender="false"`, a sending
/// task's resource is free at once (a minimum time in the buffer).
pub const TRANSFER_TIME: &str = "stochcraft:transferTime";

/// The vendor extension for a server's changeovers: a `stochcraft:changeover` element whose
/// attribute `property` names the property whose value changing makes a unit set up, holding a
/// `stochcraft:time` (a BPSim distribution or constant) and any number of `stochcraft:change`
/// elements, each with attributes `from` and `to` and a distribution, for particular changes. A
/// setup before every job is BPSim's own `TimeParameters/SetUpTime`.
pub const CHANGEOVER: &str = "stochcraft:changeover";

/// An exclusive gateway's routing rule: `stochcraft:routing` with the value `shortestQueue` on
/// the gateway's `ElementParameters` sends each entity to the way with the shortest queue that
/// has room.
pub const ROUTING: &str = "stochcraft:routing";

/// The vendor extension for the power a server's units draw in each state: a
/// `stochcraft:power` element with attributes `processing` and `idle` (needed) and `blocked`,
/// `failed` and `offShift`, each in kilowatts, in the task's `ElementParameters`.
pub const POWER: &str = "stochcraft:power";

/// The vendor extension for a server's planned time to make one item (ISO 22400-2's planned
/// runtime per item), for the OEE results: a `stochcraft:plannedRunTime` element holding one
/// BPSim distribution or constant (its mean is the plan), in the task's `ElementParameters`.
/// It does not change the simulation.
pub const PLANNED_RUN_TIME: &str = "stochcraft:plannedRunTime";

/// A batch: `stochcraft:batchSize` on a task's `ElementParameters`, the number of entities it
/// collects before sending them on together (a whole number of at least 1).
pub const BATCH_SIZE: &str = "stochcraft:batchSize";

/// A split: `stochcraft:unbatch` with the value `true` on a task's `ElementParameters` sends
/// each batch's entities on one by one.
pub const UNBATCH: &str = "stochcraft:unbatch";

/// A bill of materials: `stochcraft:quantity` on the `ElementParameters` of a sequence flow
/// into a parallel gateway, the number of entities from it in each one the join sends on.
pub const QUANTITY: &str = "stochcraft:quantity";

/// Rework: `stochcraft:rework` with the value `true` on a sequence flow's `ElementParameters`:
/// entities taking it failed their quality requirements and go back to be worked on again,
/// counted against the last server they finished at (ISO 22400-2's rework quantity).
pub const REWORK: &str = "stochcraft:rework";

/// Scrap: a `stochcraft:scrap` element on an end event's `ElementParameters`: what reaches it
/// is scrap (ISO 22400-2's scrap quantity), counted against the task its attribute `chargedTo`
/// names, or else the last task with a resource of its own each entity finished at.
pub const SCRAP: &str = "stochcraft:scrap";

/// Customer demand: a `stochcraft:demand` element on an end event's `ElementParameters`, with
/// attributes `quantity` (parts, above 0), `per` (`run`, `day`, `week` or `shift`) and an
/// optional `calendar` whose active time is the time available (needed for `shift`), for takt.
pub const DEMAND: &str = "stochcraft:demand";

/// A terminating experiment's stop on a target count: `stochcraft:stopCount` on an element's
/// `ElementParameters`, the run ending once that many entities have left the element.
pub const STOP_COUNT: &str = "stochcraft:stopCount";

/// A terminating experiment's stop when a task empties: `stochcraft:stopWhenEmpty` with the
/// value `true` on the task's `ElementParameters`.
pub const STOP_WHEN_EMPTY: &str = "stochcraft:stopWhenEmpty";

/// A three-point estimate that an element's time distribution was fitted from: a
/// `stochcraft:estimate` element with attributes `family`, `min`, `mode`, `max` or `p95`, and
/// `source`, in the element's `ElementParameters`.
pub const ESTIMATE: &str = "stochcraft:estimate";

/// A vendor extension's value: its attribute named `value` in another namespace (any prefix),
/// else its text.
pub fn vendor_value(e: &Element) -> Option<String> {
    e.attrs
        .iter()
        .find(|(n, _)| {
            n.split_once(':')
                .is_some_and(|(p, l)| p != "xmlns" && l == "value")
        })
        .map(|(_, v)| crate::xml::unescape(v))
        .or_else(|| {
            let t = e.text();
            (!t.trim().is_empty()).then_some(t)
        })
}

/// The parameter groups of `ElementParameters`, in schema order.
pub const GROUPS: [&str; 6] = [
    "TimeParameters",
    "ControlParameters",
    "ResourceParameters",
    "PriorityParameters",
    "CostParameters",
    "PropertyParameters",
];

/// A scenario's settings, in its base time unit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScenarioSettings {
    pub id: String,
    pub name: Option<String>,
    pub replication: Option<u32>,
    pub seed: Option<u64>,
    pub base_time_unit: TimeUnit,
    /// BPSim's duration, which includes the warm-up.
    pub duration: Option<f64>,
    pub warmup: Option<f64>,
    /// The scenario's start (BPSim `Start`), as written: an XML Schema date-time.
    pub start: Option<String>,
}

/// A BPSim `Calendar` as the file has it: id, name and iCalendar text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CalendarText {
    pub id: String,
    pub name: Option<String>,
    pub text: String,
}

/// One scenario with `inherits` resolved: for each referenced element, its parameters by
/// `Group/Name` (and `PropertyParameters/Property/<name>` for properties), the last definition
/// winning as the specification requires.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scenario {
    pub settings: ScenarioSettings,
    pub params: BTreeMap<String, BTreeMap<String, Element>>,
    /// Whether the data used the BPSim 1.0 namespace.
    pub version_1: bool,
    /// The calendars of the scenario and those it inherits, in file order (a scenario's own
    /// replacing an inherited one with the same id).
    pub calendars: Vec<CalendarText>,
}

impl Scenario {
    pub fn param(&self, element: &str, key: &str) -> Option<&Element> {
        self.params.get(element)?.get(key)
    }

    /// Every parameter of `element` whose key starts with `prefix` (a list such as
    /// `ResourceParameters/Role/`), in key order.
    pub fn params_with_prefix<'a>(
        &'a self,
        element: &str,
        prefix: &'a str,
    ) -> impl Iterator<Item = &'a Element> + 'a {
        self.params
            .get(element)
            .into_iter()
            .flat_map(move |m| {
                m.range(prefix.to_owned()..)
                    .take_while(move |(k, _)| k.starts_with(prefix))
            })
            .map(|(_, v)| v)
    }
}

fn parse_unit(s: &str, notes: &mut Vec<Note>, at: Option<&str>) -> Option<TimeUnit> {
    let u = TimeUnit::from_bpsim_name(s.trim());
    if u.is_none() {
        notes.push(Note::warning(
            at,
            format!("unknown time unit `{s}`; the scenario's base unit is used"),
        ));
    }
    u
}

/// Finds the `BPSimData` element anywhere under `root` (inside a BPMN relationship's extension
/// elements, directly in an XPDL package, or as the root of a standalone file).
pub fn find_data(root: &Element) -> Option<&Element> {
    if root.is_any(&NSS, "BPSimData") {
        return Some(root);
    }
    // BPSim 2.0 data wins over 1.0 data when a file carries both.
    let all = root.descendants();
    let pick = |ns: &str| all.iter().copied().find(|e| e.is(ns, "BPSimData"));
    pick(NS2).or_else(|| pick(NS1))
}

/// The ids and names of the input scenarios (result scenarios, which carry a `result`
/// attribute, are outputs and left out).
pub fn scenario_list(data: &Element) -> Vec<(String, Option<String>)> {
    data.children_named(&NSS, "Scenario")
        .filter(|s| s.attr("result").is_none())
        .filter_map(|s| Some((s.attr("id")?, s.attr("name"))))
        .collect()
}

/// Reads scenario `id` (or the first input scenario), resolving `inherits`.
pub fn read_scenario(data: &Element, id: Option<&str>, notes: &mut Vec<Note>) -> Option<Scenario> {
    let scenarios: Vec<&Element> = data.children_named(&NSS, "Scenario").collect();
    let by_id = |want: &str| {
        scenarios
            .iter()
            .copied()
            .find(|s| s.attr("id").as_deref() == Some(want))
    };
    let chosen = match id {
        Some(want) => by_id(want)?,
        None => scenarios
            .iter()
            .copied()
            .find(|s| s.attr("result").is_none())?,
    };
    // The inheritance chain, root first; a cycle stops at the first repeat.
    let mut chain = vec![chosen];
    while let Some(parent) = chain.last().and_then(|s| s.attr("inherits")) {
        match by_id(&parent) {
            Some(p) if !chain.contains(&p) => chain.push(p),
            Some(_) => {
                notes.push(Note::warning(
                    None,
                    format!("scenario inheritance loops at `{parent}`"),
                ));
                break;
            }
            None => {
                notes.push(Note::warning(
                    None,
                    format!("inherited scenario `{parent}` does not exist"),
                ));
                break;
            }
        }
    }
    chain.reverse();
    let mut sc = Scenario {
        version_1: data.ns.as_deref() == Some(NS1),
        ..Scenario::default()
    };
    sc.settings.id = chosen.attr("id").unwrap_or_default();
    sc.settings.name = chosen.attr("name");
    for s in &chain {
        if let Some(p) = s.child(&NSS, "ScenarioParameters") {
            read_settings(p, &mut sc.settings, notes);
        }
        for ep in s.children_named(&NSS, "ElementParameters") {
            let Some(r) = ep.attr("elementRef") else {
                continue;
            };
            let target = sc.params.entry(r).or_default();
            for group in ep.elements() {
                let g = group.local().to_owned();
                if group.local() == "VendorExtension" {
                    let name = group.attr("name").unwrap_or_default();
                    target.insert(format!("VendorExtension/{name}"), group.clone());
                    continue;
                }
                for p in group.elements() {
                    let key = match (p.local(), p.attr("name")) {
                        ("Property", Some(n)) => format!("{g}/Property/{n}"),
                        // Several roles are allowed; keep each.
                        ("Role", _) => format!("{g}/Role/{}", target.len()),
                        (l, _) => format!("{g}/{}", normalise_param(l)),
                    };
                    // An inheriting scenario overrides only the values it provides: a parameter
                    // that only adds result requests keeps the inherited value.
                    let has_value = p.elements().any(|v| v.local() != "ResultRequest");
                    if has_value || !target.contains_key(&key) {
                        target.insert(key, p.clone());
                    }
                }
            }
        }
        for c in s.children_named(&NSS, "Calendar") {
            let Some(id) = c.attr("id") else {
                notes.push(Note::warning(None, "a calendar without an id is ignored"));
                continue;
            };
            let cal = CalendarText {
                id,
                name: c.attr("name"),
                text: c.text(),
            };
            match sc.calendars.iter_mut().find(|x| x.id == cal.id) {
                Some(old) => *old = cal,
                None => sc.calendars.push(cal),
            }
        }
    }
    Some(sc)
}

/// Accepts common spellings: the schema's `SetUpTime` and the specification's `setupTime`.
fn normalise_param(local: &str) -> &str {
    match local {
        "SetupTime" => "SetUpTime",
        other => other,
    }
}

fn read_settings(p: &Element, s: &mut ScenarioSettings, notes: &mut Vec<Note>) {
    if let Some(r) = p.attr("replication") {
        match r.trim().parse() {
            Ok(n) => s.replication = Some(n),
            Err(_) => notes.push(Note::warning(
                None,
                format!("replication `{r}` is not a whole number"),
            )),
        }
    }
    if let Some(v) = p.attr("seed") {
        // `xs:long`: a negative seed is kept as the same 64 bits.
        match v.trim().parse::<i64>() {
            Ok(n) => s.seed = Some(n as u64),
            Err(_) => notes.push(Note::warning(
                None,
                format!("seed `{v}` is not a whole number"),
            )),
        }
    }
    if let Some(u) = p
        .attr("baseTimeUnit")
        .and_then(|u| parse_unit(&u, notes, None))
    {
        s.base_time_unit = u;
    }
    let unit = s.base_time_unit;
    for (name, slot) in [("Duration", &mut s.duration), ("Warmup", &mut s.warmup)] {
        if let Some(param) = p.child(&NSS, name) {
            match value(param, unit, notes, None) {
                Some(Value::Number(v)) => *slot = Some(v),
                Some(Value::Dist(Dist::Constant(v))) => *slot = Some(v),
                _ => notes.push(Note::warning(
                    None,
                    format!("scenario {name} is not a fixed time and is ignored"),
                )),
            }
        }
    }
    if let Some(param) = p.child(&NSS, "Start") {
        match value(param, unit, notes, None) {
            Some(Value::Text(t)) => s.start = Some(t),
            _ => notes.push(Note::warning(
                None,
                "the scenario start is not a date and time (DateTimeParameter) and is ignored",
            )),
        }
    }
}

/// A parameter's input value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A fixed number; times are in the scenario's base unit.
    Number(f64),
    Bool(bool),
    Text(String),
    Dist(Dist),
    /// Text drawn by shares: a `UserDistribution` whose data points are `StringParameter`s.
    Choice(Vec<(f64, String)>),
    /// A BPSim expression (XPath; see `stochcraft_model::expr`).
    Expression(String),
}

fn input_values(param: &Element) -> Vec<&Element> {
    param
        .elements()
        .filter(|v| {
            v.ns.as_deref().is_some_and(|n| NSS.contains(&n)) && v.local() != "ResultRequest"
        })
        .filter(|v| v.attr("result").is_none())
        .collect()
}

/// The default value of a parameter element (the first value without `validFor`), in `unit`.
/// Result values are skipped, and so are values tied to calendars, with a note: only the
/// parameters that read them with [`timed_values`] use them.
pub fn value(
    param: &Element,
    unit: TimeUnit,
    notes: &mut Vec<Note>,
    at: Option<&str>,
) -> Option<Value> {
    let values = input_values(param);
    if values.iter().any(|v| v.attr("validFor").is_some()) {
        notes.push(Note::warning(
            at,
            format!(
                "{}: values valid for a calendar are used only for arrivals and resource quantity and availability; they are ignored here",
                param.local()
            ),
        ));
    }
    default_value(param, unit, notes, at)
}

/// The default value of a parameter element, without a note about values tied to calendars.
pub fn default_value(
    param: &Element,
    unit: TimeUnit,
    notes: &mut Vec<Note>,
    at: Option<&str>,
) -> Option<Value> {
    let v = input_values(param)
        .into_iter()
        .find(|v| v.attr("validFor").is_none())?;
    parse_value(v, unit, notes, at)
}

/// The values of a parameter element valid for a calendar (`validFor`), in file order: the
/// calendar's id and the value, in `unit`.
pub fn timed_values(
    param: &Element,
    unit: TimeUnit,
    notes: &mut Vec<Note>,
    at: Option<&str>,
) -> Vec<(String, Value)> {
    input_values(param)
        .into_iter()
        .filter_map(|v| {
            let c = v.attr("validFor")?;
            Some((c, parse_value(v, unit, notes, at)?))
        })
        .collect()
}

fn num(e: &Element, name: &str) -> Option<f64> {
    e.attr(name)?.trim().parse::<f64>().ok()
}

/// Seconds-per-unit factor from `from` to `to`.
pub(crate) fn factor(from: TimeUnit, to: TimeUnit) -> f64 {
    to.convert(1.0, from)
}

fn parse_value(
    v: &Element,
    unit: TimeUnit,
    notes: &mut Vec<Note>,
    at: Option<&str>,
) -> Option<Value> {
    let own_unit = v
        .attr("timeUnit")
        .and_then(|u| parse_unit(&u, notes, at))
        .unwrap_or(unit);
    let c = factor(own_unit, unit);
    let bad = |notes: &mut Vec<Note>, what: &str| {
        notes.push(Note::error(at, format!("{}: {what}", v.local())));
        None
    };
    let n = |name: &str| num(v, name);
    let dist = match v.local() {
        "FloatingParameter" | "NumericParameter" => {
            return match n("value") {
                Some(x) => Some(Value::Number(x * c)),
                None => bad(notes, "`value` is missing or not a number"),
            };
        }
        "DurationParameter" => {
            let raw = v.attr("value").unwrap_or_default();
            return match duration::seconds(&raw) {
                Some(secs) => Some(Value::Number(secs / unit.seconds())),
                None => bad(notes, &format!("`{raw}` is not an ISO 8601 duration")),
            };
        }
        "BooleanParameter" => {
            return match v.attr("value").as_deref().map(str::trim) {
                Some("true" | "1") => Some(Value::Bool(true)),
                Some("false" | "0") => Some(Value::Bool(false)),
                _ => bad(notes, "`value` is not true or false"),
            };
        }
        "StringParameter" => return Some(Value::Text(v.attr("value").unwrap_or_default())),
        "ExpressionParameter" => {
            return Some(Value::Expression(v.attr("value").unwrap_or_default()));
        }
        "NegativeExponentialDistribution" => n("mean").map(|mean| Dist::Exponential { mean }),
        "UniformDistribution" => n("min")
            .zip(n("max"))
            .map(|(min, max)| Dist::Uniform { min, max }),
        "TriangularDistribution" => match (n("min"), n("mode"), n("max")) {
            (Some(min), Some(mode), Some(max)) => Some(Dist::Triangular { min, mode, max }),
            _ => None,
        },
        "NormalDistribution" => n("mean")
            .zip(n("standardDeviation"))
            .map(|(mean, std_dev)| Dist::Normal { mean, std_dev }),
        "LogNormalDistribution" => n("mean")
            .zip(n("standardDeviation"))
            .map(|(mean, std_dev)| Dist::LogNormal { mean, std_dev }),
        "TruncatedNormalDistribution" => {
            match (n("mean"), n("standardDeviation"), n("min"), n("max")) {
                (Some(mean), Some(std_dev), Some(min), Some(max)) => Some(Dist::TruncatedNormal {
                    mean,
                    std_dev,
                    min,
                    max,
                }),
                _ => None,
            }
        }
        "GammaDistribution" => n("shape")
            .zip(n("scale"))
            .map(|(shape, scale)| Dist::Gamma { shape, scale }),
        "BetaDistribution" => n("shape")
            .zip(n("scale"))
            .map(|(shape, scale)| Dist::Beta { shape, scale }),
        "WeibullDistribution" => n("shape")
            .zip(n("scale"))
            .map(|(shape, scale)| Dist::Weibull { shape, scale }),
        "ErlangDistribution" => n("mean").zip(n("k")).and_then(|(mean, k)| {
            // The schema types `k` as a double; it must be a whole number of at least 1.
            (k.fract() == 0.0 && k >= 1.0 && k <= u64::MAX as f64)
                .then_some(Dist::Erlang { mean, k: k as u64 })
        }),
        "PoissonDistribution" => n("mean").map(|mean| Dist::Poisson { mean }),
        "BinomialDistribution" => n("probability")
            .zip(n("trials"))
            .and_then(|(probability, t)| {
                (t.fract() == 0.0 && t >= 0.0 && t <= u64::MAX as f64).then_some(Dist::Binomial {
                    probability,
                    trials: t as u64,
                })
            }),
        "UserDistribution" => {
            let discrete = v
                .attr("discrete")
                .is_some_and(|d| d.trim() == "true" || d.trim() == "1");
            let mut points = Vec::new();
            let mut texts = Vec::new();
            for p in v.children_named(&NSS, "UserDistributionDataPoint") {
                let prob = num(p, "probability");
                let val = p
                    .elements()
                    .find_map(|x| match parse_value(x, own_unit, notes, at) {
                        Some(Value::Number(n)) => Some(Value::Number(n)),
                        Some(Value::Dist(Dist::Constant(n))) => Some(Value::Number(n)),
                        Some(Value::Text(t)) => Some(Value::Text(t)),
                        _ => None,
                    });
                match (prob, val) {
                    (Some(pr), Some(Value::Number(n))) => points.push((pr, n)),
                    (Some(pr), Some(Value::Text(t))) => texts.push((pr, t)),
                    _ => {
                        return bad(notes, "a data point needs a probability and a fixed value");
                    }
                }
            }
            // Text values: a mix drawn by shares (a product mix), for properties.
            match (points.is_empty(), texts.is_empty()) {
                (true, false) => return Some(Value::Choice(texts)),
                (false, false) => {
                    return bad(notes, "data points are either all numbers or all text");
                }
                _ => {}
            }
            Some(Dist::User { points, discrete })
        }
        "EnumParameter" => {
            notes.push(Note::warning(
                at,
                "EnumParameter (historical values) is not supported yet; parameter ignored",
            ));
            return None;
        }
        "DateTimeParameter" => return Some(Value::Text(v.attr("value").unwrap_or_default())),
        other => {
            notes.push(Note::warning(
                at,
                format!("unknown parameter value `{other}` ignored"),
            ));
            return None;
        }
    };
    let Some(d) = dist else {
        return bad(notes, "a required attribute is missing or not a number");
    };
    match scale(&d, c) {
        Some(scaled) => Some(Value::Dist(scaled)),
        None => bad(
            notes,
            "this distribution cannot be converted to another time unit",
        ),
    }
}

/// `d` with every value multiplied by `c` (`c` > 0), for a change of time unit. `None` for
/// count distributions and the beta, whose shape does not survive scaling.
pub fn scale(d: &Dist, c: f64) -> Option<Dist> {
    if c == 1.0 {
        return Some(d.clone());
    }
    Some(match d.clone() {
        Dist::Constant(v) => Dist::Constant(v * c),
        Dist::Exponential { mean } => Dist::Exponential { mean: mean * c },
        Dist::Uniform { min, max } => Dist::Uniform {
            min: min * c,
            max: max * c,
        },
        Dist::Triangular { min, mode, max } => Dist::Triangular {
            min: min * c,
            mode: mode * c,
            max: max * c,
        },
        Dist::Normal { mean, std_dev } => Dist::Normal {
            mean: mean * c,
            std_dev: std_dev * c,
        },
        Dist::LogNormal { mean, std_dev } => Dist::LogNormal {
            mean: mean * c,
            std_dev: std_dev * c,
        },
        Dist::TruncatedNormal {
            mean,
            std_dev,
            min,
            max,
        } => Dist::TruncatedNormal {
            mean: mean * c,
            std_dev: std_dev * c,
            min: min * c,
            max: max * c,
        },
        Dist::Gamma { shape, scale } => Dist::Gamma {
            shape,
            scale: scale * c,
        },
        Dist::Weibull { shape, scale } => Dist::Weibull {
            shape,
            scale: scale * c,
        },
        Dist::Erlang { mean, k } => Dist::Erlang { mean: mean * c, k },
        Dist::User { points, discrete } => Dist::User {
            points: points.into_iter().map(|(p, v)| (p, v * c)).collect(),
            discrete,
        },
        Dist::Beta { .. } | Dist::Poisson { .. } | Dist::Binomial { .. } => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::parse;

    fn data(xml: &str) -> Element {
        parse(xml.as_bytes())
            .map(|d| d.root)
            .unwrap_or_else(|e| unreachable!("{e}"))
    }

    const TWO_SCENARIOS: &str = r#"<b:BPSimData xmlns:b="http://www.bpsim.org/schemas/2.0">
      <b:Scenario id="S1" name="base">
        <b:ScenarioParameters replication="5" seed="-1" baseTimeUnit="hour">
          <b:Warmup><b:DurationParameter value="PT30M"/></b:Warmup>
          <b:Duration><b:DurationParameter value="PT10H"/></b:Duration>
        </b:ScenarioParameters>
        <b:ElementParameters elementRef="task">
          <b:ControlParameters><b:TriggerCount><b:NumericParameter value="3"/></b:TriggerCount></b:ControlParameters>
          <b:TimeParameters><b:ProcessingTime><b:ResultRequest>mean</b:ResultRequest>
            <b:TriangularDistribution min="6" mode="12" max="30" timeUnit="min"/></b:ProcessingTime></b:TimeParameters>
        </b:ElementParameters>
      </b:Scenario>
      <b:Scenario id="S2" inherits="S1">
        <b:ElementParameters elementRef="task">
          <b:TimeParameters><b:ProcessingTime><b:FloatingParameter value="0.5"/></b:ProcessingTime></b:TimeParameters>
        </b:ElementParameters>
      </b:Scenario>
      <b:Scenario id="S3" inherits="S2">
        <b:ElementParameters elementRef="task">
          <b:TimeParameters><b:ProcessingTime><b:ResultRequest>sum</b:ResultRequest></b:ProcessingTime></b:TimeParameters>
        </b:ElementParameters>
      </b:Scenario>
      <b:Scenario id="S2_results" result="S2"/>
    </b:BPSimData>"#;

    #[test]
    fn scenarios_settings_and_units() {
        let d = data(TWO_SCENARIOS);
        assert_eq!(
            scenario_list(&d),
            vec![
                ("S1".into(), Some("base".into())),
                ("S2".into(), None),
                ("S3".into(), None)
            ]
        );
        let mut notes = vec![];
        let s1 = read_scenario(&d, None, &mut notes).unwrap_or_default();
        assert_eq!(s1.settings.replication, Some(5));
        assert_eq!(s1.settings.seed, Some(u64::MAX));
        assert_eq!(s1.settings.base_time_unit, TimeUnit::Hours);
        assert_eq!(
            (s1.settings.warmup, s1.settings.duration),
            (Some(0.5), Some(10.0))
        );
        // Parameter groups in any order; a distribution in minutes becomes hours.
        let pt = s1.param("task", "TimeParameters/ProcessingTime");
        let v = pt.and_then(|p| value(p, TimeUnit::Hours, &mut notes, None));
        assert_eq!(
            v,
            Some(Value::Dist(Dist::Triangular {
                min: 0.1,
                mode: 0.2,
                max: 0.5
            }))
        );
        assert_eq!(
            s1.param("task", "ControlParameters/TriggerCount")
                .and_then(|p| value(p, TimeUnit::Hours, &mut notes, None)),
            Some(Value::Number(3.0))
        );
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn inheritance_overrides_only_what_it_redefines() {
        let d = data(TWO_SCENARIOS);
        let mut notes = vec![];
        let s2 = read_scenario(&d, Some("S2"), &mut notes).unwrap_or_default();
        assert_eq!(s2.settings.id, "S2");
        assert_eq!(s2.settings.replication, Some(5), "inherited");
        let pt = s2
            .param("task", "TimeParameters/ProcessingTime")
            .and_then(|p| value(p, TimeUnit::Hours, &mut notes, None));
        assert_eq!(pt, Some(Value::Number(0.5)), "overridden");
        assert!(
            s2.param("task", "ControlParameters/TriggerCount").is_some(),
            "inherited"
        );
        assert!(read_scenario(&d, Some("nope"), &mut notes).is_none());
        // Adding only a result request keeps the inherited value (as the technical support
        // reference model's resource scenario does).
        let s3 = read_scenario(&d, Some("S3"), &mut notes).unwrap_or_default();
        let pt = s3
            .param("task", "TimeParameters/ProcessingTime")
            .and_then(|p| value(p, TimeUnit::Hours, &mut notes, None));
        assert_eq!(pt, Some(Value::Number(0.5)));
    }

    #[test]
    fn bpsim_1_and_odd_spellings() {
        let d = data(
            r#"<BPSimData xmlns="http://www.bpsim.org/schemas/1.0"><Scenario id="a">
            <ElementParameters elementRef="t"><TimeParameters><SetupTime><FloatingParameter value="2"/></SetupTime></TimeParameters></ElementParameters>
            </Scenario></BPSimData>"#,
        );
        let mut notes = vec![];
        let s = read_scenario(&d, None, &mut notes).unwrap_or_default();
        assert!(s.version_1);
        assert_eq!(
            s.settings.base_time_unit,
            TimeUnit::Minutes,
            "BPSim's default"
        );
        assert!(s.param("t", "TimeParameters/SetUpTime").is_some());
    }

    #[test]
    fn every_distribution_reads_and_scales() {
        let cases = [
            (
                r#"<b:NegativeExponentialDistribution mean="2" timeUnit="hour"/>"#,
                Some(Dist::Exponential { mean: 120.0 }),
            ),
            (
                r#"<b:UniformDistribution min="1" max="3"/>"#,
                Some(Dist::Uniform { min: 1.0, max: 3.0 }),
            ),
            (
                r#"<b:NormalDistribution mean="5" standardDeviation="1"/>"#,
                Some(Dist::Normal {
                    mean: 5.0,
                    std_dev: 1.0,
                }),
            ),
            (
                r#"<b:LogNormalDistribution mean="5" standardDeviation="1"/>"#,
                Some(Dist::LogNormal {
                    mean: 5.0,
                    std_dev: 1.0,
                }),
            ),
            (
                r#"<b:TruncatedNormalDistribution max="1000" mean="2" min="1" standardDeviation="1"/>"#,
                Some(Dist::TruncatedNormal {
                    mean: 2.0,
                    std_dev: 1.0,
                    min: 1.0,
                    max: 1000.0,
                }),
            ),
            (
                r#"<b:GammaDistribution shape="2" scale="3" timeUnit="s"/>"#,
                Some(Dist::Gamma {
                    shape: 2.0,
                    scale: 0.05,
                }),
            ),
            (
                r#"<b:BetaDistribution shape="2" scale="3"/>"#,
                Some(Dist::Beta {
                    shape: 2.0,
                    scale: 3.0,
                }),
            ),
            (
                r#"<b:WeibullDistribution shape="1.5" scale="4"/>"#,
                Some(Dist::Weibull {
                    shape: 1.5,
                    scale: 4.0,
                }),
            ),
            (
                r#"<b:ErlangDistribution mean="6" k="3"/>"#,
                Some(Dist::Erlang { mean: 6.0, k: 3 }),
            ),
            (r#"<b:ErlangDistribution mean="6" k="2.5"/>"#, None),
            (
                r#"<b:PoissonDistribution mean="4"/>"#,
                Some(Dist::Poisson { mean: 4.0 }),
            ),
            (r#"<b:PoissonDistribution mean="4" timeUnit="hour"/>"#, None),
            (
                r#"<b:BinomialDistribution probability="0.3" trials="10"/>"#,
                Some(Dist::Binomial {
                    probability: 0.3,
                    trials: 10,
                }),
            ),
            (
                r#"<b:UserDistribution discrete="true"><b:UserDistributionDataPoint probability="0.25"><b:FloatingParameter value="1"/></b:UserDistributionDataPoint><b:UserDistributionDataPoint probability="0.75"><b:NumericParameter value="4"/></b:UserDistributionDataPoint></b:UserDistribution>"#,
                Some(Dist::User {
                    points: vec![(0.25, 1.0), (0.75, 4.0)],
                    discrete: true,
                }),
            ),
            (r#"<b:TriangularDistribution min="1" max="3"/>"#, None),
        ];
        for (xml, want) in cases {
            let p = data(&format!(r#"<b:P xmlns:b="{NS2}">{xml}</b:P>"#));
            let mut notes = vec![];
            let got = value(&p, TimeUnit::Minutes, &mut notes, Some("x"));
            match want {
                Some(d) => assert_eq!(got, Some(Value::Dist(d)), "{xml}"),
                None => assert!(
                    got.is_none() && notes.iter().any(|n| n.severity == crate::Severity::Error),
                    "{xml}: {got:?} {notes:?}"
                ),
            }
        }
    }

    #[test]
    fn result_and_calendar_values_are_skipped() {
        let p = data(&format!(
            r#"<b:P xmlns:b="{NS2}"><b:NumericParameter value="9" result="count"/><b:FloatingParameter value="2" validFor="C1"/><b:FloatingParameter value="1"/></b:P>"#
        ));
        let mut notes = vec![];
        assert_eq!(
            value(&p, TimeUnit::Minutes, &mut notes, None),
            Some(Value::Number(1.0))
        );
        assert_eq!(notes.len(), 1, "{notes:?}");
        let mut notes = vec![];
        assert_eq!(
            default_value(&p, TimeUnit::Minutes, &mut notes, None),
            Some(Value::Number(1.0))
        );
        assert_eq!(
            timed_values(&p, TimeUnit::Minutes, &mut notes, None),
            [("C1".to_owned(), Value::Number(2.0))]
        );
        assert!(notes.is_empty(), "{notes:?}");
    }
}
