//! Decision tables (OMG DMN 1.6, decision D30): routing and classifying rules a model reads from
//! a `.dmn` file beside its BPMN file.
//!
//! A task, an exclusive gateway or a divert names a decision of such a file
//! ([`DecisionUse`], the `stochcraft:decision` vendor extension). As an entity enters it, the
//! decision's table is evaluated on the entity's properties and its outputs are written back as
//! properties; a gateway routes on an output directly, or by its conditions over the outputs.
//! `stochcraft-io` reads and writes the files; this module holds the tables, the subset of FEEL
//! their cells use, evaluation and the checks.
//!
//! What is supported, as the specification defines it:
//!
//! - decision tables (clause 8) with the hit policies UNIQUE (the default), FIRST, PRIORITY and
//!   COLLECT, the last with or without an aggregation (SUM, COUNT, MIN, MAX); rules in rows;
//!   input values, output values (PRIORITY's order), default output entries and rule
//!   annotations;
//! - input entries in S-FEEL's simple unary tests (clause 9.2, grammar rule 12): a comparison
//!   `< <= > >=` with a value, a value alone (equal to it), an interval such as `[1..5]`,
//!   `(1..5]` or `]1..5[`, a list of those separated by commas, `not(...)` of a list, and `-`;
//!   values are numbers, strings in double quotes, `true` and `false`;
//! - output entries that are such a value.
//!
//! Anything else is refused with a message naming it ([`DmnError`]): the hit policies ANY, RULE
//! ORDER and OUTPUT ORDER; names, dates, times and durations as values; `=` and `!=` before a
//! value; ranges and comparisons of strings or booleans; any other FEEL expression. Literal
//! expressions, decision requirement graphs and the rest of FEEL come later.
//!
//! Matching follows FEEL (clause 10.3.2.12, and the decision table semantics of clause
//! 10.3.4.5): a value is compared only with a value of its own kind, so the number 5 does not
//! equal the string `"5"`; an entity without the property gives `null`, which no comparison
//! matches, which `-` does not match, and which `not(...)` does (`null = 1` is false). Numbers are
//! `f64`, not the 34 decimal digits FEEL allows, as S-FEEL permits implementations to limit
//! precision.
//!
//! [`Compiled::check`] finds, exactly, the overlaps a UNIQUE table may not have and the inputs no
//! rule covers, each with an example: it splits each input's values at every number and string
//! the cells name, so every rule matches all of a piece or none of it.

use crate::expr::Value;

/// The namespace of DMN 1.6 models, which StochCraft writes.
pub const DMN_NS: &str = "https://www.omg.org/spec/DMN/20240513/MODEL/";

/// The namespaces of the DMN versions whose decision tables StochCraft reads (1.1 to 1.6).
pub const DMN_NAMESPACES: &[&str] = &[
    DMN_NS,
    "https://www.omg.org/spec/DMN/20230324/MODEL/",
    "https://www.omg.org/spec/DMN/20211108/MODEL/",
    "https://www.omg.org/spec/DMN/20191111/MODEL/",
    "http://www.omg.org/spec/DMN/20180521/MODEL/",
    "http://www.omg.org/spec/DMN/20151101/dmn.xsd",
];

/// The most pieces of the input space [`Compiled::check`] looks at before it stops: far more
/// than any hand-written table needs. A search stopped there with something still to find says
/// so ([`Finding::Unchecked`]).
pub const MAX_PIECES: usize = 200_000;

/// The most rules of a SUM whose possible sums [`Compiled::aggregates`] lists (2¹² - 1 sums).
pub const MAX_SUMMED: usize = 12;

/// The decisions of one `.dmn` file (a DMN `definitions`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Definitions {
    pub id: String,
    pub name: String,
    /// The model's own namespace (DMN's required `namespace` attribute).
    pub namespace: String,
    pub decisions: Vec<Decision>,
}

impl Definitions {
    /// The decision with id `id`.
    pub fn decision(&self, id: &str) -> Option<&Decision> {
        self.decisions.iter().find(|d| d.id == id)
    }

    /// The decision with id `id`, to change.
    pub fn decision_mut(&mut self, id: &str) -> Option<&mut Decision> {
        self.decisions.iter_mut().find(|d| d.id == id)
    }
}

/// A decision whose logic is a decision table.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub id: String,
    pub name: String,
    /// The name of the decision's variable, when it has one (DMN: the same as the decision's
    /// name).
    pub variable: Option<String>,
    pub table: Table,
}

impl Decision {
    /// The name of output `i`: its own name, or, for a table with one output (which DMN gives no
    /// name), the decision's variable name, else the decision's name.
    pub fn output_name(&self, i: usize) -> String {
        let own = self
            .table
            .outputs
            .get(i)
            .and_then(|o| o.name.as_deref())
            .map(str::trim)
            .filter(|n| !n.is_empty());
        match own {
            Some(n) => n.to_owned(),
            None => self
                .variable
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or(self.name.trim())
                .to_owned(),
        }
    }

    /// The decision's name, else its id: what messages call it.
    pub fn label(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.id
        } else {
            &self.name
        }
    }

    /// The table checked and ready to evaluate.
    pub fn compile(&self) -> Result<Compiled, DmnError> {
        Compiled::new(self)
    }
}

/// A decision table, as written: every cell is kept as its text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub id: Option<String>,
    pub hit_policy: HitPolicy,
    /// COLLECT's aggregation, if any.
    pub aggregation: Option<Aggregation>,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    /// The names of the rule annotation columns.
    pub annotations: Vec<String>,
    pub rules: Vec<Rule>,
}

/// Which of the matching rules a table returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HitPolicy {
    /// At most one rule matches any input (DMN's default).
    #[default]
    Unique,
    /// The first matching rule, in rule order.
    First,
    /// The matching rule whose outputs come first in the outputs' lists of values.
    Priority,
    /// Every matching rule, in rule order, or their aggregation.
    Collect,
}

impl HitPolicy {
    pub const ALL: [HitPolicy; 4] = [
        HitPolicy::Unique,
        HitPolicy::First,
        HitPolicy::Priority,
        HitPolicy::Collect,
    ];

    /// The name DMN's `hitPolicy` attribute uses.
    pub fn name(self) -> &'static str {
        match self {
            HitPolicy::Unique => "UNIQUE",
            HitPolicy::First => "FIRST",
            HitPolicy::Priority => "PRIORITY",
            HitPolicy::Collect => "COLLECT",
        }
    }

    /// The policy named `s`, or why it cannot be used.
    pub fn from_name(s: &str) -> Result<Self, DmnError> {
        match s.trim() {
            "UNIQUE" => Ok(HitPolicy::Unique),
            "FIRST" => Ok(HitPolicy::First),
            "PRIORITY" => Ok(HitPolicy::Priority),
            "COLLECT" => Ok(HitPolicy::Collect),
            other @ ("ANY" | "RULE ORDER" | "OUTPUT ORDER") => Err(DmnError::new(format!(
                "the hit policy {other} is not supported yet (UNIQUE, FIRST, PRIORITY and COLLECT are)"
            ))),
            other => Err(DmnError::new(format!("“{other}” is not a DMN hit policy"))),
        }
    }
}

/// How COLLECT combines the outputs of the matching rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aggregation {
    Sum,
    Count,
    Min,
    Max,
}

impl Aggregation {
    pub const ALL: [Aggregation; 4] = [
        Aggregation::Sum,
        Aggregation::Count,
        Aggregation::Min,
        Aggregation::Max,
    ];

    /// The name DMN's `aggregation` attribute uses.
    pub fn name(self) -> &'static str {
        match self {
            Aggregation::Sum => "SUM",
            Aggregation::Count => "COUNT",
            Aggregation::Min => "MIN",
            Aggregation::Max => "MAX",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Aggregation::ALL.into_iter().find(|a| a.name() == s.trim())
    }
}

/// An input column: what it reads, and the values it may take.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Input {
    pub id: Option<String>,
    pub label: Option<String>,
    /// The input expression: the name of the entity property it reads.
    pub expression: String,
    /// The FEEL type the expression has (`number`, `string`, `boolean`), if given.
    pub type_ref: Option<String>,
    /// The values the input may take (unary tests), if given.
    pub values: Option<String>,
}

/// An output column.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Output {
    pub id: Option<String>,
    pub label: Option<String>,
    /// The output's name; a table with one output may leave it out ([`Decision::output_name`]).
    pub name: Option<String>,
    pub type_ref: Option<String>,
    /// The values the output may take, in decreasing priority (unary tests), if given.
    pub values: Option<String>,
    /// The output when no rule matches, if given.
    pub default: Option<String>,
}

/// A rule: one test per input, one value per output, one text per annotation column.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rule {
    pub id: Option<String>,
    pub description: Option<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub annotations: Vec<String>,
}

/// Which decision of which file an element uses (the `stochcraft:decision` vendor extension).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionUse {
    /// The `.dmn` file, as a reference relative to the model's file.
    pub file: String,
    /// The decision's id in that file.
    pub decision: String,
    /// On a gateway: the output to route by, when the table has several.
    pub route: Option<String>,
}

/// Why a table, a cell or a file cannot be used: what is wrong, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmnError {
    /// Where, in words ("rule 3, input 2"); empty for the whole table.
    pub place: String,
    pub message: String,
}

impl DmnError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            place: String::new(),
            message: message.into(),
        }
    }

    fn at(mut self, place: String) -> Self {
        if self.place.is_empty() {
            self.place = place;
        } else {
            self.place = format!("{place}, {}", self.place);
        }
        self
    }
}

impl std::fmt::Display for DmnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.place.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.place, self.message)
        }
    }
}

impl std::error::Error for DmnError {}

/// The kind of a FEEL value in the supported subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Number,
    Text,
    Boolean,
}

impl Kind {
    fn of(v: &Value) -> Kind {
        match v {
            Value::Number(_) => Kind::Number,
            Value::Text(_) => Kind::Text,
            Value::Boolean(_) => Kind::Boolean,
        }
    }

    fn words(self) -> &'static str {
        match self {
            Kind::Number => "numbers",
            Kind::Text => "strings",
            Kind::Boolean => "true or false",
        }
    }
}

/// A FEEL value written in a cell: a number, a string or a boolean.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Number(f64),
    Text(String),
    Boolean(bool),
}

impl Literal {
    pub fn value(&self) -> Value {
        match self {
            Literal::Number(n) => Value::Number(*n),
            Literal::Text(s) => Value::Text(s.clone()),
            Literal::Boolean(b) => Value::Boolean(*b),
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Literal::Number(_) => Kind::Number,
            Literal::Text(_) => Kind::Text,
            Literal::Boolean(_) => Kind::Boolean,
        }
    }

    /// The value as FEEL writes it.
    pub fn feel(&self) -> String {
        match self {
            Literal::Number(n) => number_text(*n),
            Literal::Text(s) => quote(s),
            Literal::Boolean(b) => b.to_string(),
        }
    }

    /// The value as text, unquoted: what a gateway matches against its ways.
    pub fn text(&self) -> String {
        match self {
            Literal::Text(s) => s.clone(),
            other => other.feel(),
        }
    }
}

/// `v` as FEEL writes it.
pub fn value_feel(v: &Value) -> String {
    match v {
        Value::Number(n) => number_text(*n),
        Value::Text(s) => quote(s),
        Value::Boolean(b) => b.to_string(),
    }
}

fn number_text(n: f64) -> String {
    if n == 0.0 {
        "0".to_owned()
    } else {
        n.to_string()
    }
}

/// `s` as a FEEL string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A comparison before a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

/// One simple positive unary test.
#[derive(Debug, Clone, PartialEq)]
enum Positive {
    Compare(Cmp, Literal),
    Interval {
        low: f64,
        low_open: bool,
        high: f64,
        high_open: bool,
    },
}

impl Positive {
    /// FEEL's `e in (test)`: `None` is null.
    fn test(&self, v: Option<&Value>) -> Option<bool> {
        match self {
            Positive::Compare(Cmp::Eq, lit) => match v {
                // `null = x` is false for any x that is not null.
                None => Some(false),
                Some(v) if Kind::of(v) == lit.kind() => Some(*v == lit.value()),
                Some(_) => None,
            },
            Positive::Compare(op, Literal::Number(x)) => match v {
                Some(Value::Number(n)) => Some(match op {
                    Cmp::Lt => n < x,
                    Cmp::Le => n <= x,
                    Cmp::Gt => n > x,
                    Cmp::Ge => n >= x,
                    Cmp::Eq => n == x,
                }),
                _ => None,
            },
            // Refused when parsing: only numbers are compared.
            Positive::Compare(..) => None,
            Positive::Interval {
                low,
                low_open,
                high,
                high_open,
            } => match v {
                Some(Value::Number(n)) => {
                    let above = if *low_open { n > low } else { n >= low };
                    let below = if *high_open { n < high } else { n <= high };
                    Some(above && below)
                }
                _ => None,
            },
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Positive::Compare(_, lit) => lit.kind(),
            Positive::Interval { .. } => Kind::Number,
        }
    }

    fn numbers(&self, out: &mut Vec<f64>) {
        match self {
            Positive::Compare(_, Literal::Number(x)) => out.push(*x),
            Positive::Compare(..) => {}
            Positive::Interval { low, high, .. } => {
                out.push(*low);
                out.push(*high);
            }
        }
    }
}

/// A cell's unary tests, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Tests {
    /// `-`.
    any: bool,
    /// `not(...)`.
    negated: bool,
    list: Vec<Positive>,
}

impl Tests {
    /// Whether this is `-`.
    pub fn is_any(&self) -> bool {
        self.any
    }

    /// FEEL's `e in (tests)` for the positive list: true if one test holds, false if all are
    /// false, else null.
    fn contains(&self, v: Option<&Value>) -> Option<bool> {
        let mut null = false;
        for p in &self.list {
            match p.test(v) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => null = true,
            }
        }
        if null { None } else { Some(false) }
    }

    /// Whether `v` satisfies these tests as an input entry (DMN 10.3.4.5), with the input's
    /// values `values` if it has some.
    pub fn satisfies(&self, v: Option<&Value>, values: Option<&Tests>) -> bool {
        if self.any {
            return match values {
                Some(vals) => vals.satisfies(v, None),
                None => v.is_some(),
            };
        }
        let hit = self.contains(v);
        if self.negated {
            hit == Some(false)
        } else {
            hit == Some(true)
        }
    }

    /// The kind of value these tests compare with, if they name any.
    fn kind(&self) -> Option<Kind> {
        self.list.first().map(Positive::kind)
    }

    fn numbers(&self, out: &mut Vec<f64>) {
        for p in &self.list {
            p.numbers(out);
        }
    }

    fn texts(&self, out: &mut Vec<String>) {
        for p in &self.list {
            if let Positive::Compare(_, Literal::Text(s)) = p {
                out.push(s.clone());
            }
        }
    }

    /// The values, in order, if these tests are a plain list of values (what PRIORITY reads).
    fn plain_values(&self) -> Option<Vec<Literal>> {
        if self.any || self.negated {
            return None;
        }
        self.list
            .iter()
            .map(|p| match p {
                Positive::Compare(Cmp::Eq, lit) => Some(lit.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Parses a cell's unary tests (an input entry, or an input's or output's values).
pub fn parse_tests(src: &str) -> Result<Tests, DmnError> {
    let s = src.trim();
    if s.is_empty() {
        return Err(DmnError::new("the cell is empty: write - for any value"));
    }
    if s == "-" {
        return Ok(Tests {
            any: true,
            negated: false,
            list: Vec::new(),
        });
    }
    let (negated, body) = match s.strip_prefix("not") {
        Some(rest) if rest.trim_start().starts_with('(') => {
            let inner = rest.trim_start();
            let Some(inner) = inner.strip_prefix('(').and_then(|r| r.strip_suffix(')')) else {
                return Err(DmnError::new(format!(
                    "“{s}” is not complete: write not( and the tests, then )"
                )));
            };
            (true, inner)
        }
        _ => (false, s),
    };
    let mut list = Vec::new();
    for item in split_list(body)? {
        list.push(parse_positive(item.trim())?);
    }
    if list.is_empty() {
        return Err(DmnError::new(format!("“{s}” has no test in it")));
    }
    let kind = list.first().map(Positive::kind);
    if list.iter().any(|p| Some(p.kind()) != kind) {
        return Err(DmnError::new(format!(
            "“{s}” mixes values of different kinds"
        )));
    }
    Ok(Tests {
        any: false,
        negated,
        list,
    })
}

/// Splits a list at its top-level commas (not those in strings).
fn split_list(s: &str) -> Result<Vec<&str>, DmnError> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            ',' => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if in_string {
        return Err(DmnError::new(format!(
            "“{s}” has a string with no closing \""
        )));
    }
    out.push(&s[start..]);
    if out.iter().any(|p| p.trim().is_empty()) {
        return Err(DmnError::new(format!(
            "“{s}” has an empty item in its list"
        )));
    }
    Ok(out)
}

fn parse_positive(s: &str) -> Result<Positive, DmnError> {
    for (prefix, op) in [
        ("<=", Cmp::Le),
        (">=", Cmp::Ge),
        ("<", Cmp::Lt),
        (">", Cmp::Gt),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return match parse_literal(rest)? {
                Literal::Number(x) => Ok(Positive::Compare(op, Literal::Number(x))),
                Literal::Text(_) => Err(DmnError::new(format!(
                    "“{s}” compares strings, which is not supported yet: list the strings instead"
                ))),
                Literal::Boolean(_) => Err(DmnError::new(format!(
                    "“{s}” compares a boolean: write true or false alone"
                ))),
            };
        }
    }
    if s.starts_with("!=") || s.starts_with('=') {
        return Err(DmnError::new(format!(
            "“{s}”: = and != before a value are not in S-FEEL's unary tests; write the value alone, or not(value)"
        )));
    }
    if s.starts_with(['[', '(', ']']) {
        return parse_interval(s);
    }
    Ok(Positive::Compare(Cmp::Eq, parse_literal(s)?))
}

fn parse_interval(s: &str) -> Result<Positive, DmnError> {
    let low_open = !s.starts_with('[');
    let high_open = !s.ends_with(']');
    if !s.ends_with([']', ')', '[']) || s.chars().count() < 2 {
        return Err(DmnError::new(format!(
            "“{s}” is not a complete range: write it as [1..5], (1..5), ]1..5[ or a mix"
        )));
    }
    let inner = &s[1..s.len() - 1];
    let Some((a, b)) = inner.split_once("..") else {
        return Err(DmnError::new(format!(
            "“{s}” is not a range: write its ends with .. between them, as in [1..5]"
        )));
    };
    let end = |t: &str| -> Result<f64, DmnError> {
        match parse_literal(t)? {
            Literal::Number(x) => Ok(x),
            _ => Err(DmnError::new(format!(
                "“{s}”: only ranges of numbers are supported yet"
            ))),
        }
    };
    let (low, high) = (end(a)?, end(b)?);
    if low > high {
        return Err(DmnError::new(format!("“{s}” starts above where it ends")));
    }
    Ok(Positive::Interval {
        low,
        low_open,
        high,
        high_open,
    })
}

/// Parses a FEEL value: a number, a string in double quotes, `true` or `false` (an output
/// entry, or a value in a test).
pub fn parse_literal(src: &str) -> Result<Literal, DmnError> {
    let s = src.trim();
    if s.is_empty() {
        return Err(DmnError::new("a value is missing"));
    }
    if let Some(body) = s.strip_prefix('"') {
        return parse_string(s, body).map(Literal::Text);
    }
    match s {
        "true" => return Ok(Literal::Boolean(true)),
        "false" => return Ok(Literal::Boolean(false)),
        "null" => {
            return Err(DmnError::new("null as a value is not supported yet"));
        }
        _ => {}
    }
    if let Some(x) = parse_number(s) {
        return Ok(Literal::Number(x));
    }
    let lower = s.to_ascii_lowercase();
    if ["date(", "time(", "duration(", "date and time("]
        .iter()
        .any(|p| lower.replace(' ', "").starts_with(&p.replace(' ', "")))
        || s.starts_with('@')
    {
        return Err(DmnError::new(format!(
            "“{s}”: dates, times and durations are not supported yet"
        )));
    }
    let name_like = s
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '?');
    if name_like
        && s.chars()
            .all(|c| c.is_alphanumeric() || " _.?'".contains(c))
    {
        return Err(DmnError::new(format!(
            "“{s}” is a name: names as values are not supported yet; write a number, a string in double quotes, true or false"
        )));
    }
    Err(DmnError::new(format!(
        "“{s}” is not in the supported subset of FEEL: write a number, a string in double quotes, true or false"
    )))
}

/// FEEL's numeric literal: an optional `-`, then digits with an optional fraction, or `.` and
/// digits.
fn parse_number(s: &str) -> Option<f64> {
    let digits = s.strip_prefix('-').unwrap_or(s).trim_start();
    let (whole, frac) = match digits.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (digits, None),
    };
    let all_digits = |t: &str| t.chars().all(|c| c.is_ascii_digit());
    let ok = all_digits(whole)
        && frac.is_none_or(|f| !f.is_empty() && all_digits(f))
        && !(whole.is_empty() && frac.is_none());
    if !ok {
        return None;
    }
    let text: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let text = if text.starts_with('.') || text.starts_with("-.") {
        text.replacen('.', "0.", 1)
    } else {
        text
    };
    text.parse::<f64>().ok().filter(|x| x.is_finite())
}

fn parse_string(whole: &str, body: &str) -> Result<String, DmnError> {
    let mut out = String::new();
    let mut chars = body.chars();
    loop {
        match chars.next() {
            None => {
                return Err(DmnError::new(format!("“{whole}” has no closing \"")));
            }
            Some('"') => {
                let rest: String = chars.collect();
                if !rest.trim().is_empty() {
                    return Err(DmnError::new(format!(
                        "“{whole}” goes on after its closing \""
                    )));
                }
                return Ok(out);
            }
            Some('\\') => match chars.next() {
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let c = u32::from_str_radix(&hex, 16)
                        .ok()
                        .filter(|_| hex.len() == 4)
                        .and_then(char::from_u32);
                    match c {
                        Some(c) => out.push(c),
                        None => {
                            return Err(DmnError::new(format!(
                                "“{whole}” has a \\u escape that is not four hex digits of a character"
                            )));
                        }
                    }
                }
                _ => {
                    return Err(DmnError::new(format!(
                        "“{whole}” has an escape FEEL does not know"
                    )));
                }
            },
            Some(c) => out.push(c),
        }
    }
}

/// A FEEL type name StochCraft understands: `number`, `string` or `boolean` (with or without
/// DMN 1.1's `feel:` prefix), and the integer names some tools write for a number.
fn type_kind(type_ref: Option<&str>) -> Result<Option<(Kind, bool)>, DmnError> {
    let Some(t) = type_ref.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    let bare = t.strip_prefix("feel:").unwrap_or(t);
    Ok(Some(match bare {
        "number" | "double" | "decimal" => (Kind::Number, false),
        "integer" | "long" | "int" => (Kind::Number, true),
        "string" => (Kind::Text, false),
        "boolean" => (Kind::Boolean, false),
        "Any" => return Ok(None),
        other => {
            return Err(DmnError::new(format!(
                "the type “{other}” is not supported yet (number, string and boolean are)"
            )));
        }
    }))
}

/// Whether `s` is a name StochCraft reads as an entity property: letters, digits and `_`,
/// starting with a letter or `_` (FEEL names may have more, which are not supported yet).
pub fn is_property_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// An input column, checked.
#[derive(Debug, Clone, PartialEq)]
struct ColumnIn {
    name: String,
    values: Option<Tests>,
    kind: Option<Kind>,
    integer: bool,
    /// Every number and string its tests and values name.
    numbers: Vec<f64>,
    texts: Vec<String>,
}

/// An output column, checked.
#[derive(Debug, Clone, PartialEq)]
struct ColumnOut {
    name: String,
    kind: Option<Kind>,
    /// The output's values in decreasing priority, if given as a list of values.
    order: Option<Vec<Literal>>,
    default: Option<Literal>,
}

/// A rule, checked.
#[derive(Debug, Clone, PartialEq)]
struct RuleIn {
    tests: Vec<Tests>,
    outputs: Vec<Literal>,
}

/// What a table gives for one input.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// No rule matched and there is no default (FEEL's `null`); also a UNIQUE table that
    /// matched more than one rule, which [`Compiled::check`] reports.
    Null,
    /// One value per output: of the rule hit, or the defaults (`None`: an output with no
    /// default, null).
    Row(Vec<Option<Value>>),
    /// COLLECT without an aggregation: each matching rule's outputs, in rule order.
    Rows(Vec<Vec<Value>>),
    /// COLLECT with an aggregation: one number.
    Number(f64),
}

/// A problem [`Compiled::check`] finds.
#[derive(Debug, Clone, PartialEq)]
pub enum Finding {
    /// Two rules of a UNIQUE table (numbered from 1) both match `example`.
    Overlap {
        first: usize,
        second: usize,
        example: String,
    },
    /// No rule matches `example` (and there is no default output).
    Gap { example: String },
    /// The table has too many pieces of input to look at all of them ([`MAX_PIECES`]): the
    /// search stopped before it could tell whether rules overlap or an input is not covered, so
    /// the table is not known to be free of either.
    Unchecked,
}

/// A decision table checked and ready to evaluate.
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    hit_policy: HitPolicy,
    aggregation: Option<Aggregation>,
    inputs: Vec<ColumnIn>,
    outputs: Vec<ColumnOut>,
    rules: Vec<RuleIn>,
}

impl Compiled {
    fn new(d: &Decision) -> Result<Self, DmnError> {
        let t = &d.table;
        if t.outputs.is_empty() {
            return Err(DmnError::new("the table has no output column"));
        }
        if t.aggregation.is_some() && t.hit_policy != HitPolicy::Collect {
            return Err(DmnError::new(format!(
                "an aggregation applies only to the COLLECT hit policy, not {}",
                t.hit_policy.name()
            )));
        }
        if t.aggregation.is_some() && t.outputs.len() > 1 {
            return Err(DmnError::new(
                "a table with several outputs cannot aggregate them (DMN 8.2.11)",
            ));
        }
        let mut inputs = Vec::new();
        for (i, input) in t.inputs.iter().enumerate() {
            let place = format!("input {}", i + 1);
            let name = input.expression.trim();
            if !is_property_name(name) {
                return Err(DmnError::new(format!(
                    "the input expression “{name}” is not supported yet: write the name of an entity property (letters, digits and underscores)"
                ))
                .at(place));
            }
            let typed = type_kind(input.type_ref.as_deref()).map_err(|e| e.at(place.clone()))?;
            let values = match input.values.as_deref() {
                Some(v) => Some(parse_tests(v).map_err(|e| e.at(format!("{place} values")))?),
                None => None,
            };
            inputs.push(ColumnIn {
                name: name.to_owned(),
                values,
                kind: typed.map(|(k, _)| k),
                integer: typed.is_some_and(|(_, i)| i),
                numbers: Vec::new(),
                texts: Vec::new(),
            });
        }
        let mut outputs = Vec::new();
        for (j, output) in t.outputs.iter().enumerate() {
            let place = format!("output {}", j + 1);
            let name = d.output_name(j);
            if !is_property_name(&name) {
                return Err(DmnError::new(format!(
                    "the output name “{name}” is not supported yet: name it as an entity property (letters, digits and underscores)"
                ))
                .at(place));
            }
            if outputs
                .iter()
                .any(|(o, _): &(ColumnOut, Option<Tests>)| o.name == name)
            {
                return Err(DmnError::new(format!("two outputs are named “{name}”")).at(place));
            }
            let typed = type_kind(output.type_ref.as_deref()).map_err(|e| e.at(place.clone()))?;
            let values = match output.values.as_deref() {
                Some(v) => Some(parse_tests(v).map_err(|e| e.at(format!("{place} values")))?),
                None => None,
            };
            let default = match output.default.as_deref() {
                Some(v) => Some(parse_literal(v).map_err(|e| e.at(format!("{place} default")))?),
                None => None,
            };
            let order = values.as_ref().and_then(Tests::plain_values);
            if t.hit_policy == HitPolicy::Priority && values.is_some() && order.is_none() {
                return Err(DmnError::new(
                    "under PRIORITY the output values must be a list of values, highest priority first",
                )
                .at(place));
            }
            let mut col = ColumnOut {
                name,
                kind: typed.map(|(k, _)| k),
                order,
                default: None,
            };
            if let Some(dv) = default {
                check_output(&mut col, values.as_ref(), &dv)
                    .map_err(|e| e.at(format!("{place} default")))?;
                col.default = Some(dv);
            }
            outputs.push((col, values));
        }
        if t.hit_policy == HitPolicy::Priority && outputs.iter().all(|(c, _)| c.order.is_none()) {
            return Err(DmnError::new(
                "the PRIORITY hit policy needs output values that list each output's values in priority order",
            ));
        }
        let mut rules = Vec::new();
        for (r, rule) in t.rules.iter().enumerate() {
            let place = |what: String| format!("rule {}, {what}", r + 1);
            if rule.inputs.len() != t.inputs.len() {
                return Err(DmnError::new(format!(
                    "the rule has {} input entries for {} inputs",
                    rule.inputs.len(),
                    t.inputs.len()
                ))
                .at(format!("rule {}", r + 1)));
            }
            if rule.outputs.len() != t.outputs.len() {
                return Err(DmnError::new(format!(
                    "the rule has {} output entries for {} outputs",
                    rule.outputs.len(),
                    t.outputs.len()
                ))
                .at(format!("rule {}", r + 1)));
            }
            let mut tests = Vec::new();
            for (i, cell) in rule.inputs.iter().enumerate() {
                let at = place(format!("input {}", i + 1));
                let parsed = parse_tests(cell).map_err(|e| e.at(at.clone()))?;
                if let (Some(k), Some(col)) = (parsed.kind(), inputs.get_mut(i)) {
                    match col.kind {
                        Some(c) if c != k => {
                            return Err(DmnError::new(format!(
                                "the input {} holds {}, but this test compares {}",
                                col.name,
                                c.words(),
                                k.words()
                            ))
                            .at(at));
                        }
                        Some(_) => {}
                        None => col.kind = Some(k),
                    }
                }
                tests.push(parsed);
            }
            let mut values = Vec::new();
            for (j, cell) in rule.outputs.iter().enumerate() {
                let at = place(format!("output {}", j + 1));
                let v = parse_literal(cell).map_err(|e| e.at(at.clone()))?;
                if let Some((col, allowed)) = outputs.get_mut(j) {
                    check_output(col, allowed.as_ref(), &v).map_err(|e| e.at(at.clone()))?;
                }
                values.push(v);
            }
            rules.push(RuleIn {
                tests,
                outputs: values,
            });
        }
        // The input's own values fix its kind as well; and the pieces its values fall into are
        // cut at every number and string the tests and values name.
        for (i, col) in inputs.iter_mut().enumerate() {
            let (mut numbers, mut texts) = (Vec::new(), Vec::new());
            for t in rules
                .iter()
                .filter_map(|r| r.tests.get(i))
                .chain(&col.values)
            {
                t.numbers(&mut numbers);
                t.texts(&mut texts);
            }
            col.numbers = numbers;
            col.texts = texts;
            if let Some(k) = col.values.as_ref().and_then(Tests::kind) {
                match col.kind {
                    Some(c) if c != k => {
                        return Err(DmnError::new(format!(
                            "the input {} holds {}, but its values are {}",
                            col.name,
                            c.words(),
                            k.words()
                        )));
                    }
                    _ => col.kind = Some(k),
                }
            }
        }
        let outputs: Vec<ColumnOut> = outputs.into_iter().map(|(c, _)| c).collect();
        if let Some(a) = t.aggregation
            && a != Aggregation::Count
            && outputs
                .iter()
                .any(|o| o.kind.is_some_and(|k| k != Kind::Number))
        {
            return Err(DmnError::new(format!(
                "the aggregation {} needs number outputs",
                a.name()
            )));
        }
        Ok(Self {
            hit_policy: t.hit_policy,
            aggregation: t.aggregation,
            inputs,
            outputs,
            rules,
        })
    }

    pub fn hit_policy(&self) -> HitPolicy {
        self.hit_policy
    }

    pub fn aggregation(&self) -> Option<Aggregation> {
        self.aggregation
    }

    /// Whether the table gives a list (COLLECT without an aggregation), which one entity
    /// property per output cannot hold.
    pub fn gives_list(&self) -> bool {
        self.hit_policy == HitPolicy::Collect && self.aggregation.is_none()
    }

    /// The property names the inputs read, in column order.
    pub fn input_names(&self) -> Vec<&str> {
        self.inputs.iter().map(|c| c.name.as_str()).collect()
    }

    /// The output names, in column order, with the kind of value each gives (a number for an
    /// aggregation), if known.
    pub fn outputs(&self) -> Vec<(&str, Option<Kind>)> {
        self.outputs
            .iter()
            .map(|o| {
                let kind = if self.aggregation.is_some() {
                    Some(Kind::Number)
                } else {
                    o.kind
                };
                (o.name.as_str(), kind)
            })
            .collect()
    }

    /// Every value output `j` can give: each rule's entry and the default, in rule order, once
    /// each.
    pub fn output_values(&self, j: usize) -> Vec<Literal> {
        let mut out: Vec<Literal> = Vec::new();
        let all = self
            .rules
            .iter()
            .filter_map(|r| r.outputs.get(j))
            .chain(self.outputs.get(j).and_then(|o| o.default.as_ref()));
        for v in all {
            if !out.contains(v) {
                out.push(v.clone());
            }
        }
        out
    }

    /// Output `j`'s default (DMN's default output entry), given when no rule matches, before any
    /// aggregation: a COLLECT table with an aggregation gives it as it is.
    pub fn default_value(&self, j: usize) -> Option<&Literal> {
        self.outputs.get(j)?.default.as_ref()
    }

    /// The numbers the aggregation can give when a rule matches, each once, least first: COUNT
    /// every count from 1 to the number of rules; MIN and MAX a rule's output; SUM the sum of
    /// the outputs of some of the rules, added in rule order as evaluation adds them. Not all of
    /// them need be reachable (rules that never match together still have their sum listed).
    /// `None` without an aggregation, or for a SUM over more than [`MAX_SUMMED`] rules, whose
    /// sums are too many to list.
    pub fn aggregates(&self) -> Option<Vec<f64>> {
        let a = self.aggregation?;
        let cells: Vec<f64> = self
            .rules
            .iter()
            .filter_map(|r| match r.outputs.first() {
                Some(Literal::Number(x)) => Some(*x),
                _ => None,
            })
            .collect();
        let mut out: Vec<f64> = match a {
            Aggregation::Count => (1..=self.rules.len()).map(|n| n as f64).collect(),
            Aggregation::Min | Aggregation::Max => cells,
            Aggregation::Sum => {
                if cells.len() > MAX_SUMMED {
                    return None;
                }
                (1..(1_usize << cells.len()))
                    .map(|set| {
                        cells
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| (set >> i) & 1 == 1)
                            .map(|(_, x)| *x)
                            .sum::<f64>()
                    })
                    .collect()
            }
        };
        out.sort_by(f64::total_cmp);
        out.dedup();
        Some(out)
    }

    /// Whether the table has a default output, so that every input gets a result.
    pub fn has_default(&self) -> bool {
        self.outputs.iter().any(|o| o.default.is_some())
    }

    /// The rules (numbered from 0) that match the inputs `read` gives, in rule order.
    pub fn matches(&self, read: &dyn Fn(&str) -> Option<Value>) -> Vec<usize> {
        let values: Vec<Option<Value>> = self.inputs.iter().map(|c| read(&c.name)).collect();
        self.matching(&values)
    }

    fn matching(&self, values: &[Option<Value>]) -> Vec<usize> {
        self.rules
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.tests
                    .iter()
                    .zip(&self.inputs)
                    .zip(values)
                    .all(|((t, c), v)| t.satisfies(v.as_ref(), c.values.as_ref()))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Evaluates the table on the inputs `read` gives (by property name; `None` is null).
    ///
    /// An input's values are the values it allows (DMN 8.3.3): a value outside them is an error,
    /// which gives null, as a UNIQUE table matching more than one rule does, and not the
    /// default output. A missing property is no such value.
    pub fn evaluate(&self, read: &dyn Fn(&str) -> Option<Value>) -> Outcome {
        let values: Vec<Option<Value>> = self.inputs.iter().map(|c| read(&c.name)).collect();
        let outside = self.inputs.iter().zip(&values).any(|(c, v)| {
            v.is_some()
                && c.values
                    .as_ref()
                    .is_some_and(|allowed| !allowed.satisfies(v.as_ref(), None))
        });
        if outside {
            return Outcome::Null;
        }
        let hits = self.matching(&values);
        let values = |i: usize| -> Vec<Value> {
            self.rules
                .get(i)
                .map(|r| r.outputs.iter().map(Literal::value).collect())
                .unwrap_or_default()
        };
        let row = |i: usize| -> Vec<Option<Value>> { values(i).into_iter().map(Some).collect() };
        if hits.is_empty() {
            if !self.has_default() {
                return Outcome::Null;
            }
            // Outputs without a default give null; their properties are cleared.
            return Outcome::Row(
                self.outputs
                    .iter()
                    .map(|o| o.default.as_ref().map(Literal::value))
                    .collect(),
            );
        }
        match self.hit_policy {
            HitPolicy::Unique => match hits.as_slice() {
                [one] => Outcome::Row(row(*one)),
                _ => Outcome::Null,
            },
            HitPolicy::First => hits
                .first()
                .map_or(Outcome::Null, |i| Outcome::Row(row(*i))),
            HitPolicy::Priority => {
                let rank = |i: usize| -> Vec<usize> {
                    let Some(r) = self.rules.get(i) else {
                        return Vec::new();
                    };
                    self.outputs
                        .iter()
                        .zip(&r.outputs)
                        .filter_map(|(o, v)| {
                            let order = o.order.as_ref()?;
                            Some(order.iter().position(|x| x == v).unwrap_or(order.len()))
                        })
                        .collect()
                };
                // The highest priority; between equals, the first in rule order.
                hits.iter()
                    .copied()
                    .min_by(|a, b| rank(*a).cmp(&rank(*b)).then(a.cmp(b)))
                    .map_or(Outcome::Null, |i| Outcome::Row(row(i)))
            }
            HitPolicy::Collect => {
                let first: Vec<&Literal> = hits
                    .iter()
                    .filter_map(|i| self.rules.get(*i)?.outputs.first())
                    .collect();
                let numbers = || {
                    first
                        .iter()
                        .map(|v| match v {
                            Literal::Number(x) => Some(*x),
                            _ => None,
                        })
                        .collect::<Option<Vec<f64>>>()
                };
                match self.aggregation {
                    None => Outcome::Rows(hits.iter().map(|i| values(*i)).collect()),
                    Some(Aggregation::Count) => Outcome::Number(hits.len() as f64),
                    Some(Aggregation::Sum) => {
                        numbers().map_or(Outcome::Null, |xs| Outcome::Number(xs.iter().sum()))
                    }
                    Some(Aggregation::Min) => numbers()
                        .and_then(|xs| xs.into_iter().reduce(f64::min))
                        .map_or(Outcome::Null, Outcome::Number),
                    Some(Aggregation::Max) => numbers()
                        .and_then(|xs| xs.into_iter().reduce(f64::max))
                        .map_or(Outcome::Null, Outcome::Number),
                }
            }
        }
    }

    /// The value to give each output's property for the inputs `read` gives: `None` clears it
    /// (a null result). `None` overall for a table that gives a list ([`Compiled::gives_list`]).
    pub fn properties(&self, read: &dyn Fn(&str) -> Option<Value>) -> Option<Vec<Option<Value>>> {
        let n = self.outputs.len();
        Some(match self.evaluate(read) {
            Outcome::Null => vec![None; n],
            Outcome::Row(values) => values,
            Outcome::Number(x) => vec![Some(Value::Number(x))],
            Outcome::Rows(_) => return None,
        })
    }

    /// Finds the overlaps a UNIQUE table may not have and the inputs no rule covers (unless
    /// there is a default output), the first of each, each with an example. `integer` says,
    /// per input, whether its property holds whole numbers only (a `long`), so that the gaps
    /// between whole numbers are no gaps.
    pub fn check(&self, integer: &[bool]) -> Vec<Finding> {
        let whole: Vec<bool> = self
            .inputs
            .iter()
            .enumerate()
            .map(|(i, c)| c.integer || integer.get(i).copied().unwrap_or(false))
            .collect();
        self.search(&whole, false)
    }

    /// As [`Compiled::check`], for a table a block uses: `whole` says, per input, whether the
    /// property it reads holds whole numbers only (a `long`). The types the table itself gives
    /// its inputs do not count here, since what reaches them is the property's values; and a
    /// `long` holds only the numbers of its 64 bits, so no example lies beyond them.
    pub fn check_bound(&self, whole: &[bool]) -> Vec<Finding> {
        let whole: Vec<bool> = (0..self.inputs.len())
            .map(|i| whole.get(i).copied().unwrap_or(false))
            .collect();
        self.search(&whole, true)
    }

    /// Of each input, the first input reading the same property: the inputs reading one
    /// property see one value.
    fn leaders(&self) -> Vec<usize> {
        self.inputs
            .iter()
            .enumerate()
            .map(|(i, c)| {
                self.inputs
                    .iter()
                    .position(|d| d.name == c.name)
                    .unwrap_or(i)
            })
            .collect()
    }

    /// The search of [`Compiled::check`]: `whole` says, per input, whether its property holds
    /// whole numbers only; with `long`, those are the numbers a 64-bit `long` can hold.
    fn search(&self, whole: &[bool], long: bool) -> Vec<Finding> {
        let leader = self.leaders();
        let want_overlap = self.hit_policy == HitPolicy::Unique;
        // The pieces of each property's values, at its first input (the others follow it), cut
        // at what all its inputs name. And a missing property (null), which `not(...)` matches:
        // two rules can overlap there, though a gap there is none (`-` does not match it).
        let atoms: Vec<Vec<Option<Value>>> = (0..self.inputs.len())
            .map(|i| {
                if leader.get(i) != Some(&i) {
                    return Vec::new();
                }
                let group: Vec<&ColumnIn> = self
                    .inputs
                    .iter()
                    .zip(&leader)
                    .filter(|(_, l)| **l == i)
                    .map(|(c, _)| c)
                    .collect();
                let integer = leader.iter().zip(whole).any(|(l, w)| *l == i && *w);
                let mut pieces = pieces_of(&group, integer);
                if integer && long {
                    pieces.retain(fits_long);
                }
                let mut out: Vec<Option<Value>> = pieces.into_iter().map(Some).collect();
                if want_overlap {
                    out.push(None);
                }
                out
            })
            .collect();
        let mut search = Search {
            table: self,
            atoms: &atoms,
            leader: &leader,
            overlap: None,
            gap: None,
            pieces: 0,
            want_overlap,
            want_gap: !self.has_default(),
        };
        let all: Vec<usize> = (0..self.rules.len()).collect();
        search.walk(0, &all, &mut Vec::new());
        // Stopped at the limit with something still to find: what was not looked at may hold it.
        let cut = search.pieces >= MAX_PIECES && !search.found_all();
        let mut out = Vec::new();
        if let Some((a, b, example)) = search.overlap {
            out.push(Finding::Overlap {
                first: a + 1,
                second: b + 1,
                example,
            });
        }
        if let Some(example) = search.gap {
            out.push(Finding::Gap { example });
        }
        if cut {
            out.push(Finding::Unchecked);
        }
        out
    }

    /// Describes a point of the input space: `size = 5.5 and class = "B"` (`null` for a missing
    /// property); inputs left open are left out, and so is a property named again.
    fn describe(&self, point: &[Option<Value>]) -> String {
        let leader = self.leaders();
        let parts: Vec<String> = self
            .inputs
            .iter()
            .zip(point)
            .enumerate()
            .filter(|(i, _)| leader.get(*i) == Some(i))
            .map(|(_, (c, v))| {
                let v = v.as_ref().map_or_else(|| "null".to_owned(), value_feel);
                format!("{} = {v}", c.name)
            })
            .collect();
        if parts.is_empty() {
            "any entity".to_owned()
        } else {
            parts.join(" and ")
        }
    }
}

/// Checks output value `v` against its column's kind and allowed values, fixing the kind if
/// it was open.
fn check_output(col: &mut ColumnOut, allowed: Option<&Tests>, v: &Literal) -> Result<(), DmnError> {
    match col.kind {
        Some(k) if k != v.kind() => {
            return Err(DmnError::new(format!(
                "the output {} holds {}, not “{}”",
                col.name,
                k.words(),
                v.feel()
            )));
        }
        Some(_) => {}
        None => col.kind = Some(v.kind()),
    }
    if let Some(allowed) = allowed
        && !allowed.satisfies(Some(&v.value()), None)
    {
        return Err(DmnError::new(format!(
            "“{}” is not one of the output's values",
            v.feel()
        )));
    }
    Ok(())
}

/// Whether `v`, a number, is one a `long` property can give: from -2⁶³ up to the float nearest
/// 2⁶³ - 1, which is 2⁶³ itself (a `long` is read as the nearest float).
fn fits_long(v: &Value) -> bool {
    match v {
        Value::Number(x) => (LONG_LOW..=-LONG_LOW).contains(x),
        _ => true,
    }
}

/// -2⁶³, the least `long`, exactly.
const LONG_LOW: f64 = -9_223_372_036_854_775_808.0;

/// The next number above `x` (a whole number if `integer`) that is not `x` itself, if there is
/// one: `x + 1`, or where that rounds back to `x` (beyond 2⁵³), the next float.
fn above(x: f64, integer: bool) -> Option<f64> {
    let y = if integer { x.floor() + 1.0 } else { x + 1.0 };
    let y = if y > x { y } else { x.next_up() };
    (y > x && y.is_finite()).then_some(y)
}

/// As [`above`], below `x`.
fn below(x: f64, integer: bool) -> Option<f64> {
    let y = if integer { x.ceil() - 1.0 } else { x - 1.0 };
    let y = if y < x { y } else { x.next_down() };
    (y < x && y.is_finite()).then_some(y)
}

/// The pieces the values of a property fall into, as `group` (the inputs reading it) cut them:
/// one value from each, such that every test of the table holds for all of a piece or for none
/// of it. Only pieces within the inputs' own values, if they have some.
fn pieces_of(group: &[&ColumnIn], integer: bool) -> Vec<Value> {
    let in_domain = |v: &Value| {
        group
            .iter()
            .all(|c| c.values.as_ref().is_none_or(|t| t.satisfies(Some(v), None)))
    };
    let mut kinds: Vec<Kind> = Vec::new();
    for k in group.iter().filter_map(|c| c.kind) {
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    let mut out = Vec::new();
    if kinds.is_empty() {
        out.push(Value::Number(0.0));
    }
    for kind in kinds {
        match kind {
            Kind::Boolean => {
                out.push(Value::Boolean(true));
                out.push(Value::Boolean(false));
            }
            Kind::Text => {
                let mut texts: Vec<String> =
                    group.iter().flat_map(|c| c.texts.iter().cloned()).collect();
                texts.sort();
                texts.dedup();
                // A string none of the cells names stands for all the others.
                let mut other = "other".to_owned();
                let mut n = 1;
                while texts.contains(&other) {
                    other = format!("other{n}");
                    n += 1;
                }
                out.extend(texts.into_iter().map(Value::Text));
                out.push(Value::Text(other));
            }
            Kind::Number => {
                let mut b: Vec<f64> = group
                    .iter()
                    .flat_map(|c| c.numbers.iter().copied())
                    .collect();
                b.sort_by(f64::total_cmp);
                b.dedup();
                let whole = |x: f64| x.fract() == 0.0;
                let (Some(&lo), Some(&hi)) = (b.first(), b.last()) else {
                    out.push(Value::Number(0.0));
                    continue;
                };
                // Each value strictly inside a piece is one that can be held: at large
                // magnitudes `x + 1` is `x` again, and between two adjacent floats there is
                // nothing.
                out.extend(below(lo, integer).map(Value::Number));
                for (k, &x) in b.iter().enumerate() {
                    if !integer || whole(x) {
                        out.push(Value::Number(x));
                    }
                    if let Some(&next) = b.get(k + 1) {
                        let inside = if integer {
                            above(x, true)
                        } else {
                            Some(x / 2.0 + next / 2.0)
                        };
                        if let Some(m) = inside.filter(|m| x < *m && *m < next) {
                            out.push(Value::Number(m));
                        }
                    }
                }
                out.extend(above(hi, integer).map(Value::Number));
            }
        }
    }
    out.retain(in_domain);
    out
}

/// The depth-first search of [`Compiled::check`].
struct Search<'a> {
    table: &'a Compiled,
    /// Per input, the values to try (`None`: null); empty for an input that follows its leader.
    atoms: &'a [Vec<Option<Value>>],
    /// Per input, the first input reading the same property ([`Compiled::leaders`]).
    leader: &'a [usize],
    overlap: Option<(usize, usize, String)>,
    gap: Option<String>,
    pieces: usize,
    want_overlap: bool,
    want_gap: bool,
}

impl Search<'_> {
    /// Whether everything wanted has been found.
    fn found_all(&self) -> bool {
        (!self.want_overlap || self.overlap.is_some()) && (!self.want_gap || self.gap.is_some())
    }

    fn done(&self) -> bool {
        self.found_all() || self.pieces >= MAX_PIECES
    }

    fn walk(&mut self, col: usize, rules: &[usize], point: &mut Vec<Option<Value>>) {
        // Copies of the shared references, so that walking deeper can borrow `self`.
        let (table, all, leader) = (self.table, self.atoms, self.leader);
        if self.done() {
            return;
        }
        if rules.is_empty() {
            // An entity missing a property is no gap: no rule is meant to match it.
            if self.want_gap && self.gap.is_none() && point.iter().all(Option::is_some) {
                self.gap = Some(table.describe(point));
            }
            return;
        }
        let Some(atoms) = all.get(col) else {
            if self.want_overlap
                && self.overlap.is_none()
                && let [a, b, ..] = rules
            {
                self.overlap = Some((*a, *b, table.describe(point)));
            }
            return;
        };
        // A rule open on every remaining input covers what is left: no gap below, and with only
        // one rule left no overlap either.
        let open = |r: &usize| {
            table.rules.get(*r).is_some_and(|rule| {
                rule.tests
                    .iter()
                    .skip(col)
                    .zip(table.inputs.iter().skip(col))
                    .all(|(t, c)| t.is_any() && c.values.is_none())
            })
        };
        if rules.len() == 1 && open(&rules[0]) {
            return;
        }
        if !self.want_overlap && rules.iter().any(open) {
            return;
        }
        let column = table.inputs.get(col);
        // An input reading the same property as an earlier one sees that one's value.
        let follow: Vec<Option<Value>>;
        let atoms: &[Option<Value>] = match leader.get(col) {
            Some(&l) if l != col => {
                follow = vec![point.get(l).cloned().flatten()];
                &follow
            }
            _ => atoms,
        };
        for atom in atoms {
            self.pieces += 1;
            if self.done() {
                return;
            }
            let values = column.and_then(|c| c.values.as_ref());
            let here: Vec<usize> = rules
                .iter()
                .copied()
                .filter(|r| {
                    table
                        .rules
                        .get(*r)
                        .and_then(|rule| rule.tests.get(col))
                        .is_some_and(|t| t.satisfies(atom.as_ref(), values))
                })
                .collect();
            point.push(atom.clone());
            self.walk(col + 1, &here, point);
            point.pop();
        }
    }
}

#[cfg(test)]
mod tests;
