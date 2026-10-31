// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Expression evaluator behind the `print` command.
//!
//! ```text
//! list    := expr (',' expr)*
//! expr    := term (('+' | '-') term)*
//! term    := unary (('*' | '/' | '%') unary)*
//! unary   := '-' unary | primary
//! primary := NUMBER | STRING | '(' expr ')' | KEY | `KEY` | CALL
//! call    := ('now' | 'today') '(' ')' | ('year' | 'month' | ... ) '(' expr ')'
//! ```
//! Strings support `${expr}` interpolation. `+` adds numbers or concatenates
//! strings; mixing the two is a type error.

use crate::time::{self, TimeFn};
use crate::value::{fmt_f64, is_valid_number, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum EVal {
    Num(f64),
    Str(String),
}

impl EVal {
    pub fn render(&self) -> String {
        match self {
            EVal::Num(f) => fmt_f64(*f),
            EVal::Str(s) => s.clone(),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            EVal::Num(_) => "int",
            EVal::Str(_) => "str",
        }
    }
}

/// Looks a key (optionally with a `.path`) up in the database.
pub type Resolver<'a> = dyn FnMut(&str) -> Result<Option<Value>, String> + 'a;

struct Parser<'a, 'r> {
    src: &'a [char],
    pos: usize,
    resolve: &'r mut Resolver<'a>,
}

/// Evaluates a comma-separated list of expressions.
pub fn eval_list<'a>(src: &'a [char], resolve: &mut Resolver<'a>) -> Result<Vec<EVal>, String> {
    let mut p = Parser { src, pos: 0, resolve };
    let mut out = Vec::new();
    p.skip_ws();
    if p.pos >= p.src.len() {
        return Ok(out);
    }
    loop {
        out.push(p.expr()?);
        p.skip_ws();
        if p.pos >= p.src.len() {
            break;
        }
        if p.src[p.pos] == ',' {
            p.pos += 1;
            p.skip_ws();
            if p.pos >= p.src.len() {
                break;
            }
        } else {
            return Err("SyntaxError: missing separator between tokens.".into());
        }
    }
    Ok(out)
}

fn is_call(word: &str) -> bool {
    matches!(word.to_ascii_lowercase().as_str(), "now" | "today") || TimeFn::parse(word).is_some()
}

/// A number for arithmetic, which is done with 64-bit floats: refused when
/// such a float would round it, rather than printing another number.
fn exact_number(text: &str) -> Result<f64, String> {
    crate::num::exact_f64(text).ok_or_else(|| {
        format!("ValueError: {text} cannot be held exactly by a 64-bit float; use get to read it as stored")
    })
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | ':' | '.' | '$' | '@' | '#' | '[' | ']')
}

impl<'a, 'r> Parser<'a, 'r> {
    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_ws();
        self.src.get(self.pos).copied()
    }

    fn expr(&mut self) -> Result<EVal, String> {
        let mut left = self.term()?;
        while let Some(op) = self.peek() {
            if op != '+' && op != '-' {
                break;
            }
            self.pos += 1;
            let right = self.term()?;
            left = match (op, left, right) {
                ('+', EVal::Num(a), EVal::Num(b)) => EVal::Num(a + b),
                ('+', EVal::Str(a), EVal::Str(b)) => EVal::Str(a + &b),
                ('+', a, b) => return Err(format!("TypeError: cannot concatenate {} and {}", a.kind(), b.kind())),
                (_, EVal::Num(a), EVal::Num(b)) => EVal::Num(a - b),
                (_, a, b) => {
                    return Err(format!("TypeError: unsupported operand types for -: {} and {}", a.kind(), b.kind()))
                }
            };
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<EVal, String> {
        let mut left = self.unary()?;
        while let Some(op) = self.peek() {
            if op != '*' && op != '/' && op != '%' {
                break;
            }
            self.pos += 1;
            let right = self.unary()?;
            let (a, b) = match (&left, &right) {
                (EVal::Num(a), EVal::Num(b)) => (*a, *b),
                (a, b) => {
                    return Err(format!("TypeError: unsupported operand types for {op}: {} and {}", a.kind(), b.kind()))
                }
            };
            left = EVal::Num(match op {
                '*' => a * b,
                _ if b == 0.0 => return Err("ZeroDivisionError: division by zero".into()),
                '/' => a / b,
                _ => a % b,
            });
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<EVal, String> {
        if self.peek() == Some('-') {
            self.pos += 1;
            return match self.unary()? {
                EVal::Num(n) => Ok(EVal::Num(-n)),
                EVal::Str(_) => Err("TypeError: bad operand type for unary -: str".into()),
            };
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<EVal, String> {
        let Some(c) = self.peek() else {
            return Err("SyntaxError: unexpected end of expression".into());
        };
        match c {
            '(' => {
                self.pos += 1;
                let v = self.expr()?;
                if self.peek() != Some(')') {
                    return Err("SyntaxError: missing closing parenthesis".into());
                }
                self.pos += 1;
                Ok(v)
            }
            '"' | '\'' | '`' => {
                let start = self.pos + 1;
                let mut end = start;
                while end < self.src.len() && self.src[end] != c {
                    end += 1;
                }
                if end >= self.src.len() {
                    return Err(format!("Syntax Error: Unclosed quote detected ({c})."));
                }
                self.pos = end + 1;
                let text: String = self.src[start..end].iter().collect();
                if c == '`' {
                    self.lookup(&text)
                } else {
                    Ok(EVal::Str(self.interpolate(start, end)?))
                }
            }
            c if is_ident_char(c) => {
                let start = self.pos;
                while self.pos < self.src.len() && is_ident_char(self.src[self.pos]) {
                    self.pos += 1;
                }
                let word: String = self.src[start..self.pos].iter().collect();
                if is_valid_number(&word) {
                    Ok(EVal::Num(exact_number(&word)?))
                } else if self.peek() == Some('(') && is_call(&word) {
                    self.call(&word)
                } else {
                    self.lookup(&word)
                }
            }
            other => Err(format!("SyntaxError: unexpected character '{other}'")),
        }
    }

    /// `now()`, `today()`, and `year(...)` to `weekday(...)` of a date, time or datetime.
    fn call(&mut self, name: &str) -> Result<EVal, String> {
        self.pos += 1;
        let lower = name.to_ascii_lowercase();
        let arg = if matches!(lower.as_str(), "now" | "today") { None } else { Some(self.expr()?) };
        if self.peek() != Some(')') {
            return Err(format!(
                "SyntaxError: {lower}() takes {}",
                if arg.is_some() { "one value" } else { "nothing" }
            ));
        }
        self.pos += 1;
        match (lower.as_str(), arg) {
            ("now", _) => Ok(EVal::Str(time::shown_instant(time::now()))),
            ("today", _) => Ok(EVal::Str(time::shown_date(time::today()))),
            (f, Some(v)) => {
                let func = TimeFn::parse(f).expect("a time function");
                let part = match &v {
                    EVal::Str(s) => func.of(&serde_json::Value::String(s.clone())),
                    EVal::Num(_) => None,
                };
                part.map(|n| EVal::Num(n as f64))
                    .ok_or_else(|| format!("TypeError: {f}() takes a date, a time or a datetime, not '{}'", v.render()))
            }
            _ => unreachable!("the functions without a value are above"),
        }
    }

    fn lookup(&mut self, name: &str) -> Result<EVal, String> {
        match (self.resolve)(name)? {
            None => Err(format!("NameError: '{name}' is not defined")),
            Some(Value::Num(n)) => Ok(EVal::Num(exact_number(&n)?)),
            Some(v) => Ok(EVal::Str(v.display())),
        }
    }

    /// Expands `${expr}` inside the string literal spanning `src[start..end]`.
    fn interpolate(&mut self, start: usize, end: usize) -> Result<String, String> {
        let src = self.src;
        let mut out = String::new();
        let mut i = start;
        while i < end {
            if src[i] == '$' && i + 1 < end && src[i + 1] == '{' {
                let mut close = i + 2;
                while close < end && src[close] != '}' {
                    close += 1;
                }
                if close >= end {
                    out.push(src[i]);
                    i += 1;
                    continue;
                }
                let mut sub = Parser { src: &src[i + 2..close], pos: 0, resolve: &mut *self.resolve };
                let v = sub.expr().map_err(|_| "EvaluationError in string interpolation.".to_string())?;
                sub.skip_ws();
                if sub.pos < sub.src.len() {
                    return Err("EvaluationError in string interpolation.".into());
                }
                out.push_str(&v.render());
                i = close + 1;
            } else {
                out.push(src[i]);
                i += 1;
            }
        }
        Ok(out)
    }
}
