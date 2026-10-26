// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The command interpreter: turns one line of shell syntax into operations
//! on a `Session` and collects the text to show.

use crate::engine::{BatchOp, CmpOp, Cond};
use crate::error::KivaError;
use crate::expr;
use crate::format::FLAG_COMP_MASK;
use crate::jsonpath;
use crate::lexer::{split_by_and, tokenize, Kind, Token};
use crate::query::{self, Agg, Aggregation, Expr, Func, Query};
use crate::session::{Need, Session};
use crate::util::{b64_decode, deadline_in, fmt_time, hex_decode, human_size, now_ms, secs_left};
use crate::value::{fmt_f64, is_valid_number, Type, Value};
use serde_json::Value as Json;
use std::sync::atomic::Ordering;
use std::time::Instant;

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    #[default]
    None,
    Exit,
    Clear,
}

/// Everything a command produced. Each line is flagged as error or not.
#[derive(Default, Debug)]
pub struct Out {
    pub lines: Vec<(bool, String)>,
    pub action: Action,
}

impl Out {
    pub fn say(&mut self, s: impl Into<String>) {
        for l in s.into().split('\n') {
            self.lines.push((false, l.to_string()));
        }
    }

    pub fn err(&mut self, s: impl Into<String>) {
        for l in s.into().split('\n') {
            self.lines.push((true, l.to_string()));
        }
    }

    pub fn has_err(&self) -> bool {
        self.lines.iter().any(|(e, _)| *e)
    }

    pub fn text(&self) -> String {
        self.lines.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n")
    }
}

impl From<KivaError> for String {
    fn from(e: KivaError) -> String {
        e.to_string()
    }
}

pub(crate) type R = Result<(), String>;

const RESERVED: &[&str] = &[
    "set", "get", "update", "change", "del", "typeof", "scan", "stats", "compact", "exit", "quit", "clear", "help",
    "has", "print", "bump", "keys", "range", "string", "number", "boolean", "null", "array", "object", "bytes", "and",
    "ttl", "to", "all", "limit", "reverse", "where", "return", "from", "true", "false", "similar",
];

pub fn is_reserved(s: &str) -> bool {
    RESERVED.iter().any(|k| k.eq_ignore_ascii_case(s))
}

/// Rules every key name must satisfy, whatever front end it comes from.
pub fn validate_key_name(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("InvalidKeyName: key cannot be empty.".into());
    }
    if key.len() > crate::format::MAX_KEY_LEN {
        return Err(format!("InvalidKeyName: keys are limited to {} bytes.", crate::format::MAX_KEY_LEN));
    }
    if key.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("InvalidKeyName: '{key}' cannot be purely numeric."));
    }
    if let Some(c) = key.chars().find(|c| c.is_whitespace() || c.is_control() || "\"'`,()[]{}*?".contains(*c)) {
        return Err(format!("InvalidKeyName: '{key}' contains the forbidden character '{}'.", c.escape_default()));
    }
    Ok(())
}

pub(crate) fn type_kw(t: &Token) -> Option<Type> {
    if t.kind != Kind::Bare {
        return None;
    }
    Type::from_name(&t.text.to_ascii_lowercase())
}

/// A key being created or renamed: bare text or backticks only.
pub(crate) fn key_for_write(t: &Token) -> Result<String, String> {
    match t.kind {
        Kind::DQuote | Kind::SQuote => {
            return Err(format!("Key '{}' cannot use quotes. Use bare text or backticks (``).", t.text))
        }
        Kind::Bare if is_reserved(&t.text) => {
            return Err(format!("'{}' is a reserved keyword and cannot be used as a key.", t.text))
        }
        Kind::Bare | Kind::Backtick => {}
        _ => return Err(format!("'{}' is not a valid key name.", t.text)),
    }
    validate_key_name(&t.text)?;
    Ok(t.text.clone())
}

/// A key being looked up.
pub(crate) fn key_for_read(t: &Token) -> Result<String, String> {
    match t.kind {
        Kind::DQuote | Kind::SQuote => Err(format!("Key '{}' is quoted. Keys must be bare text.", t.text)),
        Kind::Bare | Kind::Backtick => Ok(t.text.clone()),
        _ => Err(format!("'{}' is not a valid key name.", t.text)),
    }
}

/// A plain word or quoted string used as a name, pattern or bound.
pub(crate) fn word(t: Option<&Token>, what: &str) -> Result<String, String> {
    match t {
        Some(t) if matches!(t.kind, Kind::Bare | Kind::DQuote | Kind::SQuote | Kind::Backtick) => Ok(t.text.clone()),
        Some(t) => Err(format!("unexpected '{}' where {what} was expected", t.text)),
        None => Err(format!("missing {what}")),
    }
}

pub(crate) fn uint(t: Option<&Token>, what: &str) -> Result<u64, String> {
    match t {
        Some(t) if t.is_bare() => {
            t.text.parse::<u64>().map_err(|_| format!("ValueError: {what} must be a non-negative integer."))
        }
        _ => Err(format!("ValueError: {what} must be a non-negative integer.")),
    }
}

/// Removes a trailing clause such as `if not exists` (any case) and tells
/// whether it was there. Quoted words are values, never a clause.
fn strip_clause<'a>(a: &'a [Token], words: &[&str]) -> (&'a [Token], bool) {
    match a.len().checked_sub(words.len()) {
        Some(n) if a[n..].iter().zip(words).all(|(t, w)| t.is_kw(w)) => (&a[..n], true),
        _ => (a, false),
    }
}

fn parse_bytes(text: &str) -> Option<Vec<u8>> {
    if let Some(h) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return hex_decode(h);
    }
    text.strip_prefix("b64:").and_then(b64_decode)
}

fn parse_doc(text: &str, want: Option<Type>) -> Result<Value, String> {
    let j: Json = serde_json::from_str(text).map_err(|e| format!("Invalid JSON literal: {e}"))?;
    match (&j, want) {
        (Json::Array(_), None | Some(Type::Array)) | (Json::Object(_), None | Some(Type::Object)) => Ok(Value::Doc(j)),
        (_, Some(t)) => Err(format!("Value is not a JSON {}.", t.name())),
        _ => Err("Value is not a JSON array or object.".into()),
    }
}

/// Reads a value literal. Without a forced type the type is inferred from
/// the syntax: quotes make a string, `[...]`/`{...}` a document, and bare
/// text must be a number, boolean, `null` or bytes (`0x..` / `b64:..`).
pub(crate) fn parse_value(t: &Token, forced: Option<Type>, key: &str) -> Result<Value, String> {
    if t.kind == Kind::Backtick {
        return Err(format!("Value for '{key}' cannot use backticks."));
    }
    if matches!(t.kind, Kind::Comma | Kind::LParen | Kind::RParen) {
        return Err(format!("Unexpected '{}' where a value was expected.", t.text));
    }
    let quoted = t.is_quoted();
    let bare = t.kind == Kind::Bare;
    match forced {
        None => {
            if quoted {
                return Ok(Value::Str(t.text.clone()));
            }
            if t.kind == Kind::Json {
                return parse_doc(&t.text, None);
            }
            match t.text.as_str() {
                "true" => return Ok(Value::Bool(true)),
                "false" => return Ok(Value::Bool(false)),
                "null" => return Ok(Value::Null),
                _ => {}
            }
            if is_valid_number(&t.text) {
                return Ok(Value::Num(t.text.clone()));
            }
            if let Some(b) = parse_bytes(&t.text) {
                return Ok(Value::Bytes(b));
            }
            Err(format!("String values like '{}' must be quoted (\"\" or '').", t.text))
        }
        Some(Type::String) => {
            if !quoted {
                return Err("Explicit 'string' type requires quotes \"\" or ''.".into());
            }
            Ok(Value::Str(t.text.clone()))
        }
        Some(Type::Number) => {
            if !bare {
                return Err("Numbers and Booleans must not be quoted.".into());
            }
            if !is_valid_number(&t.text) {
                return Err(format!("'{}' is not a valid number.", t.text));
            }
            Ok(Value::Num(t.text.clone()))
        }
        Some(Type::Boolean) => {
            if !bare {
                return Err("Numbers and Booleans must not be quoted.".into());
            }
            match t.text.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                other => Err(format!("'{other}' is not a valid boolean (use true or false).")),
            }
        }
        Some(Type::Null) => {
            if bare && t.text == "null" {
                Ok(Value::Null)
            } else {
                Err("The only value of type 'null' is null.".into())
            }
        }
        Some(ty @ (Type::Array | Type::Object)) => {
            if t.kind == Kind::Json || quoted {
                parse_doc(&t.text, Some(ty))
            } else {
                Err(format!("Value is not a JSON {}.", ty.name()))
            }
        }
        Some(Type::Bytes) => {
            if quoted {
                return Ok(Value::Bytes(t.text.as_bytes().to_vec()));
            }
            parse_bytes(&t.text)
                .map(Value::Bytes)
                .ok_or_else(|| "Bytes must be written as 0x<hex>, b64:<base64> or a quoted string.".into())
        }
    }
}

pub(crate) struct Cur<'a> {
    pub t: &'a [Token],
    pub i: usize,
}

impl<'a> Cur<'a> {
    pub fn new(t: &'a [Token]) -> Cur<'a> {
        Cur { t, i: 0 }
    }

    pub fn peek(&self) -> Option<&'a Token> {
        self.t.get(self.i)
    }

    pub fn next(&mut self) -> Option<&'a Token> {
        let t = self.t.get(self.i);
        if t.is_some() {
            self.i += 1;
        }
        t
    }

    pub fn eat(&mut self, kw: &str) -> bool {
        if self.peek().is_some_and(|t| t.is_kw(kw)) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    pub fn rest(&self) -> &'a [Token] {
        &self.t[self.i.min(self.t.len())..]
    }

    /// Fails if tokens are left over.
    pub fn finish(&self, usage: &str) -> R {
        match self.peek() {
            Some(t) => Err(format!("Unexpected '{}'.\nUsage: {usage}", t.text)),
            None => Ok(()),
        }
    }
}

/// `[reverse] [limit N] [return f1, f2]` in any order.
#[derive(Default)]
struct Tail {
    limit: usize,
    reverse: bool,
    fields: Option<Vec<String>>,
}

fn parse_tail(c: &mut Cur) -> Result<Tail, String> {
    let mut tail = Tail::default();
    loop {
        if c.eat("limit") {
            tail.limit = uint(c.next(), "limit")? as usize;
            if tail.limit == 0 {
                return Err("ValueError: limit must be greater than zero.".into());
            }
        } else if c.eat("reverse") {
            tail.reverse = true;
        } else if c.eat("return") {
            let mut fields = Vec::new();
            while let Some(t) = c.peek() {
                if t.kind == Kind::Comma {
                    c.next();
                } else if t.is_kw("limit") || t.is_kw("reverse") {
                    break;
                } else {
                    fields.push(word(c.next(), "a field name")?);
                }
            }
            if fields.is_empty() {
                return Err("'return' needs at least one field name.".into());
            }
            tail.fields = Some(fields);
        } else {
            break;
        }
    }
    Ok(tail)
}

fn aggregate_usage(func: Func) -> String {
    let verb = func.name();
    let target = if func == Func::Count { "<pattern>[.<field>]" } else { "<pattern>.<field>" };
    format!("{verb} where <condition> | {verb} {target} [where <condition>] [group by <field>] [limit N]")
}

/// What an aggregate reads: `where <condition>` (the field is then the one
/// the condition tests), or `<pattern>[.<field>] [where <condition>]`, then
/// `group by <field>` and `limit N` in any order.
fn parse_aggregate(c: &mut Cur, func: Func) -> Result<(Query, Option<String>, Option<String>, usize), String> {
    let verb = func.name();
    let usage = || format!("Usage: {}", aggregate_usage(func));
    let (q, field) = if c.eat("where") {
        let q = parse_query(c, None)?;
        let field = if func == Func::Count {
            None
        } else {
            let mut fields: Vec<&str> = q.expr.iter().flat_map(|e| e.conditions()).map(|c| c.field.as_str()).collect();
            fields.sort();
            fields.dedup();
            match fields.as_slice() {
                [f] => Some(f.to_string()),
                _ => {
                    return Err(format!(
                        "the condition tests several fields: say which one {verb} reads, before 'where': \
                         {verb} {}.<field> where ...",
                        q.pattern
                    ))
                }
            }
        };
        (q, field)
    } else {
        let target = word(c.next(), "a key pattern").map_err(|e| format!("{e}\n{}", usage()))?;
        let (pattern, field) = match split_target(&target) {
            Ok((p, f)) => (p, Some(f)),
            Err(_) if func == Func::Count => (target, None),
            Err(e) => return Err(format!("{e}\n{}", usage())),
        };
        if let Some(f) = &field {
            query::check_field(f)?;
        }
        let expr = if c.eat("where") { parse_query(c, Some(&pattern))?.expr } else { None };
        (Query { pattern, expr }, field)
    };
    let (mut group_by, mut limit) = (None, 0);
    loop {
        if c.eat("group") {
            if !c.eat("by") {
                return Err("expected 'by' after 'group': group by <field>".into());
            }
            let f = word(c.next(), "the field to group by")?;
            let f = f.strip_prefix(q.pattern.as_str()).and_then(|f| f.strip_prefix('.')).unwrap_or(&f).to_string();
            query::check_field(&f)?;
            group_by = Some(f);
        } else if c.eat("limit") {
            limit = uint(c.next(), "limit")? as usize;
            if limit == 0 {
                return Err("ValueError: limit must be greater than zero.".into());
            }
        } else {
            break;
        }
    }
    refuse_unsupported(c.peek())?;
    Ok((q, field, group_by, limit))
}

fn project(doc: &Json, fields: &[String]) -> Json {
    let mut m = serde_json::Map::new();
    for f in fields {
        let v = jsonpath::parse_path(f).ok().and_then(|p| jsonpath::get(doc, &p).cloned()).unwrap_or(Json::Null);
        m.insert(f.clone(), v);
    }
    Json::Object(m)
}

fn render_row(key: &str, value: &Value, fields: &Option<Vec<String>>) -> String {
    match (fields, value) {
        (Some(f), Value::Doc(doc)) => format!("{key}: {}", project(doc, f)),
        _ => format!("{key}: {} [{}]", value.display(), value.ty().name()),
    }
}

fn find_op(s: &str) -> Option<(usize, usize)> {
    let pos = s.find(['=', '!', '<', '>'])?;
    let two = s.get(pos..pos + 2).filter(|o| CmpOp::parse(o).is_some());
    Some((pos, if two.is_some() { 2 } else { 1 }))
}

fn literal_json(text: &str, quoted: bool) -> Result<Json, String> {
    if quoted {
        return Ok(Json::String(text.to_string()));
    }
    match text {
        "true" => Ok(Json::Bool(true)),
        "false" => Ok(Json::Bool(false)),
        "null" => Ok(Json::Null),
        t if is_valid_number(t) => Ok(crate::value::num_to_json(t)),
        t => Err(format!("String values like '{t}' must be quoted (\"\" or '').")),
    }
}

/// Refuses a word of a query that belongs to something the language leaves
/// out (joins, `and`, `group by`...), saying what to do instead.
fn refuse_unsupported(t: Option<&Token>) -> R {
    match t {
        Some(t) if t.kind == Kind::Comma => Err(query::Unsupported::Join.message().to_string()),
        Some(t) if t.is_bare() => match query::unsupported_word(&t.text) {
            Some(u) => Err(format!("'{}': {}", t.text, u.message())),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

/// An unquoted value: a literal, never a field of other documents.
fn condition_value(text: &str) -> Result<Json, String> {
    if query::is_field_reference(text) {
        return Err(format!("'{text}' names a field of other documents: {}", query::Unsupported::Join.message()));
    }
    literal_json(text, false)
}

/// Parses the condition of a query: comparisons `<field> <op> <value>`
/// joined by `and`, `or`, `not` and parentheses (`not` first, then
/// `and`, then `or`). The first comparison names the key pattern
/// (`user:*.age > 18`) unless `pattern` is given; the others may repeat it
/// or name the field alone (`role = "dev"`). See [`crate::query`].
fn parse_query(c: &mut Cur, pattern: Option<&str>) -> Result<Query, String> {
    let mut p = QueryParser { pattern: pattern.map(String::from) };
    let expr = p.or(c, 0)?;
    expr.check_size()?;
    // A list of values, a join or a pipeline right after the condition.
    match c.peek() {
        Some(t) if t.kind == Kind::Comma => return Err(query::Unsupported::Operator.message().into()),
        Some(t) if t.kind == Kind::RParen => return Err("')' without its '('".into()),
        // `group by` is read by the aggregates, and refused by the others.
        Some(t) if t.is_kw("group") => {}
        t => refuse_unsupported(t)?,
    }
    let pattern = p.pattern.ok_or("expected a condition such as: user:*.role = \"admin\"")?;
    Ok(Query { pattern, expr: Some(expr) })
}

struct QueryParser {
    /// Set by the first comparison when the command did not name it.
    pattern: Option<String>,
}

impl QueryParser {
    fn or(&mut self, c: &mut Cur, depth: usize) -> Result<Expr, String> {
        let mut parts = vec![self.and(c, depth)?];
        while c.eat("or") {
            parts.push(self.and(c, depth)?);
        }
        Ok(Expr::or(parts))
    }

    fn and(&mut self, c: &mut Cur, depth: usize) -> Result<Expr, String> {
        let mut parts = vec![self.unary(c, depth)?];
        while c.eat("and") {
            parts.push(self.unary(c, depth)?);
        }
        Ok(Expr::and(parts))
    }

    fn unary(&mut self, c: &mut Cur, depth: usize) -> Result<Expr, String> {
        if depth > query::MAX_DEPTH {
            return Err(format!("a query nests and / or / not at most {} levels deep", query::MAX_DEPTH));
        }
        if c.eat("not") {
            return Ok(Expr::negate(self.unary(c, depth + 1)?));
        }
        if c.peek().is_some_and(|t| t.kind == Kind::LParen) {
            c.next();
            let e = self.or(c, depth + 1)?;
            if !c.peek().is_some_and(|t| t.kind == Kind::RParen) {
                return Err("missing ')' to close the '('".into());
            }
            c.next();
            return Ok(e);
        }
        let (cond, negated) = parse_one_cond(c, self.pattern.as_deref())?;
        if self.pattern.is_none() {
            self.pattern = Some(cond.pattern.clone());
        }
        let e = Expr::test(cond)?;
        Ok(if negated { Expr::negate(e) } else { e })
    }
}

/// The filter of a vector search: `<field> <op> <value>` conditions joined
/// by `and`, on the documents of `pattern` (the field may repeat it).
fn parse_filter(c: &mut Cur, pattern: &str) -> Result<Vec<Cond>, String> {
    let mut filter = Vec::new();
    loop {
        let (cond, negated) = parse_one_cond(c, Some(pattern))?;
        if negated {
            return Err(format!(
                "a vector filter cannot use 'not {}': filter on the values you want instead",
                cond.op.symbol()
            ));
        }
        filter.push(cond);
        if !c.eat("and") {
            break;
        }
    }
    if c.peek().is_some_and(|t| t.is_kw("or") || t.is_kw("not")) {
        return Err("a vector filter joins its conditions with 'and' only: run one search per alternative \
                    and merge the results in your application."
            .into());
    }
    Ok(filter)
}

/// One condition, and whether it was written `not in`, `not like` or `not
/// ilike`. With `pattern`, the target is a field of its documents.
fn parse_one_cond(c: &mut Cur, pattern: Option<&str>) -> Result<(Cond, bool), String> {
    const USAGE: &str = "expected a condition such as: user:*.role = \"admin\"";
    let t0 = c.next().ok_or(USAGE)?;
    let mut target = t0.text.clone();
    let mut op_text = String::new();
    let mut value: Option<Json> = None;

    // Unquoted target may have the operator (and value) glued to it: age>18
    if t0.is_bare() {
        if let Some((pos, len)) = find_op(&t0.text) {
            target = t0.text[..pos].to_string();
            op_text = t0.text[pos..pos + len].to_string();
            let rest = &t0.text[pos + len..];
            if !rest.is_empty() {
                value = Some(condition_value(rest)?);
            }
        }
    }
    // `in`, `like`, `ilike`, each of them after `not`.
    let word_op =
        |t: Option<&Token>| t.filter(|t| t.is_bare()).and_then(|t| CmpOp::parse(&t.text)).filter(|o| o.is_word());
    let mut negated = false;
    if op_text.is_empty() {
        if c.peek().is_some_and(|t| t.is_kw("not")) && word_op(c.rest().get(1)).is_some() {
            c.next();
            negated = true;
        }
        if let Some(op) = word_op(c.peek()) {
            c.next();
            op_text = op.symbol().to_string();
        }
    }
    if op_text.is_empty() {
        let t1 = c.next();
        refuse_unsupported(t1)?;
        let t1 = t1.filter(|t| t.is_bare()).ok_or(USAGE)?;
        let (pos, len) = find_op(&t1.text).filter(|(p, _)| *p == 0).ok_or(USAGE)?;
        let _ = pos;
        op_text = t1.text[..len].to_string();
        let rest = &t1.text[len..];
        if !rest.is_empty() {
            value = Some(condition_value(rest)?);
        }
    }
    let op = CmpOp::parse(&op_text).ok_or_else(|| format!("unknown comparison operator '{op_text}'"))?;
    let value = match value {
        Some(v) => v,
        None => {
            let t = c.next().ok_or(USAGE)?;
            match t.kind {
                Kind::Bare => condition_value(&t.text)?,
                Kind::DQuote | Kind::SQuote => literal_json(&t.text, true)?,
                Kind::Json => {
                    serde_json::from_str::<Json>(&t.text).map_err(|e| format!("Invalid JSON literal: {e}"))?
                }
                _ => return Err(USAGE.into()),
            }
        }
    };
    query::check_value(op, &value)?;
    let (pattern, field) = match pattern {
        Some(p) => {
            let field = match target.strip_prefix(p).and_then(|f| f.strip_prefix('.')) {
                Some(f) => f.to_string(),
                None if target.contains('*') => {
                    return Err(format!(
                        "a condition tests fields of the documents of {p}, not of '{target}': {}",
                        query::Unsupported::Join.message()
                    ))
                }
                None => target,
            };
            (p.to_string(), field)
        }
        None => split_target(&target)?,
    };
    query::check_field(&field)?;
    Ok((Cond { pattern, field, op, value }, negated))
}

/// Splits `<pattern>.<field>`: the field starts at the first '.' after the last wildcard.
pub(crate) fn split_target(target: &str) -> Result<(String, String), String> {
    const NEED: &str = "expected both a key pattern and a field: <pattern>.<field>, e.g. doc:*.embedding";
    let from = target.rfind('*').map(|i| i + 1).unwrap_or(0);
    let Some(dot) = target[from..].find('.').map(|i| i + from) else {
        // `user:*.na*`: the last wildcard is in the field.
        if let Some(first) = target.find('*') {
            if let Some(dot) = target[first..].find('.') {
                query::check_field(&target[first + dot + 1..])?;
            }
        }
        return Err(NEED.into());
    };
    let (pattern, field) = (target[..dot].to_string(), target[dot + 1..].to_string());
    if pattern.is_empty() || field.is_empty() {
        return Err(NEED.into());
    }
    Ok((pattern, field))
}

const HELP: &str = r#"
--- KivaDB Shell Help ---
  SYNTAX RULES:
  - Strings        : MUST be quoted -> "Hello" or 'Kiva'.
  - Numbers/Bools  : MUST NOT be quoted -> 42, 3.14, true, false.
  - Arrays/Objects : JSON literals -> ["red", "green"]  {"name": "Fordi"}
  - Null / Bytes   : null, 0xDEADBEEF, b64:SGVsbG8=
  - Key Names      : Cannot be purely numeric ('44' is invalid, 'id44' is OK).
                     Use backticks for reserved words -> `set`.
  - Multi-Keys     : Use 'and' to chain operations -> get u and age.
  - Types          : string, number, boolean, null, array, object, bytes.

  DATA:
  set [t] <key> <val> [ttl <sec>]   : Create a key (fails if it exists;
                                      'if not exists' skips it instead).
  add <prefix> <val> [ttl <sec>]    : Create a key named <prefix> + a new identifier
                                      (add place: {...} -> place:6720f9c3a1b2c3d4e5f60718).
  update [t] <key> <val>          : Replace the value of an existing key (type-safe).
  get [t] <k1> [and <k2>...]        : Read values. 'get user:1.name' reads one field.
  has [t] <key> | typeof <key>      : Existence / type checks.
  del [t] <key> [if exists]         : Delete keys. 'del all keys' wipes the workspace (admin).
  change [t] <old> to [t] <new> [v] : Atomic rename / type migration.
  bump [t] <key> <add|set> <sec>    : Extend or reset a TTL.
  print <expr1>, <expr2>            : Arithmetic and "${key}" interpolation.

  QUERIES:
  scan [pattern] [limit N]          : Keys with type, size, date and TTL.
  keys [like P] [from A to B] [limit N] [reverse]
  range <A> to <B> [reverse] [limit N] [return f1, f2]
  json get|set|delete <key> <.path> [value]
  json append|insert|incr <key> <.path> <v> : Change an array or a number in place.
  json type|len|keys <key> [<.path>]  : Inspect a document.
  get where <condition> [limit N] [return f1, f2]
  count|sum|avg|min|max where ... [group by f] : Aggregates; also <verb> <pat>.<field> [where ...].
  conditions: <pat>.<f> <op> <v> [and|or ...], not, ( ) (op: = != > >= < <=),
              <f> [not] in [v1, v2...], <f> [not] like|ilike "Kin%" (ilike: any case).
  create index on <pat> (<field>) [using hash] : Secondary index (B-tree, or hash for =).
                                    Also <pat>.<field>; drop index, indexes.
  create unique index on <pat> (<f1>[, <f2>...]) : Refuse two documents with the same values.
  get similar <pat>.<field> to [..] [where f = v [and ...]] [limit N] [metric cosine|dot|l2] [return f1, f2]
                                    : Vector similarity search (AI embeddings).
  create vector index on <pat> (<field>) [using hnsw] : Vector index (exact, or HNSW). Also: drop vector index.
  tune vector index on <pat> (<field>) [recall 0.95] [auto] : Measure an HNSW index and keep the ef_search it needs (auto: again as it grows).
  semantic lookup|store|stats|clear : Semantic cache of answers (questions found by meaning).
  explain <query>                   : Show how a query would be executed.
  convert <key> to msgpack|json     : Change a document's storage encoding.

  TRANSACTIONS:
  begin | commit | rollback         : Group writes atomically.
  savepoint <name> | rollback to <name> | transaction status

  MAINTENANCE:
  stats | compact | check [repair] | hint info | encryption
  memory [pin|unpin "<pattern>"]    : Memory use; keep the values of some keys in RAM.
  backup to "<file.tar.gz>" [workspace <n>] : Archive the home or a workspace (admin).
  autocompact on|off|status|threshold <N>
  compress lz4|zlib|none            : Re-encode the whole workspace.

  WORKSPACES & CONFIG:
  workspace list|current|switch <n>|create <n> [path]|remove <n>
  config list | config get <key> | config set <key> <value>

  USERS (admin):
  create user "<n>" identified by "<p>" [role admin|editor|viewer]
  modify user "<n>" password "<p>" | modify user "<n>" role "<r>"
  drop user "<n>" | users | whoami | auth <user> <password>
  grant read|write on all|workspace "<w>" to "<n>"
  revoke read|write [on all|workspace "<w>"] from "<n>"
  audit last <N> | audit since "<YYYY-MM-DD>" | audit failed-logins

  SCRIPTING (Lua):
  eval '<lua code>' [args...]       : kiva.get/set/del/has/typeof/scan/incr, ARGV
  script load "<file>" as "<n>" | script run "<n>" [args] | script list|show|delete
  create trigger [name] on set|update|del "<pat>" run "<script>" | drop trigger | triggers
  bench script "<n>" iterations <N> | bench write <N> | bench read <N>

  REPLICATION & EVENTS:
  replica status|start|stop|sync|promote | replica of <host> <port>
  wait <N> [timeout_ms] | failover to <replica> | failover status
  subscribe <pattern> | unsubscribe [pattern]

  clear | help | exit | shutdown (stops the whole database, admin)
--------------------------------------------------------------------------
"#;

impl Session {
    /// Runs one command line and returns what it printed.
    pub fn execute(&mut self, line: &str) -> Out {
        let mut out = Out::default();
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("--") {
            return out;
        }
        let tokens = match tokenize(line) {
            Ok(t) => t,
            Err(e) => {
                out.err(e);
                return out;
            }
        };
        if tokens.is_empty() {
            return out;
        }
        let inst = self.inst.clone();
        let _slot = inst.exec_slots.acquire();
        let started = Instant::now();
        let cmd = if tokens[0].is_bare() { tokens[0].text.to_ascii_lowercase() } else { String::new() };

        self.wrote = false;
        let res = match self.check(required(&cmd, &tokens)) {
            Ok(()) => self.dispatch(&cmd, &tokens, line, &mut out),
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = res {
            out.err(format!("Error: {e}"));
        }

        // An error inside a transaction aborts it.
        const DATA_WRITES: &[&str] =
            &["set", "add", "update", "del", "change", "bump", "json", "convert", "eval", "script"];
        if self.txn.is_some() && out.has_err() && DATA_WRITES.contains(&cmd.as_str()) {
            self.rollback();
            out.err("Transaction rolled back (an error occurred inside the transaction).");
        }

        // Synchronous replication: wait for the configured number of replicas.
        if self.wrote && !inst.repl.is_replica() {
            let (sync, min_ack) = {
                let c = inst.cfg();
                (c.get("replication", "sync_mode") == "sync", c.get_u64("replication", "min_slaves_ack") as usize)
            };
            if sync && min_ack > 0 {
                let got = inst.repl.wait(min_ack, std::time::Duration::from_secs(5));
                if got < min_ack {
                    out.err(format!("Warning: write acknowledged by {got} of {min_ack} required replica(s)."));
                }
            }
        }

        const AUDITED: &[&str] = &[
            "create", "drop", "tune", "grant", "revoke", "modify", "del", "compact", "compress", "failover", "replica",
        ];
        let audited = AUDITED.contains(&cmd.as_str())
            || (cmd == "config" && tokens.get(1).is_some_and(|t| t.is_kw("set")))
            || (cmd == "workspace" && tokens.get(1).is_some_and(|t| t.is_kw("create") || t.is_kw("remove")))
            || (cmd == "check" && tokens.get(1).is_some_and(|t| t.is_kw("repair")))
            || (cmd == "script" && tokens.get(1).is_some_and(|t| t.is_kw("load") || t.is_kw("delete")));
        if audited {
            inst.audit.log(self.user_name(), &redact(&cmd, &tokens, line), !out.has_err());
        }

        let m = &inst.metrics;
        if !matches!(cmd.as_str(), "get" | "set" | "update" | "del") {
            m.others.fetch_add(1, Ordering::Relaxed);
        }
        if out.has_err() {
            m.errors.fetch_add(1, Ordering::Relaxed);
        }
        m.micros.fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        out
    }

    fn dispatch(&mut self, cmd: &str, t: &[Token], line: &str, out: &mut Out) -> R {
        let a = &t[1..];
        match cmd {
            "set" => self.chain(t, out, Session::set_one),
            "update" => self.chain(t, out, Session::update_one),
            "get" => {
                if a.first().is_some_and(|x| x.is_kw("range")) {
                    self.cmd_range(&a[1..], out)
                } else if a.first().is_some_and(|x| x.is_kw("where")) {
                    self.cmd_where(&a[1..], out)
                } else if a.first().is_some_and(|x| x.is_kw("similar")) {
                    self.cmd_similar(&a[1..], out)
                } else {
                    self.chain(t, out, Session::get_one)
                }
            }
            "del" => self.cmd_del(t, out),
            "has" => self.chain(t, out, Session::has_one),
            "typeof" => self.chain(t, out, Session::typeof_one),
            "change" => self.cmd_change(a, out),
            "bump" => self.cmd_bump(a, out),
            "add" => self.cmd_add(a, out),
            "print" => self.cmd_print(&line[t[0].end..], out),
            "scan" => self.cmd_scan(a, out),
            "keys" => self.cmd_keys(a, out),
            "range" => self.cmd_range(a, out),
            "count" | "sum" | "avg" | "min" | "max" => self.cmd_aggregate(cmd, a, out),
            "explain" => self.cmd_explain(a, out),
            "json" => self.cmd_json(a, out),
            "convert" => self.cmd_convert(a, out),
            "begin" | "commit" | "rollback" | "savepoint" | "transaction" => self.cmd_txn(cmd, a, out),
            "stats" => self.cmd_stats(a, out),
            "help" | "h" => {
                out.say(HELP);
                Ok(())
            }
            "clear" | "cls" => {
                out.action = Action::Clear;
                Ok(())
            }
            "exit" | "quit" => {
                out.action = Action::Exit;
                Ok(())
            }
            "shutdown" => {
                self.inst.audit.log(self.user_name(), "shutdown", true);
                self.inst.stop_requested.store(true, Ordering::SeqCst);
                out.say("KivaDB is shutting down.");
                out.action = Action::Exit;
                Ok(())
            }
            "ping" => {
                out.say("PONG");
                Ok(())
            }
            "version" => {
                out.say(format!("KivaDB {}", crate::instance::VERSION));
                Ok(())
            }
            _ => self.dispatch_admin(cmd, t, out),
        }
    }

    /// Runs `f` once per `and`-separated segment, reporting errors per segment.
    fn chain(&mut self, t: &[Token], out: &mut Out, f: fn(&mut Session, &[Token], &mut Out) -> R) -> R {
        let groups = split_by_and(t);
        for (i, mut seg) in groups.into_iter().enumerate() {
            // Segments after `and` may repeat the verb or leave it out.
            if i == 0 || seg[0].is_kw(&t[0].text) {
                seg.remove(0);
            }
            if let Err(e) = f(self, &seg, out) {
                out.err(format!("Error: {e}"));
            }
        }
        Ok(())
    }

    fn set_one(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "set [type] <key> <value> [ttl <sec>] [if not exists]";
        let (a, if_absent) = strip_clause(a, &["if", "not", "exists"]);
        let mut c = Cur::new(a);
        let forced = match c.peek().and_then(type_kw) {
            Some(ty) if a.len() >= 3 => {
                c.next();
                Some(ty)
            }
            _ => None,
        };
        let (Some(kt), Some(vt)) = (c.next(), c.next()) else {
            return Err(format!("Syntax error. Missing key or value.\nUsage: {USAGE}"));
        };
        let mut ttl = 0i64;
        if c.eat("ttl") {
            ttl = uint(c.next(), "TTL")? as i64;
            if ttl <= 0 {
                return Err("ValueError: TTL must be a positive integer.".into());
            }
        }
        if let Some(extra) = c.peek() {
            return Err(format!("Invalid set syntax near '{}'\nUsage: {USAGE}", extra.text));
        }
        let key = key_for_write(kt)?;
        let value = parse_value(vt, forced, &key)?;
        if self.kv_exists(&key)? {
            if if_absent {
                out.say(format!("OK: {key} skipped (already exists)"));
                return Ok(());
            }
            return Err(format!("Key '{key}' already exists. Use 'update' to change it."));
        }
        let expires_at = deadline_in(ttl);
        self.kv_put(&key, value, expires_at, false)?;
        out.say(format!("OK: {key} saved."));
        Ok(())
    }

    fn update_one(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "update [type] <key> <value>";
        let mut c = Cur::new(a);
        let forced = match c.peek().and_then(type_kw) {
            Some(ty) if a.len() >= 3 => {
                c.next();
                Some(ty)
            }
            _ => None,
        };
        let (Some(kt), Some(vt)) = (c.next(), c.next()) else {
            return Err(format!("Syntax error. Missing key or value.\nUsage: {USAGE}"));
        };
        if let Some(extra) = c.peek() {
            return Err(format!("Invalid update syntax near '{}'\nUsage: {USAGE}", extra.text));
        }
        let key = key_for_write(kt)?;
        let Some((old, meta)) = self.kv_get(&key)? else {
            return Err(format!("Key '{key}' not found. Use 'set' to create it."));
        };
        let actual = old.ty();
        if let Some(f) = forced {
            if f != actual {
                return Err(format!("Type mismatch. '{key}' is a [{}].", actual.name()));
            }
        }
        let value = parse_value(vt, forced, &key)?;
        // A key holding null has no type yet: it may take any value.
        if actual != Type::Null && value.ty() != actual {
            return Err(format!(
                "Cannot update [{}] with a value formatted as [{}].",
                actual.name(),
                value.ty().name()
            ));
        }
        self.kv_put(&key, value, meta.expires_at, meta.msgpack)?;
        out.say(format!("OK: {key} updated."));
        Ok(())
    }

    /// Resolves `key` or `key.path.inside.document`.
    /// Returns the value and the name of the key it came from.
    pub(crate) fn lookup(&mut self, name: &str) -> Result<Option<(Value, String)>, String> {
        if let Some((v, _)) = self.kv_get(name)? {
            return Ok(Some((v, name.to_string())));
        }
        for (i, ch) in name.char_indices() {
            if (ch != '.' && ch != '[') || i == 0 {
                continue;
            }
            let (key, path) = (&name[..i], &name[i..]);
            if !self.kv_exists(key)? {
                continue;
            }
            let Some((v, _)) = self.kv_get(key)? else { continue };
            let doc = match &v {
                Value::Doc(j) => j.clone(),
                // A string holding JSON text can be navigated too.
                Value::Str(s) => match serde_json::from_str::<Json>(s) {
                    Ok(j) if j.is_object() || j.is_array() => j,
                    _ => return Ok(None),
                },
                _ => return Ok(None),
            };
            let segs = jsonpath::parse_path(path)?;
            return Ok(jsonpath::get(&doc, &segs).map(|j| (Value::from_json(j.clone()), key.to_string())));
        }
        Ok(None)
    }

    fn typed_key(a: &[Token], usage: &str) -> Result<(Option<Type>, String), String> {
        match a {
            [k] => Ok((None, key_for_read(k)?)),
            [ty, k] if type_kw(ty).is_some() => Ok((type_kw(ty), key_for_read(k)?)),
            _ => Err(format!(
                "Invalid syntax near '{}'\nUsage: {usage}",
                a.last().map(|t| t.text.as_str()).unwrap_or("")
            )),
        }
    }

    fn get_one(&mut self, a: &[Token], out: &mut Out) -> R {
        let (want, name) = Self::typed_key(a, "get [type] <key>")?;
        match self.lookup(&name)? {
            None => out.say(format!("{name}: (nil)")),
            Some((v, _)) => {
                if let Some(w) = want {
                    if w != v.ty() {
                        return Err(format!("Type mismatch. '{name}' is a [{}].", v.ty().name()));
                    }
                }
                out.say(format!("{name}: {} [{}]", v.display(), v.ty().name()));
            }
        }
        Ok(())
    }

    fn has_one(&mut self, a: &[Token], out: &mut Out) -> R {
        let (want, key) = Self::typed_key(a, "has [type] <key>")?;
        match (self.kv_type(&key)?, want) {
            (None, _) => out.say(format!("No: Key '{key}' does not exist.")),
            (Some(_), None) => out.say(format!("Yes: Key '{key}' exists.")),
            (Some(actual), Some(w)) if actual == w => {
                out.say(format!("Yes: Key '{key}' exists with type [{}].", actual.name()))
            }
            (Some(actual), Some(w)) => {
                out.say(format!("No: Key '{key}' exists but type is [{}] (expected [{}]).", actual.name(), w.name()))
            }
        }
        Ok(())
    }

    fn typeof_one(&mut self, a: &[Token], out: &mut Out) -> R {
        let [k] = a else {
            return Err("Usage: typeof <key>".into());
        };
        let key = key_for_read(k)?;
        match self.kv_type(&key)? {
            Some(ty) => out.say(format!(" -> {key} is a [{}]", ty.name())),
            None => out.say(format!(" -> {key} does not exist (nil)")),
        }
        Ok(())
    }

    fn cmd_del(&mut self, t: &[Token], out: &mut Out) -> R {
        if t.len() == 3 && t[1].is_kw("all") && t[2].is_kw("keys") {
            self.check(Need::Admin)?;
            self.writable()?;
            if self.txn.is_some() {
                return Err("'del all keys' cannot run inside a transaction.".into());
            }
            self.engine.reset()?;
            self.wrote = true;
            out.say("All keys deleted. Database reset.");
            return Ok(());
        }
        self.chain(t, out, Session::del_one)
    }

    fn del_one(&mut self, a: &[Token], out: &mut Out) -> R {
        let (a, if_present) = strip_clause(a, &["if", "exists"]);
        let (want, key) = Self::typed_key(a, "del [type] <key> [if exists] | del all keys")?;
        let Some(actual) = self.kv_type(&key)? else {
            if if_present {
                out.say(format!("OK: {key} skipped (does not exist)"));
                return Ok(());
            }
            out.say(format!("Not found: {key}"));
            return Ok(());
        };
        if let Some(w) = want {
            if w != actual {
                return Err(format!("Type mismatch. Cannot delete '{key}' because it is a [{}].", actual.name()));
            }
        }
        self.kv_del(&key)?;
        out.say(format!("Deleted: {key}"));
        Ok(())
    }

    fn cmd_change(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "change [type] <old> to [type] <new> [value]";
        let mut c = Cur::new(a);
        let old_type = match c.peek().and_then(type_kw) {
            Some(ty) if a.get(1).is_some_and(|t| !t.is_kw("to")) => {
                c.next();
                Some(ty)
            }
            _ => None,
        };
        let old_key = key_for_read(c.next().ok_or(format!("Invalid change syntax.\nUsage: {USAGE}"))?)?;
        if !c.eat("to") {
            return Err(format!("Missing 'to' keyword.\nUsage: {USAGE}"));
        }
        let new_type = match c.peek().and_then(type_kw) {
            Some(ty) if c.rest().len() >= 2 => {
                c.next();
                Some(ty)
            }
            _ => None,
        };
        let new_key = key_for_write(c.next().ok_or("Missing target key.")?)?;
        let value_tok = c.next();
        c.finish(USAGE)?;

        let Some((old_val, meta)) = self.kv_get(&old_key)? else {
            return Err(format!("Source key '{old_key}' not found."));
        };
        let actual = old_val.ty();
        if let Some(ot) = old_type {
            if ot != actual {
                return Err(format!("Type mismatch for source key. '{old_key}' is actually a '{}'.", actual.name()));
            }
        }
        let same = old_key == new_key;
        match value_tok {
            None => {
                if new_type.is_some_and(|nt| nt != actual) {
                    return Err("Cannot change type without providing a new value.".into());
                }
                if same {
                    out.say(format!("Renamed: {old_key} -> {new_key} (No change needed)"));
                    return Ok(());
                }
                if self.kv_exists(&new_key)? {
                    return Err(format!("Failed to rename. Target '{new_key}' already exists."));
                }
                self.apply(vec![
                    BatchOp::Put {
                        key: new_key.clone(),
                        value: old_val,
                        expires_at: meta.expires_at,
                        msgpack: meta.msgpack,
                    },
                    BatchOp::Del { key: old_key.clone() },
                ])?;
                out.say(format!("Renamed: {old_key} -> {new_key} (Type preserved)"));
            }
            Some(vt) => {
                let value = parse_value(vt, new_type, &new_key)
                    .map_err(|e| format!("Migration failed ({e}) Source key '{old_key}' has been preserved."))?;
                if !same && self.kv_exists(&new_key)? {
                    return Err(format!("Migration failed. Target '{new_key}' already exists."));
                }
                let ty = value.ty();
                let mut ops = vec![BatchOp::Put { key: new_key.clone(), value, expires_at: 0, msgpack: false }];
                if !same {
                    ops.push(BatchOp::Del { key: old_key.clone() });
                }
                // One atomic batch: the old key only disappears if the new one is written.
                self.apply(ops)?;
                out.say(format!("Migrated: {old_key} -> {new_key} (Type: {})", ty.name()));
            }
        }
        Ok(())
    }

    /// `add place: {...}`: stores the value under the prefix followed by a
    /// new identifier ([`crate::util::new_id`]) and says which key it is.
    fn cmd_add(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "add <prefix> <value> [ttl <sec>]";
        let mut c = Cur::new(a);
        let (Some(pt), Some(vt)) = (c.next(), c.next()) else {
            return Err(format!("Syntax error. Missing prefix or value.\nUsage: {USAGE}"));
        };
        let mut ttl = 0i64;
        if c.eat("ttl") {
            ttl = uint(c.next(), "TTL")? as i64;
            if ttl <= 0 {
                return Err("ValueError: TTL must be a positive integer.".into());
            }
        }
        if let Some(extra) = c.peek() {
            return Err(format!("Invalid add syntax near '{}'\nUsage: {USAGE}", extra.text));
        }
        if !matches!(pt.kind, Kind::Bare | Kind::Backtick) {
            return Err(format!("The prefix '{}' is bare text, such as place: (no quotes).", pt.text));
        }
        let key = self.new_key(&pt.text)?;
        let value = parse_value(vt, None, &key)?;
        self.kv_put(&key, value, deadline_in(ttl), false)?;
        out.say(format!("OK: {key} saved."));
        Ok(())
    }

    /// `prefix` followed by a new identifier: a key that does not exist.
    pub(crate) fn new_key(&mut self, prefix: &str) -> Result<String, String> {
        for _ in 0..8 {
            let key = format!("{prefix}{}", crate::util::new_id());
            validate_key_name(&key)?;
            if !self.kv_exists(&key)? {
                return Ok(key);
            }
        }
        Err(format!("could not find a free key after {prefix}: every one tried exists"))
    }

    fn cmd_bump(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: bump [type] <key> <add|set> <ttl_seconds>";
        let (want, rest) = match a {
            [ty, _, _, _] if type_kw(ty).is_some() => (type_kw(ty), &a[1..]),
            [_, _, _] => (None, a),
            _ => return Err(USAGE.into()),
        };
        let key = key_for_read(&rest[0])?;
        let add = if rest[1].is_kw("add") {
            true
        } else if rest[1].is_kw("set") {
            false
        } else {
            return Err(format!("Invalid operation '{}'. Use 'add' or 'set'.", rest[1].text));
        };
        let secs = rest[2].text.parse::<i64>().ok().filter(|n| *n > 0 && rest[2].is_bare());
        let Some(secs) = secs else {
            return Err("ValueError: TTL must be a positive integer.".into());
        };
        let Some((value, meta)) = self.kv_get(&key)? else {
            return Err(format!("Key '{key}' not found or already expired."));
        };
        if let Some(w) = want {
            if w != value.ty() {
                return Err(format!(
                    "TypeError: Key '{key}' is of type '{}', expected '{}'",
                    value.ty().name(),
                    w.name()
                ));
            }
        }
        let (now, ms) = (now_ms(), secs.saturating_mul(1000));
        let expires_at = if add && meta.expires_at > now { meta.expires_at + ms } else { now + ms };
        self.kv_put(&key, value, expires_at, meta.msgpack)?;
        out.say(format!("OK: Key '{key}' TTL updated successfully ({} {secs}s).", if add { "add" } else { "set" }));
        Ok(())
    }

    fn cmd_print(&mut self, raw: &str, out: &mut Out) -> R {
        let chars: Vec<char> = raw.chars().collect();
        let mut resolve = |name: &str| self.lookup(name).map(|o| o.map(|(v, _)| v));
        let values = expr::eval_list(&chars, &mut resolve)?;
        out.say(values.iter().map(|v| v.render()).collect::<Vec<_>>().join(" "));
        Ok(())
    }

    fn cmd_scan(&mut self, a: &[Token], out: &mut Out) -> R {
        let mut c = Cur::new(a);
        let pattern = match c.peek() {
            Some(t) if !t.is_kw("limit") => word(c.next(), "a key pattern")?,
            _ => "*".to_string(),
        };
        let limit = if c.eat("limit") { uint(c.next(), "limit")? as usize } else { 0 };
        c.finish("scan [pattern] [limit N]")?;

        let rows = self.engine.keys_matching(&pattern, limit);
        out.say(format!("\n--- KivaDB Scan (workspace: {}) ---", self.ws));
        if rows.is_empty() {
            out.say(" [!] No keys to show.");
            out.say("-".repeat(78));
            return Ok(());
        }
        out.say(format!(" {:<24} | {:<7} | {:>10} | {:<19} | STATUS", "KEY", "TYPE", "SIZE", "MODIFIED"));
        out.say("-".repeat(78));
        for (key, e) in &rows {
            let mut status = Vec::new();
            if e.expires_at > 0 {
                status.push(format!("TTL: {}s", secs_left(e.expires_at)));
            }
            if e.flags & FLAG_COMP_MASK != 0 {
                status.push(e.compression().name().to_string());
            }
            if e.is_msgpack() {
                status.push("msgpack".to_string());
            }
            out.say(format!(
                " {:<24} | {:<7} | {:>10} | {:<19} | {}",
                key,
                e.ty.name(),
                human_size(e.raw_size as u64),
                fmt_time(e.timestamp),
                status.join(", ")
            ));
        }
        out.say("-".repeat(78));
        out.say(format!("Total: {} keys.\n", rows.len()));
        Ok(())
    }

    fn cmd_keys(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "keys [like <pattern>] [from <A> to <B>] [limit N] [reverse]";
        let mut c = Cur::new(a);
        let mut pattern = None;
        let (mut from, mut to) = (None, None);
        loop {
            if c.eat("like") {
                pattern = Some(word(c.next(), "a key pattern")?);
            } else if c.eat("from") {
                from = Some(word(c.next(), "a lower bound")?);
                if !c.eat("to") {
                    return Err(format!("Missing 'to' keyword.\nUsage: {USAGE}"));
                }
                to = Some(word(c.next(), "an upper bound")?);
            } else {
                break;
            }
        }
        let tail = parse_tail(&mut c)?;
        c.finish(USAGE)?;
        let keys: Vec<String> = match &pattern {
            Some(p) => {
                let mut v: Vec<String> = self
                    .engine
                    .keys_matching(p, 0)
                    .into_iter()
                    .map(|(k, _)| k)
                    .filter(|k| from.as_ref().is_none_or(|f| k >= f) && to.as_ref().is_none_or(|t| k <= t))
                    .collect();
                if tail.reverse {
                    v.reverse();
                }
                if tail.limit > 0 {
                    v.truncate(tail.limit);
                }
                v
            }
            None => self
                .engine
                .range(from.as_deref(), to.as_deref(), tail.reverse, tail.limit)
                .into_iter()
                .map(|(k, _)| k)
                .collect(),
        };
        for k in &keys {
            out.say(k.as_str());
        }
        out.say(format!("({} keys)", keys.len()));
        Ok(())
    }

    fn cmd_range(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "range <from> to <to> [reverse] [limit N] [return f1, f2]";
        let mut c = Cur::new(a);
        let from = word(c.next(), "a lower bound").map_err(|e| format!("{e}\nUsage: {USAGE}"))?;
        if !c.eat("to") {
            return Err(format!("Missing 'to' keyword.\nUsage: {USAGE}"));
        }
        let to = word(c.next(), "an upper bound")?;
        let tail = parse_tail(&mut c)?;
        c.finish(USAGE)?;
        let rows = self.engine.range(Some(&from), Some(&to), tail.reverse, tail.limit);
        let mut shown = 0;
        for (key, _) in rows {
            match self.engine.get(&key) {
                Ok(Some((v, _))) => {
                    out.say(render_row(&key, &v, &tail.fields));
                    shown += 1;
                }
                Ok(None) => {}
                Err(e) => out.err(format!("{key}: Error: {e}")),
            }
        }
        out.say(format!("({shown} keys)"));
        Ok(())
    }

    fn cmd_where(&mut self, a: &[Token], out: &mut Out) -> R {
        let mut c = Cur::new(a);
        let q = parse_query(&mut c, None)?;
        let tail = parse_tail(&mut c)?;
        refuse_unsupported(c.peek())?;
        c.finish("get where <condition> [limit N] [return f1, f2]")?;
        let (rows, _) = self.engine.query_expr(&q, tail.limit)?;
        for (key, doc) in &rows {
            match &tail.fields {
                Some(f) => out.say(format!("{key}: {}", project(doc, f))),
                None => out.say(format!("{key}: {doc}")),
            }
        }
        out.say(format!("({} matches)", rows.len()));
        Ok(())
    }

    /// `count | sum | avg | min | max`, over the documents a query selects,
    /// optionally per value of a field.
    fn cmd_aggregate(&mut self, verb: &str, a: &[Token], out: &mut Out) -> R {
        let func = Func::parse(verb).ok_or("not an aggregate")?;
        let mut c = Cur::new(a);
        let (q, field, group_by, limit) = parse_aggregate(&mut c, func)?;
        c.finish(&aggregate_usage(func))?;
        let agg = Agg::new(verb, func, field.as_deref())?;
        let mut acc = Aggregation::new(vec![agg], group_by.as_deref())?;
        self.engine.scan_query(&q, limit, |_, doc| acc.add(&doc))?;
        let r = acc.finish();
        let name = {
            let mut n = verb[..1].to_uppercase() + &verb[1..];
            if let (Some(f), Some(_)) = (&field, &group_by) {
                n.push_str(&format!(" of {f}"));
            }
            n
        };
        // Count shows the number alone; the others, how many values made it.
        let detail = |v: u64, matches: usize, grouped: bool| match func {
            Func::Count if field.is_none() => String::new(),
            Func::Count | Func::Sum => format!(" ({matches} matches)"),
            _ if grouped => format!(" ({v} values)"),
            _ => format!(" ({v} values, {matches} matches)"),
        };
        let show = |v: &Json| match v {
            Json::Number(n) => n.as_f64().map(fmt_f64).unwrap_or_else(|| n.to_string()),
            v => v.to_string(),
        };
        match &group_by {
            None => {
                let (_, v, n) = &r.groups[0].values[0];
                out.say(format!("{name}: {}{}", show(v), detail(*n, r.matches, false)));
            }
            Some(g) => {
                out.say(format!("{name} by {g} ({} groups, {} matches):", r.groups.len(), r.matches));
                for grp in &r.groups {
                    let (_, v, n) = &grp.values[0];
                    let key = grp.key.as_ref().map(|k| k.to_string()).unwrap_or_default();
                    out.say(format!("  {key}: {}{}", show(v), detail(*n, grp.matches, true)));
                }
            }
        }
        Ok(())
    }

    fn cmd_similar(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str =
            "get similar <pattern>.<field> to [n1, n2, ...] | <key>.<field> [where <field> <op> <value> [and ...]] [limit N] [metric cosine|dot|l2] [ef N] [exact] [return f1, f2]";
        let mut c = Cur::new(a);
        let target = word(c.next(), "<pattern>.<field>").map_err(|e| format!("{e}\nUsage: {USAGE}"))?;
        let (pattern, field) = split_target(&target)?;
        if !c.eat("to") {
            return Err(format!("Missing 'to' keyword.\nUsage: {USAGE}"));
        }
        let vt = c.next().ok_or_else(|| format!("Missing the query vector.\nUsage: {USAGE}"))?;
        let json = match vt.kind {
            Kind::Json => serde_json::from_str::<Json>(&vt.text).map_err(|e| format!("Invalid JSON literal: {e}"))?,
            // "similar to this document": the vector stored in another key.
            Kind::Bare | Kind::Backtick => match self.lookup(&vt.text)? {
                Some((v, _)) => v.to_json(),
                None => return Err(format!("'{}' does not exist.", vt.text)),
            },
            Kind::DQuote | Kind::SQuote => return Err(crate::vector::NOT_EMBEDDINGS.into()),
            _ => return Err(format!("The query vector must be a JSON array of numbers.\nUsage: {USAGE}")),
        };
        let vector = crate::vector::from_json(&json).ok_or("The query vector must be a non-empty array of numbers.")?;
        let mut metric = crate::vector::Metric::Cosine;
        let mut tail = Tail::default();
        let (mut ef, mut exact) = (0, false);
        let mut filter = Vec::new();
        loop {
            if c.eat("where") {
                if !filter.is_empty() {
                    return Err("one 'where' per search: join its conditions with 'and'".into());
                }
                filter = parse_filter(&mut c, &pattern)?;
                continue;
            }
            if c.eat("metric") {
                let m = word(c.next(), "a metric")?;
                metric = crate::vector::Metric::parse(&m).ok_or("metric must be cosine, dot or l2")?;
                continue;
            }
            if c.eat("ef") {
                ef = uint(c.next(), "ef")? as usize;
                if !(1..=100_000).contains(&ef) {
                    return Err("ef must be between 1 and 100000".into());
                }
                continue;
            }
            if c.eat("exact") {
                exact = true;
                continue;
            }
            let before = c.i;
            let t = parse_tail(&mut c)?;
            if t.limit > 0 {
                tail.limit = t.limit;
            }
            if t.fields.is_some() {
                tail.fields = t.fields;
            }
            if c.i == before {
                break;
            }
        }
        c.finish(USAGE)?;
        let mut q = crate::engine::VectorQuery::new(&pattern, &field, vector, metric, tail.limit);
        q.ef = ef;
        q.exact = exact;
        q.filter = filter;
        let (r, cached) = self.similar_cached(&q)?;
        for (key, score, doc) in &r.rows {
            let shown = match &tail.fields {
                Some(f) => project(doc, f),
                None => doc.clone(),
            };
            out.say(format!("{key}: {score:.4} {shown}"));
        }
        if let Some(similarity) = cached {
            out.say(format!(
                "({} results, {} metric, semantic cache: a query at similarity {similarity:.4} was answered earlier)",
                r.rows.len(),
                metric.name()
            ));
            return Ok(());
        }
        out.say(format!(
            "({} results, {} metric, {} vectors compared, {}{})",
            r.rows.len(),
            metric.name(),
            r.compared,
            match (r.indexed, r.approximate) {
                (true, true) => "HNSW index, approximate",
                (true, false) => "vector index",
                _ => "documents read from disk",
            },
            r.filter.as_ref().map(|f| format!("; filter {f}")).unwrap_or_default()
        ));
        Ok(())
    }

    fn cmd_explain(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: explain get where ... | explain count|sum|avg|min|max ... | explain range <A> to <B> | explain get <key>";
        let mut c = Cur::new(a);
        if c.eat("distributed") {
            return Err(query::Unsupported::Distributed.message().into());
        }
        let verb = c.next().ok_or(USAGE)?;
        let total = self.engine.len();
        out.say("Query Plan");
        let agg = Func::parse(&verb.text).filter(|_| verb.is_bare());
        if (verb.is_kw("get") && c.eat("where")) || agg.is_some() {
            let q = match agg {
                Some(f) => parse_aggregate(&mut c, f)?.0,
                None => {
                    let q = parse_query(&mut c, None)?;
                    parse_tail(&mut c)?;
                    refuse_unsupported(c.peek())?;
                    q
                }
            };
            c.finish(USAGE)?;
            let plan = self.engine.explain_query(&q);
            let filter = match q.expr.as_ref() {
                Some(Expr::Test(t)) => {
                    let cond = &t.cond;
                    format!("{}.{} {} {}", cond.pattern, cond.field, cond.op.symbol(), cond.value)
                }
                _ => q.describe(),
            };
            out.say(format!("  Filter    : {filter}"));
            out.say(format!("  Strategy  : {}", plan.strategy));
            match &plan.index {
                Some(ix) => out.say(format!("  Index     : {ix}")),
                None => out.say("  Index     : none"),
            }
            out.say(format!("  Estimated : {} of {} keys examined", plan.candidates, plan.total_keys));
            if plan.index.is_none() {
                // The comparisons an index would serve, and that have none.
                let mut tips: Vec<String> = Vec::new();
                for cond in q.expr.iter().flat_map(|e| e.positive_conditions()) {
                    let tip = format!("create index on {} ({})", cond.pattern, cond.field);
                    if crate::engine::Engine::indexable(cond)
                        && self.engine.explain(cond).index.is_none()
                        && !tips.contains(&tip)
                    {
                        tips.push(tip);
                    }
                }
                for tip in tips {
                    out.say(format!("  Tip       : {tip}"));
                }
            }
        } else if verb.is_kw("range") || (verb.is_kw("get") && c.eat("range")) {
            let from = word(c.next(), "a lower bound")?;
            if !c.eat("to") {
                return Err(USAGE.into());
            }
            let to = word(c.next(), "an upper bound")?;
            let n = self.engine.range(Some(&from), Some(&to), false, 0).len();
            out.say("  Strategy  : ORDERED INDEX RANGE SCAN");
            out.say("  Index     : primary (in-memory B-tree, keys sorted)");
            out.say(format!("  Estimated : {n} of {total} keys examined"));
        } else if verb.is_kw("get") {
            let key = word(c.next(), "a key")?;
            out.say("  Strategy  : POINT LOOKUP");
            out.say("  Index     : primary (in-memory B-tree)");
            out.say(format!(
                "  Estimated : 1 of {total} keys examined, {} disk read",
                if self.engine.contains(&key) { 1 } else { 0 }
            ));
        } else {
            return Err(USAGE.into());
        }
        Ok(())
    }

    fn cmd_json(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: json get|type|len|keys <key> [<.path>] | json set <key> <.path> <value> \
                             | json delete <key> <.path> | json append <key> <.path> <value>... \
                             | json insert <key> <.path[n]> <value> | json incr <key> <.path> <number>";
        let mut c = Cur::new(a);
        let sub = c.next().ok_or(USAGE)?.text.to_ascii_lowercase();
        let key = key_for_read(c.next().ok_or(USAGE)?)?;
        let reading = matches!(sub.as_str(), "get" | "type" | "len" | "keys");
        let path_text = match c.next() {
            // `["a.b"]` reaches us as a JSON literal token.
            Some(t) if t.kind == Kind::Json => t.text.clone(),
            Some(t) => word(Some(t), "a path such as .name").map_err(|_| USAGE.to_string())?,
            None if reading => ".".to_string(),
            None => return Err(USAGE.into()),
        };
        let path = jsonpath::parse_path(&path_text)?;
        let at = match path_text.trim() {
            _ if path.is_empty() => key.clone(),
            p if p.starts_with(['.', '[']) => format!("{key}{p}"),
            p => format!("{key}.{p}"),
        };

        if reading {
            c.finish(USAGE)?;
            let Some((value, _)) = self.kv_get(&key)? else {
                return Err(format!("Key '{key}' not found."));
            };
            let doc = doc_of(&key, value)?;
            let Some(j) = jsonpath::get(&doc, &path) else {
                out.say(format!("{at}: (nil)"));
                return Ok(());
            };
            let ty = || Value::from_json(j.clone()).ty().name();
            match sub.as_str() {
                "get" => {
                    let v = Value::from_json(j.clone());
                    out.say(format!("{at}: {} [{}]", v.display(), v.ty().name()));
                }
                "type" => out.say(format!("{at}: [{}]", ty())),
                "len" => match jsonpath::len(j) {
                    Some(n) => out.say(format!("{at}: {n}")),
                    None => return Err(format!("TypeError: {at} is a [{}], which has no length.", ty())),
                },
                _ => {
                    let Json::Object(m) = j else {
                        return Err(format!("TypeError: {at} is a [{}], not an object.", ty()));
                    };
                    if m.is_empty() {
                        out.say(format!("{at}: no fields"));
                    }
                    for k in m.keys() {
                        out.say(k.clone());
                    }
                }
            }
            return Ok(());
        }

        enum Change {
            Set(Json),
            Delete,
            Append(Vec<Json>),
            Insert(Json),
            Incr(serde_json::Number),
        }
        let change = match sub.as_str() {
            "set" => Change::Set(parse_value(c.next().ok_or(USAGE)?, None, &key)?.to_json()),
            "delete" | "del" => Change::Delete,
            "append" => {
                let mut values = Vec::new();
                while let Some(t) = c.next() {
                    if t.kind != Kind::Comma {
                        values.push(parse_value(t, None, &key)?.to_json());
                    }
                }
                if values.is_empty() {
                    return Err(USAGE.into());
                }
                Change::Append(values)
            }
            "insert" => Change::Insert(parse_value(c.next().ok_or(USAGE)?, None, &key)?.to_json()),
            "incr" => {
                let t = c.next().ok_or(USAGE)?;
                match literal_json(&t.text, t.is_quoted()) {
                    Ok(Json::Number(n)) if t.is_bare() => Change::Incr(n),
                    _ => return Err("ValueError: the increment must be a number.".into()),
                }
            }
            _ => return Err(USAGE.into()),
        };
        c.finish(USAGE)?;

        // Read, change and write back under the engine lock: a client
        // changing another field of the same document at the same time
        // cannot lose this change, nor this one lose theirs.
        let done = self.kv_modify(&key, |value| {
            let mut doc = doc_of(&key, value)?;
            let msg = match change {
                Change::Set(v) => {
                    jsonpath::set(&mut doc, &path, v).map_err(|e| KivaError::Invalid(format!("{e} ({at})")))?;
                    format!("OK: {at} updated.")
                }
                Change::Delete => {
                    if !jsonpath::delete(&mut doc, &path) {
                        return Ok((None, format!("Not found: {at}")));
                    }
                    format!("Deleted: {at}")
                }
                Change::Append(values) => {
                    let n = jsonpath::append(&mut doc, &path, values)
                        .map_err(|e| KivaError::Invalid(format!("{e} ({at})")))?;
                    format!("OK: {at} now has {n} elements.")
                }
                Change::Insert(v) => {
                    let n =
                        jsonpath::insert(&mut doc, &path, v).map_err(|e| KivaError::Invalid(format!("{e} ({at})")))?;
                    format!("OK: inserted at {at}; the array now has {n} elements.")
                }
                Change::Incr(by) => {
                    let v =
                        jsonpath::incr(&mut doc, &path, &by).map_err(|e| KivaError::Invalid(format!("{e} ({at})")))?;
                    format!("{at}: {v} [number]")
                }
            };
            if !(doc.is_object() || doc.is_array()) {
                return Err(KivaError::Invalid("the root of a document must stay an object or array.".into()));
            }
            Ok((Some(Value::Doc(doc)), msg))
        })?;
        match done {
            Some(msg) => out.say(msg),
            None => return Err(format!("Key '{key}' not found.")),
        }
        Ok(())
    }

    fn cmd_convert(&mut self, a: &[Token], out: &mut Out) -> R {
        const USAGE: &str = "Usage: convert <key> to msgpack|json";
        let [k, to, fmt] = a else {
            return Err(USAGE.into());
        };
        if !to.is_kw("to") {
            return Err(USAGE.into());
        }
        let msgpack = match fmt.text.to_ascii_lowercase().as_str() {
            "msgpack" | "messagepack" => true,
            "json" => false,
            _ => return Err(USAGE.into()),
        };
        let key = key_for_read(k)?;
        let Some((value, meta)) = self.kv_get(&key)? else {
            return Err(format!("Key '{key}' not found."));
        };
        if !value.ty().is_doc() {
            return Err(format!(
                "TypeError: only arrays and objects can be converted; '{key}' is a [{}].",
                value.ty().name()
            ));
        }
        let name = if msgpack { "MessagePack" } else { "JSON" };
        if meta.msgpack == msgpack {
            out.say(format!("OK: {key} is already stored as {name}."));
            return Ok(());
        }
        let (before, after) = (value.encode(meta.msgpack)?.len(), value.encode(msgpack)?.len());
        self.kv_put(&key, value, meta.expires_at, msgpack)?;
        out.say(format!("OK: {key} converted to {name} ({before} -> {after} bytes)."));
        Ok(())
    }

    fn cmd_txn(&mut self, cmd: &str, a: &[Token], out: &mut Out) -> R {
        match cmd {
            "begin" => {
                let id = self.begin()?;
                out.say(format!("Transaction {id} started."));
            }
            "commit" => {
                let n = self.commit()?;
                out.say(format!("Transaction committed ({n} keys written)."));
            }
            "savepoint" => {
                let [name] = a else {
                    return Err("Usage: savepoint <name>".into());
                };
                self.savepoint(&name.text)?;
                out.say(format!("Savepoint '{}' created.", name.text));
            }
            "rollback" => match a {
                [] => {
                    if !self.rollback() {
                        return Err("no active transaction".into());
                    }
                    out.say("Transaction rolled back.");
                }
                [to, name] if to.is_kw("to") => {
                    self.rollback_to(&name.text)?;
                    out.say(format!("Rolled back to savepoint '{}'.", name.text));
                }
                _ => return Err("Usage: rollback | rollback to <savepoint>".into()),
            },
            _ => {
                if !(a.len() == 1 && a[0].is_kw("status")) {
                    return Err("Usage: transaction status".into());
                }
                match &self.txn {
                    Some(t) => {
                        out.say(format!("Transaction ID: {}", t.id));
                        out.say("State: ACTIVE");
                        out.say(format!("Savepoints: {}", t.savepoints.len()));
                        out.say(format!("Modified keys: {}", t.writes.len()));
                    }
                    None => out.say("State: NONE (no active transaction)"),
                }
            }
        }
        Ok(())
    }

    fn cmd_stats(&mut self, a: &[Token], out: &mut Out) -> R {
        if !a.is_empty() {
            return Err("'stats' command does not accept arguments.".into());
        }
        let s = self.engine.stats();
        let frag = s.fragmentation * 100.0;
        out.say(format!("\n{}", "=".repeat(58)));
        out.say("     KivaDB Engine Statistics");
        out.say("-".repeat(58));
        out.say(format!(" > Workspace            : {}", self.ws));
        out.say(format!(" > Total Keys           : {} ({} with TTL)", s.keys, s.ttl_keys));
        out.say(format!(" > Storage Size         : {}", human_size(s.file_bytes)));
        out.say(format!(" > Live Data Size       : {}", human_size(s.live_bytes)));
        out.say(format!(
            " > Disk Fragmentation   : {frag:.1}%{}",
            if frag >= 20.0 { " [Action Needed: 'compact']" } else { "" }
        ));
        out.say(format!(" > Memory Usage (RAM)   : {}", human_size(s.memory_bytes)));
        if s.sync_writes {
            let per_sync = if s.disk_syncs > 0 { s.durable_writes as f64 / s.disk_syncs as f64 } else { 0.0 };
            out.say(format!(
                " > Durability           : on disk before each answer ({} writes, {} disk syncs, {per_sync:.1} writes per sync)",
                s.durable_writes, s.disk_syncs
            ));
        } else {
            out.say(
                " > Durability           : on disk within 1 second ('config set sync_writes true': before each answer)",
            );
        }
        let saved = if s.raw_value_bytes > 0 {
            (1.0 - s.stored_value_bytes as f64 / s.raw_value_bytes as f64) * 100.0
        } else {
            0.0
        };
        out.say(format!(
            " > Compression          : {} ({} -> {}, {saved:.1}% saved, {} records)",
            s.compression,
            human_size(s.raw_value_bytes),
            human_size(s.stored_value_bytes),
            s.compressed_records
        ));
        let mode = match s.load_mode {
            "hint" => "from hint file",
            "scan" => "full log scan",
            _ => "new database",
        };
        out.say(format!(" > Startup              : {:.2} ms ({mode})", s.load_ms));
        if s.recovered_bytes > 0 || s.recovered_compaction {
            out.say(format!(
                " > Crash Recovery       : {} bytes of torn log discarded{}",
                s.recovered_bytes,
                if s.recovered_compaction { ", interrupted compaction cleaned up" } else { "" }
            ));
        }
        out.say(format!(" > Checksum Errors      : {}", s.corrupted_detected));
        match s.last_check {
            Some((ts, ok, bad)) => {
                out.say(format!(" > Last Integrity Check : {} (OK: {ok}, corrupted: {bad})", fmt_time(ts)))
            }
            None => out.say(" > Last Integrity Check : never"),
        }
        out.say(format!(
            " > Auto-Compaction      : {} (threshold {}%), {} run(s){}",
            if s.auto_compact { "on" } else { "off" },
            s.compact_threshold,
            s.compactions,
            s.last_compact.map(|t| format!(", last {}", fmt_time(t))).unwrap_or_default()
        ));
        if let Some(err) = &s.compact_error {
            out.err(format!(" > Auto-Compaction Error: {err}"));
        }
        out.say(format!(" > Secondary Indexes    : {} (+ {} vector)", s.indexes, s.vector_indexes));
        if s.cache_limit > 0 {
            let total = s.cache_hits + s.cache_misses;
            let rate = if total > 0 { s.cache_hits as f64 * 100.0 / total as f64 } else { 0.0 };
            out.say(format!(
                " > Value Cache          : {} of {} used, {} values, {rate:.1}% hits",
                human_size(s.cache_bytes),
                human_size(s.cache_limit),
                s.cache_entries
            ));
        } else {
            out.say(" > Value Cache          : off (values are read from disk)");
        }
        let repl = &self.inst.repl;
        if repl.is_replica() {
            let l = repl.link();
            out.say(format!(" > Replication          : replica of {} ({})", l.master, l.status));
            out.say(format!(
                " > Last Sync            : {}",
                l.last_sync.map(fmt_time).unwrap_or_else(|| "never".into())
            ));
        } else {
            let reps = repl.replicas();
            let seq = repl.seq();
            let lag = reps.iter().map(|r| seq.saturating_sub(r.acked)).max().unwrap_or(0);
            out.say(format!(" > Replication          : master, {} replica(s), max lag {lag} ops", reps.len()));
        }
        out.say(format!(" > Database Path        : {}", s.path));
        out.say(format!("{}\n", "=".repeat(58)));
        Ok(())
    }
}

/// The privilege a command needs before it is even parsed further.
fn required(cmd: &str, t: &[Token]) -> Need {
    let sub = |kw: &str| t.get(1).is_some_and(|x| x.is_kw(kw));
    match cmd {
        "auth" | "login" | "help" | "h" | "exit" | "quit" | "ping" | "version" | "clear" | "cls" => Need::None,
        "set" | "add" | "update" | "del" | "change" | "bump" | "begin" | "commit" | "rollback" | "savepoint"
        | "convert" | "eval" | "wait" => Need::Write,
        "json" if !["get", "type", "len", "keys"].iter().any(|k| sub(k)) => Need::Write,
        "script" if sub("run") => Need::Write,
        "script" if sub("load") || sub("delete") => Need::Admin,
        "semantic" if sub("store") || sub("clear") => Need::Write,
        "memory" if sub("pin") || sub("unpin") => Need::Admin,
        // Compacting only rewrites one's own file: allowed at home.
        "compact" => Need::Owner,
        "compress" | "config" | "create" | "drop" | "grant" | "revoke" | "users" | "audit" | "replica" | "failover"
        | "bench" | "shutdown" | "encryption" | "backup" | "tune" => Need::Admin,
        "check" if sub("repair") => Need::Admin,
        "autocompact" if !sub("status") => Need::Admin,
        "workspace" if sub("create") || sub("remove") => Need::Admin,
        _ => Need::Read,
    }
}

/// The command as it should appear in the audit log: passwords masked.
fn redact(cmd: &str, t: &[Token], line: &str) -> String {
    let secret_follows = |i: usize| {
        i > 0
            && (t[i - 1].is_kw("by")
                || t[i - 1].is_kw("password")
                || (cmd == "config" && t[i - 1].text.to_ascii_lowercase().contains("password")))
    };
    if !(0..t.len()).any(secret_follows) {
        return line.to_string();
    }
    t.iter()
        .enumerate()
        .map(|(i, tok)| {
            if secret_follows(i) {
                "\"***\"".to_string()
            } else if tok.is_quoted() {
                format!("\"{}\"", tok.text)
            } else {
                tok.text.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn doc_of(key: &str, value: Value) -> crate::error::Result<Json> {
    match value {
        Value::Doc(d) => Ok(d),
        other => {
            Err(KivaError::Invalid(format!("TypeError: '{key}' is a [{}], not an object or array.", other.ty().name())))
        }
    }
}
