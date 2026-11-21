//! The diagram of a model: where each stock, flow and auxiliary is drawn, the pipe of each
//! flow, and the information links between variables (XMILE 1.0's `<views>`, section 6.1).
//!
//! Positions are kept by canonical variable name, as the centre of the shape, in a plane with
//! y growing downwards. A stock and an auxiliary have a size; a flow's position is its valve,
//! which lies on its pipe, a list of points joined at right angles. A flow with no stock at one
//! end has a cloud there: the cloud is the pipe's end point, so it is not stored apart.
//!
//! A model read from a file with no diagram gets one from [`Layout::auto`], a deterministic
//! layout: stocks in rows along their chains of flows, an auxiliary above the first variable
//! it feeds, and what is left in a row below.

use std::collections::{BTreeMap, BTreeSet};

use crate::expr::{Ast, parse};
use crate::model::{SdModel, VarKind, Variable, canonical};

/// The size of a stock when the file gives none.
pub const STOCK_SIZE: (f64, f64) = (60.0, 40.0);
/// The size of an auxiliary when the file gives none.
pub const AUX_SIZE: (f64, f64) = (20.0, 20.0);
/// The side of a flow's valve, which a flow does not state.
pub const VALVE_SIZE: f64 = 20.0;

/// Distance between columns of stocks, and between rows.
const COLUMN: f64 = 220.0;
const ROW: f64 = 200.0;
const ORIGIN: (f64, f64) = (150.0, 200.0);
const EPS: f64 = 1e-6;

/// One shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// The centre (a flow's valve).
    pub x: f64,
    pub y: f64,
    /// A stock's or auxiliary's size; `None` is the default for its kind. Always `None` for a flow.
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// A flow's pipe, from its source end to its sink end; empty for other variables.
    pub pts: Vec<(f64, f64)>,
}

impl Shape {
    fn at(x: f64, y: f64) -> Self {
        Self {
            x,
            y,
            width: None,
            height: None,
            pts: Vec::new(),
        }
    }

    /// The width and height of the shape, which the kind of variable gives when the shape does not.
    pub fn size(&self, kind: &VarKind) -> (f64, f64) {
        let default = match kind {
            VarKind::Stock { .. } => STOCK_SIZE,
            VarKind::Aux => AUX_SIZE,
            VarKind::Flow { .. } => (VALVE_SIZE, VALVE_SIZE),
        };
        match (kind, self.width, self.height) {
            (VarKind::Flow { .. }, _, _) => default,
            (_, Some(w), Some(h)) => (w, h),
            _ => default,
        }
    }
}

/// An information link: the variable `from` is used in the equation of `to`.
#[derive(Debug, Clone, PartialEq)]
pub struct Connector {
    /// Canonical names.
    pub from: String,
    pub to: String,
    /// The angle in degrees at which the link leaves `from`, if it is bent.
    pub angle: Option<f64>,
}

/// The diagram.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// By canonical variable name.
    pub shapes: BTreeMap<String, Shape>,
    pub connectors: Vec<Connector>,
}

/// The names (canonical) an equation uses, including the functions it calls, since a graphical
/// function is called by its name.
pub(crate) fn references(eqn: &str) -> BTreeSet<String> {
    fn walk(a: &Ast, out: &mut BTreeSet<String>) {
        match a {
            Ast::Num(_) => {}
            Ast::Name(n) => {
                out.insert(n.clone());
            }
            Ast::Neg(x) | Ast::Not(x) => walk(x, out),
            Ast::Bin(_, x, y) => {
                walk(x, out);
                walk(y, out);
            }
            Ast::If(c, x, y) => {
                walk(c, out);
                walk(x, out);
                walk(y, out);
            }
            Ast::Call(n, args) => {
                out.insert(n.clone());
                for x in args {
                    walk(x, out);
                }
            }
        }
    }
    let mut out = BTreeSet::new();
    if let Ok(ast) = parse(eqn) {
        walk(&ast, &mut out);
    }
    out
}

/// The canonical names of the variables of `model` an equation of `v` uses (not `v` itself).
fn used_by(model: &SdModel, v: &Variable) -> BTreeSet<String> {
    let names: BTreeSet<String> = model.vars.iter().map(|x| canonical(&x.name)).collect();
    let me = canonical(&v.name);
    v.eqn
        .as_deref()
        .map(references)
        .unwrap_or_default()
        .into_iter()
        .filter(|n| *n != me && names.contains(n))
        .collect()
}

/// The stocks a flow leaves and enters (canonical names; the first, by name, of each).
pub(crate) fn ends(model: &SdModel, flow: &str) -> (Option<String>, Option<String>) {
    let mut src = None;
    let mut dst = None;
    let mut stocks: Vec<(String, &VarKind)> = model
        .vars
        .iter()
        .filter(|v| matches!(v.kind, VarKind::Stock { .. }))
        .map(|v| (canonical(&v.name), &v.kind))
        .collect();
    stocks.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, kind) in stocks {
        if let VarKind::Stock {
            inflows, outflows, ..
        } = kind
        {
            if src.is_none() && outflows.iter().any(|f| canonical(f) == flow) {
                src = Some(name.clone());
            }
            if dst.is_none() && inflows.iter().any(|f| canonical(f) == flow) {
                dst = Some(name);
            }
        }
    }
    (src, dst)
}

/// The stocks the flow `flow` leaves and enters (their canonical names; where more than one
/// lists it, the first by name), or `None` at an end where it has a cloud.
pub fn flow_ends(model: &SdModel, flow: &str) -> (Option<String>, Option<String>) {
    ends(model, &canonical(flow))
}

/// The point on the side of the stock at `c` (centre) and `size` that faces `target`.
fn facing(c: (f64, f64), size: (f64, f64), target: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (target.0 - c.0, target.1 - c.1);
    if dx.abs() * size.1 >= dy.abs() * size.0 {
        (c.0 + dx.signum() * size.0 / 2.0, c.1)
    } else {
        (c.0, c.1 + dy.signum() * size.1 / 2.0)
    }
}

/// A pipe from `a` to `b`: straight if they line up, else with one right-angled bend.
fn ortho(a: (f64, f64), b: (f64, f64)) -> Vec<(f64, f64)> {
    if (a.0 - b.0).abs() < EPS || (a.1 - b.1).abs() < EPS {
        vec![a, b]
    } else {
        vec![a, (b.0, a.1), b]
    }
}

/// The point of the polyline closest to `p`, and how far it is.
fn project(pts: &[(f64, f64)], p: (f64, f64)) -> ((f64, f64), f64) {
    let mut best = (p, f64::INFINITY);
    for w in pts.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else {
            continue;
        };
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        let t = if len2 < EPS {
            0.0
        } else {
            (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
        };
        let q = (a.0 + t * dx, a.1 + t * dy);
        let d = (p.0 - q.0).hypot(p.1 - q.1);
        if d < best.1 {
            best = (q, d);
        }
    }
    if pts.len() == 1
        && let Some(a) = pts.first()
    {
        return (*a, (p.0 - a.0).hypot(p.1 - a.1));
    }
    best
}

/// The middle of the first segment of a polyline.
fn first_middle(pts: &[(f64, f64)]) -> Option<(f64, f64)> {
    let a = pts.first()?;
    let b = pts.get(1).unwrap_or(a);
    Some(((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0))
}

fn overlaps(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    (a.0 - b.0).abs() * 2.0 < a.2 + b.2 && (a.1 - b.1).abs() * 2.0 < a.3 + b.3
}

impl Layout {
    /// Whether every variable of `model` has a shape.
    pub fn is_complete(&self, model: &SdModel) -> bool {
        model
            .vars
            .iter()
            .all(|v| self.shapes.contains_key(&canonical(&v.name)))
    }

    /// The centre and size of the shape of `name` (canonical), if it has one.
    fn rect(&self, model: &SdModel, name: &str) -> Option<(f64, f64, f64, f64)> {
        let s = self.shapes.get(name)?;
        let v = model.vars.iter().find(|v| canonical(&v.name) == name)?;
        let (w, h) = s.size(&v.kind);
        Some((s.x, s.y, w, h))
    }

    /// A layout for `model`, with every variable placed and a link for each use of one variable
    /// in the equation of another. Deterministic: ties are broken by name.
    pub fn auto(model: &SdModel) -> Layout {
        let mut l = Layout::default();
        let mut stocks: Vec<String> = model
            .vars
            .iter()
            .filter(|v| matches!(v.kind, VarKind::Stock { .. }))
            .map(|v| canonical(&v.name))
            .collect();
        stocks.sort();
        let mut flows: Vec<String> = model
            .vars
            .iter()
            .filter(|v| matches!(v.kind, VarKind::Flow { .. }))
            .map(|v| canonical(&v.name))
            .collect();
        flows.sort();
        let flow_ends: BTreeMap<String, (Option<String>, Option<String>)> =
            flows.iter().map(|f| (f.clone(), ends(model, f))).collect();

        // Stocks: a breadth-first rank along the flows, from the stocks nothing flows into
        // first, each connected chain in a row of its own.
        let mut out_edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut has_in: BTreeSet<String> = BTreeSet::new();
        for (src, dst) in flow_ends.values() {
            if let (Some(s), Some(d)) = (src, dst)
                && s != d
            {
                out_edges.entry(s.clone()).or_default().insert(d.clone());
                has_in.insert(d.clone());
            }
        }
        let roots: Vec<&String> = stocks
            .iter()
            .filter(|s| !has_in.contains(*s))
            .chain(stocks.iter().filter(|s| has_in.contains(*s)))
            .collect();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut chains: Vec<Vec<(String, usize)>> = Vec::new();
        for root in roots {
            if seen.contains(root) {
                continue;
            }
            seen.insert(root.clone());
            let mut chain = vec![(root.clone(), 0usize)];
            let mut k = 0;
            while let Some((cur, rank)) = chain.get(k).cloned() {
                k += 1;
                for next in out_edges.get(&cur).into_iter().flatten() {
                    if seen.insert(next.clone()) {
                        chain.push((next.clone(), rank + 1));
                    }
                }
            }
            chains.push(chain);
        }
        let mut y0 = ORIGIN.1;
        for chain in &chains {
            let mut per_rank: BTreeMap<usize, usize> = BTreeMap::new();
            let mut rows = 1usize;
            for (name, rank) in chain {
                let sub = per_rank.entry(*rank).or_insert(0);
                l.shapes.insert(
                    name.clone(),
                    Shape {
                        x: ORIGIN.0 + *rank as f64 * COLUMN,
                        y: y0 + *sub as f64 * ROW,
                        width: Some(STOCK_SIZE.0),
                        height: Some(STOCK_SIZE.1),
                        pts: Vec::new(),
                    },
                );
                *sub += 1;
                rows = rows.max(*sub);
            }
            y0 += rows as f64 * ROW;
        }

        // Flows between stocks first, then those with a cloud.
        let mut lanes: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut left_used: BTreeSet<String> = BTreeSet::new();
        let mut right_used: BTreeSet<String> = BTreeSet::new();
        let mut extra_in: BTreeMap<String, usize> = BTreeMap::new();
        let mut extra_out: BTreeMap<String, usize> = BTreeMap::new();
        let mut loose: Vec<String> = Vec::new();
        for f in &flows {
            let Some((src, dst)) = flow_ends.get(f) else {
                continue;
            };
            let (Some(s), Some(t)) = (src, dst) else {
                continue;
            };
            let (Some(sr), Some(tr)) = (l.rect(model, s), l.rect(model, t)) else {
                continue;
            };
            let lane = {
                let n = lanes.entry((s.clone(), t.clone())).or_insert(0);
                *n += 1;
                *n - 1
            } as f64;
            let (pts, valve) = if tr.0 > sr.0 + EPS {
                let a = (sr.0 + sr.2 / 2.0, sr.1 + lane * 12.0);
                let b = (tr.0 - tr.2 / 2.0, tr.1 + lane * 12.0);
                left_used.insert(t.clone());
                right_used.insert(s.clone());
                let pts = ortho(a, b);
                let mid = first_middle(&pts).unwrap_or(a);
                (pts, (mid.0 + lane * 25.0, mid.1))
            } else {
                // Back to a stock at or left of the source: under both.
                let dx = if s == t { 15.0 } else { 0.0 };
                let low = sr.1.max(tr.1) + 90.0 + lane * 30.0;
                let pts = vec![
                    (sr.0 + dx, sr.1 + sr.3 / 2.0),
                    (sr.0 + dx, low),
                    (tr.0 - dx, low),
                    (tr.0 - dx, tr.1 + tr.3 / 2.0),
                ];
                let mid = ((sr.0 + tr.0) / 2.0, low);
                (pts, mid)
            };
            l.shapes.insert(
                f.clone(),
                Shape {
                    x: valve.0,
                    y: valve.1,
                    width: None,
                    height: None,
                    pts,
                },
            );
        }
        for f in &flows {
            let Some((src, dst)) = flow_ends.get(f) else {
                continue;
            };
            if l.shapes.contains_key(f) {
                continue;
            }
            let pts = match (src, dst) {
                (None, None) => {
                    loose.push(f.clone());
                    continue;
                }
                (None, Some(t)) => {
                    let Some(tr) = l.rect(model, t) else {
                        continue;
                    };
                    let m = if left_used.insert(t.clone()) {
                        0
                    } else {
                        let n = extra_in.entry(t.clone()).or_insert(0);
                        *n += 1;
                        *n
                    };
                    if m == 0 {
                        vec![(tr.0 - tr.2 / 2.0 - 110.0, tr.1), (tr.0 - tr.2 / 2.0, tr.1)]
                    } else {
                        let cy = tr.1 + 70.0 + 50.0 * (m - 1) as f64;
                        vec![
                            (tr.0 - 125.0, cy),
                            (tr.0 - 15.0, cy),
                            (tr.0 - 15.0, tr.1 + tr.3 / 2.0),
                        ]
                    }
                }
                (Some(s), None) => {
                    let Some(sr) = l.rect(model, s) else {
                        continue;
                    };
                    let m = if right_used.insert(s.clone()) {
                        0
                    } else {
                        let n = extra_out.entry(s.clone()).or_insert(0);
                        *n += 1;
                        *n
                    };
                    if m == 0 {
                        vec![(sr.0 + sr.2 / 2.0, sr.1), (sr.0 + sr.2 / 2.0 + 110.0, sr.1)]
                    } else {
                        let cy = sr.1 + 70.0 + 50.0 * (m - 1) as f64;
                        vec![
                            (sr.0 + 15.0, sr.1 + sr.3 / 2.0),
                            (sr.0 + 15.0, cy),
                            (sr.0 + 125.0, cy),
                        ]
                    }
                }
                (Some(_), Some(_)) => continue,
            };
            // A bent pipe has its valve on the leg the cloud is on.
            let mid = if pts.len() == 3 {
                match (pts.first(), pts.get(1), pts.get(2)) {
                    (Some(a), Some(b), Some(c)) => {
                        if (a.1 - b.1).abs() < EPS {
                            ((a.0 + b.0) / 2.0, a.1)
                        } else {
                            ((b.0 + c.0) / 2.0, b.1)
                        }
                    }
                    _ => continue,
                }
            } else {
                let Some(m) = first_middle(&pts) else {
                    continue;
                };
                m
            };
            l.shapes.insert(
                f.clone(),
                Shape {
                    x: mid.0,
                    y: mid.1,
                    width: None,
                    height: None,
                    pts,
                },
            );
        }
        // Flows with no stock at either end: a row of their own under the stocks.
        let bottom = l.bottom();
        for (i, f) in loose.iter().enumerate() {
            let x = ORIGIN.0 + i as f64 * COLUMN;
            let y = bottom + 100.0;
            l.shapes.insert(
                f.clone(),
                Shape {
                    x: x + 70.0,
                    y,
                    width: None,
                    height: None,
                    pts: vec![(x, y), (x + 140.0, y)],
                },
            );
        }

        // Auxiliaries: above the first variable each feeds that is already placed, repeating
        // while that places more; then a row below.
        let mut aux: Vec<String> = model
            .vars
            .iter()
            .filter(|v| matches!(v.kind, VarKind::Aux))
            .map(|v| canonical(&v.name))
            .collect();
        aux.sort();
        let uses: BTreeMap<String, BTreeSet<String>> = model
            .vars
            .iter()
            .map(|v| (canonical(&v.name), used_by(model, v)))
            .collect();
        let mut above: BTreeMap<String, usize> = BTreeMap::new();
        let mut row_y: Option<f64> = None;
        let mut row_i = 0usize;
        loop {
            let mut placed_any = false;
            let mut left = Vec::new();
            for a in std::mem::take(&mut aux) {
                let target = model
                    .vars
                    .iter()
                    .filter(|v| !matches!(v.kind, VarKind::Aux))
                    .chain(model.vars.iter().filter(|v| matches!(v.kind, VarKind::Aux)))
                    .map(|v| canonical(&v.name))
                    .filter(|n| {
                        uses.get(n).is_some_and(|u| u.contains(&a)) && l.shapes.contains_key(n)
                    })
                    .min_by_key(|n| {
                        let is_aux = model
                            .vars
                            .iter()
                            .any(|v| canonical(&v.name) == *n && matches!(v.kind, VarKind::Aux));
                        (is_aux, n.clone())
                    });
                let Some(t) = target else {
                    left.push(a);
                    continue;
                };
                let Some(tr) = l.rect(model, &t) else {
                    left.push(a);
                    continue;
                };
                let k = {
                    let n = above.entry(t.clone()).or_insert(0);
                    *n += 1;
                    *n - 1
                };
                let dx = match k {
                    0 => 0.0,
                    k if k % 2 == 1 => 45.0 * k.div_ceil(2) as f64,
                    k => -45.0 * (k / 2) as f64,
                };
                l.shapes.insert(
                    a,
                    Shape {
                        x: tr.0 + dx,
                        y: tr.1 - 90.0,
                        width: Some(AUX_SIZE.0),
                        height: Some(AUX_SIZE.1),
                        pts: Vec::new(),
                    },
                );
                placed_any = true;
            }
            aux = left;
            if aux.is_empty() {
                break;
            }
            if !placed_any {
                // None feeds anything placed: the next that feeds no other one left goes in a
                // row under everything, and what feeds it can then go above it.
                let pick = aux
                    .iter()
                    .position(|a| {
                        !aux.iter()
                            .any(|b| uses.get(b).is_some_and(|u| u.contains(a)))
                    })
                    .unwrap_or(0);
                let a = aux.remove(pick);
                let y = *row_y.get_or_insert_with(|| l.bottom() + 100.0);
                l.shapes.insert(
                    a,
                    Shape {
                        x: ORIGIN.0 + 60.0 * row_i as f64,
                        y,
                        width: Some(AUX_SIZE.0),
                        height: Some(AUX_SIZE.1),
                        pts: Vec::new(),
                    },
                );
                row_i += 1;
            }
        }
        l.separate(model);
        l.sync_all_links(model);
        l
    }

    /// The lowest y of anything placed (shapes and pipes).
    fn bottom(&self) -> f64 {
        let mut b = ORIGIN.1;
        for s in self.shapes.values() {
            b = b.max(s.y + 20.0);
            for p in &s.pts {
                b = b.max(p.1);
            }
        }
        b
    }

    /// Moves an auxiliary or a valve that overlaps another shape until it does not (an
    /// auxiliary down, a valve along its pipe), so no two shapes overlap.
    fn separate(&mut self, model: &SdModel) {
        let mut names: Vec<String> = self.shapes.keys().cloned().collect();
        // Stocks stay; then valves; then auxiliaries.
        let rank = |n: &String| {
            model
                .vars
                .iter()
                .find(|v| canonical(&v.name) == *n)
                .map_or(3, |v| match v.kind {
                    VarKind::Stock { .. } => 0,
                    VarKind::Flow { .. } => 1,
                    VarKind::Aux => 2,
                })
        };
        names.sort_by_key(|n| (rank(n), n.clone()));
        let mut done: Vec<(f64, f64, f64, f64)> = Vec::new();
        for n in names {
            let Some(r) = self.rect(model, &n) else {
                continue;
            };
            let flow = rank(&n) == 1;
            let mut cur = r;
            let mut tries = 0;
            while done.iter().any(|d| overlaps(*d, cur)) && tries < 400 {
                tries += 1;
                if flow {
                    // Along the first leg, alternately each way.
                    let step = 25.0 * (tries as f64 / 2.0).ceil();
                    let sign = if tries % 2 == 1 { 1.0 } else { -1.0 };
                    cur.0 = r.0 + sign * step;
                } else {
                    cur.1 += 25.0;
                }
            }
            if flow {
                if let Some(s) = self.shapes.get_mut(&n) {
                    let (q, d) = project(&s.pts, (cur.0, cur.1));
                    if d < 1.0 {
                        s.x = q.0;
                        s.y = q.1;
                        cur = (q.0, q.1, cur.2, cur.3);
                    } else {
                        cur = r;
                    }
                }
            } else if let Some(s) = self.shapes.get_mut(&n) {
                s.y = cur.1;
            }
            done.push(cur);
        }
    }

    /// Gives a shape to every variable that has none: all of them if there is no diagram, else
    /// each missing one in a row below the rest. A flow with no pipe gets one.
    pub fn complete(&mut self, model: &SdModel) {
        self.shapes
            .retain(|k, _| model.vars.iter().any(|v| canonical(&v.name) == *k));
        if self.shapes.is_empty() {
            *self = Layout::auto(model);
            return;
        }
        let missing: Vec<&Variable> = model
            .vars
            .iter()
            .filter(|v| !self.shapes.contains_key(&canonical(&v.name)))
            .collect();
        let y = self.bottom() + 100.0;
        for (k, v) in missing.iter().enumerate() {
            let shape = self.fresh(v, ORIGIN.0 + 90.0 * k as f64, y);
            self.shapes.insert(canonical(&v.name), shape);
        }
        let flows: Vec<String> = self
            .shapes
            .iter()
            .filter(|(n, s)| {
                s.pts.is_empty()
                    && model.vars.iter().any(|v| {
                        canonical(&v.name) == **n && matches!(v.kind, VarKind::Flow { .. })
                    })
            })
            .map(|(n, _)| n.clone())
            .collect();
        for f in flows {
            self.reroute(model, &f);
        }
        self.connectors
            .retain(|c| self.shapes.contains_key(&c.from) && self.shapes.contains_key(&c.to));
        for v in missing {
            self.sync_links(model, &canonical(&v.name));
        }
    }

    /// A shape for `v` with its centre at (x, y).
    fn fresh(&self, v: &Variable, x: f64, y: f64) -> Shape {
        match v.kind {
            VarKind::Stock { .. } => Shape {
                width: Some(STOCK_SIZE.0),
                height: Some(STOCK_SIZE.1),
                ..Shape::at(x, y)
            },
            VarKind::Aux => Shape {
                width: Some(AUX_SIZE.0),
                height: Some(AUX_SIZE.1),
                ..Shape::at(x, y)
            },
            VarKind::Flow { .. } => Shape {
                pts: vec![(x - 50.0, y), (x + 50.0, y)],
                ..Shape::at(x, y)
            },
        }
    }

    /// Places `v` (already in `model`) in the first free place in a row under the diagram, or
    /// at `at`. A flow's pipe goes to the stocks that list it.
    pub fn place(&mut self, model: &SdModel, v: &Variable, at: Option<(f64, f64)>) {
        let name = canonical(&v.name);
        let (x, y) = at.unwrap_or_else(|| {
            let y = self.bottom() + 100.0;
            let mut x = ORIGIN.0;
            let mut tries = 0;
            let (w, h) = self.fresh(v, 0.0, 0.0).size(&v.kind);
            while tries < 400
                && model.vars.iter().any(|o| {
                    canonical(&o.name) != name
                        && self
                            .rect(model, &canonical(&o.name))
                            .is_some_and(|r| overlaps(r, (x, y, w, h)))
                })
            {
                x += 90.0;
                tries += 1;
            }
            (x, y)
        });
        let shape = self.fresh(v, x, y);
        self.shapes.insert(name.clone(), shape);
        if matches!(v.kind, VarKind::Flow { .. }) {
            if at.is_none() {
                // Between the stocks it joins, or beside the one it is at.
                let (src, dst) = ends(model, &name);
                let from = src.as_deref().and_then(|n| self.rect(model, n));
                let to = dst.as_deref().and_then(|n| self.rect(model, n));
                let valve = match (from, to) {
                    (Some(a), Some(b)) => Some(((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)),
                    (Some(a), None) => Some((a.0 + 130.0, a.1)),
                    (None, Some(b)) => Some((b.0 - 130.0, b.1)),
                    (None, None) => None,
                };
                if let (Some(p), Some(s)) = (valve, self.shapes.get_mut(&name)) {
                    s.x = p.0;
                    s.y = p.1;
                    s.pts = vec![(p.0 - 80.0, p.1), (p.0 + 80.0, p.1)];
                }
            }
            self.reroute(model, &name);
        }
        self.sync_links(model, &name);
    }

    /// Takes a variable out: its shape, and the links to and from it.
    pub fn remove(&mut self, name: &str) {
        self.shapes.remove(name);
        self.connectors.retain(|c| c.from != name && c.to != name);
    }

    /// Renames a variable (canonical names).
    pub fn rename(&mut self, old: &str, new: &str) {
        if let Some(s) = self.shapes.remove(old) {
            self.shapes.insert(new.to_owned(), s);
        }
        for c in &mut self.connectors {
            if c.from == old {
                c.from = new.to_owned();
            }
            if c.to == old {
                c.to = new.to_owned();
            }
        }
    }

    /// Makes the links into `name` those its equation asks for: one from each variable it
    /// uses, and none from a variable it does not.
    pub fn sync_links(&mut self, model: &SdModel, name: &str) {
        let Some(v) = model.vars.iter().find(|v| canonical(&v.name) == name) else {
            return;
        };
        let want = used_by(model, v);
        self.connectors
            .retain(|c| c.to != name || want.contains(&c.from));
        for from in want {
            if !self
                .connectors
                .iter()
                .any(|c| c.from == from && c.to == name)
            {
                self.connectors.push(Connector {
                    from,
                    to: name.to_owned(),
                    angle: None,
                });
            }
        }
    }

    /// [`Layout::sync_links`] for every variable, and none for a link to or from a variable
    /// the model does not have.
    pub fn sync_all_links(&mut self, model: &SdModel) {
        let known: BTreeSet<String> = model.vars.iter().map(|v| canonical(&v.name)).collect();
        self.connectors
            .retain(|c| known.contains(&c.from) && known.contains(&c.to));
        for v in &model.vars {
            self.sync_links(model, &canonical(&v.name));
        }
    }

    /// Draws the pipe of `flow` (canonical) from the stock it leaves to the stock it enters, or
    /// to a cloud where it has none (the end of the pipe it had, or one 80 away from its
    /// valve), and puts the valve on the pipe.
    pub fn reroute(&mut self, model: &SdModel, flow: &str) {
        let Some(shape) = self.shapes.get(flow).cloned() else {
            return;
        };
        let valve = (shape.x, shape.y);
        let (src, dst) = ends(model, flow);
        let a = match src.as_deref().and_then(|s| self.rect(model, s)) {
            Some(r) => facing((r.0, r.1), (r.2, r.3), valve),
            None => shape
                .pts
                .first()
                .copied()
                .unwrap_or((valve.0 - 80.0, valve.1)),
        };
        let b = match dst.as_deref().and_then(|s| self.rect(model, s)) {
            Some(r) => facing((r.0, r.1), (r.2, r.3), valve),
            None => shape
                .pts
                .last()
                .copied()
                .unwrap_or((valve.0 + 80.0, valve.1)),
        };
        let mut pts = ortho(a, b);
        if (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS {
            pts = vec![a, (a.0 + 80.0, a.1)];
        }
        let (q, d) = project(&pts, valve);
        let valve = if d < EPS {
            valve
        } else if (src.is_some() || dst.is_some()) && d > 40.0 {
            first_middle(&pts).unwrap_or(q)
        } else {
            q
        };
        if let Some(s) = self.shapes.get_mut(flow) {
            s.pts = pts;
            s.x = valve.0;
            s.y = valve.1;
        }
    }

    /// Moves the shape of `name` (canonical) to the centre (x, y). A flow's pipe moves with its
    /// valve; the pipes of the flows at a stock follow the stock.
    pub fn move_to(&mut self, model: &SdModel, name: &str, x: f64, y: f64) {
        let Some(old) = self.shapes.get(name).cloned() else {
            return;
        };
        let (dx, dy) = (x - old.x, y - old.y);
        if let Some(s) = self.shapes.get_mut(name) {
            s.x = x;
            s.y = y;
            for p in &mut s.pts {
                p.0 += dx;
                p.1 += dy;
            }
        }
        let is_stock = model
            .vars
            .iter()
            .any(|v| canonical(&v.name) == name && matches!(v.kind, VarKind::Stock { .. }));
        if is_stock {
            let attached: Vec<String> = model
                .vars
                .iter()
                .filter(|v| matches!(v.kind, VarKind::Flow { .. }))
                .map(|v| canonical(&v.name))
                .filter(|f| {
                    let (s, d) = ends(model, f);
                    s.as_deref() == Some(name) || d.as_deref() == Some(name)
                })
                .collect();
            for f in attached {
                self.reroute(model, &f);
            }
        } else if model
            .vars
            .iter()
            .any(|v| canonical(&v.name) == name && matches!(v.kind, VarKind::Flow { .. }))
        {
            self.reroute(model, name);
        }
    }

    /// Gives the flow `name` (canonical) the pipe `pts`, and moves its valve onto it if it is
    /// not already on it.
    pub fn set_pipe(&mut self, name: &str, pts: Vec<(f64, f64)>) {
        if let Some(s) = self.shapes.get_mut(name) {
            let (q, d) = project(&pts, (s.x, s.y));
            if d.is_finite() && d >= EPS {
                s.x = q.0;
                s.y = q.1;
            }
            s.pts = pts;
        }
    }

    /// Two shapes that overlap, if any (their canonical names): what a layout of `model` must
    /// not have.
    pub fn first_overlap(&self, model: &SdModel) -> Option<(String, String)> {
        let names: Vec<&String> = self.shapes.keys().collect();
        for (i, a) in names.iter().enumerate() {
            for b in names.iter().skip(i + 1) {
                if let (Some(ra), Some(rb)) = (self.rect(model, a), self.rect(model, b))
                    && overlaps(ra, rb)
                {
                    return Some(((*a).clone(), (*b).clone()));
                }
            }
        }
        None
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn chain() -> SdModel {
        SdModel {
            vars: vec![
                Variable::stock("Susceptible", "990", &[], &["infection"]),
                Variable::stock("Infected", "10", &["infection"], &["recovery"]),
                Variable::stock("Recovered", "0", &["recovery"], &[]),
                Variable::flow("infection", "rate * Infected"),
                Variable::flow("recovery", "Infected / 5"),
                Variable::aux("rate", "0.1"),
                Variable::aux("unused", "1"),
            ],
            ..SdModel::default()
        }
    }

    #[test]
    fn auto_layout_puts_a_chain_in_a_row_without_overlaps() {
        let m = chain();
        let l = Layout::auto(&m);
        assert!(l.is_complete(&m));
        assert_eq!(l.first_overlap(&m), None);
        let (a, b, c) = (
            &l.shapes["susceptible"],
            &l.shapes["infected"],
            &l.shapes["recovered"],
        );
        assert!(a.x < b.x && b.x < c.x);
        assert_eq!(a.y, b.y);
        // A pipe between two stocks on a row is straight.
        assert_eq!(l.shapes["infection"].pts.len(), 2);
        // `rate` feeds `infection`: above its valve; `unused` is in the row below.
        assert!(l.shapes["rate"].y < l.shapes["infection"].y);
        assert!(l.shapes["unused"].y > b.y);
        // Links: rate and infected into infection, infected into recovery.
        assert_eq!(l.connectors.len(), 3);
        // Stable.
        assert_eq!(l, Layout::auto(&m));
    }

    #[test]
    fn cloud_flows_and_parallel_flows_do_not_overlap() {
        let m = SdModel {
            vars: vec![
                Variable::stock("A", "0", &["in1", "in2", "x"], &["out1", "out2", "x2"]),
                Variable::stock("B", "0", &["x2"], &["x"]),
                Variable::flow("in1", "1"),
                Variable::flow("in2", "1"),
                Variable::flow("out1", "1"),
                Variable::flow("out2", "1"),
                Variable::flow("x", "1"),
                Variable::flow("x2", "1"),
                Variable::flow("alone", "1"),
                Variable::aux("p", "alone"),
                Variable::aux("q", "p + A"),
                Variable::aux("r", "p + A"),
                Variable::aux("s", "p + A"),
            ],
            ..SdModel::default()
        };
        let l = Layout::auto(&m);
        assert!(l.is_complete(&m));
        assert_eq!(l.first_overlap(&m), None);
    }

    #[test]
    fn renaming_and_removing_move_the_shapes_and_links() {
        let m = chain();
        let mut l = Layout::auto(&m);
        l.rename("rate", "speed");
        assert!(l.shapes.contains_key("speed") && !l.shapes.contains_key("rate"));
        assert!(l.connectors.iter().any(|c| c.from == "speed"));
        l.remove("speed");
        assert!(l.connectors.iter().all(|c| c.from != "speed"));
    }

    #[test]
    fn moving_a_stock_takes_its_pipes_with_it() {
        let m = chain();
        let mut l = Layout::auto(&m);
        l.move_to(&m, "infected", 400.0, 500.0);
        let end = l.shapes["infection"].pts.last().copied().unwrap();
        let r = l.rect(&m, "infected").unwrap();
        // The pipe ends on the stock's edge.
        assert!(
            (end.0 - r.0).abs() <= r.2 / 2.0 + EPS && (end.1 - r.1).abs() <= r.3 / 2.0 + EPS,
            "{end:?} {r:?}"
        );
        for p in l.shapes["infection"].pts.windows(2) {
            assert!(
                (p[0].0 - p[1].0).abs() < EPS || (p[0].1 - p[1].1).abs() < EPS,
                "not at right angles: {:?}",
                l.shapes["infection"].pts
            );
        }
    }
}
