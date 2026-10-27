// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Unix time in milliseconds: the unit of every expiry (`expires_at`).
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The deadline `ttl` seconds from now, in milliseconds; 0 (none) when
/// `ttl` is not positive.
pub fn deadline_in(ttl: i64) -> i64 {
    if ttl > 0 {
        now_ms().saturating_add(ttl.saturating_mul(1000))
    } else {
        0
    }
}

/// Seconds left before `expires_at` (milliseconds), rounded up; for display.
pub fn secs_left(expires_at: i64) -> i64 {
    ((expires_at - now_ms()).max(0) + 999) / 1000
}

/// An expiry read from a file or a stream written before expiries were kept
/// in milliseconds: values below 10^11 can only be seconds (as milliseconds
/// they would fall in 1973), so they are converted.
pub fn expiry_to_ms(expires_at: i64) -> i64 {
    if expires_at > 0 && expires_at < 100_000_000_000 {
        expires_at * 1000
    } else {
        expires_at
    }
}

/// Local time, `YYYY-MM-DD HH:MM:SS`.
pub fn fmt_time(ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => "-".to_string(),
    }
}

/// Parses `YYYY-MM-DD` or `YYYY-MM-DD HH:MM:SS` (local time) into a Unix timestamp.
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .or_else(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)))?;
    Local.from_local_datetime(&naive).earliest().map(|d| d.timestamp())
}

pub fn human_size(n: u64) -> String {
    const KB: f64 = 1024.0;
    let f = n as f64;
    if n < 1024 {
        format!("{n} bytes")
    } else if f < KB * KB {
        format!("{:.2} KB", f / KB)
    } else if f < KB * KB * KB {
        format!("{:.2} MB", f / (KB * KB))
    } else {
        format!("{:.2} GB", f / (KB * KB * KB))
    }
}

/// The memory this process may use: the machine's RAM, or the limit of the
/// container it runs in when that is lower. `None` when it cannot be read.
pub fn physical_memory() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let total = std::fs::read_to_string("/proc/meminfo").ok().and_then(|m| {
            let line = m.lines().find(|l| l.starts_with("MemTotal:"))?;
            line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb * 1024)
        })?;
        // cgroup v2, then v1: "max" or a huge number means no limit.
        let limit = ["/sys/fs/cgroup/memory.max", "/sys/fs/cgroup/memory/memory.limit_in_bytes"]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok()?.trim().parse::<u64>().ok());
        Some(limit.map_or(total, |l| l.min(total)))
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        struct MemoryStatusEx {
            length: u32,
            memory_load: u32,
            total_phys: u64,
            avail_phys: u64,
            total_page_file: u64,
            avail_page_file: u64,
            total_virtual: u64,
            avail_virtual: u64,
            avail_extended_virtual: u64,
        }
        extern "system" {
            fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
        }
        let mut s = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: the structure has the layout Win32 expects, with its size set.
        let ok = unsafe { GlobalMemoryStatusEx(&mut s) } != 0;
        (ok && s.total_phys > 0).then_some(s.total_phys)
    }
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn sysctlbyname(
                name: *const std::ffi::c_char,
                old: *mut std::ffi::c_void,
                old_len: *mut usize,
                new: *mut std::ffi::c_void,
                new_len: usize,
            ) -> i32;
        }
        let mut bytes = 0u64;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: "hw.memsize" is a 64-bit integer, written into `bytes`.
        let rc = unsafe {
            sysctlbyname(c"hw.memsize".as_ptr(), (&mut bytes as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0)
        };
        (rc == 0 && bytes > 0).then_some(bytes)
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        None
    }
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let mut out = Vec::with_capacity(b.len() / 2);
    for pair in b.chunks(2) {
        out.push((nib(pair[0])? << 4) | nib(pair[1])?);
    }
    Some(out)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        s.push(B64[(n >> 18) as usize & 63] as char);
        s.push(B64[(n >> 12) as usize & 63] as char);
        s.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    s
}

pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Glob matching with `*` (any run) and `?` (any single character).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// The literal part of a glob before its first wildcard.
pub fn glob_prefix(pattern: &str) -> &str {
    match pattern.find(['*', '?']) {
        Some(i) => &pattern[..i],
        None => pattern,
    }
}

/// A new identifier for a key: 24 hexadecimal characters, in the format of
/// MongoDB's ObjectId. Four bytes of seconds since 1970, five random bytes
/// drawn once per process, three bytes of a counter starting at a random
/// value. Identifiers made later sort after the earlier ones (by the second,
/// then by the counter within one process), and two processes, on one
/// machine or several, do not share the random part.
pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::OnceLock;
    static PROCESS: OnceLock<[u8; 5]> = OnceLock::new();
    static COUNTER: OnceLock<AtomicU32> = OnceLock::new();
    let process = PROCESS.get_or_init(|| {
        let r = random_bytes(5);
        [r[0], r[1], r[2], r[3], r[4]]
    });
    let counter = COUNTER.get_or_init(|| {
        let r = random_bytes(3);
        AtomicU32::new(u32::from_be_bytes([0, r[0], r[1], r[2]]))
    });
    let n = counter.fetch_add(1, Ordering::Relaxed) & 0x00ff_ffff;
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as u32;
    let mut bytes = Vec::with_capacity(12);
    bytes.extend_from_slice(&secs.to_be_bytes());
    bytes.extend_from_slice(process);
    bytes.extend_from_slice(&n.to_be_bytes()[1..]);
    hex_encode(&bytes)
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    if getrandom::getrandom(&mut v).is_err() {
        // Extremely unlikely; fall back to a time-derived value rather than panic.
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        for (i, b) in v.iter_mut().enumerate() {
            *b = (t >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(31);
        }
    }
    v
}

/// Takes an exclusive lock on `file`, waiting up to half a second: a lock
/// just released can still look held for an instant (on Linux, a process
/// being started inherits open files until it runs its program).
pub fn lock_exclusive(file: &std::fs::File) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    loop {
        if file.try_lock().is_ok() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

pub fn random_u64() -> u64 {
    let b = random_bytes(8);
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

pub fn is_loopback(bind: &str) -> bool {
    bind == "localhost" || bind == "::1" || bind.starts_with("127.")
}

/// Decodes `%XX` escapes and `+` in a URL component.
pub fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let pair = [b[i + 1], b[i + 2]];
                match std::str::from_utf8(&pair).ok().and_then(hex_decode) {
                    Some(v) => {
                        out.push(v[0]);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `1.4 s ahead of`, `250 ms behind`: a clock drift in words.
pub fn drift_text(drift_ms: i64) -> String {
    let side = if drift_ms >= 0 { "ahead of" } else { "behind" };
    let d = drift_ms.unsigned_abs();
    if d >= 1000 {
        format!("{:.1} s {side}", d as f64 / 1000.0)
    } else {
        format!("{d} ms {side}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiries_are_milliseconds_and_old_seconds_are_converted() {
        // Seconds, as files written by KivaDB 1.5 hold them.
        assert_eq!(expiry_to_ms(1_793_000_000), 1_793_000_000_000);
        // Already milliseconds, or none.
        assert_eq!(expiry_to_ms(1_793_000_000_123), 1_793_000_000_123);
        assert_eq!(expiry_to_ms(0), 0);
        let d = deadline_in(10);
        assert!((d - now_ms() - 10_000).abs() < 1000);
        assert_eq!(deadline_in(0), 0);
        assert_eq!(secs_left(now_ms() + 1500), 2);
        assert_eq!(secs_left(now_ms() - 5), 0);
    }
}
