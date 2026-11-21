//! Units of measure: a product of named units with whole-number powers, so `people/year`,
//! `1/year` and the dimensionless `1` are all the same kind of thing.
//!
//! Names are compared in lower case with a plural `s` removed, so `Year` and `years` are the
//! same unit, and so are the common spellings of a time unit (`yr`, `min`) and its full name.

use std::collections::BTreeMap;

/// A unit: each base unit with its power (never 0). The empty product is dimensionless.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Units(BTreeMap<String, i32>);

/// A time unit's canonical name and its length in seconds. A month is a twelfth of a year, a
/// year 365.25 days.
const TIME_UNITS: &[(&[&str], &str, f64)] = &[
    (&["second", "sec", "s"], "second", 1.0),
    (&["minute", "min"], "minute", 60.0),
    (&["hour", "hr", "h"], "hour", 3600.0),
    (&["day", "d"], "day", 86_400.0),
    (&["week", "wk"], "week", 604_800.0),
    (&["month", "mo"], "month", 2_629_800.0),
    (&["year", "yr", "y"], "year", 31_557_600.0),
];

fn singular(word: &str) -> String {
    let w = word.to_lowercase();
    if w.len() > 2 && w.ends_with('s') && !w.ends_with("ss") {
        w.get(..w.len() - 1).unwrap_or_default().to_owned()
    } else {
        w
    }
}

/// The name a unit is compared by.
fn base_name(word: &str) -> String {
    let s = singular(word);
    for (names, canon, _) in TIME_UNITS {
        if names.contains(&s.as_str()) {
            return (*canon).to_owned();
        }
    }
    s
}

/// The time unit a name stands for, as (canonical name, seconds), if it is one.
pub fn time_unit(name: &str) -> Option<(&'static str, f64)> {
    let s = singular(name);
    TIME_UNITS
        .iter()
        .find(|(names, _, _)| names.contains(&s.as_str()))
        .map(|(_, canon, secs)| (*canon, *secs))
}

impl Units {
    pub fn dimensionless() -> Self {
        Self::default()
    }

    pub fn base(name: &str) -> Self {
        let mut m = BTreeMap::new();
        m.insert(base_name(name), 1);
        Self(m)
    }

    pub fn is_dimensionless(&self) -> bool {
        self.0.is_empty()
    }

    pub fn mul(&self, other: &Units) -> Units {
        let mut m = self.0.clone();
        for (k, p) in &other.0 {
            let e = m.entry(k.clone()).or_insert(0);
            *e += p;
        }
        m.retain(|_, p| *p != 0);
        Units(m)
    }

    pub fn inverse(&self) -> Units {
        Units(self.0.iter().map(|(k, p)| (k.clone(), -p)).collect())
    }

    pub fn div(&self, other: &Units) -> Units {
        self.mul(&other.inverse())
    }

    pub fn pow(&self, n: i32) -> Units {
        let mut m: BTreeMap<String, i32> = self.0.iter().map(|(k, p)| (k.clone(), p * n)).collect();
        m.retain(|_, p| *p != 0);
        Units(m)
    }

    /// The square root, if every power is even.
    pub fn sqrt(&self) -> Option<Units> {
        if self.0.values().all(|p| p % 2 == 0) {
            Some(Units(
                self.0.iter().map(|(k, p)| (k.clone(), p / 2)).collect(),
            ))
        } else {
            None
        }
    }

    /// Reads `people/year`, `1/year`, `m^2`, `(a*b)/c` and the like. An error says what is wrong.
    pub fn parse(text: &str) -> Result<Units, String> {
        let toks = lex(text)?;
        if toks.is_empty() {
            return Ok(Units::dimensionless());
        }
        let mut p = UParser { toks, i: 0 };
        let u = p.quotient()?;
        if p.i < p.toks.len() {
            return Err("something follows the end of the units".to_owned());
        }
        Ok(u)
    }
}

impl std::fmt::Display for Units {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            return write!(f, "1");
        }
        let part = |k: &str, p: i32| {
            if p == 1 {
                k.to_owned()
            } else {
                format!("{k}^{p}")
            }
        };
        let top: Vec<String> = self
            .0
            .iter()
            .filter(|(_, p)| **p > 0)
            .map(|(k, p)| part(k, *p))
            .collect();
        let bottom: Vec<String> = self
            .0
            .iter()
            .filter(|(_, p)| **p < 0)
            .map(|(k, p)| part(k, -p))
            .collect();
        let numerator = if top.is_empty() {
            "1".to_owned()
        } else {
            top.join("*")
        };
        match bottom.len() {
            0 => write!(f, "{numerator}"),
            1 => write!(f, "{numerator}/{}", bottom.join("*")),
            _ => write!(f, "{numerator}/({})", bottom.join("*")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum UTok {
    Name(String),
    Num(i32),
    Star,
    Slash,
    Caret,
    Minus,
    Open,
    Close,
}

fn lex(text: &str) -> Result<Vec<UTok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        match c {
            c if c.is_whitespace() => i += 1,
            '*' | '\u{b7}' => {
                out.push(UTok::Star);
                i += 1;
            }
            '/' => {
                out.push(UTok::Slash);
                i += 1;
            }
            '^' => {
                out.push(UTok::Caret);
                i += 1;
            }
            '-' => {
                out.push(UTok::Minus);
                i += 1;
            }
            '(' => {
                out.push(UTok::Open);
                i += 1;
            }
            ')' => {
                out.push(UTok::Close);
                i += 1;
            }
            c if c.is_ascii_digit() => {
                let mut j = i;
                while chars.get(j).is_some_and(char::is_ascii_digit) {
                    j += 1;
                }
                let s: String = chars.get(i..j).unwrap_or_default().iter().collect();
                out.push(UTok::Num(
                    s.parse().map_err(|_| format!("`{s}` is too big"))?,
                ));
                i = j;
            }
            c if c.is_alphabetic() || c == '_' || c == '$' || c == '%' => {
                let mut j = i;
                while chars
                    .get(j)
                    .is_some_and(|d| d.is_alphanumeric() || matches!(d, '_' | '$' | '%'))
                {
                    j += 1;
                }
                let s: String = chars.get(i..j).unwrap_or_default().iter().collect();
                out.push(UTok::Name(s));
                i = j;
            }
            other => return Err(format!("unexpected `{other}` in units")),
        }
    }
    Ok(out)
}

struct UParser {
    toks: Vec<UTok>,
    i: usize,
}

impl UParser {
    fn peek(&self) -> Option<&UTok> {
        self.toks.get(self.i)
    }

    /// product (`/` product)*, left to right.
    fn quotient(&mut self) -> Result<Units, String> {
        let mut u = self.product()?;
        while self.peek() == Some(&UTok::Slash) {
            self.i += 1;
            let d = self.product()?;
            u = u.div(&d);
        }
        Ok(u)
    }

    /// power (`*` power)*.
    fn product(&mut self) -> Result<Units, String> {
        let mut u = self.power()?;
        while self.peek() == Some(&UTok::Star) {
            self.i += 1;
            let r = self.power()?;
            u = u.mul(&r);
        }
        Ok(u)
    }

    fn power(&mut self) -> Result<Units, String> {
        let base = match self.peek().cloned() {
            Some(UTok::Name(n)) => {
                self.i += 1;
                Units::base(&n)
            }
            Some(UTok::Num(1)) => {
                self.i += 1;
                Units::dimensionless()
            }
            Some(UTok::Num(n)) => return Err(format!("`{n}` is not a unit (only `1` is)")),
            Some(UTok::Open) => {
                self.i += 1;
                let u = self.quotient()?;
                if self.peek() != Some(&UTok::Close) {
                    return Err("`)` was expected in units".to_owned());
                }
                self.i += 1;
                u
            }
            _ => return Err("a unit was expected".to_owned()),
        };
        if self.peek() == Some(&UTok::Caret) {
            self.i += 1;
            let negative = self.peek() == Some(&UTok::Minus);
            if negative {
                self.i += 1;
            }
            return match self.peek().cloned() {
                Some(UTok::Num(n)) => {
                    self.i += 1;
                    Ok(base.pow(if negative { -n } else { n }))
                }
                _ => Err("a whole-number power was expected after `^`".to_owned()),
            };
        }
        Ok(base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Units {
        Units::parse(s).unwrap_or_default()
    }

    #[test]
    fn products_and_quotients_balance() {
        assert_eq!(u("people/year"), u("People / Years"));
        assert_eq!(u("1/year"), u("year^-1"));
        assert_eq!(u("m^2/m"), u("m"));
        assert_eq!(u("people/year").mul(&u("year")), u("people"));
        assert!(u("people/people").is_dimensionless());
        assert!(u("1").is_dimensionless());
        assert_ne!(u("people"), u("dollar"));
    }

    #[test]
    fn time_units_have_one_name_and_a_length() {
        assert_eq!(u("yr"), u("Years"));
        assert_eq!(u("min"), u("minute"));
        assert_eq!(time_unit("Months").map(|(n, _)| n), Some("month"));
        assert_eq!(time_unit("people"), None);
        assert_eq!(time_unit("days").map(|(_, s)| s), Some(86_400.0));
    }

    #[test]
    fn bad_units_are_errors_and_text_reads_back() {
        assert!(Units::parse("a/").is_err());
        assert!(Units::parse("a^b").is_err());
        assert!(Units::parse("2").is_err());
        assert_eq!(u("people/year").to_string(), "people/year");
        assert_eq!(u("1/year").to_string(), "1/year");
        assert_eq!(Units::dimensionless().to_string(), "1");
        assert_eq!(u("m^2").sqrt(), Some(u("m")));
        assert_eq!(u("m").sqrt(), None);
    }
}
