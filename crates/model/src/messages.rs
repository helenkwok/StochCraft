//! The words of validation messages (issue #145).
//!
//! A [`Problem`] has a stable `code` that clients and tests match on. Its human message is built
//! here, in three steps, so a window can show it in the user's language while the command line
//! and agents keep English:
//!
//! 1. [`Diagnostic::describe`] turns a diagnostic into a [`Message`]: one or more [`Part`]s,
//!    each a template `key` plus the data it needs (`args`: a block's name, a number, a time or
//!    other label in the window's words).
//! 2. [`TEMPLATES`] holds the English text of every key, with `{name}` placeholders. A message
//!    says which block it is about by its name (its id when it has none), says "block" and never
//!    shows a field key: a field is named by the label the window's form gives it, in curly
//!    quotes. Each ends with what to do.
//! 3. [`Message::english`] fills the templates in. A client that has its own words for a key (the
//!    window's catalogs) fills its template from the same `args`.
//!
//! Arguments whose names are in [`LABEL_ARGS`] hold a label of the window in English: the source
//! text of a catalog row, for the window to translate. All others are data (names, numbers,
//! times, expressions) and are shown as they are.

use stochcraft_random::{DistError, Requirement};

use crate::{
    Behaviour, Diagnostic, ElementId, ExperimentField, FitError, IgnoredWhat, IgnoredWhy, Model,
    Problem, PropertyFault, PropertyType, StartingFault, StopFault,
};

/// A piece of a message: a template and what fills it.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// The template's key in [`TEMPLATES`].
    pub key: &'static str,
    /// What fills the template's `{name}` placeholders.
    pub args: Vec<(&'static str, String)>,
}

impl Part {
    /// The part in English.
    pub fn english(&self) -> String {
        fill(template(self.key).unwrap_or(self.key), &self.args)
    }
}

/// A diagnostic's message: parts that read one after the other.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub parts: Vec<Part>,
    /// The field of the block's form (or of the model's page, `x/…`) to show for a problem with
    /// one particular field, where there is one.
    pub field: Option<String>,
}

impl Message {
    /// The message in English, its parts joined by a space.
    pub fn english(&self) -> String {
        self.parts
            .iter()
            .map(Part::english)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The arguments that hold a label of the window, in English, rather than data.
pub const LABEL_ARGS: &[&str] = &["field", "param", "state", "way", "type", "type2", "metric"];

/// `template` with each `{name}` replaced by its value in `args` (a name with no value is kept).
pub fn fill(template: &str, args: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        match tail.find('}') {
            Some(close) => {
                let name = &tail[..close];
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, value)) => out.push_str(value),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &tail[close + 1..];
            }
            None => {
                out.push('{');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The English template for `key`.
pub fn template(key: &str) -> Option<&'static str> {
    TEMPLATES.iter().find(|(k, _)| *k == key).map(|(_, t)| *t)
}

/// Every template: its key and its English text. The window has its own words for the few
/// that say "replication" (it says "run").
pub const TEMPLATES: &[(&str, &str)] = &[
    (
        "invalid_id",
        "The id “{id}” is not valid. Start an id with a letter or an underscore, then use only letters, digits, dots, hyphens and underscores.",
    ),
    (
        "duplicate_id",
        "More than one block, flow, pool or calendar has the id “{id}”. Give each its own id.",
    ),
    (
        "unknown_element",
        "The flow {flow} uses a block that does not exist. Remove the flow, or connect it to a block that exists.",
    ),
    (
        "into_source",
        "The flow {flow} leads into an Arrivals block, which takes no input. Remove the flow, or send it to another block.",
    ),
    (
        "out_of_sink",
        "The flow {flow} leaves a Departures block, which has no output. Remove the flow, or start it from another block.",
    ),
    (
        "self_loop",
        "The flow {flow} leads from a block back to itself. Remove it, or send it to another block.",
    ),
    (
        "no_output",
        "{block} has nowhere to send entities. Add a flow from it to the next block.",
    ),
    (
        "several_outputs",
        "{block} has {count} flows out, and only a Decision or a Split or join block can send entities down several. Remove the extra flows, or add a Decision block.",
    ),
    (
        "bad_distribution_not_a_number",
        "In “{field}” of {block}, set “{param}” to a finite number.",
    ),
    (
        "bad_distribution_above_zero",
        "In “{field}” of {block}, set “{param}” to a number above 0.",
    ),
    (
        "bad_distribution_zero_or_more",
        "In “{field}” of {block}, set “{param}” to 0 or more.",
    ),
    (
        "bad_distribution_one_or_more",
        "In “{field}” of {block}, set “{param}” to 1 or more.",
    ),
    (
        "bad_distribution_share",
        "In “{field}” of {block}, set “{param}” to a number from 0 to 1.",
    ),
    (
        "bad_distribution_bounds",
        "In “{field}” of {block}, set “Minimum” below “Maximum”, and any “Most likely” value between them.",
    ),
    (
        "bad_distribution_no_points",
        "In “{field}” of {block}, add data points whose probabilities add up to more than 0.",
    ),
    (
        "zero_capacity",
        "{block} would never have a unit to work with. Give it at least 1 unit.",
    ),
    (
        "no_time_between_arrivals",
        "{block} would make entities without end, because its “Time between arrivals” is always 0. Give it a time above 0, or set an “Arrival limit”.",
    ),
    (
        "no_source",
        "The model has no Arrivals block, so nothing will happen. Add an Arrivals block.",
    ),
    (
        "no_sink",
        "The model has no Departures block, so no time in system is measured. Add a Departures block.",
    ),
    (
        "unreachable",
        "No entity can reach {block} from an Arrivals block. Connect it to a block that entities pass through.",
    ),
    (
        "no_path_to_sink",
        "Entities in {block} can never reach a Departures block. Connect it on towards Departures.",
    ),
    (
        "bad_power",
        "Set the “{state}” power of {block} to a number of kilowatts, 0 or more.",
    ),
    (
        "bad_experiment_warmup",
        "Set the warm-up of the experiment to 0 or more.",
    ),
    (
        "bad_experiment_duration",
        "Set the duration of the experiment to more than 0.",
    ),
    (
        "bad_experiment_replications",
        "Set the number of replications of the experiment to 1 or more.",
    ),
    (
        "bad_layout_scale",
        "Set the layout scale (length units per drawing unit) to a finite number above 0.",
    ),
    (
        "bad_property_no_name",
        "A property of {block} has no name. Give it a name, or remove it.",
    ),
    (
        "bad_property_twice",
        "{block} sets the property “{name}” twice. Keep one of them.",
    ),
    (
        "bad_property_shares",
        "The property “{name}” of {block} needs shares that are not negative and add up to more than 0. Change the shares.",
    ),
    (
        "property_type_conflict",
        "The property “{name}” has the type “{type2}” on {block} but “{type}” on another block. Give it one type everywhere.",
    ),
    (
        "property_value_type",
        "The value of the property “{name}” on {block} does not fit its type, “{type}”. Change the value or the type.",
    ),
    (
        "bad_expression",
        "The expression for the property “{name}” on {block} cannot be used: {error}. Correct “{expression}”.",
    ),
    (
        "unknown_property",
        "{block} reads the property “{name}”, which no block sets, so it would read as empty. Set it on an earlier block, or correct the name.",
    ),
    (
        "changes_without_property",
        "The setup of {block} has times for particular changes but no property to watch. Choose a property under “Only when this property changes”.",
    ),
    (
        "times_by_value_without_property",
        "{block} has times for particular values but no property to read them from. Choose a property under “Service time by property”.",
    ),
    (
        "duplicate_time_by_value",
        "{block} has two times for the value “{value}”. Keep one.",
    ),
    (
        "time_by_value_negative",
        "Set the time of {block} for the value “{value}” to a mean of 0 or more.",
    ),
    (
        "time_for_unknown_pool",
        "{block} has a service time for the pool “{pool}”, which the model does not have. Remove that time, or choose a pool that exists.",
    ),
    (
        "duplicate_time_by_pool",
        "{block} has two service times for the pool “{pool}”. Keep one.",
    ),
    (
        "time_for_pool_not_needed",
        "{block} has a service time for the pool “{pool}”, which it does not need. Add the pool to its needs, or remove that time.",
    ),
    (
        "scrap_not_against_server",
        "The scrap at {block} counts against {other}, which is not a task with a resource. Choose a task with a resource.",
    ),
    (
        "planned_run_time_not_positive",
        "Set “Planned run time per item” of {block} to a mean above 0.",
    ),
    (
        "patience_negative",
        "Set “Time before giving up” of {block} to a mean of 0 or more.",
    ),
    (
        "abandoned_without_patience",
        "The flow {flow} is ticked “Gave up”, but only a task with a resource and a “Time before giving up” sends entities that give up. Untick it, or set a “Time before giving up” on {block}.",
    ),
    (
        "several_ways",
        "{count} flows out of {block} are ticked “{way}”; at most one can be. Untick all but one.",
    ),
    (
        "turned_away_without_room",
        "The flow {flow} is ticked “Turned away”, but only a task with a resource and a limited waiting room turns entities away. Untick it, or limit the waiting room of {block}.",
    ),
    (
        "turned_away_and_abandoned",
        "The flow {flow} is ticked both “Gave up” and “Turned away”. Keep one of them.",
    ),
    (
        "balking_probability",
        "Set every chance of joining in the balking table of {block} to a number from 0 to 1.",
    ),
    (
        "balking_rows_order",
        "Put the rows of the balking table of {block} in increasing order of “Number waiting”.",
    ),
    (
        "balked_without_balking",
        "The flow {flow} is ticked “Balked”, but only a task with a resource and balking sends entities that balk. Untick it, or turn on balking for {block}.",
    ),
    (
        "balked_and_other_way",
        "The flow {flow} is ticked “Balked” and also “Gave up” or “Turned away”. Keep one of them.",
    ),
    (
        "queue_limit_without_room",
        "The waiting room of {block} is set by calendar or per working server, but has no number of places, so it is unlimited. Give it a number of places, or make it fixed.",
    ),
    (
        "queue_by_shift_empty",
        "The waiting room of {block} is set by calendar, but no calendar has a value. Add a value for a calendar, or make the waiting room fixed.",
    ),
    (
        "health_check_limit_not_valid",
        "Set “Fails above, waiting per working server” of {block} to a number of 0 or more.",
    ),
    (
        "health_check_interval_not_positive",
        "Set “Check every” of {block} to a mean above 0.",
    ),
    (
        "demand_not_positive",
        "Set the demand of {block} to a number of parts above 0.",
    ),
    (
        "shift_without_calendar",
        "The demand of {block} is per shift but names no calendar to say when the shifts are. Choose a calendar.",
    ),
    (
        "demand_calendar_unknown",
        "The demand of {block} names a calendar that does not exist. Choose a calendar that exists.",
    ),
    (
        "bad_probability",
        "The probability {value} on the flow {flow} is not from 0 to 1. Enter a probability from 0 to 1.",
    ),
    (
        "probability_sum",
        "The probabilities of the flows out of {block} add up to {sum}, not 1, so they are used in proportion. Change them to add up to 1 if that is not what you want.",
    ),
    (
        "bad_condition",
        "The condition on the flow {flow} cannot be used: {error}. Correct “{expression}”.",
    ),
    (
        "routing_ignored_condition_plain",
        "The condition on the flow {flow} is ignored, because only flows out of a Decision block choose a route. Remove the condition.",
    ),
    (
        "routing_ignored_probability_plain",
        "The probability on the flow {flow} is ignored, because only flows out of a Decision block choose a route. Remove the probability.",
    ),
    (
        "routing_ignored_condition_shortest",
        "The condition on the flow {flow} is ignored, because the Decision sends entities to the shortest queue. Remove the condition.",
    ),
    (
        "routing_ignored_probability_shortest",
        "The probability on the flow {flow} is ignored, because the Decision sends entities to the shortest queue. Remove the probability.",
    ),
    (
        "routing_ignored_default_shortest",
        "The default on the flow {flow} is ignored, because the Decision sends entities to the shortest queue. Remove the default.",
    ),
    (
        "routing_ignored_probability_condition",
        "The probability on the flow {flow} is ignored, because the Decision routes by condition. Remove the probability.",
    ),
    (
        "bad_default",
        "The default flow of {block} is not one of its own flows out. Choose one of the flows out of {block} as the default.",
    ),
    (
        "no_default",
        "{block} routes by condition but has no default flow, so an entity for which no condition holds leaves the model here. Choose a default flow.",
    ),
    (
        "divert_no_default",
        "{block} diverts entities but has no default flow, so an entity whose way is full has nowhere else to go. Choose a default flow.",
    ),
    (
        "join_never_completes",
        "{block} waits for an entity on the flow {flow}, which none can reach, so it never sends one on. Remove that flow, or connect it to a block that entities reach.",
    ),
    (
        "buffer_behind_gateway",
        "{block} has a limited waiting room and is fed through {gateway}. While it is full, {gateway} holds up every entity behind it, and they wait before it without limit. Remove the gateway before {block}, or remove the limit.",
    ),
    (
        "blocking_cycle",
        "The limited waiting rooms of {loop} feed each other in a loop, so when all of them are full the loop deadlocks. Remove the limit on one of them, or break the loop.",
    ),
    (
        "empty_pool",
        "The pool {block} has no units. Give it at least 1 unit.",
    ),
    (
        "no_needs",
        "{block} names no resource it needs. Add one under “Needs”.",
    ),
    (
        "unknown_resource",
        "{block} needs a resource pool that does not exist. Choose a pool that exists.",
    ),
    (
        "no_pool_with_roles",
        "{block} needs units with the roles {roles}, which no pool has. Give a pool those roles, or change what {block} needs.",
    ),
    (
        "need_never_met",
        "{block} needs {needed} units at once, but at most {available} exist, so this can never be met. Ask for fewer units, or add units to the pool.",
    ),
    (
        "bad_priority",
        "Set the priority of {block} to a finite number, not {value}.",
    ),
    (
        "unused_pool",
        "No task uses the pool {block}. Add it to a task's needs, or remove the pool.",
    ),
    (
        "unknown_calendar",
        "{block} has a value for a calendar that does not exist. Choose a calendar that exists, or remove the value.",
    ),
    (
        "empty_calendar",
        "The calendar {block} has no entries, so it is never active. Add an entry.",
    ),
    (
        "timed_ignored",
        "{block} allows no arrivals outside the calendars but names no calendar, so it never creates entities. Add arrivals by calendar, or allow arrivals outside calendars.",
    ),
    (
        "zero_batch_size",
        "Set the batch size of {block} to 1 or more.",
    ),
    (
        "bad_conveyor_above_zero",
        "Set “{field}” of the conveyor {block} to a finite number above 0.",
    ),
    (
        "bad_conveyor_zero_or_more",
        "Set “{field}” of the conveyor {block} to a finite number of 0 or more.",
    ),
    (
        "bad_conveyor_nonzero",
        "Set “{field}” of the conveyor {block} to a finite number other than 0.",
    ),
    (
        "conveyor_too_short",
        "The conveyor {block} is {length} long but each entity takes {pitch} (item length plus gap), so not even one fits. Lengthen the conveyor, or shorten the item or the gap.",
    ),
    (
        "conveyor_length_mismatch",
        "The drawn path of the conveyor {block} is {path_length} long, but its “Length” is {length}, so the drawing and the run disagree. Make “Length” match the path, or change the shape.",
    ),
    (
        "place_off_path",
        "{block} is placed {s} along the conveyor {conveyor}, whose path is {length} long. Place it between 0 and {length} along the path.",
    ),
    (
        "points_out_of_order",
        "Entities on the conveyor {conveyor} reach {before} before {block}, but {block} is placed nearer the start ({s} along it, {before} at {before_s}). Place {block} further along the path than {before}, or swap them in the flows.",
    ),
    (
        "bad_junction_time",
        "The junction on the flow {flow} takes {time} to cross. Make its transfer time 0 or more.",
    ),
    (
        "transport_not_simulated",
        "{block} is a transport, which StochCraft does not simulate yet. Remove it to run the model.",
    ),
    (
        "decision_unusable",
        "{block} uses the decision table “{decision}” of the file {file}, which it cannot use: {reason} Correct the table, or take it off the block.",
    ),
    (
        "decision_overlap",
        "{block} uses the decision table “{decision}”, whose rules {first} and {second} both match {example}, but under its hit policy Unique only one may. Change the rules so that only one matches, or choose another hit policy.",
    ),
    (
        "decision_gap",
        "{block} uses the decision table “{decision}”, in which no rule matches {example}, so such an entity gets no result there. Add a rule or a default output, if that is not intended.",
    ),
    (
        "decision_route",
        "{block} routes by the decision table “{decision}”, which can give “{value}”, the name of none of its ways out. Make each output the name or id of a block the gateway leads to.",
    ),
    (
        "transport_pool_missing",
        "The pool {pool} of the transport {block} does not exist. Choose a pool that exists.",
    ),
    (
        "transport_pool_not_mobile",
        "The pool {pool} of the transport {block} is not mobile. Give the pool speeds and a home.",
    ),
    (
        "bad_mobility_speed",
        "Set “{field}” of the pool {block} to a finite number above 0.",
    ),
    (
        "unknown_home",
        "The home of the pool {block} is not a block of the model. Choose a block as its home.",
    ),
    (
        "transport_ends",
        "The transport {block} needs one block before it, where it picks up, and one after it, where it drops off, but has {inputs} before and {outputs} after. Connect it to one block on each side.",
    ),
    (
        "no_shape",
        "{other} has no place on the canvas, so the distances for {block} are not known. Place {other} on the canvas.",
    ),
    (
        "zero_quantity",
        "The flow {flow} asks for no entities. Set its “Units” to 1 or more.",
    ),
    (
        "quantity_ignored",
        "The “Units” on the flow {flow} is ignored, because only a Split or join block takes several entities from one flow. Remove it.",
    ),
    (
        "nothing_to_unbatch",
        "No block makes batches, so {block} passes every entity through. Add a Batch block before it, or remove {block}.",
    ),
    (
        "bad_stop_missing",
        "The stop condition names a block that does not exist. Choose a block that exists.",
    ),
    (
        "bad_stop_count",
        "Set the count of the stop condition for {block} to 1 or more.",
    ),
    (
        "bad_stop_empty",
        "The stop condition waits for {block} to empty, but only a task can empty. Choose a task.",
    ),
    (
        "bad_observation",
        "Observed data is given for a block that does not exist, so there is no result to check it against. Remove it, or choose a block that exists.",
    ),
    (
        "bad_starting_missing",
        "Entities inside at the start are given for a block that does not exist. Remove them.",
    ),
    (
        "bad_starting_kind",
        "{block} cannot have entities inside at the start: only a task or a batch can. Remove its “Inside at the start”.",
    ),
    (
        "bad_starting_too_many",
        "{block} has {count} entities inside at the start, but it holds at most {most}, its units and its waiting room. Set “Inside at the start” to {most} or fewer, or make the waiting room larger.",
    ),
    (
        "stop_with_warmup",
        "The stop condition at {block} can be met before the warm-up ends. A replication that stops during the warm-up has no statistics after it and is left out of the results. Shorten the warm-up, or stop later.",
    ),
    (
        "ended_in_warmup_all",
        "Every replication stopped before the warm-up ended, so no metric is reported. Shorten the warm-up, or stop later.",
    ),
    (
        "ended_in_warmup_some",
        "{replications} of {of} replications stopped before the warm-up ended, so their statistics are left out of every metric. Shorten the warm-up, or stop later.",
    ),
    (
        "overloaded",
        "{block} is offered {load}% of the work its units can do, so its waiting room grows without limit and the results describe no steady state. Add units, or shorten the service.",
    ),
    (
        "no_steady_state",
        "{block} is busy {utilisation}% of the time, and at the end at least {backlog} entities are still waiting against {left} that left. Work arrives faster than it is done, so the results describe no steady state. Add units, or shorten the service.",
    ),
    (
        "one_replication",
        "The run has one replication, so its results cannot be told from chance. Run at least 2 replications, and usually 10 or more.",
    ),
    (
        "no_warmup_at",
        "The run has no warm-up, and the number inside {block} settles only after about {warmup} {unit} (MSER-5 on its mean across replications). The start from empty moves its mean by {shift}. Set a warm-up of about {warmup} {unit}.",
    ),
    (
        "no_warmup_late",
        "The run has no warm-up, and the number inside {block} is still settling halfway through the run (MSER-5 on its mean across replications). The start from empty moves its mean by {shift}. Lengthen the run, and set a warm-up.",
    ),
    (
        "estimate_ignored",
        "{block} has a three-point estimate but no time to fit it to, so the estimate is ignored. Remove the estimate.",
    ),
    (
        "bad_estimate_not_finite",
        "The three-point estimate of {block} cannot be fitted: every point must be a finite number. Correct the points.",
    ),
    (
        "bad_estimate_not_in_order",
        "The three-point estimate of {block} cannot be fitted: the points must be in order, minimum, most likely, then maximum or 95th percentile. Correct the points.",
    ),
    (
        "bad_estimate_no_maximum",
        "The three-point estimate of {block} cannot be fitted: log-normal and Weibull estimates have no maximum. Give a 95th percentile instead.",
    ),
    (
        "estimate_mismatch",
        "The time of {block} is no longer the one fitted from its three-point estimate. Fit it again, or remove the estimate.",
    ),
    (
        "normal_often_negative",
        "In “{field}” of {block}, the normal distribution draws below 0 more than 1 time in 100, and each such draw counts as 0, so the times are longer on average than its mean. Use a truncated normal with a minimum of 0, or a log-normal.",
    ),
    (
        "behaviour_unbalanced",
        "The entities of {block} do not add up: {came} came in or were inside at the start, but {went} left, were lost, gave up or are still inside.",
    ),
    (
        "behaviour_unaccounted",
        "The entities of the model do not add up: {came} were created or inside at the start, but {went} reached Departures, left the model elsewhere or are still inside.",
    ),
    (
        "behaviour_nothing_out",
        "No entity left the model, of {came} created or inside at the start. Check that the routing leads to Departures and that the run is long enough for an entity to get through.",
    ),
    (
        "behaviour_nothing_moved",
        "The entities going round the closed model are stuck, of {came} created or inside at the start: no block where they were at the end finished any during the run. Check that every block of the loop can serve them and that the run is long enough for an entity to get round.",
    ),
    (
        "behaviour_littles_law",
        "Little's law does not hold at {block}: the time-average number inside was {mean}, but the entities' own times inside give {from_stays}.",
    ),
    (
        "behaviour_busy_counted",
        "The utilisation of {block} counts {busy} units busy on average, but {working} were processing or setting up: units idle, blocked, off shift, failed or restarting were counted busy.",
    ),
    (
        "behaviour_busy_missed",
        "The utilisation of {block} counts {busy} units busy on average, but {working} were processing or setting up: units at work were not counted busy.",
    ),
    (
        "behaviour_states_total",
        "The shares of the time of the units of {block} in each state add up to {total}, not 1.",
    ),
    (
        "behaviour_out_of_range",
        "“{metric}” of {block} is {value}, outside 0 to 1 (0% to 100%).",
    ),
    (
        "behaviour_runs",
        "This was found in {replications} of {of} replications, and the furthest off is shown.",
    ),
    (
        "behaviour_report",
        "This points to a fault in StochCraft rather than in the model. Please report it with the model.",
    ),
];

/// The label the window gives a field, in English: what it says in the form, and so in the
/// catalogs. A key it does not know comes back as it is.
pub fn field_label(key: &str) -> &str {
    FIELD_LABELS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, label)| *label)
}

/// The fields a message can name, by the key the engine gives them, with their labels.
pub const FIELD_LABELS: &[(&str, &str)] = &[
    ("interarrival", "Time between arrivals"),
    ("service", "Service time"),
    ("setup", "Setup time"),
    ("time_to_failure", "Time to failure"),
    ("repair", "Repair time"),
    ("transfer", "Transfer time"),
    ("planned_run_time", "Planned run time per item"),
    ("patience", "Time before giving up"),
    ("check_interval", "Check every"),
    ("restart", "Out for (restart)"),
    ("duration", "Duration"),
    ("load", "Load"),
    ("unload", "Unload"),
    ("value", "Value"),
    ("mean", "Mean"),
    ("min", "Minimum"),
    ("max", "Maximum"),
    ("mode", "Most likely"),
    ("std_dev", "Standard deviation"),
    ("shape", "Shape"),
    ("scale", "Scale"),
    ("k", "Stages (k)"),
    ("probability", "Probability"),
    ("length", "Length"),
    ("speed", "Speed"),
    ("item_length", "Item length"),
    ("gap", "Gap"),
    ("radius", "Radius"),
    ("angle", "Angle"),
    ("rise", "Rise"),
    ("turns", "Turns"),
    ("speed_empty", "Speed empty"),
    ("speed_loaded", "Speed loaded"),
    ("processing", "Processing"),
    ("idle", "Idle"),
    ("blocked", "Blocked"),
    ("failed", "Failed"),
    ("off_shift", "Off shift"),
];

/// How the window labels the results the behaviour checks name, in English.
const METRIC_LABELS: &[(&str, &str)] = &[
    ("utilisation", "Utilisation"),
    ("availability", "Availability"),
    ("blocking_probability", "Share of units blocked"),
    ("time_off_shift", "Time off shift"),
    ("time_processing", "Time processing"),
    ("time_starved", "Time starved"),
    ("time_blocked", "Time blocked"),
    ("time_failed", "Time failed"),
    ("time_restarting", "Time restarting"),
    ("time_setup", "Time setting up"),
    ("oee_availability", "OEE availability"),
    ("setup_ratio", "Setup ratio"),
];

/// The three ways out a flow can be ticked for.
const WAYS: [&str; 3] = ["Gave up", "Turned away", "Balked"];

/// Every label a message can pass in a [`LABEL_ARGS`] argument, in English: each needs a row in
/// the window's catalogs.
pub fn labels() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = FIELD_LABELS.iter().map(|(_, l)| *l).collect();
    out.extend(METRIC_LABELS.iter().map(|(_, l)| *l));
    out.extend(WAYS);
    out.extend(PropertyType::ALL.map(type_label));
    out
}

fn metric_label(key: &str) -> &str {
    METRIC_LABELS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, label)| *label)
}

/// How the window names a property's type.
fn type_label(ty: PropertyType) -> &'static str {
    match ty {
        PropertyType::String => "Text",
        PropertyType::Boolean => "Yes or no",
        PropertyType::Long => "Whole number",
        PropertyType::Double => "Decimal number",
        PropertyType::Duration => "Duration",
        PropertyType::DateTime => "Date and time",
    }
}

impl Model {
    /// What to call element, pool or calendar `id` in a message: its name, or its id when it has
    /// none (or is not in the model).
    pub fn display_name(&self, id: &ElementId) -> String {
        let named = |name: &str| (!name.trim().is_empty()).then(|| name.to_owned());
        self.element(id)
            .and_then(|e| named(&e.name))
            .or_else(|| {
                self.resources
                    .iter()
                    .find(|r| &r.id == id)
                    .and_then(|r| named(&r.name))
            })
            .or_else(|| {
                self.calendars
                    .iter()
                    .find(|c| &c.id == id)
                    .and_then(|c| named(&c.name))
            })
            .unwrap_or_else(|| id.to_string())
    }

    /// Flow `id` as "from → to", by the names of its blocks; `None` when there is no such flow.
    pub fn flow_text(&self, id: &ElementId) -> Option<(String, String)> {
        let c = self.connections.iter().find(|c| &c.id == id)?;
        Some((self.display_name(&c.from), self.display_name(&c.to)))
    }
}

/// What a message calls things: by the model's names, or by id with no model.
struct Names<'a>(Option<&'a Model>);

impl Names<'_> {
    fn name(&self, id: &ElementId) -> String {
        self.0
            .map_or_else(|| id.to_string(), |m| m.display_name(id))
    }

    /// The first block of flow `id`, and the flow as "from → to"; its id twice with no model or
    /// no such flow.
    fn flow(&self, id: &ElementId) -> (String, String) {
        match self.0.and_then(|m| m.flow_text(id)) {
            Some((from, to)) => (from.clone(), format!("{from} → {to}")),
            None => (self.name(id), self.name(id)),
        }
    }
}

type Args = Vec<(&'static str, String)>;

fn part(key: &'static str, args: Args) -> Vec<Part> {
    vec![Part { key, args }]
}

/// A number as short as it reads: at most four decimals, no trailing zeros.
fn num(v: f64) -> String {
    num_to(v, 4)
}

/// The decimal places that show a step of `step` (0.0001 needs 4; 0.1 needs 1), at most 12.
fn places_for(step: f64) -> usize {
    let mut places = 0;
    let mut s = step;
    while places < 12 && s < 1.0 - 1e-9 {
        s *= 10.0;
        places += 1;
    }
    places
}

/// `v` with at most `decimals` decimal places, trailing zeros dropped.
fn num_to(v: f64, decimals: usize) -> String {
    // A value that is no number at all is said in words: users read "NaN" and "inf" as codes.
    if v.is_nan() {
        return "undefined".to_owned();
    }
    if v.is_infinite() {
        return "infinite".to_owned();
    }
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

impl Diagnostic {
    /// What this diagnostic says, in `model`'s words: its block by name.
    pub fn describe(&self, model: &Model) -> Message {
        self.describe_with(&Names(Some(model)))
    }

    /// The diagnostic in English, with its severity ("error: …"), naming blocks by their ids: for
    /// logs and tests that have no model at hand. [`Diagnostic::describe`] names them.
    pub fn plain(&self) -> String {
        let level = match self.severity {
            crate::Severity::Error => "error",
            crate::Severity::Warning => "warning",
        };
        format!("{level}: {}", self.describe_with(&Names(None)).english())
    }

    fn describe_with(&self, names: &Names<'_>) -> Message {
        // A flow's problems are about its block: the one it leaves.
        let block = match &self.at {
            Some(id) => names.flow(id).0,
            None => String::new(),
        };
        let at = self.at.as_ref();
        // The flow a problem is about: the one it names, else the one it is at.
        let flow_of = |connection: Option<&ElementId>| -> String {
            match connection.or(at) {
                Some(c) => names.flow(c).1,
                None => String::new(),
            }
        };
        let b = || ("block", block.clone());
        let mut field = None;
        let parts = match &self.problem {
            Problem::InvalidId { id } => part("invalid_id", vec![("id", id.clone())]),
            Problem::DuplicateId { id } => part("duplicate_id", vec![("id", id.to_string())]),
            Problem::UnknownElement { connection, .. } => {
                part("unknown_element", vec![("flow", flow_of(Some(connection)))])
            }
            Problem::IntoSource { connection } => {
                part("into_source", vec![("flow", flow_of(Some(connection)))])
            }
            Problem::OutOfSink { connection } => {
                part("out_of_sink", vec![("flow", flow_of(Some(connection)))])
            }
            Problem::SelfLoop { connection } => {
                part("self_loop", vec![("flow", flow_of(Some(connection)))])
            }
            Problem::NoOutput => part("no_output", vec![b()]),
            Problem::SeveralOutputs { count } => {
                part("several_outputs", vec![b(), ("count", count.to_string())])
            }
            Problem::BadDistribution { parameter, error } => {
                field = distribution_field(parameter);
                let key = match error {
                    DistError::NotFinite(_) => "bad_distribution_not_a_number",
                    DistError::OutOfRange { requirement, .. } => match requirement {
                        Requirement::AboveZero => "bad_distribution_above_zero",
                        Requirement::ZeroOrMore => "bad_distribution_zero_or_more",
                        Requirement::OneOrMore => "bad_distribution_one_or_more",
                        Requirement::UnitInterval => "bad_distribution_share",
                    },
                    DistError::BadBounds => "bad_distribution_bounds",
                    DistError::EmptyUserDistribution => "bad_distribution_no_points",
                };
                let param = match error {
                    DistError::NotFinite(p) | DistError::OutOfRange { parameter: p, .. } => {
                        param_key(p)
                    }
                    DistError::BadBounds | DistError::EmptyUserDistribution => "",
                };
                let mut args = vec![b(), ("field", field_label(field_key(parameter)).to_owned())];
                if !param.is_empty() {
                    args.push(("param", field_label(param).to_owned()));
                }
                part(key, args)
            }
            Problem::ZeroCapacity => part("zero_capacity", vec![b()]),
            Problem::NoTimeBetweenArrivals => part("no_time_between_arrivals", vec![b()]),
            Problem::NoSource => part("no_source", vec![]),
            Problem::NoSink => part("no_sink", vec![]),
            Problem::Unreachable => part("unreachable", vec![b()]),
            Problem::NoPathToSink => part("no_path_to_sink", vec![b()]),
            Problem::BadPower { state } => part(
                "bad_power",
                vec![b(), ("state", field_label(state).to_owned())],
            ),
            Problem::BadExperiment { field: f } => {
                let (key, path) = match f {
                    ExperimentField::Warmup => ("bad_experiment_warmup", "x/warmup"),
                    ExperimentField::Duration => ("bad_experiment_duration", "x/duration"),
                    ExperimentField::Replications => {
                        ("bad_experiment_replications", "x/replications")
                    }
                };
                field = Some(path.to_owned());
                part(key, vec![])
            }
            Problem::BadLayoutScale => part("bad_layout_scale", vec![]),
            Problem::BadProperty { name, reason } => match reason {
                PropertyFault::NoName => part("bad_property_no_name", vec![b()]),
                PropertyFault::SetTwice => {
                    part("bad_property_twice", vec![b(), ("name", name.clone())])
                }
                PropertyFault::BadShares => {
                    part("bad_property_shares", vec![b(), ("name", name.clone())])
                }
            },
            Problem::PropertyTypeConflict {
                name,
                first,
                second,
            } => part(
                "property_type_conflict",
                vec![
                    b(),
                    ("name", name.clone()),
                    ("type", type_label(*first).to_owned()),
                    ("type2", type_label(*second).to_owned()),
                ],
            ),
            Problem::PropertyValueType { name, ty, .. } => part(
                "property_value_type",
                vec![
                    b(),
                    ("name", name.clone()),
                    ("type", type_label(*ty).to_owned()),
                ],
            ),
            Problem::BadExpression {
                name,
                expression,
                error,
            } => part(
                "bad_expression",
                vec![
                    b(),
                    ("name", name.clone()),
                    ("expression", expression.clone()),
                    ("error", error.to_string()),
                ],
            ),
            Problem::UnknownProperty { name } => {
                part("unknown_property", vec![b(), ("name", name.clone())])
            }
            Problem::ChangesWithoutProperty => part("changes_without_property", vec![b()]),
            Problem::TimesByValueWithoutProperty => {
                part("times_by_value_without_property", vec![b()])
            }
            Problem::DuplicateTimeByValue { value } => part(
                "duplicate_time_by_value",
                vec![b(), ("value", value.clone())],
            ),
            Problem::TimeByValueNegative { value } => part(
                "time_by_value_negative",
                vec![b(), ("value", value.clone())],
            ),
            Problem::TimeForUnknownPool { id } => {
                part("time_for_unknown_pool", vec![b(), ("pool", names.name(id))])
            }
            Problem::DuplicateTimeByPool { id } => part(
                "duplicate_time_by_pool",
                vec![b(), ("pool", names.name(id))],
            ),
            Problem::TimeForPoolNotNeeded { id } => part(
                "time_for_pool_not_needed",
                vec![b(), ("pool", names.name(id))],
            ),
            Problem::ScrapNotAgainstServer { id } => part(
                "scrap_not_against_server",
                vec![b(), ("other", names.name(&ElementId::unchecked(id)))],
            ),
            Problem::PlannedRunTimeNotPositive => part("planned_run_time_not_positive", vec![b()]),
            Problem::PatienceNegative => part("patience_negative", vec![b()]),
            Problem::AbandonedWithoutPatience { connection } => part(
                "abandoned_without_patience",
                vec![b(), ("flow", flow_of(Some(connection)))],
            ),
            Problem::SeveralAbandonedWays { count } => part(
                "several_ways",
                vec![
                    b(),
                    ("count", count.to_string()),
                    ("way", WAYS[0].to_owned()),
                ],
            ),
            Problem::TurnedAwayWithoutRoom { connection } => part(
                "turned_away_without_room",
                vec![b(), ("flow", flow_of(Some(connection)))],
            ),
            Problem::SeveralTurnedAwayWays { count } => part(
                "several_ways",
                vec![
                    b(),
                    ("count", count.to_string()),
                    ("way", WAYS[1].to_owned()),
                ],
            ),
            Problem::TurnedAwayAndAbandoned { connection } => part(
                "turned_away_and_abandoned",
                vec![("flow", flow_of(Some(connection)))],
            ),
            Problem::BalkingProbability => part("balking_probability", vec![b()]),
            Problem::BalkingRowsOrder => part("balking_rows_order", vec![b()]),
            Problem::BalkedWithoutBalking { connection } => part(
                "balked_without_balking",
                vec![b(), ("flow", flow_of(Some(connection)))],
            ),
            Problem::SeveralBalkedWays { count } => part(
                "several_ways",
                vec![
                    b(),
                    ("count", count.to_string()),
                    ("way", WAYS[2].to_owned()),
                ],
            ),
            Problem::BalkedAndOtherWay { connection } => part(
                "balked_and_other_way",
                vec![("flow", flow_of(Some(connection)))],
            ),
            Problem::QueueLimitWithoutRoom => part("queue_limit_without_room", vec![b()]),
            Problem::QueueByShiftEmpty => part("queue_by_shift_empty", vec![b()]),
            Problem::HealthCheckLimitNotValid => part("health_check_limit_not_valid", vec![b()]),
            Problem::HealthCheckIntervalNotPositive => {
                part("health_check_interval_not_positive", vec![b()])
            }
            Problem::DemandNotPositive => part("demand_not_positive", vec![b()]),
            Problem::ShiftWithoutCalendar => part("shift_without_calendar", vec![b()]),
            Problem::DemandCalendarUnknown { .. } => part("demand_calendar_unknown", vec![b()]),
            Problem::BadProbability { value } => part(
                "bad_probability",
                vec![("flow", flow_of(None)), ("value", num(*value))],
            ),
            Problem::ProbabilitySum { sum } => {
                part("probability_sum", vec![b(), ("sum", num(*sum))])
            }
            Problem::BadCondition { expression, error } => part(
                "bad_condition",
                vec![
                    ("flow", flow_of(None)),
                    ("expression", expression.clone()),
                    ("error", error.to_string()),
                ],
            ),
            Problem::RoutingIgnored { what, reason } => {
                let key = match (what, reason) {
                    (IgnoredWhat::Condition, IgnoredWhy::NotDecision) => {
                        "routing_ignored_condition_plain"
                    }
                    (IgnoredWhat::Probability, IgnoredWhy::NotDecision) => {
                        "routing_ignored_probability_plain"
                    }
                    (IgnoredWhat::Condition, IgnoredWhy::ShortestQueue) => {
                        "routing_ignored_condition_shortest"
                    }
                    (IgnoredWhat::Probability, IgnoredWhy::ShortestQueue) => {
                        "routing_ignored_probability_shortest"
                    }
                    (IgnoredWhat::Default, IgnoredWhy::ShortestQueue) => {
                        "routing_ignored_default_shortest"
                    }
                    // A default flow is only set on a Decision, which is never "not a decision".
                    (IgnoredWhat::Default, IgnoredWhy::NotDecision | IgnoredWhy::ByCondition) => {
                        "routing_ignored_default_shortest"
                    }
                    (
                        IgnoredWhat::Condition | IgnoredWhat::Probability,
                        IgnoredWhy::ByCondition,
                    ) => "routing_ignored_probability_condition",
                };
                part(key, vec![("flow", flow_of(None))])
            }
            Problem::BadDefault { .. } => part("bad_default", vec![b()]),
            Problem::NoDefault => part("no_default", vec![b()]),
            Problem::DivertNoDefault => part("divert_no_default", vec![b()]),
            Problem::JoinNeverCompletes { connection } => part(
                "join_never_completes",
                vec![b(), ("flow", flow_of(Some(connection)))],
            ),
            Problem::BufferBehindGateway { gateway } => part(
                "buffer_behind_gateway",
                vec![b(), ("gateway", names.name(gateway))],
            ),
            Problem::BlockingCycle { through } => {
                let mut ring: Vec<String> = through.iter().map(|i| names.name(i)).collect();
                if let Some(first) = ring.first().cloned() {
                    ring.push(first);
                }
                part("blocking_cycle", vec![("loop", ring.join(" → "))])
            }
            Problem::EmptyPool => part("empty_pool", vec![b()]),
            Problem::NoNeeds => part("no_needs", vec![b()]),
            Problem::UnknownResource { .. } => part("unknown_resource", vec![b()]),
            Problem::NoPoolWithRoles { roles } => {
                part("no_pool_with_roles", vec![b(), ("roles", roles.join(", "))])
            }
            Problem::NeedNeverMet { needed, available } => part(
                "need_never_met",
                vec![
                    b(),
                    ("needed", needed.to_string()),
                    ("available", available.to_string()),
                ],
            ),
            Problem::BadPriority { value } => {
                part("bad_priority", vec![b(), ("value", num(*value))])
            }
            Problem::UnusedPool => part("unused_pool", vec![b()]),
            Problem::UnknownCalendar { .. } => part("unknown_calendar", vec![b()]),
            Problem::EmptyCalendar => part("empty_calendar", vec![b()]),
            Problem::TimedIgnored => part("timed_ignored", vec![b()]),
            Problem::ZeroBatchSize => part("zero_batch_size", vec![b()]),
            Problem::BadConveyorValue { field: f } => {
                let key = match *f {
                    "gap" => "bad_conveyor_zero_or_more",
                    "angle" => "bad_conveyor_nonzero",
                    _ => "bad_conveyor_above_zero",
                };
                part(key, vec![b(), ("field", field_label(f).to_owned())])
            }
            Problem::ConveyorTooShort { length, pitch } => part(
                "conveyor_too_short",
                vec![b(), ("length", num(*length)), ("pitch", num(*pitch))],
            ),
            Problem::ConveyorLengthMismatch {
                length,
                path_length,
            } => {
                // Shown at the typed length, which overrides the path's (D23). Both are written to
                // the step a path's length is taken to (0.1 mm in the model's unit), so they differ
                // in the message whenever they differ by more than it.
                field = Some("length".to_owned());
                let places = places_for(crate::site::length_step(
                    names
                        .0
                        .map_or(crate::LengthUnit::Metres, |m| m.experiment.length_unit),
                ))
                .max(4);
                part(
                    "conveyor_length_mismatch",
                    vec![
                        b(),
                        ("length", num_to(*length, places)),
                        ("path_length", num_to(*path_length, places)),
                    ],
                )
            }
            Problem::PlaceOffPath {
                conveyor,
                s,
                length,
            } => part(
                "place_off_path",
                vec![
                    b(),
                    ("s", num(*s)),
                    ("conveyor", names.name(conveyor)),
                    ("length", num(*length)),
                ],
            ),
            Problem::PointsOutOfOrder {
                conveyor,
                s,
                before,
                before_s,
            } => part(
                "points_out_of_order",
                vec![
                    b(),
                    ("conveyor", names.name(conveyor)),
                    ("before", names.name(before)),
                    ("s", num(*s)),
                    ("before_s", num(*before_s)),
                ],
            ),
            Problem::BadJunctionTime { time } => part(
                "bad_junction_time",
                vec![("flow", flow_of(None)), ("time", num(*time))],
            ),
            Problem::TransportNotSimulated => part("transport_not_simulated", vec![b()]),
            Problem::DecisionUnusable {
                file,
                decision,
                reason,
            } => part(
                "decision_unusable",
                vec![
                    b(),
                    ("decision", decision.clone()),
                    ("file", file.clone()),
                    ("reason", reason.clone()),
                ],
            ),
            Problem::DecisionOverlap {
                decision,
                first,
                second,
                example,
            } => part(
                "decision_overlap",
                vec![
                    b(),
                    ("decision", decision.clone()),
                    ("first", first.to_string()),
                    ("second", second.to_string()),
                    ("example", example.clone()),
                ],
            ),
            Problem::DecisionGap { decision, example } => part(
                "decision_gap",
                vec![
                    b(),
                    ("decision", decision.clone()),
                    ("example", example.clone()),
                ],
            ),
            Problem::DecisionRoute { decision, value } => part(
                "decision_route",
                vec![
                    b(),
                    ("decision", decision.clone()),
                    ("value", value.clone()),
                ],
            ),
            Problem::TransportPool { pool, exists } => part(
                if *exists {
                    "transport_pool_not_mobile"
                } else {
                    "transport_pool_missing"
                },
                vec![b(), ("pool", names.name(pool))],
            ),
            Problem::BadMobilitySpeed { field: f } => part(
                "bad_mobility_speed",
                vec![b(), ("field", field_label(f).to_owned())],
            ),
            Problem::UnknownHome { .. } => part("unknown_home", vec![b()]),
            Problem::TransportEnds { inputs, outputs } => part(
                "transport_ends",
                vec![
                    b(),
                    ("inputs", inputs.to_string()),
                    ("outputs", outputs.to_string()),
                ],
            ),
            Problem::NoShape { element } => {
                part("no_shape", vec![b(), ("other", names.name(element))])
            }
            Problem::ZeroQuantity => part("zero_quantity", vec![("flow", flow_of(None))]),
            Problem::QuantityIgnored => part("quantity_ignored", vec![("flow", flow_of(None))]),
            Problem::NothingToUnbatch => part("nothing_to_unbatch", vec![b()]),
            Problem::BadStop { reason } => match reason {
                StopFault::NoSuchBlock => part("bad_stop_missing", vec![]),
                StopFault::ZeroCount => part("bad_stop_count", vec![b()]),
                StopFault::NotATask => part("bad_stop_empty", vec![b()]),
            },
            Problem::BadObservation => part("bad_observation", vec![]),
            Problem::BadStarting { reason } => match reason {
                StartingFault::NoSuchBlock => part("bad_starting_missing", vec![]),
                StartingFault::CannotHold => part("bad_starting_kind", vec![b()]),
                StartingFault::TooMany { count, most } => {
                    field = Some("starting".to_owned());
                    part(
                        "bad_starting_too_many",
                        vec![
                            b(),
                            ("count", count.to_string()),
                            ("most", most.to_string()),
                        ],
                    )
                }
            },
            Problem::StopWithWarmup => part("stop_with_warmup", vec![b()]),
            Problem::EndedInWarmup { replications, of } => {
                if replications == of {
                    part("ended_in_warmup_all", vec![])
                } else {
                    part(
                        "ended_in_warmup_some",
                        vec![
                            ("replications", replications.to_string()),
                            ("of", of.to_string()),
                        ],
                    )
                }
            }
            Problem::Overloaded { load } => part(
                "overloaded",
                vec![b(), ("load", format!("{:.0}", load * 100.0))],
            ),
            Problem::NoSteadyState {
                backlog,
                left,
                utilisation,
            } => part(
                "no_steady_state",
                vec![
                    b(),
                    ("utilisation", format!("{:.1}", utilisation * 100.0)),
                    ("backlog", format!("{backlog:.0}")),
                    ("left", format!("{left:.0}")),
                ],
            ),
            Problem::OneReplication => part("one_replication", vec![]),
            Problem::NoWarmup {
                warmup,
                shift,
                unit,
            } => match warmup {
                Some(w) => part(
                    "no_warmup_at",
                    vec![
                        b(),
                        ("warmup", num(*w)),
                        ("unit", (*unit).to_owned()),
                        ("shift", num(shift.abs())),
                    ],
                ),
                None => part("no_warmup_late", vec![b(), ("shift", num(shift.abs()))]),
            },
            Problem::EstimateIgnored => part("estimate_ignored", vec![b()]),
            Problem::BadEstimate { error } => part(
                match error {
                    FitError::NotFinite => "bad_estimate_not_finite",
                    FitError::NotInOrder => "bad_estimate_not_in_order",
                    FitError::NoMaximum => "bad_estimate_no_maximum",
                },
                vec![b()],
            ),
            Problem::EstimateMismatch => part("estimate_mismatch", vec![b()]),
            Problem::NormalOftenNegative { parameter } => part(
                "normal_often_negative",
                vec![b(), ("field", field_label(field_key(parameter)).to_owned())],
            ),
            Problem::Behaviour {
                found,
                replications,
                of,
            } => {
                let (key, args, fault): (&'static str, Args, bool) = match found {
                    Behaviour::Unbalanced { came, went } => (
                        "behaviour_unbalanced",
                        vec![b(), ("came", came.to_string()), ("went", went.to_string())],
                        true,
                    ),
                    Behaviour::Unaccounted { came, went } => (
                        "behaviour_unaccounted",
                        vec![("came", came.to_string()), ("went", went.to_string())],
                        true,
                    ),
                    Behaviour::NothingOut { came } => (
                        "behaviour_nothing_out",
                        vec![("came", came.to_string())],
                        false,
                    ),
                    Behaviour::NothingMoved { came } => (
                        "behaviour_nothing_moved",
                        vec![("came", came.to_string())],
                        false,
                    ),
                    Behaviour::LittlesLaw { mean, from_stays } => (
                        "behaviour_littles_law",
                        vec![b(), ("mean", num(*mean)), ("from_stays", num(*from_stays))],
                        true,
                    ),
                    Behaviour::BusyNotWorking { busy, working } => (
                        if busy > working {
                            "behaviour_busy_counted"
                        } else {
                            "behaviour_busy_missed"
                        },
                        vec![b(), ("busy", num(*busy)), ("working", num(*working))],
                        true,
                    ),
                    Behaviour::StatesTotal { total } => (
                        "behaviour_states_total",
                        vec![b(), ("total", num(*total))],
                        true,
                    ),
                    Behaviour::OutOfRange { metric, value } => (
                        "behaviour_out_of_range",
                        vec![
                            b(),
                            ("metric", metric_label(metric).to_owned()),
                            ("value", num(*value)),
                        ],
                        true,
                    ),
                };
                let mut parts = part(key, args);
                if *of > 1 {
                    parts.extend(part(
                        "behaviour_runs",
                        vec![
                            ("replications", replications.to_string()),
                            ("of", of.to_string()),
                        ],
                    ));
                }
                if fault {
                    parts.extend(part("behaviour_report", vec![]));
                }
                parts
            }
        };
        Message { parts, field }
    }
}

/// The key of the field a distribution `parameter` of a block belongs to, as the window labels
/// it ("property value" is a property's value).
fn field_key(parameter: &str) -> &str {
    if parameter == "property value" {
        "value"
    } else {
        parameter
    }
}

/// The inspector's key for the field of a distribution `parameter`, where it has one to focus.
fn distribution_field(parameter: &str) -> Option<String> {
    Some(match parameter {
        "interarrival" | "service" | "duration" | "transfer" | "setup" | "planned_run_time"
        | "patience" => parameter.to_owned(),
        "time_to_failure" => "breakdown/time_to_failure".to_owned(),
        "repair" => "breakdown/repair".to_owned(),
        _ => return None,
    })
}

/// The key of a distribution's own parameter: the BPSim name the distribution reports, as the
/// window's form names it.
fn param_key(parameter: &str) -> &str {
    if parameter == "standardDeviation" {
        "std_dev"
    } else {
        parameter
    }
}
