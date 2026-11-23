//! Whether anything in a process model is drawn at random, read from the model's types.
//!
//! `experiment.optimise` compares designs exactly, one replication each, only when nothing is
//! drawn. Every struct here is taken apart field by field and every enum matched in full, so a
//! field or a kind of element added to the model does not compile until this file says whether
//! it draws. Where a value might be drawn (an exclusive gateway's way, balking, scrap, an
//! expression), it counts as drawn: a false "drawn" only means the designs are compared
//! statistically, a false "not drawn" would claim an exact comparison that is not one.

use stochcraft_model::{
    BalkJoin, Balking, Breakdown, ByResource, ByValue, Change, Connection, Dist, Element,
    ElementId, ElementKind, HealthCheck, Model, Patience, ProcessingBy, ProcessingByResource,
    Property, PropertyValue, ResourcePool, Routing, Setup, Timed, Transfer,
};

/// Whether running `model` draws any random number that can change a result.
pub(crate) fn draws(model: &Model) -> bool {
    let Model {
        name: _,
        elements,
        connections,
        resources,
        // Calendars, units, the run's length and seed, positions, three-point estimates (kept
        // beside the distributions they set), starting contents and a Monte Carlo model's
        // inputs (not part of a process run) draw nothing in a run.
        calendars: _,
        experiment: _,
        layout: _,
        estimates: _,
        starting: _,
        monte_carlo: _,
    } = model;
    elements.iter().any(|e| element(e, connections))
        || connections.iter().any(connection)
        || resources.iter().any(pool)
}

fn dist(d: &Dist) -> bool {
    !matches!(d, Dist::Constant(_))
}

fn timed(t: &[Timed<Dist>]) -> bool {
    t.iter().any(|t| dist(&t.value))
}

fn element(e: &Element, connections: &[Connection]) -> bool {
    let Element {
        id,
        name: _,
        kind,
        properties,
    } = e;
    properties.iter().any(property) || kind_draws(id, kind, connections)
}

fn property(p: &Property) -> bool {
    let Property {
        name: _,
        ty: _,
        value,
    } = p;
    match value {
        PropertyValue::Number(_) | PropertyValue::Boolean(_) | PropertyValue::Text(_) => false,
        PropertyValue::Random(d) => dist(d),
        PropertyValue::Choice(values) => values.len() > 1,
        // An expression may call a distribution.
        PropertyValue::Expression(_) => true,
    }
}

fn kind_draws(id: &ElementId, kind: &ElementKind, connections: &[Connection]) -> bool {
    match kind {
        ElementKind::Source {
            interarrival,
            limit: _,
            timed: by_calendar,
            idle_outside: _,
        } => dist(interarrival) || timed(by_calendar),
        ElementKind::Server {
            capacity: _,
            service,
            processing_by,
            queue_capacity: _,
            queue_limit: _,
            breakdown,
            transfer,
            available: _,
            availability: _,
            shifts: _,
            on_shift_end: _,
            power: _,
            setup,
            planned_run_time,
            patience,
            balking,
            health_check,
        } => {
            dist(service)
                || processing_by.as_ref().is_some_and(by_value)
                || breakdown.as_ref().is_some_and(fails)
                || transfer.as_ref().is_some_and(moves)
                || setup.as_ref().is_some_and(sets_up)
                || planned_run_time.as_ref().is_some_and(dist)
                || patience.as_ref().is_some_and(waits)
                || balking.as_ref().is_some_and(balks)
                || health_check.as_ref().is_some_and(checks)
        }
        ElementKind::SharedTask {
            service,
            processing_by,
            processing_by_resource,
            needs: _,
            priority: _,
        } => {
            dist(service)
                || processing_by.as_ref().is_some_and(by_value)
                || processing_by_resource.as_ref().is_some_and(by_resource)
        }
        ElementKind::Delay {
            duration,
            processing_by,
        } => dist(duration) || processing_by.as_ref().is_some_and(by_value),
        // Scrap counts as drawn: whether an entity is scrap may come from a drawn way.
        ElementKind::Sink { scrap, demand: _ } => scrap.is_some(),
        ElementKind::ExclusiveGateway {
            default: _,
            routing,
        } => match routing {
            // The shortest queue, ties to the way taken longest ago: nothing drawn.
            Routing::ShortestQueue => false,
            // A condition, else the default; with no conditions the way is drawn. Drawn unless
            // every way out has a condition or is the only one.
            Routing::Standard | Routing::Divert => {
                let out: Vec<&Connection> = connections.iter().filter(|c| &c.from == id).collect();
                out.len() > 1 && out.iter().any(|c| c.condition.is_none())
            }
        },
        ElementKind::ParallelGateway | ElementKind::Unbatch => false,
        ElementKind::Batch { size: _ } => false,
        ElementKind::Conveyor {
            length: _,
            speed: _,
            accumulating: _,
            item_length: _,
            gap: _,
            shape: _,
        } => false,
        ElementKind::Transport {
            pool: _,
            load,
            unload,
        } => dist(load) || dist(unload),
    }
}

fn by_value(p: &ProcessingBy) -> bool {
    let ProcessingBy {
        property: _,
        by_value,
    } = p;
    by_value.iter().any(|v| {
        let ByValue { value: _, time } = v;
        dist(time)
    })
}

fn by_resource(p: &ProcessingByResource) -> bool {
    let ProcessingByResource { by_resource } = p;
    by_resource.iter().any(|r| {
        let ByResource { resource: _, time } = r;
        dist(time)
    })
}

fn fails(b: &Breakdown) -> bool {
    let Breakdown {
        time_to_failure,
        repair,
        interruptible: _,
        hands_on_while_down: _,
    } = b;
    dist(time_to_failure) || dist(repair)
}

fn moves(t: &Transfer) -> bool {
    let Transfer {
        time,
        holds_sender: _,
    } = t;
    dist(time)
}

fn sets_up(s: &Setup) -> bool {
    let Setup {
        time,
        on_change_of: _,
        by_change,
    } = s;
    dist(time)
        || by_change.iter().any(|c| {
            let Change {
                from: _,
                to: _,
                time,
            } = c;
            dist(time)
        })
}

fn waits(p: &Patience) -> bool {
    let Patience { time, until: _ } = p;
    dist(time)
}

fn balks(b: &Balking) -> bool {
    let Balking {
        join,
        counts_in_service: _,
    } = b;
    match join {
        // A row's probability of 0 or 1 draws nothing.
        BalkJoin::Table(rows) => rows
            .iter()
            .any(|r| r.probability > 0.0 && r.probability < 1.0),
        BalkJoin::Reciprocal => true,
    }
}

fn checks(h: &HealthCheck) -> bool {
    let HealthCheck {
        interval,
        limit: _,
        restart,
        keep: _,
        on_takeout: _,
        place_while_out: _,
    } = h;
    dist(interval) || dist(restart)
}

fn connection(c: &Connection) -> bool {
    let Connection {
        id: _,
        from: _,
        to: _,
        probability,
        condition: _,
        quantity: _,
        rework: _,
        abandoned: _,
        turned_away: _,
        balked: _,
    } = c;
    // A probability strictly between 0 and 1 is a draw wherever it is.
    probability.is_some_and(|p| p > 0.0 && p < 1.0)
}

fn pool(p: &ResourcePool) -> bool {
    let ResourcePool {
        id: _,
        name: _,
        quantity: _,
        roles: _,
        shifts: _,
        available: _,
        availability: _,
        mobility: _,
    } = p;
    false
}
