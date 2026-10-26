// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The query language: conditions combined with `and`, `or`, `not`, the
//! aggregates computed over the documents they select, and what the
//! language deliberately leaves out.
//!
//! A query reads **one key pattern** and keeps the documents whose fields
//! satisfy an [`Expr`]: comparisons `<field> <op> <value>` joined by `and`,
//! `or` and `not`, with parentheses. The engine answers it from the
//! secondary indexes when it can (one index, or several intersected for
//! `and` and merged for `or`), and otherwise reads the documents under the
//! pattern's prefix once; either way every candidate is tested against the
//! whole expression, so an index only ever narrows the reading.
//!
//! An [`Aggregation`] then folds the documents found: `count`, `sum`,
//! `avg`, `min`, `max`, optionally per value of one field (`group by`), in
//! one pass and without keeping the documents.
//!
//! Joins, sorting pipelines (`order by`, `distinct`, `having`) and queries
//! spanning several servers are not part of the language. This module
//! recognises them, in the shell's syntax and in structured requests, and
//! refuses them with what to do instead, rather than with a generic syntax
//! error.

use crate::engine::{compare_json, CmpOp, Cond, MAX_IN_VALUES};
use crate::jsonpath::{self, Seg};
use serde_json::{Map, Value as Json};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

/// Comparisons one query may hold.
pub const MAX_CONDITIONS: usize = 64;
/// Nesting of `and` / `or` / `not` one query may hold.
pub const MAX_DEPTH: usize = 16;
/// Groups one aggregation may build; beyond, it is refused rather than
/// letting one query fill the memory.
pub const MAX_GROUPS: usize = 100_000;

/// What a query tried to do that the language does not offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// Conditions combined by other means than `and` / `or` / `not`.
    Combination,
    /// Two key patterns, or a value taken from other documents.
    Join,
    /// `order by`, `distinct`, `having`...
    Pipeline,
    /// A query sent to several servers or shards.
    Distributed,
    /// `between`, `contains`...: not one of the comparisons.
    Operator,
}

impl Unsupported {
    pub fn message(self) -> &'static str {
        match self {
            Unsupported::Combination => {
                "conditions are combined with the words and, or, not and parentheses: \
                 get where user:*.age > 18 and (role = \"admin\" or role = \"dev\"). In a structured request, \
                 they go in \"where\": {\"and\": [...]}, {\"or\": [...]}, {\"not\": {...}}."
            }
            Unsupported::Join => {
                "joins are not supported: a query reads one key pattern and compares with a literal value. \
                 Keep related data in the same document, or read the related keys by name (get a and b)."
            }
            Unsupported::Pipeline => {
                "sorting pipelines (order by, distinct, having...) are not supported: the aggregates are \
                 count, sum, avg, min and max, with 'group by <field>'; 'range' and 'keys' return keys in order."
            }
            Unsupported::Distributed => {
                "distributed queries are not supported: one KivaDB server holds all of a workspace's data \
                 (replicas hold copies, see 'replica status'), so a query never spans several servers."
            }
            Unsupported::Operator => {
                "the comparison operators are = != > >= < <= with one value, in [...] with a list of \
                 values, like and ilike with a text pattern (% for any text, _ for one character); for a \
                 range, join two comparisons with 'and'."
            }
        }
    }
}

/// The construct a word of a query belongs to, if it is one the language
/// leaves out.
pub fn unsupported_word(word: &str) -> Option<Unsupported> {
    if word.starts_with('@') {
        return Some(Unsupported::Distributed);
    }
    Some(match word.to_ascii_lowercase().as_str() {
        "&&" | "||" | "!" | "xor" => Unsupported::Combination,
        "join" | "inner" | "outer" | "left" | "right" | "cross" | "natural" | "lookup" | "on" | "using" => {
            Unsupported::Join
        }
        "group" | "order" | "sort" | "by" | "having" | "distinct" | "union" | "select" | "pipeline" | "aggregate"
        | "offset" => Unsupported::Pipeline,
        "shard" | "shards" | "node" | "nodes" | "cluster" | "across" | "partition" | "distributed" => {
            Unsupported::Distributed
        }
        "between" | "contains" | "matches" | "regexp" | "rlike" | "is" => Unsupported::Operator,
        _ => return None,
    })
}

/// Fields of a structured request that ask for something the language
/// leaves out.
pub fn unsupported_request_field(name: &str) -> Option<Unsupported> {
    Some(match name.to_ascii_lowercase().as_str() {
        "and" | "or" | "not" | "conditions" | "filters" | "filter" => Unsupported::Combination,
        "join" | "joins" | "lookup" | "with" => Unsupported::Join,
        "group" | "groupby" | "order" | "order_by" | "orderby" | "sort" | "pipeline" | "distinct" | "having" => {
            Unsupported::Pipeline
        }
        "shard" | "shards" | "nodes" | "cluster" | "distributed" => Unsupported::Distributed,
        _ => return None,
    })
}

/// Refuses a structured query that carries any of the fields above.
pub fn check_request(req: &Json) -> Result<(), String> {
    if let Some(obj) = req.as_object() {
        if let Some(u) = obj.keys().find_map(|k| unsupported_request_field(k)) {
            return Err(u.message().to_string());
        }
    }
    Ok(())
}

/// A field of the condition: a path, without wildcards.
pub fn check_field(field: &str) -> Result<(), String> {
    if field.contains(['*', '?']) {
        return Err(format!(
            "the field '{field}' contains a wildcard: wildcards go in the key pattern, before the field: {}",
            Unsupported::Join.message()
        ));
    }
    jsonpath::parse_path(field)?;
    Ok(())
}

/// The value of the condition: one string, number, boolean or null; for
/// `in`, a list of them; for `like` and `ilike`, a text pattern.
pub fn check_value(op: CmpOp, value: &Json) -> Result<(), String> {
    let scalar = |v: &Json| !matches!(v, Json::Array(_) | Json::Object(_));
    match op {
        CmpOp::In => match value.as_array() {
            Some(list) if list.is_empty() => Err("'in' needs at least one value: in [\"Goma\", \"Bukavu\"]".into()),
            Some(list) if list.len() > MAX_IN_VALUES => {
                Err(format!("an 'in' list holds at most {MAX_IN_VALUES} values, this one has {}", list.len()))
            }
            Some(list) if list.iter().all(scalar) => Ok(()),
            Some(_) => Err("the values of an 'in' list are strings, numbers, booleans or null".into()),
            None => Err(format!("'in' takes a list of values, such as in [\"Goma\", \"Bukavu\"], not {value}")),
        },
        CmpOp::Like | CmpOp::ILike if value.is_string() => Ok(()),
        CmpOp::Like | CmpOp::ILike => Err(format!(
            "'{}' takes a quoted text pattern, such as \"Kin%\" (% for any text, _ for one character), not {value}",
            op.symbol()
        )),
        _ if scalar(value) => Ok(()),
        _ => Err(format!(
            "a condition compares with one string, number, boolean or null, not with an array or an object: {}",
            Unsupported::Operator.message()
        )),
    }
}

/// Whether an unquoted value names a field of other documents
/// (`order:*.user`, `user:2.age`): comparing with it would be a join.
pub fn is_field_reference(text: &str) -> bool {
    match text.find('.') {
        Some(dot) => {
            let (target, field) = (&text[..dot], &text[dot + 1..]);
            target.contains([':', '*', '?']) && !field.is_empty()
        }
        None => false,
    }
}

/// One comparison, with its field path parsed once.
#[derive(Clone, Debug)]
pub struct Test {
    pub cond: Cond,
    path: Vec<Seg>,
}

/// The condition of a query: comparisons joined by `and`, `or`, `not`.
#[derive(Clone, Debug)]
pub enum Expr {
    Test(Test),
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
}

impl Expr {
    pub fn test(cond: Cond) -> Result<Expr, String> {
        check_field(&cond.field)?;
        check_value(cond.op, &cond.value)?;
        let path = jsonpath::parse_path(&cond.field)?;
        Ok(Expr::Test(Test { cond, path }))
    }

    /// `and` of the parts; nested `and`s are flattened.
    pub fn and(parts: Vec<Expr>) -> Expr {
        Self::join(parts, true)
    }

    pub fn or(parts: Vec<Expr>) -> Expr {
        Self::join(parts, false)
    }

    fn join(parts: Vec<Expr>, and: bool) -> Expr {
        let mut flat = Vec::with_capacity(parts.len());
        for p in parts {
            match p {
                Expr::And(v) if and => flat.extend(v),
                Expr::Or(v) if !and => flat.extend(v),
                p => flat.push(p),
            }
        }
        if flat.len() == 1 {
            return flat.pop().expect("one part");
        }
        if and {
            Expr::And(flat)
        } else {
            Expr::Or(flat)
        }
    }

    pub fn negate(e: Expr) -> Expr {
        match e {
            Expr::Not(inner) => *inner,
            e => Expr::Not(Box::new(e)),
        }
    }

    /// Whether `doc` satisfies the expression. A comparison on a field the
    /// document lacks is false, so `not` of it is true.
    pub fn matches(&self, doc: &Json) -> bool {
        match self {
            Expr::Test(t) => jsonpath::get(doc, &t.path).is_some_and(|v| compare_json(v, t.cond.op, &t.cond.value)),
            Expr::And(v) => v.iter().all(|e| e.matches(doc)),
            Expr::Or(v) => v.iter().any(|e| e.matches(doc)),
            Expr::Not(e) => !e.matches(doc),
        }
    }

    /// Every comparison, left to right.
    pub fn conditions(&self) -> Vec<&Cond> {
        let mut out = Vec::new();
        self.walk(false, &mut |c, _| out.push(c));
        out
    }

    /// The comparisons not under a `not`: those an index could serve.
    pub fn positive_conditions(&self) -> Vec<&Cond> {
        let mut out = Vec::new();
        self.walk(false, &mut |c, negated| {
            if !negated {
                out.push(c)
            }
        });
        out
    }

    fn walk<'a>(&'a self, negated: bool, f: &mut impl FnMut(&'a Cond, bool)) {
        match self {
            Expr::Test(t) => f(&t.cond, negated),
            Expr::And(v) | Expr::Or(v) => v.iter().for_each(|e| e.walk(negated, f)),
            Expr::Not(e) => e.walk(!negated, f),
        }
    }

    fn depth(&self) -> usize {
        match self {
            Expr::Test(_) => 0,
            Expr::And(v) | Expr::Or(v) => 1 + v.iter().map(Expr::depth).max().unwrap_or(0),
            Expr::Not(e) => 1 + e.depth(),
        }
    }

    /// Refuses an expression too large to be a query.
    pub fn check_size(&self) -> Result<(), String> {
        let n = self.conditions().len();
        if n > MAX_CONDITIONS {
            return Err(format!("a query holds at most {MAX_CONDITIONS} comparisons, this one has {n}"));
        }
        if self.depth() > MAX_DEPTH {
            return Err(format!("a query nests and / or / not at most {MAX_DEPTH} levels deep"));
        }
        Ok(())
    }

    /// The condition of a structured request, on the documents of `pattern`:
    /// - `{"field": "age", "cmp": ">", "value": 18}` (`cmp` defaults to `=`);
    /// - `{"and": [...]}`, `{"or": [...]}`, `{"not": {...}}`;
    /// - `{"role": "dev", "city": "Goma"}`: equalities joined by `and`;
    /// - `[...]`: its items joined by `and`.
    pub fn from_json(j: &Json, pattern: &str) -> Result<Expr, String> {
        let e = Self::from_json_at(j, pattern, 0)?;
        e.check_size()?;
        Ok(e)
    }

    fn from_json_at(j: &Json, pattern: &str, depth: usize) -> Result<Expr, String> {
        const SHAPE: &str = "a condition is {\"field\": \"age\", \"cmp\": \">\", \"value\": 18}, {\"role\": \"dev\"}, \
                             {\"and\": [...]}, {\"or\": [...]} or {\"not\": {...}}";
        if depth > MAX_DEPTH {
            return Err(format!("a query nests and / or / not at most {MAX_DEPTH} levels deep"));
        }
        let list = |v: &Json, name: &str| -> Result<Vec<Expr>, String> {
            let items = v.as_array().filter(|a| !a.is_empty()).ok_or(format!("\"{name}\" takes a non-empty array"))?;
            items.iter().map(|i| Self::from_json_at(i, pattern, depth + 1)).collect()
        };
        let leaf = |field: &str, cmp: CmpOp, value: Json| {
            let field = field.strip_prefix(pattern).and_then(|f| f.strip_prefix('.')).unwrap_or(field);
            Self::test(Cond { pattern: pattern.to_string(), field: field.to_string(), op: cmp, value })
        };
        match j {
            Json::Array(_) => Ok(Expr::and(list(j, "and")?)),
            Json::Object(m) => {
                let logic = ["and", "or", "not"].into_iter().find(|k| m.contains_key(*k));
                if let Some(k) = logic {
                    if m.len() != 1 {
                        return Err(format!("\"{k}\" must be alone in its object: {SHAPE}"));
                    }
                    let v = &m[k];
                    return match k {
                        "and" => Ok(Expr::and(list(v, k)?)),
                        "or" => Ok(Expr::or(list(v, k)?)),
                        _ => Ok(Expr::negate(Self::from_json_at(v, pattern, depth + 1)?)),
                    };
                }
                if let Some(f) = m.get("field") {
                    if let Some(extra) = m.keys().find(|k| !["field", "cmp", "value"].contains(&k.as_str())) {
                        return Err(format!("unknown key \"{extra}\" in a condition: {SHAPE}"));
                    }
                    let field = f.as_str().ok_or("\"field\" must be a string")?;
                    let cmp = CmpOp::parse(m.get("cmp").and_then(Json::as_str).unwrap_or("="))
                        .ok_or(format!("cmp must be one of {}", CmpOp::LIST))?;
                    let value = m.get("value").ok_or(format!("the condition on {field} needs a \"value\""))?;
                    return leaf(field, cmp, value.clone());
                }
                if m.is_empty() {
                    return Err(SHAPE.into());
                }
                // `{"city": ["Goma", "Bukavu"]}`: one of the values.
                let op = |v: &Json| if v.is_array() { CmpOp::In } else { CmpOp::Eq };
                Ok(Expr::and(m.iter().map(|(k, v)| leaf(k, op(v), v.clone())).collect::<Result<_, _>>()?))
            }
            _ => Err(SHAPE.into()),
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let part = |e: &Expr, f: &mut fmt::Formatter<'_>, wrap: bool| {
            if wrap {
                write!(f, "({e})")
            } else {
                write!(f, "{e}")
            }
        };
        match self {
            Expr::Test(t) => write!(f, "{} {} {}", t.cond.field, t.cond.op.symbol(), t.cond.value),
            Expr::And(v) | Expr::Or(v) => {
                let and = matches!(self, Expr::And(_));
                for (i, e) in v.iter().enumerate() {
                    if i > 0 {
                        f.write_str(if and { " and " } else { " or " })?;
                    }
                    part(e, f, and && matches!(e, Expr::Or(_)))?;
                }
                Ok(())
            }
            Expr::Not(e) => {
                f.write_str("not ")?;
                part(e, f, !matches!(**e, Expr::Test(_)))
            }
        }
    }
}

/// A query: the documents of `pattern` that satisfy `expr` (every
/// document of the pattern when there is none).
#[derive(Clone, Debug)]
pub struct Query {
    pub pattern: String,
    pub expr: Option<Expr>,
}

impl Query {
    /// The query of one comparison.
    pub fn single(cond: Cond) -> Result<Query, String> {
        Ok(Query { pattern: cond.pattern.clone(), expr: Some(Expr::test(cond)?) })
    }

    pub fn matches(&self, doc: &Json) -> bool {
        self.expr.as_ref().is_none_or(|e| e.matches(doc))
    }

    /// `user:* where age > 18 and role = "dev"`.
    pub fn describe(&self) -> String {
        match &self.expr {
            Some(e) => format!("{} where {e}", self.pattern),
            None => format!("{} (every document)", self.pattern),
        }
    }
}

/// An aggregate function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl Func {
    pub fn parse(s: &str) -> Option<Func> {
        Some(match s.to_ascii_lowercase().as_str() {
            "count" => Func::Count,
            "sum" => Func::Sum,
            "avg" | "average" | "mean" => Func::Avg,
            "min" => Func::Min,
            "max" => Func::Max,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Func::Count => "count",
            Func::Sum => "sum",
            Func::Avg => "avg",
            Func::Min => "min",
            Func::Max => "max",
        }
    }
}

/// One aggregate to compute, under a name.
#[derive(Clone, Debug)]
pub struct Agg {
    pub name: String,
    pub func: Func,
    /// The field it reads; `count` without one counts the documents.
    pub field: Option<String>,
    path: Vec<Seg>,
}

impl Agg {
    pub fn new(name: &str, func: Func, field: Option<&str>) -> Result<Agg, String> {
        let path = match field {
            Some(f) => {
                check_field(f)?;
                jsonpath::parse_path(f)?
            }
            None if func == Func::Count => Vec::new(),
            None => return Err(format!("{} needs a field", func.name())),
        };
        Ok(Agg { name: name.to_string(), func, field: field.map(String::from), path })
    }
}

/// A value of the `group by` field, ordered: null (or missing), booleans,
/// numbers, text, then arrays and objects by their JSON text.
#[derive(Clone, Debug)]
pub struct GroupKey(pub Json);

fn rank(v: &Json) -> u8 {
    match v {
        Json::Null => 0,
        Json::Bool(_) => 1,
        Json::Number(_) => 2,
        Json::String(_) => 3,
        _ => 4,
    }
}

impl Ord for GroupKey {
    fn cmp(&self, other: &Self) -> Ordering {
        let (a, b) = (&self.0, &other.0);
        rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
            (Json::Bool(x), Json::Bool(y)) => x.cmp(y),
            (Json::Number(x), Json::Number(y)) => x.as_f64().unwrap_or(0.0).total_cmp(&y.as_f64().unwrap_or(0.0)),
            (Json::String(x), Json::String(y)) => x.cmp(y),
            (Json::Null, Json::Null) => Ordering::Equal,
            _ => a.to_string().cmp(&b.to_string()),
        })
    }
}

impl PartialOrd for GroupKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for GroupKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for GroupKey {}

#[derive(Clone, Debug, Default)]
struct Acc {
    /// Values taken into account.
    n: u64,
    sum: f64,
    best: Option<Json>,
}

#[derive(Clone, Debug)]
struct Bucket {
    matches: usize,
    accs: Vec<Acc>,
}

/// Aggregates folded over documents, one at a time.
#[derive(Clone, Debug)]
pub struct Aggregation {
    aggs: Vec<Agg>,
    group_by: Option<(String, Vec<Seg>)>,
    groups: BTreeMap<GroupKey, Bucket>,
    matches: usize,
}

/// The result of one group (the only one without `group by`).
#[derive(Clone, Debug, PartialEq)]
pub struct GroupResult {
    /// The value of the `group by` field; `None` without `group by`.
    pub key: Option<Json>,
    pub matches: usize,
    /// `(name, value, values taken into account)` per aggregate.
    pub values: Vec<(String, Json, u64)>,
}

/// What an aggregation found.
#[derive(Clone, Debug, PartialEq)]
pub struct Aggregated {
    pub matches: usize,
    pub groups: Vec<GroupResult>,
}

/// A number as JSON: an integer when it is one.
pub fn num(f: f64) -> Json {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 {
        Json::from(f as i64)
    } else {
        serde_json::Number::from_f64(f).map(Json::Number).unwrap_or(Json::Null)
    }
}

impl Aggregation {
    pub fn new(aggs: Vec<Agg>, group_by: Option<&str>) -> Result<Aggregation, String> {
        if aggs.is_empty() {
            return Err("nothing to compute: give count, sum, avg, min or max".into());
        }
        let group_by = match group_by {
            Some(f) => {
                check_field(f)?;
                Some((f.to_string(), jsonpath::parse_path(f)?))
            }
            None => None,
        };
        let mut a = Aggregation { aggs, group_by, groups: BTreeMap::new(), matches: 0 };
        if a.group_by.is_none() {
            let empty = a.empty_bucket();
            a.groups.insert(GroupKey(Json::Null), empty);
        }
        Ok(a)
    }

    fn empty_bucket(&self) -> Bucket {
        Bucket { matches: 0, accs: vec![Acc::default(); self.aggs.len()] }
    }

    /// Folds one document in.
    pub fn add(&mut self, doc: &Json) -> Result<(), String> {
        let key = match &self.group_by {
            Some((_, path)) => GroupKey(jsonpath::get(doc, path).cloned().unwrap_or(Json::Null)),
            None => GroupKey(Json::Null),
        };
        if !self.groups.contains_key(&key) {
            if self.groups.len() >= MAX_GROUPS {
                let field = self.group_by.as_ref().map(|g| g.0.as_str()).unwrap_or("");
                return Err(format!(
                    "more than {MAX_GROUPS} distinct values of {field}: group by a field with fewer values"
                ));
            }
            let empty = self.empty_bucket();
            self.groups.insert(key.clone(), empty);
        }
        self.matches += 1;
        let bucket = self.groups.get_mut(&key).expect("inserted above");
        bucket.matches += 1;
        for (agg, acc) in self.aggs.iter().zip(bucket.accs.iter_mut()) {
            if agg.field.is_none() {
                acc.n += 1;
                continue;
            }
            let Some(v) = jsonpath::get(doc, &agg.path) else {
                continue;
            };
            match agg.func {
                Func::Count => acc.n += 1,
                Func::Sum | Func::Avg => {
                    if let Some(x) = v.as_f64() {
                        acc.n += 1;
                        acc.sum += x;
                    }
                }
                Func::Min | Func::Max => {
                    if !(v.is_number() || v.is_string()) {
                        continue;
                    }
                    acc.n += 1;
                    let better = acc.best.as_ref().is_none_or(|b| {
                        let o = GroupKey(v.clone()).cmp(&GroupKey(b.clone()));
                        if agg.func == Func::Min {
                            o == Ordering::Less
                        } else {
                            o == Ordering::Greater
                        }
                    });
                    if better {
                        acc.best = Some(v.clone());
                    }
                }
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Aggregated {
        let grouped = self.group_by.is_some();
        let groups = self
            .groups
            .into_iter()
            .map(|(key, b)| GroupResult {
                key: grouped.then_some(key.0),
                matches: b.matches,
                values: self
                    .aggs
                    .iter()
                    .zip(b.accs)
                    .map(|(agg, acc)| {
                        let v = match agg.func {
                            Func::Count => Json::from(acc.n),
                            Func::Sum => num(acc.sum),
                            Func::Avg if acc.n == 0 => Json::Null,
                            Func::Avg => num(acc.sum / acc.n as f64),
                            Func::Min | Func::Max => acc.best.unwrap_or(Json::Null),
                        };
                        (agg.name.clone(), v, acc.n)
                    })
                    .collect(),
            })
            .collect();
        Aggregated { matches: self.matches, groups }
    }

    /// The aggregates of a structured request: `{"n": "count", "mean":
    /// {"avg": "age"}}`, or a list `["count", {"avg": "age"}]` named
    /// `count`, `avg_age`...
    pub fn functions_from_json(j: &Json) -> Result<Vec<Agg>, String> {
        const SHAPE: &str =
            "\"compute\" is {\"name\": \"count\" | {\"sum\"|\"avg\"|\"min\"|\"max\"|\"count\": \"<field>\"}} \
                             or a list of those values";
        let one = |name: Option<&str>, v: &Json| -> Result<Agg, String> {
            match v {
                Json::String(s) => {
                    let func = Func::parse(s).ok_or(SHAPE)?;
                    Agg::new(name.unwrap_or(func.name()), func, None)
                }
                Json::Object(m) if m.len() == 1 => {
                    let (f, field) = m.iter().next().expect("one entry");
                    let func = Func::parse(f).ok_or(SHAPE)?;
                    let field = field.as_str().ok_or(SHAPE)?;
                    let auto = format!("{}_{}", func.name(), field.replace(['.', '[', ']'], "_"));
                    Agg::new(name.unwrap_or(&auto), func, Some(field))
                }
                _ => Err(SHAPE.into()),
            }
        };
        let aggs: Vec<Agg> = match j {
            Json::Null => vec![Agg::new("count", Func::Count, None)?],
            Json::Object(m) => m.iter().map(|(k, v)| one(Some(k), v)).collect::<Result<_, _>>()?,
            Json::Array(a) => a.iter().map(|v| one(None, v)).collect::<Result<_, _>>()?,
            _ => return Err(SHAPE.into()),
        };
        if aggs.len() > 32 {
            return Err("at most 32 aggregates per request".into());
        }
        Ok(aggs)
    }
}

/// An aggregation result as JSON.
pub fn aggregated_json(a: &Aggregated, grouped: bool) -> Json {
    let values = |g: &GroupResult| {
        let mut m = Map::new();
        for (name, v, _) in &g.values {
            m.insert(name.clone(), v.clone());
        }
        Json::Object(m)
    };
    if grouped {
        let groups: Vec<Json> = a
            .groups
            .iter()
            .map(|g| serde_json::json!({ "key": g.key, "matches": g.matches, "values": values(g) }))
            .collect();
        serde_json::json!({ "matches": a.matches, "groups": groups })
    } else {
        let g = &a.groups[0];
        serde_json::json!({ "matches": a.matches, "values": values(g) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cond(field: &str, op: &str, value: Json) -> Expr {
        Expr::test(Cond { pattern: "user:*".into(), field: field.into(), op: CmpOp::parse(op).unwrap(), value })
            .unwrap()
    }

    #[test]
    fn recognises_what_the_language_leaves_out() {
        assert_eq!(unsupported_word("&&"), Some(Unsupported::Combination));
        assert_eq!(unsupported_word("and"), None);
        assert_eq!(unsupported_word("join"), Some(Unsupported::Join));
        assert_eq!(unsupported_word("order"), Some(Unsupported::Pipeline));
        assert_eq!(unsupported_word("@node2"), Some(Unsupported::Distributed));
        assert_eq!(unsupported_word("between"), Some(Unsupported::Operator));
        assert_eq!(unsupported_word("in"), None);
        assert_eq!(unsupported_word("ilike"), None);
        assert_eq!(unsupported_word("limit"), None);
        assert_eq!(unsupported_word("return"), None);

        assert!(check_request(&json!({"op": "query", "target": "a:*.b", "join": "c:*"})).is_err());
        assert!(check_request(&json!({"op": "query", "target": "a:*.b", "order_by": "b"})).is_err());
        assert!(check_request(&json!({"op": "query", "target": "a:*.b", "and": []})).is_err());
        assert!(check_request(&json!({"op": "query", "target": "a:*", "where": {}, "group_by": "b"})).is_ok());

        assert!(check_field("address.city").is_ok());
        assert!(check_field("tags[*]").is_err());
        assert!(check_field("a..b").is_err());
        assert!(check_value(CmpOp::Eq, &json!("x")).is_ok());
        assert!(check_value(CmpOp::Eq, &json!(["a", "b"])).is_err());
        assert!(check_value(CmpOp::In, &json!(["a", 2, null])).is_ok());
        assert!(check_value(CmpOp::In, &json!([])).is_err());
        assert!(check_value(CmpOp::In, &json!([["a"]])).is_err());
        assert!(check_value(CmpOp::In, &json!("a")).is_err());
        assert!(check_value(CmpOp::Like, &json!("Kin%")).is_ok());
        assert!(check_value(CmpOp::ILike, &json!(3)).is_err());

        assert!(is_field_reference("order:*.user"));
        assert!(is_field_reference("user:2.age"));
        assert!(!is_field_reference("3.5"));
        assert!(!is_field_reference("admin"));
    }

    #[test]
    fn like_patterns() {
        use crate::engine::{like_match, like_prefix};
        for (text, pattern, fold, want) in [
            ("Kinshasa", "Kin%", false, true),
            ("Kinshasa", "kin%", false, false),
            ("Kinshasa", "kin%", true, true),
            ("Kinshasa", "%sha%", false, true),
            ("Kinshasa", "K_nshasa", false, true),
            ("Kinshasa", "K_shasa", false, false),
            ("", "%", false, true),
            ("", "_", false, false),
            ("abcabc", "%abc", false, true),
            ("aaab", "%a%b", false, true),
            ("aaab", "%a%c", false, false),
            ("50% off", "50\\%%", false, true),
            ("50 off", "50\\%%", false, false),
            ("a_b", "a\\_b", false, true),
            ("axb", "a\\_b", false, false),
            ("ÉLISABETHVILLE", "élisabeth%", true, true),
            ("Mbandaka", "%ka", false, true),
        ] {
            assert_eq!(like_match(text, pattern, fold), want, "{text} like {pattern} (fold {fold})");
        }
        assert_eq!(like_prefix("Kin%sha"), "Kin");
        assert_eq!(like_prefix("50\\%_"), "50%");
        assert_eq!(like_prefix("%a"), "");
        let e = Expr::and(vec![cond("city", "in", json!(["Goma", "Bukavu"])), cond("name", "ilike", json!("a%"))]);
        assert_eq!(e.to_string(), r#"city in ["Goma","Bukavu"] and name ilike "a%""#);
        assert!(e.matches(&json!({"city": "Goma", "name": "Amani"})));
        assert!(!e.matches(&json!({"city": "Uvira", "name": "Amani"})));
        let n = Expr::from_json(&json!({"not": {"city": ["Goma"]}}), "u:*").unwrap();
        assert_eq!(n.to_string(), r#"not city in ["Goma"]"#);
    }

    #[test]
    fn expressions_combine_and_print_back() {
        let e = Expr::and(vec![
            cond("age", ">", json!(18)),
            Expr::or(vec![cond("role", "=", json!("admin")), cond("role", "=", json!("dev"))]),
            Expr::negate(cond("city", "=", json!("Goma"))),
        ]);
        assert_eq!(e.to_string(), r#"age > 18 and (role = "admin" or role = "dev") and not city = "Goma""#);
        assert!(e.matches(&json!({"age": 30, "role": "dev", "city": "Kinshasa"})));
        assert!(!e.matches(&json!({"age": 30, "role": "ops", "city": "Kinshasa"})));
        assert!(!e.matches(&json!({"age": 30, "role": "dev", "city": "Goma"})));
        // A missing field fails its comparison, so `not` of it holds.
        assert!(e.matches(&json!({"age": 30, "role": "admin"})));
        assert!(!e.matches(&json!({"role": "admin"})));
        assert_eq!(e.conditions().len(), 4);
        assert_eq!(e.positive_conditions().len(), 3);
        assert_eq!(Expr::negate(Expr::negate(cond("a", "=", json!(1)))).to_string(), "a = 1");
        // Nested and / or flatten.
        let flat = Expr::and(vec![Expr::and(vec![cond("a", "=", json!(1)), cond("b", "=", json!(2))])]);
        assert!(matches!(&flat, Expr::And(v) if v.len() == 2));
    }

    #[test]
    fn structured_conditions() {
        let j = json!({"and": [{"field": "age", "cmp": ">=", "value": 18},
                               {"or": [{"role": "dev"}, {"field": "user:*.role", "value": "admin"}]},
                               {"not": {"city": "Goma", "active": false}}]});
        let e = Expr::from_json(&j, "user:*").unwrap();
        assert_eq!(
            e.to_string(),
            r#"age >= 18 and (role = "dev" or role = "admin") and not (city = "Goma" and active = false)"#
        );
        assert!(Expr::from_json(&json!([{"a": 1}, {"b": 2}]), "x:*").is_ok());
        for bad in [
            json!({"and": []}),
            json!({"and": [{"a": 1}], "b": 2}),
            json!({"field": "a"}),
            json!({"field": "a", "value": 1, "op": "="}),
            json!({"field": "a", "cmp": "~", "value": 1}),
            json!({"field": "a", "value": [1, 2]}),
            json!({"field": "a*", "value": 1}),
            json!(3),
            json!({}),
        ] {
            assert!(Expr::from_json(&bad, "x:*").is_err(), "{bad}");
        }
        let mut deep = json!({"a": 1});
        for _ in 0..=MAX_DEPTH {
            deep = json!({ "not": deep });
        }
        assert!(Expr::from_json(&deep, "x:*").unwrap_err().contains("levels deep"));
        let many: Vec<Json> = (0..=MAX_CONDITIONS).map(|i| json!({ "f": i })).collect();
        assert!(Expr::from_json(&json!({ "or": many }), "x:*").unwrap_err().contains("at most"));
    }

    #[test]
    fn aggregations_fold_documents_per_group() {
        let docs = [
            json!({"city": "Goma", "age": 30, "name": "Bob"}),
            json!({"city": "Kinshasa", "age": 41, "name": "Carol"}),
            json!({"city": "Kinshasa", "age": "unknown", "name": "Alice"}),
            json!({"age": 25}),
        ];
        let aggs = vec![
            Agg::new("count", Func::Count, None).unwrap(),
            Agg::new("avg", Func::Avg, Some("age")).unwrap(),
            Agg::new("sum", Func::Sum, Some("age")).unwrap(),
            Agg::new("min", Func::Min, Some("name")).unwrap(),
            Agg::new("max", Func::Max, Some("age")).unwrap(),
        ];
        let mut a = Aggregation::new(aggs.clone(), Some("city")).unwrap();
        docs.iter().for_each(|d| a.add(d).unwrap());
        let r = a.finish();
        assert_eq!(r.matches, 4);
        let keys: Vec<Json> = r.groups.iter().map(|g| g.key.clone().unwrap()).collect();
        assert_eq!(keys, [json!(null), json!("Goma"), json!("Kinshasa")], "missing first, then in order");
        let kin = &r.groups[2];
        assert_eq!(kin.matches, 2);
        let v: Vec<&Json> = kin.values.iter().map(|x| &x.1).collect();
        assert_eq!(v, [&json!(2), &json!(41), &json!(41), &json!("Alice"), &json!("unknown")]);
        assert_eq!(kin.values[1].2, 1, "one numeric age");

        let mut all = Aggregation::new(aggs, None).unwrap();
        docs.iter().for_each(|d| all.add(d).unwrap());
        let r = all.finish();
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].values[1].1, json!(32), "(30 + 41 + 25) / 3");
        let empty = Aggregation::new(vec![Agg::new("avg", Func::Avg, Some("x")).unwrap()], None).unwrap().finish();
        assert_eq!((empty.matches, &empty.groups[0].values[0].1), (0, &Json::Null));
        assert!(Agg::new("s", Func::Sum, None).is_err());
        assert_eq!(num(2.5), json!(2.5));

        let f = Aggregation::functions_from_json(&json!({"n": "count", "m": {"avg": "age"}})).unwrap();
        assert_eq!((f[0].name.as_str(), f[1].func), ("n", Func::Avg));
        let f = Aggregation::functions_from_json(&json!(["count", {"max": "address.zip"}])).unwrap();
        assert_eq!(f[1].name, "max_address_zip");
        assert!(Aggregation::functions_from_json(&json!({"n": "median"})).is_err());
    }
}
