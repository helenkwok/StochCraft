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

struct Parser {
    toks: Vec<Tok>,
    i: usize,
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

    fn or(&mut self) -> Result<Ast, String> {
        let mut l = self.and()?;
        while self.word("or") {
            self.bump();
            let r = self.and()?;
            l = Ast::Bin(BinOp::Or, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn and(&mut self) -> Result<Ast, String> {
        let mut l = self.equality()?;
        while self.word("and") {
            self.bump();
            let r = self.equality()?;
            l = Ast::Bin(BinOp::And, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn equality(&mut self) -> Result<Ast, String> {
        let mut l = self.relation()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Eq) => BinOp::Eq,
                Some(Tok::Ne) => BinOp::Ne,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.relation()?;
            l = Ast::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn relation(&mut self) -> Result<Ast, String> {
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
            l = Ast::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn additive(&mut self) -> Result<Ast, String> {
        let mut l = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Add,
                Some(Tok::Minus) => BinOp::Sub,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.multiplicative()?;
            l = Ast::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn multiplicative(&mut self) -> Result<Ast, String> {
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
            l = Ast::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn unary(&mut self) -> Result<Ast, String> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.bump();
                Ok(Ast::Neg(Box::new(self.unary()?)))
            }
            Some(Tok::Plus) => {
                self.bump();
                self.unary()
            }
            Some(Tok::Ident(s)) if s == "not" => {
                self.bump();
                Ok(Ast::Not(Box::new(self.unary()?)))
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Ast, String> {
        let base = self.primary()?;
        if self.peek() == Some(&Tok::Caret) {
            self.bump();
            // Right to left, and the exponent may carry a sign: 2^-3^2 is 2^(-(3^2)).
            let exp = self.unary()?;
            return Ok(Ast::Bin(BinOp::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Ast, String> {
        match self.peek().cloned() {
            Some(Tok::Num(v)) => {
                self.bump();
                Ok(Ast::Num(v))
            }
            Some(Tok::LParen) => {
                self.bump();
                let e = self.or()?;
                self.expect(&Tok::RParen, "`)`")?;
                Ok(e)
            }
            Some(Tok::Ident(s)) if s == "if" => {
                self.bump();
                let c = self.or()?;
                if !self.word("then") {
                    return Err("`THEN` was expected".to_owned());
                }
                self.bump();
                let a = self.or()?;
                if !self.word("else") {
                    return Err("`ELSE` was expected".to_owned());
                }
                self.bump();
                let b = self.or()?;
                Ok(Ast::If(Box::new(c), Box::new(a), Box::new(b)))
            }
            Some(Tok::Ident(s)) => {
                self.bump();
                if self.peek() == Some(&Tok::LParen) {
                    self.bump();
                    let mut args = Vec::new();
                    if self.peek() == Some(&Tok::RParen) {
                        self.bump();
                        return Ok(Ast::Call(s, args));
                    }
                    loop {
                        args.push(self.or()?);
                        match self.peek() {
                            Some(Tok::Comma) => self.bump(),
                            Some(Tok::RParen) => {
                                self.bump();
                                break;
                            }
                            _ => return Err("`,` or `)` was expected".to_owned()),
                        }
                    }
                    Ok(Ast::Call(s, args))
                } else {
                    Ok(Ast::Name(s))
                }
            }
            Some(_) => Err("a value was expected".to_owned()),
            None => Err("the equation ends too soon".to_owned()),
        }
    }
}

/// Reads an equation. The error says what was expected.
pub fn parse(text: &str) -> Result<Ast, String> {
    let toks = lex(text)?;
    if toks.is_empty() {
        return Err("the equation is empty".to_owned());
    }
    let mut p = Parser { toks, i: 0 };
    let e = p.or()?;
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
}
