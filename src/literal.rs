// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Typed literals of the shell: JSON in which any value, at any depth, may
//! carry a type before it (`{"balance": decimal "1250.50", "at": now()}`).
//! See `docs/architecture/TEMPORAL_FORMATS.md` 5.2 for the grammar and
//! `docs/architecture/NATIVE_DOCUMENTS.md` 19 for the rules.
//!
//! Plain JSON is never read here: [`document`] hands a literal to
//! `serde_json` first, and reads it with types only when that fails and the
//! literal holds a type word in a value position. A literal without types
//! therefore gives the same document as before, by construction.

use crate::doc::{self, Doc, Members, MAX_DEPTH};
use chrono::{DateTime, NaiveTime, Timelike, Utc};
use serde_json::Value as Json;

/// The type words a value may carry, as error messages list them.
pub const TYPE_WORDS: &str =
    "string, int, float, decimal, number, bool, null, date, datetime, time, uuid, object, array";

/// A document literal of the shell: plain JSON as `serde_json` reads it,
/// otherwise the typed syntax. `plain` is the error `serde_json` gave,
/// returned as it is when the literal holds no type word, so that a mistake
/// in plain JSON is reported as before.
pub fn document(text: &str, plain: String) -> Result<Doc, String> {
    read_document(Parser::new(text), plain)
}

/// [`document`] in a condition (`in [decimal "0.10", ...]`), where a random
/// function is refused: each row would be compared with another value.
pub fn condition_document(text: &str, plain: String) -> Result<Doc, String> {
    let mut p = Parser::new(text);
    p.in_condition = true;
    read_document(p, plain)
}

/// The message of a random function written in a condition.
pub const RANDOM_IN_CONDITION: &str = "a random function has no place in a condition: each row would be compared \
     with another value. Draw the value first and compare with it";

fn read_document(mut p: Parser, plain: String) -> Result<Doc, String> {
    let result = p.top();
    match result {
        Ok(d) if p.typed => Ok(d),
        // Not a typed literal: the JSON error stands.
        Ok(_) => Err(plain),
        Err(_) if !p.typed => Err(plain),
        Err(e) => Err(e),
    }
}

/// Whether a word is a type word of a scalar value in a command, read before
/// the token that follows it: `decimal "0.1"`, `int 25`, `uuid "..."`.
/// Dates, times and datetimes are read as moments (`exec::time_expr`).
pub fn is_scalar_type(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "int" | "float" | "decimal" | "number" | "uuid" | "string" | "bool" | "boolean"
    )
}

/// The value of a scalar type word followed by one token of a command, the
/// token's text unquoted: `decimal` with `"1250.50"` or `1250.50`.
pub fn scalar(word: &str, text: &str, quoted: bool) -> Result<Doc, String> {
    let word = word.to_ascii_lowercase();
    let bare = |what: &str| -> Result<(), String> {
        if quoted {
            Err(format!("{word} takes {what} without quotes: {word} {text}"))
        } else {
            Ok(())
        }
    };
    let text_only = |what: &str| -> Result<(), String> {
        if quoted {
            Ok(())
        } else {
            Err(format!("{word} takes {what} in quotes: {word} \"{text}\""))
        }
    };
    match word.as_str() {
        "int" => {
            bare("an integer")?;
            int(text)
        }
        "float" => {
            bare("a number")?;
            float(text)
        }
        "number" => {
            bare("a number")?;
            untyped_number(text)
        }
        "decimal" => Ok(Doc::Decimal(doc::parse_decimal(text)?)),
        "uuid" => {
            text_only("a uuid")?;
            uuid(text).map(Doc::Str)
        }
        "string" => {
            text_only("a text")?;
            Ok(Doc::Str(text.to_string()))
        }
        "bool" | "boolean" => {
            bare("true or false")?;
            match text {
                "true" => Ok(Doc::Bool(true)),
                "false" => Ok(Doc::Bool(false)),
                _ => Err(format!("'{text}' is not a boolean: write true or false")),
            }
        }
        _ => Err(format!("'{word}' is not a type: use {TYPE_WORDS}")),
    }
}

/// An integer of `int`: no fraction, no exponent, within 64 bits.
fn int(text: &str) -> Result<Doc, String> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return Err(format!("'{text}' is not an int: an int has no fraction and no exponent (int 25)"));
    }
    text.parse::<i64>().map(Doc::Int).map_err(|_| {
        format!(
            "{text} does not fit an int (-9223372036854775808 to 9223372036854775807): write it as decimal \"{text}\""
        )
    })
}

/// A number of `float`, finite.
fn float(text: &str) -> Result<Doc, String> {
    if !is_json_number(text) {
        return Err(format!("'{text}' is not a number"));
    }
    match text.parse::<f64>() {
        Ok(f) if f.is_finite() => Ok(Doc::Float(f)),
        _ => Err(format!("{text} is beyond the range of a float")),
    }
}

/// A number without a type: an integer within 64 bits is an `int` (a `u64`
/// as 2.0 kept it), anything else a `float`, as plain JSON reads it.
fn untyped_number(text: &str) -> Result<Doc, String> {
    if !is_json_number(text) {
        return Err(format!("'{text}' is not a number"));
    }
    crate::num::check_integers(text.as_bytes())?;
    let j: Json = serde_json::from_str(text).map_err(|_| format!("{text} is beyond the range of a float"))?;
    Ok(doc::from_json(&j))
}

/// The number syntax of JSON: `-`, an integer without a leading zero, an
/// optional fraction and exponent.
fn is_json_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = usize::from(b.first() == Some(&b'-'));
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - start
    };
    let whole_start = i;
    let n = digits(&mut i);
    if n == 0 || (n > 1 && b[whole_start] == b'0') {
        return false;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if digits(&mut i) == 0 {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == b.len()
}

/// A uuid in its canonical form: 32 hexadecimal digits in groups of 8, 4,
/// 4, 4 and 12, in small letters. Read with the four dashes in their places
/// or without any; braces, `urn:uuid:` and other forms are refused.
pub fn uuid(text: &str) -> Result<String, String> {
    let bad =
        || format!("'{text}' is not a uuid: write 32 hexadecimal digits, as 550e8400-e29b-41d4-a716-446655440000");
    let hex: String = match text.len() {
        32 => text.to_string(),
        36 => {
            let b = text.as_bytes();
            if [8, 13, 18, 23].iter().any(|&i| b[i] != b'-') {
                return Err(bad());
            }
            text.chars().filter(|&c| c != '-').collect()
        }
        _ => return Err(bad()),
    };
    if hex.len() != 32 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let h = hex.to_ascii_lowercase();
    Ok(format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]))
}

/// A datetime kept to the millisecond, as every datetime is.
fn to_ms(at: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(at.timestamp_millis()).unwrap_or(at)
}

/// A time kept to the millisecond.
fn time_ms(t: NaiveTime) -> NaiveTime {
    t.with_nanosecond(t.nanosecond() / 1_000_000 * 1_000_000).unwrap_or(t)
}

/// A step of the path to the value being read, for error messages.
enum Seg {
    Key(String),
    Index(usize),
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
    /// A type word or a moment was met in a value position.
    typed: bool,
    /// Read for a condition: random functions are refused.
    in_condition: bool,
    path: Vec<Seg>,
}

type R<T> = Result<T, String>;

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser { s, b: s.as_bytes(), i: 0, typed: false, in_condition: false, path: Vec::new() }
    }

    /// `line 3, column 18 (.values[3]): <msg>`, at byte `at`.
    fn err_at(&self, at: usize, msg: impl std::fmt::Display) -> String {
        let before = &self.s[..at.min(self.s.len())];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        let mut path = String::new();
        for seg in &self.path {
            match seg {
                Seg::Key(k) if !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_') => {
                    path.push('.');
                    path.push_str(k);
                }
                Seg::Key(k) => path.push_str(&format!("[{}]", Json::String(k.clone()))),
                Seg::Index(n) => path.push_str(&format!("[{n}]")),
            }
        }
        if path.is_empty() {
            path.push('.');
        }
        format!("line {line}, column {column} ({path}): {msg}")
    }

    fn err(&self, msg: impl std::fmt::Display) -> String {
        self.err_at(self.i, msg)
    }

    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    /// At least one space after a type word.
    fn ws1(&mut self, word: &str) -> R<()> {
        let start = self.i;
        self.ws();
        if self.i == start {
            return Err(self.err(format!("a space is needed after '{word}'")));
        }
        Ok(())
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn top(&mut self) -> R<Doc> {
        self.ws();
        let d = match self.peek() {
            Some(b'{') => self.object(1)?,
            Some(b'[') => self.array(1)?,
            _ => return Err(self.err("a document starts with { or [")),
        };
        self.ws();
        if self.i < self.b.len() {
            return Err(self.err("unexpected text after the document"));
        }
        Ok(d)
    }

    fn object(&mut self, depth: usize) -> R<Doc> {
        if depth > MAX_DEPTH {
            return Err(self.err(format!("a document nested deeper than {MAX_DEPTH}")));
        }
        self.i += 1; // {
        let mut m = Members::new();
        self.ws();
        if self.eat(b'}') {
            return Ok(Doc::Object(m));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("a key is a text in double quotes"));
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(b':') {
                return Err(self.err("':' expected after a key"));
            }
            self.path.push(Seg::Key(key.clone()));
            let v = self.value(depth)?;
            self.path.pop();
            // A key given twice keeps the last value, in the place of the first.
            m.insert(key, v);
            self.ws();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b'}') {
                return Ok(Doc::Object(m));
            }
            return Err(self.err("',' or '}' expected"));
        }
    }

    fn array(&mut self, depth: usize) -> R<Doc> {
        if depth > MAX_DEPTH {
            return Err(self.err(format!("a document nested deeper than {MAX_DEPTH}")));
        }
        self.i += 1; // [
        let mut items = Vec::new();
        self.ws();
        if self.eat(b']') {
            return Ok(Doc::Array(items));
        }
        loop {
            self.path.push(Seg::Index(items.len()));
            let v = self.value(depth)?;
            self.path.pop();
            items.push(v);
            self.ws();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b']') {
                return Ok(Doc::Array(items));
            }
            return Err(self.err("',' or ']' expected"));
        }
    }

    /// A JSON string, with its escapes, read by `serde_json`.
    fn string(&mut self) -> R<String> {
        let start = self.i;
        self.i += 1;
        while self.i < self.b.len() && self.b[self.i] != b'"' {
            self.i += if self.b[self.i] == b'\\' { 2 } else { 1 };
        }
        if self.i >= self.b.len() {
            return Err(self.err_at(start, "a text is not closed by \""));
        }
        self.i += 1;
        serde_json::from_str(&self.s[start..self.i]).map_err(|e| self.err_at(start, format!("not a valid text: {e}")))
    }

    /// The text of a JSON number at the cursor.
    fn number_text(&mut self) -> &'a str {
        let start = self.i;
        while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
            self.i += 1;
        }
        &self.s[start..self.i]
    }

    fn word(&mut self) -> &'a str {
        let start = self.i;
        while self.i < self.b.len() && (self.b[self.i].is_ascii_alphanumeric() || self.b[self.i] == b'_') {
            self.i += 1;
        }
        &self.s[start..self.i]
    }

    /// A value without a type, as JSON writes it.
    fn plain(&mut self, depth: usize) -> R<Doc> {
        let start = self.i;
        match self.peek() {
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => self.string().map(Doc::Str),
            Some(b'-' | b'0'..=b'9') => {
                let t = self.number_text();
                untyped_number(t).map_err(|e| self.err_at(start, e))
            }
            _ => {
                let w = self.word();
                match w {
                    "true" => Ok(Doc::Bool(true)),
                    "false" => Ok(Doc::Bool(false)),
                    "null" => Ok(Doc::Null),
                    "" => Err(self.err_at(start, "a value is expected")),
                    w => Err(self.err_at(start, format!("'{w}' is not a value: put a text in double quotes"))),
                }
            }
        }
    }

    fn value(&mut self, depth: usize) -> R<Doc> {
        self.ws();
        if !self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            return self.plain(depth);
        }
        let start = self.i;
        let w = self.word();
        if w == "null" {
            // `null null` is the type word and its value; `null` alone a value.
            let after = self.i;
            self.ws();
            if self.i > after && self.s[self.i..].starts_with("null") {
                self.typed = true;
                self.i += 4;
                return Ok(Doc::Null);
            }
            self.i = after;
            return Ok(Doc::Null);
        }
        if matches!(w, "true" | "false") {
            self.i = start;
            return self.plain(depth);
        }
        let word = w.to_ascii_lowercase();
        // `uuid()`, `random_int(1, 6)`: a call, where `uuid "..."` is a type word.
        if crate::random::is_name(&word) && self.paren_follows() {
            self.typed = true;
            if self.in_condition {
                return Err(self.err_at(start, RANDOM_IN_CONDITION));
            }
            if crate::random::is_array_name(&word) {
                return self.array_call(&word, start, depth);
            }
            return self.random(&word, start);
        }
        match word.as_str() {
            "now" | "today" | "current_timestamp" | "current_date" | "current_time" => {
                self.typed = true;
                self.moment(&word, start)
            }
            "string" | "int" | "float" | "decimal" | "number" | "bool" | "boolean" | "date" | "datetime" | "time"
            | "uuid" | "object" | "array" => {
                self.typed = true;
                self.ws1(&word)?;
                self.typed_value(&word, depth)
            }
            _ => Err(self.err_at(start, format!("'{w}' is not a type: use {TYPE_WORDS}, or put a text in quotes"))),
        }
    }

    fn typed_value(&mut self, word: &str, depth: usize) -> R<Doc> {
        let at = self.i;
        let quoted = self.peek() == Some(b'"');
        match word {
            "object" | "array" => {
                let want = if word == "object" { b'{' } else { b'[' };
                if self.peek() != Some(want) {
                    return Err(self.err(format!("{word} takes {}", if want == b'{' { "{...}" } else { "[...]" })));
                }
                self.plain(depth)
            }
            "date" | "datetime" | "time" => {
                if !quoted {
                    return Err(self.err(format!("{word} takes a text in quotes: {word} \"...\"")));
                }
                let text = self.string()?;
                let d = match word {
                    "date" => crate::time::read_date(&text).map(Doc::Date),
                    "time" => crate::time::read_time(&text).map(|t| Doc::Time(time_ms(t))),
                    _ => crate::time::read_datetime(&text, &crate::time::zone()).map(|t| Doc::DateTime(to_ms(t))),
                };
                d.map_err(|e| self.err_at(at, e))
            }
            _ => {
                let text = if quoted {
                    self.string()?
                } else if self.peek().is_some_and(|c| c == b'-' || c.is_ascii_digit()) {
                    self.number_text().to_string()
                } else {
                    self.word().to_string()
                };
                scalar(word, &text, quoted).map_err(|e| self.err_at(at, e))
            }
        }
    }

    /// Whether `(` comes next, spaces aside; nothing is read.
    fn paren_follows(&mut self) -> bool {
        let save = self.i;
        self.ws();
        let found = self.peek() == Some(b'(');
        self.i = save;
        found
    }

    /// `uuid()`, `random_int(1, 6)`, `random_string(16)`: drawn once, here,
    /// and kept as the value it gave (RANDOM_FUNCTIONS.md 12.7).
    fn random(&mut self, word: &str, start: usize) -> R<Doc> {
        self.ws();
        self.eat(b'(');
        let mut args = Vec::new();
        self.ws();
        if !self.eat(b')') {
            loop {
                self.ws();
                let at = self.i;
                while self.i < self.b.len() && !matches!(self.b[self.i], b',' | b')' | b' ' | b'\t' | b'\n' | b'\r') {
                    self.i += 1;
                }
                if self.i == at {
                    return Err(self.err(format!("{word}: an argument is expected")));
                }
                args.push(self.s[at..self.i].to_string());
                self.ws();
                if self.eat(b')') {
                    break;
                }
                if !self.eat(b',') {
                    return Err(self.err(format!("{word}: ',' or ')' is expected")));
                }
            }
        }
        let call = crate::random::Gen::parse(word, &args).map_err(|e| self.err_at(start, e))?;
        let drawn = call.draw().map_err(|e| e.to_string())?;
        Ok(drawn_doc(drawn))
    }

    /// `shuffle([...])`, `sample([...])`, `sample([...], 2)`: the array is
    /// read as any value, typed values and functions included, then shuffled
    /// or sampled once, here (RANDOM_FUNCTIONS.md 12.5).
    fn array_call(&mut self, word: &str, start: usize, depth: usize) -> R<Doc> {
        self.ws();
        self.eat(b'(');
        let arg_at = {
            self.ws();
            self.i
        };
        let items = match self.value(depth + 1)? {
            Doc::Array(items) => items,
            other => return Err(self.err_at(arg_at, format!("{word} takes an array, not {}", other.type_name()))),
        };
        self.ws();
        let count = if self.eat(b',') {
            self.ws();
            let at = self.i;
            while self.i < self.b.len() && !matches!(self.b[self.i], b')' | b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            }
            self.ws();
            Some(self.s[at..self.i].trim().to_string())
        } else {
            None
        };
        if !self.eat(b')') {
            return Err(self.err(format!("{word}: ')' is expected")));
        }
        let call = crate::random::ArrayFn::parse(word, count.as_deref()).map_err(|e| self.err_at(start, e))?;
        array_result(call, items).map_err(|e| self.err_at(start, e))
    }

    /// `now()`, `today()`, `current_timestamp`... then `+ 30 days`, `- 2 hours`.
    fn moment(&mut self, word: &str, start: usize) -> R<Doc> {
        let call = {
            let save = self.i;
            self.ws();
            if self.eat(b'(') {
                self.ws();
                if self.eat(b')') {
                    true
                } else {
                    return Err(self.err(format!("{word}() takes nothing between its parentheses")));
                }
            } else {
                self.i = save;
                false
            }
        };
        let mut d = match word {
            "now" | "today" if !call => return Err(self.err_at(start, format!("write {word}()"))),
            "now" | "current_timestamp" => Doc::DateTime(crate::time::now()),
            "today" | "current_date" => Doc::Date(crate::time::today()),
            _ => Doc::Time(time_ms(crate::time::current_time())),
        };
        loop {
            let save = self.i;
            self.ws();
            let sign = match self.peek() {
                Some(b'+') => 1,
                Some(b'-') => -1,
                _ => {
                    self.i = save;
                    return Ok(d);
                }
            };
            self.i += 1;
            self.ws();
            let at = self.i;
            let n = self.word();
            let n: i64 =
                n.parse().map_err(|_| self.err_at(at, format!("'{n}' is not a whole number: now() - 7 days")))?;
            self.ws1(&n.to_string())?;
            let unit_at = self.i;
            let unit = self.word();
            let step = crate::time::duration(sign * n, unit).ok_or_else(|| {
                self.err_at(unit_at, format!("'{unit}' is not a unit: use ms, seconds, minutes, hours, days or weeks"))
            })?;
            d = match d {
                Doc::DateTime(t) => Doc::DateTime(t.checked_add_signed(step).ok_or_else(|| self.err("out of range"))?),
                Doc::Date(day) => {
                    if step.num_milliseconds() % 86_400_000 != 0 {
                        return Err(self.err("a date moves by days or weeks; for hours, use now()"));
                    }
                    Doc::Date(day.checked_add_signed(step).ok_or_else(|| self.err("out of range"))?)
                }
                Doc::Time(t) => Doc::Time(t.overflowing_add_signed(step).0),
                other => other,
            };
        }
    }
}

/// `shuffle`, `sample` applied to the elements of an array: an array, or one element.
pub fn array_result(call: crate::random::ArrayFn, items: Vec<Doc>) -> Result<Doc, String> {
    Ok(match call.apply(items, &mut crate::random::Source::new())? {
        crate::random::Picked::One(d) => d,
        crate::random::Picked::Many(v) => Doc::Array(v),
    })
}

/// A drawn value as a value of a document.
pub fn drawn_doc(d: crate::random::Drawn) -> Doc {
    match d {
        crate::random::Drawn::Float(f) => Doc::Float(f),
        crate::random::Drawn::Int(i) => Doc::Int(i),
        crate::random::Drawn::Text(s) => Doc::Str(s),
    }
}

/// A document as the shell shows it: typed values in the typed literal
/// syntax, dates and times in the session's formats, so that what is shown
/// can be typed back. JSON values are shown as JSON.
pub fn show(d: &Doc) -> String {
    let mut out = String::new();
    show_into(d, &mut out);
    out
}

fn show_into(d: &Doc, out: &mut String) {
    use crate::time::{shown_date, shown_instant, shown_time};
    let quoted = |s: String| Json::String(s).to_string();
    match d {
        Doc::Decimal(x) => out.push_str(&format!("decimal \"{x}\"")),
        Doc::Date(x) => out.push_str(&format!("date {}", quoted(shown_date(*x)))),
        Doc::DateTime(x) => out.push_str(&format!("datetime {}", quoted(shown_instant(*x)))),
        Doc::Time(x) => out.push_str(&format!("time {}", quoted(shown_time(*x)))),
        Doc::Float(_) => out.push_str(&doc::to_extended_json(d).to_string()),
        Doc::Array(items) => {
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                show_into(v, out);
            }
            out.push(']');
        }
        Doc::Object(m) => {
            out.push('{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&quoted(k.clone()));
                out.push_str(": ");
                show_into(v, out);
            }
            out.push('}');
        }
        other => out.push_str(&doc::to_plain_json(other).to_string()),
    }
}

/// The value of a condition for a typed value: a decimal as
/// `{"$decimal": "0.10"}`, a datetime and a time as the moments of
/// conditions, a date as its ISO text (as 2.0 compares dates), other
/// values as JSON.
pub fn condition_json(d: &Doc) -> Json {
    match d {
        Doc::Decimal(x) => serde_json::json!({ "$decimal": x.to_string() }),
        Doc::DateTime(t) => crate::time::datetime_json(*t),
        Doc::Time(t) => crate::time::time_json(*t),
        Doc::Date(day) => Json::String(crate::time::date_text(*day)),
        Doc::Array(items) => Json::Array(items.iter().map(condition_json).collect()),
        other => doc::to_plain_json(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(s: &str) -> Result<Doc, String> {
        let plain = match serde_json::from_str::<Json>(s) {
            Ok(_) => return Err("plain JSON".into()),
            Err(e) => format!("Invalid JSON literal: {e}"),
        };
        document(s, plain)
    }

    fn ext(s: &str) -> Json {
        doc::to_extended_json(&read(s).unwrap())
    }

    #[test]
    fn typed_values_at_any_depth() {
        let j = ext(r#"{
  "id": uuid "550E8400E29B41D4A716446655440000",
  "age": int 25,
  "balance": decimal "1250.50",
  "created_at": datetime "2026-10-31T16:18:55Z",
  "values": [int 8, float 0.5, decimal "0.10", date "2026-10-31"],
  "owner": {"since": date "2020-01-15", "limits": [decimal 500, 7, "x", null, true]}
}"#);
        assert_eq!(
            j,
            serde_json::json!({
                "id": "550e8400-e29b-41d4-a716-446655440000",
                "age": 25,
                "balance": {"$decimal": "1250.50"},
                "created_at": {"$datetime": "2026-10-31T16:18:55.000Z"},
                "values": [8, 0.5, {"$decimal": "0.10"}, {"$date": "2026-10-31"}],
                "owner": {"since": {"$date": "2020-01-15"}, "limits": [{"$decimal": "500"}, 7, "x", null, true]}
            })
        );
        assert!(matches!(read("[float 8]").unwrap(), Doc::Array(v) if matches!(v[0], Doc::Float(f) if f == 8.0)));
        assert!(
            matches!(read("[int 8, number 9]").unwrap(), Doc::Array(v) if matches!(v[..], [Doc::Int(8), Doc::Int(9)]))
        );
        assert_eq!(
            ext(r#"[string "a", bool true, null null, object {}, array [1]]"#),
            serde_json::json!(["a", true, null, {}, [1]])
        );
    }

    #[test]
    fn errors_name_line_column_and_path() {
        let e = read("{\n  \"a\": 1,\n  \"values\": [int 8, int 1.5]\n}").unwrap_err();
        assert!(e.starts_with("line 3, column 25 (.values[1]):"), "{e}");
        assert!(e.contains("not an int"), "{e}");
        let e = read(r#"{"d": date "31-02-2026"}"#).unwrap_err();
        assert!(e.contains("(.d)") && e.contains("not a date"), "{e}");
        let e = read(r#"{"p": decimal "0.12345678901234567890123456789"}"#).unwrap_err();
        assert!(e.contains("does not fit a decimal"), "{e}");
        let e = read(r#"{"n": int 9223372036854775808}"#).unwrap_err();
        assert!(e.contains("decimal \"9223372036854775808\""), "{e}");
        let e = read(r#"{"u": uuid "{550e8400-e29b-41d4-a716-446655440000}"}"#).unwrap_err();
        assert!(e.contains("not a uuid"), "{e}");
        let e = read(r#"[int 8, datetime created_at "2026-10-31"]"#).unwrap_err();
        assert!(e.contains("[1]"), "{e}");
        let e = read(r#"{"f": float 1e400}"#).unwrap_err();
        assert!(e.contains("beyond the range"), "{e}");
        let e = read(r#"{"a": decimal"1"}"#).unwrap_err();
        assert!(e.contains("a space is needed"), "{e}");
    }

    #[test]
    fn plain_mistakes_keep_the_json_error() {
        // No type word: the error of serde_json, unchanged.
        assert_eq!(read(r#"{"a": hello}"#).unwrap_err().split(':').next(), Some("Invalid JSON literal"));
        assert_eq!(read(r#"{"a": 1,}"#).unwrap_err().split(':').next(), Some("Invalid JSON literal"));
        // A type word inside a text or a key is text.
        assert_eq!(
            read(r#"{"int 8": "decimal 1", "x": }"#).unwrap_err().split(':').next(),
            Some("Invalid JSON literal")
        );
    }

    #[test]
    fn moments_and_duplicates() {
        let d = read(r#"{"due": today() + 30 days, "at": now() - 2 hours, "t": current_time}"#).unwrap();
        let Doc::Object(m) = d else { panic!() };
        assert!(matches!(m["due"], Doc::Date(_)));
        assert!(matches!(m["at"], Doc::DateTime(_)));
        assert!(matches!(m["t"], Doc::Time(_)));
        let Doc::Object(m) = read(r#"{"a": int 1, "b": 2, "a": int 3}"#).unwrap() else { panic!() };
        assert_eq!(m.keys().collect::<Vec<_>>(), ["a", "b"]);
        assert!(matches!(m["a"], Doc::Int(3)));
    }

    #[test]
    fn depth_is_limited() {
        let ok = format!("{}int 1{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(read(&ok).is_ok());
        let deep = format!("{}int 1{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        // Refused before the type word is reached: the limit of serde_json.
        let e = read(&deep).unwrap_err();
        assert!(e.contains("deeper than") || e.contains("recursion limit"), "{e}");
        let deep_typed =
            format!("{{\"a\": int 1, \"b\": {}1{}}}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert!(read(&deep_typed).unwrap_err().contains("deeper than"));
    }

    #[test]
    fn uuids_are_canonical() {
        assert_eq!(uuid("550E8400-E29B-41D4-A716-446655440000").unwrap(), "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(uuid("550e8400e29b41d4a716446655440000").unwrap(), "550e8400-e29b-41d4-a716-446655440000");
        for bad in [
            "550e8400-e29b-41d4-a716-44665544000",
            "550e8400e29b-41d4-a716-4466554400001",
            "urn:uuid:550e8400",
            "g50e8400e29b41d4a716446655440000",
        ] {
            assert!(uuid(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn scalars_of_commands() {
        assert!(matches!(scalar("decimal", "-100.25", false).unwrap(), Doc::Decimal(d) if d.to_string() == "-100.25"));
        assert!(matches!(scalar("decimal", "0.10", true).unwrap(), Doc::Decimal(d) if d.to_string() == "0.10"));
        assert!(matches!(scalar("int", "3", false).unwrap(), Doc::Int(3)));
        assert!(matches!(scalar("float", "0.1", false).unwrap(), Doc::Float(f) if f == 0.1));
        assert!(scalar("int", "3", true).is_err());
        assert!(scalar("uuid", "550e8400e29b41d4a716446655440000", false).is_err());
    }

    #[test]
    fn shown_as_typed_back() {
        let d = read(r#"{"b": decimal "1250.50", "f": float 8, "d": date "2026-10-31", "s": "x"}"#).unwrap();
        let shown = show(&d);
        assert_eq!(shown, r#"{"b": decimal "1250.50", "f": 8.0, "d": date "2026-10-31", "s": "x"}"#);
        assert_eq!(doc::to_extended_json(&read(&shown).unwrap()), doc::to_extended_json(&d));
    }
}
