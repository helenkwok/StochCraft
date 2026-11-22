//! Equations: XMILE 1.0's expression syntax (section 3.3), read into a tree.
//!
//! Operators from the tightest: `^` (right to left), unary `+`, `-` and `NOT` (right to left),
//! `*`, `/` and `MOD`, `+` and `-`, `<`, `<=`, `>` and `>=`, `=` and `<>`, `AND`, `OR`. So `-x^2`
//! is `-(x^2)`. `IF condition THEN a ELSE b` is an expression. A name with spaces is written in
//! double quotes. Words and function names are not case-sensitive.

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Mod,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

/// A parsed equation, with names not yet resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum Ast {
    Num(f64),
    /// A name, in its canonical form.
    Name(String),
    Neg(Box<Ast>),
    Not(Box<Ast>),
    Bin(BinOp, Box<Ast>, Box<Ast>),
    If(Box<Ast>, Box<Ast>, Box<Ast>),
    /// A function call; the name is in its canonical form.
    Call(String, Vec<Ast>),
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
    Comma,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

fn lex(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let next = chars.get(i + 1).copied();
        match c {
            c if c.is_whitespace() => i += 1,
            '+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            '/' => {
                out.push(Tok::Slash);
                i += 1;
            }
            '^' => {
                out.push(Tok::Caret);
                i += 1;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '=' => {
                out.push(Tok::Eq);
                i += 1;
            }
            '<' => match next {
                Some('=') => {
                    out.push(Tok::Le);
                    i += 2;
                }
                Some('>') => {
                    out.push(Tok::Ne);
                    i += 2;
                }
                _ => {
                    out.push(Tok::Lt);
                    i += 1;
                }
            },
            '>' => {
                if next == Some('=') {
                    out.push(Tok::Ge);
                    i += 2;
                } else {
                    out.push(Tok::Gt);
                    i += 1;
                }
            }
            '[' | ']' => {
                return Err("subscripts (arrays) are not supported".to_owned());
            }
            '"' => {
                let mut j = i + 1;
                let mut name = String::new();
                loop {
                    match chars.get(j) {
                        Some('"') => break,
                        Some('\\') => {
                            match chars.get(j + 1) {
                                Some('n') => name.push(' '),
                                Some(&e) => name.push(e),
                                None => return Err("a quoted name is not closed".to_owned()),
                            }
                            j += 2;
                        }
                        Some(&e) => {
                            name.push(e);
                            j += 1;
                        }
                        None => return Err("a quoted name is not closed".to_owned()),
                    }
                }
                out.push(Tok::Ident(crate::model::canonical(&name)));
                i = j + 1;
            }
            c if c.is_ascii_digit() || (c == '.' && next.is_some_and(|d| d.is_ascii_digit())) => {
                let mut j = i;
                while chars
                    .get(j)
                    .is_some_and(|d| d.is_ascii_digit() || *d == '.')
                {
                    j += 1;
                }
                if chars.get(j).is_some_and(|e| *e == 'e' || *e == 'E') {
                    let mut k = j + 1;
                    if chars.get(k).is_some_and(|s| *s == '+' || *s == '-') {
                        k += 1;
                    }
                    if chars.get(k).is_some_and(char::is_ascii_digit) {
                        while chars.get(k).is_some_and(char::is_ascii_digit) {
                            k += 1;
                        }
                        j = k;
                    }
                }
                let s: String = chars.get(i..j).unwrap_or_default().iter().collect();
                let v = s
                    .parse::<f64>()
                    .map_err(|_| format!("`{s}` is not a number"))?;
                out.push(Tok::Num(v));
                i = j;
            }
            c if c.is_alphabetic() || c == '_' || c == '$' => {
                let mut j = i;
                while chars
                    .get(j)
                    .is_some_and(|d| d.is_alphanumeric() || *d == '_' || *d == '$')
                {
                    j += 1;
                }
                let s: String = chars.get(i..j).unwrap_or_default().iter().collect();
                out.push(Tok::Ident(crate::model::canonical(&s)));
                i = j;
            }
            '.' => {
                return Err(
                    "`.` is not allowed here (names inside modules are not supported)".to_owned(),
                );
            }
            other => return Err(format!("unexpected `{other}`")),
        }
    }
    Ok(out)
}

/// The deepest an equation may nest, in parentheses, signs, `IF`s and function calls: each level
/// is several nested calls of the parser (one per precedence level), so this is kept well below
/// what a small stack holds (a debug build's 2 MB test thread, a 1 MB main thread on Windows).
/// Models nest a handful of levels.
const MAX_NEST: usize = 64;

/// The tallest tree an equation may build: a long run of `+` is read in a loop, not by recursion,
/// but builds a tree as tall as the run is long, and evaluating, copying and dropping it recurse
/// once per level, with small frames.
const MAX_HEIGHT: usize = 512;

/// A parsed tree and its height (a name or number has height 1).
type Node = (Ast, usize);

struct Parser {
    toks: Vec<Tok>,
    i: usize,
    /// How many of the parser's own nested calls are open now.
    nest: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }

    fn word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(s)) if s == w)
    }

    fn bump(&mut self) {
        self.i += 1;
    }

    fn expect(&mut self, t: &Tok, what: &str) -> Result<(), String> {
        if self.peek() == Some(t) {
            self.bump();
            Ok(())
        } else {
            Err(format!("{what} was expected"))
        }
    }

    /// Opens one more nested call, or says the equation nests too deeply.
    fn enter(&mut self) -> Result<(), String> {
        self.nest += 1;
        if self.nest > MAX_NEST {
            Err(too_deep(MAX_NEST, self.i))
        } else {
            Ok(())
        }
    }

    /// The node for `op` over two operands, if the tree stays within [`MAX_HEIGHT`].
    fn bin(&self, op: BinOp, l: Node, r: Node) -> Result<Node, String> {
        let h = l.1.max(r.1) + 1;
        if h > MAX_HEIGHT {
            return Err(too_deep(MAX_HEIGHT, self.i));
        }
        Ok((Ast::Bin(op, Box::new(l.0), Box::new(r.0)), h))
    }

    fn or(&mut self) -> Result<Node, String> {
        self.enter()?;
        let mut l = self.and()?;
        while self.word("or") {
            self.bump();
            let r = self.and()?;
            l = self.bin(BinOp::Or, l, r)?;
        }
        self.nest -= 1;
        Ok(l)
    }

    fn and(&mut self) -> Result<Node, String> {
        let mut l = self.equality()?;
        while self.word("and") {
            self.bump();
            let r = self.equality()?;
            l = self.bin(BinOp::And, l, r)?;
        }
        Ok(l)
    }

    fn equality(&mut self) -> Result<Node, String> {
        let mut l = self.relation()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Eq) => BinOp::Eq,
                Some(Tok::Ne) => BinOp::Ne,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.relation()?;
            l = self.bin(op, l, r)?;
        }
    }

    fn relation(&mut self) -> Result<Node, String> {
        let mut l = self.additive()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Lt) => BinOp::Lt,
                Some(Tok::Le) => BinOp::Le,
                Some(Tok::Gt) => BinOp::Gt,
                Some(Tok::Ge) => BinOp::Ge,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.additive()?;
            l = self.bin(op, l, r)?;
        }
    }

    fn additive(&mut self) -> Result<Node, String> {
        let mut l = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Add,
                Some(Tok::Minus) => BinOp::Sub,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.multiplicative()?;
            l = self.bin(op, l, r)?;
        }
    }

    fn multiplicative(&mut self) -> Result<Node, String> {
        let mut l = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinOp::Mul,
                Some(Tok::Slash) => BinOp::Div,
                Some(Tok::Ident(s)) if s == "mod" => BinOp::Mod,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.unary()?;
            l = self.bin(op, l, r)?;
        }
    }

    fn unary(&mut self) -> Result<Node, String> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.bump();
                let (e, h) = self.nested_unary()?;
                Ok((Ast::Neg(Box::new(e)), h + 1))
            }
            Some(Tok::Plus) => {
                self.bump();
                self.nested_unary()
            }
            Some(Tok::Ident(s)) if s == "not" => {
                self.bump();
                let (e, h) = self.nested_unary()?;
                Ok((Ast::Not(Box::new(e)), h + 1))
            }
            _ => self.power(),
        }
    }

    /// An operand of a sign or of `NOT`: one call deeper, and no taller than the limit.
    fn nested_unary(&mut self) -> Result<Node, String> {
        self.enter()?;
        let n = self.unary()?;
        self.nest -= 1;
        if n.1 >= MAX_HEIGHT {
            return Err(too_deep(MAX_HEIGHT, self.i));
        }
        Ok(n)
    }

    fn power(&mut self) -> Result<Node, String> {
        let base = self.primary()?;
        if self.peek() == Some(&Tok::Caret) {
            self.bump();
            // Right to left, and the exponent may carry a sign: 2^-3^2 is 2^(-(3^2)).
            let exp = self.nested_unary()?;
            return self.bin(BinOp::Pow, base, exp);
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Node, String> {
        match self.peek().cloned() {
            Some(Tok::Num(v)) => {
                self.bump();
                Ok((Ast::Num(v), 1))
            }
            Some(Tok::LParen) => {
                self.bump();
                let e = self.or()?;
                self.expect(&Tok::RParen, "`)`")?;
                Ok(e)
            }
            Some(Tok::Ident(s)) if s == "if" => {
                self.bump();
                let (c, hc) = self.or()?;
                if !self.word("then") {
                    return Err("`THEN` was expected".to_owned());
                }
                self.bump();
                let (a, ha) = self.or()?;
                if !self.word("else") {
                    return Err("`ELSE` was expected".to_owned());
                }
                self.bump();
                let (b, hb) = self.or()?;
                let h = hc.max(ha).max(hb) + 1;
                if h > MAX_HEIGHT {
                    return Err(too_deep(MAX_HEIGHT, self.i));
                }
                Ok((Ast::If(Box::new(c), Box::new(a), Box::new(b)), h))
            }
            Some(Tok::Ident(s)) => {
                self.bump();
                if self.peek() == Some(&Tok::LParen) {
                    self.bump();
                    let mut args = Vec::new();
                    let mut h = 1;
                    if self.peek() == Some(&Tok::RParen) {
                        self.bump();
                        return Ok((Ast::Call(s, args), h));
                    }
                    loop {
                        let (a, ha) = self.or()?;
                        h = h.max(ha + 1);
                        args.push(a);
                        match self.peek() {
                            Some(Tok::Comma) => self.bump(),
                            Some(Tok::RParen) => {
                                self.bump();
                                break;
                            }
                            _ => return Err("`,` or `)` was expected".to_owned()),
                        }
                    }
                    if h > MAX_HEIGHT {
                        return Err(too_deep(MAX_HEIGHT, self.i));
                    }
                    Ok((Ast::Call(s, args), h))
                } else {
                    Ok((Ast::Name(s), 1))
                }
            }
            Some(_) => Err("a value was expected".to_owned()),
            None => Err("the equation ends too soon".to_owned()),
        }
    }
}

fn too_deep(limit: usize, token: usize) -> String {
    format!(
        "the equation nests more than {limit} deep (near token {})",
        token + 1
    )
}

/// Reads an equation. The error says what was expected. An equation may nest at most
/// [`MAX_NEST`] deep and build a tree at most [`MAX_HEIGHT`] tall.
pub fn parse(text: &str) -> Result<Ast, String> {
    let toks = lex(text)?;
    if toks.is_empty() {
        return Err("the equation is empty".to_owned());
    }
    let mut p = Parser {
        toks,
        i: 0,
        nest: 0,
    };
    let (e, _) = p.or()?;
    if p.peek().is_some() {
        return Err("something follows the end of the equation".to_owned());
    }
    Ok(e)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn num(v: f64) -> Box<Ast> {
        Box::new(Ast::Num(v))
    }

    #[test]
    fn power_binds_tighter_than_a_minus_sign() {
        let e = parse("-2^2");
        assert_eq!(
            e,
            Ok(Ast::Neg(Box::new(Ast::Bin(BinOp::Pow, num(2.0), num(2.0)))))
        );
    }

    #[test]
    fn power_goes_right_to_left() {
        assert_eq!(
            parse("2^3^2"),
            Ok(Ast::Bin(
                BinOp::Pow,
                num(2.0),
                Box::new(Ast::Bin(BinOp::Pow, num(3.0), num(2.0)))
            ))
        );
    }

    #[test]
    fn reads_if_then_else_names_and_calls() {
        let e = parse("IF Birth_Rate > 0.5 THEN MIN(a, 3) ELSE \"my var\" * 2e1");
        let Ok(Ast::If(c, a, b)) = e.clone() else {
            panic!("{e:?}");
        };
        assert!(matches!(*c, Ast::Bin(BinOp::Gt, _, _)));
        assert_eq!(
            *a,
            Ast::Call("min".into(), vec![Ast::Name("a".into()), Ast::Num(3.0)])
        );
        assert_eq!(
            *b,
            Ast::Bin(BinOp::Mul, Box::new(Ast::Name("my_var".into())), num(20.0))
        );
    }

    #[test]
    fn comparison_binds_tighter_than_equality_and_and_tighter_than_or() {
        let e = parse("a < b = c AND d OR e");
        let Ok(Ast::Bin(BinOp::Or, l, _)) = e.clone() else {
            panic!("{e:?}");
        };
        let Ast::Bin(BinOp::And, ll, _) = (*l).clone() else {
            panic!("{l:?}");
        };
        assert!(matches!(*ll, Ast::Bin(BinOp::Eq, _, _)));
    }

    #[test]
    fn arrays_and_garbage_are_errors() {
        assert!(parse("a[1]").is_err_and(|m| m.contains("arrays")));
        assert!(parse("1 +").is_err());
        assert!(parse("(1").is_err());
        assert!(parse("").is_err());
        assert!(parse("1 2").is_err());
    }

    /// Parentheses, signs, `NOT`, `IF`, calls, powers and long runs of one operator all nest
    /// without bound in a file, and must be refused, not overflow the stack.
    #[test]
    fn nesting_is_limited() {
        let n = 100_000;
        let deep = [
            format!("{}1{}", "(".repeat(n), ")".repeat(n)),
            format!("{}1", "-".repeat(n)),
            format!("{}1", "+".repeat(n)),
            format!("{}1", "not ".repeat(n)),
            format!("1{}", "+1".repeat(n)),
            format!("1{}", " and 1".repeat(n)),
            format!("{}2", "2^".repeat(n)),
            format!("{}1", "if 1 then 1 else ".repeat(n)),
            format!("{}1{}", "min(1, ".repeat(n), ")".repeat(n)),
        ];
        for src in &deep {
            assert!(
                parse(src).is_err_and(|m| m.contains("nests more than")),
                "{}",
                &src[..20]
            );
        }
        // Deep, but within the limit: still read.
        let ok = format!("{}1{}", "(".repeat(50), ")".repeat(50));
        assert!(parse(&ok).is_ok());
        assert!(parse(&format!("1{}", "+1".repeat(200))).is_ok());
        assert!(parse(&format!("{}1", "-".repeat(50))).is_ok());
    }
}
