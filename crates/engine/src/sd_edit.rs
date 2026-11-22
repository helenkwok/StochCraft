//! Editing a system dynamics model (ROADMAP M4.2): `sd.new`, `sd.variable.add`, `.set` and
//! `.remove`, `sd.specs.set` and `sd.layout.set`.
//!
//! Every edit works on a copy of the model, which is compiled (a bad equation, a name that is
//! not there or an algebraic loop is refused, with the compile error, and the model is as it
//! was) and only then replaces the model. An edit's undo entry is the whole model as it was
//! ([`Undo::Sd`]), so undo swaps it back in and compiles it again, and redo does the same the
//! other way. The edits are journalled like any other command and make the last results stale.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_model::Model;
use stochcraft_sd::{
    Connector, Gf, GfKind, GfX, Layout, Method, SdModel, SimSpecs, Step, VarKind, Variable,
    canonical, find_variable, flow_ends,
};

use crate::commands::{parse, some};
use crate::sd::{SdDoc, describe, no_model, sd_error, warnings_json};
use crate::{CommandError, Engine, ErrorCode, HistoryEntry, JournalStart, StartKind, Undo};

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParams, message)
}

fn not_found(name: &str) -> CommandError {
    CommandError::new(ErrorCode::NotFound, format!("no variable `{name}`"))
}

/// Works on the layout and the model it is of together (the layout is taken out of the model
/// while it works, so the model can be read).
fn with_layout(model: &mut SdModel, f: impl FnOnce(&mut Layout, &SdModel)) {
    let mut layout = std::mem::take(&mut model.layout);
    f(&mut layout, model);
    model.layout = layout;
}

impl Engine {
    /// Puts `model` in place of the open model (after compiling it) and gives back the one it
    /// replaced: what undo and redo do.
    pub(crate) fn swap_sd_model(&mut self, model: SdModel) -> Result<SdModel, CommandError> {
        let Some(doc) = self.sd.as_mut() else {
            return Err(no_model());
        };
        let program = stochcraft_sd::compile(&model).map_err(sd_error)?;
        doc.program = program;
        doc.last_stale = true;
        Ok(std::mem::replace(&mut doc.model, model))
    }

    /// An edit of the open model: `change` works on a copy, which is compiled, and only then
    /// replaces the model. Returns what the checks found (warnings). The edit goes on the undo
    /// stack as the whole model before it.
    fn sd_edit(
        &mut self,
        label: &'static str,
        change: impl FnOnce(&mut SdModel) -> Result<(), CommandError>,
    ) -> Result<Value, CommandError> {
        let Some(doc) = self.sd.as_mut() else {
            return Err(no_model());
        };
        let mut next = doc.model.clone();
        change(&mut next)?;
        let program = stochcraft_sd::compile(&next).map_err(|e| {
            let mut c = sd_error(e);
            c.message = format!("{}; the model is unchanged", c.message);
            c
        })?;
        let diagnostics = warnings_json(&program);
        let old = std::mem::replace(&mut doc.model, next);
        doc.program = program;
        doc.last_stale = true;
        self.undo.push(HistoryEntry {
            label,
            op: Undo::Sd(Box::new(old)),
        });
        self.redo.clear();
        self.revision += 1;
        Ok(diagnostics)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MethodParam {
    Euler,
    Rk4,
}

impl From<MethodParam> for Method {
    fn from(m: MethodParam) -> Self {
        match m {
            MethodParam::Euler => Method::Euler,
            MethodParam::Rk4 => Method::Rk4,
        }
    }
}

/// A point of the diagram.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

fn finite(v: f64, what: &str) -> Result<f64, CommandError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(invalid(format!("{what} must be a number")))
    }
}

fn set_specs(
    s: &mut SimSpecs,
    start: Option<f64>,
    stop: Option<f64>,
    dt: Option<f64>,
    dt_reciprocal: Option<bool>,
    method: Option<MethodParam>,
    time_units: Option<String>,
) -> Result<(), CommandError> {
    if let Some(v) = start {
        s.start = finite(v, "`start`")?;
    }
    if let Some(v) = stop {
        s.stop = finite(v, "`stop`")?;
    }
    match (dt, dt_reciprocal) {
        (Some(v), r) => {
            let v = finite(v, "`dt`")?;
            s.dt = if r == Some(true) {
                Step::Reciprocal(v)
            } else {
                Step::Value(v)
            };
        }
        (None, Some(_)) => return Err(invalid("`dt_reciprocal` needs a `dt`")),
        (None, None) => {}
    }
    if let Some(m) = method {
        s.method = m.into();
    }
    if let Some(u) = time_units {
        s.time_units = u.trim().to_owned();
    }
    Ok(())
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct New {
    /// The model's name.
    #[serde(default)]
    pub name: Option<String>,
    /// The first time (default 0).
    #[serde(default)]
    pub start: Option<f64>,
    /// The last time (default 100); it must be after the start.
    #[serde(default)]
    pub stop: Option<f64>,
    /// The time step (default 1), in the model's time unit.
    #[serde(default)]
    pub dt: Option<f64>,
    /// The step is 1/`dt` (XMILE's `reciprocal`): `dt` 8 is a step of 0.125.
    #[serde(default)]
    pub dt_reciprocal: Option<bool>,
    /// `euler` (the default) or `rk4`.
    #[serde(default)]
    pub method: Option<MethodParam>,
    /// The time unit, as a word (`Years`, `Days`...).
    #[serde(default)]
    pub time_units: Option<String>,
    /// The interval results are saved at; by default the time step.
    #[serde(default)]
    pub save_step: Option<f64>,
}

pub(crate) fn new(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let raw = p.clone();
    let p: New = parse(p)?;
    let mut model = SdModel {
        name: p.name.clone().unwrap_or_default(),
        ..SdModel::default()
    };
    set_specs(
        &mut model.specs,
        p.start,
        p.stop,
        p.dt,
        p.dt_reciprocal,
        p.method,
        p.time_units,
    )?;
    model.specs.save_step = p.save_step;
    let program = stochcraft_sd::compile(&model).map_err(sd_error)?;
    let mut start = JournalStart::new(StartKind::NewSd, p.name);
    start.params = Some(raw);
    start.document = Some("system_dynamics".to_owned());
    let result = json!({
        "model": describe(&model, &program),
        "diagnostics": warnings_json(&program),
    });
    e.new_document(Model::default(), start);
    e.sd = Some(SdDoc {
        path: String::new(),
        file: None,
        saved: model.clone(),
        model,
        program,
        notes: Vec::new(),
        drops_on_save: false,
        bytes: None,
        last: None,
        last_stale: false,
    });
    if let Some(d) = &e.sd {
        e.journal_start.model_digest = crate::sd::sd_digest(&d.model);
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KindParam {
    Stock,
    Flow,
    Aux,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GfKindParam {
    Continuous,
    Extrapolate,
    Discrete,
}

/// A graphical function (a lookup table).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GfParam {
    /// Between the points: `continuous` (the default: linear, the end values beyond),
    /// `extrapolate` or `discrete` (a step).
    #[serde(default, rename = "type")]
    pub kind: Option<GfKindParam>,
    /// The x of each point, ascending; or leave it out and give `x_min` and `x_max` for evenly
    /// spaced points.
    #[serde(default)]
    pub x: Option<Vec<f64>>,
    #[serde(default)]
    pub x_min: Option<f64>,
    #[serde(default)]
    pub x_max: Option<f64>,
    /// The y of each point.
    pub y: Vec<f64>,
}

fn gf_from(g: GfParam) -> Result<Gf, CommandError> {
    let x = match (g.x, g.x_min, g.x_max) {
        (Some(points), None, None) => GfX::Points(points),
        (None, Some(min), Some(max)) => GfX::Scale { min, max },
        _ => {
            return Err(invalid(
                "a graphical function needs `x` (the points), or `x_min` and `x_max`, not both",
            ));
        }
    };
    let kind = match g.kind {
        None | Some(GfKindParam::Continuous) => GfKind::Continuous,
        Some(GfKindParam::Extrapolate) => GfKind::Extrapolate,
        Some(GfKindParam::Discrete) => GfKind::Discrete,
    };
    Ok(Gf { kind, x, y: g.y })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariableAdd {
    /// `stock`, `flow` or `aux` (an auxiliary, which includes a constant).
    pub kind: KindParam,
    /// The name; spaces are allowed, and an equation names it with underscores.
    pub name: String,
    /// The equation (XMILE's syntax: `/` divides); a stock's is its initial value. Needed unless
    /// an auxiliary is given a graphical function, which then is a function of time.
    #[serde(default)]
    pub equation: Option<String>,
    #[serde(default)]
    pub units: Option<String>,
    #[serde(default)]
    pub doc: Option<String>,
    /// A stock's flows in (flows that exist; or add the stock first and use `into_stock` on
    /// the flows).
    #[serde(default)]
    pub inflows: Option<Vec<String>>,
    /// A stock's flows out.
    #[serde(default)]
    pub outflows: Option<Vec<String>>,
    /// A stock or flow that cannot go negative.
    #[serde(default)]
    pub non_negative: Option<bool>,
    #[serde(default)]
    pub graphical_function: Option<GfParam>,
    /// The centre of its shape; by default the first free place under the diagram.
    #[serde(default)]
    pub position: Option<Position>,
    /// A flow: the stock it flows into.
    #[serde(default)]
    pub into_stock: Option<String>,
    /// A flow: the stock it flows out of.
    #[serde(default)]
    pub from_stock: Option<String>,
}

/// Puts the flow `flow` in the inflows or outflows of the stock `stock`.
fn attach(model: &mut SdModel, stock: &str, flow: &str, inflow: bool) -> Result<(), CommandError> {
    let idx = find_variable(model, stock).ok_or_else(|| not_found(stock))?;
    match model.vars.get_mut(idx).map(|v| &mut v.kind) {
        Some(VarKind::Stock {
            inflows, outflows, ..
        }) => {
            let list = if inflow { inflows } else { outflows };
            list.push(flow.to_owned());
            Ok(())
        }
        _ => Err(invalid(format!("`{stock}` is not a stock"))),
    }
}

pub(crate) fn variable_add(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: VariableAdd = parse(p)?;
    let name = p.name.trim().to_owned();
    if name.is_empty() {
        return Err(invalid("a variable needs a name"));
    }
    let non_negative = p.non_negative.unwrap_or(false);
    let is_flow = matches!(p.kind, KindParam::Flow);
    if !is_flow && (p.into_stock.is_some() || p.from_stock.is_some()) {
        return Err(invalid("`into_stock` and `from_stock` are for a flow"));
    }
    let kind = match p.kind {
        KindParam::Stock => VarKind::Stock {
            inflows: p.inflows.unwrap_or_default(),
            outflows: p.outflows.unwrap_or_default(),
            non_negative,
        },
        KindParam::Flow | KindParam::Aux => {
            if p.inflows.is_some() || p.outflows.is_some() {
                return Err(invalid("`inflows` and `outflows` are for a stock"));
            }
            if is_flow {
                VarKind::Flow { non_negative }
            } else {
                if p.non_negative.is_some() {
                    return Err(invalid("`non_negative` is for a stock or a flow"));
                }
                VarKind::Aux
            }
        }
    };
    let gf = p.graphical_function.map(gf_from).transpose()?;
    if p.equation.is_none() && gf.is_none() {
        return Err(invalid(
            "an equation is needed (an auxiliary can have a graphical function instead)",
        ));
    }
    let var = Variable {
        name: name.clone(),
        kind,
        eqn: p.equation,
        gf,
        units: p.units.filter(|u| !u.is_empty()),
        doc: p.doc.filter(|d| !d.is_empty()),
    };
    let at = p.position.map(|q| (q.x, q.y));
    if let Some((x, y)) = at {
        finite(x, "`x`")?;
        finite(y, "`y`")?;
    }
    let (into, from) = (p.into_stock, p.from_stock);
    let diagnostics = e.sd_edit("sd.variable.add", |m| {
        m.vars.push(var.clone());
        if let Some(s) = &into {
            attach(m, s, &name, true)?;
        }
        if let Some(s) = &from {
            attach(m, s, &name, false)?;
        }
        with_layout(m, |l, m| l.place(m, &var, at));
        Ok(())
    })?;
    Ok(json!({ "name": name, "diagnostics": diagnostics }))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariableSet {
    /// The variable to change.
    pub name: String,
    /// A new name; every equation and flow list that names the variable is rewritten.
    #[serde(default)]
    pub rename: Option<String>,
    #[serde(default)]
    pub equation: Option<String>,
    /// `null` (or empty) removes the units.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<String>")]
    pub units: Option<Option<String>>,
    /// `null` (or empty) removes the documentation.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<String>")]
    pub doc: Option<Option<String>>,
    /// A stock's flows in, replacing the list.
    #[serde(default)]
    pub inflows: Option<Vec<String>>,
    /// A stock's flows out, replacing the list (their order is the order they are limited in).
    #[serde(default)]
    pub outflows: Option<Vec<String>>,
    /// A stock or flow that cannot go negative.
    #[serde(default)]
    pub non_negative: Option<bool>,
    /// A graphical function; `null` removes it (the variable then needs an equation).
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<GfParam>")]
    pub graphical_function: Option<Option<GfParam>>,
}

pub(crate) fn variable_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: VariableSet = parse(p)?;
    let given = [
        p.rename.is_some(),
        p.equation.is_some(),
        p.units.is_some(),
        p.doc.is_some(),
        p.inflows.is_some(),
        p.outflows.is_some(),
        p.non_negative.is_some(),
        p.graphical_function.is_some(),
    ];
    if !given.contains(&true) {
        return Err(invalid("nothing to change: give at least one setting"));
    }
    let gf = match p.graphical_function.clone() {
        Some(Some(g)) => Some(Some(gf_from(g)?)),
        Some(None) => Some(None),
        None => None,
    };
    let name = p.name.clone();
    let result_name = p.rename.clone().unwrap_or_else(|| p.name.clone());
    let diagnostics = e.sd_edit("sd.variable.set", |m| {
        let idx = find_variable(m, &name).ok_or_else(|| not_found(&name))?;
        let canon = canonical(&name);
        let mut touched: Vec<String> = Vec::new();
        if let Some(v) = m.vars.get_mut(idx) {
            if let Some(eq) = &p.equation {
                v.eqn = Some(eq.clone());
            }
            if let Some(u) = &p.units {
                v.units = u.clone().filter(|s| !s.is_empty());
            }
            if let Some(d) = &p.doc {
                v.doc = d.clone().filter(|s| !s.is_empty());
            }
            if let Some(g) = gf {
                v.gf = g;
            }
            match &mut v.kind {
                VarKind::Stock {
                    inflows,
                    outflows,
                    non_negative,
                } => {
                    if let Some(list) = &p.inflows {
                        touched.extend(inflows.iter().chain(list.iter()).map(|f| canonical(f)));
                        *inflows = list.clone();
                    }
                    if let Some(list) = &p.outflows {
                        touched.extend(outflows.iter().chain(list.iter()).map(|f| canonical(f)));
                        *outflows = list.clone();
                    }
                    if let Some(n) = p.non_negative {
                        *non_negative = n;
                    }
                }
                VarKind::Flow { non_negative } => {
                    if p.inflows.is_some() || p.outflows.is_some() {
                        return Err(invalid("`inflows` and `outflows` are for a stock"));
                    }
                    if let Some(n) = p.non_negative {
                        *non_negative = n;
                    }
                }
                VarKind::Aux => {
                    if p.inflows.is_some() || p.outflows.is_some() {
                        return Err(invalid("`inflows` and `outflows` are for a stock"));
                    }
                    if p.non_negative.is_some() {
                        return Err(invalid("`non_negative` is for a stock or a flow"));
                    }
                }
            }
        }
        with_layout(m, |l, m| {
            if p.equation.is_some() {
                l.sync_links(m, &canon);
            }
            touched.sort();
            touched.dedup();
            for f in &touched {
                l.reroute(m, f);
            }
        });
        if let Some(new) = &p.rename {
            stochcraft_sd::rename_variable(m, &name, new).map_err(sd_error)?;
        }
        Ok(())
    })?;
    Ok(json!({ "name": result_name, "diagnostics": diagnostics }))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariableRemove {
    /// The variable to remove.
    pub name: String,
    /// Also take it out of the flow lists of the stocks that list it (never out of an equation).
    #[serde(default)]
    pub force: Option<bool>,
}

pub(crate) fn variable_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: VariableRemove = parse(p)?;
    let force = p.force.unwrap_or(false);
    let diagnostics = e.sd_edit("sd.variable.remove", |m| {
        if find_variable(m, &p.name).is_none() {
            return Err(not_found(&p.name));
        }
        // The flows at a stock that goes keep their pipes, with a cloud where the stock was.
        let canon = canonical(&p.name);
        let attached: Vec<String> = m
            .vars
            .iter()
            .filter(|v| matches!(v.kind, VarKind::Flow { .. }))
            .map(|v| canonical(&v.name))
            .filter(|f| {
                let (s, d) = flow_ends(m, f);
                s.as_deref() == Some(canon.as_str()) || d.as_deref() == Some(canon.as_str())
            })
            .collect();
        stochcraft_sd::remove_variable(m, &p.name, force).map_err(|err| {
            let mut c = sd_error(err);
            c.code = ErrorCode::Conflict;
            c
        })?;
        with_layout(m, |l, m| {
            for f in &attached {
                l.reroute(m, f);
            }
        });
        Ok(())
    })?;
    Ok(json!({ "name": p.name, "diagnostics": diagnostics }))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpecsSet {
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub stop: Option<f64>,
    /// The time step, in the model's time unit.
    #[serde(default)]
    pub dt: Option<f64>,
    /// The step is 1/`dt`: `dt` 8 is a step of 0.125. Needs a `dt`.
    #[serde(default)]
    pub dt_reciprocal: Option<bool>,
    /// `euler` or `rk4`.
    #[serde(default)]
    pub method: Option<MethodParam>,
    #[serde(default)]
    pub time_units: Option<String>,
    /// The interval results are saved at; `null` makes it the time step.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<f64>")]
    pub save_step: Option<Option<f64>>,
}

pub(crate) fn specs_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: SpecsSet = parse(p)?;
    let given = [
        p.start.is_some(),
        p.stop.is_some(),
        p.dt.is_some(),
        p.dt_reciprocal.is_some(),
        p.method.is_some(),
        p.time_units.is_some(),
        p.save_step.is_some(),
    ];
    if !given.contains(&true) {
        return Err(invalid("nothing to change: give at least one setting"));
    }
    let diagnostics = e.sd_edit("sd.specs.set", |m| {
        set_specs(
            &mut m.specs,
            p.start,
            p.stop,
            p.dt,
            p.dt_reciprocal,
            p.method,
            p.time_units,
        )?;
        if let Some(s) = p.save_step {
            m.specs.save_step = s;
        }
        Ok(())
    })?;
    Ok(json!({ "diagnostics": diagnostics }))
}

/// A move or a resize of one shape.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeSet {
    /// The variable.
    pub name: String,
    /// The new centre; one of them alone moves the shape along one axis. A flow's pipe moves
    /// with its valve.
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    /// A stock's or auxiliary's size.
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    /// A flow's pipe, from the end that leaves its source to the end that enters its sink: at
    /// least two points, joined at right angles. The valve is put on the pipe.
    #[serde(default)]
    pub pts: Option<Vec<Position>>,
}

/// An information link, from the variable `from` to the one that uses it, `to`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub from: String,
    pub to: String,
    /// The angle in degrees at which the link leaves `from`.
    #[serde(default)]
    pub angle: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayoutSet {
    /// Shapes to move or resize.
    #[serde(default)]
    pub shapes: Option<Vec<ShapeSet>>,
    /// Links to add. (An equation that uses a variable gets its link when it is set.)
    #[serde(default)]
    pub add_connectors: Option<Vec<Link>>,
    /// Links to remove.
    #[serde(default)]
    pub remove_connectors: Option<Vec<Link>>,
}

/// Moves, resizes or re-pipes one shape of `m`'s diagram.
fn set_shape(m: &mut SdModel, s: &ShapeSet) -> Result<(), CommandError> {
    let idx = find_variable(m, &s.name).ok_or_else(|| not_found(&s.name))?;
    let canon = canonical(&s.name);
    let Some(kind) = m.vars.get(idx).map(|v| v.kind.clone()) else {
        return Ok(());
    };
    let is_flow = matches!(kind, VarKind::Flow { .. });
    let Some(old) = m.layout.shapes.get(&canon).cloned() else {
        return Err(invalid(format!("`{}` has no shape", s.name)));
    };
    if is_flow && (s.width.is_some() || s.height.is_some()) {
        return Err(invalid("a flow's valve has no size"));
    }
    if !is_flow && s.pts.is_some() {
        return Err(invalid("`pts` is a flow's pipe"));
    }
    let (mut cx, mut cy) = (old.x, old.y);
    if s.x.is_some() || s.y.is_some() {
        cx = finite(s.x.unwrap_or(old.x), "`x`")?;
        cy = finite(s.y.unwrap_or(old.y), "`y`")?;
        with_layout(m, |l, m| l.move_to(m, &canon, cx, cy));
    }
    if s.width.is_some() || s.height.is_some() {
        let (cw, ch) = old.size(&kind);
        let w = finite(s.width.unwrap_or(cw), "`width`")?;
        let h = finite(s.height.unwrap_or(ch), "`height`")?;
        if w <= 0.0 || h <= 0.0 {
            return Err(invalid("a size must be above 0"));
        }
        if let Some(sh) = m.layout.shapes.get_mut(&canon) {
            sh.width = Some(w);
            sh.height = Some(h);
        }
        // The pipes at a stock end on its edge.
        with_layout(m, |l, m| l.move_to(m, &canon, cx, cy));
    }
    if let Some(pts) = &s.pts {
        if pts.len() < 2 {
            return Err(invalid("a pipe needs at least two points"));
        }
        let mut list = Vec::with_capacity(pts.len());
        for q in pts {
            list.push((finite(q.x, "a point's `x`")?, finite(q.y, "a point's `y`")?));
        }
        let bent = list.windows(2).any(|w| match (w.first(), w.get(1)) {
            (Some(a), Some(b)) => (a.0 - b.0).abs() > 1e-6 && (a.1 - b.1).abs() > 1e-6,
            _ => false,
        });
        if bent {
            return Err(invalid("a pipe's points must be joined at right angles"));
        }
        m.layout.set_pipe(&canon, list);
    }
    Ok(())
}

pub(crate) fn layout_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: LayoutSet = parse(p)?;
    if p.shapes.is_none() && p.add_connectors.is_none() && p.remove_connectors.is_none() {
        return Err(invalid(
            "nothing to change: give `shapes`, `add_connectors` or `remove_connectors`",
        ));
    }
    let diagnostics = e.sd_edit("sd.layout.set", |m| {
        for s in p.shapes.iter().flatten() {
            set_shape(m, s)?;
        }
        for c in p.remove_connectors.iter().flatten() {
            let (from, to) = (canonical(&c.from), canonical(&c.to));
            let before = m.layout.connectors.len();
            m.layout
                .connectors
                .retain(|k| !(k.from == from && k.to == to));
            if m.layout.connectors.len() == before {
                return Err(CommandError::new(
                    ErrorCode::NotFound,
                    format!("no link from `{}` to `{}`", c.from, c.to),
                ));
            }
        }
        for c in p.add_connectors.iter().flatten() {
            for n in [&c.from, &c.to] {
                if find_variable(m, n).is_none() {
                    return Err(not_found(n));
                }
            }
            let (from, to) = (canonical(&c.from), canonical(&c.to));
            if from == to {
                return Err(invalid("a link joins two different variables"));
            }
            if let Some(a) = c.angle {
                finite(a, "`angle`")?;
            }
            match m
                .layout
                .connectors
                .iter_mut()
                .find(|k| k.from == from && k.to == to)
            {
                Some(k) => k.angle = c.angle,
                None => m.layout.connectors.push(Connector {
                    from,
                    to,
                    angle: c.angle,
                }),
            }
        }
        Ok(())
    })?;
    Ok(json!({ "diagnostics": diagnostics }))
}
