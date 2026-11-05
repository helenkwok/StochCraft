//! The JSON forms of models, distributions, diagnostics and results: what commands take and
//! return. They are the engine's own (the files on disk stay BPSim on BPMN or XPDL); the model
//! crates below stay free of serialisation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_blocks::{ExperimentSummary, Metric};
use stochcraft_model::{
    Connection, Diagnostic, Dist, Element, ElementId, ElementKind, Experiment, Model, Property,
    PropertyType, PropertyValue, Severity, TimeUnit,
};

/// A probability distribution. Times are in the experiment's time unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DistSpec {
    /// Always `value`.
    Constant { value: f64 },
    /// Exponential (BPSim `NegativeExponentialDistribution`) with this mean.
    Exponential { mean: f64 },
    /// Uniform on [min, max).
    Uniform { min: f64, max: f64 },
    /// Triangular on [min, max], peaking at `mode`.
    Triangular { min: f64, mode: f64, max: f64 },
    /// Normal.
    Normal { mean: f64, std_dev: f64 },
    /// Normal restricted to [min, max].
    TruncatedNormal {
        mean: f64,
        std_dev: f64,
        min: f64,
        max: f64,
    },
    /// Log-normal with this mean and standard deviation (of the values, not their logarithm).
    LogNormal { mean: f64, std_dev: f64 },
    /// Gamma with shape k and scale θ (mean kθ).
    Gamma { shape: f64, scale: f64 },
    /// Beta on [0, 1] with shape parameters α = `shape`, β = `scale` (BPSim's names).
    Beta { shape: f64, scale: f64 },
    /// The sum of `k` exponentials with overall mean `mean`.
    Erlang { mean: f64, k: u64 },
    /// Weibull with shape k and scale λ.
    Weibull { shape: f64, scale: f64 },
    /// Poisson count with this mean.
    Poisson { mean: f64 },
    /// Binomial count of successes in `trials`.
    Binomial { probability: f64, trials: u64 },
    /// Data points. `discrete`: one of the values with its probability; otherwise a cumulative
    /// curve through the points, interpolated linearly.
    User {
        points: Vec<UserPoint>,
        #[serde(default)]
        discrete: bool,
    },
}

/// One point of a user distribution.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserPoint {
    pub probability: f64,
    pub value: f64,
}

impl From<&Dist> for DistSpec {
    fn from(d: &Dist) -> Self {
        match d.clone() {
            Dist::Constant(value) => DistSpec::Constant { value },
            Dist::Exponential { mean } => DistSpec::Exponential { mean },
            Dist::Uniform { min, max } => DistSpec::Uniform { min, max },
            Dist::Triangular { min, mode, max } => DistSpec::Triangular { min, mode, max },
            Dist::Normal { mean, std_dev } => DistSpec::Normal { mean, std_dev },
            Dist::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            } => DistSpec::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            },
            Dist::LogNormal { mean, std_dev } => DistSpec::LogNormal { mean, std_dev },
            Dist::Gamma { shape, scale } => DistSpec::Gamma { shape, scale },
            Dist::Beta { shape, scale } => DistSpec::Beta { shape, scale },
            Dist::Erlang { mean, k } => DistSpec::Erlang { mean, k },
            Dist::Weibull { shape, scale } => DistSpec::Weibull { shape, scale },
            Dist::Poisson { mean } => DistSpec::Poisson { mean },
            Dist::Binomial {
                probability,
                trials,
            } => DistSpec::Binomial {
                probability,
                trials,
            },
            Dist::User { points, discrete } => DistSpec::User {
                points: points
                    .into_iter()
                    .map(|(probability, value)| UserPoint { probability, value })
                    .collect(),
                discrete,
            },
        }
    }
}

impl From<DistSpec> for Dist {
    fn from(d: DistSpec) -> Self {
        match d {
            DistSpec::Constant { value } => Dist::Constant(value),
            DistSpec::Exponential { mean } => Dist::Exponential { mean },
            DistSpec::Uniform { min, max } => Dist::Uniform { min, max },
            DistSpec::Triangular { min, mode, max } => Dist::Triangular { min, mode, max },
            DistSpec::Normal { mean, std_dev } => Dist::Normal { mean, std_dev },
            DistSpec::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            } => Dist::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            },
            DistSpec::LogNormal { mean, std_dev } => Dist::LogNormal { mean, std_dev },
            DistSpec::Gamma { shape, scale } => Dist::Gamma { shape, scale },
            DistSpec::Beta { shape, scale } => Dist::Beta { shape, scale },
            DistSpec::Erlang { mean, k } => Dist::Erlang { mean, k },
            DistSpec::Weibull { shape, scale } => Dist::Weibull { shape, scale },
            DistSpec::Poisson { mean } => Dist::Poisson { mean },
            DistSpec::Binomial {
                probability,
                trials,
            } => Dist::Binomial {
                probability,
                trials,
            },
            DistSpec::User { points, discrete } => Dist::User {
                points: points.iter().map(|p| (p.probability, p.value)).collect(),
                discrete,
            },
        }
    }
}

fn exponential_1() -> DistSpec {
    DistSpec::Exponential { mean: 1.0 }
}

fn constant_1() -> DistSpec {
    DistSpec::Constant { value: 1.0 }
}

fn one() -> u32 {
    1
}

/// What an element is and its parameters. Missing parameters take their defaults: exponential
/// times with mean 1, one server unit, a delay of 1, no arrival limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KindSpec {
    /// Creates entities (a BPMN start event).
    Source {
        /// Time between arrivals.
        #[serde(default = "exponential_1")]
        interarrival: DistSpec,
        /// Stop after this many arrivals; none means no limit.
        #[serde(default)]
        limit: Option<u64>,
    },
    /// Identical units serving one entity each, first come first served (a task with a
    /// resource).
    Server {
        #[serde(default = "one")]
        capacity: u32,
        /// Time to serve one entity.
        #[serde(default = "exponential_1")]
        service: DistSpec,
    },
    /// Holds each entity for a time, with no capacity limit (a task with no resource).
    Delay {
        #[serde(default = "constant_1")]
        duration: DistSpec,
    },
    /// Removes entities (a BPMN end event).
    Sink,
    /// Sends each entity down one outgoing connection at once (a BPMN exclusive gateway): the
    /// first whose condition holds if any has one, else drawn by the connections'
    /// probabilities (`flow.set`).
    ExclusiveGateway {
        /// The outgoing connection taken when no condition holds; without one, such an entity
        /// leaves the model.
        #[serde(default)]
        default: Option<String>,
    },
    /// Copies each entity down every outgoing connection at once (a BPMN parallel gateway);
    /// with several incoming connections, first waits for an entity on each (a join).
    ParallelGateway,
}

impl From<&ElementKind> for KindSpec {
    fn from(k: &ElementKind) -> Self {
        match k {
            ElementKind::Source {
                interarrival,
                limit,
            } => KindSpec::Source {
                interarrival: interarrival.into(),
                limit: *limit,
            },
            ElementKind::Server { capacity, service } => KindSpec::Server {
                capacity: *capacity,
                service: service.into(),
            },
            ElementKind::Delay { duration } => KindSpec::Delay {
                duration: duration.into(),
            },
            ElementKind::Sink => KindSpec::Sink,
            ElementKind::ExclusiveGateway { default } => KindSpec::ExclusiveGateway {
                default: default.as_ref().map(ToString::to_string),
            },
            ElementKind::ParallelGateway => KindSpec::ParallelGateway,
        }
    }
}

impl From<KindSpec> for ElementKind {
    fn from(k: KindSpec) -> Self {
        match k {
            KindSpec::Source {
                interarrival,
                limit,
            } => ElementKind::Source {
                interarrival: interarrival.into(),
                limit,
            },
            KindSpec::Server { capacity, service } => ElementKind::Server {
                capacity,
                service: service.into(),
            },
            KindSpec::Delay { duration } => ElementKind::Delay {
                duration: duration.into(),
            },
            KindSpec::Sink => ElementKind::Sink,
            KindSpec::ExclusiveGateway { default } => ElementKind::ExclusiveGateway {
                default: default.as_deref().map(ElementId::unchecked),
            },
            KindSpec::ParallelGateway => ElementKind::ParallelGateway,
        }
    }
}

/// BPSim's time units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TimeUnitSpec {
    #[serde(rename = "ms")]
    Milliseconds,
    #[serde(rename = "s")]
    Seconds,
    #[serde(rename = "min")]
    Minutes,
    #[serde(rename = "hour")]
    Hours,
    #[serde(rename = "day")]
    Days,
    #[serde(rename = "year")]
    Years,
}

impl From<TimeUnit> for TimeUnitSpec {
    fn from(t: TimeUnit) -> Self {
        match t {
            TimeUnit::Milliseconds => TimeUnitSpec::Milliseconds,
            TimeUnit::Seconds => TimeUnitSpec::Seconds,
            TimeUnit::Minutes => TimeUnitSpec::Minutes,
            TimeUnit::Hours => TimeUnitSpec::Hours,
            TimeUnit::Days => TimeUnitSpec::Days,
            TimeUnit::Years => TimeUnitSpec::Years,
        }
    }
}

impl From<TimeUnitSpec> for TimeUnit {
    fn from(t: TimeUnitSpec) -> Self {
        match t {
            TimeUnitSpec::Milliseconds => TimeUnit::Milliseconds,
            TimeUnitSpec::Seconds => TimeUnit::Seconds,
            TimeUnitSpec::Minutes => TimeUnit::Minutes,
            TimeUnitSpec::Hours => TimeUnit::Hours,
            TimeUnitSpec::Days => TimeUnit::Days,
            TimeUnitSpec::Years => TimeUnit::Years,
        }
    }
}

/// BPSim's property types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PropertyTypeSpec {
    #[serde(rename = "string")]
    String,
    #[serde(rename = "boolean")]
    Boolean,
    /// A whole number.
    #[serde(rename = "long")]
    Long,
    #[serde(rename = "double")]
    Double,
    /// A length of time in the experiment's time unit.
    #[serde(rename = "duration")]
    Duration,
    /// A date and time, as text.
    #[serde(rename = "dateTime")]
    DateTime,
}

impl From<PropertyType> for PropertyTypeSpec {
    fn from(t: PropertyType) -> Self {
        match t {
            PropertyType::String => PropertyTypeSpec::String,
            PropertyType::Boolean => PropertyTypeSpec::Boolean,
            PropertyType::Long => PropertyTypeSpec::Long,
            PropertyType::Double => PropertyTypeSpec::Double,
            PropertyType::Duration => PropertyTypeSpec::Duration,
            PropertyType::DateTime => PropertyTypeSpec::DateTime,
        }
    }
}

impl From<PropertyTypeSpec> for PropertyType {
    fn from(t: PropertyTypeSpec) -> Self {
        match t {
            PropertyTypeSpec::String => PropertyType::String,
            PropertyTypeSpec::Boolean => PropertyType::Boolean,
            PropertyTypeSpec::Long => PropertyType::Long,
            PropertyTypeSpec::Double => PropertyType::Double,
            PropertyTypeSpec::Duration => PropertyType::Duration,
            PropertyTypeSpec::DateTime => PropertyType::DateTime,
        }
    }
}

/// What a property is set to, one of: `{"number": 3}`, `{"boolean": true}`, `{"text": "gold"}`,
/// `{"random": <distribution>}` (drawn for each entity) or `{"expression": "getProperty('n') - 1"}`
/// (BPSim's XPath, from the entity's properties as it arrived).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PropertyValueSpec {
    Number(f64),
    Boolean(bool),
    Text(String),
    Random(DistSpec),
    Expression(String),
}

impl From<&PropertyValue> for PropertyValueSpec {
    fn from(v: &PropertyValue) -> Self {
        match v {
            PropertyValue::Number(n) => PropertyValueSpec::Number(*n),
            PropertyValue::Boolean(b) => PropertyValueSpec::Boolean(*b),
            PropertyValue::Text(t) => PropertyValueSpec::Text(t.clone()),
            PropertyValue::Random(d) => PropertyValueSpec::Random(d.into()),
            PropertyValue::Expression(x) => PropertyValueSpec::Expression(x.clone()),
        }
    }
}

impl From<PropertyValueSpec> for PropertyValue {
    /// A constant "distribution" is a fixed number, as model files hold it.
    fn from(v: PropertyValueSpec) -> Self {
        match v {
            PropertyValueSpec::Number(n)
            | PropertyValueSpec::Random(DistSpec::Constant { value: n }) => {
                PropertyValue::Number(n)
            }
            PropertyValueSpec::Boolean(b) => PropertyValue::Boolean(b),
            PropertyValueSpec::Text(t) => PropertyValue::Text(t),
            PropertyValueSpec::Random(d) => PropertyValue::Random(d.into()),
            PropertyValueSpec::Expression(x) => PropertyValue::Expression(x),
        }
    }
}

/// A property an element sets on every entity that enters it (for a source: that it creates).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PropertyDoc {
    pub name: String,
    /// The declared type; without one, the type declared elsewhere in the model or else the type
    /// of the property's first value.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<PropertyTypeSpec>,
    pub value: PropertyValueSpec,
}

impl From<&Property> for PropertyDoc {
    fn from(p: &Property) -> Self {
        Self {
            name: p.name.clone(),
            ty: p.ty.map(Into::into),
            value: (&p.value).into(),
        }
    }
}

impl From<PropertyDoc> for Property {
    fn from(p: PropertyDoc) -> Self {
        Self {
            name: p.name,
            ty: p.ty.map(Into::into),
            value: p.value.into(),
        }
    }
}

/// Properties in name order: the order model files read them in (an element's assignments are
/// made together, so the order means nothing).
pub fn sorted_properties(props: impl IntoIterator<Item = Property>) -> Vec<Property> {
    let mut v: Vec<Property> = props.into_iter().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// An element of a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ElementDoc {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(flatten)]
    pub kind: KindSpec,
    /// Properties the element sets on entities; left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyDoc>,
}

/// A connection from one element to another (a BPMN sequence flow).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDoc {
    pub id: String,
    pub from: String,
    pub to: String,
    /// Out of an exclusive gateway: the chance an entity takes this connection; left out when
    /// not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability: Option<f64>,
    /// Out of an exclusive gateway: a BPSim expression that must hold to take it; left out
    /// when not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// How to run a model. `duration` excludes the warm-up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExperimentDoc {
    pub time_unit: TimeUnitSpec,
    pub warmup: f64,
    pub duration: f64,
    pub replications: u32,
    pub seed: u64,
}

impl From<&Experiment> for ExperimentDoc {
    fn from(x: &Experiment) -> Self {
        Self {
            time_unit: x.time_unit.into(),
            warmup: x.warmup,
            duration: x.duration,
            replications: x.replications,
            seed: x.seed,
        }
    }
}

impl From<ExperimentDoc> for Experiment {
    fn from(x: ExperimentDoc) -> Self {
        Self {
            time_unit: x.time_unit.into(),
            warmup: x.warmup,
            duration: x.duration,
            replications: x.replications,
            seed: x.seed,
        }
    }
}

fn default_experiment() -> ExperimentDoc {
    (&Experiment::default()).into()
}

/// A whole model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelDoc {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub elements: Vec<ElementDoc>,
    #[serde(default)]
    pub connections: Vec<ConnectionDoc>,
    #[serde(default = "default_experiment")]
    pub experiment: ExperimentDoc,
}

impl From<&Model> for ModelDoc {
    fn from(m: &Model) -> Self {
        Self {
            name: m.name.clone(),
            elements: m
                .elements
                .iter()
                .map(|e| ElementDoc {
                    id: e.id.to_string(),
                    name: e.name.clone(),
                    kind: (&e.kind).into(),
                    properties: e.properties.iter().map(Into::into).collect(),
                })
                .collect(),
            connections: m
                .connections
                .iter()
                .map(|c| ConnectionDoc {
                    id: c.id.to_string(),
                    from: c.from.to_string(),
                    to: c.to.to_string(),
                    probability: c.probability,
                    condition: c.condition.clone(),
                })
                .collect(),
            experiment: (&m.experiment).into(),
        }
    }
}

impl From<ModelDoc> for Model {
    /// Ids are kept as written; [`Model::validate`] reports invalid ones.
    fn from(m: ModelDoc) -> Self {
        Self {
            name: m.name,
            elements: m
                .elements
                .into_iter()
                .map(|e| Element {
                    id: ElementId::unchecked(&e.id),
                    name: e.name,
                    kind: e.kind.into(),
                    properties: sorted_properties(e.properties.into_iter().map(Into::into)),
                })
                .collect(),
            connections: m
                .connections
                .into_iter()
                .map(|c| Connection {
                    id: ElementId::unchecked(&c.id),
                    from: ElementId::unchecked(&c.from),
                    to: ElementId::unchecked(&c.to),
                    probability: c.probability,
                    condition: c.condition,
                })
                .collect(),
            experiment: m.experiment.into(),
        }
    }
}

/// A model as JSON.
pub fn model(m: &Model) -> Value {
    serde_json::to_value(ModelDoc::from(m)).unwrap_or(Value::Null)
}

/// Diagnostics as JSON: `severity`, `at`, a stable `code` and a `message`.
pub fn diagnostics(ds: &[Diagnostic]) -> Value {
    ds.iter()
        .map(|d| {
            json!({
                "severity": match d.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                },
                "at": d.at.as_ref().map(ElementId::as_str),
                "code": d.problem.code(),
                "message": d.to_string(),
            })
        })
        .collect()
}

/// An experiment's results as JSON: per element, each metric's mean, 95% half-width, standard
/// deviation and number of replications.
pub fn results(model: &Model, replications: u64, seed: u64, summary: &ExperimentSummary) -> Value {
    let x = &model.experiment;
    let elements: Vec<Value> = summary
        .elements
        .iter()
        .map(|e| {
            let metrics: serde_json::Map<String, Value> = e
                .metrics
                .iter()
                .map(|(m, s)| {
                    (
                        m.name().to_owned(),
                        json!({
                            "mean": s.mean,
                            "half_width": s.half_width,
                            "std_dev": s.std_dev,
                            "n": s.n,
                        }),
                    )
                })
                .collect();
            let mut v =
                json!({ "id": e.id.as_str(), "name": e.name, "kind": e.kind, "metrics": metrics });
            // The mean value the element assigned to each property; left out when it assigns
            // none, so results of models without properties are as before.
            if !e.properties.is_empty() {
                let props: serde_json::Map<String, Value> = e
                    .properties
                    .iter()
                    .map(|(name, s)| {
                        (
                            name.clone(),
                            json!({
                                "mean": s.mean,
                                "half_width": s.half_width,
                                "std_dev": s.std_dev,
                                "n": s.n,
                            }),
                        )
                    })
                    .collect();
                v["properties"] = Value::Object(props);
            }
            v
        })
        .collect();
    json!({
        "replications": replications,
        "seed": seed,
        "time_unit": TimeUnitSpec::from(x.time_unit),
        "warmup": x.warmup,
        "duration": x.duration,
        "metrics": Metric::ALL.iter().map(|m| m.name()).collect::<Vec<_>>(),
        "elements": elements,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_distribution_round_trips() {
        let all = [
            Dist::Constant(2.0),
            Dist::Exponential { mean: 1.5 },
            Dist::Uniform { min: 1.0, max: 2.0 },
            Dist::Triangular {
                min: 1.0,
                mode: 2.0,
                max: 4.0,
            },
            Dist::Normal {
                mean: 3.0,
                std_dev: 0.5,
            },
            Dist::TruncatedNormal {
                mean: 3.0,
                std_dev: 0.5,
                min: 2.0,
                max: 4.0,
            },
            Dist::LogNormal {
                mean: 3.0,
                std_dev: 1.0,
            },
            Dist::Gamma {
                shape: 2.0,
                scale: 0.5,
            },
            Dist::Beta {
                shape: 2.0,
                scale: 3.0,
            },
            Dist::Erlang { mean: 4.0, k: 3 },
            Dist::Weibull {
                shape: 1.5,
                scale: 2.0,
            },
            Dist::Poisson { mean: 4.0 },
            Dist::Binomial {
                probability: 0.3,
                trials: 10,
            },
            Dist::User {
                points: vec![(0.25, 1.0), (0.75, 2.0)],
                discrete: true,
            },
        ];
        for d in all {
            let v = serde_json::to_value(DistSpec::from(&d)).unwrap_or(Value::Null);
            let back: Option<DistSpec> = serde_json::from_value(v.clone()).ok();
            assert_eq!(back.map(Dist::from), Some(d), "{v}");
        }
        let e: Result<DistSpec, _> =
            serde_json::from_value(json!({"type": "exponential", "mean": 1, "rate": 2}));
        assert!(e.is_err(), "unknown fields are refused");
    }

    #[test]
    fn kinds_take_defaults() {
        let k: Option<KindSpec> = serde_json::from_value(json!({"kind": "server"})).ok();
        assert_eq!(
            k,
            Some(KindSpec::Server {
                capacity: 1,
                service: DistSpec::Exponential { mean: 1.0 }
            })
        );
    }
}
