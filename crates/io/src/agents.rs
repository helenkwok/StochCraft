//! Agent-based models in files (decision D16): a BPMN file with no process, whose BPSim scenario
//! gives the run's length (the warm-up plus the duration), replications, seed and time unit, and
//! whose `stochcraft:agents` vendor extension ([`crate::bpsim::AGENTS`]) holds the rest. Each
//! agent type's state chart is an SCXML 1.0 file of its own ([`crate::scxml`]), named by a
//! relative reference.
//!
//! ```xml
//! <bpsim:Scenario id="scenario" name="SIR">
//!   <bpsim:ScenarioParameters replication="20" seed="1" baseTimeUnit="day">
//!     <bpsim:Duration><bpsim:FloatingParameter value="60"/></bpsim:Duration>
//!   </bpsim:ScenarioParameters>
//!   <bpsim:VendorExtension name="stochcraft:agents" xmlns:stochcraft="…">
//!     <stochcraft:agents saveEvery="1">
//!       <stochcraft:agentType name="person" chart="person.scxml"/>
//!       <stochcraft:population name="people" type="person" count="100">
//!         <stochcraft:network kind="ring" eachSide="2"/>
//!         <stochcraft:grid rows="10" columns="10" neighbourhood="vonNeumann" wrap="true"/>
//!       </stochcraft:population>
//!       <stochcraft:environment type="world"/>
//!       <stochcraft:statistic name="infected" population="people" measure="count"
//!           condition="In('infected')"/>
//!       <stochcraft:stepping every="1" order="random"/>
//!     </stochcraft:agents>
//!   </bpsim:VendorExtension>
//! </bpsim:Scenario>
//! ```
//!
//! A network is `empty`, `random` (`meanDegree`), `ring` (`eachSide`), `smallWorld`
//! (`eachSide`, `rewire`), `scaleFree` (`initial`, `links`), `distance` (`width`, `height`,
//! `range`), `links` (`link` children with `a` and `b`) or `file` (`href`, a CSV list of links).
//! A scenario that inherits from another takes the whole extension of the nearest scenario that
//! has one.

use stochcraft_model::agents::{
    AgentType, Agents, GridSpec, MeasureSpec, NetworkSpec, PopulationSpec, StatisticSpec,
    SteppingSpec,
};

use crate::Note;
use crate::bpsim::{AGENTS, NS2, NSS, STOCHCRAFT_NS};
use crate::scxml::escape_attr;
use crate::xml::{self, Document, Element};

fn st(local: &str) -> Element {
    Element::new(&format!("stochcraft:{local}"), Some(STOCHCRAFT_NS))
}

fn set(e: &mut Element, name: &str, value: &str) {
    e.set_attr_escaped(name, &escape_attr(value));
}

fn num(v: f64) -> String {
    crate::duration::trim_float(v)
}

/// The `stochcraft:agents` element.
pub(crate) fn body_element(a: &Agents) -> Element {
    let mut body = st("agents");
    if let Some(v) = a.save_every {
        set(&mut body, "saveEvery", &num(v));
    }
    if let Some(v) = a.microstep_limit {
        set(&mut body, "microstepLimit", &v.to_string());
    }
    if let Some(v) = a.retest_limit {
        set(&mut body, "retestLimit", &v.to_string());
    }
    for t in &a.types {
        let mut e = st("agentType");
        set(&mut e, "name", &t.name);
        set(&mut e, "chart", &t.chart);
        body.push(e);
    }
    for p in &a.populations {
        let mut e = st("population");
        set(&mut e, "name", &p.name);
        set(&mut e, "type", &p.agent_type);
        set(&mut e, "count", &p.count.to_string());
        if let Some(r) = p.arrival_rate {
            set(&mut e, "arrivalRate", &num(r));
        }
        if let Some(n) = &p.network {
            e.push(network_element(n));
        }
        if let Some(g) = &p.grid {
            let mut x = st("grid");
            set(&mut x, "rows", &g.rows.to_string());
            set(&mut x, "columns", &g.columns.to_string());
            if g.von_neumann {
                set(&mut x, "neighbourhood", "vonNeumann");
            }
            if g.wrap {
                set(&mut x, "wrap", "true");
            }
            if g.many_per_cell {
                set(&mut x, "perCell", "many");
            }
            if g.random_placement {
                set(&mut x, "placement", "random");
            }
            e.push(x);
        }
        body.push(e);
    }
    if let Some(t) = &a.environment {
        let mut e = st("environment");
        set(&mut e, "type", t);
        body.push(e);
    }
    for s in &a.statistics {
        let mut e = st("statistic");
        set(&mut e, "name", &s.name);
        set(&mut e, "population", &s.population);
        set(&mut e, "measure", s.measure.file_name());
        if let Some(x) = &s.expression {
            set(&mut e, "expression", x);
        }
        if let Some(c) = &s.condition {
            set(&mut e, "condition", c);
        }
        body.push(e);
    }
    if let Some(s) = &a.stepping {
        let mut e = st("stepping");
        set(&mut e, "every", &num(s.every));
        if s.random_order {
            set(&mut e, "order", "random");
        }
        body.push(e);
    }
    body
}

fn network_element(n: &NetworkSpec) -> Element {
    let mut e = st("network");
    set(&mut e, "kind", n.file_name());
    match n {
        NetworkSpec::Empty => {}
        NetworkSpec::Random { mean_degree } => set(&mut e, "meanDegree", &num(*mean_degree)),
        NetworkSpec::Ring { each_side } => set(&mut e, "eachSide", &each_side.to_string()),
        NetworkSpec::SmallWorld { each_side, rewire } => {
            set(&mut e, "eachSide", &each_side.to_string());
            set(&mut e, "rewire", &num(*rewire));
        }
        NetworkSpec::ScaleFree { initial, links } => {
            set(&mut e, "initial", &initial.to_string());
            set(&mut e, "links", &links.to_string());
        }
        NetworkSpec::Distance {
            width,
            height,
            range,
        } => {
            set(&mut e, "width", &num(*width));
            set(&mut e, "height", &num(*height));
            set(&mut e, "range", &num(*range));
        }
        NetworkSpec::Links(links) => {
            for (a, b) in links {
                let mut l = st("link");
                set(&mut l, "a", &a.to_string());
                set(&mut l, "b", &b.to_string());
                e.push(l);
            }
        }
        NetworkSpec::File { href } => set(&mut e, "href", href),
    }
    e
}

/// The `stochcraft:agents` vendor extension.
pub(crate) fn extension_element(a: &Agents) -> Element {
    Element::new("bpsim:VendorExtension", Some(NS2))
        .with_attr("name", AGENTS)
        .with_attr("xmlns:stochcraft", STOCHCRAFT_NS)
        .with_child(body_element(a))
}

/// The agent-based part of a model as text: the `stochcraft:agents` element as it is written,
/// which two equal descriptions give alike (for digests).
pub fn text(a: &Agents) -> String {
    let body = body_element(a).with_attr("xmlns:stochcraft", STOCHCRAFT_NS);
    xml::write(&Document::new(body))
}

struct Read<'a> {
    notes: &'a mut Vec<Note>,
}

impl Read<'_> {
    fn warn(&mut self, at: Option<&str>, message: impl Into<String>) {
        self.notes.push(Note::warning(at, message));
    }

    fn error(&mut self, at: Option<&str>, message: impl Into<String>) {
        self.notes.push(Note::error(at, message));
    }

    fn float(&mut self, e: &Element, name: &str, at: &str) -> Option<f64> {
        let raw = e.attr(name)?;
        match raw.trim().parse::<f64>() {
            Ok(v) if v.is_finite() => Some(v),
            _ => {
                self.error(
                    Some(at),
                    format!("`{name}` `{raw}` is not a number and is ignored"),
                );
                None
            }
        }
    }

    fn whole(&mut self, e: &Element, name: &str, at: &str) -> Option<u64> {
        let raw = e.attr(name)?;
        match raw.trim().parse::<u64>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.error(
                    Some(at),
                    format!("`{name}` `{raw}` is not a whole number of 0 or more and is ignored"),
                );
                None
            }
        }
    }

    fn flag(&mut self, e: &Element, name: &str, yes: &str, no: &str, at: &str) -> bool {
        match e.attr(name) {
            None => false,
            Some(v) if v == yes => true,
            Some(v) if v == no => false,
            Some(v) => {
                self.warn(
                    Some(at),
                    format!("`{name}` `{v}` is not {yes} or {no}; {no} is used"),
                );
                false
            }
        }
    }

    fn network(&mut self, e: &Element, at: &str) -> Option<NetworkSpec> {
        let kind = e.attr("kind").unwrap_or_default();
        let need_f = |r: &mut Self, name: &str| {
            let v = r.float(e, name, at);
            if v.is_none() && e.attr(name).is_none() {
                r.error(Some(at), format!("a {kind} network needs `{name}`"));
            }
            v
        };
        let need_u = |r: &mut Self, name: &str| {
            let v = r.whole(e, name, at);
            if v.is_none() && e.attr(name).is_none() {
                r.error(Some(at), format!("a {kind} network needs `{name}`"));
            }
            v
        };
        Some(match kind.as_str() {
            "empty" => NetworkSpec::Empty,
            "random" => NetworkSpec::Random {
                mean_degree: need_f(self, "meanDegree")?,
            },
            "ring" => NetworkSpec::Ring {
                each_side: need_u(self, "eachSide")?,
            },
            "smallWorld" => {
                let each_side = need_u(self, "eachSide");
                let rewire = need_f(self, "rewire");
                NetworkSpec::SmallWorld {
                    each_side: each_side?,
                    rewire: rewire?,
                }
            }
            "scaleFree" => {
                let initial = need_u(self, "initial");
                let links = need_u(self, "links");
                NetworkSpec::ScaleFree {
                    initial: initial?,
                    links: links?,
                }
            }
            "distance" => {
                let (w, h, r) = (
                    need_f(self, "width"),
                    need_f(self, "height"),
                    need_f(self, "range"),
                );
                NetworkSpec::Distance {
                    width: w?,
                    height: h?,
                    range: r?,
                }
            }
            "links" => {
                let mut links = Vec::new();
                for l in e.elements().filter(|l| l.is(STOCHCRAFT_NS, "link")) {
                    match (self.whole(l, "a", at), self.whole(l, "b", at)) {
                        (Some(a), Some(b)) => links.push((a, b)),
                        _ => self.error(Some(at), "a link names its two agents by `a` and `b`"),
                    }
                }
                NetworkSpec::Links(links)
            }
            "file" => match e.attr("href") {
                Some(href) => NetworkSpec::File { href },
                None => {
                    self.error(Some(at), "a network from a file names it by `href`");
                    return None;
                }
            },
            other => {
                self.error(
                    Some(at),
                    format!("the network kind `{other}` is not known and the network is ignored"),
                );
                return None;
            }
        })
    }

    fn grid(&mut self, e: &Element, at: &str) -> Option<GridSpec> {
        let rows = self.whole(e, "rows", at);
        let columns = self.whole(e, "columns", at);
        let (Some(rows), Some(columns)) = (rows, columns) else {
            self.error(
                Some(at),
                "a grid gives its `rows` and `columns`; it is ignored",
            );
            return None;
        };
        Some(GridSpec {
            rows,
            columns,
            von_neumann: self.flag(e, "neighbourhood", "vonNeumann", "moore", at),
            wrap: self.flag(e, "wrap", "true", "false", at),
            many_per_cell: self.flag(e, "perCell", "many", "one", at),
            random_placement: self.flag(e, "placement", "random", "inOrder", at),
        })
    }

    fn body(&mut self, body: &Element) -> Agents {
        let mut a = Agents {
            save_every: self.float(body, "saveEvery", "agents"),
            microstep_limit: self.whole(body, "microstepLimit", "agents"),
            retest_limit: self.whole(body, "retestLimit", "agents"),
            ..Agents::default()
        };
        for e in body.elements() {
            if !e.is(STOCHCRAFT_NS, e.local()) {
                self.warn(
                    None,
                    format!(
                        "`{}` in an agent-based model is not known and is ignored",
                        e.name
                    ),
                );
                continue;
            }
            match e.local() {
                "agentType" => match (e.attr("name"), e.attr("chart")) {
                    (Some(name), Some(chart)) => a.types.push(AgentType { name, chart }),
                    _ => self.error(
                        None,
                        "an agent type gives its `name` and its state chart's file in `chart`; it is ignored",
                    ),
                },
                "population" => {
                    let (Some(name), Some(agent_type)) = (e.attr("name"), e.attr("type")) else {
                        self.error(
                            None,
                            "a population gives its `name` and its agent `type`; it is ignored",
                        );
                        continue;
                    };
                    let at = name.clone();
                    let mut p = PopulationSpec {
                        count: self.whole(e, "count", &at).unwrap_or(0),
                        arrival_rate: self.float(e, "arrivalRate", &at),
                        name,
                        agent_type,
                        network: None,
                        grid: None,
                    };
                    for c in e.elements() {
                        if c.is(STOCHCRAFT_NS, "network") {
                            p.network = self.network(c, &at);
                        } else if c.is(STOCHCRAFT_NS, "grid") {
                            p.grid = self.grid(c, &at);
                        } else {
                            self.warn(
                                Some(&at),
                                format!("`{}` in a population is not known and is ignored", c.name),
                            );
                        }
                    }
                    a.populations.push(p);
                }
                "environment" => match e.attr("type") {
                    Some(t) => a.environment = Some(t),
                    None => self.error(None, "the environment gives its agent `type`"),
                },
                "statistic" => {
                    let name = e.attr("name").unwrap_or_default();
                    let measure = e.attr("measure").unwrap_or_else(|| "count".to_owned());
                    let Some(measure) = MeasureSpec::from_file_name(&measure) else {
                        self.error(
                            Some(&name),
                            format!("the measure `{measure}` is not count, sum, mean, min or max; the statistic is ignored"),
                        );
                        continue;
                    };
                    a.statistics.push(StatisticSpec {
                        population: e.attr("population").unwrap_or_default(),
                        measure,
                        expression: e.attr("expression"),
                        condition: e.attr("condition"),
                        name,
                    });
                }
                "stepping" => match self.float(e, "every", "stepping") {
                    Some(every) => {
                        let random_order = self.flag(e, "order", "random", "fixed", "stepping");
                        a.stepping = Some(SteppingSpec {
                            every,
                            random_order,
                        });
                    }
                    None => self.error(
                        Some("stepping"),
                        "the stepped mode gives its step in `every`; it is ignored",
                    ),
                },
                other => self.warn(
                    None,
                    format!("`{other}` in an agent-based model is not known and is ignored"),
                ),
            }
        }
        a
    }
}

/// The agent-based model scenario `id` of `data` holds, or `None` if neither it nor a scenario it
/// inherits from has the `stochcraft:agents` extension.
pub(crate) fn read(data: &Element, id: &str, notes: &mut Vec<Note>) -> Option<Agents> {
    let chain = crate::mc::chain(data, id);
    let body = chain
        .iter()
        .rev()
        .copied()
        .flat_map(|s| s.children_named(&NSS, "VendorExtension"))
        .find(|x| x.attr("name").as_deref() == Some(AGENTS))?
        .elements()
        .find(|e| e.is(STOCHCRAFT_NS, "agents"));
    let Some(body) = body else {
        return Some(Agents::default());
    };
    Some(Read { notes }.body(body))
}

#[cfg(test)]
mod tests {
    use stochcraft_model::Model;

    use super::*;

    pub(crate) fn sample() -> Agents {
        Agents {
            types: vec![
                AgentType {
                    name: "person".into(),
                    chart: "person.scxml".into(),
                },
                AgentType {
                    name: "world".into(),
                    chart: "charts/world.scxml".into(),
                },
            ],
            populations: vec![
                PopulationSpec {
                    name: "people".into(),
                    agent_type: "person".into(),
                    count: 100,
                    arrival_rate: Some(0.25),
                    network: Some(NetworkSpec::SmallWorld {
                        each_side: 2,
                        rewire: 0.1,
                    }),
                    grid: Some(GridSpec {
                        rows: 10,
                        columns: 12,
                        von_neumann: true,
                        wrap: true,
                        many_per_cell: true,
                        random_placement: true,
                    }),
                },
                PopulationSpec {
                    name: "linked".into(),
                    agent_type: "person".into(),
                    count: 3,
                    arrival_rate: None,
                    network: Some(NetworkSpec::Links(vec![(0, 1), (1, 2)])),
                    grid: Some(GridSpec::new(2, 2)),
                },
                PopulationSpec {
                    name: "listed".into(),
                    agent_type: "person".into(),
                    count: 0,
                    arrival_rate: None,
                    network: Some(NetworkSpec::File {
                        href: "links.csv".into(),
                    }),
                    grid: None,
                },
            ],
            environment: Some("world".into()),
            statistics: vec![
                StatisticSpec {
                    name: "infected".into(),
                    population: "people".into(),
                    measure: MeasureSpec::Count,
                    expression: None,
                    condition: Some("In('infected') and $age < 40 & \"x\"\nor\ttrue".into()),
                },
                StatisticSpec {
                    name: "mean age".into(),
                    population: "people".into(),
                    measure: MeasureSpec::Mean,
                    expression: Some("$age".into()),
                    condition: None,
                },
            ],
            stepping: Some(SteppingSpec {
                every: 1.0 / 3.0,
                random_order: true,
            }),
            save_every: Some(0.1),
            microstep_limit: Some(500),
            retest_limit: None,
        }
    }

    fn model(a: Agents) -> Model {
        let mut m = Model {
            name: "Epidemic".into(),
            agents: Some(a),
            ..Model::default()
        };
        m.experiment.replications = 20;
        m.experiment.seed = 7;
        m.experiment.duration = 60.0;
        m
    }

    #[test]
    fn an_agent_model_is_a_bpmn_file_with_no_process_and_reads_back_as_it_was() {
        let m = model(sample());
        let bytes = crate::write_new(&m);
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("<bpmn:process"), "{text}");
        let loaded = crate::read(&bytes).map_err(|e| e.to_string());
        let Ok(loaded) = loaded else {
            unreachable!("{loaded:?}")
        };
        assert!(loaded.model.is_agents());
        assert_eq!(loaded.model, m, "{text}");
        assert!(loaded.notes.is_empty(), "{:?}", loaded.notes);
        // Written again, the same bytes.
        assert_eq!(crate::write_new(&loaded.model), bytes);
    }

    #[test]
    fn an_opened_agent_model_saves_unchanged_as_its_bytes_and_edited_as_the_edit() {
        let m = model(sample());
        let bytes = crate::write_new(&m);
        let loaded = crate::read(&bytes).map_err(|e| e.to_string());
        let Ok(loaded) = loaded else {
            unreachable!("{loaded:?}")
        };
        assert_eq!(loaded.opened.save(&m).as_deref(), Ok(&bytes[..]));
        let mut edited = m.clone();
        if let Some(a) = edited.agents.as_mut() {
            a.populations[0].count = 250;
            a.statistics.pop();
            a.stepping = None;
        }
        let saved = loaded.opened.save(&edited).map_err(|e| e.to_string());
        let Ok(saved) = saved else {
            unreachable!("{saved:?}")
        };
        let back = crate::read(&saved).map(|l| l.model);
        assert_eq!(back.ok(), Some(edited));
    }

    #[test]
    fn every_network_kind_reads_back() {
        let kinds = [
            NetworkSpec::Empty,
            NetworkSpec::Random { mean_degree: 4.5 },
            NetworkSpec::Ring { each_side: 3 },
            NetworkSpec::ScaleFree {
                initial: 3,
                links: 2,
            },
            NetworkSpec::Distance {
                width: 1.0,
                height: 2.5,
                range: 0.125,
            },
            NetworkSpec::Links(Vec::new()),
        ];
        for n in kinds {
            let mut a = sample();
            a.populations[0].network = Some(n);
            a.populations[0].grid = None;
            let m = model(a);
            let back = crate::read(&crate::write_new(&m)).map(|l| l.model);
            assert_eq!(back.ok(), Some(m));
        }
    }

    #[test]
    fn an_empty_extension_is_an_agent_model_with_nothing_in_it() {
        let m = model(Agents::default());
        let back = crate::read(&crate::write_new(&m)).map(|l| l.model);
        assert_eq!(back.ok(), Some(m));
    }

    #[test]
    fn bad_values_are_noted_not_guessed() {
        let m = model(sample());
        let text = String::from_utf8_lossy(&crate::write_new(&m))
            .replace("count=\"100\"", "count=\"many\"")
            .replace("measure=\"mean\"", "measure=\"median\"");
        let loaded = crate::read(text.as_bytes()).map_err(|e| e.to_string());
        let Ok(loaded) = loaded else {
            unreachable!("{loaded:?}")
        };
        let messages: Vec<&str> = loaded.notes.iter().map(|n| n.message.as_str()).collect();
        assert!(
            messages.iter().any(|m| m.contains("`count` `many`")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("`median`")),
            "{messages:?}"
        );
    }

    #[test]
    fn the_text_of_equal_models_is_equal() {
        assert_eq!(text(&sample()), text(&sample()));
        let mut other = sample();
        other.populations[0].count = 101;
        assert_ne!(text(&sample()), text(&other));
    }
}
