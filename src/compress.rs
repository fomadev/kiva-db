// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use crate::error::{KivaError, Result};
use crate::format::COMPRESS_MIN;
use std::io::{Read, Write};

/// Per-record value compression. The algorithm used is recorded in the
/// record's flags, so files may freely mix compressed and plain records.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Compression {
    None,
    Lz4,
    Zlib,
}

impl Compression {
    pub fn flag(self) -> u8 {
        match self {
            Compression::None => 0,
            Compression::Lz4 => 1,
            Compression::Zlib => 2,
        }
    }

    pub fn from_flag(f: u8) -> Compression {
        match f & 3 {
            1 => Compression::Lz4,
            2 => Compression::Zlib,
            _ => Compression::None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Compression::None => "none",
            Compression::Lz4 => "lz4",
            Compression::Zlib => "zlib",
        }
    }

    pub fn from_name(s: &str) -> Option<Compression> {
        Some(match s.to_ascii_lowercase().as_str() {
            "none" | "off" => Compression::None,
            "lz4" => Compression::Lz4,
            "zlib" => Compression::Zlib,
            _ => return None,
        })
    }
}

/// Compresses `raw` as `[raw_len u32][payload]`. Returns the stored bytes and
/// the flag actually used: values that are small or do not shrink stay plain.
pub fn pack(raw: &[u8], algo: Compression) -> (Vec<u8>, u8) {
    if algo == Compression::None || raw.len() < COMPRESS_MIN {
        return (raw.to_vec(), 0);
    }
    let body = match algo {
        Compression::Lz4 => lz4_flex::compress(raw),
        Compression::Zlib => {
            let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            if enc.write_all(raw).is_err() {
                return (raw.to_vec(), 0);
            }
            match enc.finish() {
                Ok(v) => v,
                Err(_) => return (raw.to_vec(), 0),
            }
        }
        Compression::None => unreachable!(),
    };
    if body.len() + 4 >= raw.len() {
        return (raw.to_vec(), 0);
    }
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    (out, algo.flag())
}

pub fn unpack(stored: &[u8], algo: Compression) -> Result<Vec<u8>> {
    if algo == Compression::None {
        return Ok(stored.to_vec());
    }
    if stored.len() < 4 {
        return Err(KivaError::Corrupted("compressed value is truncated".into()));
    }
    let raw_len = u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]) as usize;
    if raw_len > crate::format::MAX_VALUE_LEN {
        return Err(KivaError::Corrupted("compressed value declares an absurd size".into()));
    }
    let body = &stored[4..];
    let raw = match algo {
        Compression::Lz4 => {
            lz4_flex::decompress(body, raw_len).map_err(|e| KivaError::Corrupted(format!("lz4: {e}")))?
        }
        Compression::Zlib => {
            let mut out = Vec::with_capacity(raw_len);
            flate2::read::ZlibDecoder::new(body)
                .take(raw_len as u64 + 1)
                .read_to_end(&mut out)
                .map_err(|e| KivaError::Corrupted(format!("zlib: {e}")))?;
            out
        }
        Compression::None => unreachable!(),
    };
    if raw.len() != raw_len {
        return Err(KivaError::Corrupted("decompressed size mismatch".into()));
    }
    Ok(raw)
}
