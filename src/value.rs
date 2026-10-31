// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use crate::error::{KivaError, Result};
use crate::time;
use crate::util::{b64_decode, b64_encode, hex_encode};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde_json::Value as Json;

/// On-disk type tag of a stored value.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Type {
    String = 1,
    Number = 2,
    Boolean = 3,
    Null = 4,
    Array = 5,
    Object = 6,
    Bytes = 7,
    /// A day of the calendar: `2026-10-27`.
    Date = 8,
    /// An instant, in UTC to the millisecond.
    DateTime = 9,
    /// A time of day: `14:30:00`.
    Time = 10,
}

impl Type {
    pub fn name(self) -> &'static str {
        match self {
            Type::String => "string",
            Type::Number => "number",
            Type::Boolean => "boolean",
            Type::Null => "null",
            Type::Array => "array",
            Type::Object => "object",
            Type::Bytes => "bytes",
            Type::Date => "date",
            Type::DateTime => "datetime",
            Type::Time => "time",
        }
    }

    pub fn from_name(s: &str) -> Option<Type> {
        Some(match s {
            "string" => Type::String,
            "number" => Type::Number,
            "boolean" => Type::Boolean,
            "null" => Type::Null,
            "array" => Type::Array,
            "object" => Type::Object,
            "bytes" => Type::Bytes,
            "date" => Type::Date,
            "datetime" => Type::DateTime,
            "time" => Type::Time,
            _ => return None,
        })
    }

    pub fn from_u8(b: u8) -> Option<Type> {
        Some(match b {
            1 => Type::String,
            2 => Type::Number,
            3 => Type::Boolean,
            4 => Type::Null,
            5 => Type::Array,
            6 => Type::Object,
            7 => Type::Bytes,
            8 => Type::Date,
            9 => Type::DateTime,
            10 => Type::Time,
            _ => return None,
        })
    }

    pub fn is_doc(self) -> bool {
        matches!(self, Type::Array | Type::Object)
    }

    /// `date`, `datetime` or `time`.
    pub fn is_temporal(self) -> bool {
        matches!(self, Type::Date | Type::DateTime | Type::Time)
    }
}

/// A typed value. Numbers keep their textual form so that what was written is
/// exactly what is read back; `Doc` always holds a JSON array or object.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Num(String),
    Bool(bool),
    Null,
    Doc(Json),
    Bytes(Vec<u8>),
    Date(NaiveDate),
    DateTime(DateTime<Utc>),
    Time(NaiveTime),
}

impl Value {
    pub fn ty(&self) -> Type {
        match self {
            Value::Str(_) => Type::String,
            Value::Num(_) => Type::Number,
            Value::Bool(_) => Type::Boolean,
            Value::Null => Type::Null,
            Value::Doc(j) if j.is_array() => Type::Array,
            Value::Doc(_) => Type::Object,
            Value::Bytes(_) => Type::Bytes,
            Value::Date(_) => Type::Date,
            Value::DateTime(_) => Type::DateTime,
            Value::Time(_) => Type::Time,
        }
    }

    /// Canonical payload bytes. `msgpack` only affects documents.
    pub fn encode(&self, msgpack: bool) -> Result<Vec<u8>> {
        Ok(match self {
            Value::Str(s) => s.as_bytes().to_vec(),
            Value::Num(s) => s.as_bytes().to_vec(),
            Value::Bool(b) => {
                if *b {
                    b"true".to_vec()
                } else {
                    b"false".to_vec()
                }
            }
            Value::Null => Vec::new(),
            Value::Doc(j) => {
                if msgpack {
                    rmp_serde::to_vec(j).map_err(|e| KivaError::Invalid(format!("MessagePack encoding failed: {e}")))?
                } else {
                    serde_json::to_vec(j)?
                }
            }
            Value::Bytes(b) => b.clone(),
            // Text, as the other values: a datetime as ISO 8601 in UTC.
            Value::Date(d) => time::date_text(*d).into_bytes(),
            Value::DateTime(t) => time::iso(*t).into_bytes(),
            Value::Time(t) => time::time_text(*t).into_bytes(),
        })
    }

    pub fn decode(ty: Type, raw: &[u8], msgpack: bool) -> Result<Value> {
        let text = |raw: &[u8]| {
            std::str::from_utf8(raw)
                .map(|s| s.to_string())
                .map_err(|_| KivaError::Corrupted("value is not valid UTF-8".into()))
        };
        Ok(match ty {
            Type::String => Value::Str(text(raw)?),
            Type::Number => {
                let s = text(raw)?;
                if !is_valid_number(&s) {
                    return Err(KivaError::Corrupted("stored number is malformed".into()));
                }
                Value::Num(s)
            }
            Type::Boolean => match raw {
                b"true" => Value::Bool(true),
                b"false" => Value::Bool(false),
                _ => return Err(KivaError::Corrupted("stored boolean is malformed".into())),
            },
            Type::Null => Value::Null,
            Type::Array | Type::Object => {
                let j: Json = if msgpack {
                    rmp_serde::from_slice(raw)
                        .map_err(|e| KivaError::Corrupted(format!("MessagePack document: {e}")))?
                } else {
                    serde_json::from_slice(raw).map_err(|e| KivaError::Corrupted(format!("JSON document: {e}")))?
                };
                let ok = (ty == Type::Array && j.is_array()) || (ty == Type::Object && j.is_object());
                if !ok {
                    return Err(KivaError::Corrupted("document does not match its type tag".into()));
                }
                Value::Doc(j)
            }
            Type::Bytes => Value::Bytes(raw.to_vec()),
            Type::Date => Value::Date(time::parse_date(&text(raw)?).map_err(KivaError::Corrupted)?),
            Type::DateTime => {
                Value::DateTime(time::parse_datetime(&text(raw)?, &time::Zone::Utc).map_err(KivaError::Corrupted)?)
            }
            Type::Time => Value::Time(time::parse_time(&text(raw)?).map_err(KivaError::Corrupted)?),
        })
    }

    /// Human-readable rendering used by the shell.
    pub fn display(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Num(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Doc(j) => j.to_string(),
            Value::Bytes(b) => {
                if b.len() > 64 {
                    format!("0x{}... ({} bytes)", hex_encode(&b[..64]), b.len())
                } else {
                    format!("0x{}", hex_encode(b))
                }
            }
            Value::Date(d) => time::date_text(*d),
            // In the zone of the session.
            Value::DateTime(t) => time::show(*t),
            Value::Time(t) => time::time_text(*t),
        }
    }

    pub fn to_json(&self) -> Json {
        match self {
            Value::Str(s) => Json::String(s.clone()),
            Value::Num(s) => num_to_json(s),
            Value::Bool(b) => Json::Bool(*b),
            Value::Null => Json::Null,
            Value::Doc(j) => j.clone(),
            Value::Bytes(b) => serde_json::json!({ "$bytes": b64_encode(b) }),
            Value::Date(d) => Json::String(time::date_text(*d)),
            Value::DateTime(t) => Json::String(time::iso(*t)),
            Value::Time(t) => Json::String(time::time_text(*t)),
        }
    }

    pub fn from_json(j: Json) -> Value {
        match j {
            Json::String(s) => Value::Str(s),
            Json::Number(n) => Value::Num(n.to_string()),
            Json::Bool(b) => Value::Bool(b),
            Json::Null => Value::Null,
            Json::Object(ref m) if m.len() == 1 && m.contains_key("$bytes") => {
                match m["$bytes"].as_str().and_then(b64_decode) {
                    Some(b) => Value::Bytes(b),
                    None => Value::Doc(j),
                }
            }
            other => Value::Doc(other),
        }
    }
}

/// A stored number as JSON: a JSON number when one holds it exactly, else
/// its text, rather than a rounded float (a 23-digit integer, 20 decimals).
pub fn num_to_json(s: &str) -> Json {
    crate::num::exact_json(s).unwrap_or_else(|| Json::String(s.to_string()))
}

/// `-12`, `3.14`, `1e9`, `2.5E-3`. No leading `+`, no bare `.5`.
pub fn is_valid_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && b[i] == b'-' {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > start
    };
    if !digits(&mut i) {
        return false;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        if !digits(&mut i) {
            return false;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        if !digits(&mut i) {
            return false;
        }
    }
    i == b.len() && s.parse::<f64>().map(|f| f.is_finite()).unwrap_or(false)
}

/// Formats an arithmetic result: integers print without a fractional part.
pub fn fmt_f64(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 {
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}
