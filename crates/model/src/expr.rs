//! BPSim's expressions: the subset of XPath 1.0 that BPSim models use.
//!
//! An expression is parsed once ([`parse`]) into an [`Expr`] and evaluated many times
//! ([`Expr::eval`]) against a token's properties. What is supported:
//!
//! - numbers (`3`, `2.5`, `.5`), string literals (`'gold'`, `"gold"`), parentheses;
//! - arithmetic `+ - * div mod` and unary minus; comparisons `= != < <= > >=`; `and`, `or`;
//! - the functions `getProperty('name')` (also written `bpsim:getProperty`), `true()`,
//!   `false()`, `not()`, `boolean()`, `number()`, `string()`, `floor()`, `ceiling()`, `round()`,
//!   `concat()`, `contains()`, `starts-with()` and `string-length()`.
//!
//! Anything else (paths, variables, unions, other functions) is a parse error with its position,
//! so a model that uses it gets a diagnostic, never a wrong result. BPSim's resource functions
//! (`getResource`) belong to a task's resource selection, which is read separately (M2.5).
//!
//! Values follow XPath 1.0: numbers are `f64`, conversions between numbers, strings and booleans
//! are XPath's, and `=` compares as booleans if either side is one, else as numbers if either
//! side is one, else as strings. A property a token does not have reads as XPath's empty
//! node-set would: the empty string, which is `NaN` as a number and false as a boolean. Every
//! operation is exact IEEE arithmetic (`mod` is the truncating remainder), so results are the
//! same on every platform.

use std::fmt;

/// A value an expression computes.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),
    Text(String),
    Boolean(bool),
}

impl Value {
    /// XPath's `number()`.
    pub fn number(&self) -> f64 {
        match self {
            Value::Number(n) => *n,
            Value::Boolean(b) => f64::from(u8::from(*b)),
            Value::Text(s) => text_to_number(s),
        }
    }

    /// XPath's `boolean()`.
    pub fn boolean(&self) -> bool {
        match self {
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::Boolean(b) => *b,
            Value::Text(s) => !s.is_empty(),
        }
    }

    /// XPath's `string()`.
    pub fn text(&self) -> String {
        match self {
            Value::Number(n) => number_to_text(*n),
            Value::Boolean(b) => b.to_string(),
            Value::Text(s) => s.clone(),
        }
    }
}

/// XPath's conversion of a string to a number: optional whitespace, an optional minus, digits
/// with an optional decimal point; anything else is `NaN`.
fn text_to_number(s: &str) -> f64 {
    let t = s.trim_matches([' ', '\t', '\r', '\n']);
    let digits = t.strip_prefix('-').unwrap_or(t);
    let ok = !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        && digits.chars().filter(|&c| c == '.').count() <= 1
        && digits != ".";
    if ok {
        t.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// XPath's conversion of a number to a string: whole numbers without a decimal point.
fn number_to_text(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if n == 0.0 {
        "0".into()
    } else {
        // Rust prints the shortest decimal that reads back exactly, never in exponent form.
        n.to_string()
    }
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// A function an expression can call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    True,
    False,
    Not,
    Boolean,
    Number,
    String,
    Floor,
    Ceiling,
    Round,
    Concat,
    Contains,
    StartsWith,
    StringLength,
}

impl Func {
    fn lookup(name: &str) -> Option<(Func, usize, Option<usize>)> {
        // Name, then the least and most arguments (None: any number).
        Some(match name {
            "true" => (Func::True, 0, Some(0)),
            "false" => (Func::False, 0, Some(0)),
            "not" => (Func::Not, 1, Some(1)),
            "boolean" => (Func::Boolean, 1, Some(1)),
            "number" => (Func::Number, 1, Some(1)),
            "string" => (Func::String, 1, Some(1)),
            "floor" => (Func::Floor, 1, Some(1)),
            "ceiling" => (Func::Ceiling, 1, Some(1)),
            "round" => (Func::Round, 1, Some(1)),
            "concat" => (Func::Concat, 2, None),
            "contains" => (Func::Contains, 2, Some(2)),
            "starts-with" => (Func::StartsWith, 2, Some(2)),
            "string-length" => (Func::StringLength, 1, Some(1)),
            _ => return None,
        })
    }
}

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    /// `getProperty('name')`.
    Property(String),
    Neg(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
}

/// The kind of value an expression gives, as far as can be told without evaluating it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Number,
    Text,
    Boolean,
    /// A property's value: whatever the property holds.
    Property,
}

/// Why an expression cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprError {
    /// Where, in characters from the start (0-based).
    pub position: usize,
    pub message: String,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at character {})", self.message, self.position + 1)
    }
}

impl std::error::Error for ExprError {}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Lit(String),
    Name(String),
    Op(BinOp),
    Minus,
    LParen,
    RParen,
    Comma,
}

fn err(position: usize, message: impl Into<String>) -> ExprError {
    ExprError {
        position,
        message: message.into(),
    }
}

fn tokenize(src: &str) -> Result<Vec<(usize, Tok)>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out: Vec<(usize, Tok)> = Vec::new();
    let mut i = 0;
    // Whether the previous token ends an operand, which makes `*`, `and`, `or`, `div` and `mod`
    // operators (XPath's disambiguation rule).
    let after_operand = |out: &Vec<(usize, Tok)>| {
        matches!(
            out.last().map(|(_, t)| t),
            Some(Tok::Num(_) | Tok::Lit(_) | Tok::RParen)
        )
    };
    while let Some(&c) = chars.get(i) {
        let start = i;
        match c {
            ' ' | '\t' | '\r' | '\n' => i += 1,
            '(' => {
                out.push((start, Tok::LParen));
                i += 1;
            }
            ')' => {
                out.push((start, Tok::RParen));
                i += 1;
            }
            ',' => {
                out.push((start, Tok::Comma));
                i += 1;
            }
            '+' => {
                out.push((start, Tok::Op(BinOp::Add)));
                i += 1;
            }
            '-' => {
                out.push((start, Tok::Minus));
                i += 1;
            }
            '*' => {
                if !after_operand(&out) {
                    return Err(err(
                        start,
                        "`*` here would be a path; paths are not supported",
                    ));
                }
                out.push((start, Tok::Op(BinOp::Mul)));
                i += 1;
            }
            '=' => {
                out.push((start, Tok::Op(BinOp::Eq)));
                i += 1;
            }
            '!' if chars.get(i + 1) == Some(&'=') => {
                out.push((start, Tok::Op(BinOp::Ne)));
                i += 2;
            }
            '<' | '>' => {
                let eq = chars.get(i + 1) == Some(&'=');
                let op = match (c, eq) {
                    ('<', false) => BinOp::Lt,
                    ('<', true) => BinOp::Le,
                    ('>', false) => BinOp::Gt,
                    _ => BinOp::Ge,
                };
                out.push((start, Tok::Op(op)));
                i += if eq { 2 } else { 1 };
            }
            '\'' | '"' => {
                let close = chars[i + 1..]
                    .iter()
                    .position(|&q| q == c)
                    .ok_or_else(|| err(start, "this string is not closed"))?;
                let text: String = chars[i + 1..i + 1 + close].iter().collect();
                out.push((start, Tok::Lit(text)));
                i += close + 2;
            }
            '0'..='9' | '.' => {
                let mut j = i;
                while chars.get(j).is_some_and(char::is_ascii_digit) {
                    j += 1;
                }
                if chars.get(j) == Some(&'.') {
                    j += 1;
                    while chars.get(j).is_some_and(char::is_ascii_digit) {
                        j += 1;
                    }
                }
                let text: String = chars[i..j].iter().collect();
                if text == "." {
                    return Err(err(start, "`.` (the context node) is not supported"));
                }
                let n = text
                    .parse::<f64>()
                    .map_err(|_| err(start, format!("`{text}` is not a number")))?;
                out.push((start, Tok::Num(n)));
                i = j;
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut j = i + 1;
                while chars
                    .get(j)
                    .is_some_and(|&d| d.is_alphanumeric() || matches!(d, '_' | '-' | '.'))
                {
                    j += 1;
                }
                // A prefix: `bpsim:getProperty`, `fn:not`.
                if chars.get(j) == Some(&':')
                    && chars
                        .get(j + 1)
                        .is_some_and(|&d| d.is_alphabetic() || d == '_')
                {
                    j += 2;
                    while chars
                        .get(j)
                        .is_some_and(|&d| d.is_alphanumeric() || matches!(d, '_' | '-' | '.'))
                    {
                        j += 1;
                    }
                }
                let name: String = chars[i..j].iter().collect();
                let op = match name.as_str() {
                    "and" => Some(BinOp::And),
                    "or" => Some(BinOp::Or),
                    "div" => Some(BinOp::Div),
                    "mod" => Some(BinOp::Mod),
                    _ => None,
                };
                match op {
                    Some(op) if after_operand(&out) => out.push((start, Tok::Op(op))),
                    _ => out.push((start, Tok::Name(name))),
                }
                i = j;
            }
            '$' => return Err(err(start, "variables are not supported")),
            '/' | '[' | ']' | '@' => {
                return Err(err(
                    start,
                    "paths are not supported; use getProperty('name')",
                ));
            }
            '|' => return Err(err(start, "node-set unions are not supported")),
            other => return Err(err(start, format!("unexpected `{other}`"))),
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<(usize, Tok)>,
    at: usize,
    end: usize,
}

/// Operator precedence levels, loosest first.
const LEVELS: [&[BinOp]; 6] = [
    &[BinOp::Or],
    &[BinOp::And],
    &[BinOp::Eq, BinOp::Ne],
    &[BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge],
    &[BinOp::Add, BinOp::Sub],
    &[BinOp::Mul, BinOp::Div, BinOp::Mod],
];

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at).map(|(_, t)| t)
    }

    fn pos(&self) -> usize {
        self.toks.get(self.at).map_or(self.end, |(p, _)| *p)
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.at).map(|(_, t)| t.clone());
        self.at += 1;
        t
    }

    fn binary(&mut self, level: usize) -> Result<Expr, ExprError> {
        let Some(ops) = LEVELS.get(level) else {
            return self.unary();
        };
        let mut left = self.binary(level + 1)?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(op)) if ops.contains(op) => *op,
                Some(Tok::Minus) if ops.contains(&BinOp::Sub) => BinOp::Sub,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.binary(level + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Result<Expr, ExprError> {
        if self.peek() == Some(&Tok::Minus) {
            self.bump();
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, ExprError> {
        let pos = self.pos();
        match self.bump() {
            Some(Tok::Num(n)) => Ok(Expr::Number(n)),
            Some(Tok::Lit(s)) => Ok(Expr::Text(s)),
            Some(Tok::LParen) => {
                let e = self.binary(0)?;
                self.expect_close()?;
                Ok(e)
            }
            Some(Tok::Name(name)) => {
                if self.peek() != Some(&Tok::LParen) {
                    return Err(err(
                        pos,
                        format!(
                            "`{name}` is a path, which is not supported; use getProperty('{name}')"
                        ),
                    ));
                }
                self.bump();
                let mut args = Vec::new();
                if self.peek() != Some(&Tok::RParen) {
                    loop {
                        args.push(self.binary(0)?);
                        if self.peek() == Some(&Tok::Comma) {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                self.expect_close()?;
                call(pos, &name, args)
            }
            Some(Tok::RParen) => Err(err(pos, "`)` without a matching `(`")),
            Some(Tok::Comma) => Err(err(pos, "unexpected `,`")),
            Some(Tok::Op(_) | Tok::Minus) => Err(err(pos, "an operator needs a value before it")),
            None => Err(err(pos, "the expression ends too early")),
        }
    }

    fn expect_close(&mut self) -> Result<(), ExprError> {
        let pos = self.pos();
        match self.bump() {
            Some(Tok::RParen) => Ok(()),
            _ => Err(err(pos, "expected `)`")),
        }
    }
}

fn call(pos: usize, name: &str, args: Vec<Expr>) -> Result<Expr, ExprError> {
    let local = name
        .strip_prefix("bpsim:")
        .or_else(|| name.strip_prefix("fn:"))
        .unwrap_or(name);
    if local == "getProperty" {
        return match args.as_slice() {
            [Expr::Text(p)] if !p.is_empty() => Ok(Expr::Property(p.clone())),
            _ => Err(err(
                pos,
                "getProperty takes the property's name as a quoted string, such as getProperty('count')",
            )),
        };
    }
    if local.starts_with("getResource") {
        return Err(err(
            pos,
            format!("`{local}` selects a task's resource and is only read there (M2.5)"),
        ));
    }
    let (f, min, max) = Func::lookup(local).ok_or_else(|| {
        err(
            pos,
            format!("the function `{name}` is not supported in BPSim expressions here"),
        )
    })?;
    if args.len() < min || max.is_some_and(|m| args.len() > m) {
        let want = match max {
            Some(m) if m == min => format!("{min}"),
            Some(m) => format!("{min} to {m}"),
            None => format!("at least {min}"),
        };
        return Err(err(
            pos,
            format!("`{local}` takes {want} arguments, not {}", args.len()),
        ));
    }
    Ok(Expr::Call(f, args))
}

/// Parses an expression.
pub fn parse(src: &str) -> Result<Expr, ExprError> {
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return Err(err(0, "the expression is empty"));
    }
    let mut p = Parser {
        toks,
        at: 0,
        end: src.chars().count(),
    };
    let e = p.binary(0)?;
    if p.at < p.toks.len() {
        return Err(err(p.pos(), "unexpected text after the expression"));
    }
    Ok(e)
}

impl Expr {
    /// Evaluates the expression, reading properties with `property` (`None`: the token does not
    /// have it).
    pub fn eval(&self, property: &dyn Fn(&str) -> Option<Value>) -> Value {
        match self {
            Expr::Number(n) => Value::Number(*n),
            Expr::Text(s) => Value::Text(s.clone()),
            Expr::Property(name) => property(name).unwrap_or(Value::Text(String::new())),
            Expr::Neg(e) => Value::Number(-e.eval(property).number()),
            Expr::Binary(op, a, b) => binary(*op, a, b, property),
            Expr::Call(f, args) => {
                let arg = |i: usize| {
                    args.get(i)
                        .map_or(Value::Text(String::new()), |a| a.eval(property))
                };
                match f {
                    Func::True => Value::Boolean(true),
                    Func::False => Value::Boolean(false),
                    Func::Not => Value::Boolean(!arg(0).boolean()),
                    Func::Boolean => Value::Boolean(arg(0).boolean()),
                    Func::Number => Value::Number(arg(0).number()),
                    Func::String => Value::Text(arg(0).text()),
                    Func::Floor => Value::Number(arg(0).number().floor()),
                    Func::Ceiling => Value::Number(arg(0).number().ceil()),
                    Func::Round => Value::Number(round(arg(0).number())),
                    Func::Concat => {
                        Value::Text(args.iter().map(|a| a.eval(property).text()).collect())
                    }
                    Func::Contains => Value::Boolean(arg(0).text().contains(&arg(1).text())),
                    Func::StartsWith => Value::Boolean(arg(0).text().starts_with(&arg(1).text())),
                    Func::StringLength => Value::Number(arg(0).text().chars().count() as f64),
                }
            }
        }
    }

    /// Every property the expression reads, in order, with repeats.
    pub fn properties(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.visit(&mut |e| {
            if let Expr::Property(p) = e {
                out.push(p.as_str());
            }
        });
        out
    }

    fn visit<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        f(self);
        match self {
            Expr::Neg(e) => e.visit(f),
            Expr::Binary(_, a, b) => {
                a.visit(f);
                b.visit(f);
            }
            Expr::Call(_, args) => args.iter().for_each(|a| a.visit(f)),
            Expr::Number(_) | Expr::Text(_) | Expr::Property(_) => {}
        }
    }

    /// The kind of value the expression gives.
    pub fn kind(&self) -> Kind {
        match self {
            Expr::Number(_) | Expr::Neg(_) => Kind::Number,
            Expr::Text(_) => Kind::Text,
            Expr::Property(_) => Kind::Property,
            Expr::Binary(op, _, _) => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => Kind::Number,
                _ => Kind::Boolean,
            },
            Expr::Call(f, _) => match f {
                Func::True
                | Func::False
                | Func::Not
                | Func::Boolean
                | Func::Contains
                | Func::StartsWith => Kind::Boolean,
                Func::Number | Func::Floor | Func::Ceiling | Func::Round | Func::StringLength => {
                    Kind::Number
                }
                Func::String | Func::Concat => Kind::Text,
            },
        }
    }
}

/// XPath's `round()`: to the nearest whole number, halves towards positive infinity.
fn round(n: f64) -> f64 {
    if !n.is_finite() || n.fract() == 0.0 {
        return n;
    }
    let r = (n + 0.5).floor();
    // XPath rounds -0.5 < n < 0 to negative zero.
    if r == 0.0 && n < 0.0 { -0.0 } else { r }
}

fn binary(op: BinOp, a: &Expr, b: &Expr, property: &dyn Fn(&str) -> Option<Value>) -> Value {
    match op {
        BinOp::Or => Value::Boolean(a.eval(property).boolean() || b.eval(property).boolean()),
        BinOp::And => Value::Boolean(a.eval(property).boolean() && b.eval(property).boolean()),
        _ => {
            let (x, y) = (a.eval(property), b.eval(property));
            match op {
                BinOp::Eq => Value::Boolean(equal(&x, &y)),
                BinOp::Ne => Value::Boolean(!equal(&x, &y)),
                BinOp::Lt => Value::Boolean(x.number() < y.number()),
                BinOp::Le => Value::Boolean(x.number() <= y.number()),
                BinOp::Gt => Value::Boolean(x.number() > y.number()),
                BinOp::Ge => Value::Boolean(x.number() >= y.number()),
                BinOp::Add => Value::Number(x.number() + y.number()),
                BinOp::Sub => Value::Number(x.number() - y.number()),
                BinOp::Mul => Value::Number(x.number() * y.number()),
                BinOp::Div => Value::Number(x.number() / y.number()),
                // The truncating remainder, as XPath's `mod` and IEEE `fmod`.
                BinOp::Mod => Value::Number(x.number() % y.number()),
                BinOp::Or | BinOp::And => Value::Boolean(false),
            }
        }
    }
}

/// XPath 1.0 `=` for values that are not node-sets.
fn equal(x: &Value, y: &Value) -> bool {
    match (x, y) {
        (Value::Boolean(_), _) | (_, Value::Boolean(_)) => x.boolean() == y.boolean(),
        (Value::Number(_), _) | (_, Value::Number(_)) => x.number() == y.number(),
        _ => x.text() == y.text(),
    }
}

#[cfg(test)]
#[allow(clippy::panic)] // A case that parses when it should not fails the test.
mod tests {
    use super::*;

    fn eval(src: &str) -> Value {
        match parse(src) {
            Ok(e) => e.eval(&|name| match name {
                "n" => Some(Value::Number(3.0)),
                "tier" => Some(Value::Text("gold".into())),
                "vip" => Some(Value::Boolean(true)),
                _ => None,
            }),
            Err(e) => Value::Text(format!("error: {e}")),
        }
    }

    fn num(src: &str) -> f64 {
        eval(src).number()
    }

    #[test]
    fn arithmetic_follows_xpath() {
        assert_eq!(num("1 + 2 * 3"), 7.0);
        assert_eq!(num("(1 + 2) * 3"), 9.0);
        assert_eq!(num("7 div 2"), 3.5);
        assert_eq!(num("7 mod 3"), 1.0);
        assert_eq!(num("-7 mod 3"), -1.0);
        assert_eq!(num("- - 2"), 2.0);
        assert_eq!(num("10 - 2 - 3"), 5.0, "left to right");
        assert!(num("1 div 0").is_infinite());
        assert!(num("0 div 0").is_nan());
        assert_eq!(num(".5 + 1."), 1.5);
    }

    #[test]
    fn the_reference_models_expressions_parse_and_evaluate() {
        // As the BPSim guide's example models write them, spacing included.
        assert_eq!(num("getProperty('n') - 1"), 2.0);
        assert_eq!(num("getProperty('n') -1"), 2.0);
        assert_eq!(num("getProperty('n')-1"), 2.0);
        assert_eq!(num("getProperty('n') + 1"), 4.0);
        assert_eq!(eval("getProperty('n') > 0"), Value::Boolean(true));
        assert_eq!(eval("getProperty('n') <= 0"), Value::Boolean(false));
        assert_eq!(eval("getProperty('n') = 0"), Value::Boolean(false));
        assert_eq!(num("bpsim:getProperty(\"n\") * 2"), 6.0);
    }

    #[test]
    fn comparisons_and_logic() {
        assert_eq!(
            eval("getProperty('tier') = 'gold' and getProperty('vip')"),
            Value::Boolean(true)
        );
        assert_eq!(eval("'2' = 2"), Value::Boolean(true), "compared as numbers");
        assert_eq!(eval("'a' != 'b' or false()"), Value::Boolean(true));
        assert_eq!(eval("not(getProperty('vip'))"), Value::Boolean(false));
        assert_eq!(eval("1 < 2 = true()"), Value::Boolean(true));
        assert_eq!(eval("getProperty('n') >= 3"), Value::Boolean(true));
    }

    #[test]
    fn missing_properties_read_as_an_empty_node_set() {
        assert!(num("getProperty('nothing')").is_nan());
        assert!(num("getProperty('nothing') + 1").is_nan());
        assert_eq!(
            eval("boolean(getProperty('nothing'))"),
            Value::Boolean(false)
        );
        assert_eq!(
            eval("string(getProperty('nothing'))"),
            Value::Text(String::new())
        );
    }

    #[test]
    fn functions() {
        assert_eq!(num("floor(2.7) + ceiling(2.1)"), 5.0);
        assert_eq!(num("round(2.5)"), 3.0);
        assert_eq!(num("round(-2.5)"), -2.0, "halves round up");
        assert!(num("round(-0.2)").is_sign_negative());
        assert_eq!(eval("concat('a', 1, true())"), Value::Text("a1true".into()));
        assert_eq!(eval("string(2.50)"), Value::Text("2.5".into()));
        assert_eq!(eval("string(1 div 0)"), Value::Text("Infinity".into()));
        assert_eq!(num("string-length('héllo')"), 5.0);
        assert_eq!(eval("starts-with('gold', 'go')"), Value::Boolean(true));
        assert_eq!(eval("contains('gold', 'ol')"), Value::Boolean(true));
        assert_eq!(num("number(' 12 ')"), 12.0);
        assert!(
            num("number('1e3')").is_nan(),
            "XPath numbers have no exponent"
        );
        assert!(num("number('+1')").is_nan());
    }

    #[test]
    fn unsupported_syntax_is_an_error_with_its_position() {
        for (src, at) in [
            ("", 0),
            ("1 +", 3),
            ("(1 + 2", 6),
            ("getProperty(n)", 12),
            ("getProperty('a', 'b')", 0),
            ("count(1)", 0),
            ("$x + 1", 0),
            ("/process/count", 0),
            ("1 2", 2),
            ("'open", 0),
            ("noOfIssues - 1", 0),
            ("getResource('desk', 1)", 0),
            ("not()", 0),
            ("1 | 2", 2),
        ] {
            match parse(src) {
                Ok(e) => panic!("`{src}` parsed as {e:?}"),
                Err(e) => assert_eq!(e.position, at, "`{src}`: {e}"),
            }
        }
    }

    #[test]
    fn kinds_and_properties_read() {
        let e = parse("getProperty('a') + getProperty('b') > getProperty('a')");
        let Ok(e) = e else { panic!("{e:?}") };
        assert_eq!(e.properties(), vec!["a", "b", "a"]);
        assert_eq!(e.kind(), Kind::Boolean);
        assert_eq!(parse("1 - 2").map(|e| e.kind()), Ok(Kind::Number));
        assert_eq!(parse("concat('a', 'b')").map(|e| e.kind()), Ok(Kind::Text));
        assert_eq!(
            parse("getProperty('a')").map(|e| e.kind()),
            Ok(Kind::Property)
        );
    }
}
