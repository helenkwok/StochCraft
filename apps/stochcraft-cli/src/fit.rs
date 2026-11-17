//! `stochcraft-cli fit`: one column of measured values from a CSV file (or a plain list, one
//! value per line) fitted to distributions by the engine's `estimate.fit`.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::Path;

use serde_json::{Value, json};
use stochcraft_engine::Engine;

use crate::output::number;
use crate::{Failure, Format};

/// The values read, and what they were read from (for the table's first line).
pub struct Column {
    pub values: Vec<f64>,
    pub label: String,
    /// Rows whose cell in the column was empty, left out.
    pub empty: usize,
}

/// One line of delimited text as cells: double quotes around a cell are removed, and `""`
/// inside them is a quote.
fn cells(line: &str, delimiter: char) -> Vec<String> {
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
            c if c == delimiter && !quoted => out.push(std::mem::take(&mut cell)),
            c => cell.push(c),
        }
    }
    out.push(cell);
    out.into_iter().map(|c| c.trim().to_owned()).collect()
}

fn value(cell: &str) -> Option<f64> {
    cell.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The column `column` (a header, or a number from 1) of `text`; with none given, the only
/// column. The first row is a header when its cell in the column is not a number. The cells are
/// separated by tabs if the first row has one, by semicolons if it has those and no commas, and
/// otherwise by commas.
pub fn read_column(text: &str, column: Option<&str>) -> Result<Column, Failure> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rows: Vec<(usize, &str)> = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim_end_matches('\r')))
        .filter(|(_, l)| !l.trim().is_empty())
        .collect();
    let Some(&(_, first)) = rows.first() else {
        return Err(Failure("no values: the input is empty".into()));
    };
    let delimiter = if first.contains('\t') {
        '\t'
    } else if first.contains(';') && !first.contains(',') {
        ';'
    } else {
        ','
    };
    let head = cells(first, delimiter);
    let (index, label) = match column {
        None if head.len() == 1 => (0, String::new()),
        None => {
            return Err(Failure(format!(
                "{} columns: choose one with --column (a header or a number from 1): {}",
                head.len(),
                head.join(", ")
            )));
        }
        Some(name) => match head.iter().position(|h| h == name) {
            Some(i) => (i, format!("column {name}")),
            None => match name.parse::<usize>() {
                Ok(n) if (1..=head.len()).contains(&n) => (n - 1, format!("column {n}")),
                _ => {
                    return Err(Failure(format!(
                        "no column `{name}`: give a header or a number from 1 to {} ({})",
                        head.len(),
                        head.join(", ")
                    )));
                }
            },
        },
    };
    let has_header = head.get(index).is_some_and(|c| value(c).is_none());
    let label = match (has_header, label.is_empty()) {
        (true, true) => format!("column {}", head.get(index).map_or("", String::as_str)),
        _ => label,
    };
    let mut values = Vec::new();
    let mut empty = 0;
    for &(line, row) in rows.iter().skip(usize::from(has_header)) {
        let cs = cells(row, delimiter);
        match cs.get(index).map(String::as_str) {
            None | Some("") => empty += 1,
            Some(c) => values
                .push(value(c).ok_or_else(|| {
                    Failure(format!("line {line}: `{c}` is not a finite number"))
                })?),
        }
    }
    Ok(Column {
        values,
        label,
        empty,
    })
}

fn n(v: &Value) -> String {
    v.as_f64().map_or_else(|| "inf".into(), number)
}

/// A distribution as words: its parameters in the order of its JSON form, `type` left out.
fn describe(d: &Value) -> String {
    let params: Vec<String> = d
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| k.as_str() != "type")
        .map(|(k, v)| format!("{} {}", k.replace('_', " "), n(v)))
        .collect();
    params.join(", ")
}

fn table(source: &str, col: &Column, r: &Value) -> String {
    let mut out = String::new();
    let d = &r["data"];
    let from = if col.label.is_empty() {
        source.to_owned()
    } else {
        format!("{source}, {}", col.label)
    };
    let _ = writeln!(
        out,
        "{} values from {from}: mean {}, std dev {}, min {}, max {}",
        r["count"],
        n(&d["mean"]),
        n(&d["std_dev"]),
        n(&d["min"]),
        n(&d["max"])
    );
    if col.empty > 0 {
        let s = if col.empty == 1 { "" } else { "s" };
        let _ = writeln!(out, "{} empty cell{s} left out", col.empty);
    }
    let _ = writeln!(
        out,
        "Ranked by Anderson–Darling A² (smaller fits better); D is the Kolmogorov–Smirnov distance,\nthe largest gap between the data's and the fit's distribution functions.\n"
    );
    let mut rows =
        vec![["rank", "family", "A²", "D", "mean", "std dev", "parameters"].map(String::from)];
    for (i, c) in r["candidates"].as_array().into_iter().flatten().enumerate() {
        rows.push([
            (i + 1).to_string(),
            c["family"].as_str().unwrap_or("").to_owned(),
            n(&c["anderson_darling"]),
            n(&c["kolmogorov_smirnov"]),
            n(&c["mean"]),
            n(&c["std_dev"]),
            describe(&c["distribution"]),
        ]);
    }
    let widths: Vec<usize> = (0..7)
        .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    for row in &rows {
        let line: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(c, cell)| {
                let pad = " ".repeat(widths[c] - cell.chars().count());
                // Text left, numbers right.
                if c == 1 || c == 6 {
                    format!("{cell}{pad}")
                } else {
                    format!("{pad}{cell}")
                }
            })
            .collect();
        let _ = writeln!(out, "{}", line.join("  ").trim_end());
    }
    let skipped: Vec<String> = r["skipped"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            format!(
                "{} ({})",
                s["family"].as_str().unwrap_or(""),
                s["reason"].as_str().unwrap_or("")
            )
        })
        .collect();
    if !skipped.is_empty() {
        let _ = writeln!(out, "\nNot fitted: {}", skipped.join("; "));
    }
    let _ = writeln!(
        out,
        "\nThe best fit as a distribution for `block.set` (service, interarrival or duration):\n{}",
        r["best"]
    );
    out
}

fn csv(r: &Value) -> String {
    let mut out = String::from(
        "rank,family,anderson_darling,kolmogorov_smirnov,mean,std_dev,log_likelihood,distribution\n",
    );
    let num = |v: &Value| v.as_f64().map_or_else(String::new, |x| x.to_string());
    for (i, c) in r["candidates"].as_array().into_iter().flatten().enumerate() {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},\"{}\"",
            i + 1,
            c["family"].as_str().unwrap_or(""),
            num(&c["anderson_darling"]),
            num(&c["kolmogorov_smirnov"]),
            num(&c["mean"]),
            num(&c["std_dev"]),
            num(&c["log_likelihood"]),
            c["distribution"].to_string().replace('"', "\"\"")
        );
    }
    out
}

pub struct FitArgs<'a> {
    pub file: &'a Path,
    pub column: Option<&'a str>,
    pub families: &'a [String],
    pub min_count: Option<u32>,
    pub format: Format,
}

pub fn fit(a: &FitArgs) -> Result<u8, Failure> {
    let stdin = a.file == Path::new("-");
    let text = if stdin {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| Failure(format!("cannot read standard input: {e}")))?;
        s
    } else {
        std::fs::read_to_string(a.file)
            .map_err(|e| Failure(format!("cannot read {}: {e}", a.file.display())))?
    };
    let col = read_column(&text, a.column)?;
    let mut params = json!({ "data": col.values });
    if !a.families.is_empty() {
        let names: Vec<&str> = a.families.iter().map(|f| f.trim()).collect();
        params["families"] = json!(names);
    }
    if let Some(m) = a.min_count {
        params["min_count"] = json!(m);
    }
    let r = Engine::new().execute("estimate.fit", params)?;
    let source = if stdin {
        "standard input".to_owned()
    } else {
        a.file.display().to_string()
    };
    match a.format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default()),
        Format::Csv => print!("{}", csv(&r)),
        Format::Table => print!("{}", table(&source, &col, &r)),
    }
    Ok(0)
}

#[cfg(test)]
#[allow(clippy::panic)] // A wrong answer fails the test.
mod tests {
    use super::*;

    fn read(text: &str, column: Option<&str>) -> Result<Column, String> {
        read_column(text, column).map_err(|Failure(m)| m)
    }

    #[test]
    fn columns_are_found_by_header_or_number() {
        let text = "id,\"service, min\",wait\r\n1,2.5,0\n2,3.0,1\n\n3,,2\n4,1.5e0,3\n";
        let c = read(text, Some("service, min")).unwrap_or_else(|m| panic!("{m}"));
        assert_eq!(c.values, [2.5, 3.0, 1.5]);
        assert_eq!(c.empty, 1);
        assert_eq!(c.label, "column service, min");
        let c = read(text, Some("3")).unwrap_or_else(|m| panic!("{m}"));
        assert_eq!(c.values, [0.0, 1.0, 2.0, 3.0]);
        assert!(read(text, None).is_err_and(|m| m.contains("--column")));
        assert!(read(text, Some("4")).is_err_and(|m| m.contains("no column")));
    }

    #[test]
    fn a_plain_list_and_other_separators() {
        let c = read("1.5\n2\n 3.25 \n", None).unwrap_or_else(|m| panic!("{m}"));
        assert_eq!(c.values, [1.5, 2.0, 3.25]);
        assert_eq!(c.label, "");
        let c = read("time\n1\n2\n", None).unwrap_or_else(|m| panic!("{m}"));
        assert_eq!((c.values.len(), c.label.as_str()), (2, "column time"));
        let c = read("a;b\n1;2\n3;4\n", Some("b")).unwrap_or_else(|m| panic!("{m}"));
        assert_eq!(c.values, [2.0, 4.0]);
        let c = read("a\tb\n1\t2\n", Some("a")).unwrap_or_else(|m| panic!("{m}"));
        assert_eq!(c.values, [1.0]);
        assert!(read("1\nx\n", None).is_err_and(|m| m.contains("line 2")));
        assert!(read("1\ninf\n", None).is_err_and(|m| m.contains("line 2")));
        assert!(read("\n\n", None).is_err());
    }
}
