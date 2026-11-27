//! The input table as CSV: one input per line, under the header `name,distribution,parameters,unit`.
//!
//! `distribution` is the family's name as the engine's commands write it (`normal`,
//! `triangular`, `log_normal`…) and `parameters` its parameters as `name=value`, separated by
//! `;`, with the names the engine uses (`mean=10;std_dev=2`). A user distribution's points are
//! `points=probability:value|probability:value` with `discrete=true` or `false`. A field holding
//! a comma or a double quote is quoted, as RFC 4180 says.
//!
//! ```text
//! name,distribution,parameters,unit
//! labour,triangular,min=80;mode=100;max=150,h
//! materials,normal,mean=500;std_dev=50,USD
//! ```

use std::collections::BTreeMap;

use stochcraft_model::{Dist, McInput};

/// The header line.
pub const HEADER: &str = "name,distribution,parameters,unit";

fn num(v: f64) -> String {
    let s = format!("{v}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_owned)
}

/// A distribution's family name and parameters, as the table writes them.
pub fn describe(d: &Dist) -> (&'static str, String) {
    let pairs = |ps: &[(&str, f64)]| {
        ps.iter()
            .map(|(k, v)| format!("{k}={}", num(*v)))
            .collect::<Vec<_>>()
            .join(";")
    };
    match d {
        Dist::Constant(v) => ("constant", pairs(&[("value", *v)])),
        Dist::Exponential { mean } => ("exponential", pairs(&[("mean", *mean)])),
        Dist::Uniform { min, max } => ("uniform", pairs(&[("min", *min), ("max", *max)])),
        Dist::Triangular { min, mode, max } => (
            "triangular",
            pairs(&[("min", *min), ("mode", *mode), ("max", *max)]),
        ),
        Dist::Normal { mean, std_dev } => {
            ("normal", pairs(&[("mean", *mean), ("std_dev", *std_dev)]))
        }
        Dist::TruncatedNormal {
            mean,
            std_dev,
            min,
            max,
        } => (
            "truncated_normal",
            pairs(&[
                ("mean", *mean),
                ("std_dev", *std_dev),
                ("min", *min),
                ("max", *max),
            ]),
        ),
        Dist::LogNormal { mean, std_dev } => (
            "log_normal",
            pairs(&[("mean", *mean), ("std_dev", *std_dev)]),
        ),
        Dist::Gamma { shape, scale } => ("gamma", pairs(&[("shape", *shape), ("scale", *scale)])),
        // On [0, 1] as BPSim's; a range only where it is another.
        Dist::Beta {
            shape,
            scale,
            min,
            max,
        } if *min == 0.0 && *max == 1.0 => ("beta", pairs(&[("shape", *shape), ("scale", *scale)])),
        Dist::Beta {
            shape,
            scale,
            min,
            max,
        } => (
            "beta",
            pairs(&[
                ("shape", *shape),
                ("scale", *scale),
                ("min", *min),
                ("max", *max),
            ]),
        ),
        Dist::Weibull { shape, scale } => {
            ("weibull", pairs(&[("shape", *shape), ("scale", *scale)]))
        }
        Dist::Erlang { mean, k } => ("erlang", format!("mean={};k={k}", num(*mean))),
        Dist::Poisson { mean } => ("poisson", pairs(&[("mean", *mean)])),
        Dist::Binomial {
            probability,
            trials,
        } => (
            "binomial",
            format!("probability={};trials={trials}", num(*probability)),
        ),
        Dist::User { points, discrete } => {
            let pts = points
                .iter()
                .map(|(p, v)| format!("{}:{}", num(*p), num(*v)))
                .collect::<Vec<_>>()
                .join("|");
            ("user", format!("points={pts};discrete={discrete}"))
        }
    }
}

/// A distribution from its family name and parameters.
pub fn distribution(family: &str, parameters: &str) -> Result<Dist, String> {
    let mut map: BTreeMap<&str, &str> = BTreeMap::new();
    for pair in parameters
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| format!("`{pair}` is not `name=value`"))?;
        if map.insert(k.trim(), v.trim()).is_some() {
            return Err(format!("`{}` is given twice", k.trim()));
        }
    }
    let mut take = |k: &str| -> Result<String, String> {
        map.remove(k)
            .map(str::to_owned)
            .ok_or_else(|| format!("{family} needs `{k}`"))
    };
    let number = |s: String, k: &str| -> Result<f64, String> {
        s.parse::<f64>()
            .map_err(|_| format!("`{k}` = `{s}` is not a number"))
    };
    let whole = |s: String, k: &str| -> Result<u64, String> {
        s.parse::<u64>()
            .map_err(|_| format!("`{k}` = `{s}` is not a whole number"))
    };
    let mut f = |k: &str| take(k).and_then(|s| number(s, k));
    let d = match family {
        "constant" => Dist::Constant(f("value")?),
        "exponential" => Dist::Exponential { mean: f("mean")? },
        "uniform" => Dist::Uniform {
            min: f("min")?,
            max: f("max")?,
        },
        "triangular" => Dist::Triangular {
            min: f("min")?,
            mode: f("mode")?,
            max: f("max")?,
        },
        "normal" => Dist::Normal {
            mean: f("mean")?,
            std_dev: f("std_dev")?,
        },
        "truncated_normal" => Dist::TruncatedNormal {
            mean: f("mean")?,
            std_dev: f("std_dev")?,
            min: f("min")?,
            max: f("max")?,
        },
        "log_normal" => Dist::LogNormal {
            mean: f("mean")?,
            std_dev: f("std_dev")?,
        },
        "gamma" => Dist::Gamma {
            shape: f("shape")?,
            scale: f("scale")?,
        },
        "beta" => {
            let (shape, scale) = (f("shape")?, f("scale")?);
            // On [0, 1] unless a range is given.
            let min = match take("min") {
                Ok(s) => number(s, "min")?,
                Err(_) => 0.0,
            };
            let max = match take("max") {
                Ok(s) => number(s, "max")?,
                Err(_) => 1.0,
            };
            Dist::Beta {
                shape,
                scale,
                min,
                max,
            }
        }
        "weibull" => Dist::Weibull {
            shape: f("shape")?,
            scale: f("scale")?,
        },
        "poisson" => Dist::Poisson { mean: f("mean")? },
        "erlang" => {
            let mean = f("mean")?;
            let k = take("k").and_then(|s| whole(s, "k"))?;
            Dist::Erlang { mean, k }
        }
        "binomial" => {
            let probability = f("probability")?;
            let trials = take("trials").and_then(|s| whole(s, "trials"))?;
            Dist::Binomial {
                probability,
                trials,
            }
        }
        "user" => {
            let pts = take("points")?;
            let mut points = Vec::new();
            for p in pts.split('|').map(str::trim).filter(|p| !p.is_empty()) {
                let (pr, v) = p
                    .split_once(':')
                    .ok_or_else(|| format!("point `{p}` is not `probability:value`"))?;
                points.push((
                    number(pr.trim().to_owned(), "probability")?,
                    number(v.trim().to_owned(), "value")?,
                ));
            }
            let discrete = match take("discrete").as_deref() {
                Ok("true") => true,
                Ok("false") | Err(_) => false,
                Ok(other) => return Err(format!("`discrete` = `{other}` is not true or false")),
            };
            Dist::User { points, discrete }
        }
        other => return Err(format!("`{other}` is not a distribution StochCraft knows")),
    };
    if let Some(k) = map.keys().next() {
        return Err(format!("{family} has no parameter `{k}`"));
    }
    Ok(d)
}

/// A field, quoted if it holds a comma, a double quote or a line break.
fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// The inputs as a CSV table, with its header.
pub fn write(inputs: &[McInput]) -> String {
    let mut out = format!("{HEADER}\n");
    for i in inputs {
        let (family, params) = describe(&i.dist);
        out.push_str(&format!(
            "{},{},{},{}\n",
            field(&i.name),
            family,
            field(&params),
            field(&i.unit)
        ));
    }
    out
}

/// One line's fields: separated by commas, a quoted field may hold commas and doubled quotes.
fn row(line: &str, line_no: usize) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cell).trim().to_owned()),
            c => cell.push(c),
        }
    }
    if quoted {
        return Err(format!("line {line_no}: a double quote is not closed"));
    }
    out.push(cell.trim().to_owned());
    Ok(out)
}

/// The inputs a CSV table holds. The header must be `name,distribution,parameters,unit` (the
/// unit column may be left out); blank lines are skipped. An error names its line.
pub fn read(text: &str) -> Result<Vec<McInput>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .filter(|(_, l)| !l.trim().is_empty());
    let Some((n, header)) = lines.next() else {
        return Err(format!(
            "the table is empty: it needs the header `{HEADER}`"
        ));
    };
    let h = row(header, n)?;
    let h: Vec<&str> = h.iter().map(String::as_str).collect();
    if !(h == ["name", "distribution", "parameters", "unit"]
        || h == ["name", "distribution", "parameters"])
    {
        return Err(format!(
            "line {n}: the header must be `{HEADER}`, not `{}`",
            header.trim()
        ));
    }
    let mut inputs = Vec::new();
    for (n, line) in lines {
        let fields = row(line, n)?;
        if !(3..=4).contains(&fields.len()) {
            return Err(format!(
                "line {n}: expected 4 fields (`{HEADER}`), found {}",
                fields.len()
            ));
        }
        let dist = distribution(&fields[1], &fields[2]).map_err(|e| format!("line {n}: {e}"))?;
        inputs.push(McInput {
            name: fields[0].clone(),
            dist,
            unit: fields.get(3).cloned().unwrap_or_default(),
        });
    }
    Ok(inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_goes_through_the_table_and_back() {
        let dists = [
            Dist::Constant(-3.5),
            Dist::Exponential { mean: 2.0 },
            Dist::Uniform { min: 1.0, max: 2.5 },
            Dist::Triangular {
                min: 80.0,
                mode: 100.0,
                max: 150.0,
            },
            Dist::Normal {
                mean: 500.0,
                std_dev: 50.0,
            },
            Dist::TruncatedNormal {
                mean: 1.0,
                std_dev: 2.0,
                min: 0.0,
                max: 9.0,
            },
            Dist::LogNormal {
                mean: 3.0,
                std_dev: 0.1,
            },
            Dist::Gamma {
                shape: 2.0,
                scale: 0.25,
            },
            Dist::beta(2.0, 5.0),
            Dist::Beta {
                shape: 3.0,
                scale: 5.0,
                min: 1.0,
                max: 4.0,
            },
            Dist::Weibull {
                shape: 1.5,
                scale: 4.0,
            },
            Dist::Erlang { mean: 6.0, k: 3 },
            Dist::Poisson { mean: 4.0 },
            Dist::Binomial {
                probability: 0.3,
                trials: 10,
            },
            Dist::User {
                points: vec![(0.25, 1.0), (0.75, 4.0)],
                discrete: true,
            },
        ];
        let inputs: Vec<McInput> = dists
            .iter()
            .enumerate()
            .map(|(i, d)| McInput {
                name: format!("x{i}"),
                dist: d.clone(),
                unit: if i == 1 {
                    "USD, 2026".into()
                } else {
                    "h".into()
                },
            })
            .collect();
        let text = write(&inputs);
        assert!(text.starts_with(HEADER));
        assert!(
            text.contains("x3,triangular,min=80;mode=100;max=150,h"),
            "{text}"
        );
        assert_eq!(read(&text), Ok(inputs));
    }

    #[test]
    fn errors_name_their_line() {
        assert!(read("").is_err());
        assert!(read("a,b\n").is_err_and(|e| e.contains("line 1")));
        let bad = format!("{HEADER}\nx,normal,mean=1,h\n");
        assert!(read(&bad).is_err_and(|e| e.contains("line 2") && e.contains("std_dev")));
        let extra = format!("{HEADER}\nx,uniform,min=1;max=2;mode=3,h\n");
        assert!(read(&extra).is_err_and(|e| e.contains("mode")));
        let unknown = format!("{HEADER}\nx,cauchy,a=1,h\n");
        assert!(read(&unknown).is_err_and(|e| e.contains("cauchy")));
        // Without a unit column, and with a byte order mark and blank lines.
        let short = "\u{feff}name,distribution,parameters\n\nx,constant,value=2\n";
        assert_eq!(
            read(short),
            Ok(vec![McInput {
                name: "x".into(),
                dist: Dist::Constant(2.0),
                unit: String::new(),
            }])
        );
    }
}
