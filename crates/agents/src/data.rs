//! The `stochcraft` data model's functions (ROADMAP D17) and SCXML's event matching.
//!
//! Expressions are BPSim's (`stochcraft_model::expr`, read with variables: `$x` or
//! `getProperty('x')`), with these functions beside XPath's:
//!
//! - `In('id')`: whether the state `id` is active (SCXML §5.9.1).
//! - Random draws, each from the stream of the expression it is in: `random()` (uniform on
//!   [0, 1)), `uniform(min, max)`, `exponential(mean)`, `triangular(min, mode, max)`,
//!   `normal(mean, sd)`, `truncated-normal(mean, sd, min, max)`, `lognormal(mean, sd)` (its own
//!   mean and standard deviation), `gamma(shape, scale)`, `beta(a, b)`, `erlang(mean, k)`,
//!   `weibull(shape, scale)`, `poisson(mean)` and `binomial(p, n)`: the distributions of
//!   `stochcraft_random::Dist`, sampled as it samples them. Parameters it refuses give NaN.
//!
//! `getProperty('_event.name')` reads the name of the event being handled (SCXML's
//! `_event.name`).

use std::cell::RefCell;

use stochcraft_model::expr::{Expr, Provided, Value};
use stochcraft_random::{Dist, Rng};

/// The name under which an expression reads the current event's name.
pub(crate) const EVENT_NAME: &str = "_event.name";

const fn f(name: &'static str, min: usize, max: usize) -> Provided<'static> {
    Provided {
        name,
        min,
        max: Some(max),
    }
}

/// The functions a chart's expressions may call beyond the expression language's own.
pub const FUNCTIONS: &[Provided<'static>] = &[
    f("In", 1, 1),
    f("random", 0, 0),
    f("uniform", 2, 2),
    f("exponential", 1, 1),
    f("triangular", 3, 3),
    f("normal", 2, 2),
    f("truncated-normal", 4, 4),
    f("lognormal", 2, 2),
    f("gamma", 2, 2),
    f("beta", 2, 2),
    f("erlang", 2, 2),
    f("weibull", 2, 2),
    f("poisson", 1, 1),
    f("binomial", 2, 2),
];

/// A draw from the distribution function `name` with these arguments, or `None` if `name` is
/// not one.
pub(crate) fn draw(name: &str, args: &[Value], rng: &RefCell<Rng>) -> Option<Value> {
    let x: Vec<f64> = args.iter().map(Value::number).collect();
    let a = |i: usize| x.get(i).copied().unwrap_or(f64::NAN);
    // A whole count, as a u64.
    let count =
        |v: f64| (v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v <= 1e15).then_some(v as u64);
    let nan = Some(Value::Number(f64::NAN));
    let dist = match name {
        "random" => return Some(Value::Number(rng.borrow_mut().uniform())),
        "uniform" => Dist::Uniform {
            min: a(0),
            max: a(1),
        },
        "exponential" => Dist::Exponential { mean: a(0) },
        "triangular" => Dist::Triangular {
            min: a(0),
            mode: a(1),
            max: a(2),
        },
        "normal" => Dist::Normal {
            mean: a(0),
            std_dev: a(1),
        },
        "truncated-normal" => Dist::TruncatedNormal {
            mean: a(0),
            std_dev: a(1),
            min: a(2),
            max: a(3),
        },
        "lognormal" => Dist::LogNormal {
            mean: a(0),
            std_dev: a(1),
        },
        "gamma" => Dist::Gamma {
            shape: a(0),
            scale: a(1),
        },
        "beta" => Dist::Beta {
            shape: a(0),
            scale: a(1),
        },
        "erlang" => match count(a(1)) {
            Some(k) => Dist::Erlang { mean: a(0), k },
            None => return nan,
        },
        "weibull" => Dist::Weibull {
            shape: a(0),
            scale: a(1),
        },
        "poisson" => Dist::Poisson { mean: a(0) },
        "binomial" => match count(a(1)) {
            Some(trials) => Dist::Binomial {
                probability: a(0),
                trials,
            },
            None => return nan,
        },
        _ => return None,
    };
    if dist.check().is_err() {
        return nan;
    }
    let v = dist.sample(&mut rng.borrow_mut());
    Some(Value::Number(v))
}

/// Visits `e` and every expression inside it.
pub(crate) fn walk<'a>(e: &'a Expr, f: &mut dyn FnMut(&'a Expr)) {
    f(e);
    match e {
        Expr::Neg(x) => walk(x, f),
        Expr::Binary(_, a, b) => {
            walk(a, f);
            walk(b, f);
        }
        Expr::Call(_, args) | Expr::Provided(_, args) => {
            for a in args {
                walk(a, f);
            }
        }
        _ => {}
    }
}

/// An event descriptor as it is matched (§3.12.1): `*` stays, and a trailing `.*` or `.` is
/// dropped (`error.*`, `error.` and `error` all match the same events).
pub(crate) fn descriptor(d: &str) -> String {
    if d == "*" {
        return d.to_string();
    }
    let d = d.strip_suffix(".*").unwrap_or(d);
    d.strip_suffix('.').unwrap_or(d).to_string()
}

/// Whether a normalised descriptor matches an event name: `*` matches every event; otherwise
/// the descriptor's tokens must be the name's first tokens (`buy` matches `buy` and `buy.now`,
/// not `buyer`).
pub(crate) fn matches(descriptor: &str, name: &str) -> bool {
    descriptor == "*"
        || name == descriptor
        || name
            .strip_prefix(descriptor)
            .is_some_and(|rest| rest.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_match_whole_tokens() {
        let m = |d: &str, n: &str| matches(&descriptor(d), n);
        assert!(m("buy", "buy"));
        assert!(m("buy", "buy.now"));
        assert!(m("buy.*", "buy.now"));
        assert!(m("buy.", "buy.now"));
        assert!(!m("buy", "buyer"));
        assert!(!m("buy.now", "buy"));
        assert!(m("*", "anything.at.all"));
        assert!(m("error", "error.execution"));
        assert!(!m("error.send", "error.execution"));
    }

    #[test]
    fn draws_refuse_bad_parameters() {
        let rng = RefCell::new(Rng::new(1));
        let n = |name: &str, args: &[f64]| {
            let args: Vec<Value> = args.iter().map(|&v| Value::Number(v)).collect();
            draw(name, &args, &rng).map(|v| v.number())
        };
        assert!(n("exponential", &[-1.0]).is_some_and(f64::is_nan));
        assert!(n("erlang", &[1.0, 1.5]).is_some_and(f64::is_nan));
        assert!(n("uniform", &[2.0, 3.0]).is_some_and(|v| (2.0..3.0).contains(&v)));
        assert!(n("random", &[]).is_some_and(|v| (0.0..1.0).contains(&v)));
        assert_eq!(n("In", &[1.0]), None);
    }
}
