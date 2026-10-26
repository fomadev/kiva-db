// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Binary wire protocol.
//!
//! Every packet, in both directions: `[length u32 BE][code u16 BE][payload]`
//! where `length` counts the code and the payload. Requests carry a command
//! id, responses a status. A connection whose first byte is not `0x00` is
//! served in plain-text mode instead (one command per line), which keeps the
//! server usable from telnet or netcat.

use std::io::{self, Read, Write};

// Requests
pub const CMD_HELLO: u16 = 1;
pub const CMD_AUTH: u16 = 2;
pub const CMD_EXEC: u16 = 3;
pub const CMD_PING: u16 = 4;
pub const CMD_GET: u16 = 5;
pub const CMD_SET: u16 = 6;
pub const CMD_DEL: u16 = 7;
/// Structured request: a JSON object in, a JSON object out (see the api module).
pub const CMD_API: u16 = 8;
pub const CMD_REPL_SYNC: u16 = 20;
pub const CMD_REPL_ACK: u16 = 21;
/// Node-to-node request of the automatic failover (JSON in, JSON out).
pub const CMD_FAILOVER: u16 = 22;

// Responses
pub const ST_OK: u16 = 0;
pub const ST_ERR: u16 = 1;
pub const ST_NOT_FOUND: u16 = 2;
pub const ST_BYE: u16 = 3;
pub const ST_CLEAR: u16 = 4;
/// Unsolicited key-change notification.
pub const ST_EVENT: u16 = 9;

// Replication stream (master -> replica)
pub const R_SNAP_BEGIN: u16 = 30;
pub const R_SET: u16 = 31;
pub const R_DEL: u16 = 32;
pub const R_RESET: u16 = 33;
pub const R_SNAP_END: u16 = 34;
pub const R_PROMOTE: u16 = 35;
/// Heartbeat with the group's nodes (JSON), sent while the stream is idle.
pub const R_PING: u16 = 36;
/// The names of every workspace of the master, one per line: the replica
/// creates the missing ones and unregisters the others.
pub const R_WORKSPACES: u16 = 37;
/// The following operations are on this workspace (its name).
pub const R_USE: u16 = 38;
/// The master unregistered this workspace (its name).
pub const R_WS_REMOVE: u16 = 39;

pub const MAX_FRAME: usize = 300 * 1024 * 1024;

pub fn write_frame(w: &mut impl Write, code: u16, payload: &[u8]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(6 + payload.len());
    buf.extend_from_slice(&((payload.len() + 2) as u32).to_be_bytes());
    buf.extend_from_slice(&code.to_be_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

/// Like `read_exact`, but a read timeout does not lose the bytes already
/// received: the read resumes for as long as `keep_going` allows.
pub fn read_full(r: &mut impl Read, buf: &mut [u8], keep_going: &dyn Fn() -> bool) -> io::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed")),
            Ok(n) => filled += n,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                if !keep_going() {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn read_frame_with(r: &mut impl Read, keep_going: &dyn Fn() -> bool) -> io::Result<(u16, Vec<u8>)> {
    let mut head = [0u8; 6];
    read_full(r, &mut head, keep_going)?;
    let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if !(2..=MAX_FRAME).contains(&len) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid frame length"));
    }
    let code = u16::from_be_bytes([head[4], head[5]]);
    let mut payload = vec![0u8; len - 2];
    read_full(r, &mut payload, keep_going)?;
    Ok((code, payload))
}

pub fn read_frame(r: &mut impl Read) -> io::Result<(u16, Vec<u8>)> {
    read_frame_with(r, &|| true)
}

/// Cursor over a payload.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader { buf, pos: 0 }
    }

    pub fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.buf.len() - self.pos < n {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated payload"));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> io::Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> io::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> io::Result<u64> {
        let mut b = [0u8; 8];
        b.copy_from_slice(self.take(8)?);
        Ok(u64::from_be_bytes(b))
    }

    pub fn i64(&mut self) -> io::Result<i64> {
        Ok(self.u64()? as i64)
    }

    pub fn str16(&mut self) -> io::Result<String> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8"))
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.buf[self.pos..];
        self.pos = self.buf.len();
        s
    }
}
