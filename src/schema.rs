// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Optional schemas: the rules the documents of a key pattern follow.
//!
//! A schema is written in JSON, one rule per field:
//!
//! ```text
//! schema set user: {"name": {"type": "string", "required": true, "maxLength": 80},
//!                   "email": {"type": "string", "required": true, "unique": true},
//!                   "created_at": {"type": "datetime", "default": "now()"}}
//! ```
//!
//! The engine checks every document written under the pattern, after filling
//! in the defaults, before anything reaches the disk. Fields the schema does
//! not name are free. A rule is data, never code: the only dynamic defaults
//! are `now()`, `today()` and `current_time`, and a `pattern` is a regular
//! expression of the `regex` crate, which runs in linear time, with a size
//! bound.

use crate::doc::{self, Doc, Members};
use crate::view::{DocView, Leaf};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde_json::{Map, Value as Json};

/// Longest `pattern` accepted, and the memory its compiled form may take.
const MAX_PATTERN_LEN: usize = 1000;
const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// How deep `properties` and `items` may nest.
const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    String,
    Number,
    Integer,
    Boolean,
    Null,
    Array,
    Object,
    Date,
    DateTime,
    Time,
    Int,
    Float,
    Decimal,
    Any,
}

impl FieldType {
    fn parse(s: &str) -> Option<FieldType> {
        Some(match s {
            "string" => FieldType::String,
            "number" => FieldType::Number,
            "integer" => FieldType::Integer,
            "boolean" => FieldType::Boolean,
            "null" => FieldType::Null,
            "array" => FieldType::Array,
            "object" => FieldType::Object,
            "date" => FieldType::Date,
            "datetime" => FieldType::DateTime,
            "time" => FieldType::Time,
            "int" => FieldType::Int,
            "float" => FieldType::Float,
            "decimal" => FieldType::Decimal,
            "any" => FieldType::Any,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            FieldType::String => "string",
            FieldType::Number => "number",
            FieldType::Integer => "integer",
            FieldType::Boolean => "boolean",
            FieldType::Null => "null",
            FieldType::Array => "array",
            FieldType::Object => "object",
            FieldType::Date => "date",
            FieldType::DateTime => "datetime",
            FieldType::Time => "time",
            FieldType::Int => "int",
            FieldType::Float => "float",
            FieldType::Decimal => "decimal",
            FieldType::Any => "any",
        }
    }

    /// Whether `v` is of this type. Dates, datetimes and times are texts in a
    /// document: `2026-10-27`, ISO 8601 with its offset (`...Z`, `...+01:00`),
    /// `08:00:00`.
    fn accepts(self, v: &Json) -> bool {
        match self {
            FieldType::String => v.is_string(),
            FieldType::Number => v.is_number(),
            FieldType::Integer => v.as_f64().is_some_and(|n| n.fract() == 0.0),
            FieldType::Boolean => v.is_boolean(),
            FieldType::Null => v.is_null(),
            FieldType::Array => v.is_array(),
            FieldType::Object => v.is_object(),
            FieldType::Date => v.as_str().is_some_and(|s| crate::time::parse_date(s).is_ok()),
            FieldType::DateTime => v.as_str().is_some_and(is_instant),
            FieldType::Time => v.as_str().is_some_and(|s| crate::time::parse_time(s).is_ok()),
            FieldType::Int => v.is_i64(),
            FieldType::Float => v.is_f64(),
            // A JSON integer beyond i64 is read as a decimal (NATIVE_DOCUMENTS.md 15.4).
            FieldType::Decimal => v.is_u64() && !v.is_i64(),
            FieldType::Any => true,
        }
    }

    /// Whether a value of a document is of this type: a typed document's
    /// `int`, `float` and `decimal` are numbers, its dates, datetimes and
    /// times are of their types, as the ISO texts of JSON documents are.
    fn accepts_leaf(self, v: Leaf) -> bool {
        use FieldType as T;
        let d = match v {
            Leaf::Json(j) => return self.accepts(j),
            Leaf::Doc(d) => d,
        };
        match (self, d) {
            (T::Any, _) => true,
            (T::Number, Doc::Int(_) | Doc::Float(_) | Doc::Decimal(_)) => true,
            (T::Integer, Doc::Int(_)) => true,
            (T::Integer, Doc::Float(f)) => f.fract() == 0.0,
            (T::Integer, Doc::Decimal(x)) => x.fract().is_zero(),
            (T::Int, Doc::Int(_)) | (T::Float, Doc::Float(_)) | (T::Decimal, Doc::Decimal(_)) => true,
            (T::Date, Doc::Date(_)) | (T::DateTime, Doc::DateTime(_)) | (T::Time, Doc::Time(_)) => true,
            (_, Doc::Decimal(_) | Doc::Date(_) | Doc::DateTime(_) | Doc::Time(_)) => false,
            (T::Array, Doc::Array(_)) | (T::Object, Doc::Object(_)) => true,
            (_, Doc::Array(_) | Doc::Object(_)) => false,
            (ty, scalar) => ty.accepts(&doc::to_plain_json(scalar)),
        }
    }
}

/// An ISO 8601 moment that carries its offset: read the same whatever the zone.
fn is_instant(s: &str) -> bool {
    let s = s.trim();
    let tail = s.get(10..).unwrap_or("");
    let offset = s.ends_with('Z') || s.ends_with('z') || tail.contains('+') || tail.rfind('-').is_some();
    offset && crate::time::parse_datetime(s, &crate::time::Zone::Utc).is_ok()
}

#[derive(Clone, Debug)]
enum Fill {
    Value(Json),
    Now,
    Today,
    CurrentTime,
}

#[derive(Clone, Debug, Default)]
pub struct Rule {
    ty: Option<FieldType>,
    required: bool,
    nullable: bool,
    unique: bool,
    min: Option<f64>,
    max: Option<f64>,
    min_length: Option<usize>,
    max_length: Option<usize>,
    min_items: Option<usize>,
    max_items: Option<usize>,
    one_of: Option<Vec<Json>>,
    pattern: Option<Regex>,
    default: Option<Fill>,
    properties: Option<Fields>,
    items: Option<Box<Rule>>,
}

type Fields = Vec<(String, Rule)>;

#[derive(Clone, Debug)]
pub struct Schema {
    pub pattern: String,
    /// The JSON it was written with, kept as it is for `schema show` and the meta file.
    pub spec: Json,
    fields: Fields,
}

/// The moment a write happens: one for every default of the write, and of
/// every write of a transaction (its commit).
#[derive(Clone, Copy)]
pub struct Moment {
    pub now: DateTime<Utc>,
}

impl Moment {
    pub fn now() -> Moment {
        Moment { now: crate::time::now() }
    }
}

impl Schema {
    /// Reads a schema. Every rule is checked here, static defaults included,
    /// so that a write never meets a broken schema.
    pub fn parse(pattern: &str, spec: &Json) -> Result<Schema, String> {
        let fields = parse_fields(spec, "", 0)?;
        if fields.is_empty() {
            return Err("a schema names at least one field".into());
        }
        Ok(Schema { pattern: pattern.to_string(), spec: spec.clone(), fields })
    }

    /// The paths of the fields marked `unique` (`email`, `contact.email`).
    pub fn unique_fields(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_unique(&self.fields, "", &mut out);
        out
    }

    /// Fills in the defaults of `doc`, then checks it. On error, `doc` may
    /// hold some defaults: the caller writes nothing.
    pub fn conform(&self, doc: &mut Json, at: Moment) -> Result<(), String> {
        let Json::Object(map) = doc else {
            return Err(format!("a JSON object is expected, not {}", kind_of(doc)));
        };
        fill_and_check(&self.fields, map, "", at)
    }

    /// [`Schema::conform`] for a typed document: the defaults are added to
    /// it, and its decimals, dates and times are checked as such.
    pub fn conform_typed(&self, doc: &mut Doc, at: Moment) -> Result<(), String> {
        let Doc::Object(members) = doc else {
            return Err(format!("a JSON object is expected, not {}", doc.type_name()));
        };
        fill_and_check_typed(&self.fields, members, "", at)
    }

    /// Checks `doc` as it is, without defaults: whether a document already
    /// stored follows the schema.
    pub fn check(&self, doc: &Json) -> Result<(), String> {
        self.check_view(DocView::Json(doc))
    }

    /// [`Schema::check`], for a JSON or a typed document.
    pub fn check_view(&self, doc: DocView) -> Result<(), String> {
        match doc {
            DocView::Json(Json::Object(map)) => check_fields(&self.fields, &|n| map.get(n).map(Leaf::Json), ""),
            DocView::Typed(Doc::Object(m)) => check_fields(&self.fields, &|n| m.get(n).map(Leaf::Doc), ""),
            DocView::Json(j) => Err(format!("not a JSON object but {}", kind_of(j))),
            DocView::Typed(d) => Err(format!("not a JSON object but {}", d.type_name())),
        }
    }
}

fn kind_of(v: &Json) -> &'static str {
    match v {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::String(_) => "a string",
        Json::Array(_) => "an array",
        Json::Object(_) => "an object",
    }
}

fn collect_unique(fields: &Fields, prefix: &str, out: &mut Vec<String>) {
    for (name, r) in fields {
        let path = join(prefix, name);
        if r.unique {
            out.push(path.clone());
        }
        if let Some(p) = &r.properties {
            collect_unique(p, &path, out);
        }
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

const KNOWN_KEYS: &[&str] = &[
    "type",
    "required",
    "nullable",
    "unique",
    "min",
    "max",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "enum",
    "pattern",
    "default",
    "properties",
    "items",
];

fn parse_fields(spec: &Json, prefix: &str, depth: usize) -> Result<Fields, String> {
    let Json::Object(map) = spec else {
        let what = if prefix.is_empty() { "a schema".to_string() } else { format!("properties of {prefix}") };
        return Err(format!("{what} must be a JSON object: {{\"<field>\": {{\"type\": ...}}}}"));
    };
    if depth > MAX_DEPTH {
        return Err(format!("{prefix}: rules nest at most {MAX_DEPTH} levels deep"));
    }
    let mut fields = Vec::with_capacity(map.len());
    for (name, rule) in map {
        if name.is_empty() {
            return Err("a field name cannot be empty".into());
        }
        let path = join(prefix, name);
        fields.push((name.clone(), parse_rule(rule, &path, depth, false)?));
    }
    Ok(fields)
}

fn parse_rule(spec: &Json, path: &str, depth: usize, in_items: bool) -> Result<Rule, String> {
    let Json::Object(m) = spec else {
        return Err(format!("{path}: a rule is a JSON object such as {{\"type\": \"string\", \"required\": true}}"));
    };
    if let Some(k) = m.keys().find(|k| !KNOWN_KEYS.contains(&k.as_str())) {
        return Err(format!("{path}: unknown rule '{k}' (rules: {})", KNOWN_KEYS.join(", ")));
    }
    let flag = |k: &str| -> Result<bool, String> {
        match m.get(k) {
            None => Ok(false),
            Some(Json::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("{path}: '{k}' is true or false")),
        }
    };
    let number = |k: &str| -> Result<Option<f64>, String> {
        match m.get(k) {
            None => Ok(None),
            Some(v) => v.as_f64().map(Some).ok_or_else(|| format!("{path}: '{k}' is a number")),
        }
    };
    let count = |k: &str| -> Result<Option<usize>, String> {
        match m.get(k) {
            None => Ok(None),
            Some(v) => v.as_u64().map(|n| Some(n as usize)).ok_or_else(|| format!("{path}: '{k}' is a whole number")),
        }
    };
    let ty = match m.get("type") {
        None => None,
        Some(Json::String(s)) => Some(FieldType::parse(s).ok_or_else(|| {
            format!(
                "{path}: unknown type '{s}' (types: string, number, integer, boolean, null, array, object, date, \
                 datetime, time, any)"
            )
        })?),
        Some(_) => return Err(format!("{path}: 'type' is a name such as \"string\"")),
    };
    let mut r = Rule {
        ty,
        required: flag("required")?,
        nullable: flag("nullable")?,
        unique: flag("unique")?,
        min: number("min")?,
        max: number("max")?,
        min_length: count("minLength")?,
        max_length: count("maxLength")?,
        min_items: count("minItems")?,
        max_items: count("maxItems")?,
        ..Default::default()
    };
    let only = |what: &str, ok: bool, types: &str| -> Result<(), String> {
        if m.contains_key(what) && !ok {
            return Err(format!("{path}: '{what}' applies to {types}: give the field that type"));
        }
        Ok(())
    };
    let numeric = matches!(
        ty,
        Some(FieldType::Number | FieldType::Integer | FieldType::Int | FieldType::Float | FieldType::Decimal)
    );
    only("min", numeric, "numbers (type number, integer, int, float or decimal)")?;
    only("max", numeric, "numbers (type number, integer, int, float or decimal)")?;
    let text = ty == Some(FieldType::String);
    only("minLength", text, "strings")?;
    only("maxLength", text, "strings")?;
    only("pattern", text, "strings")?;
    let array = ty == Some(FieldType::Array);
    only("minItems", array, "arrays")?;
    only("maxItems", array, "arrays")?;
    only("items", array, "arrays")?;
    only("properties", ty == Some(FieldType::Object), "objects")?;
    if let (Some(a), Some(b)) = (r.min, r.max) {
        if a > b {
            return Err(format!("{path}: min ({a}) is above max ({b})"));
        }
    }
    if let (Some(a), Some(b)) = (r.min_length, r.max_length) {
        if a > b {
            return Err(format!("{path}: minLength ({a}) is above maxLength ({b})"));
        }
    }
    if r.unique {
        if in_items {
            return Err(format!("{path}: 'unique' applies to a field, not to the items of an array"));
        }
        if matches!(ty, Some(FieldType::Array | FieldType::Object)) {
            return Err(format!("{path}: a unique field holds a text, a number, a boolean or a date"));
        }
    }
    if let Some(v) = m.get("enum") {
        let Json::Array(values) = v else {
            return Err(format!("{path}: 'enum' is a list of the values allowed"));
        };
        if values.is_empty() {
            return Err(format!("{path}: 'enum' needs at least one value"));
        }
        r.one_of = Some(values.clone());
    }
    if let Some(v) = m.get("pattern") {
        let Some(p) = v.as_str() else {
            return Err(format!("{path}: 'pattern' is a regular expression, as a text"));
        };
        if p.len() > MAX_PATTERN_LEN {
            return Err(format!("{path}: 'pattern' is at most {MAX_PATTERN_LEN} characters"));
        }
        let re = regex::RegexBuilder::new(p)
            .size_limit(REGEX_SIZE_LIMIT)
            .dfa_size_limit(REGEX_SIZE_LIMIT)
            .build()
            .map_err(|e| format!("{path}: 'pattern' is not a usable regular expression: {e}"))?;
        r.pattern = Some(re);
    }
    if let Some(p) = m.get("properties") {
        r.properties = Some(parse_fields(p, path, depth + 1)?);
    }
    if let Some(i) = m.get("items") {
        r.items = Some(Box::new(parse_rule(i, &format!("{path}[]"), depth + 1, true)?));
    }
    if let Some(d) = m.get("default") {
        if in_items {
            return Err(format!("{path}: an item of an array has no default"));
        }
        // The only dynamic defaults, for the type they belong to; the same
        // text is a plain value for any other field.
        let dynamic = match (ty, d.as_str()) {
            (Some(FieldType::DateTime), Some("now()" | "current_timestamp")) => Some(Fill::Now),
            (Some(FieldType::Date), Some("today()" | "current_date")) => Some(Fill::Today),
            (Some(FieldType::Time), Some("current_time")) => Some(Fill::CurrentTime),
            _ => None,
        };
        r.default = Some(match dynamic {
            Some(d) => d,
            None => {
                let mut probe = Map::new();
                probe.insert("v".into(), d.clone());
                check_value(&r, Leaf::Json(&probe["v"]), path)
                    .map_err(|e| format!("the default does not follow the rule: {e}"))?;
                Fill::Value(d.clone())
            }
        });
    }
    Ok(r)
}

fn fill_and_check(fields: &Fields, map: &mut Map<String, Json>, prefix: &str, at: Moment) -> Result<(), String> {
    for (name, rule) in fields {
        if !map.contains_key(name) {
            if let Some(d) = &rule.default {
                map.insert(name.clone(), default_value(d, at));
            }
        }
        // Defaults inside an object present in the document.
        if let (Some(props), Some(Json::Object(inner))) = (&rule.properties, map.get_mut(name)) {
            fill_and_check(props, inner, &join(prefix, name), at)?;
        }
    }
    check_fields(fields, &|n| map.get(n).map(Leaf::Json), prefix)
}

fn fill_and_check_typed(fields: &Fields, map: &mut Members, prefix: &str, at: Moment) -> Result<(), String> {
    for (name, rule) in fields {
        if !map.contains_key(name) {
            if let Some(d) = &rule.default {
                map.insert(name.clone(), doc::from_json(&default_value(d, at)));
            }
        }
        if let (Some(props), Some(Doc::Object(inner))) = (&rule.properties, map.get_mut(name)) {
            fill_and_check_typed(props, inner, &join(prefix, name), at)?;
        }
    }
    check_fields(fields, &|n| map.get(n).map(Leaf::Doc), prefix)
}

fn default_value(d: &Fill, at: Moment) -> Json {
    match d {
        Fill::Value(v) => v.clone(),
        Fill::Now => Json::String(crate::time::iso(at.now)),
        // In the zone of the session writing, as `today()` in a command.
        Fill::Today => Json::String(crate::time::zone().wall(at.now).date().to_string()),
        Fill::CurrentTime => Json::String(crate::time::zone().wall(at.now).time().format("%H:%M:%S").to_string()),
    }
}

/// The fields of an object, by name.
type Lookup<'a> = dyn Fn(&str) -> Option<Leaf<'a>> + 'a;

fn check_fields<'a>(fields: &Fields, map: &Lookup<'a>, prefix: &str) -> Result<(), String> {
    for (name, rule) in fields {
        let path = join(prefix, name);
        match map(name) {
            None if rule.required => return Err(format!("{path}: required")),
            None => {}
            Some(v) => check_value(rule, v, &path)?,
        }
    }
    Ok(())
}

/// The number a value is, by its exact value: a decimal of a typed document included.
fn num_key(v: Leaf) -> Option<crate::num::NumKey> {
    use crate::num::NumKey;
    match v {
        Leaf::Json(Json::Number(n)) => Some(NumKey::Json(n.clone())),
        Leaf::Doc(Doc::Decimal(d)) => Some(NumKey::Decimal(*d)),
        Leaf::Doc(d @ (Doc::Int(_) | Doc::Float(_))) => match doc::to_plain_json(d) {
            Json::Number(n) => Some(NumKey::Json(n)),
            _ => None,
        },
        _ => None,
    }
}

fn check_value(rule: &Rule, v: Leaf, path: &str) -> Result<(), String> {
    let is_null = matches!(v, Leaf::Json(Json::Null) | Leaf::Doc(Doc::Null));
    if is_null && (rule.nullable || matches!(rule.ty, None | Some(FieldType::Null | FieldType::Any))) {
        return Ok(());
    }
    if let Some(ty) = rule.ty {
        if !ty.accepts_leaf(v) {
            let v = v.json();
            let v = v.as_ref();
            let want = match ty {
                FieldType::Date => "a date such as \"2026-10-27\"".to_string(),
                FieldType::DateTime => "a datetime with its offset, such as \"2026-10-27T14:30:00Z\"".to_string(),
                FieldType::Time => "a time such as \"08:00:00\"".to_string(),
                FieldType::Integer => "an integer".to_string(),
                t => format!("of type {}", t.name()),
            };
            let null = if v.is_null() { " (add \"nullable\": true to accept null)" } else { "" };
            return Err(format!("{path}: expected {want}, got {}{null}", short(v)));
        }
    }
    if let Some(n) = num_key(v) {
        // Exactly: 9007199254740993 is above a maximum of 9007199254740992,
        // a decimal compared by its value, never as a float.
        let vs = |m: f64| serde_json::Number::from_f64(m).map(|m| n.cmp_json(&m));
        let shown = || match &n {
            crate::num::NumKey::Json(x) => x.to_string(),
            crate::num::NumKey::Decimal(d) => d.to_string(),
        };
        if rule.min.and_then(vs) == Some(std::cmp::Ordering::Less) {
            return Err(format!("{path}: {} is below the minimum {}", shown(), rule.min.unwrap_or_default()));
        }
        if rule.max.and_then(vs) == Some(std::cmp::Ordering::Greater) {
            return Err(format!("{path}: {} is above the maximum {}", shown(), rule.max.unwrap_or_default()));
        }
    }
    if let Some(values) = &rule.one_of {
        let found = match (v.decimal(), num_key(v)) {
            (Some(_), Some(n)) => values.iter().any(|x| matches!(x, Json::Number(m) if n.cmp_json(m).is_eq())),
            _ => values.contains(&v.json()),
        };
        if !found {
            let list: Vec<String> = values.iter().map(|x| x.to_string()).collect();
            return Err(format!("{path}: {} is not one of {}", short(&v.json()), list.join(", ")));
        }
    }
    match v {
        Leaf::Json(j) => check_json_shape(rule, j, path),
        Leaf::Doc(Doc::Str(s)) => check_text(rule, s, path),
        Leaf::Doc(Doc::Array(items)) => {
            check_items(rule, items.len(), path)?;
            if let Some(each) = &rule.items {
                for (i, item) in items.iter().enumerate() {
                    check_value(each, Leaf::Doc(item), &format!("{path}[{i}]"))?;
                }
            }
            Ok(())
        }
        Leaf::Doc(Doc::Object(m)) => match &rule.properties {
            Some(props) => check_fields(props, &|n| m.get(n).map(Leaf::Doc), path),
            None => Ok(()),
        },
        Leaf::Doc(_) => Ok(()),
    }
}

fn check_items(rule: &Rule, len: usize, path: &str) -> Result<(), String> {
    if let Some(m) = rule.min_items.filter(|m| len < *m) {
        return Err(format!("{path}: {len} items, at least {m} needed"));
    }
    if let Some(m) = rule.max_items.filter(|m| len > *m) {
        return Err(format!("{path}: {len} items, at most {m} allowed"));
    }
    Ok(())
}

fn check_text(rule: &Rule, s: &str, path: &str) -> Result<(), String> {
    let len = s.chars().count();
    if let Some(m) = rule.min_length.filter(|m| len < *m) {
        return Err(format!("{path}: {len} characters, at least {m} needed"));
    }
    if let Some(m) = rule.max_length.filter(|m| len > *m) {
        return Err(format!("{path}: {len} characters, at most {m} allowed"));
    }
    if let Some(re) = &rule.pattern {
        if !re.is_match(s) {
            return Err(format!(
                "{path}: {} does not match the pattern {}",
                short(&Json::String(s.into())),
                re.as_str()
            ));
        }
    }
    Ok(())
}

/// The rules on texts, arrays and objects, for a value of a JSON document.
fn check_json_shape(rule: &Rule, v: &Json, path: &str) -> Result<(), String> {
    if let Some(s) = v.as_str() {
        check_text(rule, s, path)?;
    }
    if let Json::Array(items) = v {
        check_items(rule, items.len(), path)?;
        if let Some(each) = &rule.items {
            for (i, item) in items.iter().enumerate() {
                check_value(each, Leaf::Json(item), &format!("{path}[{i}]"))?;
            }
        }
    }
    if let (Json::Object(inner), Some(props)) = (v, &rule.properties) {
        check_fields(props, &|n| inner.get(n).map(Leaf::Json), path)?;
    }
    Ok(())
}

/// A value in an error message, cut short.
fn short(v: &Json) -> String {
    let s = v.to_string();
    if s.chars().count() > 60 {
        format!("{}...", s.chars().take(57).collect::<String>())
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at() -> Moment {
        Moment { now: "2026-10-27T14:30:00Z".parse().unwrap() }
    }

    fn users() -> Schema {
        Schema::parse(
            "user:*",
            &json!({
                "name": {"type": "string", "required": true, "minLength": 1, "maxLength": 10},
                "email": {"type": "string", "required": true, "unique": true, "pattern": "^[^@ ]+@[^@ ]+$"},
                "age": {"type": "integer", "min": 0, "max": 150},
                "role": {"enum": ["dev", "ops"], "default": "dev"},
                "active": {"type": "boolean", "default": true},
                "created_at": {"type": "datetime", "default": "now()"},
                "born": {"type": "date", "nullable": true},
                "tags": {"type": "array", "maxItems": 3, "items": {"type": "string"}},
                "address": {"type": "object", "properties": {
                    "city": {"type": "string", "required": true},
                    "country": {"type": "string", "default": "CD"}
                }}
            }),
        )
        .unwrap()
    }

    #[test]
    fn defaults_are_filled_then_every_rule_checked() {
        let s = users();
        let mut d = json!({"name": "Ana", "email": "ana@example.org", "address": {"city": "Goma"}});
        s.conform(&mut d, at()).unwrap();
        assert_eq!(d["role"], "dev");
        assert_eq!(d["active"], true);
        assert_eq!(d["created_at"], "2026-10-27T14:30:00.000Z");
        assert_eq!(d["address"]["country"], "CD");
        assert_eq!(s.unique_fields(), ["email"]);

        let refuse = |doc: Json| {
            let mut doc = doc;
            users().conform(&mut doc, at()).unwrap_err()
        };
        let base = || json!({"name": "Ana", "email": "a@b.c"});
        assert_eq!(refuse(json!({"email": "a@b.c"})), "name: required");
        let mut d = base();
        d["age"] = json!(31.5);
        assert!(refuse(d).starts_with("age: expected an integer"));
        let mut d = base();
        d["age"] = json!(-1);
        assert_eq!(refuse(d), "age: -1 is below the minimum 0");
        let mut d = base();
        d["name"] = json!("Anastasia-Maria");
        assert_eq!(refuse(d), "name: 15 characters, at most 10 allowed");
        let mut d = base();
        d["email"] = json!("not an address");
        assert!(refuse(d).contains("does not match the pattern"));
        let mut d = base();
        d["role"] = json!("boss");
        assert_eq!(refuse(d), r#"role: "boss" is not one of "dev", "ops""#);
        let mut d = base();
        d["tags"] = json!(["a", 2]);
        assert!(refuse(d).starts_with("tags[1]: expected of type string"));
        let mut d = base();
        d["address"] = json!({"country": "RW"});
        assert_eq!(refuse(d), "address.city: required");
        let mut d = base();
        d["created_at"] = json!("2026-10-27 14:30:00");
        assert!(refuse(d).contains("with its offset"), "a moment without its offset is ambiguous");
        let mut d = base();
        d["born"] = Json::Null;
        users().conform(&mut d, at()).unwrap();
        let mut d = base();
        d["active"] = Json::Null;
        assert!(refuse(d).contains("nullable"));
        assert!(refuse(json!(["not", "an", "object"])).contains("a JSON object is expected"));
    }

    #[test]
    fn a_broken_schema_is_refused_when_it_is_written() {
        let bad = |spec: Json| Schema::parse("x*", &spec).unwrap_err();
        assert!(bad(json!({})).contains("at least one field"));
        assert!(bad(json!({"a": {"typ": "string"}})).contains("unknown rule 'typ'"));
        assert!(bad(json!({"a": {"type": "text"}})).contains("unknown type 'text'"));
        assert!(bad(json!({"a": {"type": "string", "min": 1}})).contains("applies to numbers"));
        assert!(bad(json!({"a": {"type": "number", "min": 5, "max": 1}})).contains("above max"));
        assert!(bad(json!({"a": {"type": "string", "pattern": "("}})).contains("regular expression"));
        assert!(bad(json!({"a": {"type": "string", "pattern": "x".repeat(2000)}})).contains("at most"));
        assert!(bad(json!({"a": {"type": "integer", "default": "seven"}})).contains("the default does not follow"));
        assert!(bad(json!({"a": {"type": "array", "items": {"unique": true}}})).contains("not to the items"));
        assert!(bad(json!({"a": {"enum": []}})).contains("at least one value"));
        // A regular expression that would explode a backtracking engine is
        // refused by size, or runs in linear time.
        assert!(Schema::parse("x*", &json!({"a": {"type": "string", "pattern": "^(a+)+$"}})).is_ok());
        assert!(bad(json!({"a": {"type": "string", "pattern": "\\w{1000}{1000}"}})).contains("regular expression"));
    }

    #[test]
    fn the_same_text_is_dynamic_only_for_its_type() {
        let s = Schema::parse(
            "x*",
            &json!({
                "at": {"type": "datetime", "default": "now()"},
                "day": {"type": "date", "default": "today()"},
                "label": {"type": "string", "default": "now()"}
            }),
        )
        .unwrap();
        let mut d = json!({});
        s.conform(&mut d, at()).unwrap();
        assert_eq!(d["at"], "2026-10-27T14:30:00.000Z");
        assert!(crate::time::parse_date(d["day"].as_str().unwrap()).is_ok());
        assert_eq!(d["label"], "now()");
    }
}
