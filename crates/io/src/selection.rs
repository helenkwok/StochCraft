//! BPSim resource selection expressions (`ResourceParameters/Selection`, specification section
//! 7.2): `getResource(name, qty)`, `getResourceByRoles([role, …], qty)` and
//! `orResource(choice, …)` for alternatives, each with or without the `bpsim:` prefix.
//!
//! The specification writes the role list in brackets; XPath has no such literal, so a
//! parenthesised list `('a', 'b')` and a flat list (`getResourceByRoles('a', 'b', 2)`, the last
//! argument being the quantity) are read too. StochCraft writes the bracketed form.

use stochcraft_model::{ElementId, Need};

/// One parsed argument.
#[derive(Debug, Clone, PartialEq)]
enum Arg {
    Call(String, Vec<Arg>),
    List(Vec<Arg>),
    Text(String),
    Number(f64),
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn skip(&mut self) {
        while self.s.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.skip();
        if self.s.get(self.at) == Some(&c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn list(&mut self, close: u8) -> Result<Vec<Arg>, String> {
        let mut out = Vec::new();
        if self.eat(close) {
            return Ok(out);
        }
        loop {
            out.push(self.arg()?);
            if self.eat(close) {
                return Ok(out);
            }
            if !self.eat(b',') {
                return Err(format!("expected `,` or `{}`", close as char));
            }
        }
    }

    fn arg(&mut self) -> Result<Arg, String> {
        self.skip();
        let start = self.at;
        match self.s.get(self.at) {
            Some(b'\'' | b'"') => {
                let q = self.s[start];
                let end = self.s[start + 1..]
                    .iter()
                    .position(|&c| c == q)
                    .ok_or("an unterminated string")?;
                self.at = start + 2 + end;
                Ok(Arg::Text(
                    String::from_utf8_lossy(&self.s[start + 1..start + 1 + end]).into_owned(),
                ))
            }
            Some(b'[') => {
                self.at += 1;
                Ok(Arg::List(self.list(b']')?))
            }
            Some(b'(') => {
                self.at += 1;
                Ok(Arg::List(self.list(b')')?))
            }
            Some(c) if c.is_ascii_digit() || *c == b'.' || *c == b'-' => {
                while self
                    .s
                    .get(self.at)
                    .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'e' | b'E'))
                {
                    self.at += 1;
                }
                let text = String::from_utf8_lossy(&self.s[start..self.at]);
                text.parse()
                    .map(Arg::Number)
                    .map_err(|_| format!("`{text}` is not a number"))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                while self
                    .s
                    .get(self.at)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b':' | b'_' | b'-'))
                {
                    self.at += 1;
                }
                let name = String::from_utf8_lossy(&self.s[start..self.at]).into_owned();
                let name = name.strip_prefix("bpsim:").unwrap_or(&name).to_owned();
                if !self.eat(b'(') {
                    return Err(format!("`{name}` is not a function call"));
                }
                Ok(Arg::Call(name, self.list(b')')?))
            }
            _ => Err("unexpected text".into()),
        }
    }
}

/// A whole number of units, at least 1.
fn quantity(a: Option<&Arg>) -> Result<u32, String> {
    match a {
        Some(Arg::Number(q)) if *q >= 1.0 && q.fract() == 0.0 && *q <= f64::from(u32::MAX) => {
            Ok(*q as u32)
        }
        Some(Arg::Number(q)) => Err(format!(
            "a quantity must be a whole number of at least 1, not {q}"
        )),
        _ => Err("a quantity is missing".into()),
    }
}

fn need(a: &Arg) -> Result<Need, String> {
    let Arg::Call(name, args) = a else {
        return Err("expected getResource(…) or getResourceByRoles(…)".into());
    };
    match name.as_str() {
        "getResource" => match args.as_slice() {
            [Arg::Text(id), q] => Ok(Need::Resource {
                id: ElementId::unchecked(id),
                quantity: quantity(Some(q))?,
            }),
            _ => Err("getResource takes a resource id and a quantity".into()),
        },
        "getResourceByRoles" => {
            let (roles, q) = match args.as_slice() {
                [Arg::List(roles), q] => (roles.clone(), q),
                [roles @ .., q] if !roles.is_empty() => (roles.to_vec(), q),
                _ => return Err("getResourceByRoles takes roles and a quantity".into()),
            };
            let roles = roles
                .into_iter()
                .map(|r| match r {
                    Arg::Text(t) => Ok(t),
                    _ => Err("a role is a quoted name".to_owned()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            if roles.is_empty() {
                return Err("getResourceByRoles names no role".into());
            }
            Ok(Need::Roles {
                roles,
                quantity: quantity(Some(q))?,
            })
        }
        other => Err(format!("`{other}` is not a resource selection function")),
    }
}

/// The alternatives a selection expression names, in order.
pub fn parse(expr: &str) -> Result<Vec<Need>, String> {
    let mut p = Parser {
        s: expr.as_bytes(),
        at: 0,
    };
    let top = p.arg()?;
    p.skip();
    if p.at != p.s.len() {
        return Err("unexpected text after the selection".into());
    }
    match &top {
        Arg::Call(name, args) if name == "orResource" => {
            if args.is_empty() {
                return Err("orResource names no alternative".into());
            }
            args.iter().map(need).collect()
        }
        _ => Ok(vec![need(&top)?]),
    }
}

fn quote(s: &str) -> String {
    if s.contains('\'') {
        format!("\"{s}\"")
    } else {
        format!("'{s}'")
    }
}

fn format_need(n: &Need) -> String {
    match n {
        Need::Resource { id, quantity } => {
            format!("getResource({}, {quantity})", quote(id.as_str()))
        }
        Need::Roles { roles, quantity } => {
            let roles: Vec<String> = roles.iter().map(|r| quote(r)).collect();
            format!("getResourceByRoles([{}], {quantity})", roles.join(", "))
        }
    }
}

/// `needs` as a BPSim selection expression: one alternative as it is, several in `orResource`.
pub fn format(needs: &[Need]) -> String {
    match needs {
        [one] => format_need(one),
        many => {
            let parts: Vec<String> = many.iter().map(format_need).collect();
            format!("orResource({})", parts.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(id: &str, q: u32) -> Need {
        Need::Resource {
            id: ElementId::unchecked(id),
            quantity: q,
        }
    }

    fn roles(r: &[&str], q: u32) -> Need {
        Need::Roles {
            roles: r.iter().map(|s| (*s).to_owned()).collect(),
            quantity: q,
        }
    }

    #[test]
    fn the_forms_models_use_are_read() {
        assert_eq!(parse("getResource('desk', 1)"), Ok(vec![res("desk", 1)]));
        assert_eq!(
            parse("bpsim:getResource(\"desk\",2)"),
            Ok(vec![res("desk", 2)])
        );
        let r = vec![roles(&["nurse", "senior"], 2)];
        assert_eq!(
            parse("getResourceByRoles(['nurse', 'senior'], 2)").as_ref(),
            Ok(&r)
        );
        assert_eq!(
            parse("getResourceByRoles(('nurse','senior'), 2)").as_ref(),
            Ok(&r)
        );
        assert_eq!(
            parse("getResourceByRoles('nurse', 'senior', 2)").as_ref(),
            Ok(&r)
        );
        assert_eq!(
            parse("bpsim:orResource(getResource('a', 1), bpsim:getResourceByRoles(['x'], 1))"),
            Ok(vec![res("a", 1), roles(&["x"], 1)])
        );
    }

    #[test]
    fn what_cannot_be_used_is_explained() {
        assert!(parse("getResource('a', 0)").is_err_and(|e| e.contains("at least 1")));
        assert!(parse("getResource('a', 1.5)").is_err());
        assert!(parse("getResourceByRoles([], 1)").is_err());
        assert!(parse("orResource()").is_err());
        assert!(parse("getResource('a', 1) extra").is_err());
        assert!(parse("pickAny('a')").is_err_and(|e| e.contains("pickAny")));
    }

    #[test]
    fn what_is_written_reads_back() {
        for needs in [
            vec![res("desk", 1)],
            vec![roles(&["nurse", "o'neil"], 3)],
            vec![res("a", 1), roles(&["x", "y"], 2), res("b", 4)],
        ] {
            assert_eq!(
                parse(&format(&needs)),
                Ok(needs.clone()),
                "{}",
                format(&needs)
            );
        }
    }
}
