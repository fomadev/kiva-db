// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! A document as queries, indexes and aggregates read it: JSON as stored,
//! or a typed tree ([`Doc`]), without turning one into the other.
//!
//! A value of a typed document reads as its plain JSON ([`Leaf::json`]):
//! an `int` or a `float` as a JSON number, a date, a time or a datetime as
//! its ISO text, which conditions on moments compare as moments, as they
//! do for the ISO texts of JSON documents. A `decimal` is the one value
//! JSON cannot carry: [`Leaf::decimal`] gives it, so that it is compared
//! by its exact value rather than as a text.

use crate::doc::{self, Doc};
use crate::jsonpath::{self, Seg};
use crate::value::Value;
use rust_decimal::Decimal;
use serde_json::Value as Json;
use std::borrow::Cow;

#[derive(Clone, Copy, Debug)]
pub enum DocView<'a> {
    Json(&'a Json),
    Typed(&'a Doc),
}

/// The value at a path of a document.
#[derive(Clone, Copy, Debug)]
pub enum Leaf<'a> {
    Json(&'a Json),
    Doc(&'a Doc),
}

impl<'a> DocView<'a> {
    /// The document a value holds, if it holds one.
    pub fn of(v: &'a Value) -> Option<DocView<'a>> {
        match v {
            Value::Doc(j) => Some(DocView::Json(j)),
            Value::Typed(d) => Some(DocView::Typed(d)),
            _ => None,
        }
    }

    pub fn get(self, path: &[Seg]) -> Option<Leaf<'a>> {
        match self {
            DocView::Json(j) => jsonpath::get(j, path).map(Leaf::Json),
            DocView::Typed(d) => crate::docpath::get(d, path).map(Leaf::Doc),
        }
    }

    /// The whole document in plain JSON.
    pub fn json(self) -> Cow<'a, Json> {
        match self {
            DocView::Json(j) => Cow::Borrowed(j),
            DocView::Typed(d) => Cow::Owned(doc::to_plain_json(d)),
        }
    }
}

impl<'a> Leaf<'a> {
    /// The value in plain JSON.
    pub fn json(self) -> Cow<'a, Json> {
        match self {
            Leaf::Json(j) => Cow::Borrowed(j),
            Leaf::Doc(d) => Cow::Owned(doc::to_plain_json(d)),
        }
    }

    /// The decimal it is, which plain JSON would turn into a text.
    pub fn decimal(self) -> Option<&'a Decimal> {
        match self {
            Leaf::Doc(Doc::Decimal(d)) => Some(d),
            _ => None,
        }
    }
}
