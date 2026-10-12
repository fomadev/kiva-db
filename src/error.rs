// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use std::fmt;

/// Every failure the engine or the command layer can report.
#[derive(Debug)]
pub enum KivaError {
    Io(std::io::Error),
    /// A record failed its checksum or could not be decoded.
    Corrupted(String),
    /// The database (or the whole home directory) is held by another process.
    Locked(String),
    /// The file is not a KivaDB data file of a supported format.
    Format(String),
    Invalid(String),
    NotFound(String),
    Exists(String),
    Type(String),
    /// Optimistic transaction conflict detected at commit time.
    Conflict(String),
    Denied(String),
    ReadOnly(String),
    Script(String),
    Other(String),
}

pub type Result<T> = std::result::Result<T, KivaError>;

impl fmt::Display for KivaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KivaError::Io(e) => write!(f, "I/O error: {e}"),
            KivaError::Corrupted(m) => write!(f, "ERR_DATA_CORRUPTED: {m}"),
            KivaError::Locked(m) => write!(f, "database is locked by another process: {m}"),
            KivaError::Format(m) => write!(f, "invalid database file: {m}"),
            KivaError::Invalid(m) => write!(f, "{m}"),
            KivaError::NotFound(m) => write!(f, "{m}"),
            KivaError::Exists(m) => write!(f, "{m}"),
            KivaError::Type(m) => write!(f, "TypeError: {m}"),
            KivaError::Conflict(m) => write!(f, "ERR_TRANSACTION_CONFLICT: {m}"),
            KivaError::Denied(m) => write!(f, "permission denied: {m}"),
            KivaError::ReadOnly(m) => write!(f, "READONLY: {m}"),
            KivaError::Script(m) => write!(f, "ScriptError: {m}"),
            KivaError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for KivaError {}

impl From<std::io::Error> for KivaError {
    fn from(e: std::io::Error) -> Self {
        KivaError::Io(e)
    }
}

impl From<serde_json::Error> for KivaError {
    fn from(e: serde_json::Error) -> Self {
        KivaError::Invalid(format!("invalid JSON: {e}"))
    }
}

impl From<mlua::Error> for KivaError {
    fn from(e: mlua::Error) -> Self {
        KivaError::Script(e.to_string())
    }
}
