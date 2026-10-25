// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The rules of the query language, and what it deliberately leaves out.
//!
//! A query reads **one key pattern** and tests **one condition** on one
//! field of each document: `<pattern>.<field> <op> <value>`. Every query is
//! then a single pass over one index or one range of keys, served from
//! memory, with a cost `explain` can give in advance.
//!
//! Joins, several conditions, aggregation pipelines and queries spanning
//! several servers are not part of the language. This module recognises
//! them, in the shell's syntax and in structured requests, and refuses them
//! with what to do instead, rather than with a generic syntax error.

use crate::jsonpath;
use serde_json::Value as Json;

/// What a query tried to do that the language does not offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// Two conditions joined by `and` / `or`.
    Combination,
    /// Two key patterns, or a value taken from other documents.
    Join,
    /// `group by`, `order by`, `distinct`...
    Pipeline,
    /// A query sent to several servers or shards.
    Distributed,
    /// `in`, `like`, `between`...: not one of the six comparisons.
    Operator,
}

impl Unsupported {
    pub fn message(self) -> &'static str {
        match self {
            Unsupported::Combination => {
                "a query tests one condition: 'and' / 'or' are not supported. Run one query per condition \
                 and combine the keys in your application, or store the combined value in a field of its own \
                 and query that field."
            }
            Unsupported::Join => {
                "joins are not supported: a query reads one key pattern and compares with a literal value. \
                 Keep related data in the same document, or read the related keys by name (get a and b)."
            }
            Unsupported::Pipeline => {
                "aggregation pipelines (group by, order by, distinct...) are not supported: the aggregates \
                 are 'count where' and 'sum where'; 'range' and 'keys' return keys in order."
            }
            Unsupported::Distributed => {
                "distributed queries are not supported: one KivaDB server holds all of a workspace's data \
                 (replicas hold copies, see 'replica status'), so a query never spans several servers."
            }
            Unsupported::Operator => "the comparison operators are = != > >= < <=, with one value.",
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
        "and" | "or" | "&&" | "||" | "not" => Unsupported::Combination,
        "join" | "inner" | "outer" | "left" | "right" | "cross" | "natural" | "lookup" | "on" | "using" => {
            Unsupported::Join
        }
        "group" | "order" | "sort" | "by" | "having" | "distinct" | "union" | "select" | "avg" | "min" | "max"
        | "pipeline" | "aggregate" | "offset" => Unsupported::Pipeline,
        "shard" | "shards" | "node" | "nodes" | "cluster" | "across" | "partition" | "distributed" => {
            Unsupported::Distributed
        }
        "in" | "like" | "ilike" | "between" | "contains" | "matches" | "regexp" | "is" => Unsupported::Operator,
        _ => return None,
    })
}

/// Fields of a structured request that ask for something the language
/// leaves out.
pub fn unsupported_request_field(name: &str) -> Option<Unsupported> {
    Some(match name.to_ascii_lowercase().as_str() {
        "and" | "or" | "not" | "conditions" | "filters" | "where" => Unsupported::Combination,
        "join" | "joins" | "lookup" | "with" => Unsupported::Join,
        "group" | "group_by" | "groupby" | "order" | "order_by" | "orderby" | "sort" | "pipeline" | "aggregate"
        | "distinct" | "having" => Unsupported::Pipeline,
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

/// The value of the condition: one string, number, boolean or null.
pub fn check_value(value: &Json) -> Result<(), String> {
    match value {
        Json::Array(_) | Json::Object(_) => Err(format!(
            "a condition compares with one string, number, boolean or null, not with an array or an object: {}",
            Unsupported::Operator.message()
        )),
        _ => Ok(()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recognises_what_the_language_leaves_out() {
        assert_eq!(unsupported_word("AND"), Some(Unsupported::Combination));
        assert_eq!(unsupported_word("join"), Some(Unsupported::Join));
        assert_eq!(unsupported_word("group"), Some(Unsupported::Pipeline));
        assert_eq!(unsupported_word("@node2"), Some(Unsupported::Distributed));
        assert_eq!(unsupported_word("in"), Some(Unsupported::Operator));
        assert_eq!(unsupported_word("limit"), None);
        assert_eq!(unsupported_word("return"), None);

        assert!(check_request(&json!({"op": "query", "target": "a:*.b", "join": "c:*"})).is_err());
        assert!(check_request(&json!({"op": "query", "target": "a:*.b", "order_by": "b"})).is_err());
        assert!(check_request(&json!({"op": "query", "target": "a:*.b", "cmp": "=", "value": 1, "limit": 5})).is_ok());

        assert!(check_field("address.city").is_ok());
        assert!(check_field("tags[*]").is_err());
        assert!(check_field("a..b").is_err());
        assert!(check_value(&json!("x")).is_ok());
        assert!(check_value(&json!(["a", "b"])).is_err());

        assert!(is_field_reference("order:*.user"));
        assert!(is_field_reference("user:2.age"));
        assert!(!is_field_reference("3.5"));
        assert!(!is_field_reference("admin"));
    }
}
