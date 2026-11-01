// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use crate::doc::{self, Doc};
use crate::error::{KivaError, Result};
use crate::format::{FLAG_KDOC, FLAG_MSGPACK};
use crate::time;
use crate::util::{b64_decode, b64_encode, hex_encode};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde_json::Value as Json;
use std::borrow::Cow;

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

/// How a document's bytes are written, as the record flags say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocEnc {
    Json,
    MsgPack,
    Kdoc,
}

impl DocEnc {
    pub fn from_flags(flags: u8) -> DocEnc {
        if flags & FLAG_KDOC != 0 {
            DocEnc::Kdoc
        } else if flags & FLAG_MSGPACK != 0 {
            DocEnc::MsgPack
        } else {
            DocEnc::Json
        }
    }

    /// The record flags that say it.
    pub fn flags(self) -> u8 {
        match self {
            DocEnc::Json => 0,
            DocEnc::MsgPack => FLAG_MSGPACK,
            DocEnc::Kdoc => FLAG_KDOC,
        }
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
    /// An array or object holding a value JSON cannot carry (`decimal`,
    /// `date`, `time`, `datetime`), written in `KDOC`. Made by
    /// [`Value::typed`], which keeps a document JSON can hold as `Doc`: a
    /// document is `Typed` exactly when it is stored in `KDOC`.
    Typed(Doc),
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
            Value::Typed(Doc::Array(_)) => Type::Array,
            Value::Typed(_) => Type::Object,
            Value::Bytes(_) => Type::Bytes,
            Value::Date(_) => Type::Date,
            Value::DateTime(_) => Type::DateTime,
            Value::Time(_) => Type::Time,
        }
    }

    /// A document as a value: `Typed` when it holds a value JSON cannot
    /// carry, else `Doc` in plain JSON, which then loses nothing.
    pub fn typed(d: Doc) -> Result<Value> {
        check_document(&d)?;
        Ok(if d.json_encodable() { Value::Doc(doc::to_plain_json(&d)) } else { Value::Typed(d) })
    }

    /// The document as JSON, to read it: a typed one in plain JSON, its
    /// decimals, dates and times as texts.
    pub fn json_doc(&self) -> Option<Cow<'_, Json>> {
        match self {
            Value::Doc(j) => Some(Cow::Borrowed(j)),
            Value::Typed(d) => Some(Cow::Owned(doc::to_plain_json(d))),
            _ => None,
        }
    }

    /// [`Value::json_doc`], owned.
    pub fn into_json_doc(self) -> Option<Json> {
        match self {
            Value::Doc(j) => Some(j),
            Value::Typed(d) => Some(doc::to_plain_json(&d)),
            _ => None,
        }
    }

    /// The bytes as stored, and how: a typed document in `KDOC`, another
    /// document in MessagePack when `msgpack`, else as [`Value::encode`]. A
    /// `Typed` that JSON can hold is written as JSON: `KDOC` only when needed.
    pub fn encode_stored(&self, msgpack: bool) -> Result<(Vec<u8>, DocEnc)> {
        match self {
            Value::Typed(d) => {
                check_document(d)?;
                if d.json_encodable() {
                    return Value::Doc(doc::to_plain_json(d)).encode_stored(msgpack);
                }
                Ok((doc::encode(d), DocEnc::Kdoc))
            }
            Value::Doc(_) if msgpack => Ok((self.encode(true)?, DocEnc::MsgPack)),
            _ => Ok((self.encode(false)?, DocEnc::Json)),
        }
    }

    /// Canonical payload bytes. `msgpack` only affects documents; a typed
    /// one is written in plain JSON (what the network protocol carries).
    pub fn encode(&self, msgpack: bool) -> Result<Vec<u8>> {
        Ok(match self {
            Value::Typed(d) => return Value::Doc(doc::to_plain_json(d)).encode(msgpack),
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

    pub fn decode(ty: Type, raw: &[u8], enc: DocEnc) -> Result<Value> {
        let text = |raw: &[u8]| {
            std::str::from_utf8(raw)
                .map(|s| s.to_string())
                .map_err(|_| KivaError::Corrupted("value is not valid UTF-8".into()))
        };
        if enc == DocEnc::Kdoc && !ty.is_doc() {
            return Err(KivaError::Corrupted(format!("a [{}] marked as a KDOC document", ty.name())));
        }
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
            Type::Array | Type::Object if enc == DocEnc::Kdoc => {
                let d = doc::decode(raw).map_err(|e| KivaError::Corrupted(format!("KDOC document: {e}")))?;
                let ok = matches!((ty, &d), (Type::Array, Doc::Array(_)) | (Type::Object, Doc::Object(_)));
                if !ok {
                    return Err(KivaError::Corrupted("document does not match its type tag".into()));
                }
                // Read as written; one JSON could hold is kept as JSON.
                if d.json_encodable() {
                    Value::Doc(doc::to_plain_json(&d))
                } else {
                    Value::Typed(d)
                }
            }
            Type::Array | Type::Object => {
                let j: Json = if enc == DocEnc::MsgPack {
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
            Value::Typed(d) => doc::to_plain_json(d).to_string(),
            Value::Bytes(b) => {
                if b.len() > 64 {
                    format!("0x{}... ({} bytes)", hex_encode(&b[..64]), b.len())
                } else {
                    format!("0x{}", hex_encode(b))
                }
            }
            Value::Date(d) => time::shown_date(*d),
            // In the zone of the session.
            Value::DateTime(t) => time::shown_instant(*t),
            Value::Time(t) => time::shown_time(*t),
        }
    }

    pub fn to_json(&self) -> Json {
        match self {
            Value::Str(s) => Json::String(s.clone()),
            Value::Num(s) => num_to_json(s),
            Value::Bool(b) => Json::Bool(*b),
            Value::Null => Json::Null,
            Value::Doc(j) => j.clone(),
            Value::Typed(d) => doc::to_plain_json(d),
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

/// Why a typed document is not changed in place yet.
pub fn typed_unchangeable(key: &str) -> KivaError {
    KivaError::Invalid(format!(
        "'{key}' holds decimal, date, time or datetime values, which this command would turn into texts: \
         write the whole document again instead"
    ))
}

/// An array or an object that `KDOC` reads back as it is ([`doc::check`]).
fn check_document(d: &Doc) -> Result<()> {
    if !matches!(d, Doc::Array(_) | Doc::Object(_)) {
        return Err(KivaError::Invalid(format!("a document is an array or an object, not a [{}]", d.type_name())));
    }
    doc::check(d).map_err(KivaError::Invalid)
}
