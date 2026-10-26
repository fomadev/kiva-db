// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Hint file: a snapshot of the in-memory index (keys, offsets, sizes - no
//! values) so that startup does not have to scan the whole data file.
//!
//! Header (44 bytes): `KHNT` | version u32 | file_id u64 | data_len u64 |
//! count u64 | compression u8 | encrypted u8 | pad[2] | body_crc u32 | header_crc u32
//!
//! When encrypted, the (compressed) body is sealed with the instance key
//! ([`crate::crypt`]): the hint lists every key, so it is protected like the data.
//!
//! Entry: `k_size u16 | key | offset u64 | v_size u32 | raw_size u32 |
//! type u8 | flags u8 | timestamp i64 | expires_at i64 | crc32 u32`
//!
//! `data_len` is the length of the data file the snapshot covers; anything
//! appended after it is replayed from the log at startup.

use crate::compress::{self, Compression};
use crate::crypt::{self, Domain, Keys};
use crate::engine::Entry;
use crate::error::{KivaError, Result};
use crate::value::Type;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const HINT_MAGIC: &[u8; 4] = b"KHNT";
const HINT_VERSION: u32 = 1;
const HINT_HEADER_LEN: usize = 44;

pub struct HintData {
    pub file_id: u64,
    pub data_len: u64,
    pub compression: Compression,
    /// Whether the snapshot was written encrypted.
    pub encrypted: bool,
    pub entries: Vec<(String, Entry)>,
}

pub fn hint_path(data_path: &Path) -> PathBuf {
    let mut s = data_path.as_os_str().to_owned();
    s.push(".hint");
    PathBuf::from(s)
}

pub fn write_hint(
    path: &Path,
    file_id: u64,
    data_len: u64,
    index: &BTreeMap<String, Entry>,
    compression: Compression,
    keys: Option<&Keys>,
) -> Result<()> {
    let mut body = Vec::with_capacity(index.len() * 48);
    for (key, e) in index {
        let start = body.len();
        body.extend_from_slice(&(key.len() as u16).to_le_bytes());
        body.extend_from_slice(key.as_bytes());
        body.extend_from_slice(&e.offset.to_le_bytes());
        body.extend_from_slice(&e.v_size.to_le_bytes());
        body.extend_from_slice(&e.raw_size.to_le_bytes());
        body.push(e.ty as u8);
        body.push(e.flags);
        body.extend_from_slice(&e.timestamp.to_le_bytes());
        body.extend_from_slice(&e.expires_at.to_le_bytes());
        let crc = crc32fast::hash(&body[start..]);
        body.extend_from_slice(&crc.to_le_bytes());
    }
    let (mut stored, flag) = compress::pack(&body, compression);
    if let Some(keys) = keys {
        keys.seal(Domain::Hint, &file_id.to_le_bytes(), &mut stored);
    }

    let mut out = Vec::with_capacity(HINT_HEADER_LEN + stored.len());
    out.extend_from_slice(HINT_MAGIC);
    out.extend_from_slice(&HINT_VERSION.to_le_bytes());
    out.extend_from_slice(&file_id.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&(index.len() as u64).to_le_bytes());
    out.push(flag);
    out.push(keys.is_some() as u8);
    out.extend_from_slice(&[0u8; 2]);
    out.extend_from_slice(&crc32fast::hash(&stored).to_le_bytes());
    let header_crc = crc32fast::hash(&out);
    out.extend_from_slice(&header_crc.to_le_bytes());
    out.extend_from_slice(&stored);

    // Write-then-rename so a crash never leaves a half-written hint behind.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, &out)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn bad(msg: &str) -> KivaError {
    KivaError::Corrupted(format!("hint file: {msg}"))
}

pub fn read_hint(path: &Path, keys: Option<&Keys>) -> Result<HintData> {
    let data = fs::read(path)?;
    if data.len() < HINT_HEADER_LEN || &data[..4] != HINT_MAGIC {
        return Err(bad("missing signature"));
    }
    let u32_at = |i: usize| u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
    let u64_at = |i: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&data[i..i + 8]);
        u64::from_le_bytes(b)
    };
    if u32_at(40) != crc32fast::hash(&data[..40]) {
        return Err(bad("header checksum mismatch"));
    }
    if u32_at(4) != HINT_VERSION {
        return Err(bad("unsupported version"));
    }
    let file_id = u64_at(8);
    let data_len = u64_at(16);
    let count = u64_at(24) as usize;
    let compression = Compression::from_flag(data[32]);
    let stored = &data[HINT_HEADER_LEN..];
    if u32_at(36) != crc32fast::hash(stored) {
        return Err(bad("body checksum mismatch"));
    }
    let encrypted = data[33] == 1;
    let opened;
    let stored = if encrypted {
        opened = crypt::open_with(keys, Domain::Hint, &file_id.to_le_bytes(), stored)
            .map_err(|e| e.into_error("the hint file"))?;
        &opened[..]
    } else {
        stored
    };
    let body = compress::unpack(stored, compression).map_err(|_| bad("body cannot be decompressed"))?;

    let mut entries = Vec::with_capacity(count.min(1 << 24));
    let mut p = 0usize;
    while p < body.len() {
        let start = p;
        if body.len() - p < 2 {
            return Err(bad("truncated entry"));
        }
        let k = u16::from_le_bytes([body[p], body[p + 1]]) as usize;
        p += 2;
        // key + offset(8) + v_size(4) + raw_size(4) + type(1) + flags(1) + ts(8) + exp(8) + crc(4)
        if body.len() - p < k + 38 {
            return Err(bad("truncated entry"));
        }
        let key = std::str::from_utf8(&body[p..p + k]).map_err(|_| bad("key is not UTF-8"))?.to_string();
        p += k;
        let rd64 = |i: usize| {
            let mut b = [0u8; 8];
            b.copy_from_slice(&body[i..i + 8]);
            b
        };
        let rd32 = |i: usize| u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]);
        let offset = u64::from_le_bytes(rd64(p));
        let v_size = rd32(p + 8);
        let raw_size = rd32(p + 12);
        let ty = Type::from_u8(body[p + 16]).ok_or_else(|| bad("unknown type tag"))?;
        let flags = body[p + 17];
        let timestamp = i64::from_le_bytes(rd64(p + 18));
        let expires_at = crate::util::expiry_to_ms(i64::from_le_bytes(rd64(p + 26)));
        p += 34;
        if rd32(p) != crc32fast::hash(&body[start..p]) {
            return Err(bad("entry checksum mismatch"));
        }
        p += 4;
        if k == 0 || offset.saturating_add(28 + k as u64 + v_size as u64) > data_len {
            return Err(bad("entry points outside the data file"));
        }
        entries.push((
            key,
            Entry { offset, k_size: k as u16, v_size, raw_size, ty, flags, timestamp, expires_at, version: 0 },
        ));
    }
    if entries.len() != count {
        return Err(bad("entry count mismatch"));
    }
    Ok(HintData { file_id, data_len, compression, encrypted, entries })
}
