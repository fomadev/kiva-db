// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! KivaDB: a typed, persistent key-value and document database.
//!
//! The crate can be used as an embedded library through [`engine::Engine`]
//! (storage only) or [`instance::Instance`] + [`session::Session`] (the full
//! command language, users, workspaces and scripting).

pub mod api;
pub mod auth;
pub mod cache;
pub mod client;
pub mod compress;
pub mod config;
pub mod engine;
pub mod error;
pub mod exec;
mod exec_admin;
pub mod expr;
pub mod format;
pub mod hint;
pub mod http;
pub mod instance;
pub mod jsonpath;
pub mod lexer;
pub mod lineedit;
pub mod proto;
pub mod repl;
pub mod resp;
pub mod script;
pub mod server;
pub mod session;
pub mod shell;
pub mod util;
pub mod value;
pub mod vector;

pub use engine::{Engine, EngineOptions};
pub use error::{KivaError, Result};
pub use instance::{Instance, Options, VERSION};
pub use session::Session;
pub use value::{Type, Value};
