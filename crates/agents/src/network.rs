//! Networks and grids over a population (ROADMAP M4.3, stage A3).
//!
//! **Networks.** A [`Network`] links the agents of one population: links are undirected, never
//! from an agent to itself, and never two between the same pair. It is built when the run starts,
//! over the agents created then (the population's count, serials 0 to n − 1), from the
//! population's own stream `network/<n>:<population>` (`<n>` the name's length in bytes), so
//! another population, or another population's agents, never change it (D18):
//!
//! - [`Network::random`]: Erdős and Rényi's G(n, M), M = round(n·d / 2) links for a mean degree
//!   d, every set of M distinct pairs equally likely (drawn by Floyd's algorithm over the
//!   n(n − 1)/2 pairs). The mean degree is 2M / n, exactly d when n·d / 2 is whole. Every whole
//!   number the generators and grids draw below a bound is unbiased (64 random bits with
//!   rejection, Lemire 2019).
//! - [`Network::ring`]: a ring lattice, each agent linked to the k nearest on each side (degree
//!   2k; 2k < n).
//! - [`Network::small_world`]: Watts and Strogatz (1998): the ring lattice, then each link
//!   rewired with probability p, going round the ring once for the links to the nearest
//!   neighbour clockwise, then again for the second nearest, and so on: a link from i to its
//!   d-th neighbour clockwise is moved to a link from i to an agent chosen uniformly from those
//!   that are not i and not linked to i already (left as it is if there is none). The number of
//!   links stays n·k; with p = 0 it is the lattice.
//! - [`Network::scale_free`]: Barabási and Albert (1999): a complete network of m0 agents, then
//!   each next agent linked to m distinct earlier ones, each chosen with probability
//!   proportional to its degree (m0 ≥ m, m0 ≥ 2, m ≥ 1). Every agent from m0 on has degree m or
//!   more, and so do the first m0 once there are more than m0.
//! - [`Network::distance`]: positions placed uniformly at random in a rectangle of the given
//!   width and height, held in the agents' variables `x` and `y`, and a link between every two
//!   agents whose distance (√(Δx² + Δy²), worked out by [`distance`] so that it neither
//!   underflows nor overflows) is at most the range; every pair is compared. The variables are
//!   the model's: a chart may not assign them, nor `row` and `column` on a grid.
//! - [`Network::links`] and [`Network::csv`]: a list of links, as pairs of serial numbers or
//!   as the lines `a,b` of a CSV text ([`read_links`], [`write_links`]).
//! - [`Network::empty`]: no links, for links made during the run.
//!
//! Agents created later start with no links, but in a distance network, where each gets a
//! position from the network's stream and a link to every agent alive within the range. An
//! agent that leaves its population loses its links. During the run, [`Action::connect`] and
//! [`Action::disconnect`](crate::Action::disconnect) add and remove links.
//!
//! **Grids.** A [`Grid`] is rows × columns cells (numbered row by row from 0), with Moore (the
//! eight around) or von Neumann (the four beside) neighbours, wrapping at the edges or not, and
//! one agent per cell or any number. Each agent's cell is in its variables `row` and `column`
//! (from 0; −1 for an agent with no cell). The agents created at the start are placed in order
//! (agent s in cell s, or cell s mod the number of cells when many share a cell) or at random
//! (a free cell uniformly, or any cell when many share one) from the stream
//! `grid/<n>:<population>`; one created later goes in order to the first free cell from cell
//! s mod the cells on, or at random as at the start, and if every cell is taken it has none.
//! An agent's neighbours are the agents in its neighbouring cells and the others in its own
//! cell. [`Action::move_to`](crate::Action::move_to) and
//! [`Action::move_random`](crate::Action::move_random) move it.
//!
//! [`Action::connect`]: crate::Action::connect

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stochcraft_random::Rng;

/// How a population's agents are linked at the start (see the [module](self)).
#[derive(Debug, Clone, PartialEq)]
pub enum Network {
    Empty,
    Random { mean_degree: f64 },
    Ring { each_side: u64 },
    SmallWorld { each_side: u64, rewire: f64 },
    ScaleFree { initial: u64, links: u64 },
    Distance { width: f64, height: f64, range: f64 },
    Links(Vec<(u64, u64)>),
    Csv(String),
}

impl Network {
    pub fn empty() -> Self {
        Network::Empty
    }

    /// G(n, M) with M = round(n · `mean_degree` / 2).
    pub fn random(mean_degree: f64) -> Self {
        Network::Random { mean_degree }
    }

    /// Each agent linked to the `each_side` nearest on either side.
    pub fn ring(each_side: u64) -> Self {
        Network::Ring { each_side }
    }

    /// The ring lattice with each link rewired with probability `rewire`.
    pub fn small_world(each_side: u64, rewire: f64) -> Self {
        Network::SmallWorld { each_side, rewire }
    }

    /// A complete network of `initial` agents, then each next one linked to `links` earlier
    /// ones by preferential attachment.
    pub fn scale_free(initial: u64, links: u64) -> Self {
        Network::ScaleFree { initial, links }
    }

    /// Positions in a `width` × `height` rectangle, linked within `range`.
    pub fn distance(width: f64, height: f64, range: f64) -> Self {
        Network::Distance {
            width,
            height,
            range,
        }
    }

    /// These links, as pairs of serial numbers.
    pub fn links(links: Vec<(u64, u64)>) -> Self {
        Network::Links(links)
    }

    /// The links of a CSV text, as [`read_links`] reads it.
    pub fn csv(text: &str) -> Self {
        Network::Csv(text.to_string())
    }
}

/// Which cells are an agent's neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Neighbourhood {
    /// The eight cells around.
    #[default]
    Moore,
    /// The four cells beside: above, below, left and right.
    VonNeumann,
}

/// Where the agents created at the start are put.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    /// Agent s in cell s (row by row), or in cell s mod the cells when many share a cell.
    #[default]
    InOrder,
    /// A free cell at random, or any cell when many share one.
    Random,
}

/// A grid of cells for a population (see the [module](self)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub rows: u64,
    pub columns: u64,
    pub neighbourhood: Neighbourhood,
    /// Whether the edges wrap round (a torus).
    pub wrap: bool,
    /// At most one agent in a cell.
    pub one_per_cell: bool,
    pub placement: Placement,
}

impl Grid {
    /// `rows` × `columns` cells, Moore neighbours, no wrapping, one agent per cell, placed in
    /// order.
    pub fn new(rows: u64, columns: u64) -> Self {
        Self {
            rows,
            columns,
            neighbourhood: Neighbourhood::Moore,
            wrap: false,
            one_per_cell: true,
            placement: Placement::InOrder,
        }
    }

    pub fn von_neumann(mut self) -> Self {
        self.neighbourhood = Neighbourhood::VonNeumann;
        self
    }

    pub fn wrapped(mut self) -> Self {
        self.wrap = true;
        self
    }

    pub fn many_per_cell(mut self) -> Self {
        self.one_per_cell = false;
        self
    }

    pub fn random_placement(mut self) -> Self {
        self.placement = Placement::Random;
        self
    }
}

/// A line of a list of links that cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError {
    /// From 1.
    pub line: u64,
    pub message: String,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for LinkError {}

/// An index below `m` from 64 random bits: the top 53 bits as a fraction of `m`.
pub(crate) fn pick(draw: u64, m: usize) -> usize {
    (((draw >> 11) as u128 * m as u128) >> 53) as usize
}

/// A uniform integer below `m` (0 if `m` is 0), with no bias: Lemire's multiply-and-reject on all
/// 64 bits (D. Lemire, "Fast random integer generation in an interval", ACM TOMACS 29(1), 2019).
pub(crate) fn below(rng: &mut Rng, m: u64) -> u64 {
    if m == 0 {
        return 0;
    }
    let mut x = rng.next_u64() as u128 * m as u128;
    if (x as u64) < m {
        let threshold = m.wrapping_neg() % m;
        while (x as u64) < threshold {
            x = rng.next_u64() as u128 * m as u128;
        }
    }
    (x >> 64) as u64
}

/// The pair (i, j), i < j, of index k in the order (0, 1), (0, 2), (1, 2), (0, 3), ...
fn pair(k: u64) -> (u64, u64) {
    let tri = |j: u64| (j as u128 * j.saturating_sub(1) as u128 / 2) as u64;
    let mut j = ((1.0 + (1.0 + 8.0 * k as f64).sqrt()) / 2.0) as u64;
    while j > 1 && tri(j) > k {
        j -= 1;
    }
    while tri(j + 1) <= k {
        j += 1;
    }
    (k - tri(j), j)
}

/// Reads a list of links: one `a,b` per line, two serial numbers of agents below `agents`, with
/// spaces around them allowed. Blank lines and lines starting with `#` are skipped, and a first
/// line whose two fields are not numbers is a header. A link from an agent to itself, a link
/// given twice (either way round), or a serial of no agent is refused with its line.
pub fn read_links(text: &str, agents: u64) -> Result<Vec<(u64, u64)>, LinkError> {
    let mut links = Vec::new();
    let mut seen = BTreeMap::new();
    let mut first = true;
    for (i, line) in text.lines().enumerate() {
        let number = i as u64 + 1;
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let fail = |message: String| {
            Err(LinkError {
                line: number,
                message,
            })
        };
        let fields: Vec<&str> = t.split(',').map(str::trim).collect();
        let header = first;
        first = false;
        let [a, b] = fields.as_slice() else {
            return fail(format!(
                "`{t}` is not two serial numbers separated by a comma"
            ));
        };
        let (pa, pb) = (a.parse::<u64>(), b.parse::<u64>());
        let (a, b) = match (pa, pb) {
            (Ok(a), Ok(b)) => (a, b),
            _ if header && a.parse::<f64>().is_err() && b.parse::<f64>().is_err() => continue,
            _ => {
                return fail(format!(
                    "`{t}` is not two serial numbers (whole numbers of 0 or more)"
                ));
            }
        };
        if a == b {
            return fail(format!("it links agent {a} to itself"));
        }
        for s in [a, b] {
            if s >= agents {
                return fail(format!(
                    "there is no agent {s}: the population starts with {agents}"
                ));
            }
        }
        let key = (a.min(b), a.max(b));
        if let Some(earlier) = seen.insert(key, number) {
            return fail(format!(
                "agents {a} and {b} are linked already, on line {earlier}"
            ));
        }
        links.push((a, b));
    }
    Ok(links)
}

/// Writes links as [`read_links`] reads them: the header `from,to`, then one `a,b` per line.
pub fn write_links(links: &[(u64, u64)]) -> String {
    let mut s = String::from("from,to\n");
    for (a, b) in links {
        s.push_str(&format!("{a},{b}\n"));
    }
    s
}

/// The network at the start: its links (each pair once, the lower serial first, sorted) and, for
/// a distance network, each agent's position.
#[derive(Debug, Clone, Default)]
pub(crate) struct Start {
    pub links: Vec<(u64, u64)>,
    pub positions: Vec<(f64, f64)>,
}

fn norm(a: u64, b: u64) -> (u64, u64) {
    (a.min(b), a.max(b))
}

/// Builds `network` over agents 0 to `n` − 1 from `rng`; an error says what is wrong.
pub(crate) fn build(network: &Network, n: u64, rng: &mut Rng) -> Result<Start, String> {
    let mut start = Start::default();
    match network {
        Network::Empty => {}
        Network::Random { mean_degree } => {
            let d = *mean_degree;
            if !(d.is_finite() && d >= 0.0) {
                return Err(format!("the mean degree {d} is not a number of 0 or more"));
            }
            let pairs = n as u128 * n.saturating_sub(1) as u128 / 2;
            let m = (n as f64 * d / 2.0).round();
            if m > pairs as f64 || pairs > u64::MAX as u128 {
                return Err(format!(
                    "a mean degree of {d} needs {m} links, but {n} agents have only {pairs} pairs"
                ));
            }
            let (pairs, m) = (pairs as u64, m as u64);
            // Floyd's algorithm: M distinct indices out of the pairs, each set equally likely.
            let mut chosen = BTreeSet::new();
            for j in (pairs - m)..pairs {
                let t = below(rng, j + 1);
                if !chosen.insert(t) {
                    chosen.insert(j);
                }
            }
            start.links = chosen.into_iter().map(pair).collect();
        }
        Network::Ring { each_side } => {
            start.links = ring(n, *each_side)?;
        }
        Network::SmallWorld { each_side, rewire } => {
            let p = *rewire;
            if !(0.0..=1.0).contains(&p) {
                return Err(format!("the rewiring probability {p} is not from 0 to 1"));
            }
            let k = *each_side;
            let lattice = ring(n, k)?;
            let mut adj: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
            for &(a, b) in &lattice {
                adj.entry(a).or_default().insert(b);
                adj.entry(b).or_default().insert(a);
            }
            for d in 1..=k {
                for i in 0..n {
                    let u = rng.uniform();
                    if u >= p {
                        continue;
                    }
                    let j = (i + d) % n;
                    let degree = adj.get(&i).map_or(0, BTreeSet::len) as u64;
                    if degree + 1 >= n {
                        continue;
                    }
                    let w = loop {
                        let w = below(rng, n);
                        if w != i && !adj.get(&i).is_some_and(|s| s.contains(&w)) {
                            break w;
                        }
                    };
                    for (x, y) in [(i, j), (j, i)] {
                        if let Some(s) = adj.get_mut(&x) {
                            s.remove(&y);
                        }
                    }
                    adj.entry(i).or_default().insert(w);
                    adj.entry(w).or_default().insert(i);
                }
            }
            start.links = adj
                .iter()
                .flat_map(|(&a, s)| s.range(a + 1..).map(move |&b| (a, b)))
                .collect();
        }
        Network::ScaleFree { initial, links } => {
            let (m0, m) = (*initial, *links);
            if m == 0 || m0 < m || m0 < 2 {
                return Err(format!(
                    "preferential attachment needs at least 1 link per agent and a first \
                     complete network of at least 2 agents and at least as many as the links \
                     per agent; it has {m0} agents and {m} links"
                ));
            }
            // Each agent appears in `ends` once per link it has, so a uniform pick from it
            // picks an agent with probability proportional to its degree.
            let mut ends: Vec<u64> = Vec::new();
            let first = m0.min(n);
            for a in 0..first {
                for b in a + 1..first {
                    start.links.push((a, b));
                    ends.push(a);
                    ends.push(b);
                }
            }
            for t in first..n {
                let mut chosen = BTreeSet::new();
                while (chosen.len() as u64) < m {
                    let k = below(rng, ends.len() as u64) as usize;
                    if let Some(&c) = ends.get(k) {
                        chosen.insert(c);
                    }
                }
                for c in chosen {
                    start.links.push((c, t));
                    ends.push(c);
                    ends.push(t);
                }
            }
            start.links.sort_unstable();
        }
        Network::Distance {
            width,
            height,
            range,
        } => {
            let (w, h, r) = (*width, *height, *range);
            if !(w.is_finite() && w > 0.0 && h.is_finite() && h > 0.0) {
                return Err(format!("the rectangle {w} × {h} is not above 0 each way"));
            }
            if !(r.is_finite() && r >= 0.0) {
                return Err(format!("the range {r} is not a number of 0 or more"));
            }
            for _ in 0..n {
                let x = rng.uniform() * w;
                let y = rng.uniform() * h;
                start.positions.push((x, y));
            }
            for (a, &pa) in start.positions.iter().enumerate() {
                for (b, &pb) in start.positions.iter().enumerate().skip(a + 1) {
                    if within(pa, pb, r) {
                        start.links.push((a as u64, b as u64));
                    }
                }
            }
        }
        Network::Links(list) => {
            let mut seen = BTreeSet::new();
            for (k, &(a, b)) in list.iter().enumerate() {
                let place = format!("link {} ({a}, {b})", k + 1);
                if a == b {
                    return Err(format!("{place} links an agent to itself"));
                }
                if a >= n || b >= n {
                    return Err(format!(
                        "{place} names an agent the population does not start with ({n})"
                    ));
                }
                if !seen.insert(norm(a, b)) {
                    return Err(format!("{place} is given twice"));
                }
            }
            start.links = seen.into_iter().collect();
        }
        Network::Csv(text) => {
            let list = read_links(text, n).map_err(|e| e.to_string())?;
            let mut links: Vec<(u64, u64)> = list.into_iter().map(|(a, b)| norm(a, b)).collect();
            links.sort_unstable();
            start.links = links;
        }
    }
    Ok(start)
}

/// Whether two positions are within `range` of each other.
pub(crate) fn within(a: (f64, f64), b: (f64, f64), range: f64) -> bool {
    distance(a, b) <= range
}

/// The distance between two points, scaled by the larger difference so that squaring neither
/// underflows nor overflows: max · √(1 + (min / max)²). Only division, multiplication and the
/// square root (all exactly rounded everywhere) are used, so it is the same on every target,
/// which the platform's `hypot` does not promise.
pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
    let (big, small) = if dx >= dy { (dx, dy) } else { (dy, dx) };
    if big == 0.0 || !big.is_finite() {
        return big;
    }
    let t = small / big;
    big * (1.0 + t * t).sqrt()
}

fn ring(n: u64, k: u64) -> Result<Vec<(u64, u64)>, String> {
    if k > 0 && k.saturating_mul(2) >= n {
        return Err(format!(
            "a ring of {n} agents cannot link each to {k} on each side (it needs more than {})",
            k.saturating_mul(2)
        ));
    }
    let mut links: Vec<(u64, u64)> = (0..n)
        .flat_map(|i| (1..=k).map(move |d| norm(i, (i + d) % n)))
        .collect();
    links.sort_unstable();
    Ok(links)
}

/// A population's links while it runs.
#[derive(Debug, Clone)]
pub(crate) struct Net {
    pub adj: BTreeMap<u64, BTreeSet<u64>>,
    pub positions: BTreeMap<u64, (f64, f64)>,
    /// A distance network's range.
    pub range: Option<f64>,
    pub size: Option<(f64, f64)>,
    pub rng: Rng,
}

impl Net {
    pub fn new(network: &Network, n: u64, mut rng: Rng) -> Result<Net, String> {
        let start = build(network, n, &mut rng)?;
        let mut net = Net {
            adj: BTreeMap::new(),
            positions: start
                .positions
                .iter()
                .enumerate()
                .map(|(s, &p)| (s as u64, p))
                .collect(),
            range: None,
            size: None,
            rng,
        };
        if let Network::Distance {
            width,
            height,
            range,
        } = network
        {
            net.range = Some(*range);
            net.size = Some((*width, *height));
        }
        for (a, b) in start.links {
            net.connect(a, b);
        }
        Ok(net)
    }

    /// Links `a` and `b`; false if they are the same agent or linked already.
    pub fn connect(&mut self, a: u64, b: u64) -> bool {
        if a == b {
            return false;
        }
        let added = self.adj.entry(a).or_default().insert(b);
        self.adj.entry(b).or_default().insert(a);
        added
    }

    /// Unlinks `a` and `b`; false if they were not linked.
    pub fn disconnect(&mut self, a: u64, b: u64) -> bool {
        let removed = self.adj.get_mut(&a).is_some_and(|s| s.remove(&b));
        if let Some(s) = self.adj.get_mut(&b) {
            s.remove(&a);
        }
        removed
    }

    /// The agents linked to `s`, ascending.
    pub fn linked(&self, s: u64) -> Vec<u64> {
        self.adj
            .get(&s)
            .map(|x| x.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Takes away every link of `s`, and its position.
    pub fn forget(&mut self, s: u64) {
        if let Some(others) = self.adj.remove(&s) {
            for o in others {
                if let Some(x) = self.adj.get_mut(&o) {
                    x.remove(&s);
                }
            }
        }
        self.positions.remove(&s);
    }

    /// Every link, the lower serial first, sorted.
    pub fn links(&self) -> Vec<(u64, u64)> {
        self.adj
            .iter()
            .flat_map(|(&a, s)| s.range(a + 1..).map(move |&b| (a, b)))
            .collect()
    }
}

/// A population's grid while it runs: which agents are in each cell.
#[derive(Debug, Clone)]
pub(crate) struct Space {
    pub grid: Grid,
    cells: u64,
    /// The agents in each occupied cell, ascending.
    occupants: BTreeMap<u64, BTreeSet<u64>>,
    at: BTreeMap<u64, u64>,
    pub rng: Rng,
}

impl Space {
    pub fn new(grid: Grid, count: u64, rng: Rng) -> Result<Space, String> {
        let Some(cells) = grid.rows.checked_mul(grid.columns).filter(|&c| c > 0) else {
            return Err(format!(
                "a grid of {} × {} cells is not at least 1 × 1 (or is too large)",
                grid.rows, grid.columns
            ));
        };
        if grid.one_per_cell && count > cells {
            return Err(format!(
                "{count} agents at one per cell do not fit in {cells} cells"
            ));
        }
        Ok(Space {
            grid,
            cells,
            occupants: BTreeMap::new(),
            at: BTreeMap::new(),
            rng,
        })
    }

    fn is_free(&self, cell: u64) -> bool {
        !self.grid.one_per_cell || self.occupants.get(&cell).is_none_or(BTreeSet::is_empty)
    }

    /// Chooses the cell of a new agent `s` (none if every cell is taken).
    pub fn choose(&mut self, s: u64) -> Option<u64> {
        match self.grid.placement {
            Placement::InOrder => {
                // Only occupied cells are passed over, so this costs at most one step per
                // agent, not per cell.
                let mut c = s % self.cells;
                for _ in 0..=self.occupants.len() {
                    if self.is_free(c) {
                        return Some(c);
                    }
                    c = (c + 1) % self.cells;
                }
                None
            }
            Placement::Random if self.grid.one_per_cell => {
                // `occupants` holds only occupied cells (an emptied one is taken out).
                let free = self.cells - self.occupants.len() as u64;
                if free == 0 {
                    return None;
                }
                // The k-th free cell in order: k, moved on past each occupied cell at or below
                // it, in ascending order.
                let mut c = below(&mut self.rng, free);
                for &taken in self.occupants.keys() {
                    if taken <= c {
                        c += 1;
                    } else {
                        break;
                    }
                }
                Some(c)
            }
            Placement::Random => Some(below(&mut self.rng, self.cells)),
        }
    }

    pub fn place(&mut self, s: u64, cell: u64) {
        self.remove(s);
        self.occupants.entry(cell).or_default().insert(s);
        self.at.insert(s, cell);
    }

    pub fn remove(&mut self, s: u64) {
        if let Some(c) = self.at.remove(&s)
            && let Some(o) = self.occupants.get_mut(&c)
        {
            o.remove(&s);
            if o.is_empty() {
                self.occupants.remove(&c);
            }
        }
    }

    pub fn cell_of(&self, s: u64) -> Option<u64> {
        self.at.get(&s).copied()
    }

    /// (row, column) of a cell.
    pub fn row_column(&self, cell: u64) -> (u64, u64) {
        (cell / self.grid.columns, cell % self.grid.columns)
    }

    /// The cell at `row`, `column` (wrapped if the grid wraps), if there is one.
    pub fn cell_at(&self, row: f64, column: f64) -> Option<u64> {
        // A whole number an i128 holds exactly (every whole f64 below 2^127 in size does);
        // the dimensions stay integers, never rounded to f64.
        let whole =
            |v: f64| (v.is_finite() && v.fract() == 0.0 && v.abs() < 1.7e38).then_some(v as i128);
        let (r, c) = (whole(row)?, whole(column)?);
        let (rows, cols) = (self.grid.rows as i128, self.grid.columns as i128);
        let (r, c) = if self.grid.wrap {
            (r.rem_euclid(rows), c.rem_euclid(cols))
        } else {
            (r, c)
        };
        if !(0..rows).contains(&r) || !(0..cols).contains(&c) {
            return None;
        }
        u64::try_from(r * cols + c).ok()
    }

    /// The cells next to `cell`, ascending, without `cell` itself.
    pub fn neighbour_cells(&self, cell: u64) -> Vec<u64> {
        let (rows, cols) = (self.grid.rows as i128, self.grid.columns as i128);
        let (r, c) = (cell as i128 / cols, cell as i128 % cols);
        let offsets: &[(i128, i128)] = match self.grid.neighbourhood {
            Neighbourhood::Moore => &[
                (-1, -1),
                (-1, 0),
                (-1, 1),
                (0, -1),
                (0, 1),
                (1, -1),
                (1, 0),
                (1, 1),
            ],
            Neighbourhood::VonNeumann => &[(-1, 0), (0, -1), (0, 1), (1, 0)],
        };
        let mut out: Vec<u64> = offsets
            .iter()
            .filter_map(|&(dr, dc)| {
                let (mut nr, mut nc) = (r + dr, c + dc);
                if self.grid.wrap {
                    nr = nr.rem_euclid(rows);
                    nc = nc.rem_euclid(cols);
                } else if nr < 0 || nr >= rows || nc < 0 || nc >= cols {
                    return None;
                }
                Some((nr * cols + nc) as u64)
            })
            .filter(|&x| x != cell)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Whether `cell` can take another agent.
    pub fn can_enter(&self, cell: u64) -> bool {
        self.is_free(cell)
    }

    /// The agents in `s`'s neighbouring cells and the others in its own, ascending.
    pub fn neighbours(&self, s: u64) -> Vec<u64> {
        let Some(cell) = self.cell_of(s) else {
            return Vec::new();
        };
        let mut out: Vec<u64> = std::iter::once(cell)
            .chain(self.neighbour_cells(cell))
            .filter_map(|c| self.occupants.get(&c))
            .flat_map(|o| o.iter().copied())
            .filter(|&x| x != s)
            .collect();
        out.sort_unstable();
        out
    }
}

#[cfg(test)]
#[allow(clippy::panic)] // A grid that cannot be made fails the test.
mod tests {
    use super::*;

    #[test]
    fn pairs_are_numbered_in_order() {
        let mut k = 0;
        for j in 1..60u64 {
            for i in 0..j {
                assert_eq!(pair(k), (i, j), "index {k}");
                k += 1;
            }
        }
    }

    #[test]
    fn grids_find_their_neighbours() {
        let rng = Rng::new(1);
        let s = |g: Grid| match Space::new(g, 0, rng.clone()) {
            Ok(s) => s,
            Err(e) => panic!("{e}"),
        };
        let moore = s(Grid::new(3, 4));
        assert_eq!(moore.neighbour_cells(0), [1, 4, 5]);
        assert_eq!(moore.neighbour_cells(5), [0, 1, 2, 4, 6, 8, 9, 10]);
        let vn = s(Grid::new(3, 4).von_neumann().wrapped());
        assert_eq!(vn.neighbour_cells(0), [1, 3, 4, 8]);
        // On a wrapped grid of two columns, left and right are one cell.
        let thin = s(Grid::new(3, 2).von_neumann().wrapped());
        assert_eq!(thin.neighbour_cells(0), [1, 2, 4]);
        assert_eq!(moore.cell_at(2.0, 3.0), Some(11));
        assert_eq!(moore.cell_at(3.0, 0.0), None);
        assert_eq!(vn.cell_at(-1.0, 4.0), Some(8));
    }

    #[test]
    fn link_lists_are_read_with_their_line_numbers() {
        assert_eq!(
            read_links("from,to\n0,1\n\n# note\n 2 , 0\n", 3),
            Ok(vec![(0, 1), (2, 0)])
        );
        let line = |t: &str| read_links(t, 3).map_err(|e| e.line);
        assert_eq!(line("0,1\n1,1\n"), Err(2));
        assert_eq!(line("0,1\n2,1\n1,0\n"), Err(3));
        assert_eq!(line("0,1\n0,3\n"), Err(2));
        assert_eq!(line("0,1\n0;2\n"), Err(2));
        assert_eq!(line("a,b\nx,1\n"), Err(2));
        assert_eq!(line("0,-1\n"), Err(1));
    }

    /// The bounded draw uses all 64 bits: below 2^60 the old 53-bit fraction could only give
    /// multiples of 2^7. Small bounds come out evenly.
    #[test]
    fn bounded_draws_are_full_width_and_even() {
        let mut rng = Rng::new(9);
        let big = 1u64 << 60;
        let draws: Vec<u64> = (0..64).map(|_| below(&mut rng, big)).collect();
        assert!(draws.iter().all(|&d| d < big));
        assert!(draws.iter().any(|&d| d % 128 != 0), "{draws:?}");
        let mut counts = [0u32; 3];
        for _ in 0..30_000 {
            counts[below(&mut rng, 3) as usize] += 1;
        }
        // Binomial(30,000, 1/3): standard deviation about 82.
        for c in counts {
            assert!((f64::from(c) - 10_000.0).abs() < 400.0, "{counts:?}");
        }
        assert_eq!(below(&mut rng, 1), 0);
        assert_eq!(below(&mut rng, 0), 0);
    }

    /// Distances neither underflow nor overflow, and are exact where they can be.
    #[test]
    fn distances_are_scaled() {
        assert_eq!(distance((0.0, 0.0), (3.0, 4.0)), 5.0);
        assert!(within((0.0, 0.0), (3.0, 4.0), 5.0));
        assert_eq!(distance((1.0, 1.0), (1.0, 1.0)), 0.0);
        let tiny = distance((0.0, 0.0), (1e-200, 1e-200));
        assert!(tiny > 1.4e-200 && tiny < 1.5e-200, "{tiny}");
        assert!(!within((0.0, 0.0), (1e-200, 0.0), 0.0));
        let huge = distance((0.0, 0.0), (1e200, 1e200));
        assert!(
            huge.is_finite() && huge > 1.4e200 && huge < 1.5e200,
            "{huge}"
        );
        assert!(within((0.0, 0.0), (1e200, 1e200), 2e200));
    }
}
