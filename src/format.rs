// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Binary layout of the `.kiva` data file.
//!
//! File header (16 bytes): `KIVA` | format u32 | file_id u64
//!
//! Record (28-byte header + key + value), all integers little-endian:
//! `crc32 u32 | k_size u16 | v_size u32 | type u8 | flags u8 | timestamp i64 | expires_at i64`
//! The CRC covers every byte of the record that follows it.

use crate::compress::Compression;

pub const MAGIC: &[u8; 4] = b"KIVA";
pub const FORMAT_VERSION: u32 = 1;
pub const FILE_HEADER_LEN: u64 = 16;
pub const REC_HEADER_LEN: usize = 28;

pub const FLAG_COMP_MASK: u8 = 0b0000_0011;
pub const FLAG_MSGPACK: u8 = 0b0000_0100;
/// Record belongs to an atomic batch; it only takes effect once the batch's
/// final record (carrying `FLAG_TXN_COMMIT` as well) is present on disk.
pub const FLAG_TXN: u8 = 0b0010_0000;
pub const FLAG_TXN_COMMIT: u8 = 0b0100_0000;
pub const FLAG_TOMBSTONE: u8 = 0b1000_0000;

/// Values shorter than this are never compressed.
pub const COMPRESS_MIN: usize = 64;
pub const MAX_KEY_LEN: usize = 1024;
pub const MAX_VALUE_LEN: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct RecHeader {
    pub crc: u32,
    pub k_size: u16,
    pub v_size: u32,
    pub ty: u8,
    pub flags: u8,
    pub timestamp: i64,
    pub expires_at: i64,
}

impl RecHeader {
    pub fn total_len(&self) -> u64 {
        REC_HEADER_LEN as u64 + self.k_size as u64 + self.v_size as u64
    }

    pub fn compression(&self) -> Compression {
        Compression::from_flag(self.flags & FLAG_COMP_MASK)
    }
}

pub fn file_header(file_id: u64) -> [u8; 16] {
    let mut h = [0u8; 16];
    h[..4].copy_from_slice(MAGIC);
    h[4..8].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    h[8..16].copy_from_slice(&file_id.to_le_bytes());
    h
}

/// Returns the file id, or a description of why the header is unusable.
pub fn parse_file_header(h: &[u8]) -> std::result::Result<u64, String> {
    if h.len() < FILE_HEADER_LEN as usize || &h[..4] != MAGIC {
        return Err("missing KIVA signature".into());
    }
    let version = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
    if version != FORMAT_VERSION {
        return Err(format!("unsupported storage format {version} (this build reads format {FORMAT_VERSION})"));
    }
    Ok(u64::from_le_bytes([h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]]))
}

pub fn encode_record(key: &str, value: &[u8], ty: u8, flags: u8, timestamp: i64, expires_at: i64) -> Vec<u8> {
    let k = key.as_bytes();
    let mut buf = Vec::with_capacity(REC_HEADER_LEN + k.len() + value.len());
    buf.extend_from_slice(&[0u8; 4]);
    buf.extend_from_slice(&(k.len() as u16).to_le_bytes());
    buf.extend_from_slice(&(value.len() as u32).to_le_bytes());
    buf.push(ty);
    buf.push(flags);
    buf.extend_from_slice(&timestamp.to_le_bytes());
    buf.extend_from_slice(&expires_at.to_le_bytes());
    buf.extend_from_slice(k);
    buf.extend_from_slice(value);
    let crc = crc32fast::hash(&buf[4..]);
    buf[..4].copy_from_slice(&crc.to_le_bytes());
    buf
}

pub fn parse_header(b: &[u8]) -> RecHeader {
    RecHeader {
        crc: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        k_size: u16::from_le_bytes([b[4], b[5]]),
        v_size: u32::from_le_bytes([b[6], b[7], b[8], b[9]]),
        ty: b[10],
        flags: b[11],
        timestamp: i64::from_le_bytes([b[12], b[13], b[14], b[15], b[16], b[17], b[18], b[19]]),
        expires_at: i64::from_le_bytes([b[20], b[21], b[22], b[23], b[24], b[25], b[26], b[27]]),
    }
}

/// Checks the CRC of a complete record buffer.
pub fn verify_record(rec: &[u8]) -> bool {
    if rec.len() < REC_HEADER_LEN {
        return false;
    }
    let h = parse_header(rec);
    h.total_len() == rec.len() as u64 && crc32fast::hash(&rec[4..]) == h.crc
}
