// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Client for a running KivaDB server (binary protocol).
//!
//! ```no_run
//! use kivadb::client::Client;
//! use kivadb::Value;
//!
//! let mut db = Client::connect("127.0.0.1:2911")?;
//! db.set("greeting", &Value::Str("hello".into()), 0)?;
//! assert_eq!(db.get("greeting")?, Some(Value::Str("hello".into())));
//! println!("{}", db.exec("count where user:*.role = \"admin\"")?);
//! # Ok::<(), kivadb::KivaError>(())
//! ```

use crate::error::{KivaError, Result};
use crate::proto::*;
use crate::value::{Type, Value};
use std::net::{TcpStream, ToSocketAddrs};

pub struct Client {
    stream: TcpStream,
    /// Change notifications received so far (after a `subscribe`).
    pub events: Vec<String>,
    /// SHA-256 fingerprint of the server's certificate, for TLS connections.
    pub server_fingerprint: Option<String>,
}

impl Client {
    pub fn connect(addr: impl ToSocketAddrs) -> Result<Client> {
        let stream = TcpStream::connect(addr)?;
        let _ = stream.set_nodelay(true);
        Self::start(stream, None)
    }

    /// Connects with TLS. The server's certificate must have the given SHA-256
    /// fingerprint (shown by `kivadb tls` on the server); with `None`, any
    /// certificate is accepted and its fingerprint is in `server_fingerprint`,
    /// for the caller to check before sending a password.
    pub fn connect_tls(addr: &str, fingerprint: Option<&str>) -> Result<Client> {
        let secure = crate::tls::connect(addr, fingerprint, std::time::Duration::from_secs(10))?;
        Self::start(secure.stream, Some(secure.fingerprint))
    }

    fn start(stream: TcpStream, server_fingerprint: Option<String>) -> Result<Client> {
        let mut c = Client { stream, events: Vec::new(), server_fingerprint };
        // The first frame also tells the server this is a binary connection.
        c.call(CMD_HELLO, &[])?;
        Ok(c)
    }

    /// Sends one request and returns its reply, setting notifications aside.
    fn call(&mut self, cmd: u16, payload: &[u8]) -> Result<(u16, Vec<u8>)> {
        write_frame(&mut self.stream, cmd, payload)?;
        loop {
            let (code, body) = read_frame(&mut self.stream)?;
            if code == ST_EVENT {
                self.events.push(String::from_utf8_lossy(&body).into_owned());
            } else {
                return Ok((code, body));
            }
        }
    }

    fn text(body: &[u8]) -> String {
        String::from_utf8_lossy(body).into_owned()
    }

    /// Server banner: version, and whether a login is required.
    pub fn hello(&mut self) -> Result<String> {
        self.call(CMD_HELLO, &[]).map(|(_, b)| Self::text(&b))
    }

    pub fn ping(&mut self) -> Result<()> {
        self.call(CMD_PING, &[]).map(|_| ())
    }

    pub fn auth(&mut self, user: &str, password: &str) -> Result<String> {
        let mut p = user.as_bytes().to_vec();
        p.push(0);
        p.extend_from_slice(password.as_bytes());
        match self.call(CMD_AUTH, &p)? {
            (ST_OK, b) => Ok(Self::text(&b)),
            (_, b) => Err(KivaError::Denied(Self::text(&b))),
        }
    }

    /// Runs any shell command and returns what it printed.
    pub fn exec(&mut self, command: &str) -> Result<String> {
        let (code, body) = self.call(CMD_EXEC, command.as_bytes())?;
        // Every line is tagged '+' (output) or '-' (error).
        let text = Self::text(&body).lines().map(|l| l.get(1..).unwrap_or("")).collect::<Vec<_>>().join("\n");
        if code == ST_ERR {
            Err(KivaError::Other(text))
        } else {
            Ok(text)
        }
    }

    pub fn get(&mut self, key: &str) -> Result<Option<Value>> {
        match self.call(CMD_GET, key.as_bytes())? {
            (ST_OK, b) if !b.is_empty() => {
                let ty = Type::from_u8(b[0]).ok_or_else(|| KivaError::Invalid("unknown type tag".into()))?;
                Value::decode(ty, &b[1..], false).map(Some)
            }
            (ST_NOT_FOUND, _) => Ok(None),
            (_, b) => Err(KivaError::Other(Self::text(&b))),
        }
    }

    /// Creates or replaces a key. `ttl_secs == 0` means no expiry.
    pub fn set(&mut self, key: &str, value: &Value, ttl_secs: u32) -> Result<()> {
        let mut p = vec![value.ty() as u8];
        p.extend_from_slice(&ttl_secs.to_be_bytes());
        p.extend_from_slice(&(key.len() as u16).to_be_bytes());
        p.extend_from_slice(key.as_bytes());
        p.extend_from_slice(&value.encode(false)?);
        match self.call(CMD_SET, &p)? {
            (ST_OK, _) => Ok(()),
            (_, b) => Err(KivaError::Other(Self::text(&b))),
        }
    }

    /// Sends a structured request (see the `api` module) and returns its JSON answer.
    pub fn api(&mut self, request: &serde_json::Value) -> Result<serde_json::Value> {
        match self.call(CMD_API, request.to_string().as_bytes())? {
            (ST_OK, b) => Ok(serde_json::from_slice(&b)?),
            (_, b) => Err(KivaError::Other(Self::text(&b))),
        }
    }

    /// Deletes a key. Returns whether it existed.
    pub fn del(&mut self, key: &str) -> Result<bool> {
        match self.call(CMD_DEL, key.as_bytes())? {
            (ST_OK, _) => Ok(true),
            (ST_NOT_FOUND, _) => Ok(false),
            (_, b) => Err(KivaError::Other(Self::text(&b))),
        }
    }
}
