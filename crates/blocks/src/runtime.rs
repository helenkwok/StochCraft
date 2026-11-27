//! The simulation behind a compiled model: one store per element input, processes per element,
//! and the statistics each element keeps.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::{Rc, Weak};

use stochcraft_des::{
    Ctx, Gate, InterruptCause, Resource, ResourceGuard, Signal, Sim, Stopped, Store, Watch,
    Watching,
};
use stochcraft_model::calendar::{self, Calendar, DateTime};
use stochcraft_model::{
    Balking, Breakdown, Demand, DemandPer, Element, ElementId, ElementKind, HealthCheck, Model,
    OnShiftEnd, OnTakeout, PatienceUntil, PlaceWhileOut, Power, QueueLimit, Requirement,
    ResourcePool, Routing, Setup, Timed,
};
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::{Tally, TimeWeighted};

use crate::series::{Quantity, Recording};
use crate::{
    Assign, Compiled, Decide, ElementReport, Entity, Held, Report, Route, RunError, Source,
};

/// What an element assigns to entities as they enter, and the stream it draws values from; and
/// the decision table it evaluates then, after the assignments.
struct Props {
    assigns: Vec<Assign>,
    rng: RefCell<Rng>,
    decide: Option<Decide>,
}

/// The model's property slots and each element's assignments, shared by the ports.
struct Assigner {
    slots: Vec<String>,
    elements: Vec<Props>,
}

impl Assigner {
    /// The value of property `name` that `e` holds.
    fn read(&self, e: &Entity, name: &str) -> Option<stochcraft_model::expr::Value> {
        let slot = self.slots.binary_search_by(|s| s.as_str().cmp(name)).ok()?;
        e.properties.get(slot)?.as_ref().map(Held::value)
    }

    /// Makes element `index`'s assignments to `e`, all from the values it arrived with, and
    /// records the values in the element's statistics; then evaluates its decision table, if it
    /// has one, on the values after them, and writes the outputs (a null result clears them).
    fn enter(&self, index: usize, e: &mut Entity, stats: &[RefCell<BlockStats>]) {
        let Some(props) = self.elements.get(index) else {
            return;
        };
        if props.assigns.is_empty() && props.decide.is_none() {
            return;
        }
        if e.properties.len() < self.slots.len() {
            e.properties.resize(self.slots.len(), None);
        }
        self.assign(props, index, e, stats);
        if let Some(d) = &props.decide {
            let values = {
                let read = |name: &str| {
                    let slot = self.slots.binary_search_by(|s| s.as_str().cmp(name)).ok()?;
                    e.properties.get(slot)?.as_ref().map(Held::value)
                };
                d.table.properties(&read)
            };
            let mut s = stats.get(index).map(RefCell::borrow_mut);
            for ((to, tally), v) in d
                .outputs
                .iter()
                .zip(&d.tally)
                .zip(values.unwrap_or_default())
            {
                if let Some((slot, ty)) = to
                    && let Some(p) = e.properties.get_mut(*slot)
                {
                    *p = v.as_ref().and_then(|v| Held::of(v, *ty));
                    if let (Some(s), Some(k), Some(x)) =
                        (&mut s, tally, p.as_ref().and_then(Held::statistic))
                        && let Some(t) = s.properties.get_mut(*k)
                    {
                        t.add(x);
                    }
                }
            }
        }
    }

    /// Element `index`'s own assignments to `e` (see [`Assigner::enter`]).
    fn assign(&self, props: &Props, index: usize, e: &mut Entity, stats: &[RefCell<BlockStats>]) {
        if props.assigns.is_empty() {
            return;
        }
        let read = |name: &str| {
            let slot = self.slots.binary_search_by(|s| s.as_str().cmp(name)).ok()?;
            e.properties.get(slot)?.as_ref().map(Held::value)
        };
        let new: Vec<Option<Held>> = props
            .assigns
            .iter()
            .map(|a| match &a.source {
                Source::Fixed(v) => v.clone(),
                Source::Random(d) => {
                    let x = d.sample(&mut props.rng.borrow_mut());
                    Held::of(&stochcraft_model::expr::Value::Number(x), a.ty)
                }
                Source::Choice(pick, values) => {
                    let i = pick.sample(&mut props.rng.borrow_mut()) as usize;
                    let text = values.get(i).cloned().unwrap_or_default();
                    Held::of(&stochcraft_model::expr::Value::Text(text), a.ty)
                }
                Source::Expr(x) => Held::of(&x.eval(&read), a.ty),
            })
            .collect();
        let mut s = stats.get(index).map(RefCell::borrow_mut);
        for (k, (a, v)) in props.assigns.iter().zip(new).enumerate() {
            if let (Some(s), Some(x)) = (&mut s, v.as_ref().and_then(Held::statistic))
                && let Some(t) = s.properties.get_mut(k)
            {
                t.add(x);
            }
            if let Some(slot) = e.properties.get_mut(a.slot) {
                *slot = v;
            }
        }
    }
}

impl Compiled {
    /// Runs one replication for the experiment's warm-up plus duration, with random numbers from
    /// `rng` (each element draws from its own streams, keyed by its id and their purpose, so adding,
    /// removing or reordering elements leaves the others' draws as they were; property values
    /// come from streams of their own, so properties never change the other draws), and reports
    /// the statistics collected after the warm-up.
    ///
    /// A model whose entities can go round a loop without simulated time moving would never
    /// end; past [`Compiled::max_events_at_one_time`] events at one time the replication stops
    /// with [`RunError::TimeStopped`] instead.
    pub fn run(&self, rng: Rng) -> Result<Report, RunError> {
        self.run_watched(rng, &|| false)
    }

    /// As [`Compiled::run`], asking `cancelled` every so often (every thousand or so events)
    /// whether to stop, so that even a replication that would take very long, or never end,
    /// can be cancelled; it then returns [`RunError::Cancelled`]. Asking changes nothing else:
    /// a replication that is not cancelled gives exactly what [`Compiled::run`] gives.
    pub fn run_watched(&self, rng: Rng, cancelled: &dyn Fn() -> bool) -> Result<Report, RunError> {
        let x = &self.model.experiment;
        self.simulate(rng, x.warmup, x.duration, cancelled)
    }

    /// As [`Compiled::run`], with an explicit warm-up and duration (negative or non-finite values
    /// count as 0).
    pub fn run_for(&self, rng: Rng, warmup: f64, duration: f64) -> Result<Report, RunError> {
        self.simulate(rng, warmup, duration, &|| false)
    }

    fn simulate(
        &self,
        rng: Rng,
        warmup: f64,
        duration: f64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Report, RunError> {
        let mut live = self.start(rng, warmup, duration);
        let end = live.end;
        live.advance_watched(end, cancelled, u64::MAX)?;
        Ok(live.report())
    }

    /// One replication set up to run step by step (a [`Live`] run), exactly as
    /// [`Compiled::run`] would run it with `rng`.
    pub fn live(&self, rng: Rng) -> Live<'static> {
        let x = &self.model.experiment;
        let live = self.start(rng, x.warmup, x.duration);
        Live {
            compiled: Cow::Owned(self.clone()),
            ..live
        }
    }

    fn start(&self, mut rng: Rng, warmup: f64, duration: f64) -> Live<'_> {
        let clean = |t: f64| if t.is_finite() && t > 0.0 { t } else { 0.0 };
        let (warmup, duration) = (clean(warmup), clean(duration));
        let elements = &self.model.elements;
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let stats: Rc<Vec<RefCell<BlockStats>>> = Rc::new(
            self.reported
                .iter()
                .map(|a| RefCell::new(BlockStats::new(0.0, a.len())))
                .collect(),
        );
        let inputs: Vec<Vec<Store<Entity>>> = self
            .inputs
            .iter()
            .map(|&n| (0..n.max(1)).map(|_| Store::new(&ctx)).collect())
            .collect();
        let end = warmup + duration;
        let calendars = Calendars::new(&self.model, end);
        // A finite buffer's places: one per unit plus the waiting room. An entity holds one from
        // the moment it gets in (waiting, in service, or blocked on its way out) until it has
        // left, so a sender finding none free is blocked (or, for a source, loses its arrival).
        // A waiting room by timetable or per working server holds no places: a limit on the
        // number waiting instead ([`QueueLimit`]).
        let places: Vec<Option<Rc<Places>>> = elements
            .iter()
            .enumerate()
            .map(|(i, e)| match &e.kind {
                ElementKind::Server {
                    queue_capacity: Some(q),
                    queue_limit,
                    ..
                } => {
                    let room = match queue_limit {
                        // Every unit's place, on shift or not.
                        QueueLimit::Fixed => Room::Fixed(Resource::new(
                            &ctx,
                            u64::from(e.kind.most_units().unwrap_or(0)) + u64::from(*q),
                        )),
                        QueueLimit::ByShift(timed) => {
                            let steps = calendars
                                .timeline(timed)
                                .into_iter()
                                .map(|(t, k)| {
                                    (t, k.and_then(|k| timed.get(k)).map_or(*q, |x| x.value))
                                })
                                .collect();
                            Room::Limit(Limit::new(&ctx, i, &stats, LimitRule::ByShift(steps)))
                        }
                        QueueLimit::PerWorkingServer => Room::Limit(Limit::new(
                            &ctx,
                            i,
                            &stats,
                            LimitRule::PerWorkingServer(*q),
                        )),
                    };
                    Some(Rc::new(Places {
                        room,
                        held: RefCell::new(BTreeMap::new()),
                        watchers: RefCell::new(Vec::new()),
                        away: RefCell::new(Weak::new()),
                    }))
                }
                // A conveyor's entrance: as many places as pitches fit along it.
                ElementKind::Conveyor { .. } => e.kind.conveyor_places().map(|n| {
                    Rc::new(Places {
                        room: Room::Belt(Belt {
                            places: n,
                            on: RefCell::new(BTreeMap::new()),
                            free: Cell::new(true),
                            changed: Signal::new(&ctx),
                        }),
                        held: RefCell::new(BTreeMap::new()),
                        watchers: RefCell::new(Vec::new()),
                        away: RefCell::new(Weak::new()),
                    })
                }),
                _ => None,
            })
            .collect();
        // A limit by timetable that rises lets in those waiting for room then.
        for p in places.iter().flatten() {
            if let Room::Limit(Limit {
                rule: LimitRule::ByShift(steps),
                ..
            }) = &p.room
                && steps.len() > 1
            {
                let (c, p, times) = (
                    ctx.clone(),
                    Rc::clone(p),
                    steps.iter().skip(1).map(|(t, _)| *t).collect::<Vec<f64>>(),
                );
                sim.spawn(async move {
                    for t in times {
                        let now = c.now();
                        c.hold(t - now).await;
                        p.notify();
                    }
                });
            }
        }
        // Shortest-queue and divert gateways: an element sending into one routes through it
        // itself, so what it sends to is the gateway's ways.
        let routes_itself = |i: usize| {
            elements.get(i).is_some_and(|e| {
                matches!(
                    e.kind,
                    ElementKind::ExclusiveGateway {
                        routing: Routing::ShortestQueue | Routing::Divert,
                        ..
                    }
                )
            })
        };
        // Where an element's units send what they finish (not a patient server's way for those
        // who give up, which they take by themselves).
        let sends_to = |i: usize| -> Vec<usize> {
            self.routes
                .get(i)
                .into_iter()
                .flatten()
                .filter(|r| !r.abandoned && !r.turned_away && !r.balked)
                .flat_map(|r| {
                    if routes_itself(r.to) {
                        self.routes
                            .get(r.to)
                            .into_iter()
                            .flatten()
                            .map(|w| w.to)
                            .collect()
                    } else {
                        vec![r.to]
                    }
                })
                .collect()
        };
        let feeds_buffer: Vec<bool> = (0..elements.len())
            .map(|i| {
                sends_to(i)
                    .into_iter()
                    .any(|t| places.get(t).is_some_and(Option::is_some))
            })
            .collect();
        // Whether each element sends across a transfer that holds a server's unit.
        let feeds_transfer: Vec<bool> = (0..elements.len())
            .map(|i| {
                sends_to(i).into_iter().any(|t| {
                    elements.get(t).is_some_and(|e| {
                        matches!(
                            &e.kind,
                            ElementKind::Server {
                                transfer: Some(t),
                                ..
                            } if t.holds_sender
                        )
                    })
                })
            })
            .collect();
        let units: Vec<Vec<(f64, u32)>> = self
            .model
            .resources
            .iter()
            .map(|r| calendars.units(r))
            .collect();
        let pools = Rc::new(Pools::new(
            &ctx,
            units
                .iter()
                .map(|u| u.first().map_or(0, |(_, n)| *n))
                .collect(),
        ));
        for (p, changes) in units.into_iter().enumerate() {
            if changes.len() > 1 {
                spawn_shifts(&mut sim, Rc::clone(&pools), p, changes);
            }
        }
        let next_id = Rc::new(RefCell::new(0u64));
        // Every stream is keyed by its element's id and its purpose, never by its position, so
        // adding, removing or reordering an element changes no other element's draws.
        let base = rng.next_u64();
        let stream = |purpose: &str, i: usize| {
            let id = elements.get(i).map_or("", |e| e.id.as_str());
            Rng::keyed(base, &format!("{purpose}/{id}"))
        };
        let streams: Vec<Rng> = (0..elements.len()).map(|i| stream("element", i)).collect();
        // A stream for each server's transfer times.
        let transfers: Vec<Option<Rc<Transfer>>> = elements
            .iter()
            .enumerate()
            .map(|(i, e)| match &e.kind {
                ElementKind::Server {
                    transfer: Some(t), ..
                } => Some(Rc::new(Transfer {
                    time: t.time.clone(),
                    holds_sender: t.holds_sender,
                    rng: RefCell::new(stream("transfer", i)),
                })),
                _ => None,
            })
            .collect();
        // A stream for each server's patience.
        let patients: Vec<Option<Rc<Patient>>> = elements
            .iter()
            .enumerate()
            .map(|(i, e)| match &e.kind {
                ElementKind::Server {
                    patience: Some(p), ..
                } => Some(Rc::new(Patient {
                    index: i,
                    until: p.until,
                    time: p.time.clone(),
                    rng: RefCell::new(stream("patience", i)),
                    ctx: ctx.clone(),
                    input: inputs.get(i)?.first()?.clone(),
                    places: places.get(i).cloned().flatten(),
                    stats: Rc::clone(&stats),
                    way: RefCell::new(None),
                    next_id: Rc::clone(&next_id),
                    stays: RefCell::new(BTreeMap::new()),
                    visits: Cell::new(0),
                    held: RefCell::new(None),
                })),
                _ => None,
            })
            .collect();
        // A stream for each server's balking (drawn only for a chance strictly between 0 and
        // 1), so models without balking draw exactly as before. The servers' processes do not
        // hold these: the ports into each server hold them weakly, and the run keeps them.
        let balks: Vec<Option<Rc<Balk>>> = elements
            .iter()
            .enumerate()
            .map(|(i, e)| match &e.kind {
                ElementKind::Server {
                    balking: Some(b), ..
                } => Some(Rc::new(Balk {
                    index: i,
                    balking: b.clone(),
                    rng: RefCell::new(stream("balking", i)),
                    ctx: ctx.clone(),
                    stats: Rc::clone(&stats),
                    way: RefCell::new(None),
                })),
                _ => None,
            })
            .collect();
        let assigner = Rc::new(Assigner {
            slots: self.slots.clone(),
            elements: self
                .assigns
                .iter()
                .zip(&self.decides)
                .enumerate()
                .map(|(i, (a, d))| Props {
                    assigns: a.clone(),
                    rng: RefCell::new(stream("properties", i)),
                    decide: d.clone(),
                })
                .collect(),
        });

        // The way into element `r.to`, as itself (not routed through, even if it is a
        // shortest-queue gateway).
        let plain_port = |r: &Route| -> Option<Port> {
            let store = inputs.get(r.to)?.get(r.input)?.clone();
            Some(Port {
                store,
                stats: Rc::clone(&stats),
                assigner: Rc::clone(&assigner),
                index: r.to,
                places: places.get(r.to).cloned().flatten(),
                // Across a junction between conveyors, its transfer time (L5).
                transfer: match r.junction {
                    Some(t) => Some(Rc::new(Transfer {
                        time: Dist::Constant(t),
                        holds_sender: false,
                        rng: RefCell::new(stream("junction", r.to)),
                    })),
                    None => transfers.get(r.to).cloned().flatten(),
                },
                router: None,
                rework: r.rework,
                patient: patients
                    .get(r.to)
                    .cloned()
                    .flatten()
                    .map(|p| Rc::downgrade(&p)),
                balk: balks
                    .get(r.to)
                    .cloned()
                    .flatten()
                    .map(|b| Rc::downgrade(&b)),
            })
        };
        let routers: Vec<Option<Rc<Router>>> = (0..elements.len())
            .map(|g| {
                routes_itself(g).then(|| {
                    let routes: Vec<&Route> = self.routes.get(g).into_iter().flatten().collect();
                    let ways: Vec<Port> = routes.iter().filter_map(|r| plain_port(r)).collect();
                    let divert = elements.get(g).is_some_and(|e| {
                        matches!(
                            e.kind,
                            ElementKind::ExclusiveGateway {
                                routing: Routing::Divert,
                                ..
                            }
                        )
                    });
                    let rule = if divert {
                        Rule::Divert(Divert {
                            outs: routes
                                .iter()
                                .zip(&ways)
                                .map(|(r, p)| Out {
                                    port: p.clone(),
                                    weight: r.weight,
                                    condition: r.condition.clone(),
                                    default: r.default,
                                })
                                .collect(),
                            assigner: Rc::clone(&assigner),
                            rng: RefCell::new(stream("divert", g)),
                        })
                    } else {
                        Rule::ShortestQueue
                    };
                    Rc::new(Router::new(&ctx, g, ways, rule))
                })
            })
            .collect();
        // Each finite waiting room's way out for those turned away, if it has one. Kept here
        // (the rooms hold them weakly, since a way can lead back to its own room).
        let mut aways: Vec<Rc<Away>> = Vec::new();
        // What colours the items on a conveyor, and a view of each conveyor's items.
        let product = Product::of(&self.model, &self.slots).map(Rc::new);
        let mut belts: Vec<Option<Rc<BeltView>>> = vec![None; elements.len()];
        for ((i, element), rng) in elements.iter().enumerate().zip(streams) {
            let routed = |r: &Route| {
                let mut port = plain_port(r)?;
                port.router = routers.get(r.to).cloned().flatten();
                Some(port)
            };
            let routes = self.routes.get(i).into_iter().flatten();
            let outs: Vec<Out> = routes
                .clone()
                .filter(|r| !r.abandoned && !r.turned_away && !r.balked)
                .filter_map(|r| {
                    Some(Out {
                        port: routed(r)?,
                        weight: r.weight,
                        condition: r.condition.clone(),
                        default: r.default,
                    })
                })
                .collect();
            let patient = patients.get(i).cloned().flatten();
            if let Some(p) = &patient {
                *p.way.borrow_mut() = routes.clone().filter(|r| r.abandoned).find_map(routed);
            }
            if let (Some(way), Some(p)) = (
                routes.clone().filter(|r| r.turned_away).find_map(routed),
                places.get(i).and_then(Option::as_ref),
            ) {
                let away = Rc::new(Away {
                    index: i,
                    ctx: ctx.clone(),
                    way,
                });
                *p.away.borrow_mut() = Rc::downgrade(&away);
                aways.push(away);
            }
            if let Some(b) = balks.get(i).and_then(Option::as_ref) {
                *b.way.borrow_mut() = routes.clone().filter(|r| r.balked).find_map(routed);
            }
            // Divert points along a conveyor (L5): the diverts its items meet at `s`, each `s`
            // taken as the same fraction of the run's length as of the track's (so a typed
            // length that overrides the track's stretches them with it); and where its items
            // go at its end, the last point's default way.
            let points: Vec<BeltPoint> = match &element.kind {
                ElementKind::Conveyor { length, .. } => {
                    let unit = self.model.experiment.length_unit;
                    let track = self
                        .model
                        .site
                        .paths
                        .get(&element.id)
                        .map(|t| t.length(unit));
                    let scale = match track {
                        Some(t) if t > 0.0 && t != *length => length / t,
                        _ => 1.0,
                    };
                    // The flow into each point: from the conveyor for the first, from the
                    // point before's default for the rest; rework if it is marked so.
                    let mut from = i;
                    stochcraft_model::site::points(&self.model, &element.id)
                        .into_iter()
                        .filter_map(|p| {
                            let g = elements.iter().position(|e| e.id == p.gateway)?;
                            let rework = self.routes.get(from).is_some_and(|rs| {
                                rs.iter()
                                    .any(|r| r.to == g && r.rework && (from == i || r.default))
                            });
                            from = g;
                            Some(BeltPoint {
                                s: (p.s * scale).max(0.0),
                                gateway: g,
                                router: routers.get(g).cloned().flatten()?,
                                rework,
                            })
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            let end_way = points.last().and_then(|p| {
                self.routes
                    .get(p.gateway)?
                    .iter()
                    .find(|r| r.default)
                    .and_then(&routed)
            });
            let Some(ins) = inputs.get(i).cloned() else {
                continue;
            };
            let Some(input) = ins.first().cloned() else {
                continue;
            };
            let env = BlockEnv {
                ctx: ctx.clone(),
                stats: Rc::clone(&stats),
                assigner: Rc::clone(&assigner),
                me: i,
                places: places.get(i).cloned().flatten(),
                input,
                out: if points.is_empty() {
                    outs.first().map(|o| o.port.clone())
                } else {
                    end_way
                },
                rng,
                holds: matches!(
                    element.kind,
                    ElementKind::Server { .. } | ElementKind::SharedTask { .. }
                ),
                patient,
            };
            match element.kind.clone() {
                ElementKind::Source {
                    interarrival,
                    limit,
                    timed,
                    idle_outside,
                } => {
                    let rates = calendars.arrivals(interarrival, &timed, idle_outside);
                    spawn_source(
                        &mut sim,
                        env,
                        rates,
                        limit,
                        Rc::clone(&next_id),
                        self.first_arrival_at_zero,
                    );
                }
                ElementKind::Server {
                    capacity,
                    service,
                    processing_by,
                    breakdown,
                    available,
                    availability,
                    setup,
                    health_check,
                    ref shifts,
                    on_shift_end,
                    ..
                } => {
                    // Every unit the timetable ever has on shift, and how many are, over time.
                    let most = element.kind.most_units().unwrap_or(capacity);
                    let shifts = calendars.units_over(capacity, shifts, available, &availability);
                    let always = matches!(shifts.as_slice(), [(_, n)] if *n == most);
                    let service = Times::new(service, processing_by, &self.slots, &env.rng);
                    // A changeover reads the part's property from its slot.
                    let setup = setup.map(|setup| Setups {
                        slot: setup.on_change_of.as_deref().and_then(|name| {
                            self.slots.binary_search_by(|s| s.as_str().cmp(name)).ok()
                        }),
                        setup,
                    });
                    if breakdown.is_some() || !always || setup.is_some() || health_check.is_some() {
                        let parts = UnitServer {
                            capacity: most,
                            service,
                            breakdown,
                            shifts,
                            on_shift_end,
                            setup,
                            health: health_check,
                        };
                        spawn_unit_server(&mut sim, env, parts);
                    } else {
                        spawn_server(&mut sim, env, most, service);
                    }
                }
                ElementKind::SharedTask {
                    service,
                    processing_by,
                    processing_by_resource,
                    needs,
                    priority,
                } => {
                    // The pools with an entry, by index in the model's resources (an entry for a
                    // pool the model lacks is refused by validation).
                    let by_pool: Vec<(usize, ElementId, Dist)> = processing_by_resource
                        .into_iter()
                        .flat_map(|x| x.by_resource)
                        .filter_map(|x| {
                            let at = self
                                .model
                                .resources
                                .iter()
                                .position(|r| r.id == x.resource)?;
                            Some((at, x.resource, x.time))
                        })
                        .collect();
                    let service =
                        Times::with_pools(service, processing_by, &by_pool, &self.slots, &env.rng);
                    let request = Rc::new(Request {
                        requirements: requirements(&self.model, &needs),
                        priority,
                    });
                    spawn_shared(&mut sim, env, service, request, Rc::clone(&pools));
                }
                ElementKind::Delay {
                    duration,
                    processing_by,
                } => {
                    let duration = Times::new(duration, processing_by, &self.slots, &env.rng);
                    spawn_delay(&mut sim, env, duration)
                }
                ElementKind::Sink {
                    scrap,
                    time_in_system,
                    ..
                } => {
                    // Scrap counts against the server it names, else each entity's last one.
                    let scrap = scrap.map(|s| {
                        s.charged_to
                            .and_then(|c| elements.iter().position(|x| x.id == c))
                    });
                    // Times in system on the clock, or in the calendar's active time.
                    let clock = time_in_system.map(|x| match x.calendar {
                        Some(c) => Clock::Working(Active::new(
                            calendars.periods.get(&c).map_or(&[][..], Vec::as_slice),
                        )),
                        None => Clock::Wall,
                    });
                    spawn_sink(&mut sim, env, scrap, clock);
                }
                ElementKind::ExclusiveGateway { .. } => {
                    let router = routers.get(i).cloned().flatten();
                    spawn_exclusive(&mut sim, env, outs, router)
                }
                ElementKind::ParallelGateway => {
                    let quantities = self.quantities.get(i).cloned().unwrap_or_default();
                    spawn_parallel(&mut sim, env, ins, quantities, outs, Rc::clone(&next_id))
                }
                ElementKind::Batch { size } => spawn_batch(&mut sim, env, size),
                ElementKind::Unbatch => spawn_unbatch(&mut sim, env),
                ElementKind::Conveyor {
                    length,
                    speed,
                    accumulating,
                    item_length,
                    gap,
                    ..
                } => {
                    let view = spawn_conveyor(
                        &mut sim,
                        env,
                        length,
                        speed,
                        accumulating,
                        item_length + gap,
                        product.clone(),
                        points,
                    );
                    if let Some(slot) = belts.get_mut(i) {
                        *slot = Some(view);
                    }
                }
                // Not simulated yet (`compile` refuses a model with one): passes entities on.
                ElementKind::Transport { .. } => spawn_unbatch(&mut sim, env),
            }
        }

        // Starting contents (ROADMAP M2.9, #225): entities already inside blocks as the run
        // starts, instead of an empty start. Each goes into its block's queue now, at time 0,
        // the way an arrival from the block before it would (its block's properties assigned,
        // counted as entering, no transfer), before any process has run: the block's free units
        // take them first, in order, each with a fresh service time from the block's own stream,
        // and the rest wait. In a fixed finite waiting room each takes a place, as an arrival
        // does; a limit by timetable or per working unit holds no places. The model check keeps
        // them within what an arrival at time 0 would find room for (`Model::most_at_start`).
        // Nothing else is drawn here, so a model
        // without starting contents runs exactly as before, and no other block's draws move.
        for (i, _, count) in self.model.started_blocks() {
            let Some(port) = plain_port(&Route {
                to: i,
                input: 0,
                weight: 1.0,
                condition: None,
                default: false,
                rework: false,
                abandoned: false,
                turned_away: false,
                balked: false,
                junction: None,
            }) else {
                continue;
            };
            for _ in 0..count {
                let id = *next_id.borrow();
                if let Some(Room::Fixed(pool)) = port.places.as_ref().map(|p| &p.room) {
                    let Some(guard) = pool.try_acquire() else {
                        break;
                    };
                    if let Some(p) = &port.places {
                        p.enter(id, guard);
                    }
                }
                *next_id.borrow_mut() += 1;
                if let Some(s) = stats.get(i) {
                    s.borrow_mut().started += 1;
                }
                let now = ctx.now();
                port.send(
                    now,
                    Entity {
                        id,
                        created: now,
                        entered: now,
                        properties: Vec::new(),
                        members: Vec::new(),
                        last_server: None,
                    },
                );
            }
        }

        // Discard everything before the warm-up. With none, there is nothing to discard (and a
        // reset at time 0 would discard an arrival at time 0). A run that ends first (its stop
        // condition met) never resets: `reset_at` says whether, and when, it did.
        let reset_at = Rc::new(Cell::new((warmup <= 0.0).then_some(0.0)));
        if warmup > 0.0 {
            let (c, st, pl, at) = (
                ctx.clone(),
                Rc::clone(&stats),
                Rc::clone(&pools),
                Rc::clone(&reset_at),
            );
            sim.spawn(async move {
                c.hold(warmup).await;
                let now = c.now();
                st.iter().for_each(|s| s.borrow_mut().reset(now));
                pl.reset(now);
                at.set(Some(now));
            });
        }

        let stop = self.model.experiment.stop.as_ref().and_then(|s| {
            let i = elements.iter().position(|e| &e.id == s.element())?;
            Some((i, s.clone()))
        });
        Live {
            compiled: Cow::Borrowed(self),
            sim,
            stats,
            pools,
            _aways: aways,
            _balks: balks.into_iter().flatten().collect(),
            belts,
            feeds_buffer,
            feeds_transfer,
            warmup,
            duration,
            end,
            stop,
            stopped_at: None,
            reset_at,
            watching: Watching::new(0.0),
            recording: self.record.map(|interval| Recording {
                interval,
                times: Vec::new(),
                elements: elements
                    .iter()
                    .map(|e| Quantity::of_kind(&e.kind))
                    .chain(self.model.resources.iter().map(|_| Quantity::POOL))
                    .map(|qs| qs.iter().map(|q| (*q, Vec::new())).collect())
                    .collect(),
            }),
        }
    }

    /// The error for a replication that stopped early. Element processes are tagged with the
    /// element's index ([`tag`]).
    fn run_error(&self, stopped: Stopped) -> RunError {
        match stopped {
            Stopped::Cancelled { .. } => RunError::Cancelled,
            Stopped::TimeStopped { time, events, tag } => RunError::TimeStopped {
                time,
                events,
                element: self.tagged(tag),
            },
            Stopped::Unschedulable { time, tag } => RunError::TimeOutOfRange {
                time,
                element: self.tagged(tag),
            },
        }
    }

    /// The id and name of the element whose processes have tag `tag`.
    fn tagged(&self, tag: Option<u64>) -> Option<(stochcraft_model::ElementId, String)> {
        tag.and_then(|t| usize::try_from(t).ok())
            .and_then(|i| self.model.elements.get(i))
            .map(|e| (e.id.clone(), e.name.clone()))
    }
}

/// One replication run step by step, for watching it: advance it to a time or by one event, and
/// read each block's state at that moment. Run to its end it gives exactly the [`Report`]
/// [`Compiled::run`] gives with the same random stream.
pub struct Live<'a> {
    compiled: Cow<'a, Compiled>,
    sim: Sim,
    stats: Rc<Vec<RefCell<BlockStats>>>,
    pools: Rc<Pools>,
    /// The ways out for entities turned away, which the finite waiting rooms hold weakly.
    _aways: Vec<Rc<Away>>,
    /// Each server's balking, which the ports into it hold weakly.
    _balks: Vec<Rc<Balk>>,
    /// For each element, a view of the items on it when it is a conveyor.
    belts: Vec<Option<Rc<BeltView>>>,
    feeds_buffer: Vec<bool>,
    feeds_transfer: Vec<bool>,
    warmup: f64,
    duration: f64,
    end: f64,
    /// The model's stop condition: the index of the element it watches, and the condition.
    stop: Option<(usize, stochcraft_model::Stop)>,
    /// When the stop condition was met, if it has been (the run then ends there).
    stopped_at: Option<f64>,
    /// When the statistics were last reset: 0 with no warm-up, the warm-up's end once it has
    /// ended, `None` while it lasts (and for good, when the run stopped during it).
    reset_at: Rc<Cell<Option<f64>>>,
    /// How far the simulation has got, kept between advances so that a run in slices
    /// ([`Live::advance_by`]) counts the events at one time across them.
    watching: Watching,
    /// The values recorded so far, when the compiled model records them
    /// ([`Compiled::record_every`]).
    recording: Option<Recording>,
}

/// A block's state at one moment of a [`Live`] run. Counts are since the warm-up ended (since
/// the start while it lasts).
#[derive(Debug, Clone, PartialEq)]
pub struct LiveState {
    pub id: stochcraft_model::ElementId,
    pub entered: u64,
    pub left: u64,
    pub lost: u64,
    /// Entities waiting in the block's input now (tasks with a resource or shared resources).
    pub waiting: f64,
    /// Units busy (a task with a resource), entities inside (other tasks), or a pool's units
    /// in use, now.
    pub busy: f64,
    /// Units the block or pool has now: a task's capacity, a pool's units in its current
    /// shift; none for blocks without units.
    pub units: Option<f64>,
    /// Units down for repair now (servers with breakdowns).
    pub down: f64,
}

impl Live<'_> {
    /// The simulated time now.
    pub fn now(&self) -> f64 {
        self.sim.now()
    }

    /// When the replication ends: its warm-up plus its duration, or the time its stop
    /// condition was met (see [`Live::stopped_at`]).
    pub fn end(&self) -> f64 {
        self.end
    }

    /// When the model's stop condition ([`stochcraft_model::Experiment::stop`]) was met, ending
    /// the run, if it has been.
    pub fn stopped_at(&self) -> Option<f64> {
        self.stopped_at
    }

    /// Whether the stop condition holds now.
    fn stop_met(&self) -> bool {
        let Some((i, stop)) = &self.stop else {
            return false;
        };
        let Some(s) = self.stats.get(*i) else {
            return false;
        };
        let s = s.borrow();
        let entered = s.entered_before + s.entered;
        match stop {
            stochcraft_model::Stop::Count { count, .. } => s.left_before + s.left >= *count,
            // Empty now (an entity that gave up is gone without counting in `left`).
            stochcraft_model::Stop::Empty { .. } => entered > 0 && s.inside() == 0,
        }
    }

    /// When the warm-up ends (statistics count from then).
    pub fn warmup(&self) -> f64 {
        self.warmup
    }

    /// Whether it has reached its end, with every value due to be recorded by then recorded
    /// (a budget that runs out at the end leaves the end's record to the next call, and until
    /// then the run is not done).
    pub fn is_done(&self) -> bool {
        self.sim.now() >= self.end && !self.next_record_time().is_some_and(|t| t <= self.sim.now())
    }

    /// Runs every event up to time `until` (at most the end).
    pub fn advance_to(&mut self, until: f64) -> Result<(), RunError> {
        self.advance_watched(until, &|| false, u64::MAX).map(|_| ())
    }

    /// Runs at most `events` events towards the end; returns whether it has reached the end
    /// (and, when recording, recorded there), as [`Live::is_done`] then says.
    /// A replication run this way, slice after slice, runs exactly the events of
    /// [`Compiled::run`] in the same order, whatever the slice size, and its stuck-time limit
    /// counts across slices.
    pub fn advance_by(&mut self, events: u64) -> Result<bool, RunError> {
        self.advance_watched(self.end, &|| false, events)
    }

    /// Runs the events up to time `until` (at most the end), at most `events` of them; returns
    /// whether it got to `until`. When it didn't, the next call carries on exactly where this
    /// one stopped: run slice after slice, the events and their order are those of
    /// [`Live::advance_to`], and the stuck-time limit counts across slices.
    pub fn advance_toward(&mut self, until: f64, events: u64) -> Result<bool, RunError> {
        self.advance_watched(until, &|| false, events)
    }

    /// When the next event happens, if it is no later than the end: the time [`Live::step`]
    /// goes to.
    pub fn next_event_time(&self) -> Option<f64> {
        self.sim.next_event_time().filter(|t| *t <= self.end)
    }

    /// The events run so far.
    pub fn events(&self) -> u64 {
        self.watching.events()
    }

    /// Runs the next event (and every other one at the same time); returns its time, or `None`
    /// when nothing more happens before the end (the run is then at its end).
    pub fn step(&mut self) -> Result<Option<f64>, RunError> {
        match self.sim.next_event_time() {
            Some(t) if t <= self.end => {
                self.advance_to(t)?;
                Ok(Some(t))
            }
            _ => {
                self.advance_to(self.end)?;
                Ok(None)
            }
        }
    }

    /// Runs the events up to `until` (at most the end), at most `budget` of them; returns
    /// whether it got to `until` (false: the budget ran out first, and the next call carries on
    /// exactly where this one stopped).
    ///
    /// When recording, it stops at each time to record at (once every event at that time has
    /// run, so a budget that runs out at that time leaves the record to the next call) and reads
    /// the blocks' state there. Stopping the clock between events changes none of them: the
    /// events and their order are those of one advance (as for a run in slices).
    fn advance_watched(
        &mut self,
        until: f64,
        cancelled: &dyn Fn() -> bool,
        budget: u64,
    ) -> Result<bool, RunError> {
        let until = if until.is_nan() {
            self.now()
        } else {
            until.min(self.end)
        };
        if until < self.now() {
            return Ok(true);
        }
        if self.recording.is_none() {
            return self.advance_events(until, cancelled, budget);
        }
        let mut left = budget;
        loop {
            let to = until.min(self.end);
            let next = self.next_record_time();
            let target = match next {
                Some(t) if t <= to => t,
                _ => to,
            };
            let before = self.watching.events();
            if !self.advance_events(target, cancelled, left)? {
                return Ok(false);
            }
            left = left.saturating_sub(self.watching.events().saturating_sub(before));
            // Not when the stop condition ended the run before the time to record at.
            if next == Some(target) && target <= self.end {
                self.record(target);
                continue;
            }
            return Ok(true);
        }
    }

    /// The next time to record at, if the run has not ended before it.
    fn next_record_time(&self) -> Option<f64> {
        let r = self.recording.as_ref()?;
        // Once the end is recorded, nothing more is: the rounding below makes only one record.
        if r.times.last().is_some_and(|l| *l >= self.end) {
            return None;
        }
        let t = r.times.len() as f64 * r.interval;
        // The last interval can overshoot the end by a rounding error: it is the end.
        let t = if t > self.end && t - self.end <= 1e-9 * self.end.abs().max(1.0) {
            self.end
        } else {
            t
        };
        (t <= self.end).then_some(t)
    }

    /// Records the state of every element and pool at time `t` (now).
    fn record(&mut self, t: f64) {
        let pools = self.pools.states(&self.compiled.model.resources);
        let Some(r) = self.recording.as_mut() else {
            return;
        };
        r.times.push(t);
        let n = self.stats.len();
        for (i, quantities) in r.elements.iter_mut().enumerate() {
            let element = self.stats.get(i).map(RefCell::borrow);
            let pool = i.checked_sub(n).and_then(|p| pools.get(p));
            for (q, values) in quantities.iter_mut() {
                let v = match (&element, pool) {
                    (Some(s), _) => match q {
                        Quantity::QueueLength => s.waiting.value(),
                        Quantity::WorkInProgress => s.inside() as f64,
                        Quantity::Busy => s.busy.value(),
                        Quantity::Entered => (s.entered_before + s.entered) as f64,
                        Quantity::Left => (s.left_before + s.left) as f64,
                    },
                    (None, Some(p)) => p.busy,
                    (None, None) => f64::NAN,
                };
                values.push(v);
            }
        }
    }

    /// [`Live::advance_watched`] without recording.
    fn advance_events(
        &mut self,
        until: f64,
        cancelled: &dyn Fn() -> bool,
        budget: u64,
    ) -> Result<bool, RunError> {
        let watch = Watch {
            max_events_at_one_time: self.compiled.max_events_at_one_time,
            cancelled,
        };
        if self.stop.is_none() || self.stopped_at.is_some() {
            return self
                .sim
                .run_watched_slice(until, &watch, budget, &mut self.watching)
                .map_err(|stopped| self.compiled.run_error(stopped));
        }
        // With a stop condition: one instant at a time, checking it after each. The events are
        // the same, in the same order, as in one go. An instant the budget cuts short is
        // finished by the next call (its next event is still at that instant) before the
        // condition is checked.
        let mut left = budget;
        loop {
            let to = match self.sim.next_event_time() {
                Some(t) if t <= until => t,
                _ => until,
            };
            let before = self.watching.events();
            let finished = self
                .sim
                .run_watched_slice(to, &watch, left, &mut self.watching)
                .map_err(|stopped| self.compiled.run_error(stopped))?;
            if !finished {
                return Ok(false);
            }
            left = left.saturating_sub(self.watching.events().saturating_sub(before));
            if self.stop_met() {
                self.stopped_at = Some(self.sim.now());
                self.end = self.sim.now();
                return Ok(true);
            }
            if to >= until {
                return Ok(true);
            }
        }
    }

    /// Each element's state now, in model order, then each shared pool's.
    pub fn states(&self) -> Vec<LiveState> {
        let model = &self.compiled.model;
        let mut out: Vec<LiveState> = model
            .elements
            .iter()
            .zip(self.stats.iter())
            .map(|(e, s)| {
                let s = s.borrow();
                LiveState {
                    id: e.id.clone(),
                    entered: s.entered,
                    left: s.left,
                    lost: s.lost,
                    waiting: s.waiting.value(),
                    busy: s.busy.value(),
                    units: match &e.kind {
                        // With shifts, the units on shift now (and those finishing a job after
                        // theirs has ended).
                        ElementKind::Server { shifts, .. } if !shifts.is_empty() => {
                            Some(s.staffed.value())
                        }
                        ElementKind::Server { capacity, .. } => Some(f64::from(*capacity)),
                        _ => None,
                    },
                    down: s.down.value(),
                }
            })
            .collect();
        out.extend(self.pools.states(&model.resources));
        out
    }

    /// Every item on conveyor `element` now, front first: how far along it is (a fraction of the
    /// belt's length, 0 to 1), the item's entity id, and its product key (the index of its value
    /// of the model's first text property among that property's values; 0 without one, or when
    /// the entity has no value). Empty for an element that is not a conveyor. This is the
    /// function a 3D view calls to draw items moving along a belt.
    ///
    /// Positions follow the conveyor's rule. Accumulating, an item moves at the belt's speed from
    /// when it got on, but never closer than one pitch behind the item ahead, and never past the
    /// end. Not accumulating, it is the belt's time since it got on (which stands still while the
    /// front item waits to leave) times the speed, up to the length.
    pub fn conveyor_items(&self, element: &str) -> Vec<(f32, u64, u32)> {
        self.compiled
            .model
            .elements
            .iter()
            .position(|e| e.id.as_str() == element)
            .and_then(|i| self.belts.get(i))
            .and_then(Option::as_ref)
            .map_or_else(Vec::new, |b| b.positions(self.sim.now()))
    }

    /// The statistics so far, as a [`Report`] up to now (the full one once the run is done).
    pub fn report(&self) -> Report {
        let now = self.now().min(self.end);
        let model = &self.compiled.model;
        let hours = model.experiment.time_unit.seconds() / 3_600.0;
        // Cycle times are reported once some sink has a demand, to compare with its takt.
        let takt = model.elements.iter().any(|e| {
            matches!(
                e.kind,
                ElementKind::Sink {
                    demand: Some(_),
                    ..
                }
            )
        });
        // The quality results apply once the model marks scrap or rework somewhere.
        let quality = model.connections.iter().any(|c| c.rework)
            || model
                .elements
                .iter()
                .any(|e| matches!(e.kind, ElementKind::Sink { scrap: Some(_), .. }));
        let mut elements: Vec<ElementReport> = model
            .elements
            .iter()
            .zip(self.stats.iter())
            .zip(&self.compiled.reported)
            .zip(self.feeds_buffer.iter().zip(&self.feeds_transfer))
            .zip(&self.compiled.inputs)
            .map(|((((e, s), a), (&blocks, &transfers)), &inputs)| {
                s.borrow().report(
                    e,
                    a,
                    Sends {
                        blocks,
                        transfers,
                        quality,
                        takt,
                    },
                    inputs > 1,
                    now,
                    hours,
                )
            })
            .collect();
        // Energy per entity at each sink: the energy of every server with a power over the
        // window, over the entities that reached the sink in it (none reached it: no value).
        let powered = model
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Server { power: Some(_), .. }));
        if powered {
            let total: f64 = elements.iter().filter_map(|e| e.energy).sum();
            for (r, e) in elements.iter_mut().zip(&model.elements) {
                if matches!(e.kind, ElementKind::Sink { .. }) && r.left > 0 {
                    r.specific_energy = Some(total / r.left as f64);
                }
            }
        }
        // Takt at each sink with a demand, over the measured window.
        if takt {
            let from = self.reset_at.get().unwrap_or(0.0);
            for (r, e) in elements.iter_mut().zip(&model.elements) {
                if let ElementKind::Sink {
                    demand: Some(d), ..
                } = &e.kind
                    && let Some((available, wanted)) = takt_over(model, d, from, now)
                    && wanted > 0.0
                {
                    r.demand = Some(wanted);
                    r.takt = Some(available / wanted);
                    r.demand_met = Some(r.left as f64 / wanted);
                }
            }
        }
        // Then each shared pool, after the elements.
        elements.extend(self.pools.reports(&model.resources, now));
        let after_warmup = self.reset_at.get();
        Report {
            warmup: self.warmup,
            duration: (now - self.warmup).max(0.0).min(self.duration),
            elements,
            stopped_at: self.stopped_at,
            measured_from: after_warmup.unwrap_or(0.0),
            measured_to: now,
            after_warmup: after_warmup.is_some(),
            recording: self.recording.clone(),
        }
    }
}

/// The time available and the demand over the measured window `from`–`to` for `d`: the
/// window's length, or the time `d`'s calendar is active in it; and the quantity times the
/// days, weeks or shifts in it (a shift partly inside counting by its share), or once for the
/// run. `None` when the window is empty or the calendar is unknown.
fn takt_over(model: &Model, d: &Demand, from: f64, to: f64) -> Option<(f64, f64)> {
    if to <= from {
        return None;
    }
    let unit = model.experiment.time_unit;
    let periods = match &d.calendar {
        Some(c) => {
            let cal = model.calendars.iter().find(|x| x.id == *c)?;
            let origin = model
                .experiment
                .start
                .or_else(|| Calendar::default_origin(&model.calendars))?;
            Some(cal.periods(origin, unit, to))
        }
        None => None,
    };
    let inside = |(s, e): (f64, f64)| (e.min(to) - s.max(from)).max(0.0);
    let available = match &periods {
        Some(p) => p.iter().map(|&x| inside(x)).sum(),
        None => to - from,
    };
    let day = 86_400.0 / unit.seconds();
    let wanted = d.quantity
        * match d.per {
            DemandPer::Run => 1.0,
            DemandPer::Day => (to - from) / day,
            DemandPer::Week => (to - from) / (7.0 * day),
            DemandPer::Shift => periods?
                .iter()
                .map(|&(s, e)| inside((s, e)) / (e - s))
                .sum(),
        };
    Some((available, wanted))
}

/// The tag of element `index`'s processes (see [`Sim::spawn_tagged`]).
fn tag(index: usize) -> u64 {
    u64::try_from(index).unwrap_or(u64::MAX)
}

/// Statistics a block keeps while the model runs. Every element balances: what entered either
/// left, was lost here, or is still inside.
pub(crate) struct BlockStats {
    /// Entities that entered (a source: created; a join: those on every input, see
    /// `spawn_parallel`).
    entered: u64,
    /// Of `entered`, the entities placed inside as the run started ([`Model::starting`]).
    started: u64,
    /// `entered` and `left` before the warm-up ended (for stop conditions, which count from 0).
    entered_before: u64,
    left_before: u64,
    /// Entities sent on (a sink: removed; a split or join: one per copy sent out).
    left: u64,
    /// Entities waiting in the block's input (servers only).
    waiting: TimeWeighted,
    /// Units busy (servers) or entities inside (delays).
    busy: TimeWeighted,
    /// Time from entering a server's input to the start of service. Only entities that started
    /// service count, so when the server is overloaded, those still waiting at the end (which
    /// have waited longest) are left out and the mean is biased low.
    wait: Tally,
    /// Time from creation to reaching a sink.
    time_in_system: Tally,
    /// Sinks that report time in system: each entity's time in system, one by one (in a
    /// calendar's active time when it names one), for its percentiles and the share within a
    /// target. Empty at other sinks.
    times_in_system: Vec<f64>,
    /// Servers: entities that finished here and then reached a scrap sink charged to this
    /// server, or took a rework connection (ISO 22400-2's scrap and rework quantities).
    scrap: u64,
    rework: u64,
    /// Servers: jobs finished, whether or not the entity has left yet (ISO 22400-2's produced
    /// quantity); `left` lags it by those held while blocked.
    produced: u64,
    /// The values assigned to each of the element's properties.
    properties: Vec<Tally>,
    /// Units (servers) or entities (delays) holding a finished entity while the next buffer is
    /// full.
    blocked: TimeWeighted,
    /// Time each entity that left waited for room in the next buffer (blocking after service).
    blocked_time: Tally,
    /// Each item's time on a conveyor, from getting on until it has left the end.
    time_on: Tally,
    /// The same times one by one, for the histogram of them.
    time_on_samples: Vec<f64>,
    /// Entities lost here. A server with a finite buffer: arrivals from sources turned away
    /// because the buffer was full (they never entered), or, with a way out for those turned
    /// away, any entity turned away (each time). A source: its own arrivals lost that
    /// way (they entered and never left; the same arrival counts at the full buffer too). An
    /// exclusive gateway: entities with no connection to take (no condition held and there is
    /// no default), which leave the model there; or, routing to the shortest queue, arrivals
    /// from a source finding no way with room.
    lost: u64,
    /// Units failed or under repair (servers with breakdowns).
    down: TimeWeighted,
    /// Entities on their way here, in transfer (servers with a transfer time). They have
    /// entered and hold a place in the buffer, but have not joined the queue yet.
    transit: TimeWeighted,
    /// Units holding on through a transfer to the next element (servers and tasks with shared
    /// resources sending to a server with a transfer time). The entity is counted at the next
    /// element, in its `transit`; the unit is blocked.
    transferring: TimeWeighted,
    /// A server's units in each state, each unit in exactly one ([`UnitBook`]).
    states: UnitStates,
    /// Servers with balking: arrivals that did not join the queue (they never entered).
    balked: u64,
    /// Servers with a patience: entities that gave up, and how long each had been here.
    abandoned: u64,
    abandoned_wait: Tally,
    /// Servers with patience until done: service time spent on jobs whose entity gave up; with
    /// a health check that retries, also the work done on jobs whose unit was taken out.
    wasted: f64,
    /// Servers with a health check: units taken out by a failed check.
    taken_out: u64,
    /// Servers with a health check: the waits recorded at starts that were retried, taken back
    /// out of `wait`'s mean.
    unwait: Tally,
    /// How many times the statistics were reset (a warm-up), so a retried start's wait is taken
    /// back only from the statistics it went into.
    resets: u64,
    /// Servers: units on shift, and those finishing a job before going off ([`OnShiftEnd`]):
    /// the units the server has, for the utilisation of a server with shifts.
    staffed: TimeWeighted,
    /// When the statistics last started (0, or the warm-up's end), and the entities inside then.
    from: f64,
    inside_at_start: u64,
    /// The times entities came in and went out since `from`, each counted from `from`, summed
    /// where they come and go (not from the counts inside), so that the behaviour checks can
    /// hold them against the time-average inside (Little's law).
    stays_started: f64,
    stays_ended: f64,
    /// Servers with patience until done, for the behaviour checks: entities inside whose job
    /// goes on after they gave up (counted as abandoned then; a stand-in took their way), now
    /// and as the statistics started.
    given_up_inside: u64,
    given_up_inside_at_start: u64,
    /// For the behaviour checks: entities that left by a way marked for those giving up,
    /// balking or turned away and wait for room on it, in neither element, now and as the
    /// statistics started.
    leaving: u64,
    leaving_at_start: u64,
}

/// What an element sends to, for its report.
#[derive(Clone, Copy)]
pub(crate) struct Sends {
    /// A finite buffer.
    pub(crate) blocks: bool,
    /// A server with a transfer time.
    pub(crate) transfers: bool,
    /// Whether the model marks scrap or rework anywhere (for the quality results).
    pub(crate) quality: bool,
    /// Whether any sink has a demand (for cycle times to compare with takt).
    pub(crate) takt: bool,
}

/// The state a server's unit is in: exactly one at every instant, the first that applies of
/// failed (or under repair), out restarting (taken out by a health check), off shift holding a
/// job paused until its shift ([`OnShiftEnd::Resume`]), processing (an entity, including a job
/// that runs on into a break or resumes off shift after a repair), blocked (holding a finished
/// entity), off shift (idle because the timetable has the unit off) and starved (idle, on shift
/// and up).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitState {
    Failed,
    Restarting,
    Setup,
    Processing,
    Blocked,
    OffShift,
    Starved,
}

/// How many of a server's units are in each state, over time; they add up to its capacity.
pub(crate) struct UnitStates {
    failed: TimeWeighted,
    restarting: TimeWeighted,
    setup: TimeWeighted,
    off_shift: TimeWeighted,
    blocked: TimeWeighted,
    processing: TimeWeighted,
    starved: TimeWeighted,
}

impl UnitStates {
    fn new(t: f64) -> Self {
        Self {
            failed: TimeWeighted::new(t, 0.0),
            restarting: TimeWeighted::new(t, 0.0),
            setup: TimeWeighted::new(t, 0.0),
            off_shift: TimeWeighted::new(t, 0.0),
            blocked: TimeWeighted::new(t, 0.0),
            processing: TimeWeighted::new(t, 0.0),
            starved: TimeWeighted::new(t, 0.0),
        }
    }

    fn reset(&mut self, t: f64) {
        self.failed.reset(t);
        self.restarting.reset(t);
        self.setup.reset(t);
        self.off_shift.reset(t);
        self.blocked.reset(t);
        self.processing.reset(t);
        self.starved.reset(t);
    }

    fn get(&mut self, s: UnitState) -> Option<&mut TimeWeighted> {
        match s {
            UnitState::Failed => Some(&mut self.failed),
            UnitState::Restarting => Some(&mut self.restarting),
            UnitState::Setup => Some(&mut self.setup),
            UnitState::OffShift => Some(&mut self.off_shift),
            UnitState::Blocked => Some(&mut self.blocked),
            UnitState::Processing => Some(&mut self.processing),
            UnitState::Starved => Some(&mut self.starved),
        }
    }
}

/// What a server's unit is doing, from which its [`UnitState`] follows.
#[derive(Debug, Clone, Copy, Default)]
struct Doing {
    /// Serving an entity, including one whose service a failure interrupted.
    processing: bool,
    /// Setting up for the entity it serves (within `processing`).
    setup: bool,
    /// Holding a finished entity, blocked.
    holding: bool,
    /// Failed or under repair.
    down: bool,
    /// Taken out of service by a health check, restarting.
    out: bool,
    /// Off shift: the timetable has it off (it may still be finishing a job first).
    off: bool,
    /// Holding a job paused until it is on shift again ([`OnShiftEnd::Resume`]).
    paused: bool,
}

/// What each of a server's units is doing and whether it is on shift, kept in step with the
/// counts of units in each state ([`BlockStats`]'s `states`), which give the shares of time in
/// each state.
struct UnitBook {
    doing: RefCell<Vec<Doing>>,
}

impl UnitBook {
    /// A book of `capacity` units, all starved (idle, on shift and up) to start with.
    fn new(env: &BlockEnv, capacity: u32) -> Rc<Self> {
        let now = env.ctx.now();
        env.with_stats(|s| {
            s.states.starved.set(now, f64::from(capacity));
            s.staffed.set(now, f64::from(capacity));
        });
        let book = Rc::new(Self {
            doing: RefCell::new(vec![Doing::default(); capacity as usize]),
        });
        if let Some(p) = &env.places {
            p.watch_units(&book);
        }
        book
    }

    /// The units working now (up, not out and on shift, as a health check counts them), and
    /// of those the idle ones (holding no entity).
    fn working_and_idle(&self) -> (usize, usize) {
        let doing = self.doing.borrow();
        let working = doing.iter().filter(|d| !d.down && !d.out && !d.off);
        let idle = working
            .clone()
            .filter(|d| !d.processing && !d.holding && !d.paused)
            .count();
        (working.count(), idle)
    }

    fn state(&self, d: Doing) -> UnitState {
        if d.down {
            UnitState::Failed
        } else if d.out {
            UnitState::Restarting
        } else if d.paused {
            UnitState::OffShift
        } else if d.processing && d.setup {
            UnitState::Setup
        } else if d.processing {
            UnitState::Processing
        } else if d.holding {
            UnitState::Blocked
        } else if d.off {
            UnitState::OffShift
        } else {
            UnitState::Starved
        }
    }

    /// Moves a unit from state `from` to `to` in the counts.
    fn count(env: &BlockEnv, from: UnitState, to: UnitState) {
        if from == to {
            return;
        }
        let now = env.ctx.now();
        env.with_stats(|s| {
            for (st, delta) in [(from, -1.0), (to, 1.0)] {
                if let Some(tw) = s.states.get(st) {
                    let v = tw.value() + delta;
                    tw.set(now, v);
                }
            }
        });
    }

    /// Changes what unit `u` is doing (now), keeping the counts.
    fn change(&self, env: &BlockEnv, u: usize, f: impl FnOnce(&mut Doing)) {
        let (from, to) = {
            let mut doing = self.doing.borrow_mut();
            let Some(d) = doing.get_mut(u) else {
                return;
            };
            let from = self.state(*d);
            f(d);
            (from, self.state(*d))
        };
        Self::count(env, from, to);
        // A unit taking an entity, coming free, or working again: a limit on those waiting
        // may let more in.
        if let Some(p) = &env.places {
            p.notify();
        }
    }

    /// Whether unit `u` holds a finished entity (blocked, or through a transfer).
    fn holding(&self, u: usize) -> bool {
        self.doing.borrow().get(u).is_some_and(|d| d.holding)
    }
}

impl BlockStats {
    fn new(t: f64, properties: usize) -> Self {
        Self {
            entered: 0,
            started: 0,
            entered_before: 0,
            left_before: 0,
            left: 0,
            waiting: TimeWeighted::new(t, 0.0),
            busy: TimeWeighted::new(t, 0.0),
            wait: Tally::new(),
            time_in_system: Tally::new(),
            times_in_system: Vec::new(),
            scrap: 0,
            rework: 0,
            produced: 0,
            properties: (0..properties).map(|_| Tally::new()).collect(),
            blocked: TimeWeighted::new(t, 0.0),
            blocked_time: Tally::new(),
            time_on: Tally::new(),
            time_on_samples: Vec::new(),
            lost: 0,
            down: TimeWeighted::new(t, 0.0),
            transit: TimeWeighted::new(t, 0.0),
            transferring: TimeWeighted::new(t, 0.0),
            states: UnitStates::new(t),
            balked: 0,
            abandoned: 0,
            abandoned_wait: Tally::new(),
            wasted: 0.0,
            taken_out: 0,
            unwait: Tally::new(),
            resets: 0,
            staffed: TimeWeighted::new(t, 0.0),
            from: t,
            inside_at_start: 0,
            stays_started: 0.0,
            stays_ended: 0.0,
            given_up_inside: 0,
            given_up_inside_at_start: 0,
            leaving: 0,
            leaving_at_start: 0,
        }
    }

    /// An entity comes in, at `now`.
    fn stay_starts(&mut self, now: f64) {
        self.stays_started += now - self.from;
    }

    /// A server's time-average number of units working: processing or setting up.
    fn working(&self, end: f64) -> Option<f64> {
        let setup = self.states.setup.average(end).unwrap_or(0.0);
        self.states.processing.average(end).map(|p| p + setup)
    }

    /// An entity goes out (to the next element, giving up or discarded), at `now`.
    fn stay_ends(&mut self, now: f64) {
        self.stays_ended += now - self.from;
    }

    /// The time entities spent inside from `from` to `end`, from the times they came in and
    /// went out: those inside at `from` from then, those still inside at `end` until then.
    fn time_inside(&self, end: f64) -> f64 {
        self.stays_ended - self.stays_started + self.inside() as f64 * (end - self.from)
    }

    fn reset(&mut self, t: f64) {
        self.entered_before += self.entered;
        self.left_before += self.left;
        self.entered = 0;
        // Those placed at the start and still here now count as inside at the start.
        self.started = 0;
        self.left = 0;
        self.waiting.reset(t);
        self.busy.reset(t);
        self.wait = Tally::new();
        self.time_in_system = Tally::new();
        self.times_in_system.clear();
        self.scrap = 0;
        self.rework = 0;
        self.produced = 0;
        self.properties.iter_mut().for_each(|p| *p = Tally::new());
        self.blocked.reset(t);
        self.blocked_time = Tally::new();
        self.time_on = Tally::new();
        self.time_on_samples.clear();
        self.lost = 0;
        self.down.reset(t);
        self.transit.reset(t);
        self.transferring.reset(t);
        self.states.reset(t);
        self.balked = 0;
        self.abandoned = 0;
        self.abandoned_wait = Tally::new();
        self.wasted = 0.0;
        self.taken_out = 0;
        self.unwait = Tally::new();
        self.resets += 1;
        self.staffed.reset(t);
        self.inside_at_start = self.inside();
        self.given_up_inside_at_start = self.given_up_inside;
        self.leaving_at_start = self.leaving;
        self.from = t;
        self.stays_started = 0.0;
        self.stays_ended = 0.0;
    }

    /// The mean of the waits recorded at service starts, less those of starts that were retried.
    fn mean_wait(&self) -> Option<f64> {
        let taken_back = self.unwait.count();
        if taken_back == 0 {
            return self.wait.mean();
        }
        let n = self
            .wait
            .count()
            .checked_sub(taken_back)
            .filter(|n| *n > 0)?;
        let sum =
            self.wait.mean()? * self.wait.count() as f64 - self.unwait.mean()? * taken_back as f64;
        Some(sum / n as f64)
    }

    /// Entities inside the element now: on their way in (in transfer), waiting in its input,
    /// being served or delayed (or collected into a batch), and held while blocked after
    /// service. An entity in transfer to the next element is that element's, not this one's.
    fn inside(&self) -> u64 {
        let n =
            self.transit.value() + self.waiting.value() + self.busy.value() + self.blocked.value();
        // Sums of whole counts; `as` saturates (and a negative value cannot occur).
        n.round().max(0.0) as u64
    }

    /// The time-average of [`Self::inside`] from the last reset to `end`.
    fn mean_inside(&self, end: f64) -> Option<f64> {
        [&self.transit, &self.waiting, &self.busy, &self.blocked]
            .iter()
            .map(|tw| tw.average(end))
            .sum()
    }

    /// The energy a server with power `p` used from the last reset to `end`, in kilowatt-hours
    /// (`hours`: hours in one model time unit): each state's power times the time its units
    /// spent in it.
    fn energy(&self, p: &Power, end: f64, hours: f64) -> f64 {
        let s = &self.states;
        // A unit setting up draws its processing power (its machine runs).
        let kw_units = p.processing * (s.processing.integral(end) + s.setup.integral(end))
            + p.idle * s.starved.integral(end)
            + p.while_blocked() * s.blocked.integral(end)
            + p.while_failed() * s.failed.integral(end)
            + p.while_off_shift() * s.off_shift.integral(end)
            // A unit out restarting draws its idle power (0 without a health check).
            + p.idle * s.restarting.integral(end);
        kw_units * hours
    }

    /// The element's report; `sends`: whether it sends to a finite buffer or across a
    /// transfer; `join`: whether it is a parallel gateway with several inputs (which holds
    /// entities until each input has one); `hours`: hours in one model time unit, for energy.
    pub(crate) fn report(
        &self,
        element: &Element,
        properties: &[String],
        sends: Sends,
        join: bool,
        end: f64,
        hours: f64,
    ) -> ElementReport {
        let Sends {
            blocks,
            transfers,
            quality,
            takt,
        } = sends;
        let kind = &element.kind;
        // A server's units in all: its capacity, or with shifts the most on shift at once. The
        // shares of the units' time in each state are of these, off shift included.
        let units = f64::from(kind.most_units().unwrap_or(0));
        // Servers whose number of units follows a timetable (shifts).
        let shifted = matches!(kind, ElementKind::Server { shifts, .. } if !shifts.is_empty());
        // Servers whose units go off shift at all (shifts, or an availability calendar): their
        // utilisation is over the units on shift, as a pool's.
        let on_calendar = shifted
            || matches!(
                kind,
                ElementKind::Server {
                    available,
                    availability,
                    ..
                } if !*available || !availability.is_empty()
            );
        // Servers with a power report the share of time in each state, and their energy.
        let powered = match kind {
            ElementKind::Server { power: Some(p), .. } => Some(p),
            _ => None,
        };
        // Every server reports the share of its units' time in each state that can happen to it
        // (all of them with a power, for its energy): processing and starved always, blocked
        // where it sends to a finite buffer or across a transfer, failed with breakdowns.
        let share = |tw: &TimeWeighted, applies: bool| match kind {
            ElementKind::Server { power, .. } if applies || power.is_some() => {
                tw.average(end).map(|x| x / units)
            }
            _ => None,
        };
        let fails = matches!(
            kind,
            ElementKind::Server {
                breakdown: Some(_),
                ..
            }
        );
        let server = matches!(
            kind,
            ElementKind::Server { .. } | ElementKind::SharedTask { .. }
        );
        let finite = matches!(
            kind,
            ElementKind::Server {
                queue_capacity: Some(_),
                ..
            }
        );
        let holds = matches!(
            kind,
            ElementKind::Server { .. }
                | ElementKind::SharedTask { .. }
                | ElementKind::Delay { .. }
                | ElementKind::Conveyor { .. }
        );
        let offered = self.lost + self.entered;
        let balky = matches!(
            kind,
            ElementKind::Server {
                balking: Some(_),
                ..
            }
        );
        // ISO 22400-2's OEE elements. Off shift is planned downtime, so the time planned to be
        // busy is the units' time less it.
        let oee_availability = match kind {
            ElementKind::Server { .. } => self
                .states
                .processing
                .average(end)
                .zip(self.states.off_shift.average(end))
                .and_then(|(p, o)| {
                    let planned = units - o;
                    (planned > 0.0).then(|| p / planned)
                }),
            _ => None,
        };
        let oee_effectiveness = match kind {
            ElementKind::Server {
                planned_run_time: Some(plan),
                ..
            } => plan.mean().and_then(|m| {
                let producing = self.states.processing.integral(end);
                (producing > 0.0).then(|| m * self.produced as f64 / producing)
            }),
            _ => None,
        };
        // A server's scrap or rework over what it produced (each pass counted).
        let per_produced = |n: u64| match kind {
            ElementKind::Server { .. } if self.produced > 0 => {
                Some(n as f64 / self.produced as f64)
            }
            _ => None,
        };
        let quality_ratio = per_produced(self.produced.saturating_sub(self.scrap + self.rework));
        let patient = match kind {
            ElementKind::Server {
                patience: Some(p), ..
            } => Some(p.until),
            _ => None,
        };
        let checked = match kind {
            ElementKind::Server {
                health_check: Some(h),
                ..
            } => Some((units, h.on_takeout)),
            _ => None,
        };
        let shift_retries = matches!(
            kind,
            ElementKind::Server {
                on_shift_end: OnShiftEnd::Retry,
                ..
            }
        );
        let wastes = patient == Some(PatienceUntil::Done)
            || checked.is_some_and(|(_, o)| o == OnTakeout::Retry)
            || shift_retries;
        // Time in system by percentile, and the share within the target, at a sink that asks.
        let times = match kind {
            ElementKind::Sink {
                time_in_system: Some(x),
                ..
            } => {
                let mut sorted = self.times_in_system.clone();
                sorted.sort_by(f64::total_cmp);
                Some((sorted, x.within))
            }
            _ => None,
        };
        let percentile = |p: f64| {
            times
                .as_ref()
                .and_then(|(sorted, _)| stochcraft_stats::spread::quantile(sorted, p))
        };
        let within_target_share = times.as_ref().and_then(|(sorted, within)| {
            let w = (*within)?;
            (!sorted.is_empty())
                .then(|| sorted.iter().filter(|t| **t <= w).count() as f64 / sorted.len() as f64)
        });
        ElementReport {
            id: element.id.clone(),
            name: element.name.clone(),
            kind: kind.name(),
            entered: self.entered,
            left: self.left,
            mean_queue_length: if server {
                self.waiting.average(end)
            } else {
                None
            },
            mean_wait: if server { self.mean_wait() } else { None },
            // Units working (processing or setting up) over the units there to work. A unit
            // failed or restarting part-way through a job is not busy: that time is capacity
            // lost, still in the units' time, as Law (Simulation Modeling and Analysis, 5th ed.,
            // §14.4.2: busy time is part of up time) and ISO 22400-2's busy time count it.
            // Blocked units hold a finished entity and are not busy either.
            utilisation: match kind {
                // Over the units on shift (time off shift is planned, so left out of both),
                // both averaged over time; none on shift: 0.
                ElementKind::Server { .. } if on_calendar => self
                    .working(end)
                    .zip(self.staffed.average(end))
                    .map(|(w, s)| if s > 0.0 { w / s } else { 0.0 }),
                ElementKind::Server { capacity, .. } => {
                    self.working(end).map(|w| w / f64::from(*capacity))
                }
                _ => None,
            },
            mean_contents: match kind {
                ElementKind::Server { .. }
                | ElementKind::SharedTask { .. }
                | ElementKind::Delay { .. }
                | ElementKind::Conveyor { .. }
                | ElementKind::Batch { .. } => self.busy.average(end),
                _ => None,
            },
            work_in_progress: match kind {
                ElementKind::Server { .. }
                | ElementKind::SharedTask { .. }
                | ElementKind::Delay { .. }
                | ElementKind::Conveyor { .. }
                | ElementKind::Batch { .. } => Some(self.inside()),
                ElementKind::ParallelGateway if join => Some(self.inside()),
                _ => None,
            },
            mean_work_in_progress: match kind {
                ElementKind::Server { .. }
                | ElementKind::SharedTask { .. }
                | ElementKind::Delay { .. }
                | ElementKind::Conveyor { .. }
                | ElementKind::Batch { .. } => self.mean_inside(end),
                ElementKind::ParallelGateway if join => self.mean_inside(end),
                _ => None,
            },
            mean_time_in_system: if matches!(kind, ElementKind::Sink { .. }) {
                self.time_in_system.mean()
            } else {
                None
            },
            time_in_system_p50: percentile(0.5),
            time_in_system_p90: percentile(0.9),
            time_in_system_p95: percentile(0.95),
            within_target_share,
            mean_blocked_time: if blocks && holds {
                self.blocked_time.mean()
            } else {
                None
            },
            mean_time_on: if matches!(kind, ElementKind::Conveyor { .. }) {
                self.time_on.mean()
            } else {
                None
            },
            time_on_histogram: if matches!(kind, ElementKind::Conveyor { .. }) {
                crate::Histogram::of(&self.time_on_samples)
            } else {
                None
            },
            // Units held after their job: waiting for room in the next buffer, or through a
            // transfer to the next element.
            blocking_probability: match kind {
                ElementKind::Server { .. } if transfers => self
                    .blocked
                    .average(end)
                    .zip(self.transferring.average(end))
                    .map(|(b, t)| (b + t) / units),
                ElementKind::Server { .. } if blocks => {
                    self.blocked.average(end).map(|b| b / units)
                }
                // The share of time the front item waits at the end (only it can).
                ElementKind::Conveyor { .. } => self.blocked.average(end),
                _ => None,
            },
            availability: match kind {
                ElementKind::Server {
                    breakdown: Some(_), ..
                } => self.down.average(end).map(|d| 1.0 - d / units),
                _ => None,
            },
            // Only for servers that follow a calendar or have a power, so others report as before.
            time_off_shift: match kind {
                ElementKind::Server { power, .. } if on_calendar || power.is_some() => {
                    self.states.off_shift.average(end).map(|o| o / units)
                }
                _ => None,
            },
            time_processing: share(&self.states.processing, true),
            time_starved: share(&self.states.starved, true),
            time_blocked: share(&self.states.blocked, blocks || transfers),
            time_failed: share(&self.states.failed, fails),
            // Servers that set up, whether or not they have a power.
            time_setup: match kind {
                ElementKind::Server { setup: Some(_), .. } => {
                    self.states.setup.average(end).map(|x| x / units)
                }
                _ => None,
            },
            oee_availability,
            oee_effectiveness,
            quality_ratio: quality_ratio.filter(|_| quality),
            scrap_ratio: per_produced(self.scrap).filter(|_| quality),
            rework_ratio: per_produced(self.rework).filter(|_| quality),
            // The time working per item over the units working side by side: with shifts, the
            // mean units on shift.
            cycle_time: match kind {
                ElementKind::Server { .. } if takt && self.produced > 0 => {
                    let side_by_side = if shifted {
                        self.staffed.average(end).unwrap_or(units)
                    } else {
                        units
                    };
                    Some(
                        (self.states.processing.integral(end) + self.states.setup.integral(end))
                            / self.produced as f64
                            / side_by_side,
                    )
                }
                _ => None,
            },
            takt: None,
            demand: None,
            demand_met: None,
            oee: oee_availability.zip(oee_effectiveness).map(|(a, e)| {
                a * e
                    * if quality {
                        quality_ratio.unwrap_or(1.0)
                    } else {
                        1.0
                    }
            }),
            setup_ratio: match kind {
                ElementKind::Server { setup: Some(_), .. } => self
                    .states
                    .setup
                    .average(end)
                    .zip(self.states.processing.average(end))
                    .and_then(|(s, p)| (s + p > 0.0).then(|| s / (s + p))),
                _ => None,
            },
            // Only once time has passed, as for the time-averages.
            energy: powered.and_then(|p| {
                self.states
                    .processing
                    .average(end)
                    .map(|_| self.energy(p, end, hours))
            }),
            // Filled in by [`Live::report`], which sees every element.
            specific_energy: None,
            // Only where losses can happen, so models without them report as before.
            lost: match kind {
                ElementKind::Server {
                    queue_capacity: Some(_),
                    ..
                }
                | ElementKind::ExclusiveGateway { .. } => Some(self.lost),
                ElementKind::Source { .. } if blocks => Some(self.lost),
                _ => None,
            },
            loss_probability: if finite && offered > 0 {
                Some(self.lost as f64 / offered as f64)
            } else {
                None
            },
            abandoned: patient.map(|_| self.abandoned),
            abandoned_share: patient.and_then(|_| {
                let ended = self.abandoned + self.left;
                (ended > 0).then(|| self.abandoned as f64 / ended as f64)
            }),
            abandoned_wait: patient.and_then(|_| self.abandoned_wait.mean()),
            balked: balky.then_some(self.balked),
            balked_share: balky.then_some(()).and_then(|()| {
                let arrived = self.balked + self.lost + self.entered;
                (arrived > 0).then(|| self.balked as f64 / arrived as f64)
            }),
            wasted: wastes.then_some(self.wasted),
            wasted_share: wastes
                .then(|| self.states.processing.integral(end) + self.states.setup.integral(end))
                .and_then(|working| (working > 0.0).then(|| self.wasted / working)),
            taken_out: checked.map(|_| self.taken_out),
            // Units not failed, not out and not off shift (each unit is in one state).
            mean_in_service: checked.and_then(|(c, _)| {
                let s = &self.states;
                let away = s.failed.average(end)?
                    + s.restarting.average(end)?
                    + s.off_shift.average(end)?;
                Some(c - away)
            }),
            time_restarting: checked
                .and_then(|(c, _)| self.states.restarting.average(end).map(|x| x / c)),
            properties: properties
                .iter()
                .zip(&self.properties)
                .map(|(name, t)| (name.clone(), t.mean()))
                .collect(),
            inside_at_start: self.inside_at_start,
            inside_at_end: self.inside(),
            given_up_inside: [self.given_up_inside_at_start, self.given_up_inside],
            leaving: [self.leaving_at_start, self.leaving],
            time_inside: match kind {
                ElementKind::Server { .. }
                | ElementKind::SharedTask { .. }
                | ElementKind::Delay { .. }
                | ElementKind::Conveyor { .. } => Some(self.time_inside(end)),
                _ => None,
            },
            on_shift: match kind {
                ElementKind::Server { .. } if on_calendar => self.staffed.average(end),
                _ => None,
            },
            // Only where some were placed, so models without starting contents report as
            // before.
            started: (self.started > 0).then_some(self.started),
        }
    }
}

/// Where a block sends entities: the next block's input, with its statistics.
#[derive(Clone)]
struct Port {
    store: Store<Entity>,
    stats: Rc<Vec<RefCell<BlockStats>>>,
    assigner: Rc<Assigner>,
    index: usize,
    /// The next element's places, when its buffer is finite.
    places: Option<Rc<Places>>,
    /// The next element's transfer time, when it has one.
    transfer: Option<Rc<Transfer>>,
    /// When the next element is a shortest-queue gateway: its ways, which the sender routes
    /// to itself, so that it is the one waiting for room.
    router: Option<Rc<Router>>,
    /// Entities sent this way are rework, counted against the last server they finished at.
    rework: bool,
    /// The next element's patience, when it is a server with one. Weak, since a patience holds
    /// its way out, which may lead to another patient server and back.
    patient: Option<Weak<Patient>>,
    /// The next element's balking, when it is a server with balking. Weak, for the same reason.
    balk: Option<Weak<Balk>>,
}

/// Why an entity did not get into the next element.
#[derive(Clone, Copy)]
enum Refused {
    /// It found no room in a finite waiting room that turns away those finding none.
    TurnedAway,
    /// It chose not to join the queue ([`Balking`]).
    Balked,
}

/// A server's balking while the model runs ([`Balking`]): whether each arriving entity joins,
/// and the way out for those that do not.
struct Balk {
    /// The server.
    index: usize,
    balking: Balking,
    rng: RefCell<Rng>,
    ctx: Ctx,
    stats: Rc<Vec<RefCell<BlockStats>>>,
    /// The way out marked for those that balk; `None`: they leave the model.
    way: RefCell<Option<Port>>,
}

impl Balk {
    /// Whether an entity arriving now balks: it joins with the chance the number it finds gives
    /// ([`Balking::join_probability`]), drawn from the server's own stream only when that chance
    /// is strictly between 0 and 1.
    fn balks(&self) -> bool {
        let n = self.stats.get(self.index).map_or(0, |s| {
            let s = s.borrow();
            if self.balking.counts_in_service {
                s.inside()
            } else {
                // Sums of whole counts; `as` saturates (and a negative value cannot occur).
                (s.waiting.value() + s.transit.value()).round().max(0.0) as u64
            }
        });
        let b = self.balking.join_probability(n);
        if b >= 1.0 {
            false
        } else if b > 0.0 {
            self.rng.borrow_mut().uniform() >= b
        } else {
            true
        }
    }

    /// `e` balks: it is counted, and takes the way out for those that balk at once, on its own,
    /// as a process (as for those turned away, [`Away::take`]), or leaves the model.
    fn take(&self, e: Entity) {
        if let Some(s) = self.stats.get(self.index) {
            s.borrow_mut().balked += 1;
        }
        let Some(way) = self.way.borrow().clone() else {
            return;
        };
        leave_by(&self.ctx, &self.stats, self.index, way, e);
    }
}

/// `e` leaves element `index` by a way marked for those giving up, balking or turned away, on
/// its own, as a process (started now): where the way can hold it up, it waits there for room
/// (or is turned away there), counted meanwhile as leaving `index`, since neither element
/// holds it; otherwise it is sent straight on.
fn leave_by(ctx: &Ctx, stats: &Rc<Vec<RefCell<BlockStats>>>, index: usize, way: Port, e: Entity) {
    let waits = way.needs_room();
    if waits && let Some(s) = stats.get(index) {
        s.borrow_mut().leaving += 1;
    }
    let (c, stats) = (ctx.clone(), Rc::clone(stats));
    ctx.spawn_tagged(tag(index), async move {
        if waits {
            way.deliver(&c, e).await;
            if let Some(s) = stats.get(index) {
                let mut s = s.borrow_mut();
                s.leaving = s.leaving.saturating_sub(1);
            }
        } else {
            way.dispatch(&c, e);
        }
    });
}

/// A finite buffer's places (see [`Compiled::run_for`]), with the one each entity there holds;
/// or, for a waiting room by timetable or per working server, its limit on those waiting.
struct Places {
    room: Room,
    held: RefCell<BTreeMap<u64, ResourceGuard>>,
    /// Woken whenever a place here is given back, or the room may have room again (routers
    /// waiting for room on any way).
    watchers: RefCell<Vec<Signal>>,
    /// The server's way out for entities turned away, if it has one.
    away: RefCell<Weak<Away>>,
}

/// How a finite waiting room admits entities ([`QueueLimit`]).
enum Room {
    /// A place for every unit plus the waiting room, held from entering until leaving.
    Fixed(Resource),
    /// A limit on the number waiting that changes over time.
    Limit(Limit),
    /// A conveyor's entrance.
    Belt(Belt),
}

/// A conveyor's entrance (ROADMAP M5.1): an item gets on when fewer than `places` items are on
/// the belt and the item before it has moved a pitch along, so their slots never overlap.
struct Belt {
    /// How many pitches fit along the conveyor.
    places: u64,
    /// The entities on it, from getting on until they leave its end, each with how many times
    /// it is on: one sent round onto the belt it is leaving takes its place again before it
    /// gives the old one back.
    on: RefCell<BTreeMap<u64, u32>>,
    /// Whether the last item to get on has moved a pitch along.
    free: Cell<bool>,
    /// Woken whenever an item gets off or the entrance frees.
    changed: Signal,
}

impl Belt {
    fn admits(&self) -> bool {
        self.free.get() && self.count() < self.places
    }

    /// How many are on it.
    fn count(&self) -> u64 {
        self.on.borrow().values().map(|n| u64::from(*n)).sum()
    }

    /// Entity `id` gets on: it counts, and the entrance is taken until it has moved a pitch.
    fn take(&self, id: u64) {
        *self.on.borrow_mut().entry(id).or_insert(0) += 1;
        self.free.set(false);
    }

    /// Entity `id` gets off (once, if it is on more than once).
    fn off(&self, id: u64) {
        let mut on = self.on.borrow_mut();
        if let Some(n) = on.get_mut(&id) {
            *n -= 1;
            if *n == 0 {
                on.remove(&id);
            }
        }
    }
}

/// A limit on the entities waiting at a server ([`QueueLimit::ByShift`] and
/// [`QueueLimit::PerWorkingServer`]): an entity is admitted when those waiting (in the queue
/// or in transfer here) less the idle working units are fewer than the limit now.
struct Limit {
    ctx: Ctx,
    /// The server (its statistics).
    index: usize,
    stats: Rc<Vec<RefCell<BlockStats>>>,
    rule: LimitRule,
    /// The server's units, once it runs.
    units: RefCell<Weak<UnitBook>>,
    /// Woken whenever the room may have room again.
    changed: Signal,
}

enum LimitRule {
    /// From each time on, until the next, the most waiting (the first at 0).
    ByShift(Vec<(f64, u32)>),
    /// The most waiting per working unit.
    PerWorkingServer(u32),
}

impl Limit {
    fn new(ctx: &Ctx, index: usize, stats: &Rc<Vec<RefCell<BlockStats>>>, rule: LimitRule) -> Self {
        Self {
            ctx: ctx.clone(),
            index,
            stats: Rc::clone(stats),
            rule,
            units: RefCell::new(Weak::new()),
            changed: Signal::new(ctx),
        }
    }

    /// Whether an entity is admitted now.
    fn admits(&self) -> bool {
        let waiting = self.stats.get(self.index).map_or(0.0, |s| {
            let s = s.borrow();
            s.waiting.value() + s.transit.value()
        });
        let (working, idle) = self
            .units
            .borrow()
            .upgrade()
            .map_or((0, 0), |book| book.working_and_idle());
        let limit = match &self.rule {
            LimitRule::ByShift(steps) => {
                let now = self.ctx.now();
                let k = steps.partition_point(|(t, _)| *t <= now);
                steps
                    .get(k.saturating_sub(1))
                    .map_or(0.0, |(_, n)| f64::from(*n))
            }
            LimitRule::PerWorkingServer(n) => f64::from(*n) * working as f64,
        };
        waiting - (idle as f64) < limit
    }
}

/// A server's way out for entities turned away ([`stochcraft_model::Connection::turned_away`]).
struct Away {
    /// The server.
    index: usize,
    ctx: Ctx,
    way: Port,
}

impl Away {
    /// `e`, turned away, takes the way at once, on its own, as a process (started now): it
    /// waits for room there if the way can hold it up, or is turned away again there. Being a
    /// process, a way that leads straight back to a room that stays full goes round through the
    /// event queue, where the guard on events at one time stops it, rather than looping within
    /// one step.
    fn take(&self, e: Entity) {
        leave_by(&self.ctx, &self.way.stats, self.index, self.way.clone(), e);
    }
}

impl Places {
    /// Whether entities finding no room here are turned away, rather than wait for it.
    fn turns_away(&self) -> bool {
        self.away.borrow().strong_count() > 0
    }

    /// Waits for room for entity `id` and takes it.
    async fn wait_room(&self, id: u64) {
        match &self.room {
            Room::Fixed(pool) => {
                let guard = pool.acquire().await;
                self.enter(id, guard);
            }
            Room::Limit(l) => {
                // Nothing waits between seeing room and taking it.
                while !l.admits() {
                    l.changed.wait().await;
                }
            }
            Room::Belt(b) => {
                while !b.admits() {
                    b.changed.wait().await;
                }
                b.take(id);
            }
        }
    }

    /// Takes room for entity `id` if there is some now (for places, a place freed at this
    /// instant counts) and returns `true`; else `false`.
    async fn try_room(&self, id: u64) -> bool {
        match &self.room {
            Room::Fixed(pool) => match pool.acquire_within(0.0).await {
                Ok(guard) => {
                    self.enter(id, guard);
                    true
                }
                Err(_) => false,
            },
            Room::Limit(l) => l.admits(),
            Room::Belt(b) => {
                let admits = b.admits();
                if admits {
                    b.take(id);
                }
                admits
            }
        }
    }

    /// A place for a unit to keep for itself, if one is free ([`reserve_place`]); never with a
    /// limit on those waiting, which holds no places.
    fn reserve(&self) -> Option<ResourceGuard> {
        match &self.room {
            Room::Fixed(pool) => pool.try_acquire(),
            Room::Limit(_) | Room::Belt(_) => None,
        }
    }

    /// The server's units now run: a limit counts them.
    fn watch_units(&self, book: &Rc<UnitBook>) {
        if let Room::Limit(l) = &self.room {
            *l.units.borrow_mut() = Rc::downgrade(book);
        }
    }

    /// With a limit on those waiting: the room may have room again, so whoever waits for it
    /// looks again. Places tell their waiters themselves.
    fn notify(&self) {
        match &self.room {
            Room::Fixed(_) => return,
            Room::Limit(l) => l.changed.notify_all(),
            Room::Belt(b) => b.changed.notify_all(),
        };
        for w in self.watchers.borrow().iter() {
            w.notify_all();
        }
    }

    /// A conveyor's last item has moved a pitch along: the entrance is free again.
    fn open_entrance(&self) {
        if let Room::Belt(b) = &self.room {
            b.free.set(true);
        }
        self.notify();
    }

    /// The way out for entities turned away, if there is one.
    fn away(&self) -> Option<Rc<Away>> {
        self.away.borrow().upgrade()
    }

    /// Takes the place `guard` for entity `id`, as it gets in.
    fn enter(&self, id: u64, guard: ResourceGuard) {
        self.held.borrow_mut().insert(id, guard);
    }

    /// The place entity `id` holds, as it leaves: kept for the element (returned) if `keep`,
    /// else given back.
    fn leave_or_keep(&self, id: u64, keep: bool) -> Option<ResourceGuard> {
        if keep {
            self.held.borrow_mut().remove(&id)
        } else {
            self.leave(id);
            None
        }
    }

    /// Gives back the place entity `id` holds, as it leaves.
    fn leave(&self, id: u64) {
        if let Room::Belt(b) = &self.room {
            b.off(id);
        }
        let guard = self.held.borrow_mut().remove(&id);
        if guard.is_some() {
            drop(guard);
            for w in self.watchers.borrow().iter() {
                w.notify_all();
            }
        }
        // One fewer may be waiting (one gave up).
        self.notify();
    }

    /// Whether a place is free now (or, with a limit, an entity is admitted now).
    fn has_room(&self) -> bool {
        match &self.room {
            Room::Fixed(pool) => pool.in_use() < pool.capacity(),
            Room::Limit(l) => l.admits(),
            Room::Belt(b) => b.admits(),
        }
    }

    /// Gives back a place held for no entity (a broken unit's, [`reserve_place`]).
    fn give_back(&self, guard: ResourceGuard) {
        drop(guard);
        for w in self.watchers.borrow().iter() {
            w.notify_all();
        }
    }
}

/// A server's transfer time, and the stream it is drawn from.
struct Transfer {
    time: Dist,
    /// Whether a sending server's unit stays with the part through the transfer.
    holds_sender: bool,
    rng: RefCell<Rng>,
}

/// Where an entity that can still give up is at a patient server.
enum Stage {
    /// Waiting for a unit: in the server's input, or held by its dispatcher until a unit is
    /// free (a server whose units fail, follow a calendar or set up).
    Waiting,
    /// In transfer here; with the time it had been here when its patience ran out, if it has.
    Transit(Option<f64>),
    /// Taken by a unit, patience until done: a copy for its stand-in, the job's work, and
    /// whether it has given up.
    Serving { copy: Entity, work: f64, gone: bool },
}

/// One entity's stay at a patient server: which stay (an entity can come back later, as rework
/// or a retry, with a deadline of its own) and where it is.
struct Stay {
    visit: u64,
    stage: Stage,
}

/// A server's patience while the model runs ([`stochcraft_model::Patience`]): the deadline of
/// each entity that enters, and the way out for those that give up.
///
/// Each entity entering starts a timer that runs out after every other event at its deadline
/// ([`Ctx::hold_late`]), so a unit taking the entity at the deadline itself serves it, as a
/// hand-over beats a deadline in `des`. The timer finds the entity where it is then: waiting, it
/// is taken out of the queue and gives up; in transfer, it gives up as it arrives; in service
/// (patience until done), a stand-in gives up in its place and the unit discards the original
/// when its job is done; gone on, or served (patience until start), nothing happens.
struct Patient {
    /// The server.
    index: usize,
    until: PatienceUntil,
    time: Dist,
    rng: RefCell<Rng>,
    ctx: Ctx,
    input: Store<Entity>,
    places: Option<Rc<Places>>,
    stats: Rc<Vec<RefCell<BlockStats>>>,
    /// The way out marked for those who give up; `None`: they leave the model.
    way: RefCell<Option<Port>>,
    /// Ids for stand-ins.
    next_id: Rc<RefCell<u64>>,
    /// Each entity here that can still give up.
    stays: RefCell<BTreeMap<u64, Stay>>,
    visits: Cell<u64>,
    /// The entity the dispatcher holds while it waits for a unit (still waiting).
    held: RefCell<Option<Entity>>,
}

impl Patient {
    /// Entity `id` enters (now), waiting or in transfer: its deadline starts.
    fn arm(self: &Rc<Self>, id: u64, stage: Stage) {
        let visit = self.visits.get() + 1;
        self.visits.set(visit);
        self.stays.borrow_mut().insert(id, Stay { visit, stage });
        let p = self.time.sample(&mut self.rng.borrow_mut());
        // An infinite patience never runs out; negative or undefined ones count as 0.
        if p == f64::INFINITY {
            return;
        }
        let p = if p > 0.0 { p } else { 0.0 };
        let (me, c) = (Rc::clone(self), self.ctx.clone());
        self.ctx.spawn_tagged(tag(self.index), async move {
            c.hold_late(p).await;
            me.run_out(id, visit, p);
        });
    }

    /// Entity `id`'s patience, `waited` long, has run out on its stay `visit`.
    fn run_out(&self, id: u64, visit: u64, waited: f64) {
        let stage = {
            let mut stays = self.stays.borrow_mut();
            match stays.get_mut(&id) {
                Some(s) if s.visit == visit => match &mut s.stage {
                    Stage::Waiting => None,
                    Stage::Transit(expired) => {
                        *expired = Some(waited);
                        return;
                    }
                    Stage::Serving { copy, gone, .. } if !*gone => {
                        *gone = true;
                        Some(copy.clone())
                    }
                    Stage::Serving { .. } => return,
                },
                _ => return,
            }
        };
        match stage {
            // In service: a stand-in gives up now; the unit discards the original later.
            Some(mut stand_in) => {
                stand_in.id = {
                    let mut n = self.next_id.borrow_mut();
                    let id = *n;
                    *n += 1;
                    id
                };
                // The original stays inside until its unit is done with it.
                if let Some(s) = self.stats.get(self.index) {
                    s.borrow_mut().given_up_inside += 1;
                }
                self.give_up(stand_in, waited);
            }
            None => {
                let held = {
                    let mut h = self.held.borrow_mut();
                    if h.as_ref().is_some_and(|e| e.id == id) {
                        h.take()
                    } else {
                        None
                    }
                };
                let Some(e) = held.or_else(|| self.input.take_where(|e| e.id == id)) else {
                    return;
                };
                self.stays.borrow_mut().remove(&id);
                let now = self.ctx.now();
                if let Some(s) = self.stats.get(self.index) {
                    let mut s = s.borrow_mut();
                    let w = s.waiting.value() - 1.0;
                    s.waiting.set(now, w);
                    s.stay_ends(now);
                }
                if let Some(p) = &self.places {
                    p.leave(id);
                }
                self.give_up(e, waited);
            }
        }
    }

    /// Counts `e`, which had been here `waited` long, as giving up, and sends it the way marked
    /// for it (if there is one; else it leaves the model).
    fn give_up(&self, e: Entity, waited: f64) {
        if let Some(s) = self.stats.get(self.index) {
            let mut s = s.borrow_mut();
            s.abandoned += 1;
            s.abandoned_wait.add(waited);
        }
        let Some(way) = self.way.borrow().clone() else {
            return;
        };
        if way.needs_room() {
            // It waits for room on its own, as a process (or is turned away there).
            leave_by(&self.ctx, &self.stats, self.index, way, e);
        } else {
            way.dispatch(&self.ctx, e);
        }
    }

    /// Entity `e`'s transfer here is over: it joins the queue (`None`), or, if its patience ran
    /// out on the way, it gives up now, having been here as long as its patience (returned).
    fn arrive(&self, e: &Entity) -> Option<f64> {
        let mut stays = self.stays.borrow_mut();
        let stay = stays.get_mut(&e.id)?;
        match stay.stage {
            Stage::Transit(Some(waited)) => {
                stays.remove(&e.id);
                Some(waited)
            }
            _ => {
                stay.stage = Stage::Waiting;
                None
            }
        }
    }

    /// A unit takes `e` for a job of `work`: served, with patience until start; with patience
    /// until done, it can still give up until the unit hands it on.
    fn start(&self, e: &Entity, work: f64) {
        let mut stays = self.stays.borrow_mut();
        match self.until {
            PatienceUntil::Start => {
                stays.remove(&e.id);
            }
            PatienceUntil::Done => {
                if let Some(s) = stays.get_mut(&e.id) {
                    s.stage = Stage::Serving {
                        copy: e.clone(),
                        work: if work > 0.0 { work } else { 0.0 },
                        gone: false,
                    };
                }
            }
        }
    }

    /// Whether entity `id`, whose job is done, gave up meanwhile: then the work it took, and
    /// the entity is no longer here. Otherwise it is handed on, and can no longer give up here.
    fn done(&self, id: u64) -> Option<f64> {
        let mut stays = self.stays.borrow_mut();
        match stays.get(&id).map(|s| &s.stage) {
            Some(Stage::Serving {
                work, gone: true, ..
            }) => {
                let work = *work;
                stays.remove(&id);
                self.given_up_goes();
                Some(work)
            }
            _ => None,
        }
    }

    /// Entity `id` leaves the server, handed on.
    fn handed_on(&self, id: u64) {
        self.stays.borrow_mut().remove(&id);
    }

    /// Entity `id`'s job stopped as its unit was taken out by a health check, and it goes back
    /// to the queue: a new attempt, with a fresh patience. Returns `false` if, with patience
    /// until done, it had given up during the job already (its stand-in has gone): then it is no
    /// longer here.
    fn retry(self: &Rc<Self>, id: u64) -> bool {
        let gone = matches!(
            self.stays.borrow_mut().remove(&id).map(|s| s.stage),
            Some(Stage::Serving { gone: true, .. })
        );
        if gone {
            self.given_up_goes();
        } else {
            self.arm(id, Stage::Waiting);
        }
        !gone
    }

    /// An entity that gave up during its job, still inside, goes now (its job ended or
    /// stopped).
    fn given_up_goes(&self) {
        if let Some(s) = self.stats.get(self.index) {
            let mut s = s.borrow_mut();
            s.given_up_inside = s.given_up_inside.saturating_sub(1);
        }
    }
}

/// A shortest-queue gateway's ways ([`stochcraft_model::Routing::ShortestQueue`]).
///
/// A freed place goes first to a plain waiter for that buffer (a [`Resource`] hands it straight
/// to whoever waits in [`Port::make_room`]), so a router woken then finds no room and waits on.
struct Router {
    /// The gateway (its statistics).
    index: usize,
    /// How it chooses a way.
    rule: Rule,
    /// Its ways, as plain ports.
    ways: Vec<Port>,
    /// When each way was last taken, as a count of the choices made (0: never).
    taken: RefCell<Vec<u64>>,
    choices: Cell<u64>,
    /// Woken whenever a place is given back on any way.
    freed: Signal,
}

/// How a gateway that its senders route through chooses a way.
enum Rule {
    /// [`stochcraft_model::Routing::ShortestQueue`].
    ShortestQueue,
    /// [`stochcraft_model::Routing::Divert`].
    Divert(Divert),
}

/// A divert gateway's ways, with what the standard choice needs.
struct Divert {
    outs: Vec<Out>,
    assigner: Rc<Assigner>,
    rng: RefCell<Rng>,
}

impl Divert {
    /// The way the standard rule chooses for `e` (drawn once per entity).
    fn first(&self, e: &Entity) -> Option<usize> {
        choose(&self.outs, e, &self.assigner, &mut self.rng.borrow_mut())
    }

    fn default(&self) -> Option<usize> {
        self.outs.iter().position(|o| o.default)
    }
}

impl Router {
    fn new(ctx: &Ctx, index: usize, ways: Vec<Port>, rule: Rule) -> Self {
        let freed = Signal::new(ctx);
        for w in &ways {
            if let Some(p) = &w.places {
                p.watchers.borrow_mut().push(freed.clone());
            }
        }
        Self {
            index,
            rule,
            taken: RefCell::new(vec![0; ways.len()]),
            ways,
            choices: Cell::new(0),
            freed,
        }
    }

    /// Whether some way has a finite buffer (so that a sender can be held up).
    fn may_block(&self) -> bool {
        self.ways.iter().any(|w| w.places.is_some())
    }

    /// The way to take now: of those with room, the one with the fewest entities waiting or in
    /// transit, then the one taken longest ago, then the first; `None` if none has room. Taking
    /// it is recorded.
    fn choose(&self) -> Option<&Port> {
        let taken = self.taken.borrow();
        let best = self
            .ways
            .iter()
            .enumerate()
            .filter(|(_, w)| w.places.as_ref().is_none_or(|p| p.has_room()))
            .min_by(|(a, wa), (b, wb)| {
                wa.queue()
                    .total_cmp(&wb.queue())
                    .then(taken.get(*a).cmp(&taken.get(*b)))
                    .then(a.cmp(b))
            })
            .map(|(i, _)| i);
        drop(taken);
        let i = best?;
        let n = self.choices.get() + 1;
        self.choices.set(n);
        if let Some(t) = self.taken.borrow_mut().get_mut(i) {
            *t = n;
        }
        self.ways.get(i)
    }

    /// Waits until some way has room, takes a place there for `e`, and returns that way.
    async fn route(&self, e: &Entity) -> Port {
        let first = self.first(e);
        loop {
            // Nothing waits between looking for room and taking it, nor before starting to wait.
            if let Some(way) = self.route_with(e, first).await {
                return way;
            }
            self.freed.wait().await;
        }
    }

    /// Takes a place for `e` on the way to take now and returns it; `None` if no way has room.
    async fn route_now(&self, e: &Entity) -> Option<Port> {
        let first = self.first(e);
        self.route_with(e, first).await
    }

    /// A divert's choice for `e`, made once; none for a shortest-queue gateway.
    fn first(&self, e: &Entity) -> Option<usize> {
        match &self.rule {
            Rule::ShortestQueue => None,
            Rule::Divert(d) => d.first(e),
        }
    }

    /// The way to take now: a shortest-queue gateway's [`Router::choose`]; a divert's first
    /// choice if it has room, else its default if that has.
    fn take(&self, first: Option<usize>) -> Option<&Port> {
        match &self.rule {
            Rule::ShortestQueue => self.choose(),
            Rule::Divert(d) => {
                let room = |i: &usize| {
                    self.ways
                        .get(*i)
                        .is_some_and(|w| w.places.as_ref().is_none_or(|p| p.has_room()))
                };
                let i = first.filter(room).or_else(|| d.default().filter(room))?;
                self.ways.get(i)
            }
        }
    }

    async fn route_with(&self, e: &Entity, first: Option<usize>) -> Option<Port> {
        let way = self.take(first)?.clone();
        if let Some(p) = &way.places {
            // `choose` found room, which a request takes at once.
            p.wait_room(e.id).await;
        }
        Some(way)
    }
}

impl Port {
    /// Entities waiting at the next element, or in transit to it.
    fn queue(&self) -> f64 {
        self.stats.get(self.index).map_or(0.0, |s| {
            let s = s.borrow();
            s.waiting.value() + s.transit.value()
        })
    }

    /// Whether sending here can be held up by a full buffer (not one that turns away those
    /// finding no room).
    fn may_block(&self) -> bool {
        match &self.router {
            Some(r) => r.may_block(),
            None => self.places.as_ref().is_some_and(|p| !p.turns_away()),
        }
    }

    /// Whether sending here must first find room ([`Port::deliver`]): a finite buffer, whether
    /// it holds the sender up or turns away those finding none, or a shortest-queue gateway
    /// with one; or must first see whether the entity joins (a server with balking). Only where
    /// this is false may an entity be sent straight on ([`Port::dispatch`]).
    fn needs_room(&self) -> bool {
        match &self.router {
            Some(r) => r.may_block(),
            None => self.places.is_some() || self.balk.is_some(),
        }
    }

    /// The next element's balking, if it has one.
    fn balk(&self) -> Option<Rc<Balk>> {
        self.balk.as_ref().and_then(Weak::upgrade)
    }

    /// Whether `e`, arriving at the next element now, balks there.
    fn balks(&self) -> bool {
        self.balk().is_some_and(|b| b.balks())
    }

    /// Whether a sending server's unit stays with the part through the transfer here.
    fn holds_sender(&self) -> bool {
        self.transfer.as_ref().is_some_and(|t| t.holds_sender)
    }

    /// `e` enters the shortest-queue gateway of `router`, on its way to one of its ways.
    /// Counts `e` as rework against the last server it finished at, if this way is rework.
    fn count_rework(&self, e: &Entity) {
        if self.rework
            && let Some(s) = e.last_server.and_then(|i| self.stats.get(i))
        {
            s.borrow_mut().rework += 1;
        }
    }

    fn enter_gateway(&self, router: &Router, e: &mut Entity) {
        self.count_rework(e);
        self.assigner.enter(router.index, e, &self.stats);
        if let Some(s) = self.stats.get(router.index) {
            s.borrow_mut().entered += 1;
        }
    }

    /// An entity leaves the shortest-queue gateway of `router`, at the moment it gets a way, or
    /// is lost there.
    fn leave_gateway(&self, router: &Router, lost: bool) {
        if let Some(s) = self.stats.get(router.index) {
            let mut s = s.borrow_mut();
            if lost {
                s.lost += 1;
            } else {
                s.left += 1;
            }
        }
    }

    /// Waits for a place in the next element's finite buffer, if it has one, and takes it for
    /// `e`; returns where `e` goes: here, or, through a shortest-queue gateway, the way chosen.
    /// An error if `e` balks there (decided first, as it arrives, before it asks for room), or
    /// the buffer turns away those finding no room and has none now: then `e` goes to
    /// [`Port::refuse`] instead.
    async fn make_room(&self, e: &mut Entity) -> Result<Port, Refused> {
        if let Some(r) = &self.router {
            self.enter_gateway(r, e);
            let way = r.route(e).await;
            self.leave_gateway(r, false);
            return Ok(way);
        }
        if self.balks() {
            return Err(Refused::Balked);
        }
        if let Some(p) = &self.places {
            if p.turns_away() {
                if !p.try_room(e.id).await {
                    return Err(Refused::TurnedAway);
                }
            } else {
                p.wait_room(e.id).await;
            }
        }
        Ok(self.clone())
    }

    /// Sends `e`, first seeing whether it joins (balking) and waiting for room in a finite
    /// buffer (or, finding none in one that turns away, down its way for those); any transfer
    /// goes on by itself.
    async fn deliver(&self, ctx: &Ctx, mut e: Entity) {
        match self.make_room(&mut e).await {
            Ok(to) => to.dispatch(ctx, e),
            Err(why) => self.refuse(e, why),
        }
    }

    /// `e` did not get into the next element, for the reason given.
    fn refuse(&self, e: Entity, why: Refused) {
        match why {
            Refused::TurnedAway => self.turn_away(e),
            Refused::Balked => {
                if let Some(b) = self.balk() {
                    b.take(e);
                }
            }
        }
    }

    /// `e` finds no room in the next element's finite buffer: it is turned away, counted as
    /// lost there, and takes the element's way out for those, if it has one (else it is gone).
    fn turn_away(&self, e: Entity) {
        if let Some(s) = self.stats.get(self.index) {
            s.borrow_mut().lost += 1;
        }
        if let Some(away) = self.places.as_ref().and_then(|p| p.away()) {
            away.take(e);
        }
    }

    /// Sends `e`, which already has its place: at once, or into a transfer that goes on by
    /// itself (the sender holds nothing while it lasts).
    fn dispatch(&self, ctx: &Ctx, e: Entity) {
        self.travel(ctx, e, self.transfer_time());
    }

    /// Sends `e`: at once, or with `transfer`, into a transfer of that time that goes on by
    /// itself.
    fn travel(&self, ctx: &Ctx, e: Entity, transfer: Option<f64>) {
        match transfer {
            None => self.send(ctx.now(), e),
            Some(dt) => {
                let e = self.depart(ctx.now(), e);
                let (c, port) = (ctx.clone(), self.clone());
                ctx.spawn(async move {
                    c.hold(dt).await;
                    port.arrive(c.now(), e);
                });
            }
        }
    }

    /// A transfer time drawn for the next entity, if the next element has one (negative draws
    /// count as 0).
    fn transfer_time(&self) -> Option<f64> {
        let t = self.transfer.as_ref()?;
        let dt = t.time.sample(&mut t.rng.borrow_mut());
        Some(if dt.is_finite() && dt > 0.0 { dt } else { 0.0 })
    }

    /// `e` sets off for the next element, into a transfer: it enters it (and is counted
    /// there, in transit) now, and joins its queue when the transfer ends ([`Port::arrive`]).
    fn depart(&self, now: f64, mut e: Entity) -> Entity {
        self.count_rework(&e);
        self.assigner.enter(self.index, &mut e, &self.stats);
        if let Some(s) = self.stats.get(self.index) {
            let mut s = s.borrow_mut();
            s.entered += 1;
            s.stay_starts(now);
            let n = s.transit.value() + 1.0;
            s.transit.set(now, n);
        }
        if let Some(p) = self.patient() {
            p.arm(e.id, Stage::Transit(None));
        }
        e
    }

    /// The next element's patience, if it has one.
    fn patient(&self) -> Option<Rc<Patient>> {
        self.patient.as_ref().and_then(Weak::upgrade)
    }

    /// `e`'s transfer is over: it joins the next element's queue, unless its patience there ran
    /// out on the way, when it gives up now (counted as it does).
    fn arrive(&self, now: f64, mut e: Entity) {
        let patient = self.patient();
        let gave_up = patient.as_ref().and_then(|p| p.arrive(&e));
        e.entered = now;
        if let Some(s) = self.stats.get(self.index) {
            let mut s = s.borrow_mut();
            let n = s.transit.value() - 1.0;
            s.transit.set(now, n);
            if gave_up.is_none() {
                let w = s.waiting.value() + 1.0;
                s.waiting.set(now, w);
            } else {
                s.stay_ends(now);
            }
        }
        match (patient, gave_up) {
            (Some(p), Some(waited)) => {
                if let Some(places) = &self.places {
                    places.leave(e.id);
                }
                p.give_up(e, waited);
            }
            _ => self.store.put(e),
        }
    }

    /// Sends a new arrival if there is room (a place freed at this instant counts) and returns
    /// `true`; otherwise the arrival is lost, counted as lost by the next element, and this
    /// returns `false`. An arrival balking there returns `true` (it left the source). Through a shortest-queue gateway, the arrival is lost (counted by the
    /// gateway) if no way has room now.
    async fn offer(&self, ctx: &Ctx, mut e: Entity) -> bool {
        if let Some(r) = &self.router {
            self.enter_gateway(r, &mut e);
            let way = r.route_now(&e).await;
            self.leave_gateway(r, way.is_none());
            return match way {
                Some(way) => {
                    way.dispatch(ctx, e);
                    true
                }
                None => false,
            };
        }
        // An arrival that balks has left its source all the same; the server counts it.
        if let Some(b) = self.balk()
            && b.balks()
        {
            b.take(e);
            return true;
        }
        if let Some(p) = &self.places
            && !p.try_room(e.id).await
        {
            self.turn_away(e);
            return false;
        }
        self.dispatch(ctx, e);
        true
    }

    fn send(&self, now: f64, mut e: Entity) {
        e.entered = now;
        self.count_rework(&e);
        self.assigner.enter(self.index, &mut e, &self.stats);
        if let Some(s) = self.stats.get(self.index) {
            let mut s = s.borrow_mut();
            s.entered += 1;
            s.stay_starts(now);
            let w = s.waiting.value() + 1.0;
            s.waiting.set(now, w);
        }
        if let Some(p) = self.patient() {
            p.arm(e.id, Stage::Waiting);
        }
        self.store.put(e);
    }
}

/// One of a gateway's outgoing connections, ready to route.
struct Out {
    port: Port,
    weight: f64,
    condition: Option<stochcraft_model::expr::Expr>,
    default: bool,
}

struct BlockEnv {
    ctx: Ctx,
    stats: Rc<Vec<RefCell<BlockStats>>>,
    assigner: Rc<Assigner>,
    me: usize,
    /// This element's places, when its buffer is finite.
    places: Option<Rc<Places>>,
    input: Store<Entity>,
    out: Option<Port>,
    rng: Rng,
    /// Whether the element's units stay with an entity through a transfer to the next element
    /// (a server, a task with shared resources); otherwise the entity travels on its own.
    holds: bool,
    /// A server's own patience, if it has one.
    patient: Option<Rc<Patient>>,
}

impl BlockEnv {
    fn with_stats(&self, f: impl FnOnce(&mut BlockStats)) {
        if let Some(s) = self.stats.get(self.me) {
            f(&mut s.borrow_mut());
        }
    }

    fn adjust(&self, field: fn(&mut BlockStats) -> &mut TimeWeighted, delta: f64) {
        let now = self.ctx.now();
        self.with_stats(|s| {
            let tw = field(s);
            let v = tw.value() + delta;
            tw.set(now, v);
        });
    }
}

/// The time between arrivals over a run: from each time on, until the next, the distribution
/// that applies (`None`: no arrivals).
type Rates = Vec<(f64, Option<Dist>)>;

fn spawn_source(
    sim: &mut Sim,
    mut env: BlockEnv,
    rates: Rates,
    limit: Option<u64>,
    next_id: Rc<RefCell<u64>>,
    first_at_zero: bool,
) {
    sim.spawn_tagged(tag(env.me), async move {
        let mut made = 0u64;
        // The period of `rates` now under way.
        let mut k = 0usize;
        while limit.is_none_or(|l| made < l) {
            let arrive_now =
                first_at_zero && made == 0 && rates.first().is_some_and(|(_, d)| d.is_some());
            if !arrive_now {
                // When the rate changes before the next arrival, that arrival is dropped and the
                // next is drawn from the new rate at the change (BPSim: a calendar value takes
                // effect at once).
                loop {
                    let now = env.ctx.now();
                    let change = rates.get(k + 1).map(|(t, _)| *t);
                    match rates.get(k).and_then(|(_, d)| d.as_ref()) {
                        None => match change {
                            Some(c) => {
                                env.ctx.hold(c - now).await;
                                k += 1;
                            }
                            None => return,
                        },
                        Some(d) => {
                            let dt = d.sample(&mut env.rng);
                            match change {
                                Some(c) if now + dt > c => {
                                    env.ctx.hold(c - now).await;
                                    k += 1;
                                }
                                _ => {
                                    env.ctx.hold(dt).await;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            let now = env.ctx.now();
            let id = {
                let mut n = next_id.borrow_mut();
                let id = *n;
                *n += 1;
                id
            };
            made += 1;
            env.with_stats(|s| s.entered += 1);
            let mut entity = Entity {
                id,
                created: now,
                entered: now,
                properties: Vec::new(),
                members: Vec::new(),
                last_server: None,
            };
            // A source sets its properties on the entities it creates.
            env.assigner.enter(env.me, &mut entity, &env.stats);
            // It leaves, unless the next element's finite buffer is full and it is lost.
            let sent = match &env.out {
                Some(out) => out.offer(&env.ctx, entity).await,
                None => true,
            };
            env.with_stats(|s| {
                if sent {
                    s.left += 1;
                } else {
                    s.lost += 1;
                }
            });
        }
    });
}

/// The model's calendars over one run: when each is active, in model time.
struct Calendars {
    periods: BTreeMap<ElementId, Vec<(f64, f64)>>,
    end: f64,
}

impl Calendars {
    fn new(model: &Model, end: f64) -> Self {
        let origin = model
            .experiment
            .start
            .or_else(|| Calendar::default_origin(&model.calendars))
            .unwrap_or(DateTime(0));
        let unit = model.experiment.time_unit;
        Self {
            periods: model
                .calendars
                .iter()
                .map(|c| (c.id.clone(), c.periods(origin, unit, end)))
                .collect(),
            end,
        }
    }

    fn timeline<T>(&self, timed: &[Timed<T>]) -> Vec<(f64, Option<usize>)> {
        if timed.is_empty() {
            return vec![(0.0, None)];
        }
        let periods = |c: &ElementId| self.periods.get(c).cloned().unwrap_or_default();
        calendar::timeline(timed, &periods, self.end)
    }

    /// A source's time between arrivals over the run.
    fn arrivals(&self, default: Dist, timed: &[Timed<Dist>], idle_outside: bool) -> Rates {
        self.timeline(timed)
            .into_iter()
            .map(|(t, i)| {
                let d = match i {
                    Some(i) => timed.get(i).map(|x| x.value.clone()),
                    None => (!idle_outside).then(|| default.clone()),
                };
                (t, d)
            })
            .collect()
    }

    /// A pool's units over the run: its shift's quantity while it is available, else none.
    fn units(&self, r: &ResourcePool) -> Vec<(f64, u32)> {
        self.units_over(r.quantity, &r.shifts, r.available, &r.availability)
    }

    /// Units on shift over the run, from each time on until the next (the first at 0): the
    /// shift's units (`quantity` outside every one of `shifts`) while available, else none. A
    /// pool's, or a server's own.
    fn units_over(
        &self,
        quantity: u32,
        shifts: &[Timed<u32>],
        available: bool,
        availability: &[Timed<bool>],
    ) -> Vec<(f64, u32)> {
        let q = self.timeline(shifts);
        let a = self.timeline(availability);
        let mut cuts: Vec<f64> = q.iter().chain(&a).map(|(t, _)| *t).collect();
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let at = |line: &[(f64, Option<usize>)], t: f64| {
            line.iter()
                .take_while(|(s, _)| *s <= t)
                .last()
                .and_then(|(_, i)| *i)
        };
        let mut out: Vec<(f64, u32)> = Vec::new();
        for t in cuts {
            let quantity = at(&q, t)
                .and_then(|i| shifts.get(i))
                .map_or(quantity, |s| s.value);
            let available = at(&a, t)
                .and_then(|i| availability.get(i))
                .map_or(available, |s| s.value);
            let units = if available { quantity } else { 0 };
            if out.last().is_none_or(|(_, u)| *u != units) {
                out.push((t, units));
            }
        }
        out
    }
}

/// Changes pool `pool`'s units at each time of `changes` (the first, at 0, is already set).
fn spawn_shifts(sim: &mut Sim, pools: Rc<Pools>, pool: usize, changes: Vec<(f64, u32)>) {
    let ctx = pools.ctx.clone();
    sim.spawn(async move {
        for (t, units) in changes.into_iter().skip(1) {
            let now = ctx.now();
            ctx.hold(t - now).await;
            pools.set_units(pool, units);
        }
    });
}

fn spawn_server(sim: &mut Sim, env: BlockEnv, capacity: u32, service: Rc<Times>) {
    let me = tag(env.me);
    let env = Rc::new(RefCell::new(env));
    let book = UnitBook::new(&env.borrow(), capacity);
    for u in 0..capacity as usize {
        let env = Rc::clone(&env);
        let book = Rc::clone(&book);
        let service = Rc::clone(&service);
        sim.spawn_tagged(me, async move {
            loop {
                let (input, ctx) = {
                    let e = env.borrow();
                    (e.input.clone(), e.ctx.clone())
                };
                let entity = input.get().await;
                let duration = {
                    let mut e = env.borrow_mut();
                    let now = e.ctx.now();
                    e.with_stats(|s| s.wait.add(now - entity.entered));
                    e.adjust(|s| &mut s.waiting, -1.0);
                    e.adjust(|s| &mut s.busy, 1.0);
                    book.change(&e, u, |d| d.processing = true);
                    service.sample(&entity, None, &mut e.rng)
                };
                if let Some(p) = &env.borrow().patient {
                    p.start(&entity, duration);
                }
                ctx.hold(duration).await;
                {
                    let e = env.borrow();
                    e.adjust(|s| &mut s.busy, -1.0);
                    book.change(&e, u, |d| {
                        d.processing = false;
                        d.holding = true;
                    });
                }
                let mut entity = entity;
                entity.last_server = Some(env.borrow().me);
                env.borrow().with_stats(|s| s.produced += 1);
                move_on(&env, &ctx, entity, &|| false, None).await;
                book.change(&env.borrow(), u, |d| d.holding = false);
            }
        });
    }
}

/// One unit of a server whose units fail or follow a calendar.
#[derive(Default)]
struct Unit {
    /// Serving an entity (possibly interrupted by a failure), or blocked after service.
    busy: bool,
    /// Failed or under repair.
    down: bool,
    /// Taken out of service by a health check, restarting.
    out: bool,
    /// Off shift: it takes no entity, and is not checked. One that went off with a job may
    /// still be finishing it (`leaving`), or hold it paused ([`OnShiftEnd::Resume`]).
    off: bool,
    /// Off shift but finishing the job or handing on the entity it holds first; it counts as on
    /// shift until it is free ([`OnShiftEnd::Finish`]).
    leaving: bool,
    /// When it took the entity it holds (the busy unit that started last goes off first).
    started: f64,
    /// The job process while it is in service (so a failure or a health check can interrupt
    /// it).
    serving: Option<u64>,
    /// A failure is due and waits for the job to finish (non-interruptible).
    due: bool,
    /// While down (or out restarting, with [`PlaceWhileOut::Reserved`]) with no entity: the
    /// server's place it keeps, so that no entity gets in for it ([`reserve_place`]).
    reserved: Option<ResourceGuard>,
    /// For a changeover: the value of the watched property of the last entity it served
    /// (`None` before its first; `Some(None)` when that entity had none).
    last: Option<Option<String>>,
}

/// A server's setup, with the slot of the property a changeover watches.
struct Setups {
    setup: Setup,
    slot: Option<usize>,
}

impl Setups {
    /// The setup time before serving `entity` on unit `x`, if it needs one; for a changeover,
    /// the unit now remembers the entity's value.
    fn before(&self, entity: &Entity, x: &mut Unit, rng: &mut Rng) -> Option<f64> {
        if self.setup.on_change_of.is_none() {
            return Some(self.setup.time.sample(rng));
        }
        let value = self
            .slot
            .and_then(|k| entity.properties.get(k).cloned().flatten())
            .map(|h| h.value().text());
        match x.last.replace(value.clone()) {
            Some(last) if last != value => {
                let (from, to) = (last.unwrap_or_default(), value.unwrap_or_default());
                Some(self.setup.time_for(&from, &to).sample(rng))
            }
            _ => None,
        }
    }
}

/// Mixed into the seed of a server's failure stream, so it is not its service stream's or its
/// setup or health check streams'.
const BREAKDOWN_STREAM: u64 = 0x2c9b_0d37_a6f1_85e3;

/// Mixed into the seed of a server's setup stream, so it is not its service stream's.
const SETUP_STREAM: u64 = 0x5e7f_a11d_c4a9_6e01;

/// Mixed into the seed of a server's health check stream, so it is not its service stream's
/// or its setup stream's.
const HEALTH_STREAM: u64 = 0x4ea1_7c4e_c0ff_ee05;

/// Mixed into the seed of a task's stream for its processing times by property value, so it is
/// not its service stream's, its setup stream's or its health check stream's.
const BY_VALUE_STREAM: u64 = 0x7b1d_e5a1_0c3f_92d7;

/// Mixed into the seed of the streams of a shared task's service times by serving pool, so they
/// are not its service stream's or its times by property's.
const BY_RESOURCE_STREAM: u64 = 0x3d6a_94c2_b81f_e70b;

/// A task's processing time. An entity served by a pool that has an entry of the task's times by
/// pool ([`stochcraft_model::ProcessingByResource`]) takes that entry's time. Otherwise, an entity
/// whose value of the property has an entry of its times by property
/// ([`stochcraft_model::ProcessingBy`]) takes that entry's time. Any other takes the task's own
/// time, from its service stream as without entries. Each entry draws from a stream of its
/// own, seeded without drawing from the element's own, so a model without entries, or whose
/// entries no entity matches, draws exactly what it did before, and the times of the other
/// entities are the service stream's draws in the same order whatever the entries.
struct Times {
    default: Dist,
    by: Option<ByValues>,
    by_resource: Vec<ByResource>,
}

/// One entry of a shared task's service times by pool: the pool (its index in the model's
/// resources), its time, and the stream it draws from.
struct ByResource {
    pool: usize,
    time: Dist,
    rng: RefCell<Rng>,
}

/// The entries of a task's processing times by property value, the slot of the property they
/// read, and the stream they draw from.
struct ByValues {
    by: stochcraft_model::ProcessingBy,
    slot: Option<usize>,
    rng: RefCell<Rng>,
}

impl Times {
    /// The times of a task whose own time is `default`, with entries `by`; `slots` are the
    /// model's property slots and `rng` the task's own stream (not drawn from).
    fn new(
        default: Dist,
        by: Option<stochcraft_model::ProcessingBy>,
        slots: &[String],
        rng: &Rng,
    ) -> Rc<Self> {
        Self::with_pools(default, by, &[], slots, rng)
    }

    /// As [`Times::new`], with the entries of a shared task's times by pool: each a pool (its
    /// index in the model's resources), its id and its time. An entry's stream is keyed by its
    /// pool's id, not by its place in the list, so adding, removing or reordering one entry
    /// leaves the other pools' draws as they were.
    fn with_pools(
        default: Dist,
        by: Option<stochcraft_model::ProcessingBy>,
        by_pool: &[(usize, ElementId, Dist)],
        slots: &[String],
        rng: &Rng,
    ) -> Rc<Self> {
        let by = by.filter(|b| !b.by_value.is_empty()).map(|by| ByValues {
            slot: slots
                .binary_search_by(|s| s.as_str().cmp(by.property.trim()))
                .ok(),
            by,
            rng: RefCell::new(Rng::new(rng.clone().next_u64() ^ BY_VALUE_STREAM)),
        });
        let seed = rng.clone().next_u64() ^ BY_RESOURCE_STREAM;
        let by_resource = by_pool
            .iter()
            .map(|(pool, id, time)| ByResource {
                pool: *pool,
                time: time.clone(),
                rng: RefCell::new(Rng::keyed(seed, &format!("by-resource/{}", id.as_str()))),
            })
            .collect();
        Rc::new(Self {
            default,
            by,
            by_resource,
        })
    }

    /// The processing time of `entity`, served by units of pool `pool` (its index in the model's
    /// resources; `None` for a task without pools): the pool's entry, else the entity's entry
    /// by property, else from `rng` (the task's own stream).
    fn sample(&self, entity: &Entity, pool: Option<usize>, rng: &mut Rng) -> f64 {
        if let Some(p) = pool
            && let Some(r) = self.by_resource.iter().find(|r| r.pool == p)
        {
            return r.time.sample(&mut r.rng.borrow_mut());
        }
        if let Some(b) = &self.by {
            let value = b
                .slot
                .and_then(|k| entity.properties.get(k).cloned().flatten())
                .map(|h| h.value().text());
            if let Some(d) = b.by.time_for(value.as_deref()) {
                return d.sample(&mut b.rng.borrow_mut());
            }
        }
        self.default.sample(rng)
    }
}

/// The interrupt of a job whose unit fails ([`Breakdown`]).
const FAILED: u64 = 0;

/// The interrupt of a job whose unit a health check takes out ([`HealthCheck`]).
const TAKEN_OUT: u64 = 1;

/// The interrupt of a job whose unit goes off shift ([`OnShiftEnd::Retry`] and
/// [`OnShiftEnd::Resume`]).
const SHIFT_END: u64 = 2;

/// What a server whose units fail, follow a calendar or set up runs with.
struct UnitServer {
    /// Its units in all: the most ever on shift at once.
    capacity: u32,
    service: Rc<Times>,
    breakdown: Option<Breakdown>,
    /// From each time on, until the next, the units on shift (the first at 0).
    shifts: Vec<(f64, u32)>,
    on_shift_end: OnShiftEnd,
    setup: Option<Setups>,
    health: Option<HealthCheck>,
}

/// A unit that is down and holds no entity cannot take one, so the place in the server's finite
/// buffer that would be its own is kept free of entities until its repair: the buffer then holds
/// no more than its waiting room (and the other units' places) meanwhile. Taken only if a place
/// is free at that moment (none is when every place already holds an entity, such as one taken
/// for the unit before a break). With `out_too` ([`PlaceWhileOut::Reserved`]), so does a unit
/// out restarting after a health check, until its restart ends. `own` is a place the unit has
/// in hand already (that of a job given up as it was taken out): kept if the unit keeps one,
/// else given back.
fn reserve_place(
    units: &RefCell<Vec<Unit>>,
    u: usize,
    places: Option<&Rc<Places>>,
    out_too: bool,
    own: Option<ResourceGuard>,
) {
    let Some(p) = places else { return };
    let spare = {
        let mut units = units.borrow_mut();
        match units.get_mut(u) {
            Some(x) if keeps_place(x, out_too) && !x.busy && x.reserved.is_none() => {
                x.reserved = own.or_else(|| p.reserve());
                None
            }
            _ => own,
        }
    };
    if let Some(g) = spare {
        p.give_back(g);
    }
}

/// Whether unit `x` keeps its place in a finite buffer while it holds no entity: it is down, or
/// (with `out_too`, [`PlaceWhileOut::Reserved`]) out restarting.
fn keeps_place(x: &Unit, out_too: bool) -> bool {
    x.down || (out_too && x.out)
}

/// Gives back the place unit `u` keeps ([`reserve_place`]) once it no longer should (repaired,
/// and back from any restart that keeps one).
fn release_place(units: &RefCell<Vec<Unit>>, u: usize, places: Option<&Rc<Places>>, out_too: bool) {
    let kept = units
        .borrow_mut()
        .get_mut(u)
        .filter(|x| !keeps_place(x, out_too))
        .and_then(|x| x.reserved.take());
    if let (Some(g), Some(p)) = (kept, places) {
        p.give_back(g);
    }
}

/// A server whose units fail and are repaired ([`Breakdown`]), or follow a timetable (`shifts`:
/// from each time on, how many are on shift), or both. One process hands each waiting entity,
/// first come first served, to the first unit that is up, idle and on shift; a process per unit
/// makes it fail and repairs it; and one process takes units on and off shift ([`set_units`]).
///
/// With [`OnShiftEnd::Finish`] (the default), a unit going off with a job finishes it first,
/// and one a failure interrupted resumes after the repair, shift or not: the unit is off for
/// the next entity. With `Retry`, the job stops at once and the entity goes back to the head of
/// the queue, as for a health check's retry (and the dispatcher takes a unit first, then the
/// entity); with `Resume`, it stops at once and carries on with what was left once its unit is
/// on shift again (and up, and back). Failures come in elapsed time, and repairs go on, through
/// the breaks.
///
/// With a health check ([`HealthCheck`]), a process per unit checks it and takes it out for its
/// restart; failures keep coming in elapsed time while it is out. The dispatcher then waits
/// for a free unit before it takes the next entity from the queue, so every waiting entity
/// stays in the queue, where a retried job goes ahead of them all. A retried start's wait is
/// taken back, so the mean wait runs from arrival to the start of the attempt that finished.
fn spawn_unit_server(sim: &mut Sim, env: BlockEnv, parts: UnitServer) {
    let UnitServer {
        capacity,
        service,
        breakdown,
        shifts,
        on_shift_end,
        setup,
        health,
    } = parts;
    let ctx = env.ctx.clone();
    let me = tag(env.me);
    // The failures draw from their own stream, so they do not change the service times. Like the
    // setups and health checks below, it is seeded without drawing from the element's own
    // (forking would advance it), so adding or removing a breakdown leaves the service times of
    // the same seed as they were, which keeps paired comparisons paired.
    let mut fail_rng = breakdown
        .as_ref()
        .map(|_| Rng::new(env.rng.clone().next_u64() ^ BREAKDOWN_STREAM));
    // So do the setups. Their stream is seeded without drawing from the element's own (forking
    // would), so the service times and the failures stay those of the model without setups.
    let mut setup_rng = setup
        .as_ref()
        .map(|_| Rng::new(env.rng.clone().next_u64() ^ SETUP_STREAM));
    // So do the health checks, seeded the same way: adding a check changes no other draw.
    let mut health_rng = health
        .as_ref()
        .map(|_| Rng::new(env.rng.clone().next_u64() ^ HEALTH_STREAM));
    let units: Rc<RefCell<Vec<Unit>>> = Rc::new(RefCell::new(
        (0..capacity).map(|_| Unit::default()).collect(),
    ));
    let book = UnitBook::new(&env, capacity);
    // A unit came free, was repaired, or came on shift.
    let changed = Signal::new(&ctx);
    // Each unit's repair, which an interrupted job waits for; and its job ending, which a due
    // failure waits for.
    let up: Rc<Vec<Gate>> = Rc::new((0..capacity).map(|_| Gate::new(&ctx, true)).collect());
    let done: Rc<Vec<Signal>> = Rc::new((0..capacity).map(|_| Signal::new(&ctx)).collect());
    // Each unit's being back from a health check's restart, which a resumed job waits for.
    let back: Rc<Vec<Gate>> = Rc::new((0..capacity).map(|_| Gate::new(&ctx, true)).collect());
    // A job stopped on each unit (by a failure, a takeout or its shift's end) is told when the
    // unit is repaired, back, or goes on or off shift, when it does not just finish
    // ([`OnShiftEnd`]): it then resumes, or is retried, as the rule says.
    let nudge: Rc<Vec<Signal>> = Rc::new((0..capacity).map(|_| Signal::new(&ctx)).collect());
    let stops = on_shift_end != OnShiftEnd::Finish;
    // A unit out restarting keeps a place of a finite buffer, as a failed one does.
    let out_too = health
        .as_ref()
        .is_some_and(|h| h.place_while_out == PlaceWhileOut::Reserved);
    let env = Rc::new(RefCell::new(env));
    // Fewer on shift from the start (counted from time 0).
    if let Some((_, n)) = shifts.first()
        && *n < capacity
    {
        set_units(*n, &units, &book, &env.borrow(), on_shift_end, &nudge);
    }

    if shifts.len() > 1 {
        let (c, units, book, changed, env, nudge) = (
            ctx.clone(),
            Rc::clone(&units),
            Rc::clone(&book),
            changed.clone(),
            Rc::clone(&env),
            Rc::clone(&nudge),
        );
        sim.spawn_tagged(me, async move {
            for (t, n) in shifts.into_iter().skip(1) {
                let now = c.now();
                c.hold(t - now).await;
                if set_units(n, &units, &book, &env.borrow(), on_shift_end, &nudge) {
                    changed.notify_all();
                }
            }
        });
    }

    for u in 0..capacity as usize {
        let (Some(b), Some(fail_rng)) = (breakdown.clone(), fail_rng.as_mut()) else {
            break;
        };
        let (c, units, changed, up, done, env, book, nudge) = (
            ctx.clone(),
            Rc::clone(&units),
            changed.clone(),
            Rc::clone(&up),
            Rc::clone(&done),
            Rc::clone(&env),
            Rc::clone(&book),
            Rc::clone(&nudge),
        );
        let mut rng = fail_rng.fork();
        sim.spawn_tagged(me, async move {
            loop {
                c.hold(b.time_to_failure.sample(&mut rng)).await;
                let busy = units.borrow().get(u).is_some_and(|x| x.busy);
                if busy && !b.interruptible {
                    if let Some(x) = units.borrow_mut().get_mut(u) {
                        x.due = true;
                    }
                    if let Some(d) = done.get(u) {
                        d.wait().await;
                    }
                }
                let serving = units.borrow_mut().get_mut(u).and_then(|x| {
                    x.down = true;
                    x.due = false;
                    x.serving
                });
                if let Some(g) = up.get(u) {
                    g.close();
                }
                {
                    let e = env.borrow();
                    e.adjust(|s| &mut s.down, 1.0);
                    book.change(&e, u, |d| d.down = true);
                }
                let places = env.borrow().places.clone();
                reserve_place(&units, u, places.as_ref(), out_too, None);
                if let Some(pid) = serving {
                    c.interrupt(pid, FAILED);
                }
                c.hold(b.repair.sample(&mut rng)).await;
                if let Some(x) = units.borrow_mut().get_mut(u) {
                    x.down = false;
                }
                release_place(&units, u, places.as_ref(), out_too);
                {
                    let e = env.borrow();
                    e.adjust(|s| &mut s.down, -1.0);
                    book.change(&e, u, |d| d.down = false);
                }
                if let Some(g) = up.get(u) {
                    g.open();
                }
                if stops && let Some(s) = nudge.get(u) {
                    s.notify_all();
                }
                changed.notify_all();
            }
        });
    }

    for u in 0..capacity as usize {
        let (Some(h), Some(health_rng)) = (health.clone(), health_rng.as_mut()) else {
            break;
        };
        let (c, units, changed, back, env, book, nudge) = (
            ctx.clone(),
            Rc::clone(&units),
            changed.clone(),
            Rc::clone(&back),
            Rc::clone(&env),
            Rc::clone(&book),
            Rc::clone(&nudge),
        );
        let mut rng = health_rng.fork();
        sim.spawn_tagged(me, async move {
            loop {
                c.hold(h.interval.sample(&mut rng)).await;
                if !take_out(&units, u, &h, &env) {
                    continue;
                }
                let serving = units.borrow_mut().get_mut(u).and_then(|x| {
                    x.out = true;
                    x.serving.take()
                });
                if let Some(g) = back.get(u) {
                    g.close();
                }
                {
                    let e = env.borrow();
                    e.with_stats(|s| s.taken_out += 1);
                    book.change(&e, u, |d| d.out = true);
                }
                // Its place, if it has no job (one retried takes it once the job is back in
                // the queue).
                let places = env.borrow().places.clone();
                reserve_place(&units, u, places.as_ref(), out_too, None);
                if let Some(pid) = serving {
                    c.interrupt(pid, TAKEN_OUT);
                }
                c.hold(h.restart.sample(&mut rng)).await;
                if let Some(x) = units.borrow_mut().get_mut(u) {
                    x.out = false;
                }
                release_place(&units, u, places.as_ref(), out_too);
                book.change(&env.borrow(), u, |d| d.out = false);
                if let Some(g) = back.get(u) {
                    g.open();
                }
                if stops && let Some(s) = nudge.get(u) {
                    s.notify_all();
                }
                changed.notify_all();
            }
        });
    }

    let patient = env.borrow().patient.clone();
    let checked = health.is_some();
    let retries = health
        .as_ref()
        .is_some_and(|h| h.on_takeout == OnTakeout::Retry);
    let shift_retries = on_shift_end == OnShiftEnd::Retry;
    let shift_resumes = on_shift_end == OnShiftEnd::Resume;
    sim.spawn_tagged(me, async move {
        // The first unit that is up, back, idle and on shift.
        let free = || {
            units
                .borrow()
                .iter()
                .position(|x| !x.busy && !x.down && !x.due && !x.out && !x.off)
        };
        loop {
            let input = env.borrow().input.clone();
            // Where a stopped job goes back to the head of the queue, a unit first, then the
            // entity, which so stays in the queue meanwhile.
            let (entity, unit) = if checked || shift_retries {
                // A unit first, then the entity, which so stays in the queue meanwhile.
                while free().is_none() {
                    changed.wait().await;
                }
                let entity = input.get().await;
                match free() {
                    Some(u) => (entity, u),
                    // Taken out (or failed) while the entity came: back to the head.
                    None => {
                        input.put_front(entity);
                        continue;
                    }
                }
            } else {
                let mut entity = Some(input.get().await);
                // Still waiting while it waits for a unit here, so it can still give up.
                if let Some(p) = &patient {
                    *p.held.borrow_mut() = entity.take();
                }
                // The first unit that is up, idle and on shift; else wait for one.
                let unit = loop {
                    match free() {
                        Some(u) => break u,
                        None => changed.wait().await,
                    }
                };
                let Some(entity) = entity.or_else(|| patient.as_ref()?.held.borrow_mut().take())
                else {
                    // It gave up meanwhile: the unit takes the next.
                    continue;
                };
                (entity, unit)
            };
            let set_up = match (&setup, setup_rng.as_mut()) {
                (Some(s), Some(rng)) => units
                    .borrow_mut()
                    .get_mut(unit)
                    .and_then(|x| s.before(&entity, x, rng)),
                _ => None,
            };
            let started = ctx.now();
            // The statistics this start's wait goes into (a warm-up may reset them).
            let measured = env
                .borrow()
                .stats
                .get(env.borrow().me)
                .map_or(0, |s| s.borrow().resets);
            let duration = {
                let mut e = env.borrow_mut();
                e.with_stats(|s| s.wait.add(started - entity.entered));
                e.adjust(|s| &mut s.waiting, -1.0);
                e.adjust(|s| &mut s.busy, 1.0);
                book.change(&e, unit, |d| {
                    d.processing = true;
                    d.setup = set_up.is_some();
                });
                service.sample(&entity, None, &mut e.rng)
            };
            if let Some(p) = &patient {
                p.start(&entity, set_up.unwrap_or(0.0).max(0.0) + duration.max(0.0));
            }
            if let Some(x) = units.borrow_mut().get_mut(unit) {
                x.busy = true;
                x.started = started;
            }
            let (c, units2, changed2, up2, done2, env2, book2, back2, nudge2) = (
                ctx.clone(),
                Rc::clone(&units),
                changed.clone(),
                Rc::clone(&up),
                Rc::clone(&done),
                Rc::clone(&env),
                Rc::clone(&book),
                Rc::clone(&back),
                Rc::clone(&nudge),
            );
            // With a health check, any job can be interrupted: by a takeout, and by a failure
            // only if failures interrupt (a failure that waits for the job never does). So can
            // any job whose unit's shift end stops it.
            let interruptible =
                breakdown.as_ref().is_some_and(|b| b.interruptible) || checked || stops;
            let waits_for_repair = breakdown.as_ref().is_some_and(|b| !b.hands_on_while_down);
            let pid = ctx.spawn(async move {
                // The setup (if any), then the job; a failure or a takeout interrupts either
                // alike. Retried, the work done so far.
                let mut retried = None;
                'work: for (job, time) in [(false, set_up), (true, Some(duration))] {
                    let Some(mut left) = time else { continue };
                    if job {
                        book2.change(&env2.borrow(), unit, |d| d.setup = false);
                    }
                    loop {
                        if !interruptible {
                            c.hold(left).await;
                            break;
                        }
                        match c.hold_interruptible(left).await {
                            Ok(()) => break,
                            Err(i) => {
                                left = i.remaining;
                                if let Some(x) = units2.borrow_mut().get_mut(unit) {
                                    x.serving = None;
                                }
                                let off = || units2.borrow().get(unit).is_some_and(|x| x.off);
                                let shift_end = i.cause == InterruptCause::Other(SHIFT_END);
                                // The setup, or the setup and part of the job.
                                let setup = set_up.unwrap_or(0.0).max(0.0);
                                let work = if job {
                                    setup + duration.max(0.0) - left
                                } else {
                                    setup - left
                                };
                                if (retries && i.cause == InterruptCause::Other(TAKEN_OUT))
                                    || (shift_retries && shift_end && off())
                                {
                                    retried = Some(work.max(0.0));
                                    break 'work;
                                }
                                // Failed, taken out or off shift mid-job: wait for the repair,
                                // the restart and (resuming) the shift, then carry on; retrying,
                                // the job stops as soon as its unit is off shift.
                                if stops {
                                    let mut paused = false;
                                    loop {
                                        let is_off = off();
                                        if shift_retries && is_off {
                                            retried = Some(work.max(0.0));
                                            break 'work;
                                        }
                                        let wait = shift_resumes && is_off;
                                        if wait != paused {
                                            pause(&env2, &book2, unit, wait);
                                            paused = wait;
                                        }
                                        let closed = [up2.get(unit), back2.get(unit)]
                                            .into_iter()
                                            .flatten()
                                            .any(|g| !g.is_open());
                                        if !wait && !closed {
                                            break;
                                        }
                                        match nudge2.get(unit) {
                                            Some(s) => s.wait().await,
                                            None => break,
                                        }
                                    }
                                } else if checked {
                                    loop {
                                        let closed = [up2.get(unit), back2.get(unit)]
                                            .into_iter()
                                            .flatten()
                                            .find(|g| !g.is_open())
                                            .cloned();
                                        match closed {
                                            Some(g) => g.wait().await,
                                            None => break,
                                        }
                                    }
                                } else if let Some(g) = up2.get(unit) {
                                    g.wait().await;
                                }
                                if let (Some(x), Some(pid)) =
                                    (units2.borrow_mut().get_mut(unit), c.current())
                                {
                                    x.serving = Some(pid);
                                }
                            }
                        }
                    }
                }
                if let Some(x) = units2.borrow_mut().get_mut(unit) {
                    x.serving = None;
                }
                if let Some(work) = retried {
                    let freed = retry(&env2, &book2, unit, entity, work, started, measured);
                    free_unit(&units2, unit, &env2.borrow());
                    // Taken out, the unit now takes its place ([`PlaceWhileOut::Reserved`]):
                    // that of a job given up, if there was one, before anyone waiting for room.
                    let places = env2.borrow().places.clone();
                    reserve_place(&units2, unit, places.as_ref(), out_too, freed);
                    if let Some(d) = done2.get(unit) {
                        d.notify_all();
                    }
                    changed2.notify_all();
                    return;
                }
                {
                    let e = env2.borrow();
                    e.adjust(|s| &mut s.busy, -1.0);
                    book2.change(&e, unit, |d| {
                        d.processing = false;
                        d.holding = true;
                    });
                }
                // Handed on while down (or out, keeping its place): now empty, the unit keeps
                // the entity's place until its repair or restart ends ([`reserve_place`]).
                let down = || {
                    units2
                        .borrow()
                        .get(unit)
                        .is_some_and(|x| keeps_place(x, out_too))
                };
                let repaired = waits_for_repair.then(|| up2.get(unit)).flatten();
                let mut entity = entity;
                entity.last_server = Some(env2.borrow().me);
                env2.borrow().with_stats(|s| s.produced += 1);
                let kept = move_on(&env2, &c, entity, &down, repaired).await;
                book2.change(&env2.borrow(), unit, |d| d.holding = false);
                if let Some(x) = units2.borrow_mut().get_mut(unit)
                    && kept.is_some()
                {
                    x.reserved = kept;
                }
                free_unit(&units2, unit, &env2.borrow());
                if let Some(d) = done2.get(unit) {
                    d.notify_all();
                }
                changed2.notify_all();
            });
            if interruptible && let Some(x) = units.borrow_mut().get_mut(unit) {
                x.serving = Some(pid);
            }
        }
    });
}

/// Unit `u` is free again (its job done or retried): one that went off shift with it is now
/// off for good, no longer counted as on shift.
fn free_unit(units: &RefCell<Vec<Unit>>, u: usize, env: &BlockEnv) {
    let left = units.borrow_mut().get_mut(u).is_some_and(|x| {
        x.busy = false;
        std::mem::take(&mut x.leaving)
    });
    if left {
        staff(units, env);
    }
}

/// Records the units on shift now, with those finishing a job before going off.
fn staff(units: &RefCell<Vec<Unit>>, env: &BlockEnv) {
    let n = units
        .borrow()
        .iter()
        .filter(|x| !x.off || x.leaving)
        .count();
    let now = env.ctx.now();
    env.with_stats(|s| s.staffed.set(now, n as f64));
}

/// The job on unit `u` pauses until the unit is on shift again (`on`), or carries on.
fn pause(env: &Rc<RefCell<BlockEnv>>, book: &UnitBook, u: usize, on: bool) {
    let e = env.borrow();
    book.change(&e, u, |d| d.paused = on);
}

/// The units on shift become `n` (now); returns whether some came on. When fewer are to be on,
/// units go off in this order: idle units that are up and in service, the highest numbered
/// first; then units failed or out, the highest numbered first; then busy ones, those holding a
/// finished entity first, then the job started most recently first (the least work lost), the
/// highest numbered first at a tie. A unit holding a finished entity hands it on first, as one
/// with a job does with [`OnShiftEnd::Finish`]; with `Retry` or `Resume`, its job is stopped (or,
/// already stopped by a failure or a takeout, told). When more are to be on, units finishing a
/// job before going off stay on first; then units come back, those holding a paused job first,
/// then the lowest numbered first.
fn set_units(
    n: u32,
    units: &RefCell<Vec<Unit>>,
    book: &UnitBook,
    env: &BlockEnv,
    rule: OnShiftEnd,
    nudge: &[Signal],
) -> bool {
    let n = n as usize;
    let on = units.borrow().iter().filter(|x| !x.off).count();
    if n < on {
        let order: Vec<usize> = {
            let units = units.borrow();
            let on: Vec<(usize, &Unit)> =
                units.iter().enumerate().filter(|(_, x)| !x.off).collect();
            let idle = on
                .iter()
                .rev()
                .filter(|(_, x)| !x.busy && !x.down && !x.out);
            let away = on.iter().rev().filter(|(_, x)| x.down || x.out);
            let working = on.iter().rev().filter(|(_, x)| x.busy && !x.down && !x.out);
            let holding = working.clone().filter(|(u, _)| book.holding(*u));
            let mut jobs: Vec<&(usize, &Unit)> =
                working.filter(|(u, _)| !book.holding(*u)).collect();
            // Stable: the highest numbered first among those started at once.
            jobs.sort_by(|(_, a), (_, b)| b.started.total_cmp(&a.started));
            idle.chain(away)
                .chain(holding)
                .chain(jobs)
                .map(|(u, _)| *u)
                .take(on.len() - n)
                .collect()
        };
        for u in order {
            let holding = book.holding(u);
            let stop = units.borrow_mut().get_mut(u).and_then(|x| {
                x.off = true;
                if !x.busy {
                    None
                } else if holding || rule == OnShiftEnd::Finish {
                    x.leaving = true;
                    None
                } else {
                    // Its job stops: interrupted, or (stopped already) told.
                    Some(x.serving.take())
                }
            });
            book.change(env, u, |d| d.off = true);
            match stop {
                Some(Some(pid)) => {
                    env.ctx.interrupt(pid, SHIFT_END);
                }
                Some(None) => {
                    if let Some(s) = nudge.get(u) {
                        s.notify_all();
                    }
                }
                None => {}
            }
        }
    } else if n > on {
        let order: Vec<usize> = {
            let units = units.borrow();
            let off = || units.iter().enumerate().filter(|(_, x)| x.off);
            let leaving = off().filter(|(_, x)| x.leaving);
            let paused = off().filter(|(_, x)| x.busy && !x.leaving);
            let rest = off().filter(|(_, x)| !x.busy);
            leaving
                .chain(paused)
                .chain(rest)
                .map(|(u, _)| u)
                .take(n - on)
                .collect()
        };
        for u in order {
            let paused = units.borrow_mut().get_mut(u).is_some_and(|x| {
                x.off = false;
                x.busy && !std::mem::take(&mut x.leaving)
            });
            book.change(env, u, |d| d.off = false);
            if paused && let Some(s) = nudge.get(u) {
                s.notify_all();
            }
        }
    }
    staff(units, env);
    n > on
}

/// Whether a health check on unit `u` (now) takes it out: it is up, not out already and on
/// shift, more entities wait than the limit per working unit (up, not out and on shift), and
/// taking it out leaves at least the units to keep working. A unit finishing its job after its
/// shift has ended is off shift here: it takes no more entities.
fn take_out(
    units: &RefCell<Vec<Unit>>,
    u: usize,
    h: &HealthCheck,
    env: &Rc<RefCell<BlockEnv>>,
) -> bool {
    let units = units.borrow();
    if units.get(u).is_none_or(|x| x.down || x.out || x.off) {
        return false;
    }
    let working = units.iter().filter(|x| !x.down && !x.out && !x.off).count();
    // Waiting in the queue (the dispatcher holds none with a health check).
    let waiting = {
        let e = env.borrow();
        e.stats
            .get(e.me)
            .map_or(0.0, |s| s.borrow().waiting.value())
    };
    waiting > h.limit * working as f64 && working > h.keep.map_or(0, |k| k as usize)
}

/// A job on unit `unit` stops as a health check takes the unit out, or as the unit goes off
/// shift ([`OnShiftEnd::Retry`]): the `work` done on it is
/// wasted, and the entity goes back to the head of the queue at once, for any unit to start
/// afresh (with a fresh patience). One that had given up during the job already (patience
/// until done) is discarded instead, neither left nor produced; its place here, if it has one,
/// is returned for the unit to keep or give back ([`reserve_place`]). The wait recorded as it
/// started
/// (`started`, into the statistics as they were after `resets` resets) is taken back.
fn retry(
    env: &Rc<RefCell<BlockEnv>>,
    book: &UnitBook,
    unit: usize,
    entity: Entity,
    work: f64,
    started: f64,
    resets: u64,
) -> Option<ResourceGuard> {
    let e = env.borrow();
    e.with_stats(|s| {
        s.wasted += work;
        if s.resets == resets {
            s.unwait.add(started - entity.entered);
        }
    });
    e.adjust(|s| &mut s.busy, -1.0);
    book.change(&e, unit, |d| {
        d.processing = false;
        d.setup = false;
    });
    if e.patient.as_ref().is_some_and(|p| !p.retry(entity.id)) {
        // It gave up during the job: it goes, counted as abandoned already.
        let now = e.ctx.now();
        e.with_stats(|s| s.stay_ends(now));
        return e
            .places
            .as_ref()
            .and_then(|p| p.leave_or_keep(entity.id, true));
    }
    e.adjust(|s| &mut s.waiting, 1.0);
    e.input.put_front(entity);
    None
}

/// An entity whose service or delay is over leaves for the next element. Into a full finite
/// buffer it waits here first, blocking (blocking after service): the element keeps it, and its
/// place here, until there is room, and the time is recorded. As the entity's place here comes
/// free, `keep` says whether the element keeps it (a broken unit, [`reserve_place`]); then it is
/// returned instead of given back. With `repaired`, the unit holding the entity hands it on
/// only while that gate is open (it is up): a unit that is down first waits for its repair, and
/// one that breaks down while waiting for room waits for its repair once there is room,
/// keeping the place it has.
async fn move_on(
    env: &Rc<RefCell<BlockEnv>>,
    ctx: &Ctx,
    mut entity: Entity,
    keep: &dyn Fn() -> bool,
    repaired: Option<&Gate>,
) -> Option<ResourceGuard> {
    // With patience until done, an entity that gave up while its job was under way (or while it
    // was held after it) is discarded rather than handed on.
    let patient = env.borrow().patient.clone();
    let gave_up = |id: u64| patient.as_ref().and_then(|p| p.done(id));
    if let Some(work) = gave_up(entity.id) {
        return discard(env, entity.id, work, keep);
    }
    if let Some(g) = repaired {
        g.wait().await;
        if let Some(work) = gave_up(entity.id) {
            return discard(env, entity.id, work, keep);
        }
    }
    let out = env.borrow().out.clone();
    // The next element, if the entity does not get in there (it balks, or finds no room).
    let mut turned = None;
    // Where the entity goes: the next element, or the way a shortest-queue gateway there gives.
    let out = match out {
        Some(out) if out.may_block() => {
            let start = ctx.now();
            env.borrow().adjust(|s| &mut s.blocked, 1.0);
            let to = out.make_room(&mut entity).await;
            if let Some(g) = repaired {
                g.wait().await;
            }
            let now = ctx.now();
            env.borrow().adjust(|s| &mut s.blocked, -1.0);
            // It gave up while held: the place it got next goes back unused. (Through a
            // shortest-queue gateway, the gateway has counted it passing.)
            if let Some(work) = gave_up(entity.id) {
                if let Some(p) = to.as_ref().ok().and_then(|to| to.places.as_ref()) {
                    p.leave(entity.id);
                }
                return discard(env, entity.id, work, keep);
            }
            env.borrow().with_stats(|s| s.blocked_time.add(now - start));
            match to {
                Ok(to) => Some(to),
                Err(why) => {
                    turned = Some((out, why));
                    None
                }
            }
        }
        Some(out) => {
            // A buffer that turns away those finding no room can still take a place freed at
            // this instant, waiting for it within the instant: if the entity gave up meanwhile,
            // it is discarded as when held, above.
            let to = out.make_room(&mut entity).await;
            if let Some(work) = gave_up(entity.id) {
                if let Some(p) = to.as_ref().ok().and_then(|to| to.places.as_ref()) {
                    p.leave(entity.id);
                }
                return discard(env, entity.id, work, keep);
            }
            match to {
                Ok(to) => Some(to),
                Err(why) => {
                    turned = Some((out, why));
                    None
                }
            }
        }
        None => None,
    };
    if let Some(p) = &patient {
        p.handed_on(entity.id);
    }
    let transfer = out.as_ref().and_then(Port::transfer_time);
    let holds = env.borrow().holds && out.as_ref().is_some_and(Port::holds_sender);
    match (out, transfer) {
        // Across a transfer that holds the sender, a unit stays with the entity, blocked, until
        // it gets there; the entity is the next element's from the start, and keeps its place
        // here (which is the unit's) until the unit is free.
        (Some(out), Some(dt)) if holds => {
            let now = ctx.now();
            env.borrow().with_stats(|s| {
                s.left += 1;
                s.stay_ends(now);
            });
            let id = entity.id;
            let entity = out.depart(ctx.now(), entity);
            env.borrow().adjust(|s| &mut s.transferring, 1.0);
            ctx.hold(dt).await;
            let e = env.borrow();
            e.adjust(|s| &mut s.transferring, -1.0);
            let kept = e.places.as_ref().and_then(|p| p.leave_or_keep(id, keep()));
            drop(e);
            out.arrive(ctx.now(), entity);
            kept
        }
        (out, transfer) => {
            let kept = {
                let e = env.borrow();
                let now = ctx.now();
                e.with_stats(|s| {
                    s.left += 1;
                    s.stay_ends(now);
                });
                e.places
                    .as_ref()
                    .and_then(|p| p.leave_or_keep(entity.id, keep()))
            };
            if let Some(out) = out {
                out.travel(ctx, entity, transfer);
            } else if let Some((t, why)) = turned {
                // Turned away or balking there, it has left here all the same.
                t.refuse(entity, why);
            }
            kept
        }
    }
}

/// A server's job whose entity gave up before it was done (patience until done) ends: the
/// entity is discarded, neither left nor produced, and its `work` was wasted. Its place here
/// comes free as on leaving (or stays the unit's, as `keep` says; see [`move_on`]).
fn discard(
    env: &Rc<RefCell<BlockEnv>>,
    id: u64,
    work: f64,
    keep: &dyn Fn() -> bool,
) -> Option<ResourceGuard> {
    let e = env.borrow();
    let now = e.ctx.now();
    e.with_stats(|s| {
        s.wasted += work;
        // Counted as produced when its job ended.
        s.produced = s.produced.saturating_sub(1);
        s.stay_ends(now);
    });
    e.places.as_ref().and_then(|p| p.leave_or_keep(id, keep()))
}

/// One way to meet a shared task's need: `quantity` units from any of `pools` (indices into the
/// model's resources), taken in that order.
struct Alternative {
    pools: Vec<usize>,
    quantity: u32,
}

/// What a shared task asks of the pools for each entity.
struct Request {
    /// Every one is met together, all at once; each by one of its alternatives, tried in order.
    requirements: Vec<Vec<Alternative>>,
    priority: f64,
}

/// The pools each alternative of each of `needs` can draw on.
fn requirements(model: &Model, needs: &[Requirement]) -> Vec<Vec<Alternative>> {
    needs
        .iter()
        .map(|r| {
            r.alternatives
                .iter()
                .map(|need| Alternative {
                    pools: model
                        .resources
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| need.met_by(r))
                        .map(|(i, _)| i)
                        .collect(),
                    quantity: need.quantity(),
                })
                .collect()
        })
        .collect()
}

/// Takes units from `free` for each of `requirements`, in order, into `taken`: for the first,
/// its first alternative the free units can meet that leaves a way to meet all the later ones
/// (with what it leaves), and so on. So the first way in that order is taken, and an earlier
/// requirement takes its next alternative only when its first would leave a later one unmet.
/// `false`, with `free` and `taken` as they were, when there is no way to meet them all now.
fn take_all(requirements: &[Vec<Alternative>], free: &mut [i64], taken: &mut Taken) -> bool {
    take_within(MAX_COMBINATIONS, requirements, free, taken)
}

/// [`take_all`], trying at most `limit` ways to choose the later requirements' alternatives one
/// by one ([`any_way`]).
fn take_within(
    limit: usize,
    requirements: &[Vec<Alternative>],
    free: &mut [i64],
    taken: &mut Taken,
) -> bool {
    let Some((first, rest)) = requirements.split_first() else {
        return true;
    };
    // An alternative is tried only when it leaves a way to meet the later requirements, so
    // a take that cannot be met is known at once ([`feasible`]), not after trying every
    // spread of units, which for many pools of one unit each would be millions.
    first.iter().any(|alt| {
        alt.quantity > 0
            && feasible(
                limit,
                Some((alt.pools.as_slice(), alt.quantity)),
                rest,
                free,
            )
            && fill(limit, &alt.pools, alt.quantity, rest, free, taken)
    })
}

/// Most ways to choose one alternative for each requirement that [`any_way`] tries one by one.
const MAX_COMBINATIONS: usize = 4096;

/// What a requirement takes: the pools it takes from and the units it needs from them.
type Share<'a> = (&'a [usize], u32);

/// Whether `f` holds for some way to meet every one of `rest`, each by one of its alternatives
/// (of more than no units): the needs of each way to choose them, the first requirement's turning
/// fastest, while there are at most `limit` ways. With more, `f` is asked once, of a weaker way
/// that any choice meets: each requirement taking the fewest units any of its alternatives needs
/// from any pool any of them names. So a `false` still means that no choice will do. `false` when
/// a requirement has no alternative.
fn any_way(limit: usize, rest: &[Vec<Alternative>], mut f: impl FnMut(&[Share]) -> bool) -> bool {
    let choices: Vec<Vec<&Alternative>> = rest
        .iter()
        .map(|r| r.iter().filter(|alt| alt.quantity > 0).collect())
        .collect();
    if choices.iter().any(Vec::is_empty) {
        return false;
    }
    let combinations = choices.iter().try_fold(1_usize, |n, c| {
        n.checked_mul(c.len()).filter(|&n| n <= limit)
    });
    let Some(combinations) = combinations else {
        let relaxed: Vec<(Vec<usize>, u32)> = choices
            .iter()
            .map(|c| {
                let mut pools: Vec<usize> =
                    c.iter().flat_map(|alt| alt.pools.iter().copied()).collect();
                pools.sort_unstable();
                pools.dedup();
                let fewest = c.iter().map(|alt| alt.quantity).min().unwrap_or(0);
                (pools, fewest)
            })
            .collect();
        let way: Vec<Share> = relaxed.iter().map(|(p, q)| (p.as_slice(), *q)).collect();
        return f(&way);
    };
    // Counts through the choices like an odometer.
    let mut pick = vec![0_usize; choices.len()];
    for _ in 0..combinations {
        let way: Vec<Share> = choices
            .iter()
            .zip(&pick)
            .filter_map(|(c, &i)| c.get(i).map(|alt| (alt.pools.as_slice(), alt.quantity)))
            .collect();
        if f(&way) {
            return true;
        }
        for (i, c) in pick.iter_mut().zip(&choices) {
            *i += 1;
            if *i < c.len() {
                break;
            }
            *i = 0;
        }
    }
    false
}

/// Whether `current` (the pools a requirement still takes from, and the units it still needs)
/// and every one of `rest` can be met together from `free`: for some way to choose the
/// alternatives of `rest` ([`any_way`]), [`shared_out`] says the free units can be shared out
/// among them all. [`take_all`] and [`fill`] ask this before each step, so they only step where
/// a way through may lie ahead: the first way they find is still the first in their order, found
/// without trying the dead ends. With more than `limit` ways to choose, the check is the weaker
/// one, which still never says no to a take that could be met.
fn feasible(limit: usize, current: Option<Share>, rest: &[Vec<Alternative>], free: &[i64]) -> bool {
    // With nothing after it, the current one only needs enough free in its own pools: the
    // common case of a task with one requirement, checked without building a flow.
    if rest.is_empty() {
        return current.is_none_or(|(pools, left)| have(pools, free) >= i64::from(left));
    }
    any_way(limit, rest, |way| {
        let needs: Vec<Share> = current.into_iter().chain(way.iter().copied()).collect();
        shared_out(&needs, free)
    })
}

/// Units free in `pools` all told.
fn have(pools: &[usize], free: &[i64]) -> i64 {
    pools
        .iter()
        .map(|&p| free.get(p).copied().unwrap_or(0).max(0))
        .sum()
}

/// Whether the `free` units can be shared out to meet every one of `needs` (its pools, and the
/// units it needs from them) at once. A maximum flow: from a source to each need as many units
/// as it needs, on from a need to any of its pools, and from each pool to a sink as many as it
/// has free. They can all be met when the flow carries every unit needed, and the flow is found
/// by shortest augmenting paths, a handful of steps for the few needs and pools of a task.
fn shared_out(needs: &[(&[usize], u32)], free: &[i64]) -> bool {
    let demand: i64 = needs.iter().map(|&(_, q)| i64::from(q)).sum();
    if demand == 0 {
        return true;
    }
    let mut pools: Vec<usize> = needs.iter().flat_map(|&(p, _)| p.iter().copied()).collect();
    pools.sort_unstable();
    pools.dedup();
    // Nodes: the source 0, then the needs, then the pools, then the sink.
    let pool_node = |p: usize| 1 + needs.len() + pools.binary_search(&p).unwrap_or(0);
    let sink = 1 + needs.len() + pools.len();
    let mut capacity = vec![vec![0_i64; sink + 1]; sink + 1];
    for (i, &(ps, q)) in needs.iter().enumerate() {
        capacity[0][1 + i] = i64::from(q);
        for &p in ps {
            capacity[1 + i][pool_node(p)] = demand;
        }
    }
    for &p in &pools {
        capacity[pool_node(p)][sink] = free.get(p).copied().unwrap_or(0).max(0);
    }
    let mut flow = 0;
    while flow < demand {
        // The shortest path from the source to the sink with room left on every step.
        let mut from = vec![usize::MAX; sink + 1];
        from[0] = 0;
        let mut queue = VecDeque::from([0]);
        while let Some(u) = queue.pop_front() {
            for (v, &room) in capacity[u].iter().enumerate() {
                if room > 0 && from[v] == usize::MAX {
                    from[v] = u;
                    queue.push_back(v);
                }
            }
        }
        if from[sink] == usize::MAX {
            return false;
        }
        let mut push = i64::MAX;
        let mut v = sink;
        while v != 0 {
            push = push.min(capacity[from[v]][v]);
            v = from[v];
        }
        let mut v = sink;
        while v != 0 {
            capacity[from[v]][v] -= push;
            capacity[v][from[v]] += push;
            v = from[v];
        }
        flow += push;
    }
    true
}

/// Takes `left` units from `pools` (an alternative's pools, in model order), then meets `rest`
/// ([`take_all`]). The units are spread over the pools as many from the first as it has free,
/// then the next, and so on; only when that leaves a later requirement unmet are other spreads
/// tried, fewer from an earlier pool first. So a take that succeeded before is unchanged, and
/// one pool of a role is left for a later requirement that names it. Only amounts from which
/// the units left can still meet the rest ([`largest`]) are tried, the largest first, so no dead
/// end is walked into, and a range of amounts that cannot work is passed over at once, not one
/// unit at a time. `false`, with `free` and `taken` as they were, when no spread meets them all.
fn fill(
    limit: usize,
    pools: &[usize],
    left: u32,
    rest: &[Vec<Alternative>],
    free: &mut [i64],
    taken: &mut Taken,
) -> bool {
    if left == 0 {
        return take_within(limit, rest, free, taken);
    }
    let Some((&p, others)) = pools.split_first() else {
        return false;
    };
    let mut most = free.get(p).copied().unwrap_or(0).clamp(0, i64::from(left)) as u32;
    while let Some(n) = largest(limit, (p, others), left, most, rest, free) {
        let mark = taken.len();
        if n > 0
            && let Some(f) = free.get_mut(p)
        {
            *f -= i64::from(n);
            taken.push((p, n));
        }
        if fill(limit, others, left - n, rest, free, taken) {
            return true;
        }
        if n > 0
            && let Some(f) = free.get_mut(p)
        {
            *f += i64::from(n);
        }
        taken.truncate(mark);
        // Only with the weaker check ([`any_way`]) can a step it lets through fail.
        let Some(fewer) = n.checked_sub(1) else {
            return false;
        };
        most = fewer;
    }
    false
}

/// The largest number of units, at most `most`, that a requirement still needing `left` units
/// from pool `p` and then `others` can take from `p` with the units left still able to meet the
/// rest of it from `others` and every one of `rest` ([`feasible`], with the same ways to choose).
/// `None` when no number can.
///
/// For one way to choose the alternatives of `rest`, the numbers that can be taken from `p` are
/// those from some least to some largest, with none missing between: they are the flows on one
/// edge of a network carrying every unit needed, and those flows make a convex set whose ends
/// are whole numbers. So the largest at most `most` is found by halving, a few flows for a
/// billion units: `at_least(m)` (taking `m` from `p` and the rest of `left` from any of the
/// pools) holds for every `m` up to the largest number and none above. Over the ways to choose,
/// the largest of their largest numbers is the one wanted.
fn largest(
    limit: usize,
    (p, others): (usize, &[usize]),
    left: u32,
    most: u32,
    rest: &[Vec<Alternative>],
    free: &[i64],
) -> Option<u32> {
    if rest.is_empty() {
        // Fewer from `p` must be made up by the others.
        return (i64::from(left - most) <= have(others, free)).then_some(most);
    }
    let pools: Vec<usize> = std::iter::once(p).chain(others.iter().copied()).collect();
    let mut best: Option<u32> = None;
    any_way(limit, rest, |way| {
        // With `m` taken from `p`, whether `left - m` from `from` and all of `way` can be met.
        let meets = |m: u32, from: &[usize]| {
            let mut after = free.to_vec();
            if let Some(f) = after.get_mut(p) {
                *f -= i64::from(m);
            }
            let needs: Vec<Share> = std::iter::once((from, left - m))
                .chain(way.iter().copied())
                .collect();
            shared_out(&needs, &after)
        };
        let exactly = |n: u32| meets(n, others);
        let at_least = |m: u32| meets(m, &pools);
        // Only a number above the best found so far is worth finding.
        let least = best.map_or(0, |b| b + 1);
        if least > most {
            return true;
        }
        if exactly(most) {
            best = Some(most);
            return true;
        }
        // Either every number that works here is above `most`, or none reaches `least`.
        if at_least(most) || !at_least(least) {
            return false;
        }
        let (mut lo, mut hi) = (least, most - 1);
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            if at_least(mid) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        best = Some(lo);
        false
    });
    best
}

/// Units taken from the pools: (pool, how many).
type Taken = Vec<(usize, u32)>;

/// An entity waiting for units.
struct Waiting {
    request: Rc<Request>,
    /// Arrival order, for ties of priority.
    seq: u64,
    /// Where the units are handed over.
    grant: Store<Taken>,
}

/// The shared resource pools of one run: free units, the entities waiting for some, and each
/// pool's busy units over time.
struct Pools {
    ctx: Ctx,
    /// Units free now; below 0 while a pool's shift has ended with more units still busy
    /// finishing their jobs than it now has.
    free: RefCell<Vec<i64>>,
    /// Units each pool has now (its shift's quantity while available).
    units: RefCell<Vec<u32>>,
    busy: RefCell<Vec<TimeWeighted>>,
    /// Units each pool has, over time, for utilisation: its shift's, or more while units whose
    /// shift has ended are finishing their jobs.
    capacity: RefCell<Vec<TimeWeighted>>,
    /// How many times each pool's units were taken.
    taken: RefCell<Vec<u64>>,
    /// Highest priority first, then by arrival.
    waiting: RefCell<Vec<Waiting>>,
    seq: RefCell<u64>,
    /// Whether a hand-out of free units is already scheduled for the end of this instant.
    dispatch_pending: Cell<bool>,
}

impl Pools {
    fn new(ctx: &Ctx, units: Vec<u32>) -> Self {
        Self {
            ctx: ctx.clone(),
            free: RefCell::new(units.iter().map(|&u| i64::from(u)).collect()),
            busy: RefCell::new(units.iter().map(|_| TimeWeighted::new(0.0, 0.0)).collect()),
            capacity: RefCell::new(
                units
                    .iter()
                    .map(|&u| TimeWeighted::new(0.0, f64::from(u)))
                    .collect(),
            ),
            taken: RefCell::new(vec![0; units.len()]),
            units: RefCell::new(units),
            waiting: RefCell::new(Vec::new()),
            seq: RefCell::new(0),
            dispatch_pending: Cell::new(false),
        }
    }

    /// Sets pool `p`'s units (a shift change). Units taken away that are busy finish their job
    /// first; units added go to waiting entities at the end of this instant.
    fn set_units(self: &Rc<Self>, p: usize, n: u32) {
        let now = self.ctx.now();
        let more = {
            let mut units = self.units.borrow_mut();
            let mut free = self.free.borrow_mut();
            let (Some(u), Some(f)) = (units.get_mut(p), free.get_mut(p)) else {
                return;
            };
            *f += i64::from(n) - i64::from(*u);
            let more = n > *u;
            *u = n;
            let present = (i64::from(n) - (*f).min(0)) as f64;
            if let Some(c) = self.capacity.borrow_mut().get_mut(p) {
                c.set(now, present);
            }
            more
        };
        if more {
            self.dispatch_later();
        }
    }

    /// Takes the units for every one of `request`'s requirements together, the first way the
    /// free units can meet them all ([`take_all`]), or none.
    fn try_take(&self, request: &Request) -> Option<Taken> {
        let mut free = self.free.borrow_mut();
        let mut taken = Vec::new();
        if request.requirements.is_empty()
            || !take_all(&request.requirements, &mut free[..], &mut taken)
        {
            return None;
        }
        drop(free);
        self.record(&taken, true);
        Some(taken)
    }

    /// Updates the busy units of the pools in `taken`, as they are taken or given back.
    fn record(&self, taken: &Taken, take: bool) {
        let now = self.ctx.now();
        let free = self.free.borrow();
        let units = self.units.borrow();
        let mut busy = self.busy.borrow_mut();
        let mut count = self.taken.borrow_mut();
        for (k, &(p, _)) in taken.iter().enumerate() {
            // A pool two requirements both drew on was taken once.
            let again = taken[..k].iter().any(|&(q, _)| q == p);
            if let (Some(b), Some(&q), Some(&f)) = (busy.get_mut(p), units.get(p), free.get(p)) {
                b.set(now, (i64::from(q) - f) as f64);
                // Units still finishing a job after their shift are there until it ends.
                let mut capacity = self.capacity.borrow_mut();
                if let Some(c) = capacity.get_mut(p)
                    && (f < 0 || c.value() > f64::from(q))
                {
                    c.set(now, (i64::from(q) - f.min(0)) as f64);
                }
            }
            if take
                && !again
                && let Some(c) = count.get_mut(p)
            {
                *c += 1;
            }
        }
    }

    /// Waits until units for `request` are free, then takes them. An entity asking joins the
    /// queue at once, and takes units when the instant's hand-out runs ([`Pools::dispatch_later`]).
    async fn acquire(self: &Rc<Self>, request: &Rc<Request>) -> Taken {
        let grant = Store::new(&self.ctx);
        let seq = {
            let mut s = self.seq.borrow_mut();
            *s += 1;
            *s
        };
        {
            let mut w = self.waiting.borrow_mut();
            // After every waiter of the same or higher priority.
            let at = w
                .iter()
                .position(|x| x.request.priority < request.priority)
                .unwrap_or(w.len());
            w.insert(
                at,
                Waiting {
                    request: Rc::clone(request),
                    seq,
                    grant: grant.clone(),
                },
            );
        }
        self.dispatch_later();
        grant.get().await
    }

    /// Gives back `taken`, then hands free units to waiting entities at the end of this
    /// instant ([`Pools::dispatch_later`]).
    fn release(self: &Rc<Self>, taken: Taken) {
        {
            let mut free = self.free.borrow_mut();
            for &(p, n) in &taken {
                if let Some(f) = free.get_mut(p) {
                    *f += i64::from(n);
                }
            }
        }
        self.record(&taken, false);
        self.dispatch_later();
    }

    /// Schedules the hand-out of free units for the end of this instant, behind every other
    /// event at it ([`Ctx::hold_late`]). Units given back, entities that arrive and units that
    /// come on shift at one instant all count before anyone is served, so what happens does not
    /// depend on the order those events were scheduled in: the waiting entities, in priority
    /// order and then of arrival, take the free units, each trying its alternatives in the order
    /// of its task's needs (the first the free units can meet, for every requirement together:
    /// [`take_all`]). So with two units of different
    /// pools freed together and one entity waiting, it takes the unit of the pool its task lists
    /// first, not the one whose job happened to end first.
    fn dispatch_later(self: &Rc<Self>) {
        if self.dispatch_pending.replace(true) {
            return;
        }
        let (me, ctx) = (Rc::clone(self), self.ctx.clone());
        self.ctx.spawn(async move {
            ctx.hold_late(0.0).await;
            me.dispatch_pending.set(false);
            me.dispatch();
        });
    }

    /// Hands free units to waiting entities: in priority order, each that can be served now
    /// (one that cannot does not hold back the others).
    fn dispatch(&self) {
        let mut i = 0;
        loop {
            let next = {
                let w = self.waiting.borrow();
                match w.get(i) {
                    Some(x) => Rc::clone(&x.request),
                    None => break,
                }
            };
            match self.try_take(&next) {
                Some(t) => {
                    let x = self.waiting.borrow_mut().remove(i);
                    let _ = x.seq;
                    x.grant.put(t);
                }
                None => i += 1,
            }
        }
    }

    /// Discards the statistics so far (the end of the warm-up).
    fn reset(&self, t: f64) {
        self.busy.borrow_mut().iter_mut().for_each(|b| b.reset(t));
        self.capacity
            .borrow_mut()
            .iter_mut()
            .for_each(|b| b.reset(t));
        self.taken.borrow_mut().iter_mut().for_each(|c| *c = 0);
    }

    /// Each pool's report: how often its units were taken, mean units busy and utilisation.
    /// Each pool's state now.
    fn states(&self, resources: &[ResourcePool]) -> Vec<LiveState> {
        let busy = self.busy.borrow();
        let units = self.units.borrow();
        let taken = self.taken.borrow();
        resources
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let n = taken.get(i).copied().unwrap_or(0);
                LiveState {
                    id: r.id.clone(),
                    entered: n,
                    left: n,
                    lost: 0,
                    waiting: 0.0,
                    busy: busy.get(i).map_or(0.0, TimeWeighted::value),
                    units: units.get(i).map(|u| f64::from(*u)),
                    down: 0.0,
                }
            })
            .collect()
    }

    fn reports(&self, resources: &[ResourcePool], end: f64) -> Vec<ElementReport> {
        let busy = self.busy.borrow();
        let capacity = self.capacity.borrow();
        let taken = self.taken.borrow();
        resources
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mean = busy.get(i).and_then(|b| b.average(end));
                // Busy units over the units the pool had, both averaged over time.
                let constant = r.shifts.is_empty() && r.availability.is_empty();
                let units = if constant {
                    Some(f64::from(if r.available { r.quantity } else { 0 }))
                } else {
                    capacity.get(i).and_then(|c| c.average(end))
                };
                let n = taken.get(i).copied().unwrap_or(0);
                ElementReport {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    kind: "resource",
                    entered: n,
                    left: n,
                    mean_queue_length: None,
                    mean_wait: None,
                    utilisation: match (mean, units) {
                        (Some(b), Some(u)) if u > 0.0 => Some(b / u),
                        (Some(_), _) => Some(0.0),
                        _ => None,
                    },
                    mean_contents: mean,
                    work_in_progress: None,
                    mean_work_in_progress: None,
                    mean_time_in_system: None,
                    time_in_system_p50: None,
                    time_in_system_p90: None,
                    time_in_system_p95: None,
                    within_target_share: None,
                    mean_blocked_time: None,
                    mean_time_on: None,
                    time_on_histogram: None,
                    blocking_probability: None,
                    availability: None,
                    time_off_shift: None,
                    time_processing: None,
                    time_starved: None,
                    time_blocked: None,
                    time_failed: None,
                    time_setup: None,
                    oee_availability: None,
                    oee_effectiveness: None,
                    setup_ratio: None,
                    quality_ratio: None,
                    scrap_ratio: None,
                    rework_ratio: None,
                    oee: None,
                    takt: None,
                    demand: None,
                    demand_met: None,
                    cycle_time: None,
                    energy: None,
                    specific_energy: None,
                    lost: None,
                    loss_probability: None,
                    abandoned: None,
                    abandoned_share: None,
                    abandoned_wait: None,
                    balked: None,
                    balked_share: None,
                    wasted: None,
                    wasted_share: None,
                    taken_out: None,
                    mean_in_service: None,
                    time_restarting: None,
                    properties: Vec::new(),
                    inside_at_start: 0,
                    inside_at_end: 0,
                    given_up_inside: [0, 0],
                    leaving: [0, 0],
                    time_inside: None,
                    on_shift: None,
                    started: None,
                }
            })
            .collect()
    }
}

/// A task whose units come from shared pools: each entity waits in this task's queue for the
/// units its request needs, is served, and gives them back once it has left (holding them while
/// blocked after service).
fn spawn_shared(
    sim: &mut Sim,
    env: BlockEnv,
    service: Rc<Times>,
    request: Rc<Request>,
    pools: Rc<Pools>,
) {
    let ctx = env.ctx.clone();
    let me = tag(env.me);
    let env = Rc::new(RefCell::new(env));
    // Each entity's service is a process of its own, started from this one, so tagged alike.
    sim.spawn_tagged(me, async move {
        loop {
            let input = env.borrow().input.clone();
            let entity = input.get().await;
            let (c, env2, req, pl) = (
                ctx.clone(),
                Rc::clone(&env),
                Rc::clone(&request),
                Rc::clone(&pools),
            );
            let service = Rc::clone(&service);
            ctx.spawn(async move {
                let taken = pl.acquire(&req).await;
                let duration = {
                    let mut e = env2.borrow_mut();
                    let now = c.now();
                    e.with_stats(|s| s.wait.add(now - entity.entered));
                    e.adjust(|s| &mut s.waiting, -1.0);
                    e.adjust(|s| &mut s.busy, 1.0);
                    // The pool that supplied the first unit of the take, in the order it was
                    // filled (so the first requirement's), sets the time
                    // ([`stochcraft_model::ProcessingByResource`]).
                    service.sample(&entity, taken.first().map(|t| t.0), &mut e.rng)
                };
                c.hold(duration).await;
                env2.borrow().adjust(|s| &mut s.busy, -1.0);
                move_on(&env2, &c, entity, &|| false, None).await;
                pl.release(taken);
            });
        }
    });
}

/// A conveyor's own clock: the time its belt has run. An accumulating conveyor's never stops;
/// a belt that is not accumulating stops while its front item waits to leave.
struct BeltClock {
    /// The time it has stood still, before any stop now.
    stopped: Cell<f64>,
    /// When it stopped, while it stands.
    since: Cell<Option<f64>>,
    /// Woken when it runs again.
    resumed: Signal,
}

impl BeltClock {
    fn time(&self, now: f64) -> f64 {
        self.since.get().unwrap_or(now) - self.stopped.get()
    }

    fn stop(&self, now: f64) {
        self.since.set(Some(now));
    }

    fn start(&self, now: f64) {
        if let Some(s) = self.since.take() {
            self.stopped.set(self.stopped.get() + (now - s));
        }
        self.resumed.notify_all();
    }

    /// Waits until the belt has run to `target`.
    async fn until(&self, ctx: &Ctx, target: f64) {
        loop {
            if self.since.get().is_some() {
                self.resumed.wait().await;
                continue;
            }
            let left = target - self.time(ctx.now());
            // Rounding must not leave a wait too short to move the clock.
            if left <= 1e-9 * target.abs().max(1.0) {
                return;
            }
            ctx.hold(left).await;
        }
    }
}

/// A divert point on a conveyor (ROADMAP.md, M5.5, L5; [`stochcraft_model::site::points`]): a
/// divert gateway met at `s` along the belt (in the run's length).
struct BeltPoint {
    s: f64,
    /// The gateway (its statistics and its properties).
    gateway: usize,
    router: Rc<Router>,
    /// Entities meeting it are rework: the flow into it (from the conveyor, or the point before's
    /// default) is marked so; counted against the last server each finished at, as in a gateway.
    rework: bool,
}

impl BeltPoint {
    /// `e`'s front has reached the point: the divert's rule chooses a way for it, drawn as
    /// M5.1's divert draws. A way other than the default with room now takes it and is
    /// returned (the caller takes the place there); otherwise (the default chosen, or the way
    /// chosen full) it rides on, and this returns `None`. The gateway counts it in and out
    /// either way.
    fn offer(
        &self,
        e: &mut Entity,
        stats: &Rc<Vec<RefCell<BlockStats>>>,
        assigner: &Assigner,
    ) -> Option<Port> {
        if self.rework
            && let Some(s) = e.last_server.and_then(|i| stats.get(i))
        {
            s.borrow_mut().rework += 1;
        }
        assigner.enter(self.gateway, e, stats);
        if let Some(s) = stats.get(self.gateway) {
            s.borrow_mut().entered += 1;
        }
        let r = &self.router;
        let default = match &r.rule {
            Rule::Divert(d) => d.default(),
            Rule::ShortestQueue => None,
        };
        let way = r
            .first(e)
            .filter(|i| Some(*i) != default)
            .and_then(|i| r.ways.get(i))
            .filter(|w| w.places.as_ref().is_none_or(|p| p.has_room()))
            .cloned();
        if let Some(s) = stats.get(self.gateway) {
            s.borrow_mut().left += 1;
        }
        way
    }
}

/// An item on a conveyor, as its processes pass it on: the entity, when it got on, the belt's
/// own time then, and the ride's number on this belt (an entity that comes round again is
/// another ride).
type Ride = (Entity, f64, f64, u64);

/// A conveyor (ROADMAP M5.1): items ride it in order at `speed`, each a `pitch` behind the one
/// before, and leave its end into what follows. The entrance ([`Belt`]) frees again once the
/// last item has moved a pitch along. All times are worked out, not stepped: accumulating, item
/// i reaches the end at max(tᵢ + L/v, dᵢ₋₁ + p/v), tᵢ when it got on and dᵢ₋₁ when the one
/// before left; not accumulating, the same in the belt's own time, which stands still while the
/// front item waits to leave.
///
/// With divert points (L5), an item's front reaching a point is an event: the divert there
/// offers it a way, and it either leaves the belt there or rides on. Items keep their order,
/// so each point passes them one at a time, as the end does. Not accumulating, an item reaches
/// `s` at its belt time tᵢ + s/v. Accumulating, it reaches `s` when its own travel and the
/// items ahead let it ([`BeltView::reach`]); an item that left at a point goes on constraining
/// the one behind as if it moved on at the speed, as the last to leave the end does, so no
/// position jumps. A conveyor without points runs exactly as M5.1's.
#[allow(clippy::too_many_arguments)]
fn spawn_conveyor(
    sim: &mut Sim,
    env: BlockEnv,
    length: f64,
    speed: f64,
    accumulating: bool,
    pitch: f64,
    product: Option<Rc<Product>>,
    points: Vec<BeltPoint>,
) -> Rc<BeltView> {
    let me = tag(env.me);
    let ctx = env.ctx.clone();
    let places = env.places.clone();
    let clock = Rc::new(BeltClock {
        stopped: Cell::new(0.0),
        since: Cell::new(None),
        resumed: Signal::new(&ctx),
    });
    let view = Rc::new(BeltView {
        clock: Rc::clone(&clock),
        length,
        speed,
        pitch,
        accumulating,
        items: RefCell::new(VecDeque::new()),
        last_left: Cell::new(None),
        rides: Cell::new(0),
        changed: Signal::new(&ctx),
    });
    let has_points = !points.is_empty();
    // The items on the belt, in order, waiting to reach each point in turn and then the end.
    let stages: Vec<Store<Ride>> = (0..=points.len()).map(|_| Store::new(&ctx)).collect();
    let env = Rc::new(RefCell::new(env));
    {
        let (env, ctx, clock, view) = (
            Rc::clone(&env),
            ctx.clone(),
            Rc::clone(&clock),
            Rc::clone(&view),
        );
        let Some(riding) = stages.first().cloned() else {
            return view;
        };
        let places = places.clone();
        sim.spawn_tagged(me, async move {
            loop {
                let input = env.borrow().input.clone();
                let entity = input.get().await;
                let now = ctx.now();
                {
                    let e = env.borrow();
                    e.adjust(|s| &mut s.waiting, -1.0);
                    e.adjust(|s| &mut s.busy, 1.0);
                }
                let at = clock.time(now);
                let ride = view.rides.get() + 1;
                view.rides.set(ride);
                view.items.borrow_mut().push_back(OnBelt {
                    ride,
                    entity: entity.id,
                    got_on: now,
                    at,
                    key: product.as_ref().map_or(0, |p| p.key(&entity)),
                    gone: None,
                });
                riding.put((entity, now, at, ride));
                let (c, k, p) = (ctx.clone(), Rc::clone(&clock), places.clone());
                ctx.spawn(async move {
                    k.until(&c, at + pitch / speed).await;
                    if let Some(p) = p {
                        p.open_entrance();
                    }
                });
            }
        });
    }
    for (k, point) in points.into_iter().enumerate() {
        let (Some(from), Some(to)) = (stages.get(k).cloned(), stages.get(k + 1).cloned()) else {
            continue;
        };
        let (env, ctx, clock, view) = (
            Rc::clone(&env),
            ctx.clone(),
            Rc::clone(&clock),
            Rc::clone(&view),
        );
        sim.spawn_tagged(me, async move {
            loop {
                let (mut entity, got_on, at, ride) = from.get().await;
                if accumulating {
                    view.wait_reach(&ctx, ride, point.s).await;
                } else {
                    clock.until(&ctx, at + point.s / speed).await;
                }
                let (stats, assigner) = {
                    let e = env.borrow();
                    (Rc::clone(&e.stats), Rc::clone(&e.assigner))
                };
                let Some(way) = point.offer(&mut entity, &stats, &assigner) else {
                    to.put((entity, got_on, at, ride));
                    continue;
                };
                // It leaves the belt here, sideways.
                let now = ctx.now();
                {
                    let e = env.borrow();
                    e.adjust(|s| &mut s.busy, -1.0);
                    e.with_stats(|s| {
                        s.left += 1;
                        s.stay_ends(now);
                        s.time_on.add(now - got_on);
                        s.time_on_samples.push(now - got_on);
                    });
                    if let Some(p) = &e.places {
                        p.leave(entity.id);
                    }
                }
                view.diverted(ride, now, point.s);
                // Off this belt first, so a way that leads back onto it counts the entity
                // again; the way had room, which a request takes at once.
                if let Some(p) = &way.places {
                    p.wait_room(entity.id).await;
                }
                way.dispatch(&ctx, entity);
            }
        });
    }
    let Some(riding) = stages.last().cloned() else {
        return view;
    };
    let seen = Rc::clone(&view);
    sim.spawn_tagged(me, async move {
        let mut last_left = f64::NEG_INFINITY;
        loop {
            let (entity, got_on, at, ride) = riding.get().await;
            clock.until(&ctx, at + length / speed).await;
            if accumulating {
                let behind = last_left + pitch / speed - ctx.now();
                if behind > 0.0 {
                    ctx.hold(behind).await;
                }
                if has_points {
                    // An item that left at a point ahead may still be in the way.
                    seen.wait_reach(&ctx, ride, length).await;
                }
            }
            env.borrow().adjust(|s| &mut s.busy, -1.0);
            if !accumulating {
                clock.stop(ctx.now());
            }
            move_on(&env, &ctx, entity, &|| false, None).await;
            let now = ctx.now();
            if !accumulating {
                clock.start(now);
            }
            env.borrow().with_stats(|s| {
                s.time_on.add(now - got_on);
                s.time_on_samples.push(now - got_on);
            });
            last_left = now;
            // It stays in the view until it has left, even while it waits at the end.
            seen.left_end(ride, now);
        }
    });
    view
}

/// An item on a conveyor, as its view keeps it.
struct OnBelt {
    /// Its ride's number on this belt ([`Ride`]).
    ride: u64,
    entity: u64,
    /// When it got on.
    got_on: f64,
    /// The belt's own time then.
    at: f64,
    /// Its product key ([`Product::key`]).
    key: u32,
    /// When and where it left at a divert point, on an accumulating belt: it is kept, unseen,
    /// as long as it can hold up the items behind it, as if it moved on at the speed.
    gone: Option<(f64, f64)>,
}

/// What a watcher can see of a conveyor: the items on it, front first, and what it takes to place
/// them at any moment from what is kept.
struct BeltView {
    clock: Rc<BeltClock>,
    length: f64,
    speed: f64,
    pitch: f64,
    accumulating: bool,
    items: RefCell<VecDeque<OnBelt>>,
    /// When the last item left the end.
    last_left: Cell<Option<f64>>,
    /// The rides so far, numbering each item that gets on.
    rides: Cell<u64>,
    /// Woken whenever an item leaves the belt, at its end or at a point.
    changed: Signal,
}

impl BeltView {
    /// Each item's fraction of the length along the belt at `now`, its entity and its key.
    ///
    /// Accumulating, an item has moved at the speed since it got on, but is never less than a
    /// pitch behind the item ahead (for the front one, the item that left last, which went on
    /// moving at the speed), and never past the end: the same rule as the times the conveyor's
    /// processes work out. An item that left at a point counts as the item ahead too, moving
    /// on at the speed from there. Not accumulating, it has moved the belt's time since it got on
    /// times the speed, up to the end.
    fn positions(&self, now: f64) -> Vec<(f32, u64, u32)> {
        let belt_now = self.clock.time(now);
        let mut ahead: Option<f64> = None;
        // The nearest of the items that left at a point since the last one still on.
        let mut gone_ahead: Option<f64> = None;
        let mut out = Vec::new();
        for item in self.items.borrow().iter() {
            if let Some((d, s)) = item.gone {
                let x = s + (now - d) * self.speed;
                gone_ahead = Some(gone_ahead.map_or(x, |g| g.min(x)));
                continue;
            }
            let along = if self.accumulating {
                let moved = (now - item.got_on) * self.speed;
                let room = match (ahead, self.last_left.get()) {
                    (Some(a), _) => a - self.pitch,
                    (None, Some(d)) => self.length - self.pitch + (now - d) * self.speed,
                    (None, None) => f64::INFINITY,
                };
                let room = match gone_ahead.take() {
                    Some(g) => room.min(g - self.pitch),
                    None => room,
                };
                moved.min(room)
            } else {
                (belt_now - item.at) * self.speed
            };
            let x = if along.is_nan() {
                0.0
            } else {
                along.min(self.length).max(0.0)
            };
            ahead = Some(x);
            let fraction = if self.length > 0.0 {
                (x / self.length).clamp(0.0, 1.0)
            } else {
                1.0
            };
            out.push((fraction as f32, item.entity, item.key));
        }
        out
    }

    /// When the front of ride `ride` reaches `x` along an accumulating belt, as things stand: the
    /// item moves at the speed from when it got on, a pitch behind the item ahead (or behind
    /// one that left at a point, or the last to leave the end, each moving on at the speed),
    /// and an item still on the belt goes no further than the end. Every position is then
    /// min(v·t + a, c) for constants a and c, so the time is (x − a)/v, or `None` while an
    /// item ahead must first leave the end (c < x). Items leaving only ever bring it forward.
    fn reach(&self, ride: u64, x: f64) -> Option<f64> {
        let (v, p, length) = (self.speed, self.pitch, self.length);
        // The limit the items ahead set: (a, c).
        let mut limit = match self.last_left.get() {
            Some(d) => (length - v * d, f64::INFINITY),
            None => (f64::INFINITY, f64::INFINITY),
        };
        for item in self.items.borrow().iter() {
            match item.gone {
                Some((d, s)) => limit.0 = limit.0.min(s - v * d),
                None => {
                    let a = (-v * item.got_on).min(limit.0 - p);
                    let c = length.min(limit.1 - p);
                    if item.ride == ride {
                        let slack = 1e-9 * x.abs().max(1.0);
                        return (c >= x - slack).then_some((x - a) / v);
                    }
                    limit = (a, c);
                }
            }
        }
        None
    }

    /// Waits until the front of ride `ride` has reached `x` along an accumulating belt.
    async fn wait_reach(&self, ctx: &Ctx, ride: u64, x: f64) {
        loop {
            match self.reach(ride, x) {
                Some(t) => {
                    let left = t - ctx.now();
                    // Rounding must not leave a wait too short to move the clock.
                    if left <= 1e-9 * t.abs().max(1.0) {
                        return;
                    }
                    ctx.hold(left).await;
                }
                None => self.changed.wait().await,
            }
        }
    }

    /// Ride `ride` left at a point `s` along, at `now`.
    fn diverted(&self, ride: u64, now: f64, s: f64) {
        {
            let mut items = self.items.borrow_mut();
            if self.accumulating {
                if let Some(item) = items.iter_mut().find(|i| i.ride == ride) {
                    item.gone = Some((now, s));
                }
                self.forget(&mut items, now);
            } else {
                items.retain(|i| i.ride != ride);
            }
        }
        self.changed.notify_all();
    }

    /// Ride `ride` left the end at `now`. Those that left at a point ahead of it can no longer
    /// hold anything up.
    fn left_end(&self, ride: u64, now: f64) {
        {
            let mut items = self.items.borrow_mut();
            match items.iter().position(|i| i.ride == ride) {
                Some(k) => {
                    items.drain(..=k);
                }
                None => {
                    items.pop_front();
                }
            }
            self.forget(&mut items, now);
        }
        self.last_left.set(Some(now));
        self.changed.notify_all();
    }

    /// Drops the items that left at a point and have since moved more than a pitch past the
    /// end, so they hold nothing up.
    fn forget(&self, items: &mut VecDeque<OnBelt>, now: f64) {
        let beyond = self.length + self.pitch;
        items.retain(|i| match i.gone {
            Some((d, s)) => s + (now - d) * self.speed < beyond,
            None => true,
        });
    }
}

/// What colours the items on a conveyor: the model's first text property, and its values in the
/// order the model gives them.
struct Product {
    slot: usize,
    values: Vec<String>,
}

impl Product {
    /// The model's product ([`Model::product_values`]), if it has one.
    fn of(model: &Model, slots: &[String]) -> Option<Self> {
        let (name, values) = model.product_values()?;
        Some(Self {
            slot: slots.binary_search(&name).ok()?,
            values,
        })
    }

    /// The index of `e`'s value among the property's values; 0 when it has none.
    fn key(&self, e: &Entity) -> u32 {
        let Some(Some(Held::Text(s))) = e.properties.get(self.slot) else {
            return 0;
        };
        self.values
            .iter()
            .position(|v| v == s)
            .and_then(|i| u32::try_from(i).ok())
            .unwrap_or(0)
    }
}

fn spawn_delay(sim: &mut Sim, env: BlockEnv, duration: Rc<Times>) {
    let me = tag(env.me);
    let env = Rc::new(RefCell::new(env));
    let mut sim_ctx = None;
    {
        let e = env.borrow();
        sim_ctx.get_or_insert(e.ctx.clone());
    }
    let Some(ctx) = sim_ctx else { return };
    sim.spawn_tagged(me, async move {
        loop {
            let input = env.borrow().input.clone();
            let entity = input.get().await;
            let dt = {
                let mut e = env.borrow_mut();
                e.adjust(|s| &mut s.waiting, -1.0);
                e.adjust(|s| &mut s.busy, 1.0);
                duration.sample(&entity, None, &mut e.rng)
            };
            let (c, env2) = (ctx.clone(), Rc::clone(&env));
            ctx.spawn(async move {
                c.hold(dt).await;
                env2.borrow().adjust(|s| &mut s.busy, -1.0);
                move_on(&env2, &c, entity, &|| false, None).await;
            });
        }
    });
}

/// How a sink measures the times in system it keeps one by one: on the clock, or counting only
/// a calendar's active time (working time).
enum Clock {
    Wall,
    Working(Active),
}

impl Clock {
    /// The time from `from` to `to` on this clock.
    fn between(&self, from: f64, to: f64) -> f64 {
        match self {
            Clock::Wall => to - from,
            Clock::Working(a) => a.until(to) - a.until(from),
        }
    }
}

/// A calendar's active time from time 0: its periods in order, each with the active time
/// before it (prefix sums), so the active time up to any time is one binary search away.
struct Active {
    /// Start, end and the active time before the start.
    periods: Vec<(f64, f64, f64)>,
}

impl Active {
    /// From a calendar's periods: in order, not overlapping (as [`Calendar::periods`] gives
    /// them).
    fn new(periods: &[(f64, f64)]) -> Self {
        let mut before = 0.0;
        let periods = periods
            .iter()
            .map(|&(s, e)| {
                let p = (s, e, before);
                before += e - s;
                p
            })
            .collect();
        Self { periods }
    }

    /// The active time from 0 to `t`.
    fn until(&self, t: f64) -> f64 {
        let started = self.periods.partition_point(|p| p.0 <= t);
        match started.checked_sub(1).and_then(|i| self.periods.get(i)) {
            Some(&(s, e, before)) => before + (t.min(e) - s),
            None => 0.0,
        }
    }
}

/// A sink. With `scrap`, what reaches it is scrap, counted against the server given, or else
/// against each entity's last server. With `clock`, each entity's time in system is kept, as
/// that clock measures it, for its percentiles.
fn spawn_sink(sim: &mut Sim, env: BlockEnv, scrap: Option<Option<usize>>, clock: Option<Clock>) {
    sim.spawn_tagged(tag(env.me), async move {
        loop {
            let entity = env.input.get().await;
            let now = env.ctx.now();
            env.adjust(|s| &mut s.waiting, -1.0);
            env.with_stats(|s| {
                s.left += 1;
                s.time_in_system.add(now - entity.created);
                if let Some(c) = &clock {
                    s.times_in_system.push(c.between(entity.created, now));
                }
            });
            if let Some(s) = scrap
                .and_then(|charged| charged.or(entity.last_server))
                .and_then(|i| env.stats.get(i))
            {
                s.borrow_mut().scrap += 1;
            }
        }
    });
}

/// Which of `outs` an entity takes: the first whose condition holds, else the default, if any
/// connection has a condition; otherwise one drawn by weight. With one connection, nothing is
/// drawn, so a gateway with a single way out never changes the random numbers.
fn choose(outs: &[Out], e: &Entity, assigner: &Assigner, rng: &mut Rng) -> Option<usize> {
    if outs.iter().any(|o| o.condition.is_some()) {
        let read = |name: &str| assigner.read(e, name);
        return outs
            .iter()
            .position(|o| {
                !o.default
                    && o.condition
                        .as_ref()
                        .is_some_and(|c| c.eval(&read).boolean())
            })
            .or_else(|| outs.iter().position(|o| o.default));
    }
    if outs.len() <= 1 {
        return if outs.is_empty() { None } else { Some(0) };
    }
    let total: f64 = outs.iter().map(|o| o.weight).sum();
    let mut u = rng.uniform() * total;
    for (i, o) in outs.iter().enumerate() {
        if u < o.weight {
            return Some(i);
        }
        u -= o.weight;
    }
    // Rounding: the last connection with any weight.
    outs.iter().rposition(|o| o.weight > 0.0)
}

/// An exclusive gateway. A shortest-queue one (`router`) is routed through by the elements
/// sending into it; only an entity that still arrives here (from another shortest-queue gateway)
/// is routed by its own process, which then waits for room.
fn spawn_exclusive(sim: &mut Sim, mut env: BlockEnv, outs: Vec<Out>, router: Option<Rc<Router>>) {
    sim.spawn_tagged(tag(env.me), async move {
        loop {
            let entity = env.input.get().await;
            env.adjust(|s| &mut s.waiting, -1.0);
            if let Some(r) = &router {
                let way = r.route(&entity).await;
                env.with_stats(|s| s.left += 1);
                way.dispatch(&env.ctx, entity);
                continue;
            }
            let to = choose(&outs, &entity, &env.assigner, &mut env.rng);
            // With no route (no condition holds and there is no default), the entity leaves
            // the model here, and is counted as lost.
            // A full finite buffer holds the gateway, and everything behind it, up.
            match to.and_then(|i| outs.get(i)) {
                Some(out) => send_on(&env, &out.port, entity, 1.0).await,
                None => env.with_stats(|s| s.lost += 1),
            }
        }
    });
}

/// Sends `e` on from a gateway, batch or unbatch, as [`Port::deliver`] does. While a full finite
/// buffer holds it up, the `n` entities it stands for (a batch's members) still count here, as
/// blocked, and it counts as having left only once it gets in, as from a server.
async fn send_on(env: &BlockEnv, out: &Port, mut e: Entity, n: f64) {
    let blocks = out.may_block();
    if blocks {
        env.adjust(|s| &mut s.blocked, n);
    }
    let to = out.make_room(&mut e).await;
    if blocks {
        env.adjust(|s| &mut s.blocked, -n);
    }
    env.with_stats(|s| s.left += 1);
    match to {
        Ok(to) => to.dispatch(&env.ctx, e),
        Err(why) => out.refuse(e, why),
    }
}

/// The entity a join sends on: the one from its first input, with any property it lacks taken
/// from the others, in input order.
fn combine(mut parts: Vec<Entity>) -> Option<Entity> {
    let mut first = if parts.is_empty() {
        return None;
    } else {
        parts.remove(0)
    };
    for other in parts {
        if first.properties.len() < other.properties.len() {
            first.properties.resize(other.properties.len(), None);
        }
        for (mine, theirs) in first.properties.iter_mut().zip(other.properties) {
            if mine.is_none() {
                *mine = theirs;
            }
        }
    }
    Some(first)
}

/// A parallel gateway. A split sends a copy of each entity down every outgoing connection; a
/// join waits for one entity on each input and merges them into one ([`combine`]), then sends
/// it on (copied, if it also splits).
///
/// A join pairs entities by arrival order: the next one waiting on each input, whichever split
/// made them. With several entities' copies in flight, copies of different entities can be
/// merged with each other, if one branch overtakes another.
///
/// Counting: every entity that arrives counts in `entered`, on any input, and waits (counted in
/// the work in progress) until the join has one for each input; `left` counts each entity sent
/// out, one per outgoing connection. So a 2-in/1-out join reports entered = 2 x left (less those
/// still waiting at the end, which are in the work in progress), and a 1-in/2-out split
/// left = 2 x entered. The work in progress never exceeds entered − left.
///
/// An assembly (a join with a bill of materials) takes `quantities[k]` entities from input `k`
/// for each one it sends on; the parts other than the first are used up, so it reports
/// entered = left × the parts per assembly (less those still waiting at the end).
fn spawn_parallel(
    sim: &mut Sim,
    env: BlockEnv,
    inputs: Vec<Store<Entity>>,
    quantities: Vec<u32>,
    outs: Vec<Out>,
    next_id: Rc<RefCell<u64>>,
) {
    sim.spawn_tagged(tag(env.me), async move {
        loop {
            // The parts from each input (a join waits for all of them), in input order.
            let mut parts = Vec::with_capacity(inputs.len());
            for (k, input) in inputs.iter().enumerate() {
                for _ in 0..quantities.get(k).copied().unwrap_or(1).max(1) {
                    parts.push(input.get().await);
                }
            }
            env.adjust(|s| &mut s.waiting, -(parts.len() as f64));
            let Some(entity) = combine(parts) else {
                continue;
            };
            // The first connection takes the entity itself; the others take copies, each a new
            // entity with the same creation time and properties.
            for (k, out) in outs.iter().enumerate() {
                let mut e = entity.clone();
                if k > 0 {
                    let mut n = next_id.borrow_mut();
                    e.id = *n;
                    *n += 1;
                }
                send_on(&env, &out.port, e, 1.0).await;
            }
        }
    });
}

/// The most places a batch reserves before any entity has arrived.
const MAX_BATCH_PREALLOCATION: usize = 1024;

/// A batch: entities wait here until there are `size` of them, then go on together as one, the
/// first carrying the others ([`Entity::members`]). `entered` counts each entity, `left` each
/// batch, and the mean contents is the number waiting for their batch to fill.
fn spawn_batch(sim: &mut Sim, env: BlockEnv, size: u32) {
    let size = usize::try_from(size.max(1)).unwrap_or(usize::MAX);
    sim.spawn_tagged(tag(env.me), async move {
        // The size is the model's, so it only hints at the capacity: a huge one would otherwise
        // allocate for entities that never arrive.
        let mut held: Vec<Entity> = Vec::with_capacity(size.min(MAX_BATCH_PREALLOCATION));
        loop {
            let entity = env.input.get().await;
            env.adjust(|s| &mut s.waiting, -1.0);
            env.adjust(|s| &mut s.busy, 1.0);
            held.push(entity);
            if held.len() < size {
                continue;
            }
            let mut members = std::mem::take(&mut held);
            let n = members.len() as f64;
            env.adjust(|s| &mut s.busy, -n);
            let mut batch = members.remove(0);
            batch.members.extend(members);
            match &env.out {
                Some(out) => send_on(&env, out, batch, n).await,
                None => env.with_stats(|s| s.left += 1),
            }
        }
    });
}

/// Splits each batch into its entities and sends each on, the carrier first; anything else
/// passes through. `entered` counts batches as they arrive, `left` each entity sent on.
fn spawn_unbatch(sim: &mut Sim, env: BlockEnv) {
    sim.spawn_tagged(tag(env.me), async move {
        loop {
            let mut entity = env.input.get().await;
            env.adjust(|s| &mut s.waiting, -1.0);
            let members = std::mem::take(&mut entity.members);
            // The members still to go wait here, blocked, while the one before them waits for
            // room ([`send_on`] counts each from its turn).
            let blocks = env.out.as_ref().is_some_and(Port::may_block);
            let mut behind = if blocks { members.len() as f64 } else { 0.0 };
            env.adjust(|s| &mut s.blocked, behind);
            for (k, e) in std::iter::once(entity).chain(members).enumerate() {
                if k > 0 && behind > 0.0 {
                    env.adjust(|s| &mut s.blocked, -1.0);
                    behind -= 1.0;
                }
                match &env.out {
                    Some(out) => send_on(&env, out, e, 1.0).await,
                    None => env.with_stats(|s| s.left += 1),
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alt(pools: std::ops::Range<usize>, quantity: u32) -> Alternative {
        Alternative {
            pools: pools.collect(),
            quantity,
        }
    }

    #[test]
    fn a_busy_machine_ends_the_search_before_any_spread_of_workers_is_tried() {
        // Thirty worker pools of one free unit each and a machine with none free: fifteen
        // workers could be spread over the pools in 155,117,520 ways, none of which helps.
        let mut free = vec![1_i64; 30];
        free.push(0);
        let needs = vec![vec![alt(0..30, 15)], vec![alt(30..31, 1)]];
        let mut taken = Taken::new();
        assert!(!take_all(&needs, &mut free, &mut taken));
        assert!(taken.is_empty());
        assert_eq!(free.iter().sum::<i64>(), 30);
        // With the machine free, the first fifteen pools give a worker each.
        free[30] = 1;
        assert!(take_all(&needs, &mut free, &mut taken));
        let mut want: Taken = (0..15).map(|p| (p, 1)).collect();
        want.push((30, 1));
        assert_eq!(taken, want);
    }

    #[test]
    fn a_requirement_met_only_by_another_pool_still_gets_it() {
        // Two workers from pools 0 or 1, then one from pool 0 alone: the first must leave
        // pool 0's unit for the second, so it takes both from pool 1.
        let mut free = vec![1_i64, 2];
        let needs = vec![vec![alt(0..2, 2)], vec![alt(0..1, 1)]];
        let mut taken = Taken::new();
        assert!(take_all(&needs, &mut free, &mut taken));
        assert_eq!(taken, vec![(1, 2), (0, 1)]);
    }

    #[test]
    fn requirements_each_meetable_alone_but_not_together_end_the_search_at_once() {
        // Thirty-one worker pools of one unit each, one of them busy, and a task needing fifteen
        // workers and then sixteen: each alone could be met from the thirty free, both together
        // not, and the fifteen could be spread over the pools in 155,117,520 ways.
        let mut free = vec![1_i64; 31];
        free[30] = 0;
        let needs = vec![vec![alt(0..31, 15)], vec![alt(0..31, 16)]];
        let mut taken = Taken::new();
        let start = std::time::Instant::now();
        assert!(!take_all(&needs, &mut free, &mut taken));
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        assert!(taken.is_empty());
        assert_eq!(free.iter().sum::<i64>(), 30);
        // With every worker free, the first fifteen pools go to the first need, in order, and
        // the other sixteen to the second.
        free[30] = 1;
        assert!(take_all(&needs, &mut free, &mut taken));
        let want: Taken = (0..31).map(|p| (p, 1)).collect();
        assert_eq!(taken, want);
        assert!(free.iter().all(|&f| f == 0));
    }

    #[test]
    fn an_alternative_that_would_starve_a_later_requirement_is_passed_over() {
        // The first need takes pool 0 or else pool 1; the second only pool 0. Taking pool 0
        // first leaves the second unmet, so the first takes pool 1, as before.
        let mut free = vec![1_i64, 1];
        let needs = vec![vec![alt(0..1, 1), alt(1..2, 1)], vec![alt(0..1, 1)]];
        let mut taken = Taken::new();
        assert!(take_all(&needs, &mut free, &mut taken));
        assert_eq!(taken, vec![(1, 1), (0, 1)]);
        assert_eq!(free, vec![0, 0]);
    }

    #[test]
    fn too_many_ways_to_choose_still_find_the_first_take_or_none() {
        // A spare in pool 13 goes to the first need; each of thirteen more takes its own pool or
        // else the spare, 2^13 ways to choose, more than are tried one by one.
        let mut free = vec![1_i64; 14];
        let mut needs = vec![vec![alt(13..14, 1)]];
        needs.extend((0..13).map(|p| vec![alt(p..p + 1, 1), alt(13..14, 1)]));
        let mut taken = Taken::new();
        assert!(take_all(&needs, &mut free, &mut taken));
        let mut want: Taken = vec![(13, 1)];
        want.extend((0..13).map(|p| (p, 1)));
        assert_eq!(taken, want);
        // With pool 5 busy, each need alone could still be met, but not all together: fourteen
        // units are needed and thirteen are free. The weaker check made over all the ways to
        // choose at once (each need taking its own pool or the spare) sees that.
        let mut free = vec![1_i64; 14];
        free[5] = 0;
        let mut taken = Taken::new();
        assert!(!feasible(MAX_COMBINATIONS, None, &needs, &free));
        assert!(!take_all(&needs, &mut free, &mut taken));
        assert!(taken.is_empty());
        assert_eq!(free.iter().sum::<i64>(), 13);
    }

    #[test]
    fn a_combined_shortage_is_found_at_once_with_too_many_ways_to_choose() {
        // Thirty-one worker pools of one unit each, one of them busy, a task needing fifteen
        // workers and then sixteen, and thirteen more requirements each taking a unit of its own
        // pool or else of another: 2^13 = 8,192 ways to choose, more than are tried one by one.
        // Each requirement alone could be met, and the fifteen could be spread over the pools in
        // 155,117,520 ways.
        let mut free = vec![1_i64; 31 + 26];
        free[30] = 0;
        let mut needs = vec![vec![alt(0..31, 15)], vec![alt(0..31, 16)]];
        needs.extend((0..13).map(|i| {
            let p = 31 + 2 * i;
            vec![alt(p..p + 1, 1), alt(p + 1..p + 2, 1)]
        }));
        let mut taken = Taken::new();
        let start = std::time::Instant::now();
        assert!(!take_all(&needs, &mut free, &mut taken));
        assert!(start.elapsed() < std::time::Duration::from_millis(250));
        assert!(taken.is_empty());
        assert_eq!(free.iter().sum::<i64>(), 56);
        // With every worker free: the first fifteen pools, the other sixteen, then each of the
        // thirteen its first pool.
        free[30] = 1;
        assert!(take_all(&needs, &mut free, &mut taken));
        let mut want: Taken = (0..31).map(|p| (p, 1)).collect();
        want.extend((0..13).map(|i| (31 + 2 * i, 1)));
        assert_eq!(taken, want);
    }

    #[test]
    fn a_range_of_amounts_that_cannot_work_is_passed_over_at_once() {
        // Two pools of a billion units each; a billion from either, then a billion from the
        // first alone. Any units the first need takes from the first pool leave the second
        // unmet, so it takes them all from the second pool; finding that by trying one unit
        // fewer at a time would take a billion checks.
        let mut free = vec![1_000_000_000_i64; 2];
        let needs = vec![
            vec![alt(0..2, 1_000_000_000)],
            vec![alt(0..1, 1_000_000_000)],
        ];
        let mut taken = Taken::new();
        let start = std::time::Instant::now();
        assert!(take_all(&needs, &mut free, &mut taken));
        assert!(start.elapsed() < std::time::Duration::from_millis(250));
        assert_eq!(taken, vec![(1, 1_000_000_000), (0, 1_000_000_000)]);
        assert_eq!(free, vec![0, 0]);
    }

    /// The take of [`take_all`] found the plain way: every alternative in order, and every
    /// amount from each pool from the most down, one unit at a time, with no check ahead.
    fn take_all_unit_step(
        requirements: &[Vec<Alternative>],
        free: &mut [i64],
        taken: &mut Taken,
    ) -> bool {
        let Some((first, rest)) = requirements.split_first() else {
            return true;
        };
        first.iter().any(|alt| {
            alt.quantity > 0 && fill_unit_step(&alt.pools, alt.quantity, rest, free, taken)
        })
    }

    fn fill_unit_step(
        pools: &[usize],
        left: u32,
        rest: &[Vec<Alternative>],
        free: &mut [i64],
        taken: &mut Taken,
    ) -> bool {
        if left == 0 {
            return take_all_unit_step(rest, free, taken);
        }
        let Some((&p, others)) = pools.split_first() else {
            return false;
        };
        let elsewhere: i64 = others.iter().map(|&q| free[q].max(0)).sum();
        let here = free[p].clamp(0, i64::from(left)) as u32;
        for n in (0..=here).rev() {
            if i64::from(left - n) > elsewhere {
                break;
            }
            let mark = taken.len();
            if n > 0 {
                free[p] -= i64::from(n);
                taken.push((p, n));
            }
            if fill_unit_step(others, left - n, rest, free, taken) {
                return true;
            }
            if n > 0 {
                free[p] += i64::from(n);
            }
            taken.truncate(mark);
        }
        false
    }

    #[test]
    fn the_take_is_the_one_found_one_unit_at_a_time() {
        // Small random cases (seeded): up to four pools, some with none free or fewer than none,
        // and up to three requirements of up to three alternatives, some of no units and some
        // naming no pool. Each is taken with every way to choose tried one by one, and with only
        // the weaker check (at most one way, or none), and must give what the plain search gives.
        let mut state = 0x5eed_u64;
        let mut next = |below: u64| {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z ^ (z >> 31)) % below
        };
        let mut takes = 0;
        for _ in 0..3000 {
            let k = 1 + next(4) as usize;
            let free: Vec<i64> = (0..k).map(|_| next(5) as i64 - 1).collect();
            let needs: Vec<Vec<Alternative>> = (0..1 + next(3))
                .map(|_| {
                    (0..1 + next(3))
                        .map(|_| Alternative {
                            pools: (0..k).filter(|_| next(2) == 1).collect(),
                            quantity: next(4) as u32,
                        })
                        .collect()
                })
                .collect();
            let mut want_free = free.clone();
            let mut want = Taken::new();
            let found = take_all_unit_step(&needs, &mut want_free, &mut want);
            takes += usize::from(found);
            for limit in [MAX_COMBINATIONS, 1, 0] {
                let mut got_free = free.clone();
                let mut got = Taken::new();
                assert_eq!(
                    take_within(limit, &needs, &mut got_free, &mut got),
                    found,
                    "{limit}: {free:?}"
                );
                assert_eq!(got, want, "{limit}: {free:?}");
                assert_eq!(got_free, want_free, "{limit}: {free:?}");
            }
        }
        // Both kinds of case come up.
        assert!(takes > 100 && takes < 2900, "{takes}");
    }
}
