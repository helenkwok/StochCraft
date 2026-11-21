//! XMILE 1.0 (OASIS Standard, 2015): reading and writing a sensible core of it.
//!
//! **Read.** The header, `sim_specs` (`start`, `stop`, `dt` with its `reciprocal` attribute,
//! `method`, `time_units`), stocks (`eqn` as the initial value, `inflow`, `outflow`,
//! `non_negative`), flows, auxiliaries, graphical functions (`gf`, with `xscale` or `xpts`, and
//! `ypts`, of type continuous, extrapolate or discrete, also on their own in `variables`),
//! `units`, `doc`, and the `behavior` default of `non_negative`.
//!
//! **Not supported, and refused by name:** arrays (`dimensions`, `element`, `uses_arrays`),
//! modules and more than one `model`, macros, conveyors and queues, MathML equations, a flow's
//! `multiplier`, `overflow` and `leak`, `event_poster`, and any other element a variable or the
//! model does not have in XMILE 1.0. A method other than Euler or the Runge-Kutta family is
//! refused; RK2, RK45 and Gear run as RK4, as the specification allows, and the file says so in
//! its notes.
//!
//! **Not kept, and said so in the notes:** the diagrams (`views`), styles, display settings of a
//! variable (`range`, `scale`, `format`, `display`), another vendor's extensions (elements with
//! a prefix), `pause`, and groups. A file written back has the model and none of those.
//!
//! **StochCraft's extension:** the interval results are saved at, `stochcraft:save_step` in
//! `sim_specs`. Another tool ignores it, as XMILE allows for an element in another namespace.
//! The random functions take an optional seed (the specification's) and, after it, an interval
//! between draws: `NORMAL(mean, std_dev, seed, interval)`. `UNIFORM` is read as the
//! specification's `RANDOM`.

use crate::model::{Gf, GfKind, GfX, Method, SdModel, SimSpecs, Step, VarKind, Variable};
use crate::xml::{Element, escape, parse};
use crate::{ErrorKind, SdError};

/// The XMILE 1.0 namespace.
pub const NAMESPACE: &str = "http://docs.oasis-open.org/xmile/ns/XMILE/v1.0";
/// The namespace of StochCraft's extensions to a file.
pub const EXTENSION_NAMESPACE: &str = "https://github.com/helenkwok/StochCraft/ns/xmile";

/// A file read.
#[derive(Debug, Clone, PartialEq)]
pub struct XmileFile {
    pub model: SdModel,
    /// What the file has that was not kept, or was run another way.
    pub notes: Vec<String>,
}

fn xml_err(message: String) -> SdError {
    SdError {
        kind: ErrorKind::Xml,
        message,
    }
}

fn unsupported(message: String) -> SdError {
    SdError {
        kind: ErrorKind::Unsupported,
        message,
    }
}

/// Whether `bytes` look like an XMILE file: an `xmile` root element.
pub fn looks_like_xmile(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(4096)).unwrap_or_default();
    let text = String::from_utf8_lossy(head);
    text.contains("<xmile") || text.contains(":xmile")
}

fn number(text: &str, what: &str) -> Result<f64, SdError> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| xml_err(format!("{what}: `{}` is not a number", text.trim())))
}

fn list(text: &str, sep: Option<&str>, what: &str) -> Result<Vec<f64>, SdError> {
    let sep = sep.unwrap_or(",");
    text.split(sep)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| number(s, what))
        .collect()
}

/// Notes, each said once.
#[derive(Default)]
struct Notes(Vec<String>);

impl Notes {
    fn add(&mut self, note: String) {
        if !self.0.contains(&note) {
            self.0.push(note);
        }
    }
}

fn read_gf(g: &Element, owner: &str) -> Result<Gf, SdError> {
    let what = format!("the graphical function of `{owner}`");
    let kind = match g.attr("type").map(str::to_lowercase).as_deref() {
        None | Some("continuous") => GfKind::Continuous,
        Some("extrapolate") => GfKind::Extrapolate,
        Some("discrete") => GfKind::Discrete,
        Some(other) => {
            return Err(unsupported(format!(
                "{what} is of type `{other}`, which is not supported (continuous, extrapolate and discrete are)"
            )));
        }
    };
    let ypts = g
        .child("ypts")
        .ok_or_else(|| xml_err(format!("{what} has no <ypts>")))?;
    let y = list(&ypts.text(), ypts.attr("sep"), &what)?;
    let x = match (g.child("xscale"), g.child("xpts")) {
        (Some(s), None) => GfX::Scale {
            min: number(s.attr("min").unwrap_or_default(), &what)?,
            max: number(s.attr("max").unwrap_or_default(), &what)?,
        },
        (None, Some(p)) => GfX::Points(list(&p.text(), p.attr("sep"), &what)?),
        _ => {
            return Err(xml_err(format!(
                "{what} needs exactly one of <xscale> and <xpts>"
            )));
        }
    };
    Ok(Gf { kind, x, y })
}

/// Elements inside a variable that show it and change nothing about the numbers.
const DISPLAY: &[&str] = &[
    "range",
    "scale",
    "format",
    "display",
    "number_format",
    "access",
    "autoexport",
];
/// Elements inside a variable that this version does not run.
const NOT_RUN: &[(&str, &str)] = &[
    ("dimensions", "arrays"),
    ("element", "arrays"),
    ("conveyor", "conveyors"),
    ("queue", "queues"),
    ("mathml", "MathML equations"),
    ("multiplier", "a flow's multiplier"),
    ("overflow", "a queue's overflow"),
    ("leak", "a conveyor's leak"),
    ("leak_integers", "a conveyor's leak"),
    ("event_poster", "event posters"),
    ("connect", "modules"),
];

fn flag(e: &Element) -> bool {
    !e.text().eq_ignore_ascii_case("false")
}

fn read_variable(
    e: &Element,
    default_stock: bool,
    default_flow: bool,
    notes: &mut Notes,
) -> Result<Variable, SdError> {
    let name = e
        .attr("name")
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| xml_err(format!("a <{}> has no name", e.name)))?
        .to_owned();
    if e.name == "gf" {
        let g = read_gf(e, &name)?;
        return Ok(Variable {
            name,
            kind: VarKind::Aux,
            eqn: None,
            gf: Some(g),
            units: None,
            doc: None,
        });
    }
    let mut eqn = None;
    let mut units = None;
    let mut doc = None;
    let mut gf = None;
    let mut inflows = Vec::new();
    let mut outflows = Vec::new();
    let mut non_negative = None;
    for c in e.elements() {
        if let Some(p) = &c.prefix {
            notes.add(format!(
                "the extension element <{p}:{}> of `{name}` is not kept",
                c.name
            ));
            continue;
        }
        match c.name.as_str() {
            "eqn" => {
                let t = c.text();
                if !t.is_empty() {
                    eqn = Some(t);
                }
            }
            "units" => {
                let t = c.text();
                if !t.is_empty() {
                    units = Some(t);
                }
            }
            "doc" => {
                let t = c.text();
                if !t.is_empty() {
                    doc = Some(t);
                }
            }
            "gf" => gf = Some(read_gf(c, &name)?),
            "inflow" if e.name == "stock" => inflows.push(c.text()),
            "outflow" if e.name == "stock" => outflows.push(c.text()),
            "non_negative" if e.name != "aux" && e.name != "gf" => non_negative = Some(flag(c)),
            n if DISPLAY.contains(&n) => {
                notes.add(format!(
                    "the display setting <{n}> of a variable is not kept"
                ));
            }
            n => {
                return match NOT_RUN.iter().find(|(k, _)| *k == n) {
                    Some((_, what)) => Err(unsupported(format!(
                        "`{name}` uses {what} (<{n}>), which is not supported in this version"
                    ))),
                    None => Err(unsupported(format!(
                        "`{name}` has an element <{n}> that XMILE 1.0 does not have there, or that is not supported"
                    ))),
                };
            }
        }
    }
    let kind = match e.name.as_str() {
        "stock" => VarKind::Stock {
            inflows,
            outflows,
            non_negative: non_negative.unwrap_or(default_stock),
        },
        "flow" => VarKind::Flow {
            non_negative: non_negative.unwrap_or(default_flow),
        },
        _ => VarKind::Aux,
    };
    Ok(Variable {
        name,
        kind,
        eqn,
        gf,
        units,
        doc,
    })
}

fn read_specs(s: &Element, notes: &mut Notes) -> Result<SimSpecs, SdError> {
    let time_units = s.attr("time_units").unwrap_or_default().trim().to_owned();
    let method = match s.attr("method").map(str::to_lowercase).as_deref() {
        None | Some("euler") => Method::Euler,
        Some("rk4") => Method::Rk4,
        Some(m @ ("rk2" | "rk45" | "gear")) => {
            notes.add(format!(
                "the method {m} is run as RK4, as XMILE allows a tool without it to do"
            ));
            Method::Rk4
        }
        Some(other) => {
            return Err(unsupported(format!(
                "the integration method `{other}` is not supported (euler and rk4 are)"
            )));
        }
    };
    let mut specs = SimSpecs {
        time_units,
        method,
        ..SimSpecs::default()
    };
    let start = s
        .child("start")
        .ok_or_else(|| xml_err("sim_specs has no <start>".to_owned()))?;
    let stop = s
        .child("stop")
        .ok_or_else(|| xml_err("sim_specs has no <stop>".to_owned()))?;
    specs.start = number(&start.text(), "<start>")?;
    specs.stop = number(&stop.text(), "<stop>")?;
    if let Some(dt) = s.child("dt") {
        let v = number(&dt.text(), "<dt>")?;
        specs.dt = if dt
            .attr("reciprocal")
            .is_some_and(|r| r.eq_ignore_ascii_case("true"))
        {
            Step::Reciprocal(v)
        } else {
            Step::Value(v)
        };
    }
    for c in s.elements() {
        match (&c.prefix, c.name.as_str()) {
            (None, "start" | "stop" | "dt") => {}
            (None, "pause") => notes.add("the <pause> setting is not kept".to_owned()),
            (None, "run") => notes.add("the <run> setting is not kept".to_owned()),
            (Some(_), "save_step") => specs.save_step = Some(number(&c.text(), "<save_step>")?),
            (Some(p), n) => notes.add(format!("the extension element <{p}:{n}> is not kept")),
            (None, n) => {
                return Err(unsupported(format!(
                    "sim_specs has an element <{n}> that XMILE 1.0 does not have"
                )));
            }
        }
    }
    Ok(specs)
}

/// The defaults of `non_negative` that a `behavior` element gives: for stocks and for flows.
fn read_behavior(b: &Element, notes: &mut Notes) -> (Option<bool>, Option<bool>) {
    let (mut stock, mut flow) = (None, None);
    for c in b.elements() {
        match c.name.as_str() {
            "non_negative" => {
                let f = flag(c);
                stock = Some(f);
                flow = Some(f);
            }
            "stock" | "flow" => {
                for d in c.children_named("non_negative") {
                    if c.name == "stock" {
                        stock = Some(flag(d));
                    } else {
                        flow = Some(flag(d));
                    }
                }
            }
            n => notes.add(format!("the <{n}> default of <behavior> is not kept")),
        }
    }
    (stock, flow)
}

/// Reads an XMILE file.
pub fn read_xmile(text: &str) -> Result<XmileFile, SdError> {
    let root = parse(text).map_err(xml_err)?;
    if root.name != "xmile" {
        return Err(xml_err(format!(
            "the root element is <{}>, not <xmile>",
            root.full_name()
        )));
    }
    if let Some(v) = root.attr("version")
        && !v.trim().starts_with('1')
    {
        return Err(unsupported(format!(
            "this is XMILE version {v}; version 1.0 is read"
        )));
    }
    let mut notes = Notes::default();
    let mut model = SdModel::default();
    let mut root_specs = None;
    let mut models = Vec::new();
    let mut root_behavior = (None, None);
    for c in root.elements() {
        if let Some(p) = &c.prefix {
            notes.add(format!(
                "the extension element <{p}:{}> is not kept",
                c.name
            ));
            continue;
        }
        match c.name.as_str() {
            "header" => {
                for h in c.elements() {
                    match (h.prefix.is_some(), h.name.as_str()) {
                        (true, n) => notes.add(format!("the header extension <{n}> is not kept")),
                        (false, "name") => model.name = h.text(),
                        (false, "vendor") => model.vendor = h.text(),
                        (false, "product") => {
                            model.product = h.text();
                            model.product_version =
                                h.attr("version").unwrap_or_default().to_owned();
                        }
                        (false, "options") => {
                            for o in h.elements() {
                                match o.name.as_str() {
                                    "uses_arrays" => {
                                        return Err(unsupported(
                                            "the file uses arrays (<uses_arrays>), which are not supported in this version"
                                                .to_owned(),
                                        ));
                                    }
                                    "uses_macros" => {
                                        return Err(unsupported(
                                            "the file uses macros (<uses_macros>), which are not supported"
                                                .to_owned(),
                                        ));
                                    }
                                    "uses_submodels" => {
                                        return Err(unsupported(
                                            "the file uses modules (<uses_submodels>), which are not supported"
                                                .to_owned(),
                                        ));
                                    }
                                    n => notes.add(format!("the header option <{n}> is not kept")),
                                }
                            }
                        }
                        (false, n) => notes.add(format!("the header element <{n}> is not kept")),
                    }
                }
            }
            "sim_specs" => root_specs = Some(read_specs(c, &mut notes)?),
            "model" => models.push(c),
            "behavior" => root_behavior = read_behavior(c, &mut notes),
            "dimensions" => {
                return Err(unsupported(
                    "the file defines arrays (<dimensions>), which are not supported in this version"
                        .to_owned(),
                ));
            }
            "macro" => {
                return Err(unsupported(
                    "the file defines macros (<macro>), which are not supported".to_owned(),
                ));
            }
            "style" | "model_units" | "data" => {
                notes.add(format!("the <{}> section is not kept", c.name));
            }
            n => {
                return Err(unsupported(format!(
                    "the file has a <{n}> element, which XMILE 1.0 does not have at its top level, or that is not supported"
                )));
            }
        }
    }
    let mut it = models.into_iter();
    let Some(m) = it.next() else {
        return Err(xml_err("the file has no <model>".to_owned()));
    };
    if it.next().is_some() {
        return Err(unsupported(
            "the file has more than one <model> (modules), which is not supported in this version"
                .to_owned(),
        ));
    }
    let mut behavior = root_behavior;
    let mut specs = root_specs;
    let mut variables = None;
    for c in m.elements() {
        if let Some(p) = &c.prefix {
            notes.add(format!(
                "the extension element <{p}:{}> is not kept",
                c.name
            ));
            continue;
        }
        match c.name.as_str() {
            "variables" => variables = Some(c),
            "views" => notes.add("the diagrams (<views>) are not kept".to_owned()),
            "sim_specs" => specs = Some(read_specs(c, &mut notes)?),
            "behavior" => behavior = read_behavior(c, &mut notes),
            n => {
                return Err(unsupported(format!(
                    "the model has a <{n}> element that is not supported"
                )));
            }
        }
    }
    model.specs = specs.ok_or_else(|| xml_err("the file has no <sim_specs>".to_owned()))?;
    let (default_stock, default_flow) = (behavior.0.unwrap_or(false), behavior.1.unwrap_or(false));
    if let Some(vars) = variables {
        for v in vars.elements() {
            if let Some(p) = &v.prefix {
                notes.add(format!(
                    "the extension element <{p}:{}> is not kept",
                    v.name
                ));
                continue;
            }
            match v.name.as_str() {
                "stock" | "flow" | "aux" | "gf" => {
                    model
                        .vars
                        .push(read_variable(v, default_stock, default_flow, &mut notes)?);
                }
                "module" => {
                    return Err(unsupported(format!(
                        "the file has a module `{}`, which is not supported in this version",
                        v.attr("name").unwrap_or_default()
                    )));
                }
                "group" => notes.add("groups of variables are not kept".to_owned()),
                n => {
                    return Err(unsupported(format!(
                        "the file has a <{n}> among the variables, which is not supported"
                    )));
                }
            }
        }
    }
    Ok(XmileFile {
        model,
        notes: notes.0,
    })
}

fn push_text(out: &mut String, indent: usize, tag: &str, text: &str) {
    out.push_str(&"  ".repeat(indent));
    out.push_str(&format!("<{tag}>{}</{tag}>\n", escape(text)));
}

fn write_list(values: &[f64]) -> String {
    values
        .iter()
        .map(|v| format!("{v}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn write_gf(out: &mut String, g: &Gf, indent: usize) {
    let pad = "  ".repeat(indent);
    out.push_str(&format!("{pad}<gf type=\"{}\">\n", g.kind.name()));
    match &g.x {
        GfX::Scale { min, max } => {
            out.push_str(&format!("{pad}  <xscale min=\"{min}\" max=\"{max}\"/>\n"));
        }
        GfX::Points(p) => out.push_str(&format!("{pad}  <xpts>{}</xpts>\n", write_list(p))),
    }
    out.push_str(&format!("{pad}  <ypts>{}</ypts>\n", write_list(&g.y)));
    out.push_str(&format!("{pad}</gf>\n"));
}

/// Writes `model` as an XMILE 1.0 file.
pub fn write_xmile(model: &SdModel) -> String {
    let mut o = String::new();
    o.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    o.push_str(&format!(
        "<xmile version=\"1.0\" xmlns=\"{NAMESPACE}\" xmlns:stochcraft=\"{EXTENSION_NAMESPACE}\">\n"
    ));
    o.push_str("  <header>\n");
    if !model.vendor.is_empty() {
        push_text(&mut o, 2, "vendor", &model.vendor);
    }
    if !model.product.is_empty() || !model.product_version.is_empty() {
        o.push_str(&format!(
            "    <product version=\"{}\">{}</product>\n",
            escape(&model.product_version),
            escape(&model.product)
        ));
    }
    if !model.name.is_empty() {
        push_text(&mut o, 2, "name", &model.name);
    }
    o.push_str("  </header>\n");
    let s = &model.specs;
    let method = match s.method {
        Method::Euler => "Euler",
        Method::Rk4 => "RK4",
    };
    o.push_str(&format!("  <sim_specs method=\"{method}\""));
    if !s.time_units.is_empty() {
        o.push_str(&format!(" time_units=\"{}\"", escape(&s.time_units)));
    }
    o.push_str(">\n");
    push_text(&mut o, 2, "start", &format!("{}", s.start));
    push_text(&mut o, 2, "stop", &format!("{}", s.stop));
    match s.dt {
        Step::Value(v) => push_text(&mut o, 2, "dt", &format!("{v}")),
        Step::Reciprocal(r) => {
            o.push_str(&format!("    <dt reciprocal=\"true\">{r}</dt>\n"));
        }
    }
    if let Some(v) = s.save_step {
        push_text(&mut o, 2, "stochcraft:save_step", &format!("{v}"));
    }
    o.push_str("  </sim_specs>\n  <model>\n    <variables>\n");
    for v in &model.vars {
        let tag = match v.kind {
            VarKind::Stock { .. } => "stock",
            VarKind::Flow { .. } => "flow",
            VarKind::Aux => "aux",
        };
        o.push_str(&format!("      <{tag} name=\"{}\">\n", escape(&v.name)));
        if let Some(e) = &v.eqn {
            push_text(&mut o, 4, "eqn", e);
        }
        if let VarKind::Stock {
            inflows, outflows, ..
        } = &v.kind
        {
            for f in inflows {
                push_text(&mut o, 4, "inflow", f);
            }
            for f in outflows {
                push_text(&mut o, 4, "outflow", f);
            }
        }
        let nn = match &v.kind {
            VarKind::Stock { non_negative, .. } | VarKind::Flow { non_negative } => *non_negative,
            VarKind::Aux => false,
        };
        if nn {
            o.push_str("        <non_negative/>\n");
        }
        if let Some(u) = &v.units {
            push_text(&mut o, 4, "units", u);
        }
        if let Some(d) = &v.doc {
            push_text(&mut o, 4, "doc", d);
        }
        if let Some(g) = &v.gf {
            write_gf(&mut o, g, 4);
        }
        o.push_str(&format!("      </{tag}>\n"));
    }
    o.push_str("    </variables>\n  </model>\n</xmile>\n");
    o
}
