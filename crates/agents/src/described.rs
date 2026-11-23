//! A [`Model`] from the description a file holds (ROADMAP M4.3, stage A4): the
//! [`Agents`](stochcraft_model::agents::Agents) part of a `stochcraft_model::Model`, with each
//! agent type's state chart and each list of links read from the files it names.

use std::collections::BTreeMap;
use std::sync::Arc;

use stochcraft_model::agents::{Agents, MeasureSpec, NetworkSpec};

use crate::chart::{Chart, ChartError, ChartSpec};
use crate::network::{Grid, Neighbourhood, Network, Placement};
use crate::population::{Measure, Model, ModelError, Order, Population, Statistic, Stepping};

fn invalid<T>(place: impl Into<String>, message: impl Into<String>) -> Result<T, ModelError> {
    Err(ModelError::Invalid(ChartError {
        place: place.into(),
        message: message.into(),
    }))
}

/// The files a description names, by their references: each agent type's state chart, read, and
/// each list of links as its text.
#[derive(Debug, Clone, Default)]
pub struct Files {
    pub charts: BTreeMap<String, ChartSpec>,
    pub links: BTreeMap<String, String>,
}

/// Builds the model `agents` describes: each agent type's chart (from `files.charts`, by the
/// type's reference) checked and compiled once and shared by its populations, the populations
/// with their networks and grids, the environment, the statistics and the stepped mode. A type,
/// population or file that is named and not there, two types or populations of one name, and
/// a chart that does not compile are [`ModelError::Invalid`], with the place.
pub fn build(agents: &Agents, files: &Files) -> Result<Model, ModelError> {
    let mut charts: BTreeMap<&str, Arc<Chart>> = BTreeMap::new();
    for t in &agents.types {
        let place = format!("agent type `{}`", t.name);
        if charts.contains_key(t.name.as_str()) {
            return invalid(place, "two agent types have this name");
        }
        let Some(spec) = files.charts.get(&t.chart) else {
            return invalid(place, format!("its state chart `{}` was not read", t.chart));
        };
        let chart = Chart::new(spec).map_err(|e| {
            ModelError::Invalid(ChartError {
                place: format!("{place} ({}): {}", t.chart, e.place),
                message: e.message,
            })
        })?;
        charts.insert(&t.name, Arc::new(chart));
    }
    let chart_of = |name: &str, place: &str| -> Result<Arc<Chart>, ModelError> {
        match charts.get(name) {
            Some(c) => Ok(Arc::clone(c)),
            None => invalid(place, format!("there is no agent type `{name}`")),
        }
    };
    let mut model = Model::new();
    for p in &agents.populations {
        let place = format!("population `{}`", p.name);
        if model.populations.iter().any(|q| q.name == p.name) {
            return invalid(place, "two populations have this name");
        }
        let mut pop = Population::new(&p.name, chart_of(&p.agent_type, &place)?).count(p.count);
        if let Some(r) = p.arrival_rate {
            pop = pop.arrivals(r);
        }
        if let Some(n) = &p.network {
            pop = pop.network(match n {
                NetworkSpec::Empty => Network::empty(),
                NetworkSpec::Random { mean_degree } => Network::random(*mean_degree),
                NetworkSpec::Ring { each_side } => Network::ring(*each_side),
                NetworkSpec::SmallWorld { each_side, rewire } => {
                    Network::small_world(*each_side, *rewire)
                }
                NetworkSpec::ScaleFree { initial, links } => Network::scale_free(*initial, *links),
                NetworkSpec::Distance {
                    width,
                    height,
                    range,
                } => Network::distance(*width, *height, *range),
                NetworkSpec::Links(links) => Network::links(links.clone()),
                NetworkSpec::File { href } => match files.links.get(href) {
                    Some(text) => Network::csv(text),
                    None => {
                        return invalid(place, format!("its list of links `{href}` was not read"));
                    }
                },
            });
        }
        if let Some(g) = &p.grid {
            pop = pop.grid(Grid {
                rows: g.rows,
                columns: g.columns,
                neighbourhood: if g.von_neumann {
                    Neighbourhood::VonNeumann
                } else {
                    Neighbourhood::Moore
                },
                wrap: g.wrap,
                one_per_cell: !g.many_per_cell,
                placement: if g.random_placement {
                    Placement::Random
                } else {
                    Placement::InOrder
                },
            });
        }
        model = model.population(pop);
    }
    if let Some(t) = &agents.environment {
        model = model.environment(chart_of(t, "the environment")?);
    }
    for s in &agents.statistics {
        let measure = match s.measure {
            MeasureSpec::Count => Measure::Count,
            MeasureSpec::Sum => Measure::Sum,
            MeasureSpec::Mean => Measure::Mean,
            MeasureSpec::Min => Measure::Min,
            MeasureSpec::Max => Measure::Max,
        };
        model = model.statistic(Statistic {
            name: s.name.clone(),
            population: s.population.clone(),
            measure,
            expr: s.expression.clone(),
            condition: s.condition.clone(),
        });
    }
    if let Some(s) = &agents.stepping {
        model = model.stepped(Stepping {
            every: s.every,
            order: if s.random_order {
                Order::Random
            } else {
                Order::Fixed
            },
        });
    }
    if let Some(e) = agents.save_every {
        model = model.save_every(e);
    }
    if let Some(l) = agents.microstep_limit {
        model = model.microstep_limit(l);
    }
    if let Some(l) = agents.retest_limit {
        model = model.retest_limit(l);
    }
    Ok(model)
}
