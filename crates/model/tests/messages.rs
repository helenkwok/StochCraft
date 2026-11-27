//! Validation messages (issue #145): each names its block by name, says "block", shows no field
//! key and says what to do. One diagnostic for every problem, on a model whose names differ from
//! its ids, so a message that showed an id would be caught.

use std::collections::BTreeSet;

use stochcraft_model::expr::ExprError;
use stochcraft_model::messages::{LABEL_ARGS, TEMPLATES, fill, labels};
use stochcraft_model::{
    Behaviour, Calendar, Connection, Diagnostic, Element, ElementId, ElementKind, ExperimentField,
    FitError, IgnoredWhat, IgnoredWhy, Model, Problem, PropertyFault, PropertyType, ResourcePool,
    Severity, StartingFault, StopFault,
};
use stochcraft_random::{DistError, Requirement};

fn id(s: &str) -> ElementId {
    ElementId::unchecked(s)
}

/// Blocks named unlike their ids: `src` is "Front desk", `task` "Serve customer", `gate`
/// "Check gate"; flow `c1` goes from `src` to `task`; a pool and a calendar have names too.
fn model() -> Model {
    let block = |i: &str, name: &str| Element::new(id(i), name, ElementKind::ParallelGateway);
    Model {
        name: "Clinic".into(),
        elements: vec![
            block("n1", "Front desk"),
            block("n2", "Serve customer"),
            block("n3", "Check gate"),
            block("n4", "Exit door"),
        ],
        connections: vec![
            Connection::new(id("c1"), id("n1"), id("n2")),
            Connection::new(id("c2"), id("n2"), id("n4")),
        ],
        resources: vec![ResourcePool::new(
            id("p1"),
            "Night nurses",
            2,
            vec!["nurse".into()],
        )],
        calendars: vec![Calendar {
            id: id("k1"),
            name: "Weekday shifts".into(),
            events: Vec::new(),
        }],
        ..Model::default()
    }
}

fn found(at: Option<&str>, problem: Problem) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        at: at.map(id),
        problem,
    }
}

/// One diagnostic per problem, and per way a problem reads differently.
fn samples() -> Vec<Diagnostic> {
    let task = Some("n2");
    let flow = Some("c1");
    let expr_error = ExprError {
        position: 24,
        message: "the expression ends too early".into(),
    };
    let dist = |parameter: &'static str, error: DistError| {
        found(task, Problem::BadDistribution { parameter, error })
    };
    let behaviour = |at: Option<&str>, found_it: Behaviour, replications: u64, of: u64| {
        found(
            at,
            Problem::Behaviour {
                found: found_it,
                replications,
                of,
            },
        )
    };
    vec![
        found(
            None,
            Problem::InvalidId {
                id: "9 lives".into(),
            },
        ),
        found(task, Problem::DuplicateId { id: id("n2") }),
        found(
            flow,
            Problem::UnknownElement {
                connection: id("c1"),
                missing: id("ghost"),
            },
        ),
        found(
            flow,
            Problem::IntoSource {
                connection: id("c1"),
            },
        ),
        found(
            flow,
            Problem::OutOfSink {
                connection: id("c1"),
            },
        ),
        found(
            flow,
            Problem::SelfLoop {
                connection: id("c1"),
            },
        ),
        found(task, Problem::NoOutput),
        found(task, Problem::SeveralOutputs { count: 3 }),
        dist("service", DistError::NotFinite("mean")),
        dist(
            "interarrival",
            DistError::OutOfRange {
                parameter: "mean",
                requirement: Requirement::AboveZero,
            },
        ),
        dist(
            "time_to_failure",
            DistError::OutOfRange {
                parameter: "standardDeviation",
                requirement: Requirement::ZeroOrMore,
            },
        ),
        dist(
            "repair",
            DistError::OutOfRange {
                parameter: "k",
                requirement: Requirement::OneOrMore,
            },
        ),
        dist(
            "patience",
            DistError::OutOfRange {
                parameter: "probability",
                requirement: Requirement::UnitInterval,
            },
        ),
        dist("planned_run_time", DistError::BadBounds),
        dist("service", DistError::BadRange),
        dist("property value", DistError::EmptyUserDistribution),
        found(task, Problem::ZeroCapacity),
        found(task, Problem::NoTimeBetweenArrivals),
        found(None, Problem::NoSource),
        found(None, Problem::NoSink),
        found(task, Problem::Unreachable),
        found(task, Problem::NoPathToSink),
        found(task, Problem::BadPower { state: "off_shift" }),
        found(
            None,
            Problem::BadExperiment {
                field: ExperimentField::Warmup,
            },
        ),
        found(
            None,
            Problem::BadExperiment {
                field: ExperimentField::Duration,
            },
        ),
        found(
            None,
            Problem::BadExperiment {
                field: ExperimentField::Replications,
            },
        ),
        found(None, Problem::BadLayoutScale),
        found(
            task,
            Problem::BadProperty {
                name: String::new(),
                reason: PropertyFault::NoName,
            },
        ),
        found(
            task,
            Problem::BadProperty {
                name: "tier".into(),
                reason: PropertyFault::SetTwice,
            },
        ),
        found(
            task,
            Problem::BadProperty {
                name: "tier".into(),
                reason: PropertyFault::BadShares,
            },
        ),
        found(
            task,
            Problem::PropertyTypeConflict {
                name: "tier".into(),
                first: PropertyType::Long,
                second: PropertyType::String,
            },
        ),
        found(
            task,
            Problem::PropertyValueType {
                name: "tier".into(),
                ty: PropertyType::Boolean,
                value: "a number",
            },
        ),
        found(
            task,
            Problem::BadExpression {
                name: "tier".into(),
                expression: "getProperty('x') +".into(),
                error: expr_error.clone(),
            },
        ),
        found(
            task,
            Problem::UnknownProperty {
                name: "tier".into(),
            },
        ),
        found(task, Problem::ChangesWithoutProperty),
        found(task, Problem::TimesByValueWithoutProperty),
        found(
            task,
            Problem::DuplicateTimeByValue {
                value: "gold".into(),
            },
        ),
        found(
            task,
            Problem::TimeByValueNegative {
                value: "gold".into(),
            },
        ),
        found(task, Problem::TimeForUnknownPool { id: id("p9") }),
        found(task, Problem::DuplicateTimeByPool { id: id("p1") }),
        found(task, Problem::TimeForPoolNotNeeded { id: id("p1") }),
        found(task, Problem::ScrapNotAgainstServer { id: "n3".into() }),
        found(task, Problem::PlannedRunTimeNotPositive),
        found(task, Problem::PatienceNegative),
        found(
            flow,
            Problem::AbandonedWithoutPatience {
                connection: id("c1"),
            },
        ),
        found(task, Problem::SeveralAbandonedWays { count: 2 }),
        found(
            flow,
            Problem::TurnedAwayWithoutRoom {
                connection: id("c1"),
            },
        ),
        found(task, Problem::SeveralTurnedAwayWays { count: 2 }),
        found(
            flow,
            Problem::TurnedAwayAndAbandoned {
                connection: id("c1"),
            },
        ),
        found(task, Problem::BalkingProbability),
        found(task, Problem::BalkingRowsOrder),
        found(
            flow,
            Problem::BalkedWithoutBalking {
                connection: id("c1"),
            },
        ),
        found(task, Problem::SeveralBalkedWays { count: 2 }),
        found(
            flow,
            Problem::BalkedAndOtherWay {
                connection: id("c1"),
            },
        ),
        found(task, Problem::QueueLimitWithoutRoom),
        found(task, Problem::QueueByShiftEmpty),
        found(task, Problem::HealthCheckLimitNotValid),
        found(task, Problem::HealthCheckIntervalNotPositive),
        found(task, Problem::DemandNotPositive),
        found(task, Problem::ShiftWithoutCalendar),
        found(task, Problem::DemandCalendarUnknown { id: id("nowhere") }),
        found(task, Problem::TimeInSystemWithinNotPositive),
        found(
            task,
            Problem::TimeInSystemCalendarUnknown { id: id("nowhere") },
        ),
        found(flow, Problem::BadProbability { value: 1.5 }),
        found(Some("n3"), Problem::ProbabilitySum { sum: 0.99 }),
        found(
            flow,
            Problem::BadCondition {
                expression: "getProperty('x') >".into(),
                error: expr_error.clone(),
            },
        ),
        found(
            flow,
            Problem::RoutingIgnored {
                what: IgnoredWhat::Condition,
                reason: IgnoredWhy::NotDecision,
            },
        ),
        found(
            flow,
            Problem::RoutingIgnored {
                what: IgnoredWhat::Probability,
                reason: IgnoredWhy::NotDecision,
            },
        ),
        found(
            flow,
            Problem::RoutingIgnored {
                what: IgnoredWhat::Condition,
                reason: IgnoredWhy::ShortestQueue,
            },
        ),
        found(
            flow,
            Problem::RoutingIgnored {
                what: IgnoredWhat::Probability,
                reason: IgnoredWhy::ShortestQueue,
            },
        ),
        found(
            flow,
            Problem::RoutingIgnored {
                what: IgnoredWhat::Default,
                reason: IgnoredWhy::ShortestQueue,
            },
        ),
        found(
            flow,
            Problem::RoutingIgnored {
                what: IgnoredWhat::Probability,
                reason: IgnoredWhy::ByCondition,
            },
        ),
        found(
            Some("n3"),
            Problem::BadDefault {
                connection: id("c2"),
            },
        ),
        found(Some("n3"), Problem::NoDefault),
        found(Some("n3"), Problem::DivertNoDefault),
        found(
            Some("n3"),
            Problem::JoinNeverCompletes {
                connection: id("c1"),
            },
        ),
        found(task, Problem::BufferBehindGateway { gateway: id("n3") }),
        found(
            task,
            Problem::BlockingCycle {
                through: vec![id("n1"), id("n2")],
            },
        ),
        found(Some("p1"), Problem::EmptyPool),
        found(task, Problem::NoNeeds),
        found(task, Problem::UnknownResource { id: id("nobody") }),
        found(
            task,
            Problem::NoPoolWithRoles {
                roles: vec!["surgeon".into(), "porter".into()],
            },
        ),
        found(
            task,
            Problem::NeedNeverMet {
                needed: 3,
                available: 2,
            },
        ),
        found(task, Problem::BadPriority { value: f64::NAN }),
        found(Some("p1"), Problem::UnusedPool),
        found(task, Problem::UnknownCalendar { id: id("nowhere") }),
        found(Some("k1"), Problem::EmptyCalendar),
        found(task, Problem::TimedIgnored),
        found(task, Problem::ZeroBatchSize),
        found(task, Problem::BadConveyorValue { field: "length" }),
        found(task, Problem::BadConveyorValue { field: "gap" }),
        found(task, Problem::BadConveyorValue { field: "angle" }),
        found(
            task,
            Problem::ConveyorTooShort {
                length: 1.0,
                pitch: 2.0,
            },
        ),
        found(
            task,
            Problem::ConveyorLengthMismatch {
                length: 10.0,
                path_length: 12.5,
            },
        ),
        found(
            Some("n3"),
            Problem::PlaceOffPath {
                conveyor: id("n2"),
                s: 12.0,
                length: 10.0,
            },
        ),
        found(
            Some("n3"),
            Problem::PointsOutOfOrder {
                conveyor: id("n2"),
                s: 2.0,
                before: id("n4"),
                before_s: 5.0,
            },
        ),
        found(flow, Problem::BadJunctionTime { time: -1.0 }),
        found(task, Problem::TransportNotSimulated),
        found(
            task,
            Problem::DecisionUnusable {
                file: "rules.dmn".into(),
                decision: "chute".into(),
                reason: "the file was not read.".into(),
            },
        ),
        found(
            task,
            Problem::DecisionOverlap {
                decision: "Chute".into(),
                first: 2,
                second: 3,
                example: "size = 50".into(),
            },
        ),
        found(
            task,
            Problem::DecisionGap {
                decision: "Chute".into(),
                example: "size = 5.5 and class = \"other\"".into(),
            },
        ),
        found(
            task,
            Problem::DecisionRoute {
                decision: "Chute".into(),
                value: "north".into(),
            },
        ),
        found(
            task,
            Problem::DecisionUnchecked {
                decision: "Chute".into(),
            },
        ),
        found(
            task,
            Problem::TransportPool {
                pool: id("p1"),
                exists: false,
            },
        ),
        found(
            task,
            Problem::TransportPool {
                pool: id("p1"),
                exists: true,
            },
        ),
        found(
            Some("p1"),
            Problem::BadMobilitySpeed {
                field: "speed_empty",
            },
        ),
        found(Some("p1"), Problem::UnknownHome { home: id("ghost") }),
        found(
            task,
            Problem::TransportEnds {
                inputs: 2,
                outputs: 0,
            },
        ),
        found(task, Problem::NoShape { element: id("n4") }),
        found(flow, Problem::ZeroQuantity),
        found(flow, Problem::QuantityIgnored),
        found(task, Problem::NothingToUnbatch),
        found(
            Some("ghost"),
            Problem::BadStop {
                reason: StopFault::NoSuchBlock,
            },
        ),
        found(
            task,
            Problem::BadStop {
                reason: StopFault::ZeroCount,
            },
        ),
        found(
            task,
            Problem::BadStop {
                reason: StopFault::NotATask,
            },
        ),
        found(Some("ghost"), Problem::BadObservation),
        found(
            Some("ghost"),
            Problem::BadStarting {
                reason: StartingFault::NoSuchBlock,
            },
        ),
        found(
            Some("n1"),
            Problem::BadStarting {
                reason: StartingFault::CannotHold,
            },
        ),
        found(
            task,
            Problem::BadStarting {
                reason: StartingFault::TooMany { count: 9, most: 5 },
            },
        ),
        found(task, Problem::StopWithWarmup),
        found(
            None,
            Problem::EndedInWarmup {
                replications: 4,
                of: 4,
            },
        ),
        found(
            None,
            Problem::EndedInWarmup {
                replications: 2,
                of: 4,
            },
        ),
        found(task, Problem::Overloaded { load: 1.25 }),
        found(
            task,
            Problem::NoSteadyState {
                backlog: 40.0,
                left: 100.0,
                utilisation: 0.98,
            },
        ),
        found(None, Problem::OneReplication),
        found(
            task,
            Problem::NoWarmup {
                warmup: Some(30.0),
                shift: -0.4,
                unit: "min",
            },
        ),
        found(
            task,
            Problem::NoWarmup {
                warmup: None,
                shift: 0.4,
                unit: "min",
            },
        ),
        found(task, Problem::EstimateIgnored),
        found(
            task,
            Problem::BadEstimate {
                error: FitError::NotFinite,
            },
        ),
        found(
            task,
            Problem::BadEstimate {
                error: FitError::NotInOrder,
            },
        ),
        found(
            task,
            Problem::BadEstimate {
                error: FitError::NoMaximum,
            },
        ),
        found(task, Problem::EstimateMismatch),
        found(
            task,
            Problem::NormalOftenNegative {
                parameter: "service",
            },
        ),
        behaviour(task, Behaviour::Unbalanced { came: 10, went: 9 }, 1, 1),
        behaviour(None, Behaviour::Unaccounted { came: 10, went: 9 }, 2, 5),
        behaviour(None, Behaviour::NothingOut { came: 10 }, 1, 1),
        behaviour(None, Behaviour::NothingMoved { came: 6 }, 1, 1),
        behaviour(
            task,
            Behaviour::LittlesLaw {
                mean: 2.0,
                from_stays: 3.0,
            },
            1,
            1,
        ),
        behaviour(
            task,
            Behaviour::BusyNotWorking {
                busy: 2.0,
                working: 1.0,
            },
            1,
            1,
        ),
        behaviour(
            task,
            Behaviour::BusyNotWorking {
                busy: 1.0,
                working: 2.0,
            },
            1,
            1,
        ),
        behaviour(task, Behaviour::StatesTotal { total: 1.2 }, 1, 1),
        behaviour(
            task,
            Behaviour::OutOfRange {
                metric: "time_off_shift",
                value: 1.3,
            },
            1,
            1,
        ),
    ]
}

/// Messages that carry the user's own words (an expression) or a block that is not in the model,
/// so they do not name one of its blocks, or may hold the user's camel case.
const NO_BLOCK_NAME: &[&str] = &[
    "invalid_id",
    "duplicate_id",
    "bad_stop_missing",
    "bad_observation",
    "bad_starting_missing",
    "no_source",
    "no_sink",
    "behaviour_unaccounted",
    "behaviour_nothing_out",
    "behaviour_nothing_moved",
];
const USER_WORDS: &[&str] = &["bad_expression", "bad_condition"];

#[test]
fn every_message_names_its_block_and_shows_no_field_key() {
    let m = model();
    let mut keys: BTreeSet<&'static str> = BTreeSet::new();
    for d in samples() {
        let message = d.describe(&m);
        let text = message.english();
        let code = d.problem.code();
        assert!(!message.parts.is_empty(), "{code}");
        assert!(
            !text.contains('{') && !text.contains('}'),
            "{code}: a placeholder is left unfilled: {text}"
        );
        assert!(text.ends_with('.'), "{code}: {text}");
        assert!(!text.contains('`'), "{code}: {text}");
        assert!(!text.contains('—') && !text.contains('–'), "{code}: {text}");
        let first = message.parts[0].key;
        for p in &message.parts {
            keys.insert(p.key);
        }

        // The block, by name: never its id, which the sample model never uses in words.
        let name = match d.at.as_ref().map(ElementId::as_str) {
            Some("c1") => Some("Front desk"),
            Some("n2") => Some("Serve customer"),
            Some("n3") => Some("Check gate"),
            Some("p1") => Some("Night nurses"),
            Some("k1") => Some("Weekday shifts"),
            _ => None,
        };
        if let Some(name) = name
            && !NO_BLOCK_NAME.contains(&first)
        {
            assert!(text.contains(name), "{code}: no \"{name}\" in {text}");
        }
        for plain_id in ["n1", "n2", "n3", "n4", "p1", "k1", "c1", "c2"] {
            let word = format!(" {plain_id} ");
            assert!(
                !text.contains(&word) && !text.starts_with(&format!("{plain_id} ")),
                "{code}: the id {plain_id} is shown: {text}"
            );
        }

        // "Block", not "element"; no field key (a snake_case or camel case word, or a key that is
        // not an English word).
        assert!(
            !text
                .to_lowercase()
                .split(|c: char| !c.is_alphabetic())
                .any(|w| w == "element" || w == "elements"),
            "{code}: {text}"
        );
        for word in text.split_whitespace() {
            let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            assert!(!word.contains('_'), "{code}: a key, `{word}`, in {text}");
            let camel = word
                .chars()
                .zip(word.chars().skip(1))
                .any(|(a, b)| a.is_lowercase() && b.is_uppercase());
            assert!(
                !camel || word == "StochCraft" || USER_WORDS.contains(&first),
                "{code}: camel case `{word}` in {text}"
            );
            assert!(
                !["interarrival", "standardDeviation", "stochcraft:"].contains(&word),
                "{code}: {word} in {text}"
            );
        }

        // Label arguments are the window's English labels, which the catalogs hold.
        let all = labels();
        for p in &message.parts {
            for (name, value) in &p.args {
                if LABEL_ARGS.contains(name) {
                    assert!(
                        all.contains(&value.as_str()),
                        "{code}: {name} = {value:?} is not a label"
                    );
                }
            }
        }
    }

    // Every template is used by some sample, and every sample's key has a template.
    let templates: BTreeSet<&'static str> = TEMPLATES.iter().map(|(k, _)| *k).collect();
    assert_eq!(templates.len(), TEMPLATES.len(), "a key twice");
    let unused: Vec<&str> = templates.difference(&keys).copied().collect();
    assert!(unused.is_empty(), "no sample for {unused:?}");
    let unknown: Vec<&str> = keys.difference(&templates).copied().collect();
    assert!(unknown.is_empty(), "no template for {unknown:?}");
}

#[test]
fn a_block_without_a_name_is_called_by_its_id() {
    let mut m = model();
    m.elements[1].name = String::new();
    let d = found(Some("n2"), Problem::NoOutput);
    assert_eq!(
        d.describe(&m).english(),
        "n2 has nowhere to send entities. Add a flow from it to the next block."
    );
}

#[test]
fn a_flow_is_named_by_the_blocks_it_joins() {
    let d = found(Some("c1"), Problem::BadProbability { value: 1.5 });
    let message = d.describe(&model());
    assert_eq!(
        message.english(),
        "The probability 1.5 on the flow Front desk → Serve customer is not from 0 to 1. Enter a probability from 0 to 1."
    );
    assert_eq!(message.parts[0].key, "bad_probability");
}

#[test]
fn a_distribution_message_names_the_time_by_its_label_and_the_field_to_show() {
    let d = found(
        Some("n2"),
        Problem::BadDistribution {
            parameter: "service",
            error: DistError::OutOfRange {
                parameter: "mean",
                requirement: Requirement::ZeroOrMore,
            },
        },
    );
    let message = d.describe(&model());
    assert_eq!(
        message.english(),
        "In “Service time” of Serve customer, set “Mean” to 0 or more."
    );
    assert_eq!(message.field.as_deref(), Some("service"));
    assert_eq!(d.problem.code(), "bad_distribution");
}

#[test]
fn the_plain_text_has_its_severity_and_ids() {
    let d = found(Some("n2"), Problem::NoOutput);
    assert_eq!(
        d.plain(),
        "error: n2 has nowhere to send entities. Add a flow from it to the next block."
    );
    assert_eq!(d.to_string(), d.plain());
}

#[test]
fn fill_keeps_a_name_it_has_no_value_for() {
    assert_eq!(
        fill("{a} and {b} {", &[("a", "x".to_owned())]),
        "x and {b} {"
    );
}
