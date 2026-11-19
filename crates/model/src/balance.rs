//! Line balancing (ROADMAP M7.1): given the work elements of an assembly, their times and which
//! must come before which, assign them to stations for a cycle time, with the theoretical least
//! number of stations beside the result so that any gap shows.
//!
//! Three heuristics, each a published method (Groover, *Automation, Production Systems, and
//! Computer-Integrated Manufacturing*, 4th ed., 2015, ch. 15, §15.3; M. D. Kilbridge and
//! L. Wester, "A heuristic method of assembly line balancing", *Journal of Industrial
//! Engineering* 12(4), 1961; W. B. Helgeson and D. P. Birnie, "Assembly line balancing using the
//! ranked positional weight technique", *Journal of Industrial Engineering* 12(6), 1961):
//!
//! - [`Heuristic::LargestCandidate`]: fill each station with the longest element that still fits
//!   among those whose predecessors are all placed.
//! - [`Heuristic::KilbridgeWester`]: elements sit in columns by precedence level (column 0 has
//!   no predecessors; an element's column is one more than its latest predecessor's); fill each
//!   station from the leftmost column that has an element that fits, top to bottom.
//! - [`Heuristic::RankedPositionalWeight`]: an element's weight is its time plus the times of
//!   every element that must follow it; fill each station with the heaviest element that fits.
//!
//! The heuristics do not promise the fewest stations. The theoretical minimum is the total work
//! content over the cycle time, rounded up (Groover eq. 15.7). The measures are balance
//! efficiency (work content over stations times the longest station, eq. 15.14), balance delay
//! (its complement, eq. 15.15) and the smoothness index, the square root of the sum of squared
//! differences between the longest station's time and each station's (not in Groover; as in
//! Bhattacharjee et al., *Journal of Modelling in Management*, 2018, doi 10.1108/JM2-03-2017-0027).
//!
//! Ties are broken by input order, so the result depends on the order the elements are given in,
//! and always the same for the same input. Times and the cycle time are compared with a relative
//! tolerance of `1e-9`, so a station of times that add up to the cycle time in exact arithmetic
//! is full even when the floating-point sum is a little over it.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use crate::ElementId;

/// The relative tolerance for comparing times with the cycle time, and for the minimum.
const TOLERANCE: f64 = 1e-9;

/// A work element of an assembly: its time, and the elements that must be done before it.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkElement {
    pub id: ElementId,
    /// The time to do the element, in the same unit as the cycle time. Positive and finite.
    pub time: f64,
    /// The elements that must be done before this one. Repeats are ignored.
    pub predecessors: Vec<ElementId>,
}

/// One of the three heuristics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heuristic {
    LargestCandidate,
    KilbridgeWester,
    RankedPositionalWeight,
}

impl Heuristic {
    /// Every heuristic, in the order a [`Comparison`] lists them.
    pub const ALL: [Heuristic; 3] = [
        Heuristic::LargestCandidate,
        Heuristic::KilbridgeWester,
        Heuristic::RankedPositionalWeight,
    ];

    /// The name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Heuristic::LargestCandidate => "largest_candidate",
            Heuristic::KilbridgeWester => "kilbridge_wester",
            Heuristic::RankedPositionalWeight => "ranked_positional_weight",
        }
    }
}

/// One station: its elements in the order they were put on it, and their total time.
#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    pub elements: Vec<ElementId>,
    pub time: f64,
}

/// A balance from one heuristic, with its measures.
#[derive(Debug, Clone, PartialEq)]
pub struct Balance {
    pub heuristic: Heuristic,
    pub stations: Vec<Station>,
    /// The longest station's time.
    pub longest: f64,
    /// Work content over stations times the longest station's time.
    pub efficiency: f64,
    /// One minus the efficiency.
    pub delay: f64,
    /// The square root of the sum of squared differences from the longest station's time.
    pub smoothness: f64,
}

/// Every heuristic's balance for one set of elements, with the theoretical minimum beside them.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub cycle: f64,
    /// The total work content.
    pub work_content: f64,
    /// The least number of stations any balance can have.
    pub theoretical_min: usize,
    /// One balance per heuristic, in the order of [`Heuristic::ALL`].
    pub balances: Vec<Balance>,
}

/// Why the elements or the cycle time cannot be balanced.
#[derive(Debug, Clone, PartialEq)]
pub enum BalanceError {
    NoElements,
    /// The cycle time is not a positive, finite number.
    CycleNotPositive(f64),
    DuplicateElement(ElementId),
    /// The time is not a positive, finite number.
    BadTime {
        element: ElementId,
        time: f64,
    },
    UnknownPredecessor {
        element: ElementId,
        predecessor: ElementId,
    },
    /// The element takes longer than the cycle time, so no station can hold it.
    TooLong {
        element: ElementId,
        time: f64,
        cycle: f64,
    },
    /// The element is on a loop of precedences, or after one, so the order cannot be found.
    PrecedenceLoop(ElementId),
}

impl fmt::Display for BalanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BalanceError::NoElements => f.write_str("there are no work elements to balance"),
            BalanceError::CycleNotPositive(c) => {
                write!(f, "the cycle time must be positive and finite, not {c}")
            }
            BalanceError::DuplicateElement(id) => write!(f, "work element {id} is listed twice"),
            BalanceError::BadTime { element, time } => write!(
                f,
                "work element {element} has time {time}; a time must be positive and finite"
            ),
            BalanceError::UnknownPredecessor {
                element,
                predecessor,
            } => write!(
                f,
                "work element {element} comes after {predecessor}, which is not a work element"
            ),
            BalanceError::TooLong {
                element,
                time,
                cycle,
            } => write!(
                f,
                "work element {element} takes {time}, longer than the cycle time {cycle}"
            ),
            BalanceError::PrecedenceLoop(id) => write!(
                f,
                "the precedences loop through work element {id} or lead into one, so no order exists"
            ),
        }
    }
}

impl std::error::Error for BalanceError {}

/// Balances the elements for a cycle time, with one heuristic.
pub fn balance(
    elements: &[WorkElement],
    cycle: f64,
    heuristic: Heuristic,
) -> Result<Balance, BalanceError> {
    Ok(Line::new(elements, cycle)?.balance(heuristic))
}

/// Balances the elements with every heuristic, and gives the theoretical minimum.
pub fn compare(elements: &[WorkElement], cycle: f64) -> Result<Comparison, BalanceError> {
    let line = Line::new(elements, cycle)?;
    Ok(Comparison {
        cycle,
        work_content: line.work,
        theoretical_min: line.min_stations(),
        balances: Heuristic::ALL.iter().map(|&h| line.balance(h)).collect(),
    })
}

/// Each element's ranked positional weight, in the order given (Helgeson and Birnie): its time
/// plus the times of every element that must come after it, each counted once.
pub fn positional_weights(elements: &[WorkElement]) -> Result<Vec<(ElementId, f64)>, BalanceError> {
    let graph = Graph::new(elements)?;
    let weights = graph.weights();
    Ok(elements
        .iter()
        .zip(weights)
        .map(|(e, w)| (e.id.clone(), w))
        .collect())
}

/// The elements checked, indexed by their position in the input.
struct Graph<'a> {
    elements: &'a [WorkElement],
    times: Vec<f64>,
    preds: Vec<Vec<usize>>,
    succs: Vec<Vec<usize>>,
    /// The elements in an order where every predecessor comes first.
    topo: Vec<usize>,
}

impl<'a> Graph<'a> {
    fn new(elements: &'a [WorkElement]) -> Result<Self, BalanceError> {
        if elements.is_empty() {
            return Err(BalanceError::NoElements);
        }
        let mut index = BTreeMap::new();
        for (i, e) in elements.iter().enumerate() {
            if index.insert(e.id.clone(), i).is_some() {
                return Err(BalanceError::DuplicateElement(e.id.clone()));
            }
            if !(e.time.is_finite() && e.time > 0.0) {
                return Err(BalanceError::BadTime {
                    element: e.id.clone(),
                    time: e.time,
                });
            }
        }
        let n = elements.len();
        let mut preds = vec![Vec::new(); n];
        let mut succs = vec![Vec::new(); n];
        for (i, e) in elements.iter().enumerate() {
            for p in &e.predecessors {
                let Some(&j) = index.get(p) else {
                    return Err(BalanceError::UnknownPredecessor {
                        element: e.id.clone(),
                        predecessor: p.clone(),
                    });
                };
                if !preds[i].contains(&j) {
                    preds[i].push(j);
                    succs[j].push(i);
                }
            }
        }
        // Kahn's algorithm: whatever cannot be ordered is on a loop or after one.
        let mut waiting: Vec<usize> = preds.iter().map(Vec::len).collect();
        let mut queue: VecDeque<usize> = (0..n).filter(|&i| waiting[i] == 0).collect();
        let mut topo = Vec::with_capacity(n);
        while let Some(i) = queue.pop_front() {
            topo.push(i);
            for &s in &succs[i] {
                waiting[s] = waiting[s].saturating_sub(1);
                if waiting[s] == 0 {
                    queue.push_back(s);
                }
            }
        }
        if topo.len() < n {
            let stuck = (0..n).find(|&i| waiting[i] > 0).unwrap_or(0);
            return Err(BalanceError::PrecedenceLoop(elements[stuck].id.clone()));
        }
        Ok(Graph {
            elements,
            times: elements.iter().map(|e| e.time).collect(),
            preds,
            succs,
            topo,
        })
    }

    /// Ranked positional weights, by input position.
    fn weights(&self) -> Vec<f64> {
        let n = self.times.len();
        (0..n)
            .map(|i| {
                let mut follows = vec![false; n];
                let mut stack = self.succs[i].clone();
                while let Some(j) = stack.pop() {
                    if !follows[j] {
                        follows[j] = true;
                        stack.extend(self.succs[j].iter().copied());
                    }
                }
                self.times[i]
                    + (0..n)
                        .filter(|&j| follows[j])
                        .map(|j| self.times[j])
                        .sum::<f64>()
            })
            .collect()
    }

    /// Kilbridge and Wester's columns: one more than the latest predecessor's column.
    fn columns(&self) -> Vec<usize> {
        let mut column = vec![0; self.times.len()];
        for &i in &self.topo {
            column[i] = self.preds[i].iter().fold(0, |c, &p| c.max(column[p] + 1));
        }
        column
    }
}

/// A graph with its cycle time, checked: every element fits a station.
struct Line<'a> {
    graph: Graph<'a>,
    cycle: f64,
    work: f64,
}

impl<'a> Line<'a> {
    fn new(elements: &'a [WorkElement], cycle: f64) -> Result<Self, BalanceError> {
        if !(cycle.is_finite() && cycle > 0.0) {
            return Err(BalanceError::CycleNotPositive(cycle));
        }
        let graph = Graph::new(elements)?;
        for e in elements {
            if e.time > cycle * (1.0 + TOLERANCE) {
                return Err(BalanceError::TooLong {
                    element: e.id.clone(),
                    time: e.time,
                    cycle,
                });
            }
        }
        let work = graph.times.iter().sum();
        Ok(Line { graph, cycle, work })
    }

    /// The least number of stations: the work content over the cycle time, rounded up.
    fn min_stations(&self) -> usize {
        let ratio = self.work / self.cycle - TOLERANCE;
        (ratio.ceil() as usize).max(1)
    }

    fn fits(&self, station_time: f64, t: f64) -> bool {
        station_time + t <= self.cycle * (1.0 + TOLERANCE)
    }

    fn balance(&self, heuristic: Heuristic) -> Balance {
        let g = &self.graph;
        let n = g.times.len();
        let preference = match heuristic {
            Heuristic::LargestCandidate => {
                let mut order: Vec<usize> = (0..n).collect();
                order.sort_by(|&a, &b| g.times[b].total_cmp(&g.times[a]).then(a.cmp(&b)));
                order
            }
            Heuristic::KilbridgeWester => {
                let column = g.columns();
                let mut order: Vec<usize> = (0..n).collect();
                // By column, the largest first within a column (Groover, Table 15.7).
                order.sort_by(|&a, &b| {
                    column[a]
                        .cmp(&column[b])
                        .then(g.times[b].total_cmp(&g.times[a]))
                        .then(a.cmp(&b))
                });
                order
            }
            Heuristic::RankedPositionalWeight => {
                let weight = g.weights();
                let mut order: Vec<usize> = (0..n).collect();
                order.sort_by(|&a, &b| weight[b].total_cmp(&weight[a]).then(a.cmp(&b)));
                order
            }
        };
        let stations = self.fill(&preference);
        measure(heuristic, stations, self.work)
    }

    /// Opens a station and puts on it the first element in `preference` whose predecessors are
    /// all placed and that fits, until none does; then opens the next.
    fn fill(&self, preference: &[usize]) -> Vec<Station> {
        let g = &self.graph;
        let mut placed = vec![false; g.times.len()];
        let mut left = g.times.len();
        let mut stations = Vec::new();
        while left > 0 {
            let mut members = Vec::new();
            let mut time = 0.0;
            loop {
                let pick = preference.iter().copied().find(|&i| {
                    !placed[i]
                        && g.preds[i].iter().all(|&p| placed[p])
                        && self.fits(time, g.times[i])
                });
                let Some(i) = pick else { break };
                placed[i] = true;
                left -= 1;
                time += g.times[i];
                members.push(i);
            }
            // Every element fits an empty station and the placed set always holds the
            // predecessors of some unplaced element, so a station is never left empty.
            if members.is_empty() {
                break;
            }
            stations.push(Station {
                elements: members.iter().map(|&i| g.elements[i].id.clone()).collect(),
                time,
            });
        }
        stations
    }
}

fn measure(heuristic: Heuristic, stations: Vec<Station>, work: f64) -> Balance {
    let longest = stations.iter().map(|s| s.time).fold(0.0, f64::max);
    let count = stations.len() as f64;
    let efficiency = work / (count * longest);
    let smoothness = stations
        .iter()
        .map(|s| (longest - s.time) * (longest - s.time))
        .sum::<f64>()
        .sqrt();
    Balance {
        heuristic,
        stations,
        longest,
        efficiency,
        delay: 1.0 - efficiency,
        smoothness,
    }
}

#[cfg(test)]
mod tests;
